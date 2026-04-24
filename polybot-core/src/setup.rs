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
            _ => format!(
                "mode={:?} simulation_preflight=true",
                self.execution_mode
            ),
        }
    }
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
mod tests {}
