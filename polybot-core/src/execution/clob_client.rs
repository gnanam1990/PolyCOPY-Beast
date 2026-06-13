use polybot_common::errors::PolybotError;
use polybot_common::types::{Side, Trade, TradeDirection};
use polymarket_client_sdk::auth::state::{Authenticated, Unauthenticated};
use polymarket_client_sdk::auth::{
    Credentials as SdkCredentials, ExposeSecret, LocalSigner, Normal, Signer as _,
};
use polymarket_client_sdk::clob::types::request::{
    BalanceAllowanceRequest, OrderBookSummaryRequest, UpdateBalanceAllowanceRequest,
};
use polymarket_client_sdk::clob::types::{
    OrderType as SdkOrderType, Side as SdkSide, SignatureType,
};
use polymarket_client_sdk::clob::{Client as SdkClobClient, Config as SdkClobConfig};
use polymarket_client_sdk::types::{Address, U256};
use polymarket_client_sdk::{derive_proxy_wallet, derive_safe_wallet, POLYGON};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;

use super::order_builder::Order;
use super::rate_limiter::ClobRateLimiter;
use super::retry::{classify_sdk_error, RetryClass};
use super::v2_client::RelayerClient;
use super::v2_signing::{build_v2_order_payload, payload_to_relayer_json, sign_v2_order_payload};

/// Reads the Polymarket EOA private key from the environment.
///
/// Prefers `POLYMARKET_PRIVATE_KEY` (V2 canonical name, PRD §6). Falls back
/// to the legacy `POLYBOT_PRIVATE_KEY`, emitting a deprecation warning when
/// that alias is used. The alias will be removed in a future phase once V1
/// simulation is sunset.
pub(crate) fn read_private_key_from_env() -> Result<String, PolybotError> {
    match std::env::var("POLYMARKET_PRIVATE_KEY") {
        Ok(value) => Ok(value),
        Err(_) => match std::env::var("POLYBOT_PRIVATE_KEY") {
            Ok(value) => {
                tracing::warn!(
                    "POLYBOT_PRIVATE_KEY is deprecated — rename to POLYMARKET_PRIVATE_KEY before the V1 alias is removed."
                );
                Ok(value)
            }
            Err(_) => Err(PolybotError::Config(
                "POLYMARKET_PRIVATE_KEY not set (legacy POLYBOT_PRIVATE_KEY also absent)"
                    .to_string(),
            )),
        },
    }
}

#[derive(Debug, Error)]
#[error("{source}")]
pub struct SubmitOrderError {
    #[source]
    source: PolybotError,
    retry_class: RetryClass,
}

impl SubmitOrderError {
    fn non_retryable(source: PolybotError) -> Self {
        Self {
            source,
            retry_class: RetryClass::NonRetryable,
        }
    }

    fn from_sdk_submit_error(error: polymarket_client_sdk::error::Error) -> Self {
        let retry_class = classify_sdk_error(&error);
        Self {
            source: PolybotError::Execution(format!("Failed to submit CLOB order: {}", error)),
            retry_class,
        }
    }

    pub fn retry_class(&self) -> RetryClass {
        self.retry_class
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self.retry_class, RetryClass::Retryable)
    }

    pub fn into_polybot(self) -> PolybotError {
        self.source
    }
}

impl From<PolybotError> for SubmitOrderError {
    fn from(source: PolybotError) -> Self {
        Self::non_retryable(source)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletMode {
    Eoa,
    Proxy,
    GnosisSafe,
}

impl std::fmt::Display for WalletMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WalletMode::Eoa => write!(f, "EOA"),
            WalletMode::Proxy => write!(f, "Proxy"),
            WalletMode::GnosisSafe => write!(f, "GnosisSafe"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalStatus {
    pub usdc_balance: Decimal,
    pub approved_spenders: Vec<String>,
    pub ready_for_live_trading: bool,
}

impl ApprovalStatus {
    pub fn guidance_message(&self) -> String {
        format!(
            "No positive CLOB allowances detected for this wallet. Complete the first-run USDC/CTF approval flow, then rerun `cargo run -p polybot-core -- --setup-check`. CLOB-reported USDC balance: {}",
            self.usdc_balance
        )
    }
}

fn credentials_path_from_env() -> PathBuf {
    std::env::var("POLYBOT_CLOB_CREDENTIALS_PATH")
        .or_else(|_| std::env::var("POLYMARKET_CLOB_CREDENTIALS_PATH"))
        .unwrap_or_else(|_| ".\\.polybot\\clob_credentials.json".to_string())
        .into()
}

fn load_persisted_api_credentials(path: &Path) -> Result<Option<ApiCredentials>, PolybotError> {
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(path).map_err(|e| {
        PolybotError::Config(format!(
            "Failed to read persisted CLOB credentials from {}: {}",
            path.display(),
            e
        ))
    })?;

    let credentials = serde_json::from_str(&content).map_err(|e| {
        PolybotError::Config(format!(
            "Failed to parse persisted CLOB credentials from {}: {}",
            path.display(),
            e
        ))
    })?;

    Ok(Some(credentials))
}

fn persist_api_credentials(path: &Path, credentials: &ApiCredentials) -> Result<(), PolybotError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            PolybotError::Config(format!(
                "Failed to create credentials directory {}: {}",
                parent.display(),
                e
            ))
        })?;
    }

    let content = serde_json::to_string_pretty(credentials).map_err(|e| {
        PolybotError::Config(format!("Failed to serialize CLOB credentials: {}", e))
    })?;

    std::fs::write(path, content).map_err(|e| {
        PolybotError::Config(format!(
            "Failed to persist CLOB credentials to {}: {}",
            path.display(),
            e
        ))
    })
}

fn has_positive_allowance(value: &str) -> bool {
    U256::from_str(value)
        .map(|allowance| allowance > U256::ZERO)
        .unwrap_or(false)
}

/// Configuration for the Polymarket CLOB client.
#[derive(Debug, Clone)]
pub struct ClobConfig {
    /// CLOB API endpoint (default: https://clob.polymarket.com)
    pub endpoint: String,
    /// WebSocket endpoint (default: wss://ws-subscriptions-clob.polymarket.com)
    pub ws_endpoint: String,
    /// Chain ID (137 = Polygon mainnet)
    pub chain_id: u64,
    /// Private key for signing (env var: POLYMARKET_PRIVATE_KEY;
    /// legacy POLYBOT_PRIVATE_KEY accepted as deprecation alias).
    pub private_key: String,
    /// API key credentials (derived from L1 auth)
    pub api_key: Option<ApiCredentials>,
    /// Signature type: 0=EOA, 1=POLY_PROXY, 2=GNOSIS_SAFE
    pub signature_type: u8,
    /// Funder address (for proxy/safe wallets)
    pub funder_address: Option<String>,
}

