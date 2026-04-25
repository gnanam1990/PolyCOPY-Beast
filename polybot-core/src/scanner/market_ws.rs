use std::str::FromStr as _;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use polybot_common::errors::PolybotError;
use polybot_common::types::SignalSource;
use polymarket_client_sdk::clob::ws::Client as WsClient;
use polymarket_client_sdk::types::U256;
use polymarket_client_sdk::ws::config::Config as WsConfig;
use tokio::sync::{mpsc, RwLock};

use crate::config::AppConfig;

use super::wallet_tracker::{WalletActivityState, WalletPollTrigger};

pub struct MarketFastPath {
    ws_endpoint: String,
    state: Arc<RwLock<WalletActivityState>>,
}

impl MarketFastPath {
    pub fn new(_config: &AppConfig, state: Arc<RwLock<WalletActivityState>>) -> Self {
        Self {
            ws_endpoint: std::env::var("POLYBOT_WS_ENDPOINT")
                .or_else(|_| std::env::var("WS_CLOB_URL"))
                .unwrap_or_else(|_| "wss://ws-subscriptions-clob.polymarket.com".to_string()),
            state,
        }
    }

    pub async fn run(
        &self,
        trigger_tx: mpsc::Sender<WalletPollTrigger>,
    ) -> Result<(), PolybotError> {
        loop {
            let assets = self.state.read().await.tracked_assets();
            if assets.is_empty() {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }

            if let Err(error) = self.stream_assets(assets, &trigger_tx).await {
                tracing::warn!(error = %error, "Market fast-path websocket restarting after error");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }

    async fn stream_assets(
        &self,
        assets: Vec<String>,
        trigger_tx: &mpsc::Sender<WalletPollTrigger>,
    ) -> Result<(), PolybotError> {
        let asset_ids = assets
            .iter()
            .filter_map(|asset| U256::from_str(asset).ok())
            .collect::<Vec<_>>();

        if asset_ids.is_empty() {
            tokio::time::sleep(Duration::from_secs(1)).await;
            return Ok(());
        }

        let ws_client = WsClient::new(&self.ws_endpoint, WsConfig::default()).map_err(|e| {
            PolybotError::Scanner(format!("Market WS client creation failed: {}", e))
        })?;

        let mut stream =
            Box::pin(ws_client.subscribe_orderbook(asset_ids).map_err(|e| {
                PolybotError::Scanner(format!("Market WS subscription failed: {}", e))
            })?);

        loop {
            tokio::select! {
                message = stream.next() => {
                    match message {
                        Some(Ok(book)) => {
                            let asset_id = book.asset_id.to_string();
                            let wallets = self.state.read().await.wallets_for_asset(&asset_id);
                            for wallet in wallets {
                                trigger_tx.send(WalletPollTrigger {
                                    wallet,
                                    source: SignalSource::Websocket,
                                }).await.map_err(|_| PolybotError::ChannelClosed)?;
                            }
                        }
                        Some(Err(error)) => {
                            return Err(PolybotError::Scanner(format!("Market WS stream error: {}", error)));
                        }
                        None => return Err(PolybotError::Scanner("Market WS stream closed".to_string())),
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(2)) => {
                    let current_assets = self.state.read().await.tracked_assets();
                    if current_assets != assets {
                        return Ok(());
                    }
                }
            }
        }
    }
}
