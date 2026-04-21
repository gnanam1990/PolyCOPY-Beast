use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Instant;
use uuid::{Uuid, Version};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Politics,
    Sports,
    Crypto,
    Other,
}

impl Category {
    pub fn max_exposure_pct(&self) -> Decimal {
        match self {
            Category::Politics => dec!(0.25),
            Category::Sports => dec!(0.20),
            Category::Crypto => dec!(0.15),
            Category::Other => dec!(0.10),
        }
    }

    pub fn max_single_position_usd(&self) -> Decimal {
        match self {
            Category::Politics => dec!(250),
            Category::Sports => dec!(200),
            Category::Crypto => dec!(150),
            Category::Other => dec!(100),
        }
    }

    pub fn min_confidence_threshold(&self) -> u8 {
        match self {
            Category::Politics => 6,
            Category::Sports => 6,
            Category::Crypto => 7,
            Category::Other => 7,
        }
    }

    pub fn min_secret_level_threshold(&self) -> u8 {
        match self {
            Category::Politics => 5,
            Category::Sports => 5,
            Category::Crypto => 6,
            Category::Other => 7,
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Category::Politics => write!(f, "politics"),
            Category::Sports => write!(f, "sports"),
            Category::Crypto => write!(f, "crypto"),
            Category::Other => write!(f, "other"),
        }
    }
}

