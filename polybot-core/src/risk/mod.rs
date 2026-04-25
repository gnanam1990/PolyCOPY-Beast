pub mod balance;
pub mod drawdown;
pub mod limits;
pub mod sizer;

use polybot_common::constants::{
    confidence_multiplier, drawdown_multiplier as calc_drawdown, secret_level_multiplier,
};
use polybot_common::types::{Decision, RiskDecision, Signal, TradeDirection};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex, RwLock};

use crate::config::{AppConfig, RiskConfig};
use crate::metrics::Metrics;
use crate::risk::balance::DynamicBaseSize;
use crate::state::positions::PositionManager;
use crate::state::sqlite::SqliteStore;
use crate::telegram_bot::alerts::AlertBroadcaster;

fn sold_tokens_from_signal(signal: &Signal) -> Decimal {
    signal
        .target_size_tokens
        .or_else(|| {
            signal.target_size_usdc.and_then(|size_usdc| {
                signal
                    .target_price
                    .filter(|price| *price > Decimal::ZERO)
                    .map(|price| (size_usdc / price).round_dp(8))
            })
        })
        .unwrap_or(Decimal::ZERO)
}

fn mirrored_exit_tokens(
    copied_tokens: Decimal,
    sold_tokens: Decimal,
    remaining_source_tokens: Option<Decimal>,
) -> Option<Decimal> {
    let remaining_source_tokens = remaining_source_tokens?;
    if copied_tokens <= Decimal::ZERO
        || sold_tokens <= Decimal::ZERO
        || remaining_source_tokens < Decimal::ZERO
    {
        return None;
    }

    let denominator = sold_tokens + remaining_source_tokens;
    if denominator <= Decimal::ZERO {
        return None;
    }

    Some(
        (copied_tokens * (sold_tokens / denominator))
            .round_dp(8)
            .max(Decimal::ZERO)
            .min(copied_tokens),
    )
}

pub struct RiskEngine {
    config: Arc<AppConfig>,
    metrics: Arc<Metrics>,
    portfolio_drawdown_pct: Arc<Mutex<Decimal>>,
    emergency_stop: Arc<Mutex<bool>>,
    resume_requires_confirm: Arc<Mutex<bool>>,
    position_manager: Arc<Mutex<PositionManager>>,
    runtime_risk: Arc<RwLock<RiskConfig>>,
    followed_wallets: Arc<RwLock<BTreeSet<String>>>,
    balance_manager: Arc<DynamicBaseSize>,
    consecutive_losses: Arc<Mutex<u32>>,
    cooldown_until: Arc<Mutex<Option<Instant>>>,
    alerts: Option<AlertBroadcaster>,
}

