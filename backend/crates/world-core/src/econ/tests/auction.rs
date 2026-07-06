use crate::econ::{
    Ask, Bid, EconomicActorId, GOOD_FOOD, MarketGoodKey, MarketId, Money, OrderId, Quantity,
    build_clearing_plan, settlement_price,
};

fn bid(id: u64, max_price: Money, qty: Quantity, created_tick: u64) -> Bid {
    Bid {
        id: OrderId(id),
        owner: EconomicActorId(10 + id),
        market: MarketId(1),
        good: GOOD_FOOD,
        qty_remaining: qty,
        max_price,
        cash_locked_remaining: Money(max_price.0 * qty.0 / 1_000),
        created_tick,
        expires_tick: 100,
    }
}

fn ask(id: u64, min_price: Money, qty: Quantity, created_tick: u64) -> Ask {
    Ask {
        id: OrderId(id),
        owner: EconomicActorId(20 + id),
        market: MarketId(1),
        good: GOOD_FOOD,
        qty_remaining: qty,
        min_price,
        goods_locked_remaining: qty,
        created_tick,
        expires_tick: 100,
    }
}

#[test]
fn no_trade_without_price_overlap() {
    let plan = build_clearing_plan(
        MarketGoodKey {
            market: MarketId(1),
            good: GOOD_FOOD,
        },
        &[bid(1, Money(800), Quantity(1_000), 1)],
        &[ask(2, Money(1_000), Quantity(1_000), 1)],
        Money(900),
    )
    .unwrap();
    assert!(plan.fills.is_empty());
}

#[test]
fn trade_happens_with_price_overlap() {
    let plan = build_clearing_plan(
        MarketGoodKey {
            market: MarketId(1),
            good: GOOD_FOOD,
        },
        &[bid(1, Money(1_200), Quantity(1_000), 1)],
        &[ask(2, Money(1_000), Quantity(1_000), 1)],
        Money(1_100),
    )
    .unwrap();
    assert_eq!(plan.fills.len(), 1);
    assert_eq!(plan.settlement_price, Some(Money(1_100)));
}

#[test]
fn settlement_price_is_within_bid_ask_bounds() {
    assert_eq!(
        settlement_price(Money(500), Money(1_200), Money(1_000)),
        Money(1_000)
    );
    assert_eq!(
        settlement_price(Money(1_500), Money(1_200), Money(1_000)),
        Money(1_200)
    );
    assert_eq!(
        settlement_price(Money(1_100), Money(1_200), Money(1_000)),
        Money(1_100)
    );
}
