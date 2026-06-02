//! S2 drain primitives: pure release+shrink+remove on passed-in clones. Conservation
//! is asserted in isolation (total_money/total_good byte-equal; the field-difference
//! bid drain leaves zero orphaned cash in `locked`).

use crate::economy::AccountBook;
use crate::economy::orders::{drain_residual_ask, drain_residual_bid};
use crate::economy::{
    Ask, Bid, EconomicActorId, EconomyError, GoodId, InventoryBook, MarketId, Money, OrderBook,
    OrderId, Quantity, checked_order_value,
};

const GOOD: GoodId = GoodId(1);

fn ask_book(owner: EconomicActorId, qty: i64, locked: i64) -> OrderBook {
    let mut orders = OrderBook::default();
    orders.asks.insert(
        OrderId(1),
        Ask {
            id: OrderId(1),
            owner,
            market: MarketId(10),
            good: GOOD,
            qty_remaining: Quantity(qty),
            min_price: Money(500),
            goods_locked_remaining: Quantity(locked),
            created_tick: 0,
            expires_tick: 100,
        },
    );
    orders
}

fn bid_book(owner: EconomicActorId, qty: i64, max_price: Money, locked: Money) -> OrderBook {
    let mut orders = OrderBook::default();
    orders.bids.insert(
        OrderId(1),
        Bid {
            id: OrderId(1),
            owner,
            market: MarketId(10),
            good: GOOD,
            qty_remaining: Quantity(qty),
            max_price,
            cash_locked_remaining: locked,
            created_tick: 0,
            expires_tick: 100,
        },
    );
    orders
}

#[test]
fn drain_residual_ask_partial_releases_and_shrinks() {
    let owner = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(owner, GOOD, Quantity(100)).unwrap();
    inv.lock_goods(owner, GOOD, Quantity(5)).unwrap(); // available 95, locked 5
    let mut orders = ask_book(owner, 5, 5);
    let total_before = inv.total_good(GOOD).unwrap();

    let removed = drain_residual_ask(&mut orders, &mut inv, OrderId(1), Quantity(2)).unwrap();

    assert!(!removed, "partial drain keeps the order");
    let ask = &orders.asks[&OrderId(1)];
    assert_eq!(ask.qty_remaining, Quantity(3));
    assert_eq!(ask.goods_locked_remaining, Quantity(3)); // 1:1 preserved
    let bal = inv.balance(owner, GOOD);
    assert_eq!(bal.locked, Quantity(3)); // 5 - 2
    assert_eq!(bal.available, Quantity(97)); // 95 + 2
    assert_eq!(
        inv.total_good(GOOD).unwrap(),
        total_before,
        "goods conserved"
    );
}

#[test]
fn drain_residual_ask_full_removes_row_no_orphan() {
    let owner = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(owner, GOOD, Quantity(100)).unwrap();
    inv.lock_goods(owner, GOOD, Quantity(5)).unwrap();
    let mut orders = ask_book(owner, 5, 5);
    let total_before = inv.total_good(GOOD).unwrap();

    let removed = drain_residual_ask(&mut orders, &mut inv, OrderId(1), Quantity(5)).unwrap();

    assert!(removed, "full drain removes the order");
    assert!(orders.asks.is_empty(), "row removed at zero");
    let bal = inv.balance(owner, GOOD);
    assert_eq!(bal.locked, Quantity(0), "no orphaned lock");
    assert_eq!(bal.available, Quantity(100));
    assert_eq!(inv.total_good(GOOD).unwrap(), total_before);
}

#[test]
fn drain_residual_ask_rejects_invalid_q() {
    let owner = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(owner, GOOD, Quantity(100)).unwrap();
    inv.lock_goods(owner, GOOD, Quantity(5)).unwrap();
    let mut orders = ask_book(owner, 5, 5);

    assert_eq!(
        drain_residual_ask(&mut orders, &mut inv, OrderId(1), Quantity(6)),
        Err(EconomyError::InvalidOrder),
        "q greater than qty_remaining"
    );
    assert_eq!(
        drain_residual_ask(&mut orders, &mut inv, OrderId(1), Quantity(0)),
        Err(EconomyError::InvalidOrder),
        "non-positive q"
    );
    assert_eq!(
        drain_residual_ask(&mut orders, &mut inv, OrderId(999), Quantity(1)),
        Err(EconomyError::InvalidOrder),
        "missing order id"
    );
    // The failed calls must not have mutated anything.
    assert_eq!(orders.asks[&OrderId(1)].qty_remaining, Quantity(5));
    assert_eq!(inv.balance(owner, GOOD).locked, Quantity(5));
}

