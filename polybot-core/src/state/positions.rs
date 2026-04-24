use chrono::Utc;
use polybot_common::errors::PolybotError;
use polybot_common::types::{Category, Position, PositionKey, PositionStatus, Trade, TradeStatus};
use rust_decimal::Decimal;
use std::collections::HashMap;

pub struct PositionManager {
    positions: HashMap<PositionKey, Position>,
}

impl PositionManager {
    pub fn new() -> Self {
        Self {
            positions: HashMap::new(),
        }
    }

    fn key_for_trade(trade: &Trade) -> PositionKey {
        PositionKey::new(trade.market_id.clone(), trade.side)
    }

    pub fn update_from_trade(&mut self, trade: &Trade) -> Result<(), PolybotError> {
        match trade.status {
            TradeStatus::Filled | TradeStatus::PartiallyFilled => self.apply_fill(trade),
            TradeStatus::Cancelled | TradeStatus::TimedOut => {
                tracing::info!(trade_id = %trade.id, "Trade cancelled/timed out");
                Ok(())
            }
            TradeStatus::Pending => {
                tracing::debug!(trade_id = %trade.id, "Trade pending");
                Ok(())
            }
            TradeStatus::Failed(ref reason) => {
                tracing::warn!(trade_id = %trade.id, reason = %reason, "Trade failed");
                Ok(())
            }
        }
    }

    fn apply_fill(&mut self, trade: &Trade) -> Result<(), PolybotError> {
        let position = self
            .positions
            .entry(Self::key_for_trade(trade))
            .or_insert_with(|| Position {
                id: uuid::Uuid::new_v4().to_string(),
                market_id: trade.market_id.clone(),
                side: trade.side,
                entry_price: trade.price,
                current_size: Decimal::ZERO,
                average_price: Decimal::ZERO,
                opened_at: Utc::now(),
                status: PositionStatus::Open,
                category: trade.category,
            });

        let new_size = position.current_size + trade.filled_size;
        if new_size > Decimal::ZERO {
            let total_cost =
                position.average_price * position.current_size + trade.price * trade.filled_size;
            position.average_price = total_cost / new_size;
        }
        position.current_size = new_size;

        tracing::info!(
            market_id = %position.market_id,
            size = %position.current_size,
            avg_price = %position.average_price,
            "Position updated"
        );

        Ok(())
    }

    pub fn close_position(&mut self, key: &PositionKey) -> Result<Position, PolybotError> {
        let mut position = self.positions.remove(key).ok_or_else(|| {
            PolybotError::State(format!(
                "Position not found: market={} side={:?}",
                key.market_id, key.side
            ))
        })?;
        position.status = PositionStatus::Closed;
        Ok(position)
    }

    pub fn close_all_positions(&mut self) -> Vec<Position> {
        let closed_positions: Vec<Position> = self
            .positions
            .drain()
            .map(|(_, mut position)| {
                position.status = PositionStatus::Closed;
                position
            })
            .collect();

        if !closed_positions.is_empty() {
            tracing::warn!(
                count = closed_positions.len(),
                "All positions closed via emergency flatten"
            );
        }

        closed_positions
    }

    /// Mark a position as ghost (in Redis but not on-chain)
    pub fn mark_ghost(&mut self, key: &PositionKey) -> Result<(), PolybotError> {
        if let Some(pos) = self.positions.get_mut(key) {
            pos.status = PositionStatus::Ghost;
            tracing::warn!(
                market_id = %key.market_id,
                side = ?key.side,
                "Position marked as ghost"
            );
        }
        Ok(())
    }

    pub fn get_position(&self, key: &PositionKey) -> Option<&Position> {
        self.positions.get(key)
    }

    pub fn restore_positions(&mut self, positions: Vec<Position>) {
        self.positions = positions
            .into_iter()
            .map(|position| {
                (
                    PositionKey::new(position.market_id.clone(), position.side),
                    position,
                )
            })
            .collect();
    }

    #[allow(dead_code)]
    pub fn get_positions_vec(&self) -> Vec<&Position> {
        self.positions.values().collect()
    }

    pub fn open_position_count(&self) -> u32 {
        self.positions
            .values()
            .filter(|p| p.status == PositionStatus::Open)
            .count() as u32
    }

    pub fn total_exposure(&self) -> Decimal {
        self.positions
            .values()
            .filter(|p| p.status == PositionStatus::Open)
            .map(|p| p.current_size * p.average_price)
            .fold(Decimal::ZERO, |acc, x| acc + x)
    }

    pub fn market_exposure(&self, market_id: &str) -> Decimal {
        self.positions
            .values()
            .filter(|p| p.status == PositionStatus::Open && p.market_id == market_id)
            .map(|p| p.current_size * p.average_price)
            .fold(Decimal::ZERO, |acc, x| acc + x)
    }

