use axum::{
    extract::{Query, State, WebSocketUpgrade},
    extract::ws::Message as WsMessage,
    http::StatusCode,
    response::{Html, Json},
    routing::{get, post},
    Router,
};
use serde::Serialize;
use std::sync::Arc;
use std::time::SystemTime;

use crate::risk::RiskEngine;
use crate::metrics::Metrics;
use crate::state::{self, positions::PositionManager, sqlite::{RecentTradeRow, SignalLogEntry, SqliteStore}};
use rust_decimal::Decimal;
use tokio::sync::{broadcast, Mutex};

const DASHBOARD_HTML: &str = include_str!("dashboard_page.html");

#[derive(Clone)]
pub struct HealthState {
    pub start_time: SystemTime,
    pub simulation_mode: bool,
    pub paused: bool,
    pub metrics: Arc<Metrics>,
    pub sqlite_path: String,
    pub starting_balance: Decimal,
    pub risk_engine: Arc<RiskEngine>,
    pub position_manager: Arc<Mutex<PositionManager>>,
    pub event_tx: broadcast::Sender<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SignalsQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TradesQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ResumeQuery {
    pub confirm: Option<bool>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DailyStatsQuery {
    pub days: Option<usize>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DailyStatsEntry {
    pub date: String,
    pub realized_pnl: String,
    pub unrealized_pnl: String,
    pub volume_traded: String,
    pub trades_placed: u32,
    pub trades_filled: u32,
    pub trades_rejected: u32,
    pub drawdown_pct: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DailyStatsResponse {
    pub entries: Vec<DailyStatsEntry>,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub uptime_secs: u64,
    pub simulation: bool,
    pub ws_connected: bool,
    pub rpc_status: String,
    pub data_api_latency_ms: u64,
    pub last_signal_at: Option<String>,
    pub daily_pnl: String,
    pub balance_usd: String,
    pub drawdown_pct: String,
    pub paused: bool,
    pub open_positions: u64,
    pub signals_received: u64,
    pub signals_processed: u64,
    pub emergency_stops: u64,
}

#[derive(Serialize)]
pub struct ControlResponse {
    pub ok: bool,
    pub message: String,
}

pub async fn health_check(State(state): State<Arc<HealthState>>) -> Json<HealthResponse> {
    let metrics = &state.metrics;
    let uptime = state
        .start_time
        .elapsed()
        .unwrap_or(std::time::Duration::from_secs(0))
        .as_secs();

    let last_signal = metrics
        .last_signal_at
        .lock()
        .ok()
        .and_then(|guard| guard.clone());

    let ws_connected = metrics
        .ws_connected
        .load(std::sync::atomic::Ordering::Relaxed)
        == 1;
    let rpc_healthy = metrics
        .rpc_healthy
        .load(std::sync::atomic::Ordering::Relaxed)
        == 1;
    let rpc_status = if rpc_healthy { "healthy" } else { "unhealthy" }.to_string();
    let paused = metrics.is_paused() || state.paused;
    let (balance_usd, drawdown_pct) = match SqliteStore::open(std::path::Path::new(&state.sqlite_path)) {
        Ok(store) => {
            let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
            match store.get_daily_stats(&today).ok().flatten() {
                Some(stats) => {
                    let balance = stats.starting_balance + stats.realized_pnl + stats.unrealized_pnl;
                    (balance, stats.drawdown_pct * Decimal::new(100, 0))
                }
                None => (state.starting_balance, Decimal::ZERO),
            }
        }
        Err(_) => (state.starting_balance, Decimal::ZERO),
    };

    Json(HealthResponse {
        status: if paused {
            "paused".to_string()
        } else {
            "ok".to_string()
        },
        uptime_secs: uptime,
        simulation: state.simulation_mode,
        ws_connected,
        rpc_status,
        data_api_latency_ms: metrics
            .data_api_latency_ms
            .load(std::sync::atomic::Ordering::Relaxed),
        last_signal_at: last_signal,
        daily_pnl: format!("{:.2}", metrics.daily_pnl_usd()),
        balance_usd: format!("{:.2}", balance_usd),
        drawdown_pct: format!("{:.2}", drawdown_pct),
        paused,
        open_positions: metrics
            .open_positions
            .load(std::sync::atomic::Ordering::Relaxed),
        signals_received: metrics
            .signals_received
            .load(std::sync::atomic::Ordering::Relaxed),
        signals_processed: metrics
            .signals_processed
            .load(std::sync::atomic::Ordering::Relaxed),
        emergency_stops: metrics
            .emergency_stops_triggered
            .load(std::sync::atomic::Ordering::Relaxed),
    })
}

pub async fn metrics_handler(State(state): State<Arc<HealthState>>) -> String {
    let m = &state.metrics;
    let uptime = state
        .start_time
        .elapsed()
        .unwrap_or(std::time::Duration::from_secs(0))
        .as_secs();

    format!(
        "# HELP polybot_uptime_seconds Bot uptime\n# TYPE polybot_uptime_seconds gauge\npolybot_uptime_seconds {}\n\
         # HELP polybot_signals_received_total Total signals received\n# TYPE polybot_signals_received_total counter\npolybot_signals_received_total {}\n\
         # HELP polybot_signals_processed_total Signals processed by risk engine\n# TYPE polybot_signals_processed_total counter\npolybot_signals_processed_total {}\n\
         # HELP polybot_signals_skipped_total Signals skipped\n# TYPE polybot_signals_skipped_total counter\npolybot_signals_skipped_total {}\n\
         # HELP polybot_signals_manual_review_total Signals queued for manual review\n# TYPE polybot_signals_manual_review_total counter\npolybot_signals_manual_review_total {}\n\
         # HELP polybot_trades_executed_total Live trades executed\n# TYPE polybot_trades_executed_total counter\npolybot_trades_executed_total {}\n\
         # HELP polybot_trades_simulated_total Simulated trades\n# TYPE polybot_trades_simulated_total counter\npolybot_trades_simulated_total {}\n\
         # HELP polybot_trades_failed_total Failed trade attempts\n# TYPE polybot_trades_failed_total counter\npolybot_trades_failed_total {}\n\
         # HELP polybot_open_positions Current open positions\n# TYPE polybot_open_positions gauge\npolybot_open_positions {}\n\
         # HELP polybot_daily_pnl_usd Daily PnL in USD\n# TYPE polybot_daily_pnl_usd gauge\npolybot_daily_pnl_usd {:.2}\n\
         # HELP polybot_drawdown_pct Current drawdown percentage\n# TYPE polybot_drawdown_pct gauge\npolybot_drawdown_pct {:.4}\n\
         # HELP polybot_avg_latency_us Average execution latency in microseconds\n# TYPE polybot_avg_latency_us gauge\npolybot_avg_latency_us {}\n\
         # HELP polybot_max_latency_us Maximum execution latency in microseconds\n# TYPE polybot_max_latency_us gauge\npolybot_max_latency_us {}\n\
         # HELP polybot_emergency_stops_total Emergency stops triggered\n# TYPE polybot_emergency_stops_total counter\npolybot_emergency_stops_total {}\n\
         # HELP polybot_health Bot health (1=ok, 0=error)\n# TYPE polybot_health gauge\npolybot_health {}\n\
         # HELP polybot_ws_connected WebSocket connection (1=connected)\n# TYPE polybot_ws_connected gauge\npolybot_ws_connected {}\n\
         # HELP polybot_data_api_latency_ms Data API latency in milliseconds\n# TYPE polybot_data_api_latency_ms gauge\npolybot_data_api_latency_ms {}\n",
        uptime,
        m.signals_received.load(std::sync::atomic::Ordering::Relaxed),
        m.signals_processed.load(std::sync::atomic::Ordering::Relaxed),
        m.signals_skipped.load(std::sync::atomic::Ordering::Relaxed),
        m.signals_manual_review.load(std::sync::atomic::Ordering::Relaxed),
        m.trades_executed.load(std::sync::atomic::Ordering::Relaxed),
        m.trades_simulated.load(std::sync::atomic::Ordering::Relaxed),
        m.trades_failed.load(std::sync::atomic::Ordering::Relaxed),
        m.open_positions.load(std::sync::atomic::Ordering::Relaxed),
        m.daily_pnl_usd(),
        m.current_drawdown_pct(),
        m.avg_latency_us.load(std::sync::atomic::Ordering::Relaxed),
        m.max_latency_us.load(std::sync::atomic::Ordering::Relaxed),
        m.emergency_stops_triggered.load(std::sync::atomic::Ordering::Relaxed),
         if m.is_paused() || state.paused { 0 } else { 1 },
         m.ws_connected.load(std::sync::atomic::Ordering::Relaxed),
         m.data_api_latency_ms.load(std::sync::atomic::Ordering::Relaxed),
    )
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PositionResponse {
    pub id: String,
    pub market_id: String,
    pub market_name: Option<String>,
    pub side: String,
    pub entry_price: String,
    pub average_price: String,
    pub current_size: String,
    pub current_price: Option<String>,
    pub price_is_live: bool,
    pub opened_at: String,
    pub status: String,
    pub category: String,
}

async fn resolve_market_name(sqlite_path: &str, condition_id: &str) -> Option<String> {
    // Check SQLite cache first
    if let Ok(store) = SqliteStore::open(std::path::Path::new(sqlite_path)) {
        if let Ok(Some(meta)) = store.get_market_metadata(condition_id) {
            return meta.question;
        }
    }

    // Fetch from Polymarket Gamma API
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return None,
    };

    let url = format!(
        "https://gamma-api.polymarket.com/markets?limit=1&conditionIds={}",
        condition_id
    );

    match client.get(&url).send().await {
        Ok(resp) => {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(markets) = json.as_array() {
                    if let Some(market) = markets.first() {
                        // Validate API returned the CORRECT condition_id, not a fallback
                        let returned_condition_id = market.get("conditionId").and_then(|q| q.as_str());
                        if returned_condition_id != Some(condition_id) {
                            tracing::debug!(
                                requested = %condition_id,
                                returned = ?returned_condition_id,
                                "Gamma API returned fallback market; skipping cache"
                            );
                            return None;
                        }

                        let question = market.get("question").and_then(|q| q.as_str()).map(|s| s.to_string());
                        let slug = market.get("slug").and_then(|q| q.as_str()).map(|s| s.to_string());
                        let icon = market.get("icon").and_then(|q| q.as_str()).map(|s| s.to_string());
                        let resolved = market.get("resolved").and_then(|q| q.as_bool()).unwrap_or(false);

                        if let Some(ref q) = question {
                            if let Ok(store) = SqliteStore::open(std::path::Path::new(sqlite_path)) {
                                let _ = store.upsert_market_metadata(&crate::state::sqlite::MarketMetadataRow {
                                    condition_id: condition_id.to_string(),
                                    question: Some(q.clone()),
                                    slug,
                                    icon,
                                    resolved,
                                    fetched_at: chrono::Utc::now().to_rfc3339(),
                                });
                            }
                        }
                        return question;
                    }
                }
            }
        }
        Err(e) => {
            tracing::debug!(error = %e, condition_id, "Failed to fetch market metadata from Gamma API");
        }
    }
    None
}

pub async fn positions_handler(
    State(state): State<Arc<HealthState>>,
) -> Json<Vec<PositionResponse>> {
    match SqliteStore::open(std::path::Path::new(&state.sqlite_path)) {
        Ok(store) => match store.list_open_positions() {
            Ok(rows) => {
                let mut out = Vec::new();
                let path = state.sqlite_path.clone();
                for row in rows {
                    let market_name = resolve_market_name(&path, &row.position.market_id).await;
                    let has_live_price = row.current_price.is_some()
                        && !state.simulation_mode
                        && row.current_price.map(|p| p != Decimal::new(50, 2)).unwrap_or(false);
                    out.push(PositionResponse {
                        id: row.position.id,
                        market_id: row.position.market_id,
                        market_name,
                        side: format!("{:?}", row.position.side),
                        entry_price: row.position.entry_price.to_string(),
                        average_price: row.position.average_price.to_string(),
                        current_size: row.position.current_size.to_string(),
                        current_price: row.current_price.map(|v| v.to_string()),
                        price_is_live: has_live_price,
                        opened_at: row.position.opened_at.to_rfc3339(),
                        status: format!("{:?}", row.position.status),
                        category: row.position.category.to_string(),
                    });
                }
                Json(out)
            }
            _ => Json(Vec::new()),
        },
        Err(_) => Json(Vec::new()),
    }
}

pub async fn signals_handler(
    State(state): State<Arc<HealthState>>,
    Query(query): Query<SignalsQuery>,
) -> Json<Vec<SignalLogEntry>> {
    let limit = query.limit.unwrap_or(20);
    match SqliteStore::open(std::path::Path::new(&state.sqlite_path)) {
        Ok(store) => Json(store.latest_signals(limit).unwrap_or_default()),
        Err(_) => Json(Vec::new()),
    }
}

pub async fn executions_handler(
    State(state): State<Arc<HealthState>>,
    Query(query): Query<TradesQuery>,
) -> Json<Vec<RecentTradeRow>> {
    let limit = query.limit.unwrap_or(10);
    match SqliteStore::open(std::path::Path::new(&state.sqlite_path)) {
        Ok(store) => Json(store.latest_trades(limit).unwrap_or_default()),
        Err(_) => Json(Vec::new()),
    }
}

pub async fn daily_stats_handler(
    State(state): State<Arc<HealthState>>,
    Query(query): Query<DailyStatsQuery>,
) -> Json<DailyStatsResponse> {
    let days = query.days.unwrap_or(7);
    Json(match SqliteStore::open(std::path::Path::new(&state.sqlite_path)) {
        Ok(store) => {
            let rows = store.get_recent_daily_stats(days).unwrap_or_default();
            DailyStatsResponse {
                entries: rows.into_iter().map(|r| DailyStatsEntry {
                    date: r.date,
                    realized_pnl: r.realized_pnl.to_string(),
                    unrealized_pnl: r.unrealized_pnl.to_string(),
                    volume_traded: r.volume_traded.to_string(),
                    trades_placed: r.trades_placed,
                    trades_filled: r.trades_filled,
                    trades_rejected: r.trades_rejected,
                    drawdown_pct: r.drawdown_pct.to_string(),
                }).collect(),
            }
        }
        Err(_) => DailyStatsResponse { entries: Vec::new() },
    })
}

pub async fn pause_handler(
    State(state): State<Arc<HealthState>>,
) -> Result<Json<ControlResponse>, (StatusCode, Json<ControlResponse>)> {
    state.risk_engine.set_emergency_stop(true).await;
    state.metrics.set_paused(true);
    Ok(Json(ControlResponse { ok: true, message: "Trading paused.".to_string() }))
}

pub async fn resume_handler(
    State(state): State<Arc<HealthState>>,
    Query(query): Query<ResumeQuery>,
) -> Result<Json<ControlResponse>, (StatusCode, Json<ControlResponse>)> {
    if state.risk_engine.is_loss_cooldown_active().await && query.confirm != Some(true) {
        return Err((
            StatusCode::CONFLICT,
            Json(ControlResponse {
                ok: false,
                message: "Resume is blocked by active loss cooldown. Retry with ?confirm=true once you explicitly want to override it.".to_string(),
            }),
        ));
    }

    if state.risk_engine.resume_requires_confirmation().await && query.confirm != Some(true) {
        return Err((
            StatusCode::CONFLICT,
            Json(ControlResponse {
                ok: false,
                message: "Resume requires confirmation after a protection trigger. Retry with ?confirm=true".to_string(),
            }),
        ));
    }

    state.risk_engine.set_emergency_stop(false).await;
    state.risk_engine.clear_resume_confirmation().await;
    state.metrics.set_paused(false);
    Ok(Json(ControlResponse { ok: true, message: "Trading resumed.".to_string() }))
}

pub async fn emergency_stop_handler(
    State(state): State<Arc<HealthState>>,
) -> Result<Json<ControlResponse>, (StatusCode, Json<ControlResponse>)> {
    state.risk_engine.set_emergency_stop(true).await;
    state.metrics.record_emergency_stop();
    state.metrics.set_paused(true);
    let closed_positions = state::force_flatten_positions(
        state.metrics.clone(),
        state.position_manager.clone(),
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ControlResponse { ok: false, message: format!("Emergency stop failed: {}", e) }),
        )
    })?;

