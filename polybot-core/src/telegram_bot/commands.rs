use std::sync::atomic::Ordering;
use teloxide::prelude::*;

use super::alerts;
use super::auth::AuthService;
use super::confirm::{ConfirmAction, ConfirmState};
use super::rate_limiter::CommandRateLimiter;
use super::Command;
use crate::config::AppConfig;
use crate::metrics::Metrics;
use crate::risk::RiskEngine;
use crate::setup;
use crate::state;
use crate::state::positions::PositionManager;
use crate::state::reconciliation::Reconciler;
use crate::state::sqlite::{SignalLogEntry, SqliteStore, TargetRow};
use polybot_common::types::{ExecutionMode, Position};
use std::sync::Arc;
use tokio::sync::Mutex;

fn resolve_report_period(period: Option<&str>) -> Result<&'static str, &'static str> {
    match period.map(|p| p.trim().to_lowercase()) {
        None => Ok("Daily"),
        Some(p) if p.is_empty() || p == "daily" => Ok("Daily"),
        Some(p) if p == "weekly" => Ok("Weekly"),
        _ => Err("Usage: /report [daily|weekly]"),
    }
}

fn resolve_mode_switch(mode: Option<&str>) -> Result<ExecutionMode, &'static str> {
    match mode.map(|m| m.trim().to_lowercase()) {
        Some(m) if m == "sim" || m == "simulation" => Ok(ExecutionMode::Simulation),
        Some(m) if m == "live" => Ok(ExecutionMode::Live),
        Some(m) if m == "shadow" => Ok(ExecutionMode::Shadow),
        _ => Err("Usage: /mode [sim|live|shadow]"),
    }
}