    pub fn category_exposure(&self, category: Category) -> Decimal {
        self.positions
            .values()
            .filter(|p| p.status == PositionStatus::Open && p.category == category)
            .map(|p| p.current_size * p.average_price)
            .fold(Decimal::ZERO, |acc, x| acc + x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use polybot_common::types::{OrderDirection, OrderType, PositionKey, Side};
    use rust_decimal_macros::dec;

    fn test_trade(
        market_id: &str,
        price: Decimal,
        filled_size: Decimal,
        side: Side,
        category: Category,
    ) -> Trade {
        Trade {
            id: "t1".to_string(),
            signal_id: "s1".to_string(),
            market_id: market_id.to_string(),
            category,
            side,
            direction: OrderDirection::Buy,
            price,
            size: filled_size,
            size_usd: price * filled_size,
            filled_size,
            order_type: OrderType::Limit,
            status: TradeStatus::Filled,
            placed_at: Utc::now(),
            filled_at: Some(Utc::now()),
            simulated: true,
        }
    }

    #[test]
    fn add_fill_creates_position() {
        let mut pm = PositionManager::new();
        let trade = test_trade(
            "market-1",
            dec!(0.65),
            dec!(100),
            Side::Yes,
            Category::Politics,
        );
        pm.update_from_trade(&trade).unwrap();
        let pos = pm
            .get_position(&PositionKey::new("market-1", Side::Yes))
            .unwrap();
        assert_eq!(pos.current_size, dec!(100));
        assert_eq!(pos.average_price, dec!(0.65));
        assert_eq!(pos.category, Category::Politics);
    }

    #[test]
    fn multiple_fills_update_average_price() {
        let mut pm = PositionManager::new();
        let trade1 = test_trade(
            "market-1",
            dec!(0.60),
            dec!(100),
            Side::Yes,
            Category::Politics,
        );
        let trade2 = test_trade(
            "market-1",
            dec!(0.70),
            dec!(100),
            Side::Yes,
            Category::Politics,
        );
        pm.update_from_trade(&trade1).unwrap();
        pm.update_from_trade(&trade2).unwrap();
        let pos = pm
            .get_position(&PositionKey::new("market-1", Side::Yes))
            .unwrap();
        assert_eq!(pos.current_size, dec!(200));
        assert_eq!(pos.average_price, dec!(0.65));
    }

    #[test]
    fn close_position_removes_it() {
        let mut pm = PositionManager::new();
        let trade = test_trade(
            "market-1",
            dec!(0.65),
            dec!(100),
            Side::Yes,
            Category::Politics,
        );
        pm.update_from_trade(&trade).unwrap();
        let closed = pm
            .close_position(&PositionKey::new("market-1", Side::Yes))
            .unwrap();
        assert_eq!(closed.status, PositionStatus::Closed);
        assert!(pm
            .get_position(&PositionKey::new("market-1", Side::Yes))
            .is_none());
    }

    #[test]
    fn open_position_count() {
        let mut pm = PositionManager::new();
        assert_eq!(pm.open_position_count(), 0);
        pm.update_from_trade(&test_trade(
            "m1",
            dec!(0.5),
            dec!(100),
            Side::Yes,
            Category::Politics,
        ))
        .unwrap();
        assert_eq!(pm.open_position_count(), 1);
        pm.update_from_trade(&test_trade(
            "m2",
            dec!(0.6),
            dec!(100),
            Side::Yes,
            Category::Crypto,
        ))
        .unwrap();
        assert_eq!(pm.open_position_count(), 2);
    }

    #[test]
    fn exposure_helpers_track_market_and_category() {
        let mut pm = PositionManager::new();
        pm.update_from_trade(&test_trade(
            "m1",
            dec!(0.5),
            dec!(100),
            Side::Yes,
            Category::Politics,
        ))
        .unwrap();
        pm.update_from_trade(&test_trade(
            "m2",
            dec!(0.6),
            dec!(50),
            Side::Yes,
            Category::Crypto,
        ))
        .unwrap();

        assert_eq!(pm.market_exposure("m1"), dec!(50));
        assert_eq!(pm.category_exposure(Category::Politics), dec!(50));
        assert_eq!(pm.category_exposure(Category::Crypto), dec!(30));
    }

    #[test]
    fn separate_sides_create_separate_positions() {
        let mut pm = PositionManager::new();
        pm.update_from_trade(&test_trade(
            "m1",
            dec!(0.5),
            dec!(10),
            Side::Yes,
            Category::Politics,
        ))
        .unwrap();
        pm.update_from_trade(&test_trade(
            "m1",
            dec!(0.6),
            dec!(10),
            Side::No,
            Category::Politics,
        ))
        .unwrap();

        assert!(pm
            .get_position(&PositionKey::new("m1", Side::Yes))
            .is_some());
        assert!(pm.get_position(&PositionKey::new("m1", Side::No)).is_some());
        assert_eq!(pm.open_position_count(), 2);
    }

    #[test]
    fn close_all_positions_returns_closed_positions() {
        let mut pm = PositionManager::new();
        pm.update_from_trade(&test_trade(
            "m1",
            dec!(0.5),
            dec!(10),
            Side::Yes,
            Category::Politics,
        ))
        .unwrap();
        pm.update_from_trade(&test_trade(
            "m2",
            dec!(0.6),
            dec!(10),
            Side::No,
            Category::Crypto,
        ))
        .unwrap();

        let closed = pm.close_all_positions();

        assert_eq!(closed.len(), 2);
        assert!(closed
            .iter()
            .all(|position| position.status == PositionStatus::Closed));
        assert_eq!(pm.open_position_count(), 0);
    }
}
