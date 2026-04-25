use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use polybot_common::types::ExecutionMode;
use rust_decimal::Decimal;

/// v2.5: Two-step confirmation for destructive commands.
/// After a destructive command, the user must send /confirm within 60 seconds.
pub struct ConfirmState {
    pending: Mutex<HashMap<u64, PendingConfirm>>,
}

struct PendingConfirm {
    action: ConfirmAction,
    created_at: Instant,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmAction {
    EmergencyStop,
    WalletRemove(String),
    ResumeAfterLoss,
    ModeSwitch(ExecutionMode),
    WrapCollateral {
        amount: Decimal,
    },
    RedeemPositions {
        condition_id: String,
        index_sets: Vec<u64>,
    },
}

const CONFIRM_TIMEOUT: Duration = Duration::from_secs(60);

impl ConfirmState {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Register a pending confirmation for a user. Returns true if this is a new request.
    pub fn register(&self, user_id: u64, action: ConfirmAction) -> bool {
        let mut pending = self.pending.lock().unwrap();
        // Clean expired entries
        pending.retain(|_, v| v.created_at.elapsed() < CONFIRM_TIMEOUT);
        pending.insert(
            user_id,
            PendingConfirm {
                action,
                created_at: Instant::now(),
            },
        );
        true
    }

    /// Check if a /confirm from this user is valid. Returns the action if confirmed.
    pub fn confirm(&self, user_id: u64) -> Option<ConfirmAction> {
        let mut pending = self.pending.lock().unwrap();
        if let Some(entry) = pending.remove(&user_id) {
            if entry.created_at.elapsed() < CONFIRM_TIMEOUT {
                return Some(entry.action);
            }
        }
        None
    }

    /// Check if a user has a pending confirmation
    pub fn has_pending(&self, user_id: u64) -> bool {
        let pending = self.pending.lock().unwrap();
        if let Some(entry) = pending.get(&user_id) {
            entry.created_at.elapsed() < CONFIRM_TIMEOUT
        } else {
            false
        }
    }

    /// Cancel a pending confirmation
    pub fn cancel(&self, user_id: u64) {
        let mut pending = self.pending.lock().unwrap();
        pending.remove(&user_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_confirm() {
        let state = ConfirmState::new();
        state.register(123, ConfirmAction::EmergencyStop);
        let action = state.confirm(123);
        assert_eq!(action, Some(ConfirmAction::EmergencyStop));
        // Confirm can't be used twice
        let action2 = state.confirm(123);
        assert_eq!(action2, None);
    }

    #[test]
    fn confirm_without_register_returns_none() {
        let state = ConfirmState::new();
        assert!(state.confirm(123).is_none());
    }

    #[test]
    fn cancel_removes_pending() {
        let state = ConfirmState::new();
        state.register(123, ConfirmAction::EmergencyStop);
        state.cancel(123);
        assert!(state.confirm(123).is_none());
    }

    #[test]
    fn has_pending_works() {
        let state = ConfirmState::new();
        assert!(!state.has_pending(123));
        state.register(123, ConfirmAction::EmergencyStop);
        assert!(state.has_pending(123));
    }

    #[test]
    fn confirm_timeout_is_one_minute() {
        assert_eq!(CONFIRM_TIMEOUT, Duration::from_secs(60));
    }
}