    Ok(Json(ControlResponse {
        ok: true,
        message: format!("Emergency stop applied. Closed {} positions.", closed_positions),
    }))
}

pub async fn dashboard_handler() -> Html<&'static str> {
    Html(DASHBOARD_HTML)
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<HealthState>>,
) -> axum::response::Response {
    ws.on_upgrade(move |mut socket| async move {
        let mut rx = state.event_tx.subscribe();
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if socket.send(WsMessage::Text(msg.into())).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    })
}

pub fn create_health_router(state: Arc<HealthState>) -> Router {
    Router::new()
        .route("/", get(dashboard_handler))
        .route("/dashboard", get(dashboard_handler))
        .route("/health", get(health_check))
        .route("/metrics", get(metrics_handler))
        .route("/positions", get(positions_handler))
        .route("/signals", get(signals_handler))
        .route("/executions", get(executions_handler))
        .route("/daily", get(daily_stats_handler))
        .route("/ws", get(ws_handler))
        .route("/control/pause", post(pause_handler))
        .route("/health/control/pause", post(pause_handler))
        .route("/control/resume", post(resume_handler))
        .route("/health/control/resume", post(resume_handler))
        .route("/control/emergency-stop", post(emergency_stop_handler))
        .route("/health/control/emergency-stop", post(emergency_stop_handler))
        .with_state(state)
}

