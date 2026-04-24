use chrono::Utc;
use polybot_common::types::*;
use rust_decimal::Decimal;

use super::clob_client::MarketContext;

#[derive(Debug, Clone)]
pub struct Order {
    pub signal_id: String,
    pub market_id: String,
    pub token_id: String,
    pub category: Category,
    pub side: Side,
    pub direction: OrderDirection,
    pub price: Decimal,
    pub size: Decimal,
    pub size_usd: Decimal,
    pub order_type: OrderType,
}

pub fn build_order(
    decision: &RiskDecision,
    market_context: &MarketContext,
    target_price: Decimal,
    size_usd: Decimal,
) -> Order {
    let order_type = select_order_type(decision);
    let price = align_to_tick_size(target_price, market_context.tick_size);
    let normalized_size_usd = size_usd.round_dp(2);
    let raw_size = if price > Decimal::ZERO {
        (normalized_size_usd / price).round_dp(2)
    } else {
        Decimal::ZERO
    };
    let size = if raw_size > Decimal::ZERO && raw_size < market_context.min_order_size {
        market_context.min_order_size
    } else {
        raw_size
    };

    Order {
        signal_id: decision.signal_id.clone(),
        market_id: decision.market_id.clone(),
        token_id: market_context.token_id.clone(),
        category: decision.category,
        side: decision.side,
        direction: decision.direction,
        price,
        size,
        size_usd: normalized_size_usd,
        order_type,
    }
}

pub fn build_order_with_price_buffer(
    decision: &RiskDecision,
    market_context: &MarketContext,
    fetched_price: Decimal,
    size_usd: Decimal,
    price_buffer: Decimal,
    order_type: OrderType,
) -> Order {
    // Direction-aware buffer: BUY pays slightly over market (round up),
    // SELL accepts slightly under market (round down). Both tilt toward
    // a more aggressive fill on their side.
    let planned_price = if order_type.requires_price_buffer() {
        match decision.direction {
            OrderDirection::Buy => align_to_tick_size_up(
                fetched_price * (Decimal::ONE + price_buffer),
                market_context.tick_size,
            ),
            OrderDirection::Sell => align_to_tick_size(
                fetched_price * (Decimal::ONE - price_buffer),
                market_context.tick_size,
            ),
        }
    } else {
        fetched_price
    };

    let mut order = build_order(decision, market_context, planned_price, size_usd);
    order.order_type = order_type;
    order
}

fn align_to_tick_size(value: Decimal, tick_size: Decimal) -> Decimal {
    if tick_size <= Decimal::ZERO {
        return value.round_dp(2);
    }

    ((value / tick_size).floor() * tick_size).normalize()
}

fn align_to_tick_size_up(value: Decimal, tick_size: Decimal) -> Decimal {
    if tick_size <= Decimal::ZERO {
        return value.round_dp(2);
    }

    ((value / tick_size).ceil() * tick_size).normalize()
}

fn select_order_type(decision: &RiskDecision) -> OrderType {
    // v2.5: Based on combined multiplier (confidence × secret_level)
    // High multiplier -> FOK, medium -> IOC, default -> Limit
    let combined = decision.confidence_multiplier * decision.secret_level_multiplier;
    if combined >= rust_decimal_macros::dec!(1.5) {
        OrderType::Fok
    } else if combined >= Decimal::ONE {
        OrderType::Ioc
    } else {
        OrderType::Limit
    }
}

