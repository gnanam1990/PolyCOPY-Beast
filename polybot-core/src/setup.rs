use polybot_common::errors::PolybotError;
use polybot_common::types::ExecutionMode;

use crate::config::AppConfig;
use crate::execution::clob_client::{ClobClient, WalletMode};

#[derive(Debug, Clone)]
pub struct StartupPreflightReport {
    pub execution_mode: ExecutionMode,
    pub wallet_mode: Option<WalletMode>,
    pub approvals_ready: Option<bool>,
}

impl StartupPreflightReport {
    pub fn summary(&self) -> String {
        match (self.wallet_mode, self.approvals_ready) {
            (Some(wallet_mode), Some(approvals_ready)) => format!(
                "mode={:?} wallet_mode={} approvals_ready={}",
                self.execution_mode, wallet_mode, approvals_ready
            ),
            _ => format!("mode={:?} simulation_preflight=true", self.execution_mode),
        }
    }
}

fn live_v2_enabled() -> bool {
    std::env::var("POLYBOT_ENABLE_LIVE_V2")
        .map(|value| value == "true" || value == "1")
        .unwrap_or(false)
}

fn dashboard_control_auth_configured() -> bool {
    std::env::var("POLYBOT_DASHBOARD_CONTROL_KEY")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn v2_verification_condition_id() -> Option<String> {
    std::env::var("POLYBOT_V2_VERIFY_CONDITION_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn clob_endpoint() -> String {
    std::env::var("POLYBOT_CLOB_ENDPOINT")
        .or_else(|_| std::env::var("CLOB_API_URL"))
        .unwrap_or_else(|_| "https://clob.polymarket.com".to_string())
}

pub async fn run_startup_preflight(
    config: &AppConfig,
) -> Result<StartupPreflightReport, PolybotError> {
    if matches!(config.system.execution_mode, ExecutionMode::Simulation) {
        return Ok(StartupPreflightReport {
            execution_mode: config.system.execution_mode,
            wallet_mode: None,
            approvals_ready: None,
        });
    }

    if matches!(config.system.execution_mode, ExecutionMode::Live) && !live_v2_enabled() {
        return Err(PolybotError::Config(
            "Live CLOB V2 submission is disabled for the simulation-complete milestone. Set POLYBOT_ENABLE_LIVE_V2=true only after V2 endpoint verification, pUSD wrap/approve/redeem support, dashboard control auth, and full workspace tests are green.".to_string(),
        ));
    }

    if matches!(config.system.execution_mode, ExecutionMode::Live)
        && !dashboard_control_auth_configured()
    {
        return Err(PolybotError::Config(
            "Live mode requires POLYBOT_DASHBOARD_CONTROL_KEY so dashboard pause/resume/emergency-stop routes are authenticated.".to_string(),
        ));
    }

    if matches!(config.system.execution_mode, ExecutionMode::Live) {
        crate::execution::live_v2_submission_config(config)?;
    }

    let condition_id = v2_verification_condition_id().ok_or_else(|| {
        PolybotError::Config(
            "Live mode requires POLYBOT_V2_VERIFY_CONDITION_ID so setup-check can verify the real V2 market/fee response shape.".to_string(),
        )
    })?;
    let endpoint = clob_endpoint();
    let verified_market =
        crate::execution::v2_market::fetch_and_verify_v2_market_info(&endpoint, &condition_id)
            .await?;
    tracing::info!(
        condition_id = %verified_market.condition_id,
        token_count = verified_market.token_count,
        "Verified V2 market info response shape"
    );

    let mut report = StartupPreflightReport {
        execution_mode: config.system.execution_mode,
        wallet_mode: None,
        approvals_ready: None,
    };

    let client = ClobClient::from_env()?;
    let wallet_mode = client.validate_wallet_mode()?;
    let _credentials = client.authenticate().await?;
    let approvals = client.check_approvals().await?;

    report.wallet_mode = Some(wallet_mode);
    report.approvals_ready = Some(approvals.ready_for_live_trading);

    if !approvals.ready_for_live_trading {
        let auto_approve = std::env::var("POLYBOT_AUTO_APPROVE")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        if auto_approve {
            tracing::warn!("POLYBOT_AUTO_APPROVE enabled but on-chain approval transactions require manual signing.");
            tracing::warn!("Please ensure USDC and CTF conditional token approvals are set for the Polymarket exchange contracts.");
        }
        return Err(PolybotError::Config(approvals.guidance_message()));
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use serial_test::serial;

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvVarGuard {
        fn new(key: &'static str) -> Self {
            Self {
                key,
                original: std::env::var(key).ok(),
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match self.original.as_ref() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    #[serial]
    fn live_v2_gate_defaults_to_disabled() {
        let _guard = EnvVarGuard::new("POLYBOT_ENABLE_LIVE_V2");
        std::env::remove_var("POLYBOT_ENABLE_LIVE_V2");
        assert!(!super::live_v2_enabled());
    }

    #[test]
    #[serial]
    fn live_v2_gate_accepts_true() {
        let _guard = EnvVarGuard::new("POLYBOT_ENABLE_LIVE_V2");
        std::env::set_var("POLYBOT_ENABLE_LIVE_V2", "true");
        assert!(super::live_v2_enabled());
    }

    #[test]
    #[serial]
    fn live_v2_gate_accepts_one() {
        let _guard = EnvVarGuard::new("POLYBOT_ENABLE_LIVE_V2");
        std::env::set_var("POLYBOT_ENABLE_LIVE_V2", "1");
        assert!(super::live_v2_enabled());
    }

    #[test]
    #[serial]
    fn dashboard_control_auth_requires_non_empty_key() {
        let _guard = EnvVarGuard::new("POLYBOT_DASHBOARD_CONTROL_KEY");
        std::env::remove_var("POLYBOT_DASHBOARD_CONTROL_KEY");
        assert!(!super::dashboard_control_auth_configured());

        std::env::set_var("POLYBOT_DASHBOARD_CONTROL_KEY", "   ");
        assert!(!super::dashboard_control_auth_configured());

        std::env::set_var("POLYBOT_DASHBOARD_CONTROL_KEY", "test-control-key");
        assert!(super::dashboard_control_auth_configured());
    }

    #[test]
    #[serial]
    fn v2_verification_condition_id_requires_non_empty_value() {
        let _guard = EnvVarGuard::new("POLYBOT_V2_VERIFY_CONDITION_ID");
        std::env::remove_var("POLYBOT_V2_VERIFY_CONDITION_ID");
        assert!(super::v2_verification_condition_id().is_none());

        std::env::set_var("POLYBOT_V2_VERIFY_CONDITION_ID", "   ");
        assert!(super::v2_verification_condition_id().is_none());

        std::env::set_var("POLYBOT_V2_VERIFY_CONDITION_ID", "0xabc");
        assert_eq!(
            super::v2_verification_condition_id().as_deref(),
            Some("0xabc")
        );
    }
}
