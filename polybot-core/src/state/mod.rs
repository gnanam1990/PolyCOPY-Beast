pub mod migrations;
pub mod pnl;
pub mod positions;
pub mod reconciliation;
pub mod sqlite;

use polybot_common::errors::PolybotError;
use polybot_common::types::{PositionKey, Trade, TradeStatus};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};

use crate::config::AppConfig;
use crate::metrics::Metrics;

pub async fn recover_from_sqlite(
    store: &sqlite::SqliteStore,
    metrics: Arc<Metrics>,
    position_manager: Arc<Mutex<positions::PositionManager>>,
) -> Result<(), PolybotError> {
    let rows = store.list_open_positions()?;
    let positions = rows.into_iter().map(|row| row.position).collect::<Vec<_>>();
    let count = positions.len() as u32;

    position_manager.lock().await.restore_positions(positions);
    metrics.set_open_positions(count);

    Ok(())
}

fn initial_starting_balance(config: &AppConfig) -> Decimal {
    if config.risk.base_size_pct > Decimal::ZERO {
        config.risk.base_size_usd / config.risk.base_size_pct
    } else {
        config.risk.base_size_usd
    }
}

fn update_daily_stats(
    store: &sqlite::SqliteStore,
    config: &AppConfig,
    metrics: &Metrics,
    trade: &Trade,
    unrealized: Decimal,
) -> Result<(), PolybotError> {
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut stats = store.get_daily_stats(&date)?.unwrap_or(sqlite::DailyStatsRow {
        date: date.clone(),
        starting_balance: initial_starting_balance(config),
        realized_pnl: Decimal::ZERO,
        unrealized_pnl: Decimal::ZERO,
        volume_traded: Decimal::ZERO,
        trades_placed: 0,
        trades_filled: 0,
        trades_rejected: 0,
        drawdown_pct: Decimal::ZERO,
        paused_at: None,
        notes: None,
    });

    stats.unrealized_pnl = unrealized;
    stats.volume_traded += trade.size_usd;
    stats.trades_placed += 1;
    match trade.status {
        TradeStatus::Filled | TradeStatus::PartiallyFilled => stats.trades_filled += 1,
        TradeStatus::Cancelled | TradeStatus::TimedOut | TradeStatus::Failed(_) => {
            stats.trades_rejected += 1
        }
        TradeStatus::Pending => {}
    }

    let current_value = stats.starting_balance + stats.realized_pnl + stats.unrealized_pnl;
    stats.drawdown_pct = pnl::calculate_drawdown_pct(current_value, stats.starting_balance);

    metrics.update_drawdown(stats.drawdown_pct.to_f64().unwrap_or(0.0));
    store.upsert_daily_stats(&stats)
}

