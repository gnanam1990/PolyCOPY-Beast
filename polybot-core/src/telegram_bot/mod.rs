pub mod alerts;
pub mod auth;
pub mod commands;
pub mod confirm;
pub mod rate_limiter;

use polybot_common::errors::PolybotError;
use std::sync::Arc;

use crate::config::AppConfig;
use crate::metrics::Metrics;
use crate::risk::RiskEngine;
use crate::state::positions::PositionManager;
use crate::state::reconciliation::Reconciler;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::Mutex;

use teloxide::prelude::*;
use teloxide::utils::command::{BotCommands, ParseError};

fn parse_report_args(s: String) -> Result<(Option<String>,), ParseError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        Ok((None,))
    } else {
        Ok((Some(trimmed.to_string()),))
    }
}

fn parse_mode_args(s: String) -> Result<(Option<String>,), ParseError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        Ok((None,))
    } else {
        Ok((Some(trimmed.to_string()),))
    }
}

fn parse_wallet_args(s: String) -> Result<(String, Option<String>), ParseError> {
    let args = s.split_whitespace().collect::<Vec<_>>();
    match args.as_slice() {
        [action] => Ok(((*action).to_string(), None)),
        [action, address] => Ok(((*action).to_string(), Some((*address).to_string()))),
        _ => Err(ParseError::IncorrectFormat(
            "Usage: /wallet add <address> | /wallet remove <address> | /wallet list | /wallet score <address>".into(),
        )),
    }
}

#[derive(BotCommands, Clone, Debug, PartialEq)]
#[command(rename_rule = "lowercase", description = "PolyBot v3 commands", parse_with = "split")]
pub enum Command {
    #[command(description = "Show system status")]
    Status,
    #[command(description = "Show open positions")]
    Positions,
    #[command(description = "Show recent signals")]
    Signals,
    #[command(description = "Pause trading")]
    Pause,
    #[command(description = "Resume trading")]
    Resume,
    #[command(description = "Emergency stop (requires /confirm)")]
    EmergencyStop,
    #[command(description = "Confirm destructive command")]
    Confirm,
    #[command(description = "Show report: /report [daily|weekly]", parse_with = parse_report_args)]
    Report(Option<String>),
    #[command(description = "Manage followed wallets: /wallet add <address> or /wallet remove <address>", parse_with = parse_wallet_args)]
    Wallet(String, Option<String>),
    #[command(description = "Stage mode change: /mode [sim|shadow|live]", parse_with = parse_mode_args)]
    Mode(Option<String>),
    #[command(description = "Update runtime risk config: /config <key> <value>")]
    Config(String, String),
    #[command(description = "Force full reconciliation")]
    Reconcile,
    #[command(description = "Show API rate limit usage")]
    Ratelimit,
}

pub async fn start_telegram_bot(
    config: Arc<AppConfig>,
    risk_engine: Arc<RiskEngine>,
    reconciler: Arc<Reconciler>,
    metrics: Arc<Metrics>,
    position_manager: Arc<Mutex<PositionManager>>,
    alert_receiver: UnboundedReceiver<alerts::AlertMessage>,
) -> Result<(), PolybotError> {
    let bot_token = std::env::var("POLYBOT_TELEGRAM_TOKEN")
        .map_err(|_| PolybotError::Telegram("POLYBOT_TELEGRAM_TOKEN not set".to_string()))?;

    let allowed_users = std::env::var("TELEGRAM_ALLOWED_USER_IDS")
        .or_else(|_| std::env::var("POLYBOT_TELEGRAM_ALLOWED_USER_IDS"))
        .ok()
        .map(|s| {
            s.split(',')
                .filter_map(|id| id.trim().parse::<u64>().ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| config.telegram.allowed_user_ids.clone());

    if allowed_users.is_empty() {
        tracing::warn!("No Telegram users whitelisted — bot will not respond to anyone");
    }

    let bot = Bot::new(bot_token);
    bot.get_me()
        .send()
        .await
        .map_err(|e| PolybotError::Telegram(format!("Telegram bot authentication failed: {}", e)))?;

    // Shared state for auth, confirm, and rate limiting
    let auth = Arc::new(auth::AuthService::new(allowed_users));
    let confirm_state = Arc::new(confirm::ConfirmState::new());
    let rate_limiter = Arc::new(rate_limiter::CommandRateLimiter::new(
        config.telegram.command_rate_limit_per_min,
        config.telegram.emergency_stop_limit_per_hour,
    ));

    tracing::info!(
        users = auth.allowed_count(),
        "Telegram bot starting with auth"
    );

    alerts::spawn_dispatcher(bot.clone(), auth.allowed_users(), metrics.clone(), alert_receiver);

    let handler =
        Update::filter_message().branch(dptree::entry().filter_command::<Command>().endpoint(
            move |bot, msg, cmd| {
                let auth = auth.clone();
                let confirm_state = confirm_state.clone();
                let rate_limiter = rate_limiter.clone();
                let risk_engine = risk_engine.clone();
                let reconciler = reconciler.clone();
                let config = config.clone();
                let metrics = metrics.clone();
                let position_manager = position_manager.clone();
                async move {
                    commands::handle_command(
                        bot,
                        msg,
                        cmd,
                        auth,
                        confirm_state,
                        rate_limiter,
                        risk_engine,
                        reconciler,
                        config,
                        metrics,
                        position_manager,
                    )
                    .await
                }
            },
        ));

    Dispatcher::builder(bot, handler).build().dispatch().await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wallet_list_without_placeholder_argument() {
        let parsed = Command::parse("/wallet list", "polybot").unwrap();
        assert_eq!(parsed, Command::Wallet("list".to_string(), None));
    }

    #[test]
    fn parse_wallet_add_with_address() {
        let parsed = Command::parse(
            "/wallet add 0xabc123abc123abc123abc123abc123abc123abc1",
            "polybot",
        )
        .unwrap();
        assert_eq!(
            parsed,
            Command::Wallet(
                "add".to_string(),
                Some("0xabc123abc123abc123abc123abc123abc123abc1".to_string())
            )
        );
    }

    #[test]
    fn parse_report_period_and_mode_commands() {
        let report = Command::parse("/report weekly", "polybot").unwrap();
        let mode = Command::parse("/mode live", "polybot").unwrap();

        assert_eq!(report, Command::Report(Some("weekly".to_string())));
        assert_eq!(mode, Command::Mode(Some("live".to_string())));
    }
}
