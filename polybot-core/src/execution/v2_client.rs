use polybot_common::errors::PolybotError;
use polybot_common::types::{OrderType, TransactionState};

use crate::config::RelayerConfig;

#[derive(Debug, Clone)]
pub struct RelayerClient {
    base_url: String,
    api_key: String,
    api_key_address: String,
    http_client: reqwest::Client,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RelayerSubmitRequest {
    pub order: serde_json::Value,
    pub owner: String,
    #[serde(rename = "orderType")]
    pub order_type: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RelayerSubmitResponse {
    #[serde(rename = "transactionID")]
    pub transaction_id: String,
    pub state: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RelayerTransactionResponse {
    #[serde(rename = "transactionID")]
    pub transaction_id: String,
    pub state: String,
    #[serde(default)]
    pub transaction_hash: Option<String>,
    #[serde(default)]
    pub error_msg: Option<String>,
}

fn order_type_to_wire(order_type: OrderType) -> &'static str {
    match order_type {
        OrderType::Limit => "GTC",
        OrderType::Ioc => "IOC",
        OrderType::Fok => "FOK",
        OrderType::PostOnly => "GTD",
    }
}

impl RelayerClient {
    pub fn new(config: &RelayerConfig) -> Result<Self, PolybotError> {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| {
                PolybotError::Execution(format!("Failed to create relayer HTTP client: {}", e))
            })?;

        Ok(Self {
            base_url: config.url.trim_end_matches('/').to_string(),
            api_key: config.api_key.clone(),
            api_key_address: config.api_key_address.clone(),
            http_client,
        })
    }

    fn with_auth_headers(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request
            .header("RELAYER_API_KEY", &self.api_key)
            .header("RELAYER_API_KEY_ADDRESS", &self.api_key_address)
    }

    pub async fn submit_order(
        &self,
        signed_order: serde_json::Value,
        order_type: OrderType,
    ) -> Result<RelayerSubmitResponse, PolybotError> {
        let response = self
            .with_auth_headers(self.http_client.post(format!("{}/order", self.base_url)))
            .json(&RelayerSubmitRequest {
                order: signed_order,
                owner: self.api_key.clone(),
                order_type: order_type_to_wire(order_type).to_string(),
            })
            .send()
            .await
            .map_err(|e| {
                PolybotError::Execution(format!("Relayer submit request failed: {}", e))
            })?;

        if !response.status().is_success() {
            return Err(PolybotError::Execution(format!(
                "Relayer submit failed with HTTP {}",
                response.status()
            )));
        }

        response.json::<RelayerSubmitResponse>().await.map_err(|e| {
            PolybotError::Execution(format!("Failed to parse relayer submit response: {}", e))
        })
    }

    pub async fn get_transaction(
        &self,
        transaction_id: &str,
    ) -> Result<RelayerTransactionResponse, PolybotError> {
        let response = self
            .with_auth_headers(
                self.http_client
                    .get(format!("{}/transaction", self.base_url)),
            )
            .query(&[("transactionID", transaction_id)])
            .send()
            .await
            .map_err(|e| {
                PolybotError::Execution(format!("Relayer transaction poll failed: {}", e))
            })?;

        if !response.status().is_success() {
            return Err(PolybotError::Execution(format!(
                "Relayer transaction poll failed with HTTP {}",
                response.status()
            )));
        }

        response
            .json::<RelayerTransactionResponse>()
            .await
            .map_err(|e| {
                PolybotError::Execution(format!(
                    "Failed to parse relayer transaction response: {}",
                    e
                ))
            })
    }

    pub async fn poll_transaction_until_terminal(
        &self,
        transaction_id: &str,
        max_attempts: usize,
        delay: std::time::Duration,
    ) -> Result<RelayerTransactionResponse, PolybotError> {
        for attempt in 0..max_attempts {
            let response = self.get_transaction(transaction_id).await?;
            let state = map_transaction_state(&response.state)?;
            if state.is_terminal() {
                return Ok(response);
            }
            if attempt + 1 < max_attempts {
                tokio::time::sleep(delay).await;
            }
        }

        Err(PolybotError::Execution(format!(
            "Relayer transaction {} did not reach a terminal state after {} attempts",
            transaction_id, max_attempts
        )))
    }
}

pub fn map_transaction_state(raw: &str) -> Result<TransactionState, PolybotError> {
    match raw {
        "STATE_NEW" => Ok(TransactionState::New),
        "STATE_PENDING" => Ok(TransactionState::Pending),
        "STATE_SUBMITTED" => Ok(TransactionState::Submitted),
        "STATE_EXECUTED" => Ok(TransactionState::Executed),
        "STATE_MINED" => Ok(TransactionState::Mined),
        "STATE_SUCCESS" => Ok(TransactionState::Success),
        "STATE_CONFIRMED" => Ok(TransactionState::Confirmed),
        "STATE_FAILED" => Ok(TransactionState::Failed),
        "STATE_INVALID" => Ok(TransactionState::Invalid),
        other => Err(PolybotError::Execution(format!(
            "Unknown relayer state: {}",
            other
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::{Query, State},
        routing::{get, post},
        Json, Router,
    };
    use serde::Deserialize;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Default)]
    struct CaptureState {
        api_key: Mutex<Option<String>>,
        api_key_address: Mutex<Option<String>>,
        submit_body: Mutex<Option<serde_json::Value>>,
        transaction_id: Mutex<Option<String>>,
        poll_count: Mutex<u32>,
    }

    #[derive(Debug, Deserialize)]
    struct TxnQuery {
        #[serde(rename = "transactionID")]
        transaction_id: String,
    }

    async fn submit_handler(
        State(state): State<Arc<CaptureState>>,
        headers: axum::http::HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        *state.api_key.lock().unwrap() = headers
            .get("RELAYER_API_KEY")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_string());
        *state.api_key_address.lock().unwrap() = headers
            .get("RELAYER_API_KEY_ADDRESS")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_string());
        *state.submit_body.lock().unwrap() = Some(body);
        Json(serde_json::json!({
            "transactionID": "txn_submit_1",
            "state": "STATE_NEW"
        }))
    }

