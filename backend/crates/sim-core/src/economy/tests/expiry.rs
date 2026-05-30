use crate::economy::{
    create_ask, create_bid, expire_orders_at_tick, AccountBook, DirtyMarketGoods,
    EconomicActorId, GOOD_FOOD, InventoryBook, MarketGoodKey, MarketId, Money, NextOrderId,
    OrderBook, Quantity, TradeLedger,
};

#[test]
fn expired_bid_releases_remaining_cash() {
    let buyer = EconomicActorId(1);
    let market = MarketId(1);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    accounts.deposit(buyer, Money(10_000)).unwrap();

    let order = create_bid(
        &mut accounts,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        5,
        buyer,
        market,
        GOOD_FOOD,
        Quantity(2_000),
        Money(1_500),
        3,
    )
    .unwrap();

    expire_orders_at_tick(&mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, 8)
        .unwrap();

    assert!(!orders.bids.contains_key(&order));
    assert_eq!(accounts.account(buyer).available, Money(10_000));
    assert_eq!(accounts.account(buyer).locked, Money(0));
    assert!(dirty.0.contains(&MarketGoodKey { market, good: GOOD_FOOD }));
}

#[test]
fn expired_ask_releases_remaining_goods() {
    let seller = EconomicActorId(2);
    let market = MarketId(1);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    inventory.deposit(seller, GOOD_FOOD, Quantity(5_000)).unwrap();

    let order = create_ask(
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        5,
        seller,
        market,
        GOOD_FOOD,
        Quantity(2_000),
        Money(1_500),
        3,
    )
    .unwrap();

    expire_orders_at_tick(&mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, 8)
        .unwrap();

    assert!(!orders.asks.contains_key(&order));
    assert_eq!(inventory.balance(seller, GOOD_FOOD).available, Quantity(5_000));
    assert_eq!(inventory.balance(seller, GOOD_FOOD).locked, Quantity(0));
}

#[test]
fn expired_order_marks_market_good_dirty() {
    let buyer = EconomicActorId(1);
    let market = MarketId(7);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    accounts.deposit(buyer, Money(10_000)).unwrap();

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
        Quantity(1_000),
        Money(1_000),
        1,
    )
    .unwrap();
    dirty.0.clear();

    expire_orders_at_tick(&mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, 2)
        .unwrap();

    assert_eq!(dirty.0.iter().copied().collect::<Vec<_>>(), vec![MarketGoodKey { market, good: GOOD_FOOD }]);
}
