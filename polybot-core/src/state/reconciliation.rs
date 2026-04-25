use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use polybot_common::constants::{
    FULL_RECONCILIATION_INTERVAL_SECS, LIGHT_RECONCILIATION_INTERVAL_SECS,
};
use polybot_common::errors::PolybotError;
use polybot_common::types::{Category, Position, PositionKey, Side};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tokio::sync::Mutex;

/// Tolerance for comparing share sizes across sources.
/// Data API indexing may round or lag by a fraction of a share;
/// anything within 0.1 shares is treated as equal.
const SIZE_TOLERANCE: Decimal = dec!(0.1);

/// Tolerance for comparing prices (probability-equivalent USD).
/// Polymarket prices move in 1-cent ticks; anything within 0.01 is
/// treated as equal.
const PRICE_TOLERANCE: Decimal = dec!(0.01);

use crate::config::AppConfig;
use crate::execution::clob_client::ClobClient;
use crate::metrics::Metrics;
use crate::telegram_bot::alerts::AlertBroadcaster;

use super::{
    positions::PositionManager,
    sqlite::{PersistedPositionRow, SqliteStore},
};

/// v2.5: Reconciliation engine with light (30s) and full (5min) modes.
/// Source of truth: remote trading state in live/shadow, SQLite snapshot in simulation.
pub struct Reconciler {
    config: Arc<AppConfig>,
    metrics: Arc<Metrics>,
    light_interval_secs: u64,
    full_interval_secs: u64,
    position_manager: Arc<Mutex<PositionManager>>,
    sqlite_path: String,
    alerts: Option<AlertBroadcaster>,
    auto_heal: bool,
}

impl Reconciler {
    pub fn new(
        config: Arc<AppConfig>,
        metrics: Arc<Metrics>,
        position_manager: Arc<Mutex<PositionManager>>,
        auto_heal: bool,
    ) -> Self {
        Self {
            config,
            metrics,
            light_interval_secs: LIGHT_RECONCILIATION_INTERVAL_SECS,
            full_interval_secs: FULL_RECONCILIATION_INTERVAL_SECS,
            position_manager,
            sqlite_path: std::env::var("POLYBOT_SQLITE_PATH")
                .unwrap_or_else(|_| "./polybot.db".to_string()),
            alerts: None,
            auto_heal,
        }
    }

    pub fn with_intervals(mut self, light: u64, full: u64) -> Self {
        self.light_interval_secs = light;
        self.full_interval_secs = full;
        self
    }

    pub fn with_alerts(mut self, alerts: Option<AlertBroadcaster>) -> Self {
        self.alerts = alerts;
        self
    }

    /// Run light reconciliation
    pub async fn run_light(&self) -> Result<ReconciliationResult, PolybotError> {
        tracing::debug!("Running light reconciliation");
        self.reconcile(false).await
    }

    /// Run full reconciliation
    pub async fn run_full(&self) -> Result<ReconciliationResult, PolybotError> {
        tracing::debug!("Running full reconciliation");
        self.reconcile(true).await
    }

    /// Force reconciliation (triggered by /reconcile force command)
    pub async fn force_reconcile(&self) -> Result<ReconciliationResult, PolybotError> {
        tracing::info!("Force reconciliation triggered by operator");
        self.run_full().await
    }

    /// Run the continuous reconciliation loop
    pub async fn run_loop(&self) -> Result<(), PolybotError> {
        let mut light_interval =
            tokio::time::interval(std::time::Duration::from_secs(self.light_interval_secs));
        let mut full_interval =
            tokio::time::interval(std::time::Duration::from_secs(self.full_interval_secs));

        tracing::info!(
            light_interval_secs = self.light_interval_secs,
            full_interval_secs = self.full_interval_secs,
            "Reconciliation loop started"
        );

        loop {
            tokio::select! {
                _ = light_interval.tick() => {
                    match self.run_light().await {
                        Ok(result) => {
                            if result.has_issues() {
                                tracing::warn!(
                                    ghosts = result.ghost_positions.len(),
                                    missing = result.missing_positions.len(),
                                    mismatches = result.mismatches.len(),
                                    "Light reconciliation found issues"
                                );
                                if let (Some(alerts), Some(message)) = (&self.alerts, result.alert_message()) {
                                    alerts.warning(message);
                                }
                            }
                        }
                        Err(e) => tracing::error!(error = %e, "Light reconciliation failed"),
                    }
                }
                _ = full_interval.tick() => {
                    match self.run_full().await {
                        Ok(result) => {
                            if result.has_issues() {
                                tracing::warn!(
                                    ghosts = result.ghost_positions.len(),
                                    missing = result.missing_positions.len(),
                                    mismatches = result.mismatches.len(),
                                    "Full reconciliation found issues"
                                );
                                if let (Some(alerts), Some(message)) = (&self.alerts, result.alert_message()) {
                                    alerts.warning(message);
                                }
                            }
                        }
                        Err(e) => tracing::error!(error = %e, "Full reconciliation failed"),
                    }
                }
            }
        }
    }