    async fn transaction_handler(
        State(state): State<Arc<CaptureState>>,
        Query(query): Query<TxnQuery>,
        headers: axum::http::HeaderMap,
    ) -> Json<serde_json::Value> {
        *state.api_key.lock().unwrap() = headers
            .get("RELAYER_API_KEY")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_string());
        *state.api_key_address.lock().unwrap() = headers
            .get("RELAYER_API_KEY_ADDRESS")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_string());
        *state.transaction_id.lock().unwrap() = Some(query.transaction_id.clone());
        let mut poll_count = state.poll_count.lock().unwrap();
        *poll_count += 1;
        let state_value = if *poll_count >= 2 {
            "STATE_SUCCESS"
        } else {
            "STATE_PENDING"
        };
        Json(serde_json::json!({
            "transactionID": query.transaction_id,
            "state": state_value,
            "transaction_hash": "0xdeadbeef"
        }))
    }

    async fn spawn_relayer_server() -> (RelayerClient, Arc<CaptureState>) {
        let state = Arc::new(CaptureState::default());
        let app = Router::new()
            .route("/order", post(submit_handler))
            .route("/transaction", get(transaction_handler))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = RelayerClient::new(&RelayerConfig {
            url: format!("http://{}", addr),
            api_key: "test-relayer-key".to_string(),
            api_key_address: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
        })
        .unwrap();

        (client, state)
    }

    #[test]
    fn relayer_state_parses_terminal_and_non_terminal_values() {
        assert!(!map_transaction_state("STATE_NEW").unwrap().is_terminal());
        assert!(!map_transaction_state("STATE_PENDING")
            .unwrap()
            .is_terminal());
        assert!(map_transaction_state("STATE_SUCCESS")
            .unwrap()
            .is_terminal());
        assert!(map_transaction_state("STATE_CONFIRMED")
            .unwrap()
            .is_terminal());
        assert!(map_transaction_state("STATE_INVALID")
            .unwrap()
            .is_terminal());
        assert!(map_transaction_state("STATE_FAILED").unwrap().is_terminal());
    }

    #[test]
    fn relayer_state_rejects_unknown_values() {
        let err = map_transaction_state("STATE_WEIRD").unwrap_err();
        match err {
            PolybotError::Execution(message) => assert!(message.contains("Unknown relayer state")),
            other => panic!("expected execution error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn relayer_submit_sends_headers_and_parses_response() {
        let (client, state) = spawn_relayer_server().await;

        let response = client
            .submit_order(serde_json::json!({"signed": "order"}), OrderType::Fok)
            .await
            .unwrap();

        assert_eq!(response.transaction_id, "txn_submit_1");
        assert_eq!(response.state, "STATE_NEW");
        assert_eq!(
            *state.submit_body.lock().unwrap(),
            Some(serde_json::json!({
                "order": {"signed": "order"},
                "owner": "test-relayer-key",
                "orderType": "FOK",
            }))
        );
        assert_eq!(
            state.api_key.lock().unwrap().as_deref(),
            Some("test-relayer-key")
        );
        assert_eq!(
            state.api_key_address.lock().unwrap().as_deref(),
            Some("0xabc123abc123abc123abc123abc123abc123abc1")
        );
    }

    #[tokio::test]
    async fn relayer_get_transaction_sends_headers_and_query() {
        let (client, state) = spawn_relayer_server().await;

        let response = client.get_transaction("txn_abc123").await.unwrap();

        assert_eq!(response.transaction_id, "txn_abc123");
        assert_eq!(response.state, "STATE_PENDING");
        assert_eq!(response.transaction_hash.as_deref(), Some("0xdeadbeef"));
        assert_eq!(
            state.transaction_id.lock().unwrap().as_deref(),
            Some("txn_abc123")
        );
        assert_eq!(
            state.api_key.lock().unwrap().as_deref(),
            Some("test-relayer-key")
        );
    }

    #[tokio::test]
    async fn poll_transaction_until_terminal_waits_for_success() {
        let (client, state) = spawn_relayer_server().await;

        let response = client
            .poll_transaction_until_terminal("txn_poll_1", 3, std::time::Duration::from_millis(1))
            .await
            .unwrap();

        assert_eq!(response.transaction_id, "txn_poll_1");
        assert_eq!(response.state, "STATE_SUCCESS");
        assert_eq!(*state.poll_count.lock().unwrap(), 2);
    }
}
