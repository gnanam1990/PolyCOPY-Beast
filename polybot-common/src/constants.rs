use crate::types::ExecutionMode;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// v2.5 Confidence multiplier table (1-10 scale).
/// Confidence 1-3 = 0.0 (blocked, manual review queue).
pub fn confidence_multiplier(confidence: u8) -> Decimal {
    match confidence {
        1..=3 => dec!(0.0),
        4 => dec!(0.5),
        5 => dec!(0.75),
        6 => dec!(1.0),
        7 => dec!(1.25),
        8 => dec!(1.5),
        9 => dec!(1.75),
        10 => dec!(2.0),
        _ => dec!(0.0),
    }
}

/// v2.5 Secret level multiplier table (1-10 scale).
/// Secret level 1-3 = 0.0 (blocked, manual review queue).
pub fn secret_level_multiplier(secret_level: u8) -> Decimal {
    match secret_level {
        1..=3 => dec!(0.0),
        4 => dec!(0.6),
        5 => dec!(0.8),
        6 => dec!(1.0),
        7 => dec!(1.3),
        8 => dec!(1.6),
        9 => dec!(1.9),
        10 => dec!(2.5),
        _ => dec!(0.0),
    }
}

/// v2.5 Stepped drawdown multiplier.
/// 0-5% -> 1.0, 5-10% -> 0.75, 10-15% -> 0.5, 15-20% -> 0.25, >20% -> 0.0 (auto-pause).
pub fn drawdown_multiplier(portfolio_drawdown_pct: Decimal) -> Decimal {
    if portfolio_drawdown_pct < dec!(0.05) {
        dec!(1.0)
    } else if portfolio_drawdown_pct < dec!(0.10) {
        dec!(0.75)
    } else if portfolio_drawdown_pct < dec!(0.15) {
        dec!(0.5)
    } else if portfolio_drawdown_pct < dec!(0.20) {
        dec!(0.25)
    } else {
        dec!(0.0)
    }
}

/// v2.5 Defaults
pub const DEFAULT_BASE_SIZE_PCT: Decimal = dec!(0.015); // 1.5% of portfolio
pub const MIN_POSITION_USDC: Decimal = dec!(5);
pub const MAX_POSITION_USDC: Decimal = dec!(500);
pub const MAX_CONCURRENT_POSITIONS: u32 = 20;
pub const MAX_MARKET_LIQUIDITY_PCT: Decimal = dec!(0.02); // 2% of market liquidity

pub const DEFAULT_EXECUTION_MODE: ExecutionMode = ExecutionMode::Simulation;

pub const DEFAULT_DAILY_MAX_LOSS_PCT: Decimal = dec!(0.05);
pub const DEFAULT_PER_MARKET_EXPOSURE_PCT: Decimal = dec!(0.10);
pub const DEFAULT_PER_CATEGORY_EXPOSURE_PCT: Decimal = dec!(0.25);
pub const DEFAULT_MIN_CONFIDENCE: u8 = 6;
pub const DEFAULT_MIN_SECRET_LEVEL: u8 = 5;
pub const DEFAULT_SLIPPAGE_THRESHOLD: Decimal = dec!(0.02);
pub const DEFAULT_DEDUP_WINDOW_SECS: u64 = 300; // 5 minutes per v2.5
pub const LIGHT_RECONCILIATION_INTERVAL_SECS: u64 = 30;
pub const FULL_RECONCILIATION_INTERVAL_SECS: u64 = 300; // 5 minutes
pub const SIGNAL_STALENESS_SECS: u64 = 30;
pub const ORDER_TIMEOUT_SECS: u64 = 30;
pub const CLOB_RATE_LIMIT_PER_MIN: u32 = 100;
pub const CLOB_READ_LIMIT_PER_MIN: u32 = 300;
pub const CIRCUIT_BREAKER_PCT: Decimal = dec!(0.80); // activate at 80% of rate limit
pub const WS_HEARTBEAT_SECS: u64 = 30;
pub const WS_PONG_TIMEOUT_SECS: u64 = 10;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_multiplier_table() {
        assert_eq!(confidence_multiplier(1), dec!(0.0)); // blocked
        assert_eq!(confidence_multiplier(2), dec!(0.0)); // blocked
        assert_eq!(confidence_multiplier(3), dec!(0.0)); // blocked
        assert_eq!(confidence_multiplier(4), dec!(0.5));
        assert_eq!(confidence_multiplier(5), dec!(0.75));
        assert_eq!(confidence_multiplier(6), dec!(1.0));
        assert_eq!(confidence_multiplier(7), dec!(1.25));
        assert_eq!(confidence_multiplier(8), dec!(1.5));
        assert_eq!(confidence_multiplier(9), dec!(1.75));
        assert_eq!(confidence_multiplier(10), dec!(2.0));
    }

    #[test]
    fn confidence_multiplier_out_of_range() {
        assert_eq!(confidence_multiplier(0), dec!(0.0));
        assert_eq!(confidence_multiplier(15), dec!(0.0));
    }

    #[test]
    fn secret_level_multiplier_table() {
        assert_eq!(secret_level_multiplier(1), dec!(0.0)); // blocked
        assert_eq!(secret_level_multiplier(3), dec!(0.0)); // blocked
        assert_eq!(secret_level_multiplier(4), dec!(0.6));
        assert_eq!(secret_level_multiplier(5), dec!(0.8));
        assert_eq!(secret_level_multiplier(6), dec!(1.0));
        assert_eq!(secret_level_multiplier(7), dec!(1.3));
        assert_eq!(secret_level_multiplier(8), dec!(1.6));
        assert_eq!(secret_level_multiplier(9), dec!(1.9));
        assert_eq!(secret_level_multiplier(10), dec!(2.5));
    }

    #[test]
    fn drawdown_multiplier_stepped() {
        assert_eq!(drawdown_multiplier(dec!(0.0)), dec!(1.0)); // 0%
        assert_eq!(drawdown_multiplier(dec!(0.03)), dec!(1.0)); // 3%
        assert_eq!(drawdown_multiplier(dec!(0.05)), dec!(0.75)); // 5% (boundary)
        assert_eq!(drawdown_multiplier(dec!(0.07)), dec!(0.75)); // 7%
        assert_eq!(drawdown_multiplier(dec!(0.10)), dec!(0.5)); // 10%
        assert_eq!(drawdown_multiplier(dec!(0.12)), dec!(0.5)); // 12%
        assert_eq!(drawdown_multiplier(dec!(0.15)), dec!(0.25)); // 15%
        assert_eq!(drawdown_multiplier(dec!(0.18)), dec!(0.25)); // 18%
        assert_eq!(drawdown_multiplier(dec!(0.20)), dec!(0.0)); // 20% (auto-pause)
        assert_eq!(drawdown_multiplier(dec!(0.30)), dec!(0.0)); // 30% (auto-pause)
    }
}