impl RiskEngine {
    pub fn new(
        config: Arc<AppConfig>,
        metrics: Arc<Metrics>,
        position_manager: Arc<Mutex<PositionManager>>,
        alerts: Option<AlertBroadcaster>,
    ) -> Self {
        let runtime_risk = config.risk.clone();
        let followed_wallets = std::env::var("POLYBOT_FOLLOW_WALLETS")
            .ok()
            .map(|value| {
                value
                    .split(',')
                    .map(|wallet| wallet.trim().to_lowercase())
                    .filter(|wallet| !wallet.is_empty())
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let initial_balance = config.paper.starting_balance_usd;

        Self {
            config,
            metrics,
            portfolio_drawdown_pct: Arc::new(Mutex::new(Decimal::ZERO)),
            emergency_stop: Arc::new(Mutex::new(false)),
            resume_requires_confirm: Arc::new(Mutex::new(false)),
            position_manager,
            runtime_risk: Arc::new(RwLock::new(runtime_risk)),
            followed_wallets: Arc::new(RwLock::new(followed_wallets)),
            balance_manager: Arc::new(DynamicBaseSize::new(initial_balance)),
            consecutive_losses: Arc::new(Mutex::new(0)),
            cooldown_until: Arc::new(Mutex::new(None)),
            alerts,
        }
    }

    fn portfolio_reference_usd(&self, _risk_config: &RiskConfig) -> Decimal {
        self.config.paper.starting_balance_usd
    }

    fn copied_lot_side_key(side: polybot_common::types::Side) -> &'static str {
        match side {
            polybot_common::types::Side::Yes => "YES",
            polybot_common::types::Side::No => "NO",
        }
    }

    async fn fetch_source_remaining_tokens(&self, signal: &Signal) -> Option<Decimal> {
        let client = match polymarket_client_sdk::data::Client::new(
            &self.config.scanner.data_api_url,
        ) {
            Ok(c) => c,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    data_api_url = %self.config.scanner.data_api_url,
                    "Failed to construct data API client; cannot fetch source remaining position"
                );
                return None;
            }
        };
        let wallet: polymarket_client_sdk::types::Address = match signal.wallet_address.parse() {
            Ok(w) => w,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    wallet = %signal.wallet_address,
                    "Invalid wallet address; cannot fetch source remaining position"
                );
                return None;
            }
        };
        let side = Self::copied_lot_side_key(signal.side);
        let mut offset = 0;

        loop {
            let request = match polymarket_client_sdk::data::types::request::PositionsRequest::builder()
                .user(wallet)
                .limit(500)
                .and_then(|b| b.offset(offset))
            {
                Ok(b) => b.build(),
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        wallet = %signal.wallet_address,
                        offset,
                        "Failed to build positions request; cannot fetch source remaining position"
                    );
                    return None;
                }
            };

            let positions = match client.positions(&request).await {
                Ok(positions) => positions,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        wallet = %signal.wallet_address,
                        market_id = %signal.market_id,
                        "Failed to fetch source remaining position for mirrored exit"
                    );
                    return None;
                }
            };
            let batch_size = positions.len();

            if let Some(position) = positions.into_iter().find(|position| {
                position
                    .condition_id
                    .to_string()
                    .eq_ignore_ascii_case(&signal.market_id)
                    && position.outcome.eq_ignore_ascii_case(side)
            }) {
                return Some(position.size);
            }

            if batch_size < 500 {
                return None;
            }

            offset += batch_size as i32;
        }
    }

    pub async fn evaluate(&self, signal: &Signal) -> RiskDecision {
        let risk_config = self.runtime_risk.read().await.clone();
        let current_balance = self.balance_manager.current_balance();

        // 1. Check emergency stop
        if *self.emergency_stop.lock().await {
            return RiskDecision {
                signal_id: signal.signal_id.clone(),
                source_wallet: signal.wallet_address.clone(),
                market_id: signal.market_id.clone(),
                side: signal.side,
                direction: signal.direction,
                category: signal.category,
                position_size_usd: Decimal::ZERO,
                target_size_tokens: None,
                confidence_multiplier: Decimal::ZERO,
                secret_level_multiplier: Decimal::ZERO,
                drawdown_factor: Decimal::ZERO,
                blocked: true,
                manual_review: false,
                decision: Decision::EmergencyStop,
            };
        }

        if current_balance < risk_config.min_usdc_balance {
            self.set_emergency_stop(true).await;
            if let Some(alerts) = &self.alerts {
                alerts.critical(format!(
                    "USDC balance ${} fell below minimum ${}. Trading auto-paused.",
                    current_balance, risk_config.min_usdc_balance
                ));
            }
            return RiskDecision {
                signal_id: signal.signal_id.clone(),
                source_wallet: signal.wallet_address.clone(),
                market_id: signal.market_id.clone(),
                side: signal.side,
                direction: signal.direction,
                category: signal.category,
                position_size_usd: Decimal::ZERO,
                target_size_tokens: None,
                confidence_multiplier: Decimal::ZERO,
                secret_level_multiplier: Decimal::ZERO,
                drawdown_factor: Decimal::ZERO,
                blocked: true,
                manual_review: false,
                decision: Decision::EmergencyStop,
            };
        }

        if let Some(cooldown_until) = *self.cooldown_until.lock().await {
            if Instant::now() < cooldown_until {
                return RiskDecision {
                    signal_id: signal.signal_id.clone(),
                    source_wallet: signal.wallet_address.clone(),
                    market_id: signal.market_id.clone(),
                    side: signal.side,
                    direction: signal.direction,
                    category: signal.category,
                    position_size_usd: Decimal::ZERO,
                    target_size_tokens: None,
                    confidence_multiplier: Decimal::ZERO,
                    secret_level_multiplier: Decimal::ZERO,
                    drawdown_factor: Decimal::ZERO,
                    blocked: true,
                    manual_review: false,
                    decision: Decision::Skip("loss cooldown active".to_string()),
                };
            }
        }

        let followed_wallets = self.followed_wallets.read().await;
        if !followed_wallets.is_empty()
            && !followed_wallets.contains(&signal.wallet_address.to_lowercase())
        {
            return RiskDecision {
                signal_id: signal.signal_id.clone(),
                source_wallet: signal.wallet_address.clone(),
                market_id: signal.market_id.clone(),
                side: signal.side,
                direction: signal.direction,
                category: signal.category,
                position_size_usd: Decimal::ZERO,
                target_size_tokens: None,
                confidence_multiplier: Decimal::ZERO,
                secret_level_multiplier: Decimal::ZERO,
                drawdown_factor: Decimal::ZERO,
                blocked: true,
                manual_review: false,
                decision: Decision::Skip("wallet not in followed list".to_string()),
            };
        }
        drop(followed_wallets);

        if signal.direction == TradeDirection::Sell {
            let sqlite_path =
                std::env::var("POLYBOT_SQLITE_PATH").unwrap_or_else(|_| "./polybot.db".to_string());
            let store = match SqliteStore::open(std::path::Path::new(&sqlite_path)) {
                Ok(store) => store,
                Err(error) => {
                    tracing::warn!(error = %error, "Skipping mirrored sell because copied-lot store is unavailable");
                    return RiskDecision {
                        signal_id: signal.signal_id.clone(),
                        source_wallet: signal.wallet_address.clone(),
                        market_id: signal.market_id.clone(),
                        side: signal.side,
                        direction: signal.direction,
                        category: signal.category,
                        position_size_usd: Decimal::ZERO,
                        target_size_tokens: None,
                        confidence_multiplier: Decimal::ZERO,
                        secret_level_multiplier: Decimal::ZERO,
                        drawdown_factor: Decimal::ZERO,
                        blocked: true,
                        manual_review: false,
                        decision: Decision::Skip("copied lot store unavailable".to_string()),
                    };
                }
            };
            let side = Self::copied_lot_side_key(signal.side);
            let lot = match store.get_copied_lot(&signal.wallet_address, &signal.market_id, side) {
                Ok(Some(lot)) => lot,
                Ok(None) => {
                    return RiskDecision {
                        signal_id: signal.signal_id.clone(),
                        source_wallet: signal.wallet_address.clone(),
                        market_id: signal.market_id.clone(),
                        side: signal.side,
                        direction: signal.direction,
                        category: signal.category,
                        position_size_usd: Decimal::ZERO,
                        target_size_tokens: None,
                        confidence_multiplier: Decimal::ZERO,
                        secret_level_multiplier: Decimal::ZERO,
                        drawdown_factor: Decimal::ZERO,
                        blocked: true,
                        manual_review: false,
                        decision: Decision::Skip(
                            "no matching copied lot for mirrored sell".to_string(),
                        ),
                    };
                }
                Err(error) => {
                    tracing::warn!(error = %error, "Skipping mirrored sell because copied-lot lookup failed");
                    return RiskDecision {
                        signal_id: signal.signal_id.clone(),
                        source_wallet: signal.wallet_address.clone(),
                        market_id: signal.market_id.clone(),
                        side: signal.side,
                        direction: signal.direction,
                        category: signal.category,
                        position_size_usd: Decimal::ZERO,
                        target_size_tokens: None,
                        confidence_multiplier: Decimal::ZERO,
                        secret_level_multiplier: Decimal::ZERO,
                        drawdown_factor: Decimal::ZERO,
                        blocked: true,
                        manual_review: false,
                        decision: Decision::Skip("copied lot lookup failed".to_string()),
                    };
                }
            };

            let sold_tokens = sold_tokens_from_signal(signal);
            let exit_tokens = mirrored_exit_tokens(
                lot.current_size,
                sold_tokens,
                self.fetch_source_remaining_tokens(signal).await,
            )
            .unwrap_or(Decimal::ZERO);

            if exit_tokens == Decimal::ZERO {
                return RiskDecision {
                    signal_id: signal.signal_id.clone(),
                    source_wallet: signal.wallet_address.clone(),
                    market_id: signal.market_id.clone(),
                    side: signal.side,
                    direction: signal.direction,
                    category: signal.category,
                    position_size_usd: Decimal::ZERO,
                    target_size_tokens: None,
                    confidence_multiplier: Decimal::ZERO,
                    secret_level_multiplier: Decimal::ZERO,
                    drawdown_factor: Decimal::ZERO,
                    blocked: true,
                    manual_review: false,
                    decision: Decision::Skip(
                        "remaining source position unavailable for mirrored sell".to_string(),
                    ),
                };
            }

            let exit_price = signal.target_price.unwrap_or(lot.average_price);
            return RiskDecision {
                signal_id: signal.signal_id.clone(),
                source_wallet: signal.wallet_address.clone(),
                market_id: signal.market_id.clone(),
                side: signal.side,
                direction: signal.direction,
                category: signal.category,
                position_size_usd: (exit_tokens * exit_price).round_dp(2),
                target_size_tokens: Some(exit_tokens),
                confidence_multiplier: Decimal::ONE,
                secret_level_multiplier: Decimal::ONE,
                drawdown_factor: Decimal::ONE,
                blocked: false,
                manual_review: false,
                decision: Decision::Execute,
            };
        }

        // v3.0: Anti-duplication rule — one owner per token_id.
        if let Some(token_id) = signal.token_id.as_ref() {
            let sqlite_path =
                std::env::var("POLYBOT_SQLITE_PATH").unwrap_or_else(|_| "./polybot.db".to_string());
            if let Ok(store) =
                crate::state::sqlite::SqliteStore::open(std::path::Path::new(&sqlite_path))
            {
                if let Ok(Some(existing_owner)) = store.get_open_position_owner_by_token(token_id) {
                    if existing_owner.to_lowercase() != signal.wallet_address.to_lowercase() {
                        return RiskDecision {
                            signal_id: signal.signal_id.clone(),
                            source_wallet: signal.wallet_address.clone(),
                            market_id: signal.market_id.clone(),
                            side: signal.side,
                            direction: signal.direction,
                            category: signal.category,
                            position_size_usd: Decimal::ZERO,
                            target_size_tokens: None,
                            confidence_multiplier: Decimal::ZERO,
                            secret_level_multiplier: Decimal::ZERO,
                            drawdown_factor: Decimal::ZERO,
                            blocked: true,
                            manual_review: false,
                            decision: Decision::Skip(format!(
                                "Anti-dup: token {} already owned by {}",
                                token_id, existing_owner
                            )),
                        };
                    }
                }
            }
        }

        // 2. v2.5: Check manual review (confidence < 3 or secret_level < 3)
        let manual_review = signal.requires_manual_review();
        if manual_review {
            if signal.secret_level >= 8 {
                if let Some(alerts) = &self.alerts {
                    alerts.warning(format!(
                        "High-secret signal {} queued for manual review from {}",
                        signal.signal_id, signal.wallet_address
                    ));
                }
            }
            return RiskDecision {
                signal_id: signal.signal_id.clone(),
                source_wallet: signal.wallet_address.clone(),
                market_id: signal.market_id.clone(),
                side: signal.side,
                direction: signal.direction,
                category: signal.category,
                position_size_usd: Decimal::ZERO,
                target_size_tokens: None,
                confidence_multiplier: confidence_multiplier(signal.confidence),
                secret_level_multiplier: secret_level_multiplier(signal.secret_level),
                drawdown_factor: Decimal::ZERO,
                blocked: false,
                manual_review: true,
                decision: Decision::ManualReview,
            };
        }

        // 3. v2.5: Check per-category thresholds
        if signal.is_blocked_by_category_thresholds() {
            let reason = format!(
                "Blocked by category thresholds: {} requires confidence>={}, secret_level>={}. Got confidence={}, secret_level={}",
                signal.category,
                signal.category.min_confidence_threshold(),
                signal.category.min_secret_level_threshold(),
                signal.confidence,
                signal.secret_level,
            );
            return RiskDecision {
                signal_id: signal.signal_id.clone(),
                source_wallet: signal.wallet_address.clone(),
                market_id: signal.market_id.clone(),
                side: signal.side,
                direction: signal.direction,
                category: signal.category,
                position_size_usd: Decimal::ZERO,
                target_size_tokens: None,
                confidence_multiplier: confidence_multiplier(signal.confidence),
                secret_level_multiplier: secret_level_multiplier(signal.secret_level),
                drawdown_factor: Decimal::ZERO,
                blocked: true,
                manual_review: false,
                decision: Decision::Skip(reason),
            };
        }

        // 4. v2.5: v2.5 uses confidence (not secret_level) for confidence_multiplier
        let conf_mult = confidence_multiplier(signal.confidence);
        let sl_mult = secret_level_multiplier(signal.secret_level);

        if signal.secret_level >= 8 {
            if let Some(alerts) = &self.alerts {
                alerts.info(format!(
                    "High-secret signal received: {} market={} wallet={} confidence={} secret_level={}",
                    signal.signal_id,
                    signal.market_id,
                    signal.wallet_address,
                    signal.confidence,
                    signal.secret_level
                ));
            }
        }

        // 5. v2.5: Stepped drawdown multiplier
        let current_drawdown = *self.portfolio_drawdown_pct.lock().await;
        let dd_factor = calc_drawdown(current_drawdown);

        // v2.5: if drawdown > 20%, auto-pause
        if dd_factor == Decimal::ZERO {
            tracing::error!("Portfolio drawdown > 20%! Auto-pausing trading.");
            *self.resume_requires_confirm.lock().await = true;
            self.set_emergency_stop(true).await;
            if let Some(alerts) = &self.alerts {
                alerts.critical("Daily loss limit breached. Trading auto-paused.");
            }
            return RiskDecision {
                signal_id: signal.signal_id.clone(),
                source_wallet: signal.wallet_address.clone(),
                market_id: signal.market_id.clone(),
                side: signal.side,
                direction: signal.direction,
                category: signal.category,
                position_size_usd: Decimal::ZERO,
                target_size_tokens: None,
                confidence_multiplier: conf_mult,
                secret_level_multiplier: sl_mult,
                drawdown_factor: dd_factor,
                blocked: true,
                manual_review: false,
                decision: Decision::EmergencyStop,
            };
        }

        // 6. v3: runtime position sizing
        let target_size_usd = signal
            .suggested_size_usdc
            .unwrap_or(risk_config.base_size_usd);
        let category_max = signal.category.max_single_position_usd();
        let size = sizer::calculate_position_size_v3(
            target_size_usd,
            risk_config.position_multiplier,
            signal.confidence,
            signal.secret_level,
            dd_factor,
            risk_config.min_trade_size_usdc,
            category_max,
        );

        let (open_count, market_exposure, category_exposure) = {
            let positions = self.position_manager.lock().await;
            (
                positions.open_position_count(),
                positions.market_exposure(&signal.market_id),
                positions.category_exposure(signal.category),
            )
        };

        // 7. v2.5: Check max concurrent positions
        if open_count >= risk_config.max_concurrent_positions {
            if let Some(alerts) = &self.alerts {
                alerts.warning(format!(
                    "Risk limit breach: max concurrent positions reached ({}/{})",
                    open_count, risk_config.max_concurrent_positions
                ));
            }
            return RiskDecision {
                signal_id: signal.signal_id.clone(),
                source_wallet: signal.wallet_address.clone(),
                market_id: signal.market_id.clone(),
                side: signal.side,
                direction: signal.direction,
                category: signal.category,
                position_size_usd: Decimal::ZERO,
                target_size_tokens: None,
                confidence_multiplier: conf_mult,
                secret_level_multiplier: sl_mult,
                drawdown_factor: dd_factor,
                blocked: true,
                manual_review: false,
                decision: Decision::Skip(format!(
                    "Max concurrent positions reached ({}/{})",
                    open_count, risk_config.max_concurrent_positions
                )),
            };
        }

        // 8. Check other limits
        if let Some(reason) = limits::check_limits(
            &risk_config,
            signal,
            size,
            current_drawdown,
            market_exposure,
            category_exposure,
            self.portfolio_reference_usd(&risk_config),
        ) {
            if let Some(alerts) = &self.alerts {
                alerts.warning(format!(
                    "Risk limit breach for signal {}: {}",
                    signal.signal_id, reason
                ));
            }
            return RiskDecision {
                signal_id: signal.signal_id.clone(),
                source_wallet: signal.wallet_address.clone(),
                market_id: signal.market_id.clone(),
                side: signal.side,
                direction: signal.direction,
                category: signal.category,
                position_size_usd: Decimal::ZERO,
                target_size_tokens: None,
                confidence_multiplier: conf_mult,
                secret_level_multiplier: sl_mult,
                drawdown_factor: dd_factor,
                blocked: true,
                manual_review: false,
                decision: Decision::Skip(reason),
            };
        }

        RiskDecision {
            signal_id: signal.signal_id.clone(),
            source_wallet: signal.wallet_address.clone(),
            market_id: signal.market_id.clone(),
            side: signal.side,
            direction: signal.direction,
            category: signal.category,
            position_size_usd: size,
            target_size_tokens: None,
            confidence_multiplier: conf_mult,
            secret_level_multiplier: sl_mult,
            drawdown_factor: dd_factor,
            blocked: false,
            manual_review: false,
            decision: Decision::Execute,
        }
    }

    pub async fn set_emergency_stop(&self, stopped: bool) {
        *self.emergency_stop.lock().await = stopped;
        self.metrics.set_paused(stopped);
    }

    pub async fn is_emergency_stop(&self) -> bool {
        *self.emergency_stop.lock().await
    }

    #[allow(dead_code)]
    pub async fn update_drawdown(&self, drawdown_pct: Decimal) {
        *self.portfolio_drawdown_pct.lock().await = drawdown_pct;
        self.metrics
            .update_drawdown(drawdown_pct.to_f64().unwrap_or(0.0));
        if drawdown_pct >= dec!(0.20) {
            tracing::error!("Portfolio drawdown >= 20%! Auto-pausing trading per v2.5 rules.");
            self.set_emergency_stop(true).await;
        }
    }

    #[allow(dead_code)]
    pub async fn reset_daily_loss(&self) {
        *self.portfolio_drawdown_pct.lock().await = Decimal::ZERO;
        *self.resume_requires_confirm.lock().await = false;
    }

    pub async fn resume_requires_confirmation(&self) -> bool {
        *self.resume_requires_confirm.lock().await
    }

    pub async fn clear_resume_confirmation(&self) {
        *self.resume_requires_confirm.lock().await = false;
    }

    pub async fn is_loss_cooldown_active(&self) -> bool {
        self.cooldown_until
            .lock()
            .await
            .map(|until| Instant::now() < until)
            .unwrap_or(false)
    }

    pub async fn update_portfolio_balance(&self, balance_usd: Decimal) {
        self.balance_manager.update_balance(balance_usd);
    }

    pub async fn record_realized_outcome(&self, pnl_usd: Decimal) {
        let mut consecutive_losses = self.consecutive_losses.lock().await;
        let risk = self.runtime_risk.read().await;

        if pnl_usd < Decimal::ZERO {
            *consecutive_losses += 1;
            if *consecutive_losses >= risk.max_consecutive_losses {
                *self.cooldown_until.lock().await =
                    Some(Instant::now() + Duration::from_secs(risk.loss_cooldown_secs));
            }
        } else {
            *consecutive_losses = 0;
            *self.cooldown_until.lock().await = None;
        }
    }

    pub async fn add_followed_wallet(
        &self,
        wallet: &str,
    ) -> Result<(), polybot_common::errors::PolybotError> {
        let normalized = wallet.trim().to_lowercase();
        if !normalized.starts_with("0x") || normalized.len() != 42 {
            return Err(polybot_common::errors::PolybotError::Config(
                "wallet address must be a 42-char 0x-prefixed EVM address".to_string(),
            ));
        }
        self.followed_wallets.write().await.insert(normalized);
        Ok(())
    }

    pub async fn remove_followed_wallet(&self, wallet: &str) {
        self.followed_wallets
            .write()
            .await
            .remove(&wallet.trim().to_lowercase());
    }

    pub async fn list_followed_wallets(&self) -> Vec<String> {
        self.followed_wallets.read().await.iter().cloned().collect()
    }

    pub async fn update_runtime_config(
        &self,
        key: &str,
        value: &str,
    ) -> Result<String, polybot_common::errors::PolybotError> {
        let mut risk = self.runtime_risk.write().await;
        match key {
            "base_size_usd" => {
                risk.base_size_usd = value.parse().map_err(|_| {
                    polybot_common::errors::PolybotError::Config(
                        "invalid decimal for base_size_usd".to_string(),
                    )
                })?
            }
            "daily_max_loss_pct" => {
                risk.daily_max_loss_pct = value.parse().map_err(|_| {
                    polybot_common::errors::PolybotError::Config(
                        "invalid decimal for daily_max_loss_pct".to_string(),
                    )
                })?
            }
            "per_market_exposure_pct" => {
                risk.per_market_exposure_pct = value.parse().map_err(|_| {
                    polybot_common::errors::PolybotError::Config(
                        "invalid decimal for per_market_exposure_pct".to_string(),
                    )
                })?
            }
            "per_category_exposure_pct" => {
                risk.per_category_exposure_pct = value.parse().map_err(|_| {
                    polybot_common::errors::PolybotError::Config(
                        "invalid decimal for per_category_exposure_pct".to_string(),
                    )
                })?
            }
            "max_position_size_usd" => {
                risk.max_position_size_usd = value.parse().map_err(|_| {
                    polybot_common::errors::PolybotError::Config(
                        "invalid decimal for max_position_size_usd".to_string(),
                    )
                })?
            }
            "max_concurrent_positions" => {
                risk.max_concurrent_positions = value.parse().map_err(|_| {
                    polybot_common::errors::PolybotError::Config(
                        "invalid integer for max_concurrent_positions".to_string(),
                    )
                })?
            }
            "min_confidence" => {
                risk.min_confidence = value.parse().map_err(|_| {
                    polybot_common::errors::PolybotError::Config(
                        "invalid integer for min_confidence".to_string(),
                    )
                })?
            }
            "min_secret_level" => {
                risk.min_secret_level = value.parse().map_err(|_| {
                    polybot_common::errors::PolybotError::Config(
                        "invalid integer for min_secret_level".to_string(),
                    )
                })?
            }
            "slippage_threshold" => {
                risk.slippage_threshold = value.parse().map_err(|_| {
                    polybot_common::errors::PolybotError::Config(
                        "invalid decimal for slippage_threshold".to_string(),
                    )
                })?
            }
            "position_multiplier" => {
                risk.position_multiplier = value.parse().map_err(|_| {
                    polybot_common::errors::PolybotError::Config(
                        "invalid decimal for position_multiplier".to_string(),
                    )
                })?
            }
            "min_trade_size_usdc" => {
                risk.min_trade_size_usdc = value.parse().map_err(|_| {
                    polybot_common::errors::PolybotError::Config(
                        "invalid decimal for min_trade_size_usdc".to_string(),
                    )
                })?
            }
            "min_usdc_balance" => {
                risk.min_usdc_balance = value.parse().map_err(|_| {
                    polybot_common::errors::PolybotError::Config(
                        "invalid decimal for min_usdc_balance".to_string(),
                    )
                })?
            }
            "max_consecutive_losses" => {
                risk.max_consecutive_losses = value.parse().map_err(|_| {
                    polybot_common::errors::PolybotError::Config(
                        "invalid integer for max_consecutive_losses".to_string(),
                    )
                })?
            }
            "loss_cooldown_secs" => {
                risk.loss_cooldown_secs = value.parse().map_err(|_| {
                    polybot_common::errors::PolybotError::Config(
                        "invalid integer for loss_cooldown_secs".to_string(),
                    )
                })?
            }
            other => {
                return Err(polybot_common::errors::PolybotError::Config(format!(
                    "unsupported runtime config key: {}",
                    other
                )))
            }
        }
        Ok(format!("{} updated to {}", key, value))
    }

    pub async fn runtime_config_summary(&self) -> String {
        let risk = self.runtime_risk.read().await;
        format!(
            "base_size_usd={} daily_max_loss_pct={} per_market_exposure_pct={} per_category_exposure_pct={} max_position_size_usd={} max_concurrent_positions={} min_confidence={} min_secret_level={} slippage_threshold={} position_multiplier={} min_trade_size_usdc={} min_usdc_balance={} max_consecutive_losses={} loss_cooldown_secs={}",
            risk.base_size_usd,
            risk.daily_max_loss_pct,
            risk.per_market_exposure_pct,
            risk.per_category_exposure_pct,
            risk.max_position_size_usd,
            risk.max_concurrent_positions,
            risk.min_confidence,
            risk.min_secret_level,
            risk.slippage_threshold,
            risk.position_multiplier,
            risk.min_trade_size_usdc,
            risk.min_usdc_balance,
            risk.max_consecutive_losses,
            risk.loss_cooldown_secs,
        )
    }
}

