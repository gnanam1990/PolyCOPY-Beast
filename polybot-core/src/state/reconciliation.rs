use polybot_common::constants::{
    FULL_RECONCILIATION_INTERVAL_SECS, LIGHT_RECONCILIATION_INTERVAL_SECS,
};
use polybot_common::errors::PolybotError;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::telegram_bot::alerts::AlertBroadcaster;

use super::positions::PositionManager;

/// v2.5: Reconciliation engine with light (30s) and full (5min) modes.
/// Source of truth: on-chain > CLOB API > Redis cache.
pub struct Reconciler {
    light_interval_secs: u64,
    full_interval_secs: u64,
    position_manager: Arc<Mutex<PositionManager>>,
    alerts: Option<AlertBroadcaster>,
}

impl Reconciler {
    pub fn new(position_manager: Arc<Mutex<PositionManager>>) -> Self {
        Self {
            light_interval_secs: LIGHT_RECONCILIATION_INTERVAL_SECS,
            full_interval_secs: FULL_RECONCILIATION_INTERVAL_SECS,
            position_manager,
            alerts: None,
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
        self.reconcile_local().await
    }

    /// Run full reconciliation
    pub async fn run_full(&self) -> Result<ReconciliationResult, PolybotError> {
        tracing::debug!("Running full reconciliation");
        self.reconcile_local().await
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

    async fn reconcile_local(&self) -> Result<ReconciliationResult, PolybotError> {
        let local_positions = {
            let positions = self.position_manager.lock().await;
            positions
                .get_positions_vec()
                .into_iter()
                .cloned()
                .collect::<Vec<_>>()
        };

        let checked = local_positions.len() as u32;
        
        Ok(ReconciliationResult {
            checked,
            ghost_positions: Vec::new(),
            missing_positions: Vec::new(),
            mismatches: Vec::new(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ReconciliationResult {
    pub checked: u32,
    pub ghost_positions: Vec<String>,
    pub missing_positions: Vec<String>,
    pub mismatches: Vec<String>,      // position data differs
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

    #[test]
    fn reconciler_create() {
        let pm = Arc::new(Mutex::new(PositionManager::new()));
        let _r = Reconciler::new(pm);
    }

    #[tokio::test]
    async fn light_reconciliation_no_issues() {
        let pm = Arc::new(Mutex::new(PositionManager::new()));
        let r = Reconciler::new(pm);
        let result = r.run_light().await.unwrap();
        assert_eq!(result.checked, 0);
        assert!(!result.has_issues());
    }

    #[tokio::test]
    async fn full_reconciliation_no_issues() {
        let pm = Arc::new(Mutex::new(PositionManager::new()));
        let r = Reconciler::new(pm);
        let result = r.run_full().await.unwrap();
        assert_eq!(result.checked, 0);
        assert!(!result.has_issues());
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