fn format_wallet_score_message(target: &TargetRow) -> String {
    let label = target.label.as_deref().unwrap_or("(no label)");
    let categories = if target.categories.is_empty() {
        "all".to_string()
    } else {
        target
            .categories
            .iter()
            .map(|category| category.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let score = target
        .score
        .map(|value| format!("{:.2}", value))
        .unwrap_or_else(|| "No score available".to_string());

    format!(
        "Wallet score\nAddress: {}\nLabel: {}\nCategories: {}\nScore: {}",
        target.wallet_address, label, categories, score
    )
}

fn requested_mode_key() -> &'static str {
    "requested_execution_mode"
}

fn load_requested_mode_summary() -> Option<String> {
    let sqlite_path = std::env::var("POLYBOT_SQLITE_PATH").unwrap_or_else(|_| "./polybot.db".to_string());
    SqliteStore::open(std::path::Path::new(&sqlite_path))
        .ok()
        .and_then(|store| store.get_config(requested_mode_key()).ok().flatten())
}

async fn confirm_mode_switch(
    config: &Arc<AppConfig>,
    mode: ExecutionMode,
) -> Result<String, String> {
    let mut requested = (**config).clone();
    requested.system.execution_mode = mode;
    requested.system.simulation = matches!(mode, ExecutionMode::Simulation);

    if !matches!(mode, ExecutionMode::Simulation) {
        setup::run_startup_preflight(&requested)
            .await
            .map_err(|e| format!("Mode switch validation failed: {}", e))?;
    }

    let sqlite_path = std::env::var("POLYBOT_SQLITE_PATH")
        .unwrap_or_else(|_| "./polybot.db".to_string());
    let store = SqliteStore::open(std::path::Path::new(&sqlite_path))
        .map_err(|e| format!("Failed to open SQLite for mode switch persistence: {}", e))?;
    store
        .set_config(
            requested_mode_key(),
            match mode {
                ExecutionMode::Simulation => "simulation",
                ExecutionMode::Shadow => "shadow",
                ExecutionMode::Live => "live",
            },
        )
        .map_err(|e| format!("Failed to persist requested mode: {}", e))?;

    Ok(format!(
        "Mode switch to {:?} validated and staged. Restart required to apply.",
        mode
    ))
}

pub async fn handle_command(
    bot: Bot,
    msg: teloxide::types::Message,
    cmd: Command,
    auth: Arc<AuthService>,
    confirm_state: Arc<ConfirmState>,
    rate_limiter: Arc<CommandRateLimiter>,
    risk_engine: Arc<RiskEngine>,
    reconciler: Arc<Reconciler>,
    config: Arc<AppConfig>,
    metrics: Arc<Metrics>,
    position_manager: Arc<Mutex<PositionManager>>,
) -> ResponseResult<()> {
    let user_id = match msg.from {
        Some(user) => user.id.0,
        None => return Ok(()),
    };

    // v2.5: Auth check on every message
    if !auth.is_allowed(user_id) {
        return Ok(());
    }

    // v2.5: Rate limit check (except /confirm)
    if !matches!(cmd, Command::Confirm) {
        if !rate_limiter.check_command(user_id) {
            bot.send_message(msg.chat.id, "Rate limited. Max 30 commands per minute.")
                .await?;
            return Ok(());
        }
    }

    match cmd {
        Command::Status => {
            let mode = match config.system.execution_mode {
                ExecutionMode::Simulation => "SIMULATION",
                ExecutionMode::Shadow => "SHADOW",
                ExecutionMode::Live => "LIVE",
            };
            let uptime = metrics.uptime_secs();
            let uptime_fmt = format!(
                "{}h {}m {}s",
                uptime / 3600,
                (uptime % 3600) / 60,
                uptime % 60
            );
            let ws = if metrics.ws_connected.load(Ordering::Relaxed) == 1 {
                "Connected"
            } else {
                "Disconnected"
            };
            let rpc = if metrics.rpc_healthy.load(Ordering::Relaxed) == 1 {
                "Healthy"
            } else {
                "Unhealthy"
            };
            let open = metrics.open_positions.load(Ordering::Relaxed);
            let sigs = metrics.signals_received.load(Ordering::Relaxed);
            let wallets = risk_engine.list_followed_wallets().await.len();
            let bot_status = if risk_engine.is_emergency_stop().await {
                "PAUSED"
            } else {
                "ACTIVE"
            };
            let pending_mode = load_requested_mode_summary()
                .map(|mode| format!("\nPending mode switch: {} (restart required)", mode.to_uppercase()))
                .unwrap_or_default();

            bot.send_message(
                msg.chat.id,
                format!(
                    "SuperFast PolyBot v3\nMode: {}\nStatus: {}\nUptime: {}\nPositions: {}\nSignals: {}\nFollowed wallets: {}\nWS: {}\nRPC: {}{}",
                    mode, bot_status, uptime_fmt, open, sigs, wallets, ws, rpc, pending_mode
                )
            ).await?;
        }

        Command::Positions => {
            let pnl = metrics.daily_pnl_usd();
            let sqlite_path = std::env::var("POLYBOT_SQLITE_PATH")
                .unwrap_or_else(|_| "./polybot.db".to_string());
            let body = match SqliteStore::open(std::path::Path::new(&sqlite_path)) {
                Ok(store) => match store.list_open_positions() {
                    Ok(rows) if !rows.is_empty() => {
                        let positions = rows.into_iter().map(|row| row.position).collect::<Vec<_>>();
                        format_positions_message(&positions, pnl)
                    }
                    _ => fallback_positions_message(&metrics),
                },
                Err(_) => fallback_positions_message(&metrics),
            };

            bot.send_message(msg.chat.id, body).await?;
        }

        Command::Signals => {
            let sqlite_path = std::env::var("POLYBOT_SQLITE_PATH")
                .unwrap_or_else(|_| "./polybot.db".to_string());
            let body = match SqliteStore::open(std::path::Path::new(&sqlite_path)) {
                Ok(store) => match store.latest_signals(10) {
                    Ok(signals) => format_signals_message(&signals),
                    Err(_) => fallback_signals_message(&metrics),
                },
                Err(_) => fallback_signals_message(&metrics),
            };
            bot.send_message(msg.chat.id, body).await?;
        }

        Command::Pause => {
            risk_engine.set_emergency_stop(true).await;
            bot.send_message(
                msg.chat.id,
                "Trading paused. Existing positions held. Use /resume to restart.",
            )
            .await?;
        }

        Command::Resume => {
            if risk_engine.resume_requires_confirmation().await && !confirm_state.has_pending(user_id) {
                confirm_state.register(user_id, ConfirmAction::ResumeAfterLoss);
                bot.send_message(
                    msg.chat.id,
                    "Resume after loss breach requires confirmation. Reply /confirm within 30 seconds.",
                )
                .await?;
            } else if confirm_state.has_pending(user_id) {
                risk_engine.set_emergency_stop(false).await;
                risk_engine.clear_resume_confirmation().await;
                confirm_state.cancel(user_id);
                bot.send_message(msg.chat.id, "Trading resumed.").await?;
            } else {
                risk_engine.set_emergency_stop(false).await;
                risk_engine.clear_resume_confirmation().await;
                bot.send_message(msg.chat.id, "Trading resumed.").await?;
            }
        }

        Command::EmergencyStop => {
            if !rate_limiter.check_emergency_stop(user_id) {
                bot.send_message(
                    msg.chat.id,
                    "Emergency stop rate limit reached (max 3/hour). Please wait.",
                )
                .await?;
                return Ok(());
            }
            confirm_state.register(user_id, ConfirmAction::EmergencyStop);
            metrics.record_emergency_stop();
            bot.send_message(
                msg.chat.id,
                "Destructive command: EMERGENCY STOP\nThis will close all positions as market orders.\nReply /confirm within 30 seconds to proceed."
            ).await?;
        }

        Command::Confirm => match confirm_state.confirm(user_id) {
            Some(ConfirmAction::EmergencyStop) => {
                risk_engine.set_emergency_stop(true).await;
                let closed_positions = state::force_flatten_positions(
                    metrics.clone(),
                    position_manager.clone(),
                )
                .await
                .unwrap_or(0);
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "EMERGENCY STOP CONFIRMED. All trading halted. Closed {} open positions.",
                        closed_positions
                    ),
                )
                .await?;
            }
            Some(ConfirmAction::ResumeAfterLoss) => {
                risk_engine.set_emergency_stop(false).await;
                risk_engine.clear_resume_confirmation().await;
                bot.send_message(msg.chat.id, "Resume confirmed. Trading resumed.")
                    .await?;
            }
            Some(ConfirmAction::WalletRemove(addr)) => {
                risk_engine.remove_followed_wallet(&addr).await;
                let sqlite_path = std::env::var("POLYBOT_SQLITE_PATH")
                    .unwrap_or_else(|_| "./polybot.db".to_string());
                if let Ok(store) = SqliteStore::open(std::path::Path::new(&sqlite_path)) {
                    let _ = store.deactivate_target(&addr);
                }
                bot.send_message(msg.chat.id, format!("Wallet {} removal confirmed.", addr))
                    .await?;
            }
            Some(ConfirmAction::ModeSwitch(mode)) => {
                match confirm_mode_switch(&config, mode).await {
                    Ok(message) => {
                        bot.send_message(msg.chat.id, message).await?;
                    }
                    Err(message) => {
                        bot.send_message(msg.chat.id, message).await?;
                    }
                }
            }
            None => {
                bot.send_message(msg.chat.id, "No pending confirmation. Send a destructive command first, then /confirm within 30 seconds.").await?;
            }
        },

        Command::Report(period) => match resolve_report_period(period.as_deref()) {
            Ok(period) => {
                bot.send_message(msg.chat.id, alerts::format_report(&metrics, period)).await?;
            }
            Err(message) => {
                bot.send_message(msg.chat.id, message).await?;
            }
        },

        Command::Wallet(action, address) => {
            let requires_pause = matches!(action.as_str(), "add" | "remove");
            if requires_pause && !risk_engine.is_emergency_stop().await {
                bot.send_message(msg.chat.id, "Pause trading before changing the followed wallet list.")
                    .await?;
                return Ok(());
            }

            match action.as_str() {
                "add" => match address.as_deref() {
                    Some(address) => match risk_engine.add_followed_wallet(address).await {
                        Ok(()) => {
                            let sqlite_path = std::env::var("POLYBOT_SQLITE_PATH")
                                .unwrap_or_else(|_| "./polybot.db".to_string());
                            if let Ok(store) = SqliteStore::open(std::path::Path::new(&sqlite_path)) {
                                let _ = store.upsert_target(address, None, &config.scanner.target_categories, None);
                            }
                            bot.send_message(msg.chat.id, format!("Wallet {} added to copy list.", address)).await?;
                        }
                        Err(e) => {
                            bot.send_message(msg.chat.id, format!("Wallet add failed: {}", e)).await?;
                        }
                    },
                    None => {
                        bot.send_message(msg.chat.id, "Usage: /wallet add <address>").await?;
                    }
                },
                "remove" => match address {
                    Some(address) => {
                        confirm_state.register(user_id, ConfirmAction::WalletRemove(address.clone()));
                        bot.send_message(
                            msg.chat.id,
                            format!("Removing wallet {} is destructive. Reply /confirm within 30 seconds.", address),
                        )
                        .await?;
                    }
                    None => {
                        bot.send_message(msg.chat.id, "Usage: /wallet remove <address>").await?;
                    }
                },
                "list" => {
                    let wallets = risk_engine.list_followed_wallets().await;
                    let body = if wallets.is_empty() {
                        "No followed wallets configured.".to_string()
                    } else {
                        format!("Followed wallets:\n{}", wallets.join("\n"))
                    };
                    bot.send_message(msg.chat.id, body).await?;
                }
                "score" => match address.as_deref() {
                    Some(address) => {
                        let sqlite_path = std::env::var("POLYBOT_SQLITE_PATH")
                            .unwrap_or_else(|_| "./polybot.db".to_string());
                        let body = match SqliteStore::open(std::path::Path::new(&sqlite_path)) {
                            Ok(store) => match store.list_active_targets() {
                                Ok(targets) => targets
                                    .into_iter()
                                    .find(|target| target.wallet_address == address.to_lowercase())
                                    .map(|target| format_wallet_score_message(&target))
                                    .unwrap_or_else(|| format!("No active target record for wallet {}", address)),
                                Err(e) => format!("Wallet score lookup failed: {}", e),
                            },
                            Err(e) => format!("Wallet score lookup failed: {}", e),
                        };
                        bot.send_message(msg.chat.id, body).await?;
                    }
                    None => {
                        bot.send_message(msg.chat.id, "Usage: /wallet score <address>").await?;
                    }
                },
                _ => {
                    bot.send_message(msg.chat.id, "Usage: /wallet add <address> | /wallet remove <address> | /wallet list | /wallet score <address>")
                        .await?;
                }
            }
        }

        Command::Mode(requested_mode) => match resolve_mode_switch(requested_mode.as_deref()) {
            Ok(mode) => {
                confirm_state.register(user_id, ConfirmAction::ModeSwitch(mode));
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "Mode switch to {:?} requires confirmation. Reply /confirm within 30 seconds.",
                        mode
                    ),
                )
                .await?;
            }
            Err(message) => {
                bot.send_message(msg.chat.id, message).await?;
            }
        },

        Command::Config(key, value) => {
            match risk_engine.update_runtime_config(&key, &value).await {
                Ok(message) => {
                    let summary = risk_engine.runtime_config_summary().await;
                    bot.send_message(msg.chat.id, format!("{}\n{}", message, summary)).await?;
                }
                Err(e) => {
                    bot.send_message(msg.chat.id, format!("Config update failed: {}", e)).await?;
                }
            }
        }

        Command::Reconcile => match reconciler.force_reconcile().await {
            Ok(result) => {
                bot.send_message(
                    msg.chat.id,
                    format!("Full reconciliation completed.\n{}", result.summary()),
                )
                .await?;
            }
            Err(e) => {
                bot.send_message(msg.chat.id, format!("Full reconciliation failed: {}", e))
                    .await?;
            }
        },

        Command::Ratelimit => {
            let (cmds, es) = rate_limiter.get_stats(user_id);
            bot.send_message(
                msg.chat.id,
                format!("Rate Limit Status:\nCommands this minute: {}/30\nEmergency stops this hour: {}/3", cmds, es)
            ).await?;
        }
    }

    Ok(())
}

