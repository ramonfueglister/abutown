use crate::economy::{
    ECONOMY_SCALE, EconomyError, Money, Quantity, checked_order_value, integer_ewma,
};

#[test]
fn price_quantity_scale_computes_order_value() {
    assert_eq!(ECONOMY_SCALE, 1_000);
    assert_eq!(
        checked_order_value(Money(2_500), Quantity(3_000)),
        Ok(Money(7_500))
    );
}

#[test]
fn max_price_times_qty_overflow_returns_error() {
    assert_eq!(
        checked_order_value(Money(i64::MAX), Quantity(i64::MAX)),
        Err(EconomyError::Overflow)
    );
}

#[test]
fn money_add_overflow_returns_error() {
    assert_eq!(
        Money(i64::MAX).checked_add(Money(1)),
        Err(EconomyError::Overflow)
    );
}

#[test]
fn quantity_add_overflow_returns_error() {
    assert_eq!(
        Quantity(i64::MAX).checked_add(Quantity(1)),
        Err(EconomyError::Overflow)
    );
}

#[test]
fn negative_quantity_is_rejected() {
    assert_eq!(
        checked_order_value(Money(1_000), Quantity(-1)),
        Err(EconomyError::NegativeQuantity)
    );
}

#[test]
fn integer_ewma_uses_basis_points_without_float() {
    assert_eq!(
        integer_ewma(Money(1_000), Money(2_000), 2_500),
        Ok(Money(1_250))
    );
}
