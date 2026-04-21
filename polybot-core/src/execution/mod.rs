pub mod clob_client;
pub mod clob_ws;
pub mod order_builder;
pub mod rate_limiter;
pub mod retry;
pub mod rpc_pool;
pub mod transport;

use polybot_common::constants::MIN_POSITION_USDC;
use polybot_common::errors::PolybotError;
use polybot_common::types::{Decision, ExecutionMode, OrderType, RiskDecision, Trade};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, RwLock};

use crate::config::AppConfig;
use crate::metrics::Metrics;
use crate::risk::limits;
use crate::telegram_bot::alerts::AlertBroadcaster;
use transport::select_transport_mode;

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

    let _rpc_pool = rpc_pool::RpcPool::new(&config.execution.rpc_endpoints);
    let retry_policy = retry::RetryPolicy::default();
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

                let mut target_price = dec!(0.50);
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
                                ws_manager.get_cached_orderbook(&market_context.token_id).await
                            } else {
                                None
                            };

                            let book = match cached_book {
                                Some(book) => book,
                                None => {
                                    match market_data_client.get_orderbook(&market_context.token_id).await {
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

                            let midpoint =
                                clob_client::ClobClient::calculate_midpoint(&book).unwrap_or(target_price);
                            let estimated_fill =
                                clob_client::ClobClient::estimate_fill_price(&book).unwrap_or(midpoint);
                            market_prices
                                .write()
                                .await
                                .insert(decision.market_id.clone(), midpoint);

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

                let order = order_builder::build_order_with_price_buffer(
                    &decision,
                    &market_context,
                    target_price,
                    size_usd,
                    config.execution.price_buffer,
                    OrderType::Fok,
                );

                match execution_mode {
                    ExecutionMode::Simulation => {
                        tracing::info!(
                            signal_id = %decision.signal_id,
                            "Simulation mode: creating simulated trade"
                        );
                        let trade = order_builder::create_simulated_trade(&decision, &order);
                        metrics.record_trade(true);
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
                        loop {
                            match client.submit_order(&order).await {
                                Ok(trade) => {
                                    metrics.record_latency(started.elapsed().as_micros() as u64);
                                    metrics.record_trade(false);
                                    if let Some(alerts) = &alerts {
                                        alerts.info(format!(
                                            "Live trade executed: signal={} market={} size_usd={} price={}",
                                            decision.signal_id, decision.market_id, trade.size_usd, trade.price
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
}
