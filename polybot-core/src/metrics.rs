use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::SystemTime;
use tokio::sync::broadcast;

/// Shared metrics state accessible from all modules.
/// Updated atomically by scanner, risk, execution, and state modules.
/// Read by health/metrics endpoints and Telegram commands.
#[derive(Debug)]
pub struct Metrics {
    // Signal counters
    pub signals_received: AtomicU64,
    pub signals_processed: AtomicU64,
    pub signals_skipped: AtomicU64,
    pub signals_manual_review: AtomicU64,

    // Trade counters
    pub trades_executed: AtomicU64,
    pub trades_simulated: AtomicU64,
    pub trades_failed: AtomicU64,

    // Position counters
    pub open_positions: AtomicU64,
    pub total_positions_opened: AtomicU64,
    pub total_positions_closed: AtomicU64,

    // PnL (stored as cents to use atomic u64 — divide by 100 for USD)
    pub daily_pnl_cents: AtomicI64,
    pub total_pnl_cents: AtomicI64,

    // Risk state
    pub current_drawdown_bps: AtomicU64, // basis points (1/100 of a percent)
    pub emergency_stops_triggered: AtomicU64,

    // Latency tracking (microseconds)
    pub avg_latency_us: AtomicU64,
    pub max_latency_us: AtomicU64,

    // Connection state
    pub ws_connected: AtomicU64,    // 0 = disconnected, 1 = connected
    pub rpc_healthy: AtomicU64,     // 0 = unhealthy, 1 = healthy
    pub data_api_latency_ms: AtomicU64, // last known Data API latency in ms
    pub paused: AtomicU64,          // 0 = active, 1 = paused

    // Event broadcast for real-time dashboard streaming
    pub event_tx: std::sync::Mutex<Option<broadcast::Sender<String>>>,

