pub mod clob_client;
pub mod clob_ws;
pub mod order_builder;
pub mod rate_limiter;
pub mod retry;
pub mod transport;
pub mod v2_client;
pub mod v2_collateral;
pub mod v2_flow;
pub mod v2_market;
pub mod v2_order;
pub mod v2_relayer;
pub mod v2_signing;
pub mod v2_sim;

use polybot_common::constants::MIN_POSITION_USDC;
use polybot_common::errors::PolybotError;
use polybot_common::types::{Decision, ExecutionMode, OrderType, RiskDecision, Trade, TradeStatus};
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, RwLock};

use crate::config::AppConfig;
use crate::metrics::Metrics;
use crate::risk::limits;
use crate::telegram_bot::alerts::AlertBroadcaster;
use transport::select_transport_mode;

pub(crate) fn live_v2_submission_config(
    config: &AppConfig,
) -> Result<(&crate::config::RelayerConfig, &str), PolybotError> {
    let relayer = config.relayer.as_ref().ok_or_else(|| {
        PolybotError::Config(
            "Live CLOB V2 requires RELAYER_URL, RELAYER_API_KEY, and RELAYER_API_KEY_ADDRESS; V1 fallback is disabled.".to_string(),
        )
    })?;
    let builder = config.builder.as_ref().ok_or_else(|| {
        PolybotError::Config(
            "Live CLOB V2 requires BUILDER_CODE; V1 fallback is disabled.".to_string(),
        )
    })?;

    if relayer.url.trim().is_empty()
        || relayer.api_key.trim().is_empty()
        || relayer.api_key_address.trim().is_empty()
    {
        return Err(PolybotError::Config(
            "Live CLOB V2 relayer config cannot contain empty values.".to_string(),
        ));
    }

    v2_signing::parse_builder_code(&builder.code).map_err(|err| {
        PolybotError::Config(format!("Invalid BUILDER_CODE for live CLOB V2: {}", err))
    })?;

    Ok((relayer, builder.code.as_str()))
}

pub async fn cancel_open_orders_on_shutdown(config: Arc<AppConfig>) -> Result<(), PolybotError> {
    if !config.system.execution_mode.allows_live_order_submission() {
        return Ok(());
    }

    let client = clob_client::ClobClient::from_env()?;
    client.cancel_all_orders().await
}

fn simulation_transaction_id(signal_id: &str) -> String {
    format!("sim-{}", signal_id)
}

