use polybot_common::constants as C;
use polybot_common::errors::PolybotError;
use polybot_common::types::{Category, ExecutionMode};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::path::Path;

fn default_execution_mode() -> ExecutionMode {
    C::DEFAULT_EXECUTION_MODE
}

fn default_price_buffer() -> Decimal {
    rust_decimal_macros::dec!(0.01)
}

fn default_position_multiplier() -> Decimal {
    rust_decimal_macros::dec!(1.0)
}

fn default_min_trade_size_usdc() -> Decimal {
    rust_decimal_macros::dec!(1.0)
}

fn default_min_usdc_balance() -> Decimal {
    rust_decimal_macros::dec!(20)
}

fn default_max_consecutive_losses() -> u32 {
    5
}

fn default_loss_cooldown_secs() -> u64 {
    3600
}

fn default_data_api_url() -> String {
    "https://data-api.polymarket.com".to_string()
}

fn default_poll_interval_ms() -> u64 {
    2000
}

fn default_signal_max_age_secs() -> u64 {
    30
}

fn default_use_websocket() -> bool {
    true
}

fn default_fok_max_fee_bps() -> u32 {
    50
}

fn default_max_position_politics_usdc() -> Decimal {
    rust_decimal_macros::dec!(250)
}
fn default_max_position_crypto_usdc() -> Decimal {
    rust_decimal_macros::dec!(150)
}
fn default_max_position_sports_usdc() -> Decimal {
    rust_decimal_macros::dec!(200)
}
fn default_max_position_other_usdc() -> Decimal {
    rust_decimal_macros::dec!(100)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub system: SystemConfig,
    pub risk: RiskConfig,
    pub scanner: ScannerConfig,
    pub execution: ExecutionConfig,
    pub telegram: TelegramConfig,
    pub dashboard: DashboardConfig,
    #[serde(default)]
    pub relayer: Option<RelayerConfig>,
    #[serde(default = "default_collateral_config")]
    pub collateral: CollateralConfig,
    #[serde(default)]
    pub builder: Option<BuilderConfig>,
    #[serde(default)]
    pub reconciliation: ReconciliationConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemConfig {
    pub simulation: bool,
    pub log_level: String,
    #[serde(default = "default_execution_mode")]
    pub execution_mode: ExecutionMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    pub base_size_usd: Decimal,
    pub base_size_pct: Decimal,
    pub daily_max_loss_pct: Decimal,
    pub per_market_exposure_pct: Decimal,
    pub per_category_exposure_pct: Decimal,
    pub max_position_size_usd: Decimal,
    pub max_concurrent_positions: u32,
    pub max_market_liquidity_pct: Decimal,
    pub min_confidence: u8,
    pub min_secret_level: u8,
    pub slippage_threshold: Decimal,
    #[serde(default = "default_position_multiplier")]
    pub position_multiplier: Decimal,
    #[serde(default = "default_min_trade_size_usdc")]
    pub min_trade_size_usdc: Decimal,
    #[serde(default = "default_min_usdc_balance")]
    pub min_usdc_balance: Decimal,
    #[serde(default = "default_max_consecutive_losses")]
    pub max_consecutive_losses: u32,
    #[serde(default = "default_loss_cooldown_secs")]
    pub loss_cooldown_secs: u64,
    #[serde(default = "default_max_position_politics_usdc")]
    pub max_position_politics_usdc: Decimal,
    #[serde(default = "default_max_position_crypto_usdc")]
    pub max_position_crypto_usdc: Decimal,
    #[serde(default = "default_max_position_sports_usdc")]
    pub max_position_sports_usdc: Decimal,
    #[serde(default = "default_max_position_other_usdc")]
    pub max_position_other_usdc: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannerConfig {
    pub watch_dir: String,
    pub processed_dir: String,
    pub dedup_window_secs: u64,
    pub http_port: u16,
    #[serde(default = "default_data_api_url")]
    pub data_api_url: String,
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_signal_max_age_secs")]
    pub signal_max_age_secs: u64,
    #[serde(default = "default_use_websocket")]
    pub use_websocket: bool,
    #[serde(default)]
    pub target_categories: Vec<Category>,
    #[serde(default)]
    pub target_wallets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    pub slippage_threshold: Decimal,
    pub ws_reconnect_max_wait_secs: u64,
    pub heartbeat_interval_secs: u64,
    pub order_timeout_secs: u64,
    #[serde(default = "default_price_buffer")]
    pub price_buffer: Decimal,
    /// V2: taker-fee ceiling (true bps, PRD units / 100) above which the
    /// order router will not emit FOK (taker) orders and falls back to GTC.
    /// Set to 0 to disable FOK entirely.
    #[serde(default = "default_fok_max_fee_bps")]
    pub fok_max_fee_bps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub allowed_user_ids: Vec<u64>,
    pub command_rate_limit_per_min: u32,
    pub emergency_stop_limit_per_hour: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayerConfig {
    pub url: String,
    pub api_key: String,
    pub api_key_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CollateralConfig {
    /// V2 collateral token symbol. Always `"pUSD"` in v3.2.
    pub token: String,
    /// Address of the Collateral Onramp contract (USDC.e → pUSD wrapping).
    /// Empty until provided via COLLATERAL_ONRAMP_ADDRESS env var.
    pub onramp_address: String,
    /// Polygon USDC.e token address. Defaults to the canonical
    /// 0x2791Bca1... per PRD §6.
    pub usdc_e_address: String,
}

fn default_collateral_config() -> CollateralConfig {
    CollateralConfig {
        token: "pUSD".to_string(),
        onramp_address: String::new(),
        usdc_e_address: "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174".to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BuilderConfig {
    /// V2 builder code (bytes32 hex, 0x-prefixed, 66 chars).
    /// Included in every V2 order's `builder` field.
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconciliationConfig {
    /// When true, full reconciliation cycles may OVERWRITE in-memory
    /// PositionManager state from the reference snapshot. When false
    /// (default), drift is detected and alerted but no state mutation
    /// happens.
    ///
    /// Enable only after verifying the log-only output for at least
    /// 2 weeks under real load, to confirm the reference source
    /// (data-api) doesn't periodically lag behind in-memory state in
    /// ways that would wrongly "heal" real positions.
    #[serde(default = "default_auto_heal")]
    pub auto_heal: bool,
}

fn default_auto_heal() -> bool {
    // CONSERVATIVE DEFAULT — do not mutate state without operator opt-in.
    false
}

impl Default for ReconciliationConfig {
    fn default() -> Self {
        Self {
            auto_heal: default_auto_heal(),
        }
    }
}
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            system: SystemConfig {
                simulation: true,
                log_level: "info".to_string(),
                execution_mode: default_execution_mode(),
            },
            risk: RiskConfig {
                base_size_usd: rust_decimal_macros::dec!(50),
                base_size_pct: C::DEFAULT_BASE_SIZE_PCT,
                daily_max_loss_pct: C::DEFAULT_DAILY_MAX_LOSS_PCT,
                per_market_exposure_pct: C::DEFAULT_PER_MARKET_EXPOSURE_PCT,
                per_category_exposure_pct: C::DEFAULT_PER_CATEGORY_EXPOSURE_PCT,
                max_position_size_usd: C::MAX_POSITION_USDC,
                max_concurrent_positions: C::MAX_CONCURRENT_POSITIONS,
                max_market_liquidity_pct: C::MAX_MARKET_LIQUIDITY_PCT,
                min_confidence: C::DEFAULT_MIN_CONFIDENCE,
                min_secret_level: C::DEFAULT_MIN_SECRET_LEVEL,
                slippage_threshold: C::DEFAULT_SLIPPAGE_THRESHOLD,
                position_multiplier: default_position_multiplier(),
                min_trade_size_usdc: default_min_trade_size_usdc(),
                min_usdc_balance: default_min_usdc_balance(),
                max_consecutive_losses: default_max_consecutive_losses(),
                loss_cooldown_secs: default_loss_cooldown_secs(),
                max_position_politics_usdc: default_max_position_politics_usdc(),
                max_position_crypto_usdc: default_max_position_crypto_usdc(),
                max_position_sports_usdc: default_max_position_sports_usdc(),
                max_position_other_usdc: default_max_position_other_usdc(),
            },
            scanner: ScannerConfig {
                watch_dir: "./signals".to_string(),
                processed_dir: "./signals/processed".to_string(),
                dedup_window_secs: C::DEFAULT_DEDUP_WINDOW_SECS,
                http_port: 8081,
                data_api_url: default_data_api_url(),
                poll_interval_ms: default_poll_interval_ms(),
                signal_max_age_secs: default_signal_max_age_secs(),
                use_websocket: default_use_websocket(),
                target_categories: vec![],
                target_wallets: vec![],
            },
            execution: ExecutionConfig {
                slippage_threshold: C::DEFAULT_SLIPPAGE_THRESHOLD,
                ws_reconnect_max_wait_secs: 60,
                heartbeat_interval_secs: C::WS_HEARTBEAT_SECS,
                order_timeout_secs: C::ORDER_TIMEOUT_SECS,
                price_buffer: default_price_buffer(),
                fok_max_fee_bps: default_fok_max_fee_bps(),
            },
            telegram: TelegramConfig {
                allowed_user_ids: vec![],
                command_rate_limit_per_min: 30,
                emergency_stop_limit_per_hour: 3,
            },
            dashboard: DashboardConfig {
                host: "0.0.0.0".to_string(),
                port: 8080,
            },
            relayer: None,
            collateral: default_collateral_config(),
            builder: None,
            reconciliation: ReconciliationConfig::default(),
        }
    }
}

impl AppConfig {
    fn reconcile_system_mode(&mut self) {
        if self.system.execution_mode == ExecutionMode::Simulation && !self.system.simulation {
            self.system.execution_mode = ExecutionMode::Live;
        }
        self.system.simulation = matches!(self.system.execution_mode, ExecutionMode::Simulation);
    }

    pub fn load_from_file(path: &Path) -> Result<Self, PolybotError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            PolybotError::Config(format!(
                "Failed to read config file {}: {}",
                path.display(),
                e
            ))
        })?;

        let mut config: AppConfig = toml::from_str(&content)
            .map_err(|e| PolybotError::Config(format!("Failed to parse config: {}", e)))?;
        config.reconcile_system_mode();
        config.validate()?;
        Ok(config)
    }

    pub fn load() -> Result<Self, PolybotError> {
        let env_path = Path::new("config.toml");
        if env_path.exists() {
            Self::load_from_file(env_path)
        } else {
            tracing::info!("No config.toml found, using defaults");
            let mut config = Self::default();
            config.reconcile_system_mode();
            config.validate()?;
            Ok(config)
        }
    }

    pub fn validate(&self) -> Result<(), PolybotError> {
        if self.risk.base_size_usd <= Decimal::ZERO && self.risk.base_size_pct <= Decimal::ZERO {
            return Err(PolybotError::Config(
                "base_size_usd or base_size_pct must be positive".to_string(),
            ));
        }
        if self.risk.daily_max_loss_pct <= Decimal::ZERO
            || self.risk.daily_max_loss_pct > Decimal::ONE
        {
            return Err(PolybotError::Config(
                "daily_max_loss_pct must be between 0 and 1".to_string(),
            ));
        }
        if self.risk.min_confidence < 1 || self.risk.min_confidence > 10 {
            return Err(PolybotError::Config(
                "min_confidence must be between 1 and 10".to_string(),
            ));
        }
        if self.risk.max_concurrent_positions == 0 {
            return Err(PolybotError::Config(
                "max_concurrent_positions must be > 0".to_string(),
            ));
        }
        if self.risk.position_multiplier <= Decimal::ZERO {
            return Err(PolybotError::Config(
                "position_multiplier must be > 0".to_string(),
            ));
        }
        if self.risk.min_trade_size_usdc <= Decimal::ZERO {
            return Err(PolybotError::Config(
                "min_trade_size_usdc must be > 0".to_string(),
            ));
        }
        if self.risk.min_usdc_balance < Decimal::ZERO {
            return Err(PolybotError::Config(
                "min_usdc_balance must be >= 0".to_string(),
            ));
        }
        if self.risk.max_consecutive_losses == 0 {
            return Err(PolybotError::Config(
                "max_consecutive_losses must be > 0".to_string(),
            ));
        }
        if self.execution.price_buffer < Decimal::ZERO {
            return Err(PolybotError::Config(
                "execution.price_buffer must be >= 0".to_string(),
            ));
        }
        if self.scanner.dedup_window_secs == 0 {
            return Err(PolybotError::Config(
                "dedup_window_secs must be > 0".to_string(),
            ));
        }
        if self.scanner.poll_interval_ms == 0 {
            return Err(PolybotError::Config(
                "poll_interval_ms must be > 0".to_string(),
            ));
        }
        if self.scanner.signal_max_age_secs == 0 {
            return Err(PolybotError::Config(
                "signal_max_age_secs must be > 0".to_string(),
            ));
        }
        Ok(())
    }

    pub fn apply_env_overrides(&mut self) {
        if let Ok(val) = std::env::var("POLYBOT_EXECUTION_MODE") {
            self.system.execution_mode = match val.to_lowercase().as_str() {
                "simulation" => ExecutionMode::Simulation,
                "shadow" => ExecutionMode::Shadow,
                "live" => ExecutionMode::Live,
                _ => self.system.execution_mode,
            };
        }
        if let Ok(val) = std::env::var("POLYBOT_SIMULATION") {
            let simulation = val.to_lowercase() == "true" || val == "1";
            self.system.execution_mode = if simulation {
                ExecutionMode::Simulation
            } else {
                ExecutionMode::Live
            };
            self.system.simulation = simulation;
        }
        if let Ok(val) = std::env::var("POLYBOT_LOG_LEVEL") {
            self.system.log_level = val;
        }
        if let Ok(val) =
            std::env::var("POLYBOT_DATA_API_URL").or_else(|_| std::env::var("DATA_API_URL"))
        {
            self.scanner.data_api_url = val;
        }
        if let Ok(val) =
            std::env::var("POLYBOT_POLL_INTERVAL_MS").or_else(|_| std::env::var("POLL_INTERVAL_MS"))
        {
            if let Ok(parsed) = val.parse::<u64>() {
                self.scanner.poll_interval_ms = parsed;
            }
        }
        if let Ok(val) = std::env::var("POLYBOT_SIGNAL_MAX_AGE_SECS")
            .or_else(|_| std::env::var("SIGNAL_MAX_AGE_SECS"))
        {
            if let Ok(parsed) = val.parse::<u64>() {
                self.scanner.signal_max_age_secs = parsed;
            }
        }
        if let Ok(val) =
            std::env::var("POLYBOT_USE_WEBSOCKET").or_else(|_| std::env::var("USE_WEBSOCKET"))
        {
            let normalized = val.to_lowercase();
            self.scanner.use_websocket = normalized == "true" || normalized == "1";
        }
        if let Ok(val) = std::env::var("POLYBOT_TARGET_CATEGORIES") {
            self.scanner.target_categories = val
                .split(',')
                .filter_map(|category| Category::try_from(category.trim()).ok())
                .collect();
        }
        if let Ok(val) = std::env::var("POLYBOT_TARGET_WALLETS") {
            self.scanner.target_wallets = val
                .split(',')
                .map(|w| w.trim().to_lowercase())
                .filter(|w| !w.is_empty())
                .collect();
        }
        if let Ok(val) = std::env::var("POLYBOT_BASE_SIZE_USD") {
            if let Ok(d) = val.parse::<Decimal>() {
                self.risk.base_size_usd = d;
            }
        }
        if let Ok(val) = std::env::var("POLYBOT_POSITION_MULTIPLIER") {
            if let Ok(d) = val.parse::<Decimal>() {
                self.risk.position_multiplier = d;
            }
        }
        if let Ok(val) = std::env::var("POLYBOT_MIN_TRADE_SIZE_USDC") {
            if let Ok(d) = val.parse::<Decimal>() {
                self.risk.min_trade_size_usdc = d;
            }
        }
        if let Ok(val) = std::env::var("POLYBOT_MIN_USDC_BALANCE") {
            if let Ok(d) = val.parse::<Decimal>() {
                self.risk.min_usdc_balance = d;
            }
        }
        if let Ok(val) = std::env::var("POLYBOT_MAX_CONSECUTIVE_LOSSES") {
            if let Ok(d) = val.parse::<u32>() {
                self.risk.max_consecutive_losses = d;
            }
        }
        if let Ok(val) = std::env::var("POLYBOT_LOSS_COOLDOWN_SECS") {
            if let Ok(d) = val.parse::<u64>() {
                self.risk.loss_cooldown_secs = d;
            }
        }
        if let Ok(val) = std::env::var("POLYBOT_PRICE_BUFFER") {
            if let Ok(d) = val.parse::<Decimal>() {
                self.execution.price_buffer = d;
            }
        }
        if let Ok(val) = std::env::var("TELEGRAM_ALLOWED_USER_IDS")
            .or_else(|_| std::env::var("POLYBOT_TELEGRAM_ALLOWED_USER_IDS"))
        {
            self.telegram.allowed_user_ids = val
                .split(',')
                .filter_map(|s| s.trim().parse::<u64>().ok())
                .collect();
        }
        if let Ok(val) = std::env::var("POLYBOT_RECONCILIATION_AUTO_HEAL") {
            let normalized = val.to_lowercase();
            self.reconciliation.auto_heal = normalized == "true" || normalized == "1";
        }

        // V2 relayer config: all three vars must be present.
        let relayer_url = std::env::var("RELAYER_URL").ok();
        let relayer_api_key = std::env::var("RELAYER_API_KEY").ok();
        let relayer_api_key_address = std::env::var("RELAYER_API_KEY_ADDRESS").ok();
        self.relayer = match (relayer_url, relayer_api_key, relayer_api_key_address) {
            (Some(url), Some(api_key), Some(api_key_address)) => Some(RelayerConfig {
                url,
                api_key,
                api_key_address,
            }),
            (None, None, None) => None,
            _ => {
                tracing::warn!(
                    "Partial RELAYER_* config detected; requires all of RELAYER_URL, RELAYER_API_KEY, RELAYER_API_KEY_ADDRESS. Ignoring."
                );
                None
            }
        };

        if let Ok(val) = std::env::var("COLLATERAL_ONRAMP_ADDRESS") {
            self.collateral.onramp_address = val;
        }
        if let Ok(val) = std::env::var("USDC_E_ADDRESS") {
            self.collateral.usdc_e_address = val;
        }
        if let Ok(val) = std::env::var("COLLATERAL_TOKEN") {
            self.collateral.token = val;
        }

        if let Ok(code) = std::env::var("BUILDER_CODE") {
            self.builder = Some(BuilderConfig { code });
        }

        if let Ok(val) = std::env::var("FOK_MAX_FEE_BPS") {
            if let Ok(parsed) = val.parse::<u32>() {
                self.execution.fok_max_fee_bps = parsed;
            }
        }

        if let Ok(val) = std::env::var("MAX_POSITION_POLITICS_USDC") {
            if let Ok(d) = val.parse::<Decimal>() {
                self.risk.max_position_politics_usdc = d;
            }
        }
        if let Ok(val) = std::env::var("MAX_POSITION_CRYPTO_USDC") {
            if let Ok(d) = val.parse::<Decimal>() {
                self.risk.max_position_crypto_usdc = d;
            }
        }
        if let Ok(val) = std::env::var("MAX_POSITION_SPORTS_USDC") {
            if let Ok(d) = val.parse::<Decimal>() {
                self.risk.max_position_sports_usdc = d;
            }
        }
        if let Ok(val) = std::env::var("MAX_POSITION_OTHER_USDC") {
            if let Ok(d) = val.parse::<Decimal>() {
                self.risk.max_position_other_usdc = d;
            }
        }

        self.reconcile_system_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn default_config_is_valid() {
        let config = AppConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn legacy_config_without_reconciliation_uses_safe_default() {
        let raw = r#"
[system]
simulation = true
execution_mode = "simulation"
log_level = "info"

[risk]
base_size_usd = 50
base_size_pct = 0.015
daily_max_loss_pct = 0.05
per_market_exposure_pct = 0.10
per_category_exposure_pct = 0.25
max_position_size_usd = 500
max_concurrent_positions = 20
max_market_liquidity_pct = 0.02
min_confidence = 6
min_secret_level = 5
slippage_threshold = 0.02

[scanner]
watch_dir = "./signals"
processed_dir = "./signals/processed"
dedup_window_secs = 300
http_port = 8081

[execution]
slippage_threshold = 0.02
ws_reconnect_max_wait_secs = 60
heartbeat_interval_secs = 30
order_timeout_secs = 30

[telegram]
allowed_user_ids = []
command_rate_limit_per_min = 30
emergency_stop_limit_per_hour = 3

[dashboard]
host = "0.0.0.0"
port = 8080
"#;

        let config: AppConfig = toml::from_str(raw).expect("legacy config should parse");
        assert!(!config.reconciliation.auto_heal);
    }

    #[test]
    fn invalid_base_size_rejected() {
        let mut config = AppConfig::default();
        config.risk.base_size_usd = Decimal::ZERO;
        config.risk.base_size_pct = Decimal::ZERO;
        assert!(config.validate().is_err());
    }

    #[test]
    fn max_concurrent_positions_default() {
        let config = AppConfig::default();
        assert_eq!(config.risk.max_concurrent_positions, 20);
    }

    #[test]
    #[serial]
    fn apply_env_overrides_simulation() {
        std::env::set_var("POLYBOT_SIMULATION", "true");
        let mut config = AppConfig::default();
        config.apply_env_overrides();
        assert!(config.system.simulation);
        assert_eq!(config.system.execution_mode, ExecutionMode::Simulation);
        std::env::remove_var("POLYBOT_SIMULATION");
    }

    #[test]
    #[serial]
    fn apply_env_telegram_user_ids() {
        std::env::set_var("POLYBOT_TELEGRAM_ALLOWED_USER_IDS", "123,456,789");
        let mut config = AppConfig::default();
        config.apply_env_overrides();
        assert_eq!(config.telegram.allowed_user_ids, vec![123, 456, 789]);
        std::env::remove_var("POLYBOT_TELEGRAM_ALLOWED_USER_IDS");
    }

    #[test]
    #[serial]
    fn apply_env_target_wallets() {
        std::env::set_var("POLYBOT_TARGET_WALLETS", "0xabc, 0xDEF ");
        let mut config = AppConfig::default();
        config.apply_env_overrides();
        assert_eq!(config.scanner.target_wallets, vec!["0xabc", "0xdef"]);
        std::env::remove_var("POLYBOT_TARGET_WALLETS");
    }

    #[test]
    fn runtime_risk_defaults_are_valid() {
        let config = AppConfig::default();
        assert_eq!(
            config.risk.position_multiplier,
            rust_decimal_macros::dec!(1.0)
        );
        assert_eq!(
            config.risk.min_trade_size_usdc,
            rust_decimal_macros::dec!(1.0)
        );
        assert_eq!(config.risk.min_usdc_balance, rust_decimal_macros::dec!(20));
        assert_eq!(config.risk.max_consecutive_losses, 5);
        assert_eq!(config.risk.loss_cooldown_secs, 3600);
    }

    #[test]
    fn module2_scanner_defaults_are_valid() {
        let config = AppConfig::default();
        assert_eq!(
            config.scanner.data_api_url,
            "https://data-api.polymarket.com"
        );
        assert_eq!(config.scanner.poll_interval_ms, 2000);
        assert_eq!(config.scanner.signal_max_age_secs, 30);
        assert!(config.scanner.use_websocket);
        assert!(config.scanner.target_categories.is_empty());
    }

    #[test]
    fn relayer_config_is_none_by_default() {
        let config = AppConfig::default();
        assert!(config.relayer.is_none());
    }

    #[test]
    #[serial]
    fn relayer_config_parses_from_env() {
        std::env::set_var("RELAYER_URL", "https://relayer-v2.polymarket.com");
        std::env::set_var("RELAYER_API_KEY", "test-api-key");
        std::env::set_var(
            "RELAYER_API_KEY_ADDRESS",
            "0x1234567890123456789012345678901234567890",
        );
        let mut config = AppConfig::default();
        config.apply_env_overrides();
        let relayer = config
            .relayer
            .as_ref()
            .expect("relayer should be populated");
        assert_eq!(relayer.url, "https://relayer-v2.polymarket.com");
        assert_eq!(relayer.api_key, "test-api-key");
        assert_eq!(
            relayer.api_key_address,
            "0x1234567890123456789012345678901234567890"
        );
        std::env::remove_var("RELAYER_URL");
        std::env::remove_var("RELAYER_API_KEY");
        std::env::remove_var("RELAYER_API_KEY_ADDRESS");
    }

    #[test]
    #[serial]
    fn relayer_config_partial_is_rejected() {
        std::env::set_var("RELAYER_URL", "https://relayer-v2.polymarket.com");
        std::env::remove_var("RELAYER_API_KEY");
        std::env::remove_var("RELAYER_API_KEY_ADDRESS");
        let mut config = AppConfig::default();
        config.apply_env_overrides();
        assert!(
            config.relayer.is_none(),
            "partial relayer config should be None, got {:?}",
            config.relayer
        );
        std::env::remove_var("RELAYER_URL");
    }

    #[test]
    fn collateral_config_has_pusd_defaults() {
        let config = AppConfig::default();
        assert_eq!(config.collateral.token, "pUSD");
        assert_eq!(
            config.collateral.usdc_e_address,
            "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174"
        );
        assert_eq!(config.collateral.onramp_address, "");
    }

    #[test]
    #[serial]
    fn collateral_config_applies_env_overrides() {
        std::env::set_var(
            "COLLATERAL_ONRAMP_ADDRESS",
            "0xabcdef0000000000000000000000000000000000",
        );
        std::env::set_var(
            "USDC_E_ADDRESS",
            "0x1111111111111111111111111111111111111111",
        );
        let mut config = AppConfig::default();
        config.apply_env_overrides();
        assert_eq!(
            config.collateral.onramp_address,
            "0xabcdef0000000000000000000000000000000000"
        );
        assert_eq!(
            config.collateral.usdc_e_address,
            "0x1111111111111111111111111111111111111111"
        );
        std::env::remove_var("COLLATERAL_ONRAMP_ADDRESS");
        std::env::remove_var("USDC_E_ADDRESS");
    }

    #[test]
    fn builder_config_is_none_by_default() {
        let config = AppConfig::default();
        assert!(config.builder.is_none());
    }

    #[test]
    #[serial]
    fn builder_config_parses_from_env() {
        let code = "0x00000000000000000000000000000000000000000000000000000000deadbeef";
        std::env::set_var("BUILDER_CODE", code);
        let mut config = AppConfig::default();
        config.apply_env_overrides();
        let builder = config
            .builder
            .as_ref()
            .expect("builder should be populated");
        assert_eq!(builder.code, code);
        std::env::remove_var("BUILDER_CODE");
    }

    #[test]
    fn fok_max_fee_bps_defaults_to_50() {
        let config = AppConfig::default();
        assert_eq!(config.execution.fok_max_fee_bps, 50);
    }

    #[test]
    #[serial]
    fn fok_max_fee_bps_reads_from_env() {
        std::env::set_var("FOK_MAX_FEE_BPS", "20");
        let mut config = AppConfig::default();
        config.apply_env_overrides();
        assert_eq!(config.execution.fok_max_fee_bps, 20);
        std::env::remove_var("FOK_MAX_FEE_BPS");
    }

    #[test]
    fn category_caps_default_to_prd_values() {
        let config = AppConfig::default();
        assert_eq!(
            config.risk.max_position_politics_usdc,
            rust_decimal_macros::dec!(250)
        );
        assert_eq!(
            config.risk.max_position_crypto_usdc,
            rust_decimal_macros::dec!(150)
        );
        assert_eq!(
            config.risk.max_position_sports_usdc,
            rust_decimal_macros::dec!(200)
        );
        assert_eq!(
            config.risk.max_position_other_usdc,
            rust_decimal_macros::dec!(100)
        );
    }

    #[test]
    #[serial]
    fn category_caps_reads_from_env() {
        std::env::set_var("MAX_POSITION_POLITICS_USDC", "500");
        std::env::set_var("MAX_POSITION_CRYPTO_USDC", "300");
        let mut config = AppConfig::default();
        config.apply_env_overrides();
        assert_eq!(
            config.risk.max_position_politics_usdc,
            rust_decimal_macros::dec!(500)
        );
        assert_eq!(
            config.risk.max_position_crypto_usdc,
            rust_decimal_macros::dec!(300)
        );
        // Unchanged defaults for the others.
        assert_eq!(
            config.risk.max_position_sports_usdc,
            rust_decimal_macros::dec!(200)
        );
        assert_eq!(
            config.risk.max_position_other_usdc,
            rust_decimal_macros::dec!(100)
        );
        std::env::remove_var("MAX_POSITION_POLITICS_USDC");
        std::env::remove_var("MAX_POSITION_CRYPTO_USDC");
    }
}