    async fn reconcile(&self, apply_snapshot: bool) -> Result<ReconciliationResult, PolybotError> {
        let local_positions = self.local_snapshots().await;
        let reference = self.reference_snapshots(apply_snapshot).await?;
        let result = diff_snapshots(&local_positions, &reference.positions);

        if apply_snapshot {
            self.sync_to_reference(&local_positions, &reference).await?;
        }

        self.persist_last_reconciliation_at()?;

        Ok(result)
    }

    async fn local_snapshots(&self) -> Vec<PositionSnapshot> {
        let positions = self.position_manager.lock().await;
        positions
            .get_positions_vec()
            .into_iter()
            .map(PositionSnapshot::from_position)
            .collect()
    }

    async fn reference_snapshots(
        &self,
        apply_snapshot: bool,
    ) -> Result<ReferenceSnapshot, PolybotError> {
        if apply_snapshot
            && self
                .config
                .system
                .execution_mode
                .allows_network_market_data()
        {
            return Ok(ReferenceSnapshot {
                source: SnapshotSource::RemoteDataApi,
                positions: self.fetch_remote_positions().await?,
            });
        }

        Ok(ReferenceSnapshot {
            source: SnapshotSource::Sqlite,
            positions: self.fetch_sqlite_positions()?,
        })
    }

    async fn fetch_remote_positions(&self) -> Result<Vec<PositionSnapshot>, PolybotError> {
        let clob_client = ClobClient::from_env()?;
        let wallet_address = clob_client.trading_wallet_address()?;
        let client = polymarket_client_sdk::data::Client::new(&self.config.scanner.data_api_url)
            .map_err(|e| PolybotError::State(format!("Failed to create Data API client: {}", e)))?;
        let mut offset = 0;
        let mut snapshots = Vec::new();

        loop {
            let request = polymarket_client_sdk::data::types::request::PositionsRequest::builder()
                .user(wallet_address)
                .limit(500)
                .map_err(|e| {
                    PolybotError::State(format!("Invalid reconciliation positions request: {}", e))
                })?
                .offset(offset)
                .map_err(|e| {
                    PolybotError::State(format!("Invalid reconciliation positions offset: {}", e))
                })?
                .build();
            let positions = client.positions(&request).await.map_err(|e| {
                PolybotError::State(format!("Failed to fetch remote positions: {}", e))
            })?;
            let batch_size = positions.len();

            snapshots.extend(
                positions
                    .into_iter()
                    .filter_map(PositionSnapshot::from_remote_position),
            );

            if batch_size < 500 {
                break;
            }

            offset += batch_size as i32;
        }

        Ok(snapshots)
    }

    fn fetch_sqlite_positions(&self) -> Result<Vec<PositionSnapshot>, PolybotError> {
        let store = self.open_store()?;
        store.list_open_positions().map(|rows| {
            rows.into_iter()
                .map(PositionSnapshot::from_persisted_position)
                .collect()
        })
    }

    fn persist_last_reconciliation_at(&self) -> Result<(), PolybotError> {
        let store = self.open_store()?;
        store.set_config("last_reconciliation_at", &chrono::Utc::now().to_rfc3339())
    }

    fn open_store(&self) -> Result<SqliteStore, PolybotError> {
        SqliteStore::open(std::path::Path::new(&self.sqlite_path))
    }