/// L2 API credentials derived from L1 authentication.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiCredentials {
    pub api_key: String,
    pub secret: String,
    pub passphrase: String,
}

impl ApiCredentials {
    fn into_sdk_credentials(self) -> Result<SdkCredentials, PolybotError> {
        let key = Uuid::parse_str(&self.api_key)
            .map_err(|e| PolybotError::Config(format!("Invalid persisted CLOB api key: {}", e)))?;

        Ok(SdkCredentials::new(key, self.secret, self.passphrase))
    }
}

/// L2 API credentials alias.
pub type ApiKey = ApiCredentials;

/// CLOB order response from the Polymarket API.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ClobOrderResponse {
    pub success: bool,
    #[serde(default)]
    pub error_msg: String,
    #[serde(default)]
    pub order_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub taking_amount: String,
    #[serde(default)]
    pub making_amount: String,
}

/// Orderbook snapshot from the CLOB API.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct OrderBookSnapshot {
    pub market: String,
    pub asset_id: String,
    pub bids: Vec<OrderBookEntry>,
    pub asks: Vec<OrderBookEntry>,
    pub hash: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct OrderBookEntry {
    pub price: String,
    pub size: String,
}

/// Market info from the CLOB API.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct MarketInfo {
    pub condition_id: String,
    pub question_id: String,
    pub tokens: Vec<TokenInfo>,
    pub minimum_order_size: String,
    pub minimum_tick_size: String,
    #[serde(default)]
    pub neg_risk: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TokenInfo {
    pub token_id: String,
    pub outcome: String,
    pub price: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MarketContext {
    pub token_id: String,
    pub tick_size: Decimal,
    pub min_order_size: Decimal,
    pub neg_risk: bool,
}

impl MarketContext {
    pub fn simulation(token_id: impl Into<String>) -> Self {
        Self {
            token_id: token_id.into(),
            tick_size: dec!(0.01),
            min_order_size: dec!(1),
            neg_risk: false,
        }
    }
}

/// Polymarket CLOB client — wraps the official Rust SDK for order placement,
/// orderbook access, and authentication.
pub struct ClobClient {
    config: ClobConfig,
    rate_limiter: Arc<ClobRateLimiter>,
    http_client: reqwest::Client,
    public_client: SdkClobClient<Unauthenticated>,
    authenticated_client: RwLock<Option<SdkClobClient<Authenticated<Normal>>>>,
    /// Whether we have valid L2 credentials
    authenticated: RwLock<bool>,
}

fn sdk_side_for_direction(direction: TradeDirection) -> SdkSide {
    match direction {
        TradeDirection::Buy => SdkSide::Buy,
        TradeDirection::Sell => SdkSide::Sell,
    }
}

fn response_filled_size(
    order: &Order,
    response: &polymarket_client_sdk::clob::types::response::PostOrderResponse,
) -> Decimal {
    let filled_size = match order.direction {
        TradeDirection::Buy => response.taking_amount,
        TradeDirection::Sell => response.making_amount,
    };

    filled_size.max(Decimal::ZERO).min(order.size)
}

fn response_filled_notional_usd(
    order: &Order,
    response: &polymarket_client_sdk::clob::types::response::PostOrderResponse,
) -> Decimal {
    let filled_notional = match order.direction {
        TradeDirection::Buy => response.making_amount,
        TradeDirection::Sell => response.taking_amount,
    };

    filled_notional.max(Decimal::ZERO).min(order.size_usd)
}

fn map_trade_status_and_fill(
    order: &Order,
    response: &polymarket_client_sdk::clob::types::response::PostOrderResponse,
) -> (polybot_common::types::TradeStatus, Decimal, Decimal) {
    if !response.success {
        return (
            polybot_common::types::TradeStatus::Failed(
                response
                    .error_msg
                    .clone()
                    .unwrap_or_else(|| "Order rejected by CLOB".to_string()),
            ),
            Decimal::ZERO,
            order.size_usd,
        );
    }

    let response_fill = response_filled_size(order, response);
    let response_notional = response_filled_notional_usd(order, response);
    let (status, filled_size, size_usd) = match response.status {
        polymarket_client_sdk::clob::types::OrderStatusType::Matched => {
            let filled_size = if response_fill > Decimal::ZERO {
                response_fill
            } else {
                order.size
            };
            let size_usd = if response_notional > Decimal::ZERO {
                response_notional
            } else {
                order.size_usd
            };
            (
                polybot_common::types::TradeStatus::Filled,
                filled_size,
                size_usd,
            )
        }
        polymarket_client_sdk::clob::types::OrderStatusType::Live
        | polymarket_client_sdk::clob::types::OrderStatusType::Delayed
            if response_fill > Decimal::ZERO =>
        {
            (
                polybot_common::types::TradeStatus::PartiallyFilled,
                response_fill,
                response_notional,
            )
        }
        polymarket_client_sdk::clob::types::OrderStatusType::Live
        | polymarket_client_sdk::clob::types::OrderStatusType::Delayed => (
            polybot_common::types::TradeStatus::Pending,
            Decimal::ZERO,
            order.size_usd,
        ),
        polymarket_client_sdk::clob::types::OrderStatusType::Canceled => (
            polybot_common::types::TradeStatus::Cancelled,
            Decimal::ZERO,
            order.size_usd,
        ),
        polymarket_client_sdk::clob::types::OrderStatusType::Unmatched => (
            polybot_common::types::TradeStatus::TimedOut,
            Decimal::ZERO,
            order.size_usd,
        ),
        polymarket_client_sdk::clob::types::OrderStatusType::Unknown(ref raw) => (
            polybot_common::types::TradeStatus::Failed(format!(
                "Unknown CLOB order status: {}",
                raw
            )),
            Decimal::ZERO,
            order.size_usd,
        ),
        _ => (
            polybot_common::types::TradeStatus::Pending,
            Decimal::ZERO,
            order.size_usd,
        ),
    };

    (status, filled_size, size_usd)
}

fn map_submit_response_to_trade(
    order: &Order,
    response: &crate::execution::v2_client::RelayerSubmitResponse,
) -> Result<Trade, PolybotError> {
    Ok(Trade {
        id: uuid::Uuid::new_v4().to_string(),
        signal_id: order.signal_id.clone(),
        source_wallet: order.source_wallet.clone(),
        market_id: order.market_id.clone(),
        category: order.category,
        side: order.side,
        direction: order.direction,
        price: order.price,
        size: order.size,
        size_usd: order.size_usd,
        filled_size: Decimal::ZERO,
        order_type: order.order_type,
        status: polybot_common::types::TradeStatus::Pending,
        placed_at: chrono::Utc::now(),
        filled_at: None,
        simulated: false,
        transaction_id: Some(response.transaction_id.clone()),
        transaction_hash: None,
        relayer_state: Some(crate::execution::v2_client::map_transaction_state(
            &response.state,
        )?),
        taker_fee_bps: 0,
        fee_paid_usdc: Decimal::ZERO,
        rebate_usdc: Decimal::ZERO,
        retry_count: 0,
        error_msg: None,
    })
}

fn map_terminal_relayer_state_to_trade(
    mut trade: Trade,
    response: &crate::execution::v2_client::RelayerTransactionResponse,
) -> Result<Trade, PolybotError> {
    let state = crate::execution::v2_client::map_transaction_state(&response.state)?;
    trade.relayer_state = Some(state);
    trade.transaction_hash = response.transaction_hash.clone();
    match state {
        polybot_common::types::TransactionState::Success
        | polybot_common::types::TransactionState::Confirmed => {
            trade.status = polybot_common::types::TradeStatus::Filled;
            trade.filled_size = trade.size;
            trade.filled_at = Some(chrono::Utc::now());
        }
        polybot_common::types::TransactionState::Failed
        | polybot_common::types::TransactionState::Invalid => {
            trade.status = polybot_common::types::TradeStatus::Failed(
                response
                    .error_msg
                    .clone()
                    .unwrap_or_else(|| "Relayer transaction failed".to_string()),
            );
            trade.error_msg = response.error_msg.clone();
        }
        _ => {}
    }
    Ok(trade)
}

impl ClobClient {
    /// Create a new CLOB client with the given configuration.
    pub fn new(config: ClobConfig) -> Self {
        let rate_limiter = Arc::new(ClobRateLimiter::new_with_limits(80, 240)); // 80% of 100/300
        let public_client = SdkClobClient::new(&config.endpoint, SdkClobConfig::default())
            .expect("Failed to create SDK CLOB client");
        Self {
            config,
            rate_limiter,
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("Failed to create HTTP client"),
            public_client,
            authenticated_client: RwLock::new(None),
            authenticated: RwLock::new(false),
        }
    }

    /// Create a client from environment variables.
    pub fn from_env() -> Result<Self, PolybotError> {
        let private_key = read_private_key_from_env()?;

        let endpoint = std::env::var("POLYBOT_CLOB_ENDPOINT")
            .or_else(|_| std::env::var("CLOB_API_URL"))
            .unwrap_or_else(|_| "https://clob.polymarket.com".to_string());

        let ws_endpoint = std::env::var("POLYBOT_WS_ENDPOINT")
            .or_else(|_| std::env::var("WS_CLOB_URL"))
            .unwrap_or_else(|_| "wss://ws-subscriptions-clob.polymarket.com".to_string());

        let signature_type = std::env::var("POLYBOT_SIGNATURE_TYPE")
            .or_else(|_| std::env::var("POLYMARKET_SIGNATURE_TYPE"))
            .ok()
            .and_then(|v| v.parse::<u8>().ok())
            .unwrap_or(0); // EOA by default

        let funder_address = std::env::var("POLYBOT_FUNDER_ADDRESS")
            .or_else(|_| std::env::var("FUNDER_ADDRESS"))
            .ok();

        let chain_id = std::env::var("POLYBOT_CHAIN_ID")
            .or_else(|_| std::env::var("POLYGON_CHAIN_ID"))
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(137);

        let api_key = load_persisted_api_credentials(&credentials_path_from_env())?;

        let config = ClobConfig {
            endpoint,
            ws_endpoint,
            chain_id,
            private_key,
            api_key,
            signature_type,
            funder_address,
        };

        Ok(Self::new(config))
    }

    /// Create a public, read-only client for market data requests.
    pub fn public_readonly() -> Self {
        let endpoint = std::env::var("POLYBOT_CLOB_ENDPOINT")
            .unwrap_or_else(|_| "https://clob.polymarket.com".to_string());
        let ws_endpoint = std::env::var("POLYBOT_WS_ENDPOINT")
            .unwrap_or_else(|_| "wss://ws-subscriptions-clob.polymarket.com".to_string());

        Self::new(ClobConfig {
            endpoint,
            ws_endpoint,
            chain_id: 137,
            private_key: String::new(),
            api_key: None,
            signature_type: 0,
            funder_address: None,
        })
    }

    /// Authenticate with the CLOB — derive L2 API credentials from L1 private key.
    /// This performs EIP-712 signing to create or derive API credentials.
    pub async fn authenticate(&self) -> Result<ApiCredentials, PolybotError> {
        tracing::info!("Authenticating with Polymarket CLOB (L1 -> L2)");

        let signer = self.local_signer()?;
        let credentials_path = credentials_path_from_env();
        let client = if let Some(credentials) = self.config.api_key.clone() {
            match self
                .build_authenticated_client(&signer, Some(credentials.clone()))
                .await
            {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        credentials_path = %credentials_path.display(),
                        "Persisted CLOB credentials failed; deriving fresh credentials"
                    );
                    self.build_authenticated_client(&signer, None).await?
                }
            }
        } else {
            self.build_authenticated_client(&signer, None).await?
        };

        let credentials = client.credentials().clone();
        *self.authenticated_client.write().await = Some(client);
        *self.authenticated.write().await = true;
        let api_credentials = ApiCredentials {
            api_key: credentials.key().to_string(),
            secret: credentials.secret().expose_secret().to_string(),
            passphrase: credentials.passphrase().expose_secret().to_string(),
        };

        persist_api_credentials(&credentials_path, &api_credentials)?;

        Ok(api_credentials)
    }

    async fn build_authenticated_client(
        &self,
        signer: &impl polymarket_client_sdk::auth::Signer,
        credentials: Option<ApiCredentials>,
    ) -> Result<SdkClobClient<Authenticated<Normal>>, PolybotError> {
        let mut builder = self.public_client.clone().authentication_builder(signer);
        builder = builder.signature_type(self.sdk_signature_type());

        if let Some(funder) = self.config.funder_address.as_ref() {
            let funder = funder
                .parse()
                .map_err(|e| PolybotError::Execution(format!("Invalid funder address: {}", e)))?;
            builder = builder.funder(funder);
        }

        if let Some(credentials) = credentials {
            builder = builder.credentials(credentials.into_sdk_credentials()?);
        }

        builder
            .authenticate()
            .await
            .map_err(|e| PolybotError::Execution(format!("CLOB authentication failed: {}", e)))
    }

    fn sdk_signature_type(&self) -> SignatureType {
        match self.config.signature_type {
            1 => SignatureType::Proxy,
            2 => SignatureType::GnosisSafe,
            _ => SignatureType::Eoa,
        }
    }

    pub fn validate_wallet_mode(&self) -> Result<WalletMode, PolybotError> {
        if self.config.chain_id != POLYGON {
            return Err(PolybotError::Config(format!(
                "Unsupported chain id {}. Module 0 requires Polygon mainnet (137).",
                self.config.chain_id
            )));
        }

        self.local_signer()?;

        match self.config.signature_type {
            0 => {
                if self.config.funder_address.is_some() {
                    return Err(PolybotError::Config(
                        "POLYBOT_FUNDER_ADDRESS must not be set for EOA mode".to_string(),
                    ));
                }
                Ok(WalletMode::Eoa)
            }
            1 => {
                if let Some(funder) = self.config.funder_address.as_ref() {
                    let _: polymarket_client_sdk::types::Address = funder.parse().map_err(|e| {
                        PolybotError::Config(format!("Invalid proxy funder address: {}", e))
                    })?;
                }
                Ok(WalletMode::Proxy)
            }
            2 => {
                let funder = self.config.funder_address.as_ref().ok_or_else(|| {
                    PolybotError::Config(
                        "POLYBOT_FUNDER_ADDRESS is required for GnosisSafe mode".to_string(),
                    )
                })?;
                let _: polymarket_client_sdk::types::Address = funder.parse().map_err(|e| {
                    PolybotError::Config(format!("Invalid GnosisSafe funder address: {}", e))
                })?;
                Ok(WalletMode::GnosisSafe)
            }
            other => Err(PolybotError::Config(format!(
                "Unsupported POLYBOT_SIGNATURE_TYPE {}. Expected 0 (EOA), 1 (Proxy), or 2 (GnosisSafe).",
                other
            ))),
        }
    }

    fn local_signer(&self) -> Result<impl polymarket_client_sdk::auth::Signer, PolybotError> {
        if self.config.private_key.is_empty() {
            return Err(PolybotError::Config(
                "No private key configured for live trading".to_string(),
            ));
        }

        LocalSigner::from_str(&self.config.private_key)
            .map(|signer| signer.with_chain_id(Some(self.config.chain_id)))
            .map_err(|e| PolybotError::Config(format!("Invalid private key: {}", e)))
    }

    pub async fn check_approvals(&self) -> Result<ApprovalStatus, PolybotError> {
        if !*self.authenticated.read().await {
            self.authenticate().await?;
        }

        let client = self
            .authenticated_client
            .read()
            .await
            .as_ref()
            .cloned()
            .ok_or_else(|| {
                PolybotError::Execution("Authenticated CLOB client unavailable".to_string())
            })?;

        client
            .update_balance_allowance(UpdateBalanceAllowanceRequest::default())
            .await
            .map_err(|e| {
                PolybotError::Execution(format!("Failed to refresh CLOB approval state: {}", e))
            })?;

        let response = client
            .balance_allowance(BalanceAllowanceRequest::default())
            .await
            .map_err(|e| {
                PolybotError::Execution(format!(
                    "Failed to read CLOB balance/allowance state: {}",
                    e
                ))
            })?;

        let approved_spenders = response
            .allowances
            .iter()
            .filter(|(_, allowance)| has_positive_allowance(allowance))
            .map(|(address, _)| address.to_string())
            .collect::<Vec<_>>();

        Ok(ApprovalStatus {
            usdc_balance: response.balance,
            ready_for_live_trading: !approved_spenders.is_empty(),
            approved_spenders,
        })
    }

    pub fn trading_wallet_address(&self) -> Result<Address, PolybotError> {
        let signer = self.local_signer()?;
        let signer_address = signer.address();

        match self.config.signature_type {
            0 => Ok(signer_address),
            1 => {
                if let Some(funder) = self.config.funder_address.as_ref() {
                    return funder.parse().map_err(|e| {
                        PolybotError::Config(format!("Invalid proxy funder address: {}", e))
                    });
                }

                derive_proxy_wallet(signer_address, self.config.chain_id).ok_or_else(|| {
                    PolybotError::Config(
                        "Unable to derive proxy wallet address for reconciliation".to_string(),
                    )
                })
            }
            2 => {
                if let Some(funder) = self.config.funder_address.as_ref() {
                    return funder.parse().map_err(|e| {
                        PolybotError::Config(format!("Invalid GnosisSafe funder address: {}", e))
                    });
                }

                derive_safe_wallet(signer_address, self.config.chain_id).ok_or_else(|| {
                    PolybotError::Config(
                        "Unable to derive safe wallet address for reconciliation".to_string(),
                    )
                })
            }
            other => Err(PolybotError::Config(format!(
                "Unsupported POLYBOT_SIGNATURE_TYPE {}. Expected 0, 1, or 2.",
                other
            ))),
        }
    }

    pub async fn cancel_all_orders(&self) -> Result<(), PolybotError> {
        if !*self.authenticated.read().await {
            self.authenticate().await?;
        }

        let client = self
            .authenticated_client
            .read()
            .await
            .as_ref()
            .cloned()
            .ok_or_else(|| {
                PolybotError::Execution("Authenticated CLOB client unavailable".to_string())
            })?;

        client.cancel_all_orders().await.map_err(|e| {
            PolybotError::Execution(format!("Failed to cancel open CLOB orders: {}", e))
        })?;
        Ok(())
    }

    pub async fn submit_order_v2(
        &self,
        order: &Order,
        relayer: &crate::config::RelayerConfig,
        builder_code: &str,
    ) -> Result<Trade, SubmitOrderError> {
        if !self.rate_limiter.check_write().await {
            return Err(SubmitOrderError::non_retryable(PolybotError::Execution(
                "CLOB rate limit circuit breaker is open — too many requests".to_string(),
            )));
        }
        self.rate_limiter.record_write().await;

        let signer = self.local_signer().map_err(SubmitOrderError::from)?;
        let maker = self
            .trading_wallet_address()
            .map_err(SubmitOrderError::from)?;
        let payload = build_v2_order_payload(
            order,
            &maker.to_string(),
            &signer.address().to_string(),
            builder_code,
            chrono::Utc::now().timestamp_millis() as u64,
        )
        .map_err(SubmitOrderError::from)?;
        let signature = sign_v2_order_payload(&signer, &payload)
            .await
            .map_err(SubmitOrderError::from)?;
        let mut signed_payload = payload.clone();
        signed_payload.signature = signature;
        let signed_order = payload_to_relayer_json(&signed_payload);
        let relayer_client = RelayerClient::new(relayer).map_err(SubmitOrderError::from)?;
        let submit_response = relayer_client
            .submit_order(signed_order, order.order_type)
            .await
            .map_err(SubmitOrderError::from)?;
        let trade = map_submit_response_to_trade(order, &submit_response)
            .map_err(SubmitOrderError::from)?;
        let terminal = relayer_client
            .poll_transaction_until_terminal(
                &submit_response.transaction_id,
                10,
                std::time::Duration::from_millis(10),
            )
            .await
            .map_err(SubmitOrderError::from)?;
        map_terminal_relayer_state_to_trade(trade, &terminal).map_err(SubmitOrderError::from)
    }

    /// Submit a signed order to the CLOB.
    /// This is the main entry point for live trading.
    pub async fn submit_order(&self, order: &Order) -> Result<Trade, SubmitOrderError> {
        // Check rate limiter
        if !self.rate_limiter.check_write().await {
            return Err(SubmitOrderError::non_retryable(PolybotError::Execution(
                "CLOB rate limit circuit breaker is open — too many requests".to_string(),
            )));
        }
        self.rate_limiter.record_write().await;

        // In production with the SDK:
        // 1. Create limit order via client.limit_order()
        // 2. Sign with the signer
        // 3. Post order via client.post_order()
        // 4. Track order status

        tracing::info!(
            signal_id = %order.signal_id,
            order_type = ?order.order_type,
            size_usd = %order.size_usd,
            "Submitting order to CLOB"
        );

        if !*self.authenticated.read().await {
            self.authenticate().await.map_err(SubmitOrderError::from)?;
        }

        let client = self
            .authenticated_client
            .read()
            .await
            .as_ref()
            .cloned()
            .ok_or_else(|| {
                PolybotError::Execution("Authenticated CLOB client unavailable".to_string())
            })?;
        let signer = self.local_signer()?;
        let token_id = U256::from_str(&order.token_id)
            .map_err(|e| PolybotError::Execution(format!("Invalid token id for order: {}", e)))?;

        let sdk_order_type = match order.order_type {
            polybot_common::types::OrderType::Fok => SdkOrderType::FOK,
            polybot_common::types::OrderType::Ioc => SdkOrderType::FAK,
            _ => SdkOrderType::GTC,
        };

        let signable_order = client
            .limit_order()
            .token_id(token_id)
            .side(sdk_side_for_direction(order.direction))
            .price(order.price)
            .size(order.size)
            .order_type(sdk_order_type)
            .post_only(matches!(
                order.order_type,
                polybot_common::types::OrderType::PostOnly
            ))
            .build()
            .await
            .map_err(|e| PolybotError::Execution(format!("Failed to build CLOB order: {}", e)))?;

        let signed_order = client
            .sign(&signer, signable_order)
            .await
            .map_err(|e| PolybotError::Execution(format!("Failed to sign CLOB order: {}", e)))?;
        let response = client
            .post_order(signed_order)
            .await
            .map_err(SubmitOrderError::from_sdk_submit_error)?;

        let (status, filled_size, size_usd) = map_trade_status_and_fill(order, &response);

        Ok(Trade {
            id: response.order_id,
            signal_id: order.signal_id.clone(),
            source_wallet: order.source_wallet.clone(),
            market_id: order.market_id.clone(),
            category: order.category,
            side: order.side,
            direction: order.direction,
            price: order.price,
            size: order.size,
            size_usd,
            filled_size,
            order_type: order.order_type,
            status,
            placed_at: chrono::Utc::now(),
            filled_at: None,
            simulated: false,
            transaction_id: None,
            transaction_hash: None,
            relayer_state: None,
            taker_fee_bps: 0,
            fee_paid_usdc: Decimal::ZERO,
            rebate_usdc: Decimal::ZERO,
            retry_count: 0,
            error_msg: None,
        })
    }

    /// Fetch the orderbook for a given market (token_id).
    pub async fn get_orderbook(&self, token_id: &str) -> Result<OrderBookSnapshot, PolybotError> {
        if !self.rate_limiter.check_read().await {
            return Err(PolybotError::Execution(
                "CLOB read rate limit circuit breaker open".to_string(),
            ));
        }
        self.rate_limiter.record_read().await;

        let token_id = U256::from_str(token_id)
            .map_err(|e| PolybotError::Execution(format!("Invalid token id: {}", e)))?;
        let request = OrderBookSummaryRequest::builder()
            .token_id(token_id)
            .build();
        let book = self
            .public_client
            .order_book(&request)
            .await
            .map_err(|e| PolybotError::Execution(format!("Orderbook fetch failed: {}", e)))?;

        Ok(OrderBookSnapshot {
            market: book.market.to_string(),
            asset_id: book.asset_id.to_string(),
            bids: book
                .bids
                .into_iter()
                .map(|entry| OrderBookEntry {
                    price: entry.price.to_string(),
                    size: entry.size.to_string(),
                })
                .collect(),
            asks: book
                .asks
                .into_iter()
                .map(|entry| OrderBookEntry {
                    price: entry.price.to_string(),
                    size: entry.size.to_string(),
                })
                .collect(),
            hash: book.hash.unwrap_or_default(),
            timestamp: book.timestamp.timestamp_millis() as u64,
        })
    }

    /// Get market info for a given condition_id.
    pub async fn get_market(&self, condition_id: &str) -> Result<MarketInfo, PolybotError> {
        let market = self
            .public_client
            .market(condition_id)
            .await
            .map_err(|e| PolybotError::Execution(format!("Market fetch failed: {}", e)))?;

        Ok(MarketInfo {
            condition_id: market
                .condition_id
                .map(|value| value.to_string())
                .unwrap_or_default(),
            question_id: market
                .question_id
                .map(|value| value.to_string())
                .unwrap_or_default(),
            tokens: market
                .tokens
                .into_iter()
                .map(|token| TokenInfo {
                    token_id: token.token_id.to_string(),
                    outcome: token.outcome,
                    price: token.price.to_string(),
                })
                .collect(),
            minimum_order_size: market.minimum_order_size.to_string(),
            minimum_tick_size: market.minimum_tick_size.to_string(),
            neg_risk: market.neg_risk,
        })
    }

    pub async fn resolve_token_id_for_signal(
        &self,
        market_id: &str,
        side: Side,
    ) -> Result<String, PolybotError> {
        if !Self::looks_like_condition_id(market_id) && U256::from_str(market_id).is_ok() {
            return Ok(market_id.to_string());
        }

        let market = self.get_market(market_id).await?;
        let outcome = match side {
            Side::Yes => "yes",
            Side::No => "no",
        };

        market
            .tokens
            .into_iter()
            .find(|token| token.outcome.eq_ignore_ascii_case(outcome))
            .map(|token| token.token_id)
            .ok_or_else(|| {
                PolybotError::Execution(format!(
                    "Unable to resolve token for outcome {} in market {}",
                    outcome, market_id
                ))
            })
    }

    pub fn market_context_from_market_info(
        market: &MarketInfo,
        side: Side,
    ) -> Result<MarketContext, PolybotError> {
        let outcome = match side {
            Side::Yes => "yes",
            Side::No => "no",
        };

        let token_id = market
            .tokens
            .iter()
            .find(|token| token.outcome.eq_ignore_ascii_case(outcome))
            .map(|token| token.token_id.clone())
            .ok_or_else(|| {
                PolybotError::Execution(format!(
                    "Unable to resolve token for outcome {} in market {}",
                    outcome, market.condition_id
                ))
            })?;

        let tick_size = Decimal::from_str(&market.minimum_tick_size).map_err(|e| {
            PolybotError::Execution(format!(
                "Invalid market tick size {}: {}",
                market.minimum_tick_size, e
            ))
        })?;
        let min_order_size = Decimal::from_str(&market.minimum_order_size).map_err(|e| {
            PolybotError::Execution(format!(
                "Invalid market minimum order size {}: {}",
                market.minimum_order_size, e
            ))
        })?;

        Ok(MarketContext {
            token_id,
            tick_size,
            min_order_size,
            neg_risk: market.neg_risk,
        })
    }

    pub async fn get_market_context_for_signal(
        &self,
        market_id: &str,
        side: Side,
    ) -> Result<MarketContext, PolybotError> {
        if !Self::looks_like_condition_id(market_id) && U256::from_str(market_id).is_ok() {
            return Ok(MarketContext::simulation(market_id.to_string()));
        }

        let market = self.get_market(market_id).await?;
        Self::market_context_from_market_info(&market, side)
    }

    pub async fn get_orderbook_for_signal(
        &self,
        market_id: &str,
        side: Side,
    ) -> Result<(String, OrderBookSnapshot), PolybotError> {
        let context = self.get_market_context_for_signal(market_id, side).await?;
        let book = self.get_orderbook(&context.token_id).await?;
        Ok((context.token_id, book))
    }

    fn looks_like_condition_id(market_id: &str) -> bool {
        market_id.starts_with("0x") && market_id.len() == 66
    }

    /// Best bid = highest-priced buy order. Polymarket's `/book` endpoint
    /// (and the WS book channel) returns bids sorted ascending by price, so the
    /// best bid is the LAST element, not the first. To stay correct regardless
    /// of source ordering we scan for the maximum bid price.
    pub fn best_bid(book: &OrderBookSnapshot) -> Option<Decimal> {
        book.bids
            .iter()
            .filter_map(|b| b.price.parse::<Decimal>().ok())
            .max()
    }

    /// Best ask = lowest-priced sell order. Polymarket returns asks sorted
    /// descending by price, so the best ask is the LAST element, not the first.
    /// Scan for the minimum ask price to stay correct regardless of ordering.
    pub fn best_ask(book: &OrderBookSnapshot) -> Option<Decimal> {
        book.asks
            .iter()
            .filter_map(|a| a.price.parse::<Decimal>().ok())
            .min()
    }

    /// Calculate the midpoint price from an orderbook for slippage validation.
    pub fn calculate_midpoint(book: &OrderBookSnapshot) -> Option<Decimal> {
        let best_bid = Self::best_bid(book);
        let best_ask = Self::best_ask(book);

        match (best_bid, best_ask) {
            (Some(bid), Some(ask)) => Some((bid + ask) / dec!(2)),
            (Some(bid), None) => Some(bid),
            (None, Some(ask)) => Some(ask),
            (None, None) => None,
        }
    }

    pub fn estimate_fill_price(book: &OrderBookSnapshot) -> Option<Decimal> {
        Self::best_ask(book).or_else(|| Self::calculate_midpoint(book))
    }

    pub fn visible_liquidity_usd(book: &OrderBookSnapshot) -> Decimal {
        book.bids
            .iter()
            .chain(book.asks.iter())
            .filter_map(|entry| {
                let price = entry.price.parse::<Decimal>().ok()?;
                let size = entry.size.parse::<Decimal>().ok()?;
                Some(price * size)
            })
            .fold(Decimal::ZERO, |acc, value| acc + value)
    }

    /// Check if slippage is within acceptable bounds (PRD: max 2% deviation).
    pub fn check_slippage(
        midpoint: Decimal,
        target_price: Decimal,
        max_deviation_pct: Decimal,
    ) -> bool {
        if midpoint == Decimal::ZERO {
            return false;
        }
        let deviation = ((target_price - midpoint).abs() / midpoint).abs();
        deviation <= max_deviation_pct
    }

    /// Send a heartbeat to keep the trading session alive.
    /// Per PRD: must send every 5 seconds, else all orders cancelled after 10s.
    pub async fn send_heartbeat(&self) -> Result<String, PolybotError> {
        let url = format!("{}/heartbeat", self.config.endpoint);
        // In production, this would include L2 auth headers
        let response = self
            .http_client
            .post(&url)
            .send()
            .await
            .map_err(|e| PolybotError::Execution(format!("Heartbeat failed: {}", e)))?;

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| PolybotError::Execution(format!("Heartbeat parse failed: {}", e)))?;

        body.get("heartbeat_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| PolybotError::Execution("No heartbeat_id in response".to_string()))
    }

    /// Get the current rate limiter stats
    pub async fn rate_limit_stats(&self) -> (u32, u32) {
        self.rate_limiter.get_stats().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::order_builder::Order;
    use crate::execution::v2_client::RelayerSubmitResponse;
    use polybot_common::types::{Category, OrderType, TradeDirection, TradeStatus};
    use std::path::PathBuf;

    fn test_config() -> ClobConfig {
        ClobConfig {
            endpoint: "https://clob.polymarket.com".to_string(),
            ws_endpoint: "wss://ws-subscriptions-clob.polymarket.com".to_string(),
            chain_id: 137,
            private_key: "0x0000000000000000000000000000000000000000000000000000000000000001"
                .to_string(),
            api_key: None,
            signature_type: 0,
            funder_address: None,
        }
    }

    fn temp_credentials_path() -> PathBuf {
        std::env::temp_dir().join(format!("polybot-clob-credentials-{}.json", Uuid::new_v4()))
    }

    fn test_order(direction: TradeDirection) -> Order {
        Order {
            signal_id: "signal-1".to_string(),
            source_wallet: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "market-1".to_string(),
            token_id: "123".to_string(),
            category: Category::Politics,
            side: Side::Yes,
            direction,
            price: dec!(0.50),
            size: dec!(10),
            size_usd: dec!(5),
            order_type: OrderType::Fok,
        }
    }

    #[test]
    fn clob_client_creation() {
        let client = ClobClient::new(test_config());
        assert!(!client.config.endpoint.is_empty());
    }

    #[test]
    fn midpoint_calculation() {
        let book = OrderBookSnapshot {
            market: "test".to_string(),
            asset_id: "test-token".to_string(),
            bids: vec![OrderBookEntry {
                price: "0.60".to_string(),
                size: "100".to_string(),
            }],
            asks: vec![OrderBookEntry {
                price: "0.62".to_string(),
                size: "100".to_string(),
            }],
            hash: "abc".to_string(),
            timestamp: 0,
        };
        let mid = ClobClient::calculate_midpoint(&book);
        assert_eq!(mid, Some(dec!(0.61)));
    }

    #[test]
    fn midpoint_empty_book() {
        let book = OrderBookSnapshot {
            market: "test".to_string(),
            asset_id: "test-token".to_string(),
            bids: vec![],
            asks: vec![],
            hash: "abc".to_string(),
            timestamp: 0,
        };
        assert_eq!(ClobClient::calculate_midpoint(&book), None);
    }

    #[test]
    fn slippage_check_within_bounds() {
        let mid = dec!(0.60);
        let target = dec!(0.61);
        assert!(ClobClient::check_slippage(mid, target, dec!(0.02))); // 1.67% < 2%
    }

    #[test]
    fn slippage_check_exceeds_bounds() {
        let mid = dec!(0.60);
        let target = dec!(0.65);
        assert!(!ClobClient::check_slippage(mid, target, dec!(0.02))); // 8.3% > 2%
    }

    #[test]
    fn estimate_fill_price_uses_best_ask() {
        let book = OrderBookSnapshot {
            market: "test".to_string(),
            asset_id: "test-token".to_string(),
            bids: vec![OrderBookEntry {
                price: "0.58".to_string(),
                size: "50".to_string(),
            }],
            asks: vec![OrderBookEntry {
                price: "0.62".to_string(),
                size: "60".to_string(),
            }],
            hash: "abc".to_string(),
            timestamp: 0,
        };

        assert_eq!(ClobClient::estimate_fill_price(&book), Some(dec!(0.62)));
    }

    #[test]
    fn best_bid_ask_handle_polymarket_book_ordering() {
        // Polymarket's /book returns bids ascending and asks descending by
        // price, so the best bid and best ask are both the LAST element.
        let book = OrderBookSnapshot {
            market: "test".to_string(),
            asset_id: "test-token".to_string(),
            bids: vec![
                OrderBookEntry {
                    price: "0.55".to_string(),
                    size: "100".to_string(),
                },
                OrderBookEntry {
                    price: "0.58".to_string(),
                    size: "100".to_string(),
                },
                OrderBookEntry {
                    price: "0.60".to_string(), // best bid (highest), last
                    size: "100".to_string(),
                },
            ],
            asks: vec![
                OrderBookEntry {
                    price: "0.68".to_string(),
                    size: "100".to_string(),
                },
                OrderBookEntry {
                    price: "0.65".to_string(),
                    size: "100".to_string(),
                },
                OrderBookEntry {
                    price: "0.62".to_string(), // best ask (lowest), last
                    size: "100".to_string(),
                },
            ],
            hash: "abc".to_string(),
            timestamp: 0,
        };

        assert_eq!(ClobClient::best_bid(&book), Some(dec!(0.60)));
        assert_eq!(ClobClient::best_ask(&book), Some(dec!(0.62)));
        // Midpoint must use best bid/ask, not worst-priced first entries.
        assert_eq!(ClobClient::calculate_midpoint(&book), Some(dec!(0.61)));
        // Estimated fill must be the best (lowest) ask a buyer would cross.
        assert_eq!(ClobClient::estimate_fill_price(&book), Some(dec!(0.62)));
    }

    #[test]
    fn visible_liquidity_sums_book_depth() {
        let book = OrderBookSnapshot {
            market: "test".to_string(),
            asset_id: "test-token".to_string(),
            bids: vec![OrderBookEntry {
                price: "0.58".to_string(),
                size: "50".to_string(),
            }],
            asks: vec![OrderBookEntry {
                price: "0.62".to_string(),
                size: "60".to_string(),
            }],
            hash: "abc".to_string(),
            timestamp: 0,
        };

        assert_eq!(ClobClient::visible_liquidity_usd(&book), dec!(66.2));
    }

    #[test]
    #[serial_test::serial]
    fn from_env_missing_key() {
        // Clear both env var names so the fallback in read_private_key_from_env
        // also misses.
        std::env::remove_var("POLYMARKET_PRIVATE_KEY");
        std::env::remove_var("POLYBOT_PRIVATE_KEY");
        let result = ClobClient::from_env();
        assert!(result.is_err());
    }

    // env-mutating tests: serialize to avoid race with each other and with
    // any other test in this binary that reads POLYMARKET_PRIVATE_KEY /
    // POLYBOT_PRIVATE_KEY.
    #[test]
    #[serial_test::serial]
    fn private_key_reads_polymarket_env_var() {
        std::env::remove_var("POLYBOT_PRIVATE_KEY");
        std::env::set_var(
            "POLYMARKET_PRIVATE_KEY",
            "0x1111111111111111111111111111111111111111111111111111111111111111",
        );
        let got = read_private_key_from_env().expect("should resolve V2 name");
        assert!(got.starts_with("0x1"));
        std::env::remove_var("POLYMARKET_PRIVATE_KEY");
    }

    #[test]
    #[serial_test::serial]
    fn private_key_falls_back_to_polybot_env_var() {
        std::env::remove_var("POLYMARKET_PRIVATE_KEY");
        std::env::set_var(
            "POLYBOT_PRIVATE_KEY",
            "0x2222222222222222222222222222222222222222222222222222222222222222",
        );
        let got = read_private_key_from_env().expect("should resolve legacy name");
        assert!(got.starts_with("0x2"));
        std::env::remove_var("POLYBOT_PRIVATE_KEY");
    }

    #[test]
    #[serial_test::serial]
    fn private_key_missing_both_returns_error() {
        std::env::remove_var("POLYMARKET_PRIVATE_KEY");
        std::env::remove_var("POLYBOT_PRIVATE_KEY");
        let result = read_private_key_from_env();
        assert!(result.is_err(), "both unset should fail");
    }

    #[tokio::test]
    async fn rate_limiter_integration() {
        let client = ClobClient::new(test_config());
        let (writes, reads) = client.rate_limit_stats().await;
        assert_eq!(writes, 0);
        assert_eq!(reads, 0);
    }

    #[test]
    fn market_context_from_market_info_uses_market_metadata() {
        let market = MarketInfo {
            condition_id: "condition-1".to_string(),
            question_id: "question-1".to_string(),
            tokens: vec![
                TokenInfo {
                    token_id: "token-yes".to_string(),
                    outcome: "yes".to_string(),
                    price: "0.52".to_string(),
                },
                TokenInfo {
                    token_id: "token-no".to_string(),
                    outcome: "no".to_string(),
                    price: "0.48".to_string(),
                },
            ],
            minimum_order_size: "5".to_string(),
            minimum_tick_size: "0.001".to_string(),
            neg_risk: true,
        };

        let ctx = ClobClient::market_context_from_market_info(&market, Side::No).unwrap();
        assert_eq!(ctx.token_id, "token-no");
        assert_eq!(ctx.tick_size, dec!(0.001));
        assert_eq!(ctx.min_order_size, dec!(5));
        assert!(ctx.neg_risk);
    }

    #[test]
    fn wallet_mode_validation_rejects_funder_in_eoa_mode() {
        let mut config = test_config();
        config.funder_address = Some("0xabc123abc123abc123abc123abc123abc123abc1".to_string());
        let client = ClobClient::new(config);

        assert!(client.validate_wallet_mode().is_err());
    }

    #[test]
    fn wallet_mode_validation_requires_funder_for_safe_mode() {
        let mut config = test_config();
        config.signature_type = 2;
        let client = ClobClient::new(config);

        assert!(client.validate_wallet_mode().is_err());
    }

    #[test]
    fn persisted_credentials_round_trip() {
        let path = temp_credentials_path();
        let credentials = ApiCredentials {
            api_key: Uuid::new_v4().to_string(),
            secret: "secret".to_string(),
            passphrase: "passphrase".to_string(),
        };

        persist_api_credentials(&path, &credentials).unwrap();
        let loaded = load_persisted_api_credentials(&path).unwrap();

        assert_eq!(loaded, Some(credentials));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn approval_guidance_mentions_setup_check() {
        let status = ApprovalStatus {
            usdc_balance: Decimal::ZERO,
            approved_spenders: vec![],
            ready_for_live_trading: false,
        };

        assert!(status.guidance_message().contains("--setup-check"));
    }

    #[test]
    fn submit_order_error_marks_429_as_retryable() {
        let error =
            SubmitOrderError::from_sdk_submit_error(polymarket_client_sdk::error::Error::status(
                polymarket_client_sdk::error::StatusCode::TOO_MANY_REQUESTS,
                polymarket_client_sdk::error::Method::POST,
                "/order".to_string(),
                "rate limited",
            ));

        assert!(error.is_retryable());
    }

    #[test]
    fn submit_order_error_marks_400_as_non_retryable() {
        let error =
            SubmitOrderError::from_sdk_submit_error(polymarket_client_sdk::error::Error::status(
                polymarket_client_sdk::error::StatusCode::BAD_REQUEST,
                polymarket_client_sdk::error::Method::POST,
                "/order".to_string(),
                "bad order",
            ));

        assert!(!error.is_retryable());
    }

    #[test]
    fn trade_direction_maps_to_sdk_order_side() {
        assert_eq!(sdk_side_for_direction(TradeDirection::Buy), SdkSide::Buy);
        assert_eq!(sdk_side_for_direction(TradeDirection::Sell), SdkSide::Sell);
    }

    #[test]
    fn live_response_with_non_zero_fill_surfaces_partial_fill() {
        let order = test_order(TradeDirection::Sell);
        let response: polymarket_client_sdk::clob::types::response::PostOrderResponse =
            serde_json::from_str(
                r#"{
                    "errorMsg": null,
                    "makingAmount": "3",
                    "takingAmount": "1.5",
                    "orderID": "order-1",
                    "status": "LIVE",
                    "success": true,
                    "transactionHashes": [],
                    "tradeIDs": []
                }"#,
            )
            .unwrap();

        let (status, filled_size, size_usd) = map_trade_status_and_fill(&order, &response);

        assert_eq!(status, polybot_common::types::TradeStatus::PartiallyFilled);
        assert_eq!(filled_size, dec!(3));
        assert_eq!(size_usd, dec!(1.5));
    }

    #[test]
    fn relayer_submit_creates_pending_trade_with_transaction_id() {
        let order = test_order(TradeDirection::Buy);
        let response = crate::execution::v2_client::RelayerSubmitResponse {
            transaction_id: "txn_abc123".to_string(),
            state: "STATE_NEW".to_string(),
        };

        let trade = map_submit_response_to_trade(&order, &response).unwrap();

        assert_eq!(trade.transaction_id.as_deref(), Some("txn_abc123"));
        assert_eq!(
            trade.relayer_state,
            Some(polybot_common::types::TransactionState::New)
        );
        assert_eq!(trade.status, TradeStatus::Pending);
        assert_eq!(trade.filled_size, Decimal::ZERO);
    }

    #[test]
    fn terminal_relayer_failure_marks_trade_failed() {
        let order = test_order(TradeDirection::Buy);
        let submit = RelayerSubmitResponse {
            transaction_id: "txn_abc123".to_string(),
            state: "STATE_NEW".to_string(),
        };
        let trade = map_submit_response_to_trade(&order, &submit).unwrap();
        let terminal = crate::execution::v2_client::RelayerTransactionResponse {
            transaction_id: "txn_abc123".to_string(),
            state: "STATE_FAILED".to_string(),
            transaction_hash: None,
            error_msg: Some("relayer reverted".to_string()),
        };

        let updated = map_terminal_relayer_state_to_trade(trade, &terminal).unwrap();

        assert_eq!(
            updated.relayer_state,
            Some(polybot_common::types::TransactionState::Failed)
        );
        assert!(matches!(updated.status, TradeStatus::Failed(_)));
    }

    #[test]
    fn terminal_relayer_confirmed_marks_trade_filled() {
        let order = test_order(TradeDirection::Buy);
        let submit = RelayerSubmitResponse {
            transaction_id: "txn_abc123".to_string(),
            state: "STATE_NEW".to_string(),
        };
        let trade = map_submit_response_to_trade(&order, &submit).unwrap();
        let terminal = crate::execution::v2_client::RelayerTransactionResponse {
            transaction_id: "txn_abc123".to_string(),
            state: "STATE_CONFIRMED".to_string(),
            transaction_hash: Some("0xdeadbeef".to_string()),
            error_msg: None,
        };

        let updated = map_terminal_relayer_state_to_trade(trade, &terminal).unwrap();

        assert_eq!(
            updated.relayer_state,
            Some(polybot_common::types::TransactionState::Confirmed)
        );
        assert_eq!(updated.status, TradeStatus::Filled);
        assert_eq!(updated.filled_size, order.size);
        assert_eq!(updated.transaction_hash.as_deref(), Some("0xdeadbeef"));
    }
}