use rust_decimal_macros::dec;

pub async fn run_risk_engine(
    engine: Arc<RiskEngine>,
    metrics: Arc<Metrics>,
    mut receiver: mpsc::Receiver<polybot_common::types::ScannerEvent>,
    sender: mpsc::Sender<RiskDecision>,
) -> Result<(), polybot_common::errors::PolybotError> {
    while let Some(event) = receiver.recv().await {
        metrics.record_signal_received();
        metrics.broadcast_event(
            "signal_received",
            serde_json::json!({
                "signal_id": &event.signal.signal_id,
                "wallet": &event.signal.wallet_address,
                "market_id": &event.signal.market_id,
                "confidence": event.signal.confidence,
                "side": format!("{:?}", event.signal.side),
            }),
        );
        let decision = engine.evaluate(&event.signal).await;

        if matches!(decision.decision, Decision::Execute) {
            metrics.record_signal_processed();
        }

        let sqlite_path =
            std::env::var("POLYBOT_SQLITE_PATH").unwrap_or_else(|_| "./polybot.db".to_string());
        if let Ok(store) = SqliteStore::open(std::path::Path::new(&sqlite_path)) {
            let disposition = match &decision.decision {
                Decision::Execute => "execute".to_string(),
                Decision::ManualReview => "manual_review".to_string(),
                Decision::EmergencyStop => "emergency_stop".to_string(),
                Decision::Skip(reason) => format!("skip:{}", reason),
            };
            let category_str = event.signal.category.to_string();
            let side_str = match event.signal.side {
                polybot_common::types::Side::Yes => "YES",
                polybot_common::types::Side::No => "NO",
            };
            if let Err(e) = store.insert_signal_log(&crate::state::sqlite::SignalLogInsert {
                signal_id: &event.signal.signal_id,
                timestamp: &event.signal.timestamp,
                wallet_address: &event.signal.wallet_address,
                market_id: &event.signal.market_id,
                confidence: event.signal.confidence,
                secret_level: event.signal.secret_level,
                category: &category_str,
                side: side_str,
                disposition: &disposition,
            }) {
                tracing::error!(error = %e, "Failed to persist signal log to SQLite");
            }
            // v3.0: Also persist to PRD-compliant signals table
            let signal_source = match event.signal.source {
                polybot_common::types::SignalSource::Websocket => "websocket",
                polybot_common::types::SignalSource::Polling => "polling",
                polybot_common::types::SignalSource::Http => "http",
                polybot_common::types::SignalSource::Manual => "manual",
                polybot_common::types::SignalSource::Redis => "redis",
            };
            let status = match &decision.decision {
                Decision::Execute => "executed",
                Decision::ManualReview => "pending",
                Decision::EmergencyStop => "rejected",
                Decision::Skip(_) => "rejected",
            };
            let outcome = match event.signal.side {
                polybot_common::types::Side::Yes => "YES",
                polybot_common::types::Side::No => "NO",
            };
            if let Err(e) = store.insert_signal(&event.signal, signal_source, outcome, status) {
                tracing::error!(error = %e, "Failed to persist signal to PRD signals table");
            }
        }

        tracing::info!(
            signal_id = %decision.signal_id,
            decision = ?decision.decision,
            size = %decision.position_size_usd,
            blocked = decision.blocked,
            manual_review = decision.manual_review,
            "Risk decision made"
        );
        metrics.broadcast_event(
            "risk_decision",
            serde_json::json!({
                "signal_id": &decision.signal_id,
                "decision": format!("{:?}", decision.decision),
                "size": decision.position_size_usd.to_string(),
                "blocked": decision.blocked,
            }),
        );

        if sender.send(decision).await.is_err() {
            tracing::error!("Execution channel closed");
            return Err(polybot_common::errors::PolybotError::ChannelClosed);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::positions::PositionManager;
    use crate::state::sqlite::SqliteStore;
    use polybot_common::types::{Category, Side, TradeDirection};
    use rust_decimal_macros::dec;

    fn test_signal_with_direction(direction: TradeDirection) -> Signal {
        Signal {
            signal_id: "signal-1".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            wallet_address: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "market-1".to_string(),
            side: Side::Yes,
            direction,
            confidence: 6,
            secret_level: 6,
            category: Category::Politics,
            source: polybot_common::types::SignalSource::Manual,
            tx_hash: None,
            token_id: None,
            target_price: Some(dec!(0.50)),
            target_size_usdc: None,
            target_size_tokens: Some(dec!(5)),
            resolved: false,
            redeemable: false,
            suggested_size_usdc: Some(dec!(50)),
            fee_schedule: None,
            scanner_version: "1.0.0".to_string(),
        }
    }

    fn test_signal() -> Signal {
        test_signal_with_direction(TradeDirection::Buy)
    }

    fn engine_with_config(mut config: AppConfig) -> RiskEngine {
        config.system.simulation = true;
        RiskEngine::new(
            Arc::new(config),
            Arc::new(Metrics::new()),
            Arc::new(Mutex::new(PositionManager::new())),
            None,
        )
    }

    #[tokio::test]
    async fn runtime_position_multiplier_drives_live_sizing() {
        let mut config = AppConfig::default();
        config.risk.position_multiplier = dec!(2.0);
        let engine = engine_with_config(config);

        let decision = engine.evaluate(&test_signal()).await;
        assert!(matches!(decision.decision, Decision::Execute));
        assert_eq!(decision.position_size_usd, dec!(100));
    }

    #[tokio::test]
    async fn sell_signal_without_matching_copied_lot_is_skipped() {
        let sqlite_path =
            std::env::temp_dir().join(format!("polybot-risk-{}.db", uuid::Uuid::new_v4()));
        let _store = SqliteStore::open(&sqlite_path).unwrap();
        std::env::set_var("POLYBOT_SQLITE_PATH", &sqlite_path);

        let engine = engine_with_config(AppConfig::default());
        let signal = test_signal_with_direction(TradeDirection::Sell);

        let decision = engine.evaluate(&signal).await;

        assert!(matches!(decision.decision, Decision::Skip(_)));

        let _ = std::fs::remove_file(sqlite_path);
        std::env::remove_var("POLYBOT_SQLITE_PATH");
    }

    #[test]
    fn mirrored_sell_fraction_uses_remaining_source_position() {
        let exit_tokens = mirrored_exit_tokens(dec!(10), dec!(2), Some(dec!(8))).unwrap();

        assert_eq!(exit_tokens, dec!(2));
    }

    #[tokio::test]
    async fn sell_signal_skips_when_remaining_position_lookup_is_unavailable() {
        let sqlite_path =
            std::env::temp_dir().join(format!("polybot-risk-{}.db", uuid::Uuid::new_v4()));
        let store = SqliteStore::open(&sqlite_path).unwrap();
        std::env::set_var("POLYBOT_SQLITE_PATH", &sqlite_path);

        let opened_at = chrono::Utc::now();
        store
            .upsert_copied_lot(&crate::state::sqlite::CopiedLotRow {
                id: "lot-1".to_string(),
                source_wallet: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
                market_id: "market-1".to_string(),
                side: "YES".to_string(),
                current_size: dec!(4),
                average_price: dec!(0.5),
                opened_at: opened_at.to_rfc3339(),
                updated_at: opened_at.to_rfc3339(),
                last_signal_id: Some("signal-0".to_string()),
                last_tx_hash: None,
            })
            .unwrap();

        let mut config = AppConfig::default();
        config.scanner.data_api_url = "http://127.0.0.1:9".to_string();
        let engine = engine_with_config(config);
        let mut signal = test_signal_with_direction(TradeDirection::Sell);
        signal.target_size_tokens = Some(dec!(8));

        let decision = engine.evaluate(&signal).await;

        assert!(matches!(decision.decision, Decision::Skip(_)));

        let _ = std::fs::remove_file(sqlite_path);
        std::env::remove_var("POLYBOT_SQLITE_PATH");
    }

    #[tokio::test]
    async fn min_usdc_balance_blocks_runtime_flow() {
        let mut config = AppConfig::default();
        config.risk.min_usdc_balance = dec!(200);
        let engine = engine_with_config(config);
        engine.update_portfolio_balance(dec!(100)).await;

        let decision = engine.evaluate(&test_signal()).await;
        assert!(matches!(decision.decision, Decision::EmergencyStop));
        assert!(decision.blocked);
    }

    #[tokio::test]
    async fn max_consecutive_losses_triggers_cooldown() {
        let mut config = AppConfig::default();
        config.risk.max_consecutive_losses = 2;
        config.risk.loss_cooldown_secs = 60;
        let engine = engine_with_config(config);

        engine.record_realized_outcome(dec!(-1)).await;
        engine.record_realized_outcome(dec!(-1)).await;

        let decision = engine.evaluate(&test_signal()).await;
        match decision.decision {
            Decision::Skip(reason) => assert!(reason.contains("cooldown")),
            other => panic!("expected cooldown skip, got {:?}", other),
        }
        assert!(decision.blocked);
    }
}
