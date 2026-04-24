use polybot_common::constants::MAX_MARKET_LIQUIDITY_PCT;
use polybot_common::types::Signal;
use rust_decimal::Decimal;

use crate::config::RiskConfig;

pub fn check_limits(
    config: &RiskConfig,
    signal: &Signal,
    proposed_size: Decimal,
    current_drawdown: Decimal,
    current_market_exposure: Decimal,
    current_category_exposure: Decimal,
    portfolio_value_usd: Decimal,
) -> Option<String> {
    // v2.5: Check daily loss limit (drawdown-based)
    if current_drawdown >= config.daily_max_loss_pct {
        return Some(format!(
            "Daily loss {:.2}% exceeds limit {:.2}%",
            current_drawdown, config.daily_max_loss_pct
        ));
    }

    // Check max position size (hard cap + per-category cap)
    let category_max = signal.category.max_single_position_usd();
    if proposed_size > config.max_position_size_usd && proposed_size > category_max {
        return Some(format!("Position size ${:.2} exceeds max", proposed_size));
    }

    if portfolio_value_usd > Decimal::ZERO {
        let market_limit = portfolio_value_usd * config.per_market_exposure_pct;
        if current_market_exposure + proposed_size > market_limit {
            return Some(format!(
                "Market exposure ${:.2} exceeds limit ${:.2}",
                current_market_exposure + proposed_size,
                market_limit
            ));
        }

        let category_limit_pct = config
            .per_category_exposure_pct
            .min(signal.category.max_exposure_pct());
        let category_limit = portfolio_value_usd * category_limit_pct;
        if current_category_exposure + proposed_size > category_limit {
            return Some(format!(
                "Category exposure ${:.2} exceeds limit ${:.2}",
                current_category_exposure + proposed_size,
                category_limit
            ));
        }
    }

    None
}

/// v2.5: Check that position size doesn't exceed 2% of market liquidity.
/// Returns the size adjusted for market liquidity cap, or the original size if under cap.
pub fn apply_market_liquidity_cap(
    proposed_size: Decimal,
    market_liquidity_usd: Decimal,
) -> Decimal {
    if market_liquidity_usd <= Decimal::ZERO {
        // If we don't know market liquidity, don't cap
        return proposed_size;
    }

    let max_allowed = market_liquidity_usd * MAX_MARKET_LIQUIDITY_PCT;
    if proposed_size > max_allowed {
        tracing::warn!(
            proposed = %proposed_size,
            market_liquidity = %market_liquidity_usd,
            cap = %max_allowed,
            "Position size exceeds 2% of market liquidity, capping"
        );
        max_allowed
    } else {
        proposed_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use polybot_common::types::*;
    use rust_decimal_macros::dec;

    fn test_signal(secret_level: u8, confidence: u8, category: Category) -> Signal {
        Signal {
            signal_id: "test-id".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            wallet_address: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "market-1".to_string(),
            side: Side::Yes,
            direction: TradeDirection::Buy,
            confidence,
            secret_level,
            category,
            source: SignalSource::Manual,
            tx_hash: None,
            token_id: None,
            target_price: None,
            target_size_usdc: None,
            target_size_tokens: None,
            resolved: false,
            redeemable: false,
            suggested_size_usdc: Some(dec!(50)),
            fee_schedule: None,
            scanner_version: "1.0.0".to_string(),
        }
    }

    #[test]
    fn no_limits_breached() {
        let config = crate::config::AppConfig::default();
        let signal = test_signal(7, 7, Category::Politics);
        let result = check_limits(
            &config.risk,
            &signal,
            dec!(50),
            dec!(0),
            dec!(0),
            dec!(0),
            dec!(5000),
        );
        assert!(result.is_none());
    }

    #[test]
    fn daily_drawdown_limit_breached() {
        let config = crate::config::AppConfig::default();
        let signal = test_signal(7, 7, Category::Politics);
        let result = check_limits(
            &config.risk,
            &signal,
            dec!(50),
            config.risk.daily_max_loss_pct,
            dec!(0),
            dec!(0),
            dec!(5000),
        );
        assert!(result.is_some());
        assert!(result.unwrap().contains("Daily loss"));
    }

    #[test]
    fn market_exposure_limit_breached() {
        let config = crate::config::AppConfig::default();
        let signal = test_signal(7, 7, Category::Politics);
        let result = check_limits(
            &config.risk,
            &signal,
            dec!(50),
            dec!(0),
            dec!(490),
            dec!(0),
            dec!(5000),
        );
        assert!(result.unwrap().contains("Market exposure"));
    }

    #[test]
    fn category_exposure_limit_breached() {
        let config = crate::config::AppConfig::default();
        let signal = test_signal(7, 7, Category::Crypto);
        let result = check_limits(
            &config.risk,
            &signal,
            dec!(50),
            dec!(0),
            dec!(0),
            dec!(720),
            dec!(5000),
        );
        assert!(result.unwrap().contains("Category exposure"));
    }

    #[test]
    fn crypto_category_has_lower_max() {
        assert_eq!(Category::Crypto.max_single_position_usd(), dec!(150));
        assert_eq!(Category::Politics.max_single_position_usd(), dec!(250));
    }

    #[test]
    fn market_liquidity_cap_under_limit() {
        // 2% of $10,000 = $200. Proposed $50 is under.
        let size = apply_market_liquidity_cap(dec!(50), dec!(10000));
        assert_eq!(size, dec!(50));
    }

    #[test]
    fn market_liquidity_cap_over_limit() {
        // 2% of $1000 = $20. Proposed $50 exceeds, capped to $20.
        let size = apply_market_liquidity_cap(dec!(50), dec!(1000));
        assert_eq!(size, dec!(20));
    }

    #[test]
    fn market_liquidity_cap_zero_liquidity() {
        // Unknown liquidity — don't cap
        let size = apply_market_liquidity_cap(dec!(50), dec!(0));
        assert_eq!(size, dec!(50));
    }
}