fn format_positions_message(positions: &[Position], pnl: f64) -> String {
    if positions.is_empty() {
        return format!("Positions:\nNo open positions\nDaily PnL: ${:.2}", pnl);
    }

    let lines = positions
        .iter()
        .map(|position| {
            format!(
                "{} {:?} size={} avg={} status={:?}",
                position.market_id,
                position.side,
                position.current_size,
                position.average_price,
                position.status
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "Positions:\n{}\nOpen: {}\nDaily PnL: ${:.2}",
        lines,
        positions.len(),
        pnl
    )
}

fn fallback_positions_message(metrics: &Metrics) -> String {
    let open = metrics.open_positions.load(Ordering::Relaxed);
    let total_opened = metrics.total_positions_opened.load(Ordering::Relaxed);
    let total_closed = metrics.total_positions_closed.load(Ordering::Relaxed);
    let pnl = metrics.daily_pnl_usd();

    format!(
        "Positions:\nOpen: {}\nTotal opened: {}\nTotal closed: {}\nDaily PnL: ${:.2}",
        open, total_opened, total_closed, pnl
    )
}

fn format_signals_message(signals: &[SignalLogEntry]) -> String {
    if signals.is_empty() {
        return "Signals:\nNo recent signals".to_string();
    }

    let lines = signals
        .iter()
        .map(|signal| {
            format!(
                "{} {} conf={} secret={} {}",
                signal.market_id,
                signal.side,
                signal.confidence,
                signal.secret_level,
                signal.disposition
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("Signals:\n{}", lines)
}

fn fallback_signals_message(metrics: &Metrics) -> String {
    let received = metrics.signals_received.load(Ordering::Relaxed);
    let processed = metrics.signals_processed.load(Ordering::Relaxed);
    let skipped = metrics.signals_skipped.load(Ordering::Relaxed);
    let manual = metrics.signals_manual_review.load(Ordering::Relaxed);
    let last = metrics
        .last_signal_at
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| "Never".to_string());

    format!(
        "Signals:\nReceived: {}\nProcessed: {}\nSkipped: {}\nManual review: {}\nLast signal: {}",
        received, processed, skipped, manual, last
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use polybot_common::types::{Category, PositionStatus, Side};
    use rust_decimal_macros::dec;

    fn sample_position() -> Position {
        Position {
            id: "p1".to_string(),
            market_id: "market-1".to_string(),
            side: Side::Yes,
            entry_price: dec!(0.50),
            current_size: dec!(100),
            average_price: dec!(0.52),
            opened_at: Utc::now(),
            status: PositionStatus::Open,
            category: Category::Politics,
        }
    }

    fn sample_signal() -> SignalLogEntry {
        SignalLogEntry {
            signal_id: "signal-1".to_string(),
            timestamp: "2026-04-17T12:00:00Z".to_string(),
            wallet_address: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "market-1".to_string(),
            confidence: 8,
            secret_level: 9,
            category: "politics".to_string(),
            side: "YES".to_string(),
            disposition: "execute".to_string(),
            received_at: "2026-04-17T12:00:01Z".to_string(),
        }
    }

    #[test]
    fn format_positions_message_lists_open_positions() {
        let output = format_positions_message(&[sample_position()], 12.34);
        assert!(output.contains("market-1"));
        assert!(output.contains("Open: 1"));
        assert!(output.contains("Daily PnL: $12.34"));
    }

    #[test]
    fn format_signals_message_lists_recent_signals() {
        let output = format_signals_message(&[sample_signal()]);
        assert!(output.contains("market-1"));
        assert!(output.contains("conf=8"));
        assert!(output.contains("execute"));
    }

    #[test]
    fn resolve_report_period_defaults_to_daily_and_supports_weekly() {
        assert_eq!(resolve_report_period(None).unwrap(), "Daily");
        assert_eq!(resolve_report_period(Some("weekly")).unwrap(), "Weekly");
        assert!(resolve_report_period(Some("monthly")).is_err());
    }

    #[test]
    fn resolve_mode_switch_parses_live_and_simulation_aliases() {
        assert_eq!(
            resolve_mode_switch(Some("live")).unwrap(),
            polybot_common::types::ExecutionMode::Live
        );
        assert_eq!(
            resolve_mode_switch(Some("sim")).unwrap(),
            polybot_common::types::ExecutionMode::Simulation
        );
        assert!(resolve_mode_switch(Some("paper")).is_err());
    }

    #[test]
    fn wallet_score_message_handles_missing_score() {
        let target = crate::state::sqlite::TargetRow {
            wallet_address: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            label: Some("leader".to_string()),
            categories: vec![Category::Politics],
            score: None,
            active: true,
        };

        let output = format_wallet_score_message(&target);
        assert!(output.contains("leader"));
        assert!(output.contains("No score available"));
    }
}
