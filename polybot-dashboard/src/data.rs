use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthData {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsData {
    pub signals_received: u64,
    pub signals_processed: u64,
    pub signals_skipped: u64,
    pub trades_executed: u64,
    pub current_drawdown_pct: f64,
    pub open_positions: u32,
    pub daily_pnl_usd: f64,
    pub avg_latency_us: u64,
    pub max_latency_us: u64,
    pub data_api_latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionData {
    pub id: String,
    pub market_id: String,
    pub market_name: Option<String>,
    pub side: String,
    pub entry_price: String,
    pub average_price: String,
    pub current_price: Option<String>,
    pub price_is_live: bool,
    pub current_size: String,
    pub category: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalData {
    pub signal_id: String,
    pub wallet_address: String,
    pub secret_level: u8,
    pub confidence: u8,
    pub category: String,
    pub disposition: String,
    pub market_id: String,
    pub side: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyStatsData {
    pub entries: Vec<DailyStatsEntry>,
}

fn api_path(path: &str) -> String {
    path.to_string()
}

pub fn market_link(market_id: &str) -> (String, String) {
    let display = if market_id.len() > 12 {
        format!(
            "{}...{}",
            &market_id[..6],
            &market_id[market_id.len() - 4..]
        )
    } else {
        market_id.to_string()
    };
    let url = format!("https://polymarket.com/market/{}", market_id);
    (display, url)
}

pub async fn fetch_health() -> Result<HealthData, String> {
    let url = api_path("/health");
    gloo_net::http::Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("Health fetch error: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Health parse error: {}", e))
}

pub async fn fetch_metrics() -> Result<MetricsData, String> {
    let url = api_path("/metrics");
    let resp = gloo_net::http::Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("Metrics fetch error: {}", e))?;
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Metrics read error: {}", e))?;
    // Parse Prometheus-style metrics
    let mut data = MetricsData {
        signals_received: 0,
        signals_processed: 0,
        signals_skipped: 0,
        trades_executed: 0,
        current_drawdown_pct: 0.0,
        open_positions: 0,
        daily_pnl_usd: 0.0,
        avg_latency_us: 0,
        max_latency_us: 0,
        data_api_latency_ms: 0,
    };
    for line in text.lines() {
        if line.starts_with("polybot_signals_received_total ") {
            data.signals_received = line
                .split_whitespace()
                .last()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
        } else if line.starts_with("polybot_signals_processed_total ") {
            data.signals_processed = line
                .split_whitespace()
                .last()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
        } else if line.starts_with("polybot_signals_skipped_total ") {
            data.signals_skipped = line
                .split_whitespace()
                .last()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
        } else if line.starts_with("polybot_trades_executed_total ") {
            data.trades_executed = line
                .split_whitespace()
                .last()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
        } else if line.starts_with("polybot_open_positions ") {
            data.open_positions = line
                .split_whitespace()
                .last()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
        } else if line.starts_with("polybot_daily_pnl_usd ") {
            data.daily_pnl_usd = line
                .split_whitespace()
                .last()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0.0);
        } else if line.starts_with("polybot_drawdown_pct ") {
            data.current_drawdown_pct = line
                .split_whitespace()
                .last()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0.0);
        } else if line.starts_with("polybot_avg_latency_us ") {
            data.avg_latency_us = line
                .split_whitespace()
                .last()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
        } else if line.starts_with("polybot_max_latency_us ") {
            data.max_latency_us = line
                .split_whitespace()
                .last()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
        } else if line.starts_with("polybot_data_api_latency_ms ") {
            data.data_api_latency_ms = line
                .split_whitespace()
                .last()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
        }
    }
    Ok(data)
}

pub async fn fetch_positions() -> Result<Vec<PositionData>, String> {
    let url = api_path("/positions");
    gloo_net::http::Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("Positions fetch error: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Positions parse error: {}", e))
}

pub async fn fetch_signals(limit: usize) -> Result<Vec<SignalData>, String> {
    let url = format!("{}?limit={}", api_path("/signals"), limit);
    gloo_net::http::Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("Signals fetch error: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Signals parse error: {}", e))
}

pub async fn fetch_daily_stats() -> Result<Vec<DailyStatsEntry>, String> {
    gloo_net::http::Request::get(&api_path("/daily"))
        .send()
        .await
        .map_err(|e| format!("Daily stats fetch error: {}", e))?
        .json::<DailyStatsData>()
        .await
        .map_err(|e| format!("Daily stats parse error: {}", e))
        .map(|d| d.entries)
}
