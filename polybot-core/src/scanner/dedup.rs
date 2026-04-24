use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing;

use polybot_common::errors::PolybotError;
use polybot_common::types::ScannerEvent;

pub struct DedupFilter {
    seen: HashMap<String, Instant>,
    window: Duration,
}

impl DedupFilter {
    pub fn new(window_secs: u64) -> Self {
        Self {
            seen: HashMap::new(),
            window: Duration::from_secs(window_secs),
        }
    }

    /// v2.5: Dedup key is signal_id within rolling 5-minute window
    pub fn dedup_key(signal: &polybot_common::types::Signal) -> String {
        signal
            .tx_hash
            .clone()
            .unwrap_or_else(|| signal.signal_id.clone())
    }

    pub fn check_and_record(&mut self, event: &ScannerEvent) -> bool {
        let key = Self::dedup_key(&event.signal);
        let now = Instant::now();

        if let Some(seen_at) = self.seen.get(&key) {
            if now.duration_since(*seen_at) < self.window {
                tracing::debug!(key = %key, "Duplicate signal filtered within 5-min window");
                return false;
            }
        }

        self.seen.insert(key, now);
        true
    }

    pub fn cleanup(&mut self) {
        let now = Instant::now();
        self.seen
            .retain(|_, seen_at| now.duration_since(*seen_at) < self.window);
    }
}

pub async fn run_dedup_task(
    mut receiver: mpsc::Receiver<ScannerEvent>,
    sender: mpsc::Sender<ScannerEvent>,
    window_secs: u64,
) -> Result<(), PolybotError> {
    let mut filter = DedupFilter::new(window_secs);
    let mut cleanup_interval = tokio::time::interval(Duration::from_secs(60));

    loop {
        tokio::select! {
            Some(event) = receiver.recv() => {
                if filter.check_and_record(&event)
                    && sender.send(event).await.is_err()
                {
                    tracing::error!("Downstream channel closed");
                    return Err(PolybotError::ChannelClosed);
                }
            }
            _ = cleanup_interval.tick() => {
                filter.cleanup();
            }
            else => {
                tracing::info!("Dedup input channel closed, shutting down");
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use polybot_common::types::*;
    use rust_decimal_macros::dec;

    fn test_event(signal_id: &str) -> ScannerEvent {
        ScannerEvent {
            signal: Signal {
                signal_id: signal_id.to_string(),
                timestamp: "2026-04-14T12:34:56.789Z".to_string(),
                wallet_address: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
                market_id: "market-1".to_string(),
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
                fee_schedule: None,
                scanner_version: "1.0.0".to_string(),
            },
            received_at: Instant::now(),
        }
    }

    #[test]
    fn dedup_allows_new_signal() {
        let mut filter = DedupFilter::new(300);
        let event = test_event("sig-1");
        assert!(filter.check_and_record(&event));
    }

    #[test]
    fn dedup_filters_duplicate_by_signal_id() {
        let mut filter = DedupFilter::new(300);
        let event = test_event("sig-1");
        assert!(filter.check_and_record(&event));
        let event2 = test_event("sig-1");
        assert!(!filter.check_and_record(&event2));
    }

    #[test]
    fn dedup_allows_different_signal_id() {
        let mut filter = DedupFilter::new(300);
        let event1 = test_event("sig-1");
        let event2 = test_event("sig-2");
        assert!(filter.check_and_record(&event1));
        assert!(filter.check_and_record(&event2));
    }

    #[test]
    fn dedup_prefers_tx_hash_when_present() {
        let mut filter = DedupFilter::new(300);
        let mut event1 = test_event("sig-1");
        event1.signal.tx_hash = Some(
            "0xfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeedfeed"
                .to_string(),
        );
        let mut event2 = test_event("sig-2");
        event2.signal.tx_hash = event1.signal.tx_hash.clone();

        assert!(filter.check_and_record(&event1));
        assert!(!filter.check_and_record(&event2));
    }
}
