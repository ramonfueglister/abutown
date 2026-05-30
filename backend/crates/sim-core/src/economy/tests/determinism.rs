use crate::economy::{
    build_clearing_plan, Ask, Bid, EconomicActorId, GOOD_FOOD, MarketGoodKey, MarketId, Money,
    OrderId, Quantity,
};

fn bid(id: u64, created_tick: u64) -> Bid {
    Bid {
        id: OrderId(id),
        owner: EconomicActorId(id),
        market: MarketId(1),
        good: GOOD_FOOD,
        qty_remaining: Quantity(1_000),
        max_price: Money(1_500),
        cash_locked_remaining: Money(1_500),
        created_tick,
        expires_tick: 100,
    }
}

fn ask(id: u64, created_tick: u64) -> Ask {
    Ask {
        id: OrderId(id),
        owner: EconomicActorId(id),
        market: MarketId(1),
        good: GOOD_FOOD,
        qty_remaining: Quantity(1_000),
        min_price: Money(1_000),
        goods_locked_remaining: Quantity(1_000),
        created_tick,
        expires_tick: 100,
    }
}

#[test]
fn same_inputs_same_trades() {
    let key = MarketGoodKey { market: MarketId(1), good: GOOD_FOOD };
    let bids = vec![bid(2, 1), bid(1, 1)];
    let asks = vec![ask(4, 1), ask(3, 1)];
    let a = build_clearing_plan(key, &bids, &asks, Money(1_200)).unwrap();
    let b = build_clearing_plan(key, &bids, &asks, Money(1_200)).unwrap();
    assert_eq!(a, b);
}

#[test]
fn tie_break_uses_created_tick_then_order_id() {
    let key = MarketGoodKey { market: MarketId(1), good: GOOD_FOOD };
    let bids = vec![bid(2, 5), bid(1, 5)];
    let asks = vec![ask(4, 5), ask(3, 5)];
    let plan = build_clearing_plan(key, &bids, &asks, Money(1_200)).unwrap();
    assert_eq!(plan.fills[0].bid, OrderId(1));
    assert_eq!(plan.fills[0].ask, OrderId(3));
}