    async fn sync_to_reference(
        &self,
        local_positions: &[PositionSnapshot],
        reference: &ReferenceSnapshot,
    ) -> Result<(), PolybotError> {
        let store = self.open_store()?;
        let persisted_rows = store.list_open_positions()?;
        let persisted_by_key = persisted_rows
            .into_iter()
            .map(PositionSnapshot::from_persisted_position)
            .map(|snapshot| (snapshot.key(), snapshot))
            .collect::<HashMap<_, _>>();
        let local_by_key = local_positions
            .iter()
            .cloned()
            .map(|snapshot| (snapshot.key(), snapshot))
            .collect::<HashMap<_, _>>();

        let restored_snapshots = reference.positions.clone();

        let reference_keys = restored_snapshots
            .iter()
            .map(PositionSnapshot::key)
            .collect::<HashSet<_>>();

        let restored_positions = restored_snapshots
            .iter()
            .map(|snapshot| {
                snapshot.to_position(
                    local_by_key.get(&snapshot.key()),
                    persisted_by_key.get(&snapshot.key()),
                )
            })
            .collect::<Vec<_>>();

        let ghost_keys: Vec<PositionKey> = persisted_by_key
            .values()
            .filter(|snapshot| !reference_keys.contains(&snapshot.key()))
            .map(|snapshot| snapshot.key())
            .collect();

        if !self.auto_heal {
            if !restored_positions.is_empty() || !ghost_keys.is_empty() {
                tracing::warn!(
                    restore_count = restored_positions.len(),
                    ghost_count = ghost_keys.len(),
                    "Reconciliation would heal state \
                     (auto_heal=false; set reconciliation.auto_heal=true \
                     in config to enable). See debug logs for details."
                );
                for pos in &restored_positions {
                    tracing::debug!(
                        market_id = %pos.market_id,
                        side = ?pos.side,
                        size = %pos.current_size,
                        avg_price = %pos.average_price,
                        "Would restore position from reference"
                    );
                }
                for key in &ghost_keys {
                    tracing::debug!(
                        market_id = %key.market_id,
                        side = ?key.side,
                        "Would mark SQLite position as ghost"
                    );
                }
            }
            return Ok(());
        }

        {
            let mut manager = self.position_manager.lock().await;
            manager.restore_positions(restored_positions.clone());
        }
        self.metrics
            .set_open_positions(restored_positions.len() as u32);

        for persisted in persisted_by_key.values() {
            if !reference_keys.contains(&persisted.key()) {
                let mut ghost = persisted.to_position(None, None);
                ghost.status = polybot_common::types::PositionStatus::Ghost;
                store.upsert_position(
                    &ghost,
                    persisted.current_price,
                    persisted.unrealized_pnl,
                    persisted.owned_by_wallet.as_deref(),
                )?;
            }
        }

        for snapshot in &restored_snapshots {
            let key = snapshot.key();
            let restored = snapshot.to_position(local_by_key.get(&key), persisted_by_key.get(&key));
            let owner = snapshot.owned_by_wallet.clone().or_else(|| {
                persisted_by_key
                    .get(&key)
                    .and_then(|entry| entry.owned_by_wallet.clone())
            });
            store.upsert_position(
                &restored,
                snapshot.current_price,
                snapshot.unrealized_pnl,
                owner.as_deref(),
            )?;
        }

        tracing::info!(
            restored = restored_positions.len(),
            "Reconciliation auto-healed local state"
        );

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotSource {
    Sqlite,
    RemoteDataApi,
}

#[derive(Debug, Clone)]
struct ReferenceSnapshot {
    source: SnapshotSource,
    positions: Vec<PositionSnapshot>,
}

#[derive(Debug, Clone, PartialEq)]
struct PositionSnapshot {
    market_id: String,
    side: Side,
    current_size: Decimal,
    average_price: Decimal,
    current_price: Option<Decimal>,
    unrealized_pnl: Option<Decimal>,
    category: Category,
    id: Option<String>,
    opened_at: Option<chrono::DateTime<chrono::Utc>>,
    owned_by_wallet: Option<String>,
}

impl PositionSnapshot {
    fn key(&self) -> PositionKey {
        PositionKey::new(self.market_id.clone(), self.side)
    }

    fn from_position(position: &Position) -> Self {
        Self {
            market_id: position.market_id.to_lowercase(),
            side: position.side,
            current_size: position.current_size,
            average_price: position.average_price,
            current_price: None,
            unrealized_pnl: None,
            category: position.category,
            id: Some(position.id.clone()),
            opened_at: Some(position.opened_at),
            owned_by_wallet: None,
        }
    }

    fn from_persisted_position(row: PersistedPositionRow) -> Self {
        Self {
            market_id: row.position.market_id.to_lowercase(),
            side: row.position.side,
            current_size: row.position.current_size,
            average_price: row.position.average_price,
            current_price: row.current_price,
            unrealized_pnl: row.unrealized_pnl,
            category: row.position.category,
            id: Some(row.position.id),
            opened_at: Some(row.position.opened_at),
            owned_by_wallet: row.owned_by_wallet,
        }
    }

    fn from_remote_position(
        position: polymarket_client_sdk::data::types::response::Position,
    ) -> Option<Self> {
        let side = match position.outcome.to_ascii_lowercase().as_str() {
            "yes" => Side::Yes,
            "no" => Side::No,
            other => {
                tracing::warn!(
                    market_id = %position.condition_id,
                    outcome = %other,
                    "Skipping unsupported remote position outcome during reconciliation"
                );
                return None;
            }
        };

        Some(Self {
            market_id: position.condition_id.to_string().to_lowercase(),
            side,
            current_size: position.size,
            average_price: position.avg_price,
            current_price: Some(position.cur_price),
            unrealized_pnl: Some(position.cash_pnl),
            category: Category::Other,
            id: None,
            opened_at: None,
            owned_by_wallet: None,
        })
    }

    fn to_position(
        &self,
        local: Option<&PositionSnapshot>,
        persisted: Option<&PositionSnapshot>,
    ) -> Position {
        let metadata = local.or(persisted);
        Position {
            id: metadata
                .and_then(|entry| entry.id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            market_id: self.market_id.clone(),
            side: self.side,
            entry_price: self.average_price,
            current_size: self.current_size,
            average_price: self.average_price,
            opened_at: metadata
                .and_then(|entry| entry.opened_at.as_ref().cloned())
                .unwrap_or_else(chrono::Utc::now),
            status: polybot_common::types::PositionStatus::Open,
            category: metadata
                .map(|entry| entry.category)
                .unwrap_or(self.category),
        }
    }
}

fn diff_snapshots(
    local_positions: &[PositionSnapshot],
    reference_positions: &[PositionSnapshot],
) -> ReconciliationResult {
    let local_by_key = local_positions
        .iter()
        .map(|snapshot| (snapshot.key(), snapshot))
        .collect::<HashMap<_, _>>();
    let reference_by_key = reference_positions
        .iter()
        .map(|snapshot| (snapshot.key(), snapshot))
        .collect::<HashMap<_, _>>();

    let mut ghost_positions = local_by_key
        .keys()
        .filter(|key| !reference_by_key.contains_key(*key))
        .map(format_position_key)
        .collect::<Vec<_>>();
    let mut missing_positions = reference_by_key
        .keys()
        .filter(|key| !local_by_key.contains_key(*key))
        .map(format_position_key)
        .collect::<Vec<_>>();
    let mut mismatches = reference_by_key
        .iter()
        .filter_map(|(key, reference)| {
            let local = local_by_key.get(key)?;
            if sizes_close(local.current_size, reference.current_size)
                && prices_close(local.average_price, reference.average_price)
            {
                None
            } else {
                Some(format_position_key(key))
            }
        })
        .collect::<Vec<_>>();

    ghost_positions.sort();
    missing_positions.sort();
    mismatches.sort();

    let checked = local_by_key
        .keys()
        .chain(reference_by_key.keys())
        .cloned()
        .collect::<HashSet<_>>()
        .len() as u32;

    ReconciliationResult {
        checked,
        ghost_positions,
        missing_positions,
        mismatches,
    }
}

fn sizes_close(left: Decimal, right: Decimal) -> bool {
    (left - right).abs() <= SIZE_TOLERANCE
}

fn prices_close(left: Decimal, right: Decimal) -> bool {
    (left - right).abs() <= PRICE_TOLERANCE
}

/// Deprecated alias preserved for any out-of-file callers; routes to
/// [`sizes_close`]. Prefer the typed helpers above.
#[deprecated(note = "use sizes_close or prices_close for explicit intent")]
#[allow(dead_code)]
fn decimals_close(left: Decimal, right: Decimal) -> bool {
    sizes_close(left, right)
}

fn format_position_key(key: &PositionKey) -> String {
    let side = match key.side {
        Side::Yes => "YES",
        Side::No => "NO",
    };
    format!("{}:{}", key.market_id, side)
}

#[derive(Debug, Clone)]
pub struct ReconciliationResult {
    pub checked: u32,
    pub ghost_positions: Vec<String>,
    pub missing_positions: Vec<String>,
    pub mismatches: Vec<String>,
}

impl ReconciliationResult {
    pub fn has_issues(&self) -> bool {
        !self.ghost_positions.is_empty()
            || !self.missing_positions.is_empty()
            || !self.mismatches.is_empty()
    }

    pub fn alert_message(&self) -> Option<String> {
        if !self.has_issues() {
            return None;
        }

        Some(format!(
            "Reconciliation mismatch detected: checked={} ghost={} missing={} mismatches={}",
            self.checked,
            self.ghost_positions.len(),
            self.missing_positions.len(),
            self.mismatches.len(),
        ))
    }

    pub fn summary(&self) -> String {
        format!(
            "Checked: {}\nGhost positions: {}\nMissing positions: {}\nMismatches: {}",
            self.checked,
            self.ghost_positions.len(),
            self.missing_positions.len(),
            self.mismatches.len(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::metrics::Metrics;
    use polybot_common::types::{PositionStatus, Side};
    use rust_decimal_macros::dec;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn test_position(market_id: &str, side: Side, size: rust_decimal::Decimal) -> Position {
        Position {
            id: format!("{}-{side:?}", market_id),
            market_id: market_id.to_string(),
            side,
            entry_price: dec!(0.55),
            current_size: size,
            average_price: dec!(0.55),
            opened_at: chrono::Utc::now(),
            status: PositionStatus::Open,
            category: Category::Politics,
        }
    }

    fn test_reconciler(
        position_manager: Arc<Mutex<PositionManager>>,
    ) -> (
        Reconciler,
        Arc<Metrics>,
        std::path::PathBuf,
        std::sync::MutexGuard<'static, ()>,
    ) {
        test_reconciler_with_auto_heal(position_manager, false)
    }

    fn test_reconciler_with_auto_heal(
        position_manager: Arc<Mutex<PositionManager>>,
        auto_heal: bool,
    ) -> (
        Reconciler,
        Arc<Metrics>,
        std::path::PathBuf,
        std::sync::MutexGuard<'static, ()>,
    ) {
        let guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let sqlite_path =
            std::env::temp_dir().join(format!("polybot-reconcile-{}.db", uuid::Uuid::new_v4()));
        std::env::set_var("POLYBOT_SQLITE_PATH", &sqlite_path);

        let metrics = Arc::new(Metrics::new());
        let reconciler = Reconciler::new(
            Arc::new(AppConfig::default()),
            metrics.clone(),
            position_manager,
            auto_heal,
        );
        (reconciler, metrics, sqlite_path, guard)
    }

    #[test]
    fn reconciler_create() {
        let pm = Arc::new(Mutex::new(PositionManager::new()));
        let _r = Reconciler::new(
            Arc::new(AppConfig::default()),
            Arc::new(Metrics::new()),
            pm,
            false,
        );
    }

    #[tokio::test]
    async fn light_reconciliation_no_issues() {
        let pm = Arc::new(Mutex::new(PositionManager::new()));
        let (r, _metrics, sqlite_path, _guard) = test_reconciler(pm);
        let result = r.run_light().await.unwrap();
        assert_eq!(result.checked, 0);
        assert!(!result.has_issues());
        let _ = std::fs::remove_file(sqlite_path);
    }

    #[tokio::test]
    async fn full_reconciliation_no_issues() {
        let pm = Arc::new(Mutex::new(PositionManager::new()));
        let (r, _metrics, sqlite_path, _guard) = test_reconciler(pm);
        let result = r.run_full().await.unwrap();
        assert_eq!(result.checked, 0);
        assert!(!result.has_issues());
        let _ = std::fs::remove_file(sqlite_path);
    }

    #[tokio::test]
    async fn light_reconciliation_detects_missing_ghost_and_mismatch_positions() {
        let pm = Arc::new(Mutex::new(PositionManager::new()));
        {
            let mut manager = pm.lock().await;
            manager.restore_positions(vec![
                test_position("market-ghost", Side::Yes, dec!(10)),
                test_position("market-mismatch", Side::No, dec!(10)),
            ]);
        }

        let (r, _metrics, sqlite_path, _guard) = test_reconciler(pm.clone());
        let store = crate::state::sqlite::SqliteStore::open(&sqlite_path).unwrap();
        store
            .upsert_position(
                &test_position("market-missing", Side::Yes, dec!(12)),
                Some(dec!(0.60)),
                Some(dec!(1)),
                Some("0xsource"),
            )
            .unwrap();
        store
            .upsert_position(
                &test_position("market-mismatch", Side::No, dec!(15)),
                Some(dec!(0.62)),
                Some(dec!(2)),
                Some("0xsource"),
            )
            .unwrap();

        let result = r.run_light().await.unwrap();

        assert_eq!(result.checked, 3);
        assert_eq!(result.ghost_positions, vec!["market-ghost:YES"]);
        assert_eq!(result.missing_positions, vec!["market-missing:YES"]);
        assert_eq!(result.mismatches, vec!["market-mismatch:NO"]);

        let _ = std::fs::remove_file(sqlite_path);
    }

    #[tokio::test]
    async fn full_reconciliation_restores_positions_from_reference_snapshot() {
        let pm = Arc::new(Mutex::new(PositionManager::new()));
        {
            let mut manager = pm.lock().await;
            manager.restore_positions(vec![test_position("market-local", Side::Yes, dec!(10))]);
        }

        let (r, metrics, sqlite_path, _guard) = test_reconciler_with_auto_heal(pm.clone(), true);
        let store = crate::state::sqlite::SqliteStore::open(&sqlite_path).unwrap();
        store
            .upsert_position(
                &test_position("market-remote", Side::No, dec!(25)),
                Some(dec!(0.70)),
                Some(dec!(3)),
                Some("0xsource"),
            )
            .unwrap();

        let result = r.run_full().await.unwrap();
        assert!(result.has_issues());

        let manager = pm.lock().await;
        assert!(manager
            .get_position(&PositionKey::new("market-local", Side::Yes))
            .is_none());
        let restored = manager
            .get_position(&PositionKey::new("market-remote", Side::No))
            .unwrap();
        assert_eq!(restored.current_size, dec!(25));
        drop(manager);

        assert_eq!(
            metrics
                .open_positions
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert!(store
            .get_config("last_reconciliation_at")
            .unwrap()
            .is_some());

        let _ = std::fs::remove_file(sqlite_path);
    }

    #[tokio::test]
    async fn remote_full_reconciliation_replaces_local_only_positions() {
        let pm = Arc::new(Mutex::new(PositionManager::new()));
        let (r, metrics, sqlite_path, _guard) = test_reconciler_with_auto_heal(pm.clone(), true);

        let local_positions = vec![PositionSnapshot::from_position(&test_position(
            "market-local",
            Side::Yes,
            dec!(10),
        ))];
        let remote_positions = vec![PositionSnapshot::from_persisted_position(
            crate::state::sqlite::PersistedPositionRow {
                position: test_position("market-remote", Side::No, dec!(25)),
                current_price: Some(dec!(0.70)),
                unrealized_pnl: Some(dec!(3)),
                owned_by_wallet: Some("0xsource".to_string()),
                last_updated: chrono::Utc::now().to_rfc3339(),
            },
        )];

        r.sync_to_reference(
            &local_positions,
            &ReferenceSnapshot {
                source: SnapshotSource::RemoteDataApi,
                positions: remote_positions,
            },
        )
        .await
        .unwrap();

        let manager = pm.lock().await;
        assert!(manager
            .get_position(&PositionKey::new("market-local", Side::Yes))
            .is_none());
        assert!(manager
            .get_position(&PositionKey::new("market-remote", Side::No))
            .is_some());
        drop(manager);

        assert_eq!(
            metrics
                .open_positions
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        let _ = std::fs::remove_file(sqlite_path);
    }

    #[test]
    fn reconciliation_result_has_issues() {
        let result = ReconciliationResult {
            checked: 10,
            ghost_positions: vec!["m1".to_string()],
            missing_positions: vec![],
            mismatches: vec![],
        };
        assert!(result.has_issues());
    }

    #[test]
    fn reconciliation_result_no_issues() {
        let result = ReconciliationResult {
            checked: 10,
            ghost_positions: vec![],
            missing_positions: vec![],
            mismatches: vec![],
        };
        assert!(!result.has_issues());
    }

    #[test]
    fn size_tolerance_allows_small_drift() {
        assert!(sizes_close(dec!(100.0), dec!(100.05)));
        assert!(sizes_close(dec!(100.0), dec!(99.95)));
    }

    #[test]
    fn size_tolerance_catches_real_drift() {
        assert!(!sizes_close(dec!(100.0), dec!(100.5)));
    }

    #[test]
    fn price_tolerance_allows_tick_drift() {
        assert!(prices_close(dec!(0.50), dec!(0.505)));
    }

    #[test]
    fn price_tolerance_catches_real_drift() {
        assert!(!prices_close(dec!(0.50), dec!(0.52)));
    }

    #[tokio::test]
    async fn auto_heal_false_does_not_mutate_positions() {
        let pm = Arc::new(Mutex::new(PositionManager::new()));
        {
            let mut manager = pm.lock().await;
            manager.restore_positions(vec![test_position("market-local", Side::Yes, dec!(10))]);
        }

        let (r, metrics, sqlite_path, _guard) = test_reconciler_with_auto_heal(pm.clone(), false);
        let store = crate::state::sqlite::SqliteStore::open(&sqlite_path).unwrap();
        store
            .upsert_position(
                &test_position("market-remote", Side::No, dec!(25)),
                Some(dec!(0.70)),
                Some(dec!(3)),
                Some("0xsource"),
            )
            .unwrap();

        let result = r.run_full().await.unwrap();
        assert!(result.has_issues());

        // In-memory state MUST be preserved when auto_heal is off.
        let manager = pm.lock().await;
        let local = manager
            .get_position(&PositionKey::new("market-local", Side::Yes))
            .expect("local position should survive log-only reconciliation");
        assert_eq!(local.current_size, dec!(10));
        assert!(manager
            .get_position(&PositionKey::new("market-remote", Side::No))
            .is_none());
        drop(manager);

        // Metrics untouched — no set_open_positions call in log-only path.
        assert_eq!(
            metrics
                .open_positions
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );

        let _ = std::fs::remove_file(sqlite_path);
    }

    #[tokio::test]
    async fn auto_heal_true_mutates_positions() {
        let pm = Arc::new(Mutex::new(PositionManager::new()));
        {
            let mut manager = pm.lock().await;
            manager.restore_positions(vec![test_position("market-local", Side::Yes, dec!(10))]);
        }

        let (r, _metrics, sqlite_path, _guard) = test_reconciler_with_auto_heal(pm.clone(), true);
        let store = crate::state::sqlite::SqliteStore::open(&sqlite_path).unwrap();
        store
            .upsert_position(
                &test_position("market-remote", Side::No, dec!(25)),
                Some(dec!(0.70)),
                Some(dec!(3)),
                Some("0xsource"),
            )
            .unwrap();

        let _ = r.run_full().await.unwrap();

        let manager = pm.lock().await;
        assert!(manager
            .get_position(&PositionKey::new("market-local", Side::Yes))
            .is_none());
        assert!(manager
            .get_position(&PositionKey::new("market-remote", Side::No))
            .is_some());

        let _ = std::fs::remove_file(sqlite_path);
    }

    #[test]
    fn reconciliation_result_produces_alert_message_when_issues_exist() {
        let result = ReconciliationResult {
            checked: 3,
            ghost_positions: vec!["m1".to_string()],
            missing_positions: vec!["m2".to_string()],
            mismatches: vec![],
        };

        let message = result.alert_message().unwrap();
        assert!(message.contains("ghost=1"));
        assert!(message.contains("missing=1"));
    }
}
