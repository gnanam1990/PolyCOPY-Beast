use chrono::Utc;
use polybot_common::errors::PolybotError;
use polybot_common::types::{Trade, TradeStatus, TransactionRecord, TransactionState};
use rust_decimal::Decimal;

use super::order_builder::Order;
use super::v2_relayer::{apply_poll_response, transaction_record_from_submit};
use super::v2_sim::SimulatedRelayer;

#[derive(Debug, Clone)]
pub struct V2ExecutionOutcome {
    pub trade: Trade,
    pub transaction: TransactionRecord,
}

pub fn simulate_v2_success(order: &Order) -> Result<V2ExecutionOutcome, PolybotError> {
    simulate_with_relayer(
        order,
        SimulatedRelayer::success(format!("sim-{}", order.signal_id)),
    )
}

pub fn simulate_with_relayer(
    order: &Order,
    mut relayer: SimulatedRelayer,
) -> Result<V2ExecutionOutcome, PolybotError> {
    let submit = relayer.submit()?;
    let mut transaction = transaction_record_from_submit(&submit, None)?;

    for _ in 0..32 {
        if transaction.state.is_terminal() {
            break;
        }

        let poll = relayer.poll()?;
        apply_poll_response(&mut transaction, &poll)?;
    }

    if !transaction.state.is_terminal() {
        return Err(PolybotError::Execution(format!(
            "simulated relayer did not reach a terminal state for {}",
            transaction.transaction_id
        )));
    }

    let trade_status = match transaction.state {
        TransactionState::Success => TradeStatus::Filled,
        TransactionState::Failed => TradeStatus::Failed(
            transaction
                .error_msg
                .clone()
                .unwrap_or_else(|| "simulated relayer failed".to_string()),
        ),
        _ => unreachable!("terminal state checked above"),
    };

    let filled_size = if matches!(&trade_status, TradeStatus::Filled) {
        order.size
    } else {
        Decimal::ZERO
    };

    let now = Utc::now();
    let trade_id = uuid::Uuid::new_v4().to_string();
    transaction.trade_id = Some(trade_id.clone());

    Ok(V2ExecutionOutcome {
        trade: Trade {
            id: trade_id,
            signal_id: order.signal_id.clone(),
            source_wallet: order.source_wallet.clone(),
            market_id: order.market_id.clone(),
            category: order.category,
            side: order.side,
            direction: order.direction,
            price: order.price,
            size: order.size,
            size_usd: order.size_usd,
            filled_size,
            order_type: order.order_type,
            status: trade_status,
            placed_at: now,
            filled_at: (transaction.state == TransactionState::Success).then_some(now),
            simulated: true,
            transaction_id: Some(transaction.transaction_id.clone()),
            transaction_hash: transaction.transaction_hash.clone(),
            relayer_state: Some(transaction.state),
            taker_fee_bps: 0,
            fee_paid_usdc: Decimal::ZERO,
            rebate_usdc: Decimal::ZERO,
            retry_count: 0,
            error_msg: transaction.error_msg.clone(),
        },
        transaction,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::order_builder::{build_order, Order};
    use crate::execution::v2_sim::SimulatedRelayer;
    use polybot_common::types::{
        Category, Decision, OrderType, RiskDecision, Side, TradeDirection, TradeStatus,
        TransactionState,
    };
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    fn test_order() -> Order {
        let decision = RiskDecision {
            signal_id: "sig-1".to_string(),
            source_wallet: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "market-1".to_string(),
            side: Side::Yes,
            direction: TradeDirection::Buy,
            category: Category::Politics,
            position_size_usd: dec!(50),
            target_size_tokens: None,
            confidence_multiplier: dec!(1),
            secret_level_multiplier: dec!(1),
            drawdown_factor: dec!(1),
            blocked: false,
            manual_review: false,
            decision: Decision::Execute,
        };
        let ctx = crate::execution::clob_client::MarketContext {
            token_id: "token-1".to_string(),
            tick_size: dec!(0.01),
            min_order_size: dec!(1),
            neg_risk: false,
        };
        let mut order = build_order(&decision, &ctx, dec!(0.50), dec!(50));
        order.order_type = OrderType::Limit;
        order
    }

    #[test]
    fn simulated_v2_success_creates_filled_trade_and_transaction() {
        let outcome = simulate_v2_success(&test_order()).unwrap();

        assert_eq!(outcome.trade.status, TradeStatus::Filled);
        assert!(outcome.trade.simulated);
        assert_eq!(outcome.trade.filled_size, outcome.trade.size);
        assert_eq!(outcome.trade.transaction_id.as_deref(), Some("sim-sig-1"));
        assert_eq!(outcome.trade.relayer_state, Some(TransactionState::Success));
        assert!(outcome.trade.transaction_hash.is_some());
        assert_eq!(outcome.transaction.state, TransactionState::Success);
        assert_eq!(
            outcome.transaction.trade_id.as_deref(),
            Some(outcome.trade.id.as_str())
        );
    }

    #[test]
    fn simulated_v2_failure_creates_failed_trade_and_transaction() {
        let outcome =
            simulate_with_relayer(&test_order(), SimulatedRelayer::failed("sim-fail")).unwrap();

        assert!(matches!(outcome.trade.status, TradeStatus::Failed(_)));
        assert_eq!(outcome.trade.filled_size, Decimal::ZERO);
        assert_eq!(outcome.trade.transaction_id.as_deref(), Some("sim-fail"));
        assert_eq!(outcome.trade.relayer_state, Some(TransactionState::Failed));
        assert_eq!(
            outcome.trade.error_msg.as_deref(),
            Some("simulated relayer failure")
        );
        assert_eq!(outcome.transaction.state, TransactionState::Failed);
        assert_eq!(
            outcome.transaction.trade_id.as_deref(),
            Some(outcome.trade.id.as_str())
        );
    }
}