    // Timing
    pub start_time: SystemTime,
    pub last_signal_at: std::sync::Mutex<Option<String>>,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            signals_received: AtomicU64::new(0),
            signals_processed: AtomicU64::new(0),
            signals_skipped: AtomicU64::new(0),
            signals_manual_review: AtomicU64::new(0),
            trades_executed: AtomicU64::new(0),
            trades_simulated: AtomicU64::new(0),
            trades_failed: AtomicU64::new(0),
            open_positions: AtomicU64::new(0),
            total_positions_opened: AtomicU64::new(0),
            total_positions_closed: AtomicU64::new(0),
            daily_pnl_cents: AtomicI64::new(0),
            total_pnl_cents: AtomicI64::new(0),
            current_drawdown_bps: AtomicU64::new(0),
            emergency_stops_triggered: AtomicU64::new(0),
            avg_latency_us: AtomicU64::new(0),
            max_latency_us: AtomicU64::new(0),
            ws_connected: AtomicU64::new(0),
            rpc_healthy: AtomicU64::new(0),
            data_api_latency_ms: AtomicU64::new(0),
            paused: AtomicU64::new(0),
            event_tx: std::sync::Mutex::new(None),
            start_time: SystemTime::now(),
            last_signal_at: std::sync::Mutex::new(None),
        }
    }

    /// Record a signal received
    pub fn record_signal_received(&self) {
        self.signals_received.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut last) = self.last_signal_at.lock() {
            *last = Some(chrono::Utc::now().to_rfc3339());
        }
    }

    /// Record a signal processed (accepted by risk engine)
    pub fn record_signal_processed(&self) {
        self.signals_processed.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a signal skipped
    pub fn record_signal_skipped(&self) {
        self.signals_skipped.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a signal sent to manual review
    pub fn record_signal_manual_review(&self) {
        self.signals_manual_review.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a trade executed (or simulated)
    pub fn record_trade(&self, simulated: bool) {
        if simulated {
            self.trades_simulated.fetch_add(1, Ordering::Relaxed);
        } else {
            self.trades_executed.fetch_add(1, Ordering::Relaxed);
        }
        self.open_positions.fetch_add(1, Ordering::Relaxed);
        self.total_positions_opened.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a trade failure
    pub fn record_trade_failed(&self) {
        self.trades_failed.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a position closed
    pub fn record_position_closed(&self) {
        self.open_positions.fetch_sub(1, Ordering::Relaxed);
        self.total_positions_closed.fetch_add(1, Ordering::Relaxed);
    }

    /// Synchronize the current open position count with state manager data.
    pub fn set_open_positions(&self, count: u32) {
        self.open_positions.store(count as u64, Ordering::Relaxed);
    }

    /// Update daily PnL (in cents)
    pub fn update_daily_pnl(&self, pnl_usd: f64) {
        let cents = (pnl_usd * 100.0).round() as i64;
        self.daily_pnl_cents.store(cents, Ordering::Relaxed);
    }

    /// Update drawdown (in basis points, e.g. 5% = 500 bps)
    pub fn update_drawdown(&self, drawdown_pct: f64) {
        let bps = (drawdown_pct * 10000.0) as u64;
        self.current_drawdown_bps.store(bps, Ordering::Relaxed);
    }

    /// Record execution latency (in microseconds)
    pub fn record_latency(&self, latency_us: u64) {
        let current_max = self.max_latency_us.load(Ordering::Relaxed);
        if latency_us > current_max {
            self.max_latency_us.store(latency_us, Ordering::Relaxed);
        }
        // Simple moving average approximation
        let current_avg = self.avg_latency_us.load(Ordering::Relaxed);
        if current_avg == 0 {
            self.avg_latency_us.store(latency_us, Ordering::Relaxed);
        } else {
            // Exponential moving average: avg = 0.9 * avg + 0.1 * new
            let new_avg = current_avg * 9 / 10 + latency_us / 10;
            self.avg_latency_us.store(new_avg, Ordering::Relaxed);
        }
    }

    /// Record an emergency stop
    pub fn record_emergency_stop(&self) {
        self.emergency_stops_triggered
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Set Data API latency in milliseconds
    pub fn set_data_api_latency_ms(&self, ms: u64) {
        self.data_api_latency_ms.store(ms, Ordering::Relaxed);
    }
    pub fn set_ws_connected(&self, connected: bool) {
        self.ws_connected
            .store(if connected { 1 } else { 0 }, Ordering::Relaxed);
    }

    /// Set RPC health state
    pub fn set_rpc_healthy(&self, healthy: bool) {
        self.rpc_healthy
            .store(if healthy { 1 } else { 0 }, Ordering::Relaxed);
    }


    pub fn set_paused(&self, paused: bool) {
        self.paused
            .store(if paused { 1 } else { 0 }, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed) == 1
    }

    /// Get uptime in seconds
    pub fn uptime_secs(&self) -> u64 {
        self.start_time
            .elapsed()
            .unwrap_or(std::time::Duration::from_secs(0))
            .as_secs()
    }

    /// Get daily PnL in USD
    pub fn daily_pnl_usd(&self) -> f64 {
        self.daily_pnl_cents.load(Ordering::Relaxed) as f64 / 100.0
    }

    /// Get current drawdown as percentage
    pub fn current_drawdown_pct(&self) -> f64 {
        self.current_drawdown_bps.load(Ordering::Relaxed) as f64 / 10000.0
    }

    /// Attach event broadcast sender for real-time dashboard streaming
    pub fn set_event_tx(&self, tx: broadcast::Sender<String>) {
        if let Ok(mut guard) = self.event_tx.lock() {
            *guard = Some(tx);
        }
    }

    /// Broadcast a JSON event string to dashboard WebSocket clients (fire-and-forget)
    pub fn broadcast_event(
        &self,
        event_type: &str,
        payload: serde_json::Value,
    ) {
        if let Ok(guard) = self.event_tx.lock() {
            if let Some(ref tx) = *guard {
                let msg = serde_json::json!({
                    "type": event_type,
                    "data": payload,
                    "ts": chrono::Utc::now().to_rfc3339(),
                })
                .to_string();
                let _ = tx.send(msg);
            }
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_creation() {
        let m = Metrics::new();
        assert_eq!(m.signals_received.load(Ordering::Relaxed), 0);
        assert_eq!(m.trades_executed.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn record_signals() {
        let m = Metrics::new();
        m.record_signal_received();
        m.record_signal_received();
        m.record_signal_processed();
        m.record_signal_skipped();
        assert_eq!(m.signals_received.load(Ordering::Relaxed), 2);
        assert_eq!(m.signals_processed.load(Ordering::Relaxed), 1);
        assert_eq!(m.signals_skipped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn record_trades() {
        let m = Metrics::new();
        m.record_trade(true);
        m.record_trade(false);
        assert_eq!(m.trades_simulated.load(Ordering::Relaxed), 1);
        assert_eq!(m.trades_executed.load(Ordering::Relaxed), 1);
        assert_eq!(m.open_positions.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn daily_pnl() {
        let m = Metrics::new();
        m.update_daily_pnl(123.45);
        assert!((m.daily_pnl_usd() - 123.45).abs() < 0.01);
    }

    #[test]
    fn daily_pnl_supports_losses() {
        let m = Metrics::new();
        m.update_daily_pnl(-12.34);
        assert!((m.daily_pnl_usd() + 12.34).abs() < 0.01);
    }

    #[test]
    fn latency_tracking() {
        let m = Metrics::new();
        m.record_latency(500);
        m.record_latency(600);
        m.record_latency(1000);
        assert!(m.max_latency_us.load(Ordering::Relaxed) >= 1000);
    }

    #[test]
    fn record_latency_updates_average_and_max() {
        let m = Metrics::new();
        m.record_latency(300);
        m.record_latency(900);
        assert!(m.max_latency_us.load(Ordering::Relaxed) >= 900);
        assert!(
            m.avg_latency_us.load(Ordering::Relaxed) >= 360,
            "expected average latency to track the latest samples"
        );
    }

    #[test]
    fn connection_state() {
        let m = Metrics::new();
        m.set_ws_connected(true);
        m.set_rpc_healthy(false);
        assert_eq!(m.ws_connected.load(Ordering::Relaxed), 1);
        assert_eq!(m.rpc_healthy.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn paused_state() {
        let m = Metrics::new();
        assert!(!m.is_paused());
        m.set_paused(true);
        assert!(m.is_paused());
    }

    #[test]
    fn drawdown() {
        let m = Metrics::new();
        m.update_drawdown(0.075); // 7.5%
        assert!((m.current_drawdown_pct() - 0.075).abs() < 0.001);
    }
}