#[test]
fn drain_residual_bid_partial_releases_field_difference() {
    // max_price 1000, qty 5 -> lock = 1000*5/1000 = Money(5).
    let owner = EconomicActorId(1);
    let max_price = Money(1_000);
    let lock = checked_order_value(max_price, Quantity(5)).unwrap();
    assert_eq!(lock, Money(5));
    let mut acc = AccountBook::default();
    acc.deposit(owner, Money(100)).unwrap();
    acc.lock_cash(owner, lock).unwrap(); // available 95, locked 5
    let mut orders = bid_book(owner, 5, max_price, lock);
    let total_before = acc.total_money().unwrap();

    let removed = drain_residual_bid(&mut orders, &mut acc, OrderId(1), Quantity(2)).unwrap();

    assert!(!removed);
    let bid = &orders.bids[&OrderId(1)];
    assert_eq!(bid.qty_remaining, Quantity(3));
    // new lock = checked_order_value(1000, 3) = Money(3); released = 5 - 3 = 2.
    assert_eq!(bid.cash_locked_remaining, Money(3));
    let a = acc.account(owner);
    assert_eq!(a.locked, Money(3));
    assert_eq!(a.available, Money(97)); // 95 + 2
    assert_eq!(acc.total_money().unwrap(), total_before, "money conserved");
}

#[test]
fn drain_residual_bid_fractional_full_drain_zero_orphan() {
    // FRACTIONAL price: max_price 1500, qty 3 -> lock = 1500*3/1000 = floor(4.5) = Money(4).
    // A per-q recompute (value(1500,1)*3 = 1*3 = 3) would strand Money(1) in `locked`.
    // The field-difference rule (release the FULL field at new_qty==0) leaves zero orphan.
    let owner = EconomicActorId(1);
    let max_price = Money(1_500);
    let lock = checked_order_value(max_price, Quantity(3)).unwrap();
    assert_eq!(lock, Money(4));
    let mut acc = AccountBook::default();
    acc.deposit(owner, Money(100)).unwrap();
    acc.lock_cash(owner, lock).unwrap(); // available 96, locked 4
    let mut orders = bid_book(owner, 3, max_price, lock);
    let total_before = acc.total_money().unwrap();

    let removed = drain_residual_bid(&mut orders, &mut acc, OrderId(1), Quantity(3)).unwrap();

    assert!(removed, "full drain removes the order");
    assert!(orders.bids.is_empty(), "row removed at zero");
    let a = acc.account(owner);
    assert_eq!(
        a.locked,
        Money(0),
        "ZERO orphan — full field released, no floor-drift residue"
    );
    assert_eq!(
        a.available,
        Money(100),
        "all locked cash returned to available"
    );
    assert_eq!(acc.total_money().unwrap(), total_before, "money conserved");
}

#[test]
fn drain_residual_bid_rejects_invalid_q() {
    let owner = EconomicActorId(1);
    let max_price = Money(1_000);
    let lock = checked_order_value(max_price, Quantity(5)).unwrap();
    let mut acc = AccountBook::default();
    acc.deposit(owner, Money(100)).unwrap();
    acc.lock_cash(owner, lock).unwrap();
    let mut orders = bid_book(owner, 5, max_price, lock);

    assert_eq!(
        drain_residual_bid(&mut orders, &mut acc, OrderId(1), Quantity(6)),
        Err(EconomyError::InvalidOrder)
    );
    assert_eq!(
        drain_residual_bid(&mut orders, &mut acc, OrderId(1), Quantity(0)),
        Err(EconomyError::InvalidOrder)
    );
    assert_eq!(
        drain_residual_bid(&mut orders, &mut acc, OrderId(999), Quantity(1)),
        Err(EconomyError::InvalidOrder)
    );
    assert_eq!(orders.bids[&OrderId(1)].qty_remaining, Quantity(5));
    assert_eq!(acc.account(owner).locked, lock);
}
