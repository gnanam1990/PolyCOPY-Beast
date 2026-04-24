use polybot_common::constants::drawdown_multiplier;
use rust_decimal::Decimal;

// v2.5: Drawdown module now delegates to the stepped curve in constants.
// Kept as a module for organizational consistency.

/// Calculate drawdown factor using v2.5 stepped curve.
/// Delegates to `polybot_common::constants::drawdown_multiplier`.
pub fn calculate_drawdown_factor(
    current_drawdown_pct: Decimal,
    _daily_max_loss_pct: Decimal,
) -> Decimal {
    // v2.5: drawdown is based on portfolio drawdown percentage directly,
    // not relative to daily_max_loss. The stepped curve is:
    // 0-5% -> 1.0, 5-10% -> 0.75, 10-15% -> 0.5, 15-20% -> 0.25, >20% -> 0.0
    drawdown_multiplier(current_drawdown_pct)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn no_drawdown() {
        let factor = calculate_drawdown_factor(dec!(0), dec!(0.05));
        assert_eq!(factor, dec!(1.0));
    }

    #[test]
    fn stepped_5_pct() {
        let factor = calculate_drawdown_factor(dec!(0.05), dec!(0.05));
        assert_eq!(factor, dec!(0.75));
    }

    #[test]
    fn stepped_10_pct() {
        let factor = calculate_drawdown_factor(dec!(0.10), dec!(0.05));
        assert_eq!(factor, dec!(0.5));
    }

    #[test]
    fn stepped_15_pct() {
        let factor = calculate_drawdown_factor(dec!(0.15), dec!(0.05));
        assert_eq!(factor, dec!(0.25));
    }

    #[test]
    fn stepped_20_pct_auto_pause() {
        let factor = calculate_drawdown_factor(dec!(0.20), dec!(0.05));
        assert_eq!(factor, dec!(0.0));
    }

    #[test]
    fn under_5_pct() {
        let factor = calculate_drawdown_factor(dec!(0.03), dec!(0.05));
        assert_eq!(factor, dec!(1.0));
    }
}
