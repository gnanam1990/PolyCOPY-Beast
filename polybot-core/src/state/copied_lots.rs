use chrono::{DateTime, Utc};
use polybot_common::errors::PolybotError;
use polybot_common::types::{Side, TradeDirection};
use rust_decimal::Decimal;

#[derive(Debug, Clone, PartialEq)]
pub struct CopiedLot {
    pub id: String,
    pub source_wallet: String,
    pub market_id: String,
    pub side: Side,
    pub current_size: Decimal,
    pub average_price: Decimal,
    pub opened_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_signal_id: Option<String>,
    pub last_tx_hash: Option<String>,
}

impl CopiedLot {
    pub fn apply_fill(
        &self,
        direction: TradeDirection,
        fill_size: Decimal,
        fill_price: Decimal,
    ) -> Result<Option<Self>, PolybotError> {
        match direction {
            TradeDirection::Buy => {
                let new_size = self.current_size + fill_size;
                let total_cost = self.average_price * self.current_size + fill_price * fill_size;

                Ok(Some(Self {
                    current_size: new_size,
                    average_price: if new_size > Decimal::ZERO {
                        total_cost / new_size
                    } else {
                        Decimal::ZERO
                    },
                    updated_at: Utc::now(),
                    ..self.clone()
                }))
            }
            TradeDirection::Sell => {
                let new_size = (self.current_size - fill_size).max(Decimal::ZERO);
                if new_size == Decimal::ZERO {
                    Ok(None)
                } else {
                    Ok(Some(Self {
                        current_size: new_size,
                        updated_at: Utc::now(),
                        ..self.clone()
                    }))
                }
            }
        }
    }

    pub fn reduce_fraction(&self, fraction: Decimal) -> Result<Self, PolybotError> {
        if fraction <= Decimal::ZERO || fraction > Decimal::ONE {
            return Err(PolybotError::State(format!(
                "invalid exit fraction: {}",
                fraction
            )));
        }

        let exit_size = (self.current_size * fraction).round_dp(8);
        let remaining = (self.current_size - exit_size).max(Decimal::ZERO);

        Ok(Self {
            current_size: remaining,
            updated_at: Utc::now(),
            ..self.clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn test_lot() -> CopiedLot {
        CopiedLot {
            id: "lot-1".to_string(),
            source_wallet: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "market-1".to_string(),
            side: Side::Yes,
            current_size: dec!(10),
            average_price: dec!(0.55),
            opened_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_signal_id: Some("sig-1".to_string()),
            last_tx_hash: None,
        }
    }

    #[test]
    fn reduce_lot_by_fraction_only_affects_matching_wallet() {
        let lot = test_lot();

        let reduced = lot.reduce_fraction(dec!(0.30)).unwrap();
        assert_eq!(reduced.current_size, dec!(7));
    }

    #[test]
    fn buy_fill_updates_weighted_average_cost() {
        let lot = test_lot();

        let updated = lot
            .apply_fill(TradeDirection::Buy, dec!(5), dec!(0.70))
            .unwrap()
            .unwrap();

        assert_eq!(updated.current_size, dec!(15));
        assert_eq!(updated.average_price, dec!(0.60));
    }

    #[test]
    fn sell_fill_partially_reduces_size() {
        let lot = test_lot();

        let updated = lot
            .apply_fill(TradeDirection::Sell, dec!(4), dec!(0.70))
            .unwrap()
            .unwrap();

        assert_eq!(updated.current_size, dec!(6));
        assert_eq!(updated.average_price, lot.average_price);
    }

    #[test]
    fn sell_fill_fully_exits_to_none() {
        let lot = test_lot();

        let updated = lot.apply_fill(TradeDirection::Sell, dec!(10), dec!(0.70)).unwrap();

        assert_eq!(updated, None);
    }
}
