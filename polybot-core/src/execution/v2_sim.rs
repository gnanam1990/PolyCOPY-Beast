use polybot_common::errors::PolybotError;
use polybot_common::types::TransactionState;

use super::v2_relayer::{RelayerSubmitResponse, RelayerTransactionResponse};

#[derive(Debug, Clone)]
pub struct SimulatedRelayer {
    transaction_id: String,
    states: Vec<TransactionState>,
    poll_index: usize,
}

impl SimulatedRelayer {
    pub fn success(transaction_id: impl Into<String>) -> Self {
        Self::scripted(
            transaction_id,
            vec![
                TransactionState::New,
                TransactionState::Pending,
                TransactionState::Submitted,
                TransactionState::Success,
            ],
        )
    }

    pub fn failed(transaction_id: impl Into<String>) -> Self {
        Self::scripted(
            transaction_id,
            vec![
                TransactionState::New,
                TransactionState::Pending,
                TransactionState::Submitted,
                TransactionState::Failed,
            ],
        )
    }

    pub fn scripted(transaction_id: impl Into<String>, states: Vec<TransactionState>) -> Self {
        Self {
            transaction_id: transaction_id.into(),
            states,
            poll_index: 0,
        }
    }

    pub fn submit(&self) -> Result<RelayerSubmitResponse, PolybotError> {
        let Some(first) = self.states.first() else {
            return Err(PolybotError::Execution(
                "simulated relayer requires at least one state".to_string(),
            ));
        };
        Ok(RelayerSubmitResponse {
            transaction_id: self.transaction_id.clone(),
            state: first.as_sqlite_str().to_string(),
        })
    }

    pub fn poll(&mut self) -> Result<RelayerTransactionResponse, PolybotError> {
        if self.states.is_empty() {
            return Err(PolybotError::Execution(
                "simulated relayer requires at least one state".to_string(),
            ));
        }
        self.poll_index = (self.poll_index + 1).min(self.states.len() - 1);
        let state = self.states[self.poll_index];
        Ok(RelayerTransactionResponse {
            transaction_id: self.transaction_id.clone(),
            state: state.as_sqlite_str().to_string(),
            transaction_hash: state
                .is_terminal()
                .then(|| format!("0xsim{}", self.transaction_id.replace('-', ""))),
            error_msg: (state == TransactionState::Failed)
                .then(|| "simulated relayer failure".to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulated_relayer_advances_to_success() {
        let mut relayer = SimulatedRelayer::success("sim-txn-1");
        let submit = relayer.submit().unwrap();
        assert_eq!(submit.state, "STATE_NEW");

        assert_eq!(relayer.poll().unwrap().state, "STATE_PENDING");
        assert_eq!(relayer.poll().unwrap().state, "STATE_SUBMITTED");
        let final_state = relayer.poll().unwrap();
        assert_eq!(final_state.state, "STATE_SUCCESS");
        assert!(final_state.transaction_hash.is_some());
    }

    #[test]
    fn simulated_relayer_can_fail() {
        let mut relayer = SimulatedRelayer::failed("sim-txn-2");
        let _ = relayer.submit().unwrap();
        let _ = relayer.poll().unwrap();
        let _ = relayer.poll().unwrap();
        let final_state = relayer.poll().unwrap();
        assert_eq!(final_state.state, "STATE_FAILED");
        assert!(final_state.transaction_hash.is_some());
        assert_eq!(
            final_state.error_msg.as_deref(),
            Some("simulated relayer failure")
        );
    }
}