pub async fn start_health_server(
    state: Arc<HealthState>,
    port: u16,
) -> Result<(), polybot_common::errors::PolybotError> {
    let app = create_health_router(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Health/metrics server starting on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        polybot_common::errors::PolybotError::Config(format!("Failed to bind health server: {}", e))
    })?;

    axum::serve(listener, app).await.map_err(|e| {
        polybot_common::errors::PolybotError::Config(format!("Health server error: {}", e))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::risk::RiskEngine;
    use crate::state::positions::PositionManager;
    use rust_decimal_macros::dec;
    use tokio::sync::Mutex;

    fn test_health_state(sqlite_path: String) -> Arc<HealthState> {
        let metrics = Arc::new(Metrics::new());
        let position_manager = Arc::new(Mutex::new(PositionManager::new()));
        let risk_engine = Arc::new(RiskEngine::new(
            Arc::new(AppConfig::default()),
            metrics.clone(),
            position_manager.clone(),
            None,
        ));

        Arc::new(HealthState {
            start_time: SystemTime::now(),
            simulation_mode: true,
            paused: false,
            metrics,
            sqlite_path,
            starting_balance: dec!(1000),
            risk_engine,
            position_manager,
            event_tx: tokio::sync::broadcast::channel(2).0,
        })
    }

    #[tokio::test]
    async fn health_check_includes_balance_and_drawdown_fields() {
        let sqlite_path = std::env::temp_dir().join(format!("polybot-health-{}.db", uuid::Uuid::new_v4()));
        let state = test_health_state(sqlite_path.to_string_lossy().to_string());

        let response = health_check(State(state)).await.0;
        assert_eq!(response.balance_usd, "1000.00");
        assert_eq!(response.drawdown_pct, "0.00");

        let _ = std::fs::remove_file(sqlite_path);
    }

    #[test]
    fn health_router_exposes_executions_and_control_routes() {
        let sqlite_path = std::env::temp_dir().join(format!("polybot-health-routes-{}.db", uuid::Uuid::new_v4()));
        let state = test_health_state(sqlite_path.to_string_lossy().to_string());
        let router = create_health_router(state);

        let dbg = format!("{:?}", router);
        assert!(dbg.contains("/executions"));
        assert!(dbg.contains("/control/pause"));
        assert!(dbg.contains("/control/resume"));
        assert!(dbg.contains("/control/emergency-stop"));
        assert!(dbg.contains("/health/control/pause"));
        assert!(dbg.contains("/health/control/resume"));
        assert!(dbg.contains("/health/control/emergency-stop"));

        let _ = std::fs::remove_file(sqlite_path);
    }

    #[tokio::test]
    async fn positions_handler_prefers_sqlite_positions_when_available() {
        let sqlite_path = std::env::temp_dir().join(format!("polybot-health-pos-{}.db", uuid::Uuid::new_v4()));
        let state = test_health_state(sqlite_path.to_string_lossy().to_string());
        let store = SqliteStore::open(&sqlite_path).unwrap();
        let position = polybot_common::types::Position {
            id: "pos-1".to_string(),
            market_id: "market-1".to_string(),
            side: polybot_common::types::Side::Yes,
            entry_price: dec!(0.55),
            current_size: dec!(10),
            average_price: dec!(0.55),
            opened_at: chrono::Utc::now(),
            status: polybot_common::types::PositionStatus::Open,
            category: polybot_common::types::Category::Politics,
        };
        store.upsert_position(&position, Some(dec!(0.60)), Some(dec!(0.5)), Some("0xabc")).unwrap();

        let positions = positions_handler(State(state)).await.0;
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].market_id, "market-1");

        let _ = std::fs::remove_file(sqlite_path);
    }

    #[tokio::test]
    async fn resume_handler_requires_explicit_confirmation_after_loss_breach() {
        let sqlite_path = std::env::temp_dir().join(format!("polybot-health-resume-{}.db", uuid::Uuid::new_v4()));
        let metrics = Arc::new(Metrics::new());
        let position_manager = Arc::new(Mutex::new(PositionManager::new()));
        let mut config = AppConfig::default();
        config.risk.max_consecutive_losses = 1;
        let risk_engine = Arc::new(RiskEngine::new(
            Arc::new(config),
            metrics.clone(),
            position_manager.clone(),
            None,
        ));
        risk_engine.record_realized_outcome(dec!(-1)).await;
        let state = Arc::new(HealthState {
            start_time: SystemTime::now(),
            simulation_mode: true,
            paused: false,
            metrics,
            sqlite_path: sqlite_path.to_string_lossy().to_string(),
            starting_balance: dec!(1000),
            risk_engine,
            position_manager,
            event_tx: tokio::sync::broadcast::channel(2).0,
        });

        let result = resume_handler(State(state), Query(ResumeQuery { confirm: None })).await;
        assert!(result.is_err());

        let _ = std::fs::remove_file(sqlite_path);
    }
}