impl TryFrom<&str> for Category {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s.to_lowercase().as_str() {
            "politics" => Ok(Category::Politics),
            "sports" => Ok(Category::Sports),
            "crypto" => Ok(Category::Crypto),
            _ => Ok(Category::Other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Side {
    Yes,
    No,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderType {
    Limit,
    Ioc,
    Fok,
    PostOnly,
}

impl OrderType {
    pub fn requires_price_buffer(self) -> bool {
        matches!(self, OrderType::Fok)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    Simulation,
    Shadow,
    Live,
}

impl ExecutionMode {
    pub fn allows_network_market_data(self) -> bool {
        !matches!(self, ExecutionMode::Simulation)
    }

    pub fn allows_ws_market_data(self) -> bool {
        !matches!(self, ExecutionMode::Simulation)
    }

    pub fn allows_live_order_submission(self) -> bool {
        matches!(self, ExecutionMode::Live)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ExecutionMode::Simulation => "simulation",
            ExecutionMode::Shadow => "shadow",
            ExecutionMode::Live => "live",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignalSource {
    Manual,
    Polling,
    Websocket,
    Http,
    Redis,
}

fn default_signal_source() -> SignalSource {
    SignalSource::Manual
}

/// Signal schema (v2.5 base with Module 1 extensions for core fields).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub signal_id: String,
    pub timestamp: String,
    pub wallet_address: String,
    pub market_id: String,
    pub side: Side,
    pub confidence: u8,
    pub secret_level: u8,
    pub category: Category,
    #[serde(default = "default_signal_source")]
    pub source: SignalSource,
    #[serde(default)]
    pub tx_hash: Option<String>,
    #[serde(default)]
    pub token_id: Option<String>,
    #[serde(default)]
    pub target_price: Option<Decimal>,
    #[serde(default)]
    pub target_size_usdc: Option<Decimal>,
    #[serde(default)]
    pub resolved: bool,
    #[serde(default)]
    pub redeemable: bool,
    #[serde(default)]
    pub suggested_size_usdc: Option<Decimal>,
    #[serde(default = "default_scanner_version")]
    pub scanner_version: String,
}

fn default_scanner_version() -> String {
    "1.0.0".to_string()
}

#[derive(Debug, Clone)]
pub struct ScannerEvent {
    pub signal: Signal,
    pub received_at: Instant,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    InvalidSignalId(String),
    InvalidTimestamp(String),
    InvalidWalletAddress(String),
    InvalidMarketId(String),
    InvalidTxHash(String),
    SecretLevelOutOfRange(u8),
    ConfidenceOutOfRange(u8),
    InvalidSide(String),
    StaleTimestamp(String),
    FutureTimestamp(String),
    ResolvedMarket(String),
    RedeemableMarket(String),
    BlockedByConfidenceThreshold {
        confidence: u8,
        category: Category,
    },
    BlockedBySecretLevelThreshold {
        secret_level: u8,
        category: Category,
    },
}

impl Signal {
    /// Validates the signal against v2.5 PRD rules.
    /// Returns Ok(()) if valid, or Err with list of validation errors.
    /// Signals with confidence < 3 or secret_level < 3 are flagged for manual review (not blocked here).
    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        self.validate_with_max_age_secs(30)
    }

    pub fn validate_with_max_age_secs(
        &self,
        max_age_secs: i64,
    ) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();

        // signal_id: must be a valid UUID v4
        if self.signal_id.is_empty()
            || Uuid::parse_str(&self.signal_id)
                .ok()
                .and_then(|id| id.get_version())
                != Some(Version::Random)
        {
            errors.push(ValidationError::InvalidSignalId(self.signal_id.clone()));
        }

        // timestamp: must be valid ISO 8601, not older than 30s from now
        if let Ok(dt) = DateTime::parse_from_rfc3339(&self.timestamp) {
            let age = Utc::now().signed_duration_since(dt);
            if age.num_seconds() > max_age_secs {
                errors.push(ValidationError::StaleTimestamp(self.timestamp.clone()));
            } else if age.num_seconds() < -max_age_secs {
                errors.push(ValidationError::FutureTimestamp(self.timestamp.clone()));
            }
        } else {
            errors.push(ValidationError::InvalidTimestamp(self.timestamp.clone()));
        }

        // wallet_address: must be 0x-prefixed, 42 chars, and hex-encoded
        let is_valid_wallet = self.wallet_address.starts_with("0x")
            && self.wallet_address.len() == 42
            && self.wallet_address[2..]
                .chars()
                .all(|ch| ch.is_ascii_hexdigit());
        if !is_valid_wallet {
            errors.push(ValidationError::InvalidWalletAddress(
                self.wallet_address.clone(),
            ));
        }

        if let Some(tx_hash) = &self.tx_hash {
            let is_valid_hash = tx_hash.starts_with("0x")
                && tx_hash.len() == 66
                && tx_hash[2..].chars().all(|ch| ch.is_ascii_hexdigit());
            if !is_valid_hash {
                errors.push(ValidationError::InvalidTxHash(tx_hash.clone()));
            }
        }

        // market_id: must be non-empty
        if self.market_id.is_empty() {
            errors.push(ValidationError::InvalidMarketId(self.market_id.clone()));
        }

        if self.resolved {
            errors.push(ValidationError::ResolvedMarket(self.market_id.clone()));
        }

        if self.redeemable {
            errors.push(ValidationError::RedeemableMarket(self.market_id.clone()));
        }

        // secret_level: 1-10
        if self.secret_level < 1 || self.secret_level > 10 {
            errors.push(ValidationError::SecretLevelOutOfRange(self.secret_level));
        }

        // confidence: 1-10
        if self.confidence < 1 || self.confidence > 10 {
            errors.push(ValidationError::ConfidenceOutOfRange(self.confidence));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Returns true if this signal should be queued for manual review
    /// (confidence < 3 or secret_level < 3 per v2.5 PRD)
    pub fn requires_manual_review(&self) -> bool {
        self.confidence < 3 || self.secret_level < 3
    }

    /// Returns true if this signal should be blocked entirely
    /// (below per-category confidence/secret_level thresholds)
    pub fn is_blocked_by_category_thresholds(&self) -> bool {
        self.confidence < self.category.min_confidence_threshold()
            || self.secret_level < self.category.min_secret_level_threshold()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "reason")]
pub enum Decision {
    Execute,
    Skip(String),
    ManualReview,
    EmergencyStop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskDecision {
    pub signal_id: String,
    pub market_id: String,
    pub side: Side,
    pub category: Category,
    pub position_size_usd: Decimal,
    pub confidence_multiplier: Decimal,
    pub secret_level_multiplier: Decimal,
    pub drawdown_factor: Decimal,
    pub blocked: bool,
    pub manual_review: bool,
    pub decision: Decision,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PositionStatus {
    Open,
    Closed,
    Ghost, // v2.5: position in Redis but not on-chain
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub id: String,
    pub market_id: String,
    pub side: Side,
    pub entry_price: Decimal,
    pub current_size: Decimal,
    pub average_price: Decimal,
    pub opened_at: DateTime<Utc>,
    pub status: PositionStatus,
    pub category: Category,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PositionKey {
    pub market_id: String,
    pub side: Side,
}

impl PositionKey {
    pub fn new(market_id: impl Into<String>, side: Side) -> Self {
        Self {
            market_id: market_id.into(),
            side,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub id: String,
    pub signal_id: String,
    pub market_id: String,
    pub category: Category,
    pub side: Side,
    pub price: Decimal,
    pub size: Decimal,
    pub size_usd: Decimal,
    pub filled_size: Decimal,
    pub order_type: OrderType,
    pub status: TradeStatus,
    pub placed_at: DateTime<Utc>,
    pub filled_at: Option<DateTime<Utc>>,
    pub simulated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TradeStatus {
    Pending,
    PartiallyFilled,
    Filled,
    Cancelled,
    TimedOut,
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_key_distinguishes_market_side_pairs() {
        let yes = PositionKey::new("market-1", Side::Yes);
        let no = PositionKey::new("market-1", Side::No);

        assert_ne!(yes, no);
        assert_eq!(yes.market_id, "market-1");
        assert_eq!(yes.side, Side::Yes);
        assert_eq!(no.side, Side::No);
    }

    #[test]
    fn category_profiles() {
        assert_eq!(Category::Politics.max_single_position_usd(), dec!(250));
        assert_eq!(Category::Crypto.max_single_position_usd(), dec!(150));
        assert_eq!(Category::Politics.min_confidence_threshold(), 6);
        assert_eq!(Category::Crypto.min_confidence_threshold(), 7);
        assert_eq!(Category::Politics.min_secret_level_threshold(), 5);
        assert_eq!(Category::Crypto.min_secret_level_threshold(), 6);
        assert_eq!(Category::Other.min_secret_level_threshold(), 7);
    }

    #[test]
    fn side_serialization() {
        let json = serde_json::to_string(&Side::Yes).unwrap();
        assert_eq!(json, "\"YES\"");
        let json = serde_json::to_string(&Side::No).unwrap();
        assert_eq!(json, "\"NO\"");
    }

    #[test]
    fn fok_requires_price_buffer() {
        assert!(OrderType::Fok.requires_price_buffer());
        assert!(!OrderType::Limit.requires_price_buffer());
    }

    fn create_test_signal() -> Signal {
        Signal {
            signal_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            timestamp: "2026-04-14T12:34:56.789Z".to_string(),
            wallet_address: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "0xdef456".to_string(),
            side: Side::Yes,
            confidence: 7,
            secret_level: 7,
            category: Category::Politics,
            source: SignalSource::Manual,
            tx_hash: None,
            token_id: None,
            target_price: None,
            target_size_usdc: None,
            resolved: false,
            redeemable: false,
            suggested_size_usdc: Some(dec!(50)),
            scanner_version: "1.0.0".to_string(),
        }
    }

    #[test]
    fn signal_valid() {
        // Use a recent timestamp
        let mut signal = create_test_signal();
        signal.timestamp = Utc::now().to_rfc3339();
        assert!(signal.validate().is_ok());
    }

    #[test]
    fn signal_validate_with_custom_max_age_accepts_recent_signal() {
        let mut signal = create_test_signal();
        signal.timestamp = (Utc::now() - chrono::Duration::seconds(45)).to_rfc3339();

        assert!(signal.validate_with_max_age_secs(60).is_ok());
    }

    #[test]
    fn signal_rejects_resolved_market() {
        let mut signal = create_test_signal();
        signal.timestamp = Utc::now().to_rfc3339();
        signal.resolved = true;

        let err = signal.validate().unwrap_err();
        assert!(err
            .iter()
            .any(|e| matches!(e, ValidationError::ResolvedMarket(id) if id == "0xdef456")));
    }

    #[test]
    fn signal_rejects_redeemable_market() {
        let mut signal = create_test_signal();
        signal.timestamp = Utc::now().to_rfc3339();
        signal.redeemable = true;

        let err = signal.validate().unwrap_err();
        assert!(err
            .iter()
            .any(|e| matches!(e, ValidationError::RedeemableMarket(id) if id == "0xdef456")));
    }

    #[test]
    fn signal_invalid_secret_level() {
        let mut signal = create_test_signal();
        signal.secret_level = 0;
        let err = signal.validate().unwrap_err();
        assert!(err
            .iter()
            .any(|e| matches!(e, ValidationError::SecretLevelOutOfRange(0))));
    }

    #[test]
    fn signal_invalid_confidence_zero() {
        let mut signal = create_test_signal();
        signal.confidence = 0;
        let err = signal.validate().unwrap_err();
        assert!(err
            .iter()
            .any(|e| matches!(e, ValidationError::ConfidenceOutOfRange(0))));
    }

    #[test]
    fn signal_invalid_non_uuid_signal_id() {
        let mut signal = create_test_signal();
        signal.signal_id = "not-a-uuid".to_string();
        let err = signal.validate().unwrap_err();
        assert!(err
            .iter()
            .any(|e| matches!(e, ValidationError::InvalidSignalId(id) if id == "not-a-uuid")));
    }

    #[test]
    fn signal_future_timestamp_is_rejected() {
        let mut signal = create_test_signal();
        signal.timestamp = (Utc::now() + chrono::Duration::seconds(120)).to_rfc3339();
        let err = signal.validate().unwrap_err();
        assert!(err
            .iter()
            .any(|e| matches!(e, ValidationError::FutureTimestamp(_))));
    }

    #[test]
    fn signal_invalid_wallet_address() {
        let mut signal = create_test_signal();
        signal.wallet_address = "abc".to_string();
        let err = signal.validate().unwrap_err();
        assert!(err
            .iter()
            .any(|e| matches!(e, ValidationError::InvalidWalletAddress(_))));
    }

    #[test]
    fn signal_invalid_non_hex_wallet_address() {
        let mut signal = create_test_signal();
        signal.wallet_address = "0xzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz".to_string();
        let err = signal.validate().unwrap_err();
        assert!(err
            .iter()
            .any(|e| matches!(e, ValidationError::InvalidWalletAddress(addr) if addr == "0xzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz")));
    }

    #[test]
    fn signal_requires_manual_review_low_confidence() {
        let mut signal = create_test_signal();
        signal.confidence = 2;
        assert!(signal.requires_manual_review());
        signal.confidence = 7;
        signal.secret_level = 2;
        assert!(signal.requires_manual_review());
    }

    #[test]
    fn signal_blocked_by_category_thresholds() {
        let mut signal = create_test_signal();
        signal.category = Category::Crypto;
        signal.confidence = 5; // below min 7
        assert!(signal.is_blocked_by_category_thresholds());
        signal.confidence = 8;
        signal.secret_level = 5; // below min 6 for crypto
        assert!(signal.is_blocked_by_category_thresholds());
    }

    #[test]
    fn trade_default_not_simulated() {
        let trade = Trade {
            id: "t1".to_string(),
            signal_id: "s1".to_string(),
            market_id: "m1".to_string(),
            category: Category::Politics,
            side: Side::Yes,
            price: dec!(0.65),
            size: dec!(100),
            size_usd: dec!(65),
            filled_size: dec!(0),
            order_type: OrderType::Limit,
            status: TradeStatus::Pending,
            placed_at: Utc::now(),
            filled_at: None,
            simulated: false,
        };
        assert!(!trade.simulated);
    }
}
