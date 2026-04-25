use polybot_common::errors::PolybotError;
use rust_decimal::Decimal;

#[derive(Debug, Clone, PartialEq)]
pub struct VirtualPusdAccount {
    available: Decimal,
    reserved: Decimal,
    fees_paid: Decimal,
    rebates_earned: Decimal,
}

impl VirtualPusdAccount {
    pub fn new(starting_balance: Decimal) -> Self {
        Self {
            available: starting_balance.max(Decimal::ZERO),
            reserved: Decimal::ZERO,
            fees_paid: Decimal::ZERO,
            rebates_earned: Decimal::ZERO,
        }
    }

    pub fn available(&self) -> Decimal {
        self.available
    }

    pub fn reserved(&self) -> Decimal {
        self.reserved
    }

    pub fn fees_paid(&self) -> Decimal {
        self.fees_paid
    }

    pub fn rebates_earned(&self) -> Decimal {
        self.rebates_earned
    }

    pub fn reserve(&mut self, amount: Decimal) -> Result<(), PolybotError> {
        if amount <= Decimal::ZERO {
            return Err(PolybotError::Risk(
                "pUSD reserve amount must be positive".to_string(),
            ));
        }
        if self.available < amount {
            return Err(PolybotError::Risk(format!(
                "insufficient virtual pUSD: available={}, required={}",
                self.available, amount
            )));
        }
        self.available -= amount;
        self.reserved += amount;
        Ok(())
    }

    pub fn release(&mut self, amount: Decimal) -> Result<(), PolybotError> {
        if amount <= Decimal::ZERO {
            return Err(PolybotError::Risk(
                "pUSD release amount must be positive".to_string(),
            ));
        }
        if self.reserved < amount {
            return Err(PolybotError::Risk(format!(
                "insufficient reserved pUSD to release: reserved={}, release={}",
                self.reserved, amount
            )));
        }
        self.reserved -= amount;
        self.available += amount;
        Ok(())
    }

    pub fn fill(
        &mut self,
        reserved_spend: Decimal,
        fee: Decimal,
        rebate: Decimal,
    ) -> Result<(), PolybotError> {
        if reserved_spend <= Decimal::ZERO {
            return Err(PolybotError::Risk(
                "pUSD fill amount must be positive".to_string(),
            ));
        }
        if fee < Decimal::ZERO {
            return Err(PolybotError::Risk(
                "pUSD fill fee must not be negative".to_string(),
            ));
        }
        if rebate < Decimal::ZERO {
            return Err(PolybotError::Risk(
                "pUSD fill rebate must not be negative".to_string(),
            ));
        }
        if self.reserved < reserved_spend {
            return Err(PolybotError::Risk(format!(
                "insufficient reserved pUSD: reserved={}, fill={}",
                self.reserved, reserved_spend
            )));
        }
        self.reserved -= reserved_spend;
        self.fees_paid += fee;
        self.rebates_earned += rebate;
        self.available += rebate;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn virtual_pusd_reserve_release_and_fill() {
        let mut account = VirtualPusdAccount::new(dec!(100));
        account.reserve(dec!(25)).unwrap();
        assert_eq!(account.available(), dec!(75));
        assert_eq!(account.reserved(), dec!(25));

        account.release(dec!(5)).unwrap();
        assert_eq!(account.available(), dec!(80));
        assert_eq!(account.reserved(), dec!(20));

        account.fill(dec!(20), dec!(0.10), dec!(0.02)).unwrap();
        assert_eq!(account.available(), dec!(80.02));
        assert_eq!(account.reserved(), dec!(0));
        assert_eq!(account.fees_paid(), dec!(0.10));
        assert_eq!(account.rebates_earned(), dec!(0.02));
    }

    #[test]
    fn virtual_pusd_rejects_insufficient_balance() {
        let mut account = VirtualPusdAccount::new(dec!(10));
        let err = account.reserve(dec!(11)).unwrap_err();
        assert!(format!("{}", err).contains("insufficient virtual pUSD"));
    }

    #[test]
    fn virtual_pusd_rejects_over_release() {
        let mut account = VirtualPusdAccount::new(dec!(10));
        account.reserve(dec!(4)).unwrap();

        let err = account.release(dec!(5)).unwrap_err();
        assert!(format!("{}", err).contains("insufficient reserved pUSD"));
        assert_eq!(account.available(), dec!(6));
        assert_eq!(account.reserved(), dec!(4));
    }

    #[test]
    fn virtual_pusd_rejects_negative_fee_and_rebate() {
        let mut account = VirtualPusdAccount::new(dec!(10));
        account.reserve(dec!(4)).unwrap();

        let fee_err = account.fill(dec!(1), dec!(-0.01), dec!(0)).unwrap_err();
        assert!(format!("{}", fee_err).contains("fee must not be negative"));

        let rebate_err = account.fill(dec!(1), dec!(0), dec!(-0.01)).unwrap_err();
        assert!(format!("{}", rebate_err).contains("rebate must not be negative"));
        assert_eq!(account.available(), dec!(6));
        assert_eq!(account.reserved(), dec!(4));
        assert_eq!(account.fees_paid(), dec!(0));
        assert_eq!(account.rebates_earned(), dec!(0));
    }
}
