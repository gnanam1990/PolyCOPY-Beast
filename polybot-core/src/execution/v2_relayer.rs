use chrono::Utc;
use polybot_common::errors::PolybotError;
use polybot_common::types::{TransactionKind, TransactionRecord, TransactionState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayerSubmitRequest {
    pub signed_order: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayerPollRequest {
    #[serde(rename = "transactionID")]
    pub transaction_id: String,
}

impl RelayerPollRequest {
    pub fn new(transaction_id: impl Into<String>) -> Self {
        Self {
            transaction_id: transaction_id.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayerSubmitResponse {
    #[serde(rename = "transactionID")]
    pub transaction_id: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayerTransactionResponse {
    #[serde(rename = "transactionID")]
    pub transaction_id: String,
    pub state: String,
    #[serde(default)]
    pub transaction_hash: Option<String>,
    #[serde(default)]
    pub error_msg: Option<String>,
}

pub fn map_transaction_state(raw: &str) -> Result<TransactionState, PolybotError> {
    match raw {
        "STATE_NEW" => Ok(TransactionState::New),
        "STATE_PENDING" => Ok(TransactionState::Pending),
        "STATE_SUBMITTED" => Ok(TransactionState::Submitted),
        "STATE_SUCCESS" => Ok(TransactionState::Success),
        "STATE_FAILED" => Ok(TransactionState::Failed),
        other => Err(PolybotError::Execution(format!(
            "unknown relayer transaction state: {}",
            other
        ))),
    }
}

pub fn transaction_record_from_submit(
    response: &RelayerSubmitResponse,
    trade_id: Option<String>,
) -> Result<TransactionRecord, PolybotError> {
    Ok(TransactionRecord {
        transaction_id: response.transaction_id.clone(),
        trade_id,
        kind: TransactionKind::Order,
        state: map_transaction_state(&response.state)?,
        submitted_at: Utc::now(),
        confirmed_at: None,
        transaction_hash: None,
        error_msg: None,
    })
}

pub fn apply_poll_response(
    record: &mut TransactionRecord,
    response: &RelayerTransactionResponse,
) -> Result<(), PolybotError> {
    record.state = map_transaction_state(&response.state)?;
    record.transaction_hash = response.transaction_hash.clone();
    record.error_msg = response.error_msg.clone();
    if record.state.is_terminal() {
        record.confirmed_at = Some(Utc::now());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relayer_state_parses_terminal_and_non_terminal_values() {
        assert!(!map_transaction_state("STATE_NEW").unwrap().is_terminal());
        assert!(!map_transaction_state("STATE_PENDING")
            .unwrap()
            .is_terminal());
        assert!(map_transaction_state("STATE_SUCCESS")
            .unwrap()
            .is_terminal());
        assert!(map_transaction_state("STATE_FAILED").unwrap().is_terminal());
    }

    #[test]
    fn submit_response_creates_transaction_record() {
        let response = RelayerSubmitResponse {
            transaction_id: "txn_abc".to_string(),
            state: "STATE_NEW".to_string(),
        };
        let record =
            transaction_record_from_submit(&response, Some("trade-1".to_string())).unwrap();
        assert_eq!(record.transaction_id, "txn_abc");
        assert_eq!(record.trade_id.as_deref(), Some("trade-1"));
        assert_eq!(record.state, TransactionState::New);
    }

    #[test]
    fn poll_request_serializes_transaction_id_field() {
        let request = RelayerPollRequest::new("txn_abc");
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value, serde_json::json!({ "transactionID": "txn_abc" }));
    }

    #[test]
    fn poll_response_marks_terminal_confirmation() {
        let response = RelayerSubmitResponse {
            transaction_id: "txn_abc".to_string(),
            state: "STATE_NEW".to_string(),
        };
        let mut record =
            transaction_record_from_submit(&response, Some("trade-1".to_string())).unwrap();
        let poll = RelayerTransactionResponse {
            transaction_id: "txn_abc".to_string(),
            state: "STATE_SUCCESS".to_string(),
            transaction_hash: Some("0xabc".to_string()),
            error_msg: None,
        };
        apply_poll_response(&mut record, &poll).unwrap();
        assert_eq!(record.state, TransactionState::Success);
        assert_eq!(record.transaction_hash.as_deref(), Some("0xabc"));
        assert!(record.confirmed_at.is_some());
    }
}