pub async fn run_execution_engine(
    config: Arc<AppConfig>,
    metrics: Arc<Metrics>,
    alerts: Option<AlertBroadcaster>,
    market_prices: Arc<RwLock<HashMap<String, Decimal>>>,
    mut receiver: mpsc::Receiver<RiskDecision>,
    state_sender: mpsc::Sender<Trade>,
) -> Result<(), PolybotError> {
    let execution_mode = config.system.execution_mode;
    let transport_plan = select_transport_mode(execution_mode);
    tracing::info!(
        execution_mode = execution_mode.as_str(),
        uses_market_data = transport_plan.uses_market_data,
        uses_ws_market_data = transport_plan.uses_ws_market_data,
        submits_orders = transport_plan.submits_orders,
        "Execution transport selected"
    );

    let retry_policy = retry::RetryPolicy::default();
    let live_v2_submission = if transport_plan.submits_orders {
        Some(live_v2_submission_config(config.as_ref())?)
    } else {
        None
    };

    let market_data_client = transport_plan
        .uses_market_data
        .then(clob_client::ClobClient::public_readonly);

    let ws_manager = if transport_plan.uses_ws_market_data {
        let ws_manager = Arc::new(clob_ws::ClobWsManager::new(
            clob_client::ClobConfig {
                endpoint: std::env::var("POLYBOT_CLOB_ENDPOINT")
                    .unwrap_or_else(|_| "https://clob.polymarket.com".to_string()),
                ws_endpoint: std::env::var("POLYBOT_WS_ENDPOINT")
                    .unwrap_or_else(|_| "wss://ws-subscriptions-clob.polymarket.com".to_string()),
                chain_id: 137,
                private_key: String::new(),
                api_key: None,
                signature_type: 0,
                funder_address: None,
            },
            metrics.clone(),
            alerts.clone(),
        ));

        let ws_task = ws_manager.clone();
        tokio::spawn(async move {
            if let Err(e) = ws_task.connect_with_backoff().await {
                tracing::error!(error = %e, "CLOB WebSocket manager stopped");
            }
        });

        Some(ws_manager)
    } else {
        tracing::info!("Skipping CLOB market-data WebSocket in simulation mode");
        None
    };

    let clob_client = if transport_plan.submits_orders {
        match clob_client::ClobClient::from_env() {
            Ok(client) => {
                tracing::info!("CLOB client initialized for live order submission");
                metrics.set_rpc_healthy(true);
                Some(client)
            }
            Err(e) => {
                metrics.set_rpc_healthy(false);
                tracing::error!(
                    error = %e,
                    "Failed to initialize CLOB client for live submission"
                );
                None
            }
        }
    } else {
        tracing::info!("CLOB submission client skipped for non-live execution mode");
        None
    };

    while let Some(decision) = receiver.recv().await {
        match decision.decision {
            Decision::Execute => {
                tracing::info!(
                    signal_id = %decision.signal_id,
                    market_id = %decision.market_id,
                    side = ?decision.side,
                    size_usd = %decision.position_size_usd,
                    "Executing trade"
                );

                let mut target_price = config.paper.fixed_entry_price;
                let mut size_usd = decision.position_size_usd;
                let mut market_context =
                    clob_client::MarketContext::simulation(decision.market_id.clone());

                if let Some(market_data_client) = market_data_client.as_ref() {
                    match market_data_client
                        .get_market_context_for_signal(&decision.market_id, decision.side)
                        .await
                    {
                        Ok(context) => {
                            market_context = context;

                            if let Some(ws_manager) = ws_manager.as_ref() {
                                ws_manager
                                    .subscribe_token(market_context.token_id.clone())
                                    .await;
                            }

                            let cached_book = if let Some(ws_manager) = ws_manager.as_ref() {
                                ws_manager
                                    .get_cached_orderbook(&market_context.token_id)
                                    .await
                            } else {
                                None
                            };

                            let book = match cached_book {
                                Some(book) => book,
                                None => {
                                    match market_data_client
                                        .get_orderbook(&market_context.token_id)
                                        .await
                                    {
                                        Ok(book) => book,
                                        Err(e) => {
                                            metrics.record_trade_failed();
                                            if let Some(alerts) = &alerts {
                                                alerts.critical(format!(
                                                    "Warm-book fallback failed for signal {} token {}: {}",
                                                    decision.signal_id, market_context.token_id, e
                                                ));
                                            }
                                            tracing::error!(
                                                error = %e,
                                                signal_id = %decision.signal_id,
                                                token_id = %market_context.token_id,
                                                "Warm-book fallback orderbook fetch failed"
                                            );
                                            continue;
                                        }
                                    }
                                }
                            };

                            let (midpoint, has_real_price) =
                                match clob_client::ClobClient::calculate_midpoint(&book) {
                                    Some(mp) => (mp, true),
                                    None => (target_price, false),
                                };
                            let estimated_fill =
                                clob_client::ClobClient::estimate_fill_price(&book)
                                    .unwrap_or(midpoint);
                            if has_real_price {
                                market_prices
                                    .write()
                                    .await
                                    .insert(decision.market_id.clone(), midpoint);
                            }

                            if !clob_client::ClobClient::check_slippage(
                                midpoint,
                                estimated_fill,
                                config.execution.slippage_threshold,
                            ) {
                                metrics.record_trade_failed();
                                if let Some(alerts) = &alerts {
                                    alerts.warning(format!(
                                        "Trade rejected by slippage guard for signal {} market {}",
                                        decision.signal_id, decision.market_id
                                    ));
                                }
                                tracing::warn!(
                                    signal_id = %decision.signal_id,
                                    market_id = %decision.market_id,
                                    midpoint = %midpoint,
                                    estimated_fill = %estimated_fill,
                                    "Trade rejected by slippage guard"
                                );
                                continue;
                            }

                            let visible_liquidity =
                                clob_client::ClobClient::visible_liquidity_usd(&book);
                            size_usd =
                                limits::apply_market_liquidity_cap(size_usd, visible_liquidity);
                            target_price = estimated_fill;
                        }
                        Err(e) => {
                            metrics.record_trade_failed();
                            if let Some(alerts) = &alerts {
                                alerts.critical(format!(
                                    "Orderbook fetch failed for signal {} market {}: {}",
                                    decision.signal_id, decision.market_id, e
                                ));
                            }
                            tracing::error!(
                                error = %e,
                                signal_id = %decision.signal_id,
                                market_id = %decision.market_id,
                                "Orderbook fetch failed"
                            );
                            continue;
                        }
                    }
                } else {
                    market_prices
                        .write()
                        .await
                        .insert(decision.market_id.clone(), target_price);
                }

                if size_usd < MIN_POSITION_USDC {
                    metrics.record_trade_failed();
                    if let Some(alerts) = &alerts {
                        alerts.warning(format!(
                            "Trade rejected after liquidity cap for signal {} market {}",
                            decision.signal_id, decision.market_id
                        ));
                    }
                    tracing::warn!(
                        signal_id = %decision.signal_id,
                        market_id = %decision.market_id,
                        capped_size_usd = %size_usd,
                        "Trade rejected after liquidity cap fell below minimum position size"
                    );
                    continue;
                }

                let effective_price_buffer = if execution_mode == ExecutionMode::Simulation {
                    Decimal::ZERO
                } else {
                    config.execution.price_buffer
                };
                let order = order_builder::build_order_with_price_buffer(
                    &decision,
                    &market_context,
                    target_price,
                    size_usd,
                    effective_price_buffer,
                    OrderType::Fok,
                );

                match execution_mode {
                    ExecutionMode::Simulation => {
                        tracing::info!(
                            signal_id = %decision.signal_id,
                            "Simulation mode: creating simulated trade"
                        );
                        let outcome = v2_flow::simulate_with_relayer(
                            &order,
                            v2_sim::SimulatedRelayer::success(simulation_transaction_id(
                                &decision.signal_id,
                            )),
                        )?;
                        let trade = outcome.trade;
                        metrics.broadcast_event(
                            "relayer_update",
                            serde_json::json!({
                                "transaction_id": outcome.transaction.transaction_id,
                                "state": outcome.transaction.state.as_sqlite_str(),
                                "mode": "simulation",
                            }),
                        );
                        metrics.record_trade(true);
                        metrics.broadcast_event(
                            "trade_placed",
                            serde_json::json!({
                                "signal_id": &decision.signal_id,
                                "market_id": &decision.market_id,
                                "size_usd": trade.size_usd.to_string(),
                                "price": trade.price.to_string(),
                                "mode": "simulation",
                            }),
                        );
                        if let Some(alerts) = &alerts {
                            alerts.info(format!(
                                "Trade executed in simulation: signal={} market={} size_usd={} price={}",
                                decision.signal_id, decision.market_id, trade.size_usd, trade.price
                            ));
                        }
                        if state_sender.send(trade).await.is_err() {
                            tracing::error!("State channel closed");
                            return Err(PolybotError::ChannelClosed);
                        }
                    }
                    ExecutionMode::Shadow => {
                        tracing::info!(
                            signal_id = %decision.signal_id,
                            market_id = %decision.market_id,
                            "Shadow mode: order planned, submission skipped"
                        );
                    }
                    ExecutionMode::Live => {
                        let Some(client) = clob_client.as_ref() else {
                            metrics.record_trade_failed();
                            tracing::error!(
                                signal_id = %decision.signal_id,
                                market_id = %decision.market_id,
                                "Live execution requested but submission client is unavailable"
                            );
                            continue;
                        };

                        let started = Instant::now();
                        let mut attempt = 0u32;
                        let (relayer, builder_code) = live_v2_submission.ok_or_else(|| {
                            PolybotError::Config(
                                "Live CLOB V2 submission config was not initialized.".to_string(),
                            )
                        })?;
                        loop {
                            let submit_result =
                                client.submit_order_v2(&order, relayer, builder_code).await;
                            match submit_result {
                                Ok(trade) => {
                                    if matches!(trade.status, TradeStatus::PartiallyFilled) {
                                        tracing::info!(
                                            signal_id = %decision.signal_id,
                                            market_id = %decision.market_id,
                                            filled_size = %trade.filled_size,
                                            requested_size = %trade.size,
                                            "Partial fill received; forwarding filled amount to state without retrying remainder"
                                        );
                                    }
                                    if matches!(trade.status, TradeStatus::Pending)
                                        && trade.transaction_id.is_some()
                                    {
                                        tracing::info!(
                                            signal_id = %decision.signal_id,
                                            market_id = %decision.market_id,
                                            transaction_id = %trade.transaction_id.clone().unwrap_or_default(),
                                            relayer_state = ?trade.relayer_state,
                                            "Relayer accepted order; tracking async transaction lifecycle"
                                        );
                                    }
                                    metrics.record_latency(started.elapsed().as_micros() as u64);
                                    if !matches!(trade.status, TradeStatus::Pending) {
                                        metrics.record_trade(false);
                                    }
                                    metrics.broadcast_event("trade_placed", serde_json::json!({
                                        "signal_id": &decision.signal_id,
                                        "market_id": &decision.market_id,
                                        "size_usd": trade.size_usd.to_string(),
                                        "price": trade.price.to_string(),
                                        "mode": "live",
                                        "transaction_id": &trade.transaction_id,
                                        "relayer_state": trade.relayer_state.map(|s| s.as_sqlite_str().to_string()),
                                    }));
                                    if let Some(alerts) = &alerts {
                                        alerts.info(format!(
                                            "Live order submitted: signal={} market={} size_usd={} price={} txid={}",
                                            decision.signal_id,
                                            decision.market_id,
                                            trade.size_usd,
                                            trade.price,
                                            trade.transaction_id.clone().unwrap_or_default()
                                        ));
                                    }
                                    if state_sender.send(trade).await.is_err() {
                                        tracing::error!("State channel closed");
                                        return Err(PolybotError::ChannelClosed);
                                    }
                                    break;
                                }
                                Err(err) => {
                                    let retry_class = err.retry_class();
                                    if retry_policy.should_retry(attempt, retry_class) {
                                        let delay = retry_policy.backoff_delay(attempt);
                                        tracing::warn!(
                                            signal_id = %decision.signal_id,
                                            market_id = %decision.market_id,
                                            attempt = attempt + 1,
                                            delay_ms = delay.as_millis(),
                                            retry_class = ?retry_class,
                                            error = %err,
                                            "Retryable order submission failure, backing off"
                                        );
                                        attempt += 1;
                                        tokio::time::sleep(delay).await;
                                        continue;
                                    }

                                    let error = err.into_polybot();
                                    metrics.record_trade_failed();
                                    if let Some(alerts) = &alerts {
                                        alerts.critical(format!(
                                            "Order submission failed for signal {} market {}: {}",
                                            decision.signal_id, decision.market_id, error
                                        ));
                                    }
                                    tracing::error!(error = %error, "Order submission failed");
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            Decision::Skip(_reason) => {
                metrics.record_signal_skipped();
            }
            Decision::ManualReview => {
                metrics.record_signal_manual_review();
            }
            Decision::EmergencyStop => {
                metrics.record_emergency_stop();
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use polybot_common::types::ExecutionMode;

    use crate::config::{AppConfig, BuilderConfig, RelayerConfig};

    fn valid_builder_code() -> String {
        format!("0x{}", "00".repeat(32))
    }

    fn valid_relayer_config() -> RelayerConfig {
        RelayerConfig {
            url: "https://relayer-v2.polymarket.com".to_string(),
            api_key: "test-relayer-key".to_string(),
            api_key_address: "0x1234567890123456789012345678901234567890".to_string(),
        }
    }

    #[test]
    fn simulation_mode_uses_fully_offline_transport() {
        let plan = super::transport::select_transport_mode(ExecutionMode::Simulation);
        assert!(!plan.uses_market_data);
        assert!(!plan.uses_ws_market_data);
        assert!(!plan.submits_orders);
    }

    #[test]
    fn shadow_mode_keeps_read_only_market_access() {
        let plan = super::transport::select_transport_mode(ExecutionMode::Shadow);
        assert!(plan.uses_market_data);
        assert!(plan.uses_ws_market_data);
        assert!(!plan.submits_orders);
    }

    #[tokio::test]
    async fn shutdown_cancel_skips_non_live_mode() {
        let config = crate::config::AppConfig::default();
        assert!(
            super::cancel_open_orders_on_shutdown(std::sync::Arc::new(config))
                .await
                .is_ok()
        );
    }

    #[test]
    fn simulation_transaction_id_is_stable_for_signal() {
        assert_eq!(super::simulation_transaction_id("abc"), "sim-abc");
    }

    #[test]
    fn live_v2_submission_config_requires_relayer() {
        let mut config = AppConfig {
            builder: Some(BuilderConfig {
                code: valid_builder_code(),
            }),
            ..AppConfig::default()
        };
        config.relayer = None;

        let err = super::live_v2_submission_config(&config).unwrap_err();
        assert!(err.to_string().contains("RELAYER_URL"));
    }

    #[test]
    fn live_v2_submission_config_requires_builder() {
        let mut config = AppConfig {
            relayer: Some(valid_relayer_config()),
            ..AppConfig::default()
        };
        config.builder = None;

        let err = super::live_v2_submission_config(&config).unwrap_err();
        assert!(err.to_string().contains("BUILDER_CODE"));
    }

    #[test]
    fn live_v2_submission_config_rejects_invalid_builder_code() {
        let config = AppConfig {
            relayer: Some(valid_relayer_config()),
            builder: Some(BuilderConfig {
                code: "0xdeadbeef".to_string(),
            }),
            ..AppConfig::default()
        };

        let err = super::live_v2_submission_config(&config).unwrap_err();
        assert!(err.to_string().contains("Invalid BUILDER_CODE"));
    }

    #[test]
    fn live_v2_submission_config_accepts_complete_v2_config() {
        let config = AppConfig {
            relayer: Some(valid_relayer_config()),
            builder: Some(BuilderConfig {
                code: valid_builder_code(),
            }),
            ..AppConfig::default()
        };

        let (relayer, builder_code) = super::live_v2_submission_config(&config).unwrap();
        assert_eq!(relayer.url, "https://relayer-v2.polymarket.com");
        assert_eq!(builder_code, valid_builder_code());
    }
}
