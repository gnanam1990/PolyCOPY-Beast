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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub system: SystemConfig,
    pub risk: RiskConfig,
    pub scanner: ScannerConfig,
    pub execution: ExecutionConfig,
    pub telegram: TelegramConfig,
    pub dashboard: DashboardConfig,
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
    pub rpc_endpoints: Vec<String>,
    pub ws_reconnect_max_wait_secs: u64,
    pub heartbeat_interval_secs: u64,
    pub order_timeout_secs: u64,
    #[serde(default = "default_price_buffer")]
    pub price_buffer: Decimal,
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
                rpc_endpoints: vec!["https://polygon-rpc.com".to_string()],
                ws_reconnect_max_wait_secs: 60,
                heartbeat_interval_secs: C::WS_HEARTBEAT_SECS,
                order_timeout_secs: C::ORDER_TIMEOUT_SECS,
                price_buffer: default_price_buffer(),
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
        if self.execution.rpc_endpoints.len() < 2 {
            tracing::warn!(
                "v2.5 requires minimum 2 RPC endpoints. Only {} configured.",
                self.execution.rpc_endpoints.len()
            );
        }
        if self.execution.rpc_endpoints.is_empty() {
            return Err(PolybotError::Config(
                "At least one RPC endpoint required".to_string(),
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
        if let Ok(val) = std::env::var("POLYGON_RPC_URL") {
            self.execution.rpc_endpoints = vec![val];
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

        self.reconcile_system_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let config = AppConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn invalid_base_size_rejected() {
        let mut config = AppConfig::default();
        config.risk.base_size_usd = Decimal::ZERO;
        config.risk.base_size_pct = Decimal::ZERO;
        assert!(config.validate().is_err());
    }

    #[test]
    fn empty_rpc_endpoints_rejected() {
        let mut config = AppConfig::default();
        config.execution.rpc_endpoints = vec![];
        assert!(config.validate().is_err());
    }

    #[test]
    fn max_concurrent_positions_default() {
        let config = AppConfig::default();
        assert_eq!(config.risk.max_concurrent_positions, 20);
    }

    #[test]
    fn apply_env_overrides_simulation() {
        std::env::set_var("POLYBOT_SIMULATION", "true");
        let mut config = AppConfig::default();
        config.apply_env_overrides();
        assert!(config.system.simulation);
        assert_eq!(config.system.execution_mode, ExecutionMode::Simulation);
        std::env::remove_var("POLYBOT_SIMULATION");
    }

    #[test]
    fn apply_env_telegram_user_ids() {
        std::env::set_var("POLYBOT_TELEGRAM_ALLOWED_USER_IDS", "123,456,789");
        let mut config = AppConfig::default();
        config.apply_env_overrides();
        assert_eq!(config.telegram.allowed_user_ids, vec![123, 456, 789]);
        std::env::remove_var("POLYBOT_TELEGRAM_ALLOWED_USER_IDS");
    }

    #[test]
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
}