pub async fn force_flatten_positions(
    metrics: Arc<Metrics>,
    position_manager: Arc<Mutex<positions::PositionManager>>,
) -> Result<usize, PolybotError> {
    let closed_positions = {
        let mut manager = position_manager.lock().await;
        manager.close_all_positions()
    };

    metrics.set_open_positions(0);
    for _ in 0..closed_positions.len() {
        metrics.total_positions_closed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    Ok(closed_positions.len())
}

pub async fn run_state_manager(
    config: Arc<AppConfig>,
    metrics: Arc<Metrics>,
    position_manager: Arc<Mutex<positions::PositionManager>>,
    market_prices: Arc<RwLock<HashMap<String, Decimal>>>,
    receiver: mpsc::Receiver<Trade>,
) -> Result<(), PolybotError> {
    let sqlite_path = std::env::var("POLYBOT_SQLITE_PATH")
        .unwrap_or_else(|_| "./polybot.db".to_string());
    let sqlite_path = sqlite::SqliteStore::open(std::path::Path::new(&sqlite_path))
        .map(|_| sqlite_path)
        .ok();

    run_in_memory(receiver, metrics, position_manager, market_prices, sqlite_path.as_deref(), &config).await
}

async fn run_in_memory(
    mut receiver: mpsc::Receiver<Trade>,
    metrics: Arc<Metrics>,
    position_manager: Arc<Mutex<positions::PositionManager>>,
    market_prices: Arc<RwLock<HashMap<String, Decimal>>>,
    sqlite_path: Option<&str>,
    config: &AppConfig,
) -> Result<(), PolybotError> {
    while let Some(trade) = receiver.recv().await {
        tracing::info!(
            trade_id = %trade.id,
            signal_id = %trade.signal_id,
            status = ?trade.status,
            simulated = trade.simulated,
            "Processing trade (in-memory)"
        );

        if let Some(sqlite_path) = sqlite_path {
            match sqlite::SqliteStore::open(std::path::Path::new(sqlite_path)) {
                Ok(store) => {
                    if let Err(e) = store.insert_trade(&trade) {
                        tracing::error!(error = %e, "Failed to persist trade to SQLite");
                    }
                }
                Err(e) => tracing::error!(error = %e, "Failed to open SQLite for trade persistence"),
            }
        }

        let sqlite_store = sqlite_path.and_then(|path| sqlite::SqliteStore::open(std::path::Path::new(path)).ok());

        let (position_snapshot, current_price, open_positions, unrealized) = {
            let mut position_manager = position_manager.lock().await;
            if let Err(e) = position_manager.update_from_trade(&trade) {
                tracing::error!(error = %e, "Failed to update position from trade");
            }

            let current_prices_in_memory = market_prices.read().await.clone();
            (
                position_manager
                    .get_position(&PositionKey::new(trade.market_id.clone(), trade.side))
                    .cloned(),
                current_prices_in_memory.get(&trade.market_id).copied(),
                position_manager.open_position_count(),
                pnl::calculate_unrealized_pnl(&position_manager, &current_prices_in_memory),
            )
        };

        if let (Some(store), Some(pos)) = (sqlite_store.as_ref(), position_snapshot.as_ref()) {
            let owner = match store.lookup_signal_wallet(&trade.signal_id) {
                Ok(owner) => owner,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to lookup signal owner for SQLite position persistence");
                    None
                }
            };
            if let Err(e) = store.upsert_position(pos, current_price, Some(unrealized), owner.as_deref()) {
                tracing::error!(error = %e, "Failed to persist position to SQLite");
            }
            if let Err(e) = update_daily_stats(store, config, &metrics, &trade, unrealized) {
                tracing::error!(error = %e, "Failed to update SQLite daily stats");
            }
        }

        metrics.set_open_positions(open_positions);
        metrics.update_daily_pnl(unrealized.to_f64().unwrap_or(0.0));
        tracing::info!(unrealized_pnl = %unrealized, "Unrealized PnL updated");
    }

    tracing::info!("State manager shutting down (in-memory)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use polybot_common::types::{Category, OrderDirection, OrderType, Side};

    fn test_trade(market_id: &str, side: Side, category: Category) -> Trade {
        Trade {
            id: format!("trade-{market_id}-{side:?}"),
            signal_id: "signal-1".to_string(),
            market_id: market_id.to_string(),
            category,
            side,
            direction: OrderDirection::Buy,
            price: Decimal::new(50, 2),
            size: Decimal::new(10, 0),
            size_usd: Decimal::new(500, 2),
            filled_size: Decimal::new(10, 0),
            order_type: OrderType::Limit,
            status: TradeStatus::Filled,
            placed_at: Utc::now(),
            filled_at: Some(Utc::now()),
            simulated: true,
            transaction_id: None,
            transaction_hash: None,
            relayer_state: None,
            taker_fee_bps: 0,
            fee_paid_usdc: Decimal::ZERO,
            rebate_usdc: Decimal::ZERO,
            retry_count: 0,
            error_msg: None,
        }
    }

    #[tokio::test]
    async fn force_flatten_positions_clears_positions_and_metrics() {
        let metrics = Arc::new(Metrics::new());
        let position_manager = Arc::new(Mutex::new(positions::PositionManager::new()));
        {
            let mut manager = position_manager.lock().await;
            manager
                .update_from_trade(&test_trade("m1", Side::Yes, Category::Politics))
                .unwrap();
            manager
                .update_from_trade(&test_trade("m2", Side::No, Category::Crypto))
                .unwrap();
        }
        metrics.set_open_positions(2);

        let closed = force_flatten_positions(metrics.clone(), position_manager.clone())
            .await
            .unwrap();

        assert_eq!(closed, 2);
        assert_eq!(metrics.open_positions.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(
            metrics
                .total_positions_closed
                .load(std::sync::atomic::Ordering::Relaxed),
            2
        );
        assert_eq!(position_manager.lock().await.open_position_count(), 0);
    }

    #[tokio::test]
    async fn recover_from_sqlite_restores_positions_and_metrics() {
        let store = sqlite::SqliteStore::open_in_memory().unwrap();
        let position = polybot_common::types::Position {
            id: "pos-1".to_string(),
            market_id: "market-1".to_string(),
            side: Side::Yes,
            entry_price: Decimal::new(60, 2),
            current_size: Decimal::new(10, 0),
            average_price: Decimal::new(60, 2),
            opened_at: Utc::now(),
            status: polybot_common::types::PositionStatus::Open,
            category: Category::Politics,
        };
        store
            .upsert_position(&position, Some(Decimal::new(65, 2)), Some(Decimal::new(5, 0)), Some("0xabc"))
            .unwrap();

        let metrics = Arc::new(Metrics::new());
        let position_manager = Arc::new(Mutex::new(positions::PositionManager::new()));

        recover_from_sqlite(&store, metrics.clone(), position_manager.clone())
            .await
            .unwrap();

        assert_eq!(position_manager.lock().await.open_position_count(), 1);
        assert_eq!(metrics.open_positions.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn run_in_memory_persists_positions_and_daily_stats_to_sqlite() {
        let sqlite_path = std::env::temp_dir().join(format!("polybot-state-{}.db", uuid::Uuid::new_v4()));
        std::env::set_var("POLYBOT_SQLITE_PATH", &sqlite_path);

        let store = sqlite::SqliteStore::open(&sqlite_path).unwrap();
        let timestamp = Utc::now().to_rfc3339();
        store.insert_signal_log(&sqlite::SignalLogInsert {
            signal_id: "signal-1",
            timestamp: &timestamp,
            wallet_address: "0xabc123abc123abc123abc123abc123abc123abc1",
            market_id: "m1",
            confidence: 7,
            secret_level: 7,
            category: "politics",
            side: "YES",
            disposition: "execute",
        }).unwrap();

        let metrics = Arc::new(Metrics::new());
        let position_manager = Arc::new(Mutex::new(positions::PositionManager::new()));
        let market_prices = Arc::new(RwLock::new(HashMap::from([("m1".to_string(), Decimal::new(70, 2))])));
        let (tx, rx) = mpsc::channel(4);
        let config = AppConfig::default();

        tx.send(test_trade("m1", Side::Yes, Category::Politics)).await.unwrap();
        drop(tx);

        run_in_memory(
            rx,
            metrics,
            position_manager,
            market_prices,
            Some(sqlite_path.to_string_lossy().as_ref()),
            &config,
        )
            .await
            .unwrap();

        let reopened = sqlite::SqliteStore::open(&sqlite_path).unwrap();
        let positions = reopened.list_open_positions().unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].owned_by_wallet.as_deref(), Some("0xabc123abc123abc123abc123abc123abc123abc1"));

        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let stats = reopened.get_daily_stats(&today).unwrap().unwrap();
        assert_eq!(stats.trades_placed, 1);
        assert_eq!(stats.trades_filled, 1);
        assert_eq!(stats.volume_traded, Decimal::new(500, 2));

        let _ = std::fs::remove_file(sqlite_path);
        std::env::remove_var("POLYBOT_SQLITE_PATH");
    }
}