pub fn create_simulated_trade(decision: &RiskDecision, order: &Order) -> Trade {
    Trade {
        id: uuid::Uuid::new_v4().to_string(),
        signal_id: decision.signal_id.clone(),
        market_id: order.market_id.clone(),
        category: decision.category,
        side: order.side,
        direction: order.direction,
        price: order.price,
        size: order.size,
        size_usd: order.size_usd,
        filled_size: order.size, // Simulated: fully filled
        order_type: order.order_type,
        status: TradeStatus::Filled,
        placed_at: Utc::now(),
        filled_at: Some(Utc::now()),
        simulated: true,
        transaction_id: None,
        transaction_hash: None,
        relayer_state: None,
        taker_fee_bps: 0,
        fee_paid_usdc: rust_decimal::Decimal::ZERO,
        rebate_usdc: rust_decimal::Decimal::ZERO,
        retry_count: 0,
        error_msg: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::clob_client::MarketContext;
    use rust_decimal_macros::dec;

    fn test_decision(conf_mult: Decimal, sl_mult: Decimal) -> RiskDecision {
        test_decision_with_direction(conf_mult, sl_mult, OrderDirection::Buy)
    }

    fn test_decision_with_direction(
        conf_mult: Decimal,
        sl_mult: Decimal,
        direction: OrderDirection,
    ) -> RiskDecision {
        RiskDecision {
            signal_id: "test-id".to_string(),
            market_id: "market-1".to_string(),
            side: Side::No,
            direction,
            category: Category::Crypto,
            position_size_usd: dec!(50),
            confidence_multiplier: conf_mult,
            secret_level_multiplier: sl_mult,
            drawdown_factor: dec!(1.0),
            blocked: false,
            manual_review: false,
            decision: Decision::Execute,
        }
    }

    #[test]
    fn high_combined_selects_fok() {
        let decision = test_decision(dec!(1.5), dec!(2.5)); // 3.75 combined
        let order = build_order(&decision, &test_market_context(), dec!(0.50), dec!(50));
        assert_eq!(order.order_type, OrderType::Fok);
    }

    #[test]
    fn medium_combined_selects_ioc() {
        let decision = test_decision(dec!(1.0), dec!(1.0)); // 1.0 combined
        let order = build_order(&decision, &test_market_context(), dec!(0.50), dec!(50));
        assert_eq!(order.order_type, OrderType::Ioc);
    }

    #[test]
    fn low_combined_selects_limit() {
        let decision = test_decision(dec!(0.5), dec!(0.6)); // 0.3 combined
        let order = build_order(&decision, &test_market_context(), dec!(0.50), dec!(50));
        assert_eq!(order.order_type, OrderType::Limit);
    }

    #[test]
    fn simulated_trade_is_marked() {
        let decision = test_decision(dec!(1.0), dec!(1.0));
        let order = build_order(&decision, &test_market_context(), dec!(0.50), dec!(50));
        let trade = create_simulated_trade(&decision, &order);
        assert!(trade.simulated);
        assert_eq!(trade.status, TradeStatus::Filled);
        assert_eq!(trade.market_id, "market-1");
        assert_eq!(trade.side, Side::No);
        assert_eq!(trade.category, Category::Crypto);
    }

    #[test]
    fn build_order_preserves_signal_context() {
        let decision = test_decision(dec!(1.0), dec!(1.0));
        let order = build_order(&decision, &test_market_context(), dec!(0.25), dec!(50));

        assert_eq!(order.market_id, "market-1");
        assert_eq!(order.token_id, "token-1");
        assert_eq!(order.side, Side::No);
        assert_eq!(order.price, dec!(0.25));
        assert_eq!(order.size_usd, dec!(50));
        assert_eq!(order.size, dec!(200));
    }

    #[test]
    fn build_order_respects_tick_size() {
        let decision = test_decision(dec!(1.0), dec!(1.0));
        let ctx = MarketContext {
            token_id: "token-1".to_string(),
            tick_size: dec!(0.001),
            min_order_size: dec!(1),
            neg_risk: false,
        };

        let order = build_order(&decision, &ctx, dec!(0.537), dec!(50));
        assert_eq!(order.price, dec!(0.537));
    }

    #[test]
    fn fok_plan_applies_price_buffer() {
        let decision = test_decision(dec!(1.0), dec!(1.0));
        let ctx = test_market_context();
        let order = build_order_with_price_buffer(
            &decision,
            &ctx,
            dec!(0.50),
            dec!(50),
            dec!(0.01),
            OrderType::Fok,
        );
        assert_eq!(order.price, dec!(0.51));
    }

    #[test]
    fn buy_fok_plan_applies_upward_price_buffer() {
        let decision =
            test_decision_with_direction(dec!(1.0), dec!(1.0), OrderDirection::Buy);
        let ctx = test_market_context();
        let order = build_order_with_price_buffer(
            &decision,
            &ctx,
            dec!(0.50),
            dec!(50),
            dec!(0.02),
            OrderType::Fok,
        );
        assert_eq!(order.price, dec!(0.51));
        assert_eq!(order.direction, OrderDirection::Buy);
    }

    #[test]
    fn sell_fok_plan_applies_reverse_price_buffer() {
        let decision =
            test_decision_with_direction(dec!(1.0), dec!(1.0), OrderDirection::Sell);
        let ctx = test_market_context();
        let order = build_order_with_price_buffer(
            &decision,
            &ctx,
            dec!(0.50),
            dec!(50),
            dec!(0.02),
            OrderType::Fok,
        );
        assert_eq!(order.price, dec!(0.49));
        assert_eq!(order.direction, OrderDirection::Sell);
    }

    #[test]
    fn build_order_propagates_direction() {
        let ctx = test_market_context();
        let buy =
            test_decision_with_direction(dec!(1.0), dec!(1.0), OrderDirection::Buy);
        let buy_order = build_order(&buy, &ctx, dec!(0.50), dec!(50));
        assert_eq!(buy_order.direction, OrderDirection::Buy);

        let sell =
            test_decision_with_direction(dec!(1.0), dec!(1.0), OrderDirection::Sell);
        let sell_order = build_order(&sell, &ctx, dec!(0.50), dec!(50));
        assert_eq!(sell_order.direction, OrderDirection::Sell);
    }

    #[test]
    fn simulated_trade_carries_direction() {
        let ctx = test_market_context();
        let decision =
            test_decision_with_direction(dec!(1.0), dec!(1.0), OrderDirection::Sell);
        let order = build_order(&decision, &ctx, dec!(0.50), dec!(50));
        let trade = create_simulated_trade(&decision, &order);
        assert_eq!(trade.direction, OrderDirection::Sell);
    }

    #[test]
    fn fok_plan_is_not_silently_downgraded() {
        let decision = test_decision(dec!(1.0), dec!(1.0));
        let ctx = test_market_context();
        let order = build_order_with_price_buffer(
            &decision,
            &ctx,
            dec!(0.50),
            dec!(50),
            dec!(0.00),
            OrderType::Fok,
        );
        assert_eq!(order.order_type, OrderType::Fok);
    }

    fn test_market_context() -> MarketContext {
        MarketContext {
            token_id: "token-1".to_string(),
            tick_size: dec!(0.01),
            min_order_size: dec!(1),
            neg_risk: false,
        }
    }
}
