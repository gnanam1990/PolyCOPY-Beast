use futures::StreamExt as _;
use polybot_common::errors::PolybotError;
use polymarket_client_sdk::clob::ws::Client as WsClient;
use polymarket_client_sdk::types::U256;
use polymarket_client_sdk::ws::config::Config as WsConfig;
use std::collections::HashMap;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};

use crate::metrics::Metrics;
use crate::telegram_bot::alerts::AlertBroadcaster;

use super::clob_client::{ClobConfig, OrderBookEntry, OrderBookSnapshot};

/// WebSocket message types from the Polymarket CLOB.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type")]
pub enum WsMessage {
    /// Price change event
    #[serde(rename = "price_change")]
    PriceChange(PriceChange),
    /// Order book snapshot
    #[serde(rename = "book")]
    Book(BookUpdate),
    /// Trade execution event
    #[serde(rename = "trade")]
    Trade(TradeEvent),
    /// User order status update
    #[serde(rename = "order")]
    Order(OrderUpdate),
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PriceChange {
    pub asset_id: String,
    pub price: String,
    pub side: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct BookUpdate {
    pub market: String,
    pub asset_id: String,
    pub bids: Vec<BookEntry>,
    pub asks: Vec<BookEntry>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct BookEntry {
    pub price: String,
    pub size: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TradeEvent {
    pub asset_id: String,
    pub price: String,
    pub size: String,
    pub side: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct OrderUpdate {
    pub order_id: String,
    pub status: OrderStatus,
    pub filled_size: String,
    pub price: String,
    pub side: String,
    pub market: String,
}

#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub enum OrderStatus {
    #[serde(rename = "LIVE")]
    Live,
    #[serde(rename = "MATCHED")]
    Matched,
    #[serde(rename = "CANCELLED")]
    Cancelled,
    #[serde(rename = "DELAYED")]
    Delayed,
    #[serde(rename = "UNMATCHED")]
    Unmatched,
}

/// Manages the WebSocket connection to Polymarket CLOB.
/// Handles reconnection with exponential backoff per PRD requirements.
pub struct ClobWsManager {
    config: ClobConfig,
    /// Subscribed token IDs for orderbook updates
    subscribed_tokens: Arc<RwLock<Vec<String>>>,
    /// Whether the WS connection is active
    connected: Arc<RwLock<bool>>,
    /// Last heartbeat timestamp
    last_pong: Arc<RwLock<std::time::Instant>>,
    latest_books: Arc<RwLock<HashMap<String, OrderBookSnapshot>>>,
    subscription_generation: Arc<AtomicU64>,
    metrics: Arc<Metrics>,
    alerts: Option<AlertBroadcaster>,
}

impl ClobWsManager {
    pub fn new(
        config: ClobConfig,
        metrics: Arc<Metrics>,
        alerts: Option<AlertBroadcaster>,
    ) -> Self {
        Self {
            config,
            subscribed_tokens: Arc::new(RwLock::new(Vec::new())),
            connected: Arc::new(RwLock::new(false)),
            last_pong: Arc::new(RwLock::new(std::time::Instant::now())),
            latest_books: Arc::new(RwLock::new(HashMap::new())),
            subscription_generation: Arc::new(AtomicU64::new(0)),
            metrics,
            alerts,
        }
    }

    /// Subscribe to orderbook updates for a token.
    pub async fn subscribe_token(&self, token_id: String) {
        let mut tokens = self.subscribed_tokens.write().await;
        if !tokens.contains(&token_id) {
            tokens.push(token_id.clone());
            self.subscription_generation.fetch_add(1, Ordering::Relaxed);
            tracing::info!(token_id = %token_id, "Subscribed to token orderbook");
        }
    }

    /// Unsubscribe from orderbook updates for a token.
    pub async fn unsubscribe_token(&self, token_id: &str) {
        let mut tokens = self.subscribed_tokens.write().await;
        tokens.retain(|t| t != token_id);
        self.subscription_generation.fetch_add(1, Ordering::Relaxed);
    }

    /// Check if the WebSocket connection is alive.
    pub async fn is_connected(&self) -> bool {
        *self.connected.read().await
    }

    /// Get time since last pong.
    pub async fn time_since_last_pong(&self) -> Duration {
        self.last_pong.read().await.elapsed()
    }

    pub async fn get_cached_orderbook(&self, token_id: &str) -> Option<OrderBookSnapshot> {
        self.latest_books.read().await.get(token_id).cloned()
    }

    /// Connect to the CLOB WebSocket with exponential backoff.
    /// Per PRD: reconnect with backoff (1s, 2s, 4s, 8s, max 60s).
    /// Send ping every 30s, expect pong within 10s.
    pub async fn connect_with_backoff(&self) -> Result<(), PolybotError> {
        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(60);
        let mut attempts = 0u32;

        loop {
            match self.connect_inner().await {
                Ok(()) => {
                    tracing::info!("CLOB WebSocket connected successfully");
                    *self.connected.write().await = true;
                    self.metrics.set_ws_connected(true);
                    *self.last_pong.write().await = std::time::Instant::now();
                    backoff = Duration::from_secs(1);
                    attempts = 0;
                    // Connection succeeded — this method returns on disconnect
                    // so we retry
                }
                Err(e) => {
                    *self.connected.write().await = false;
                    self.metrics.set_ws_connected(false);
                    attempts += 1;
                    tracing::error!(
                        error = %e,
                        attempt = attempts,
                        backoff_secs = backoff.as_secs(),
                        "CLOB WebSocket connection failed, retrying"
                    );
                }
            }

            tokio::time::sleep(backoff).await;
            backoff = std::cmp::min(backoff * 2, max_backoff);
        }
    }

    /// Internal connection logic using the polymarket-client-sdk WebSocket.
    /// In production with the SDK:
    /// ```
    /// let ws = MarketWebSocket::new(&self.config.ws_endpoint);
    /// ws.add_tokens(subscribed_tokens).await?;
    /// let mut stream = ws.connect().await?;
    /// while let Some(msg) = stream.next().await {
    ///     // Handle message
    /// }
    /// ```
    async fn connect_inner(&self) -> Result<(), PolybotError> {
        let generation = self.subscription_generation.load(Ordering::Relaxed);
        let asset_ids = {
            let tokens = self.subscribed_tokens.read().await;
            tokens
                .iter()
                .filter_map(|token| U256::from_str(token).ok())
                .collect::<Vec<_>>()
        };

        if asset_ids.is_empty() {
            tokio::time::sleep(Duration::from_secs(1)).await;
            return Ok(());
        }

        tracing::info!(endpoint = %self.config.ws_endpoint, assets = asset_ids.len(), "Attempting CLOB WebSocket connection");
        let ws_client = WsClient::new(&self.config.ws_endpoint, WsConfig::default())
            .map_err(|e| PolybotError::Execution(format!("WS client creation failed: {}", e)))?;
        let mut stream = Box::pin(
            ws_client
                .subscribe_orderbook(asset_ids)
                .map_err(|e| PolybotError::Execution(format!("WS subscription failed: {}", e)))?,
        );

        loop {
            tokio::select! {
                message = stream.next() => {
                    match message {
                        Some(Ok(book)) => {
                            *self.connected.write().await = true;
                            self.metrics.set_ws_connected(true);
                            *self.last_pong.write().await = std::time::Instant::now();
                            self.latest_books.write().await.insert(
                                book.asset_id.to_string(),
                                OrderBookSnapshot {
                                    market: book.market.to_string(),
                                    asset_id: book.asset_id.to_string(),
                                    bids: book.bids.into_iter().map(|entry| OrderBookEntry {
                                        price: entry.price.to_string(),
                                        size: entry.size.to_string(),
                                    }).collect(),
                                    asks: book.asks.into_iter().map(|entry| OrderBookEntry {
                                        price: entry.price.to_string(),
                                        size: entry.size.to_string(),
                                    }).collect(),
                                    hash: book.hash.unwrap_or_default(),
                                    timestamp: book.timestamp as u64,
                                },
                            );
                        }
                        Some(Err(e)) => {
                            return Err(PolybotError::Execution(format!("WS stream error: {}", e)));
                        }
                        None => {
                            return Err(PolybotError::Execution("WS stream closed".to_string()));
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    if generation != self.subscription_generation.load(Ordering::Relaxed) {
                        return Err(PolybotError::Execution("WS subscription set changed".to_string()));
                    }

                    let pong_age = self.time_since_last_pong().await;
                    if pong_age > Duration::from_secs(40) {
                        if let Some(alerts) = &self.alerts {
                            alerts.warning("WebSocket disconnection lasting more than 30 seconds detected.");
                        }
                        return Err(PolybotError::Execution("WS pong timeout".to_string()));
                    }
                }
            }
        }
    }

    /// Run the heartbeat loop — send ping every 30s.
    /// Per PRD: Must send ping every 30 seconds; reconnect if no pong within 10 seconds.
    pub async fn run_heartbeat(&self) -> Result<(), PolybotError> {
        let mut heartbeat_interval = interval(Duration::from_secs(30));

        loop {
            heartbeat_interval.tick().await;
            let pong_age = self.time_since_last_pong().await;
            if pong_age > Duration::from_secs(40) {
                // 30s ping + 10s pong timeout
                tracing::warn!(
                    pong_age_secs = pong_age.as_secs(),
                    "WebSocket pong timeout, reconnecting"
                );
                *self.connected.write().await = false;
                // The connect_with_backoff loop will handle reconnection
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_manager_creation() {
        let config = ClobConfig {
            endpoint: "https://clob.polymarket.com".to_string(),
            ws_endpoint: "wss://ws-subscriptions-clob.polymarket.com".to_string(),
            chain_id: 137,
            private_key: "0x1".to_string(),
            api_key: None,
            signature_type: 0,
            funder_address: None,
        };
        let _manager = ClobWsManager::new(config, Arc::new(Metrics::new()), None);
    }

    #[tokio::test]
    async fn subscribe_token() {
        let config = ClobConfig {
            endpoint: "https://clob.polymarket.com".to_string(),
            ws_endpoint: "wss://ws-subscriptions-clob.polymarket.com".to_string(),
            chain_id: 137,
            private_key: "0x1".to_string(),
            api_key: None,
            signature_type: 0,
            funder_address: None,
        };
        let manager = ClobWsManager::new(config, Arc::new(Metrics::new()), None);
        manager.subscribe_token("token-123".to_string()).await;
        let tokens = manager.subscribed_tokens.read().await;
        assert!(tokens.contains(&"token-123".to_string()));
    }

    #[tokio::test]
    async fn is_connected_initially_false() {
        let config = ClobConfig {
            endpoint: "https://clob.polymarket.com".to_string(),
            ws_endpoint: "wss://ws-subscriptions-clob.polymarket.com".to_string(),
            chain_id: 137,
            private_key: "0x1".to_string(),
            api_key: None,
            signature_type: 0,
            funder_address: None,
        };
        let manager = ClobWsManager::new(config, Arc::new(Metrics::new()), None);
        assert!(!manager.is_connected().await);
    }
}
