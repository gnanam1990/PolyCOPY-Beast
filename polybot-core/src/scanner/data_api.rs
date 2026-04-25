use std::sync::Arc;
use std::time::Duration;

use polybot_common::errors::PolybotError;
use polybot_common::types::{ScannerEvent, SignalSource};
use serde_json::Value;
use tokio::sync::{mpsc, RwLock};

use crate::config::AppConfig;
use crate::metrics::Metrics;
use crate::risk::RiskEngine;

use super::schema::normalize_data_api_trade;
use super::wallet_tracker::{category_allowed, WalletActivityState, WalletPollTrigger};

pub struct DataApiPoller {
    client: reqwest::Client,
    base_url: String,
    poll_interval_ms: u64,
    signal_max_age_secs: u64,
    allowed_categories: Vec<polybot_common::types::Category>,
    state: Arc<RwLock<WalletActivityState>>,
    metrics: Option<Arc<Metrics>>,
}

impl DataApiPoller {
    pub fn new(
        config: &AppConfig,
        state: Arc<RwLock<WalletActivityState>>,
        metrics: Arc<Metrics>,
    ) -> Result<Self, PolybotError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| {
                PolybotError::Scanner(format!("Failed to build Data API client: {}", e))
            })?;

        Ok(Self {
            client,
            base_url: config
                .scanner
                .data_api_url
                .trim_end_matches('/')
                .to_string(),
            poll_interval_ms: config.scanner.poll_interval_ms,
            signal_max_age_secs: config.scanner.signal_max_age_secs,
            allowed_categories: config.scanner.target_categories.clone(),
            state,
            metrics: Some(metrics),
        })
    }

    pub async fn run(
        &self,
        risk_engine: Arc<RiskEngine>,
        sender: mpsc::Sender<ScannerEvent>,
        mut trigger_rx: mpsc::Receiver<WalletPollTrigger>,
    ) -> Result<(), PolybotError> {
        let mut interval = tokio::time::interval(Duration::from_millis(self.poll_interval_ms));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let wallets = risk_engine.list_followed_wallets().await;
                    self.state.write().await.retain_wallets(&wallets);
                    for wallet in wallets {
                        self.poll_wallet(&wallet, SignalSource::Polling, &sender).await?;
                    }
                }
                Some(trigger) = trigger_rx.recv() => {
                    self.poll_wallet(&trigger.wallet, trigger.source, &sender).await?;
                }
                else => return Ok(()),
            }
        }
    }

    async fn poll_wallet(
        &self,
        wallet: &str,
        source: SignalSource,
        sender: &mpsc::Sender<ScannerEvent>,
    ) -> Result<(), PolybotError> {
        let last_seen = self.state.read().await.last_seen_for_wallet(wallet);
        let mut query = vec![
            ("user", wallet.to_string()),
            ("type", "TRADE".to_string()),
            ("limit", "100".to_string()),
            ("sortBy", "TIMESTAMP".to_string()),
            ("sortDirection", "DESC".to_string()),
        ];

        if let Some(last_seen) = last_seen {
            query.push(("start", last_seen.to_string()));
        }

        let start = std::time::Instant::now();
        let response = self
            .client
            .get(format!("{}/activity", self.base_url))
            .query(&query)
            .send()
            .await
            .map_err(|e| {
                PolybotError::Scanner(format!("Data API activity request failed: {}", e))
            })?;

        let elapsed_ms = start.elapsed().as_millis() as u64;
        if let Some(ref metrics) = self.metrics {
            metrics.set_data_api_latency_ms(elapsed_ms);
        }

        if !response.status().is_success() {
            return Err(PolybotError::Scanner(format!(
                "Data API activity request failed with status {}",
                response.status()
            )));
        }

        let payload: Value = response.json().await.map_err(|e| {
            PolybotError::Scanner(format!("Invalid Data API activity response: {}", e))
        })?;

        let activities = extract_activity_items(payload);
        let mut max_seen = last_seen.unwrap_or_default();

        for activity in activities {
            let activity_timestamp = activity
                .get("timestamp")
                .and_then(|value| {
                    value
                        .as_i64()
                        .or_else(|| value.as_str().and_then(|s| s.parse::<i64>().ok()))
                })
                .unwrap_or_default();
            max_seen = max_seen.max(activity_timestamp);

            let mut signal = match normalize_data_api_trade(&activity.to_string(), source) {
                Ok(signal) => signal,
                Err(error) => {
                    tracing::debug!(error = %error, wallet = %wallet, "Skipping non-normalizable Data API activity");
                    continue;
                }
            };

            if !category_allowed(signal.category, &self.allowed_categories) {
                continue;
            }

            if signal
                .validate_with_max_age_secs(self.signal_max_age_secs as i64)
                .is_err()
            {
                continue;
            }

            signal.wallet_address = wallet.to_lowercase();

            self.state.write().await.record_activity(
                wallet,
                signal.token_id.as_deref(),
                activity_timestamp,
            );

            sender
                .send(ScannerEvent {
                    signal,
                    received_at: std::time::Instant::now(),
                })
                .await
                .map_err(|_| PolybotError::ChannelClosed)?;
        }

        if max_seen > 0 {
            self.state
                .write()
                .await
                .record_activity(wallet, None, max_seen);
        }

        Ok(())
    }
}

fn extract_activity_items(payload: Value) -> Vec<Value> {
    match payload {
        Value::Array(items) => items,
        Value::Object(map) => map
            .get("items")
            .or_else(|| map.get("activity"))
            .or_else(|| map.get("data"))
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}
