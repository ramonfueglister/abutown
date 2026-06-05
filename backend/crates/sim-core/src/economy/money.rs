pub const ECONOMY_SCALE: i128 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EconomyError {
    Overflow,
    NegativeMoney,
    NegativeQuantity,
    ZeroPrice,
    InsufficientFunds,
    InsufficientGoods,
    InvalidOrder,
    /// A runtime SFC conservation invariant was violated (money minted/destroyed, or a
    /// net-zero sentinel held stranded cash). UNRECOVERABLE — surfaced fail-fast.
    ConservationViolation,
}

use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct Money(pub i64);

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct Quantity(pub i64);

impl Money {
    pub const ZERO: Money = Money(0);

    pub fn checked_add(self, rhs: Money) -> Result<Money, EconomyError> {
        self.0
            .checked_add(rhs.0)
            .map(Money)
            .ok_or(EconomyError::Overflow)
    }

    pub fn checked_sub(self, rhs: Money) -> Result<Money, EconomyError> {
        self.0
            .checked_sub(rhs.0)
            .map(Money)
            .ok_or(EconomyError::Overflow)
    }
}

impl Quantity {
    pub const ZERO: Quantity = Quantity(0);

    pub fn checked_add(self, rhs: Quantity) -> Result<Quantity, EconomyError> {
        self.0
            .checked_add(rhs.0)
            .map(Quantity)
            .ok_or(EconomyError::Overflow)
    }

    pub fn checked_sub(self, rhs: Quantity) -> Result<Quantity, EconomyError> {
        self.0
            .checked_sub(rhs.0)
            .map(Quantity)
            .ok_or(EconomyError::Overflow)
    }
}

pub fn checked_order_value(price: Money, qty: Quantity) -> Result<Money, EconomyError> {
    if price.0 < 0 {
        return Err(EconomyError::NegativeMoney);
    }
    if price.0 == 0 {
        return Err(EconomyError::ZeroPrice);
    }
    if qty.0 < 0 {
        return Err(EconomyError::NegativeQuantity);
    }
    let raw = (price.0 as i128)
        .checked_mul(qty.0 as i128)
        .ok_or(EconomyError::Overflow)?
        / ECONOMY_SCALE;
    let out = i64::try_from(raw).map_err(|_| EconomyError::Overflow)?;
    Ok(Money(out))
}

pub fn integer_ewma(old: Money, new: Money, alpha_bps: u16) -> Result<Money, EconomyError> {
    if alpha_bps > 10_000 {
        return Err(EconomyError::InvalidOrder);
    }
    let old_weight = 10_000_i128 - i128::from(alpha_bps);
    let new_weight = i128::from(alpha_bps);
    let raw = (old.0 as i128)
        .checked_mul(old_weight)
        .and_then(|old_part| {
            (new.0 as i128)
                .checked_mul(new_weight)
                .and_then(|new_part| old_part.checked_add(new_part))
        })
        .ok_or(EconomyError::Overflow)?
        / 10_000;
    let out = i64::try_from(raw).map_err(|_| EconomyError::Overflow)?;
    Ok(Money(out))
}
