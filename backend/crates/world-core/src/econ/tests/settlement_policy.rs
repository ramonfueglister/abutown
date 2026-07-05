use crate::econ::{
    AccountBook, Ask, Bid, DirtyMarketGoods, EconomicActorId, GOOD_FOOD, InventoryBook,
    MarketGoodKey, MarketGoodState, MarketGoods, MarketId, Money, NextOrderId, OrderBook, OrderId,
    Quantity, SettlementPolicy, TradeLedger, build_clearing_plan, build_clearing_plan_with_policy,
    clear_market_good_with_policy, create_ask, create_bid, settlement_price,
    settlement_price_with_policy,
};

fn key() -> MarketGoodKey {
    MarketGoodKey {
        market: MarketId(1),
        good: GOOD_FOOD,
    }
}

fn sbid(id: u64, max_price: Money, qty: Quantity) -> Bid {
    Bid {
        id: OrderId(id),
        owner: EconomicActorId(10 + id),
        market: MarketId(1),
        good: GOOD_FOOD,
        qty_remaining: qty,
        max_price,
        cash_locked_remaining: Money(max_price.0 * qty.0 / 1_000),
        created_tick: 1,
        expires_tick: 100,
    }
}
fn sask(id: u64, min_price: Money, qty: Quantity) -> Ask {
    Ask {
        id: OrderId(id),
        owner: EconomicActorId(20 + id),
        market: MarketId(1),
        good: GOOD_FOOD,
        qty_remaining: qty,
        min_price,
        goods_locked_remaining: qty,
        created_tick: 1,
        expires_tick: 100,
    }
}

#[test]
fn settlement_price_with_policy_midpoint_and_anchored() {
    // Midpoint: (bid + ask) / 2, floored.
    assert_eq!(
        settlement_price_with_policy(
            Money(1_050),
            Money(1_200),
            Money(1_000),
            SettlementPolicy::Midpoint
        ),
        Money(1_100)
    );
    assert_eq!(
        settlement_price_with_policy(
            Money(0),
            Money(1_001),
            Money(1_000),
            SettlementPolicy::Midpoint
        ),
        Money(1_000) // (1001+1000)/2 = 1000 floored
    );
    // Anchored matches the existing settlement_price exactly.
    for last in [Money(500), Money(1_050), Money(1_500)] {
        assert_eq!(
            settlement_price_with_policy(
                last,
                Money(1_200),
                Money(1_000),
                SettlementPolicy::Anchored
            ),
            settlement_price(last, Money(1_200), Money(1_000))
        );
    }
}

#[test]
fn build_clearing_plan_policy_changes_only_the_price() {
    let bids = [sbid(1, Money(1_200), Quantity(10))];
    let asks = [sask(2, Money(1_000), Quantity(10))];

    // last=1050 is within [1000,1200]: anchored keeps 1050, midpoint -> 1100.
    let anchored = build_clearing_plan_with_policy(
        key(),
        &bids,
        &asks,
        Money(1_050),
        SettlementPolicy::Anchored,
    )
    .unwrap();
    let midpoint = build_clearing_plan_with_policy(
        key(),
        &bids,
        &asks,
        Money(1_050),
        SettlementPolicy::Midpoint,
    )
    .unwrap();
    let default = build_clearing_plan(key(), &bids, &asks, Money(1_050)).unwrap();

    assert_eq!(anchored.settlement_price, Some(Money(1_050)));
    assert_eq!(midpoint.settlement_price, Some(Money(1_100)));
    assert_eq!(
        default.settlement_price, anchored.settlement_price,
        "default == anchored"
    );
    // Quantity/fills identical regardless of policy.
    assert_eq!(anchored.fills, midpoint.fills);
}

#[test]
fn clear_market_good_with_midpoint_conserves_and_settles_at_midpoint() {
    let buyer = EconomicActorId(1);
    let seller = EconomicActorId(2);
    let market = MarketId(1);
    let k = key();
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut goods = MarketGoods::default();
    let mut st = MarketGoodState::new(k);
    st.last_settlement_price = Money(1_050);
    st.dirty = true;
    goods.0.insert(k, st);

    accounts.deposit(buyer, Money(100_000)).unwrap();
    inventory
        .deposit(seller, GOOD_FOOD, Quantity(1_000))
        .unwrap();
    create_bid(
        &mut accounts,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        1,
        buyer,
        market,
        GOOD_FOOD,
        Quantity(10),
        Money(1_200),
        10,
    )
    .unwrap();
    create_ask(
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        1,
        seller,
        market,
        GOOD_FOOD,
        Quantity(10),
        Money(1_000),
        10,
    )
    .unwrap();

    let m0 = accounts.total_money().unwrap();
    let g0 = inventory.total_good(GOOD_FOOD).unwrap();

    clear_market_good_with_policy(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut goods,
        k,
        2,
        SettlementPolicy::Midpoint,
    )
    .unwrap();

    assert_eq!(
        goods.0.get(&k).unwrap().last_settlement_price,
        Money(1_100),
        "settled at midpoint"
    );
    assert_eq!(accounts.total_money().unwrap(), m0, "money conserved");
    assert_eq!(
        inventory.total_good(GOOD_FOOD).unwrap(),
        g0,
        "goods conserved"
    );
    assert_eq!(inventory.balance(buyer, GOOD_FOOD).available, Quantity(10));
}
