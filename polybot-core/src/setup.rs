use polybot_common::errors::PolybotError;
use polybot_common::types::ExecutionMode;
use serde::Deserialize;

use crate::config::AppConfig;
use crate::execution::clob_client::{ClobClient, WalletMode};

const POLYGON_MAINNET_CHAIN_ID: u64 = 137;

#[derive(Debug, Clone)]
pub struct StartupPreflightReport {
    pub execution_mode: ExecutionMode,
    pub verified_rpc_endpoint: String,
    pub wallet_mode: Option<WalletMode>,
    pub approvals_ready: Option<bool>,
}

impl StartupPreflightReport {
    pub fn summary(&self) -> String {
        match (self.wallet_mode, self.approvals_ready) {
            (Some(wallet_mode), Some(approvals_ready)) => format!(
                "mode={:?} rpc={} wallet_mode={} approvals_ready={}",
                self.execution_mode, self.verified_rpc_endpoint, wallet_mode, approvals_ready
            ),
            _ => format!(
                "mode={:?} rpc={} simulation_preflight=true",
                self.execution_mode, self.verified_rpc_endpoint
            ),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RpcChainIdResponse {
    result: Option<String>,
}

pub async fn run_startup_preflight(
    config: &AppConfig,
) -> Result<StartupPreflightReport, PolybotError> {
    if matches!(config.system.execution_mode, ExecutionMode::Simulation) {
        return Ok(StartupPreflightReport {
            execution_mode: config.system.execution_mode,
            verified_rpc_endpoint: config
                .execution
                .rpc_endpoints
                .first()
                .cloned()
                .unwrap_or_else(|| "simulation-rpc-skipped".to_string()),
            wallet_mode: None,
            approvals_ready: None,
        });
    }

    let verified_rpc_endpoint = validate_rpc_connectivity(&config.execution.rpc_endpoints).await?;

    let mut report = StartupPreflightReport {
        execution_mode: config.system.execution_mode,
        verified_rpc_endpoint,
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

async fn validate_rpc_connectivity(endpoints: &[String]) -> Result<String, PolybotError> {
    if endpoints.is_empty() {
        return Err(PolybotError::Config("No RPC endpoints configured".to_string()));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| PolybotError::Config(format!("Failed to create RPC validation client: {}", e)))?;
    let mut last_error = None;

    for endpoint in endpoints {
        match validate_rpc_endpoint(&client, endpoint).await {
            Ok(()) => return Ok(endpoint.clone()),
            Err(error) => {
                tracing::warn!(endpoint = %endpoint, error = %error, "RPC endpoint failed startup validation");
                last_error = Some(format!("{} ({})", endpoint, error));
            }
        }
    }

    Err(PolybotError::Config(format!(
        "No RPC endpoint passed Polygon mainnet validation: {}",
        last_error.unwrap_or_else(|| "unknown validation error".to_string())
    )))
}

async fn validate_rpc_endpoint(
    client: &reqwest::Client,
    endpoint: &str,
) -> Result<(), PolybotError> {
    let response = client
        .post(endpoint)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_chainId",
            "params": [],
        }))
        .send()
        .await
        .map_err(|e| PolybotError::Config(format!("RPC request failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(PolybotError::Config(format!(
            "RPC endpoint returned HTTP {}",
            response.status()
        )));
    }

    let body: RpcChainIdResponse = response
        .json()
        .await
        .map_err(|e| PolybotError::Config(format!("Invalid RPC chain-id response: {}", e)))?;
    let raw_chain_id = body
        .result
        .ok_or_else(|| PolybotError::Config("RPC response did not include result".to_string()))?;
    let chain_id = parse_chain_id_hex(&raw_chain_id)?;
    if chain_id != POLYGON_MAINNET_CHAIN_ID {
        return Err(PolybotError::Config(format!(
            "RPC endpoint is on chain {} instead of {}",
            chain_id, POLYGON_MAINNET_CHAIN_ID
        )));
    }

    Ok(())
}


fn parse_chain_id_hex(value: &str) -> Result<u64, PolybotError> {
    let trimmed = value.trim();
    let raw = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .ok_or_else(|| PolybotError::Config(format!("Invalid hex chain id: {}", value)))?;

    u64::from_str_radix(raw, 16)
        .map_err(|e| PolybotError::Config(format!("Invalid hex chain id {}: {}", value, e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, routing::post, Router};
    use serde_json::json;

    #[test]
    fn parse_chain_id_hex_accepts_polygon() {
        assert_eq!(parse_chain_id_hex("0x89").unwrap(), 137);
    }

    #[test]
    fn parse_chain_id_hex_rejects_invalid_values() {
        assert!(parse_chain_id_hex("137").is_err());
        assert!(parse_chain_id_hex("0xzz").is_err());
    }

    async fn spawn_rpc_server(chain_id: &'static str) -> String {
        let app = Router::new().route(
            "/",
            post(move || async move { Json(json!({"jsonrpc": "2.0", "id": 1, "result": chain_id})) }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn validate_rpc_connectivity_uses_first_polygon_mainnet_endpoint() {
        let wrong_chain = spawn_rpc_server("0x1").await;
        let polygon = spawn_rpc_server("0x89").await;

        let selected = validate_rpc_connectivity(&[wrong_chain, polygon.clone()])
            .await
            .unwrap();

        assert_eq!(selected, polygon);
    }

    #[tokio::test]
    async fn simulation_preflight_skips_rpc_validation() {
        let mut config = AppConfig::default();
        config.execution.rpc_endpoints.clear();

        let report = run_startup_preflight(&config).await.unwrap();

        assert!(report.verified_rpc_endpoint.contains("simulation"));
        assert!(report.wallet_mode.is_none());
        assert!(report.approvals_ready.is_none());
    }
}
