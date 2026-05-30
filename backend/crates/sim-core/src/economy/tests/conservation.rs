use crate::economy::{
    AccountBook, DirtyMarketGoods, EconomicActorId, GOOD_FOOD, InventoryBook, MarketGoodKey,
    MarketGoodState, MarketGoods, MarketId, Money, NextOrderId, OrderBook, Quantity, TradeLedger,
    clear_market_good, create_ask, create_bid,
};

fn seeded_market_state(market: MarketId) -> MarketGoodState {
    MarketGoodState {
        key: MarketGoodKey {
            market,
            good: GOOD_FOOD,
        },
        last_settlement_price: Money(1_100),
        ewma_reference_price: Money(1_100),
        traded_qty_last_tick: Quantity(0),
        unmet_demand_last_tick: Quantity(0),
        unsold_supply_last_tick: Quantity(0),
        dirty: true,
        last_cleared_tick: 0,
    }
}

#[test]
fn auction_conserves_total_money() {
    let buyer = EconomicActorId(1);
    let seller = EconomicActorId(2);
    let market = MarketId(1);
    let key = MarketGoodKey {
        market,
        good: GOOD_FOOD,
    };
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut goods = MarketGoods::default();
    goods.0.insert(key, seeded_market_state(market));
    accounts.deposit(buyer, Money(10_000)).unwrap();
    inventory
        .deposit(seller, GOOD_FOOD, Quantity(2_000))
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
        Quantity(1_000),
        Money(1_500),
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
        Quantity(1_000),
        Money(1_000),
        10,
    )
    .unwrap();
    let before = accounts.total_money().unwrap();

    clear_market_good(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut goods,
        key,
        2,
    )
    .unwrap();

    assert_eq!(accounts.total_money().unwrap(), before);
}

#[test]
fn auction_conserves_total_goods() {
    let buyer = EconomicActorId(1);
    let seller = EconomicActorId(2);
    let market = MarketId(1);
    let key = MarketGoodKey {
        market,
        good: GOOD_FOOD,
    };
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut goods = MarketGoods::default();
    goods.0.insert(key, seeded_market_state(market));
    accounts.deposit(buyer, Money(10_000)).unwrap();
    inventory
        .deposit(seller, GOOD_FOOD, Quantity(2_000))
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
        Quantity(1_000),
        Money(1_500),
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
        Quantity(1_000),
        Money(1_000),
        10,
    )
    .unwrap();
    let before = inventory.total_good(GOOD_FOOD).unwrap();

    clear_market_good(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut goods,
        key,
        2,
    )
    .unwrap();

    assert_eq!(inventory.total_good(GOOD_FOOD).unwrap(), before);
}

#[test]
fn successful_bid_refunds_locked_surplus() {
    let buyer = EconomicActorId(1);
    let seller = EconomicActorId(2);
    let market = MarketId(1);
    let key = MarketGoodKey {
        market,
        good: GOOD_FOOD,
    };
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut goods = MarketGoods::default();
    goods.0.insert(key, seeded_market_state(market));
    accounts.deposit(buyer, Money(10_000)).unwrap();
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
        Quantity(1_000),
        Money(1_500),
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
        Quantity(1_000),
        Money(1_000),
        10,
    )
    .unwrap();

    clear_market_good(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut goods,
        key,
        2,
    )
    .unwrap();

    assert_eq!(accounts.account(buyer).locked, Money(0));
    assert_eq!(accounts.account(buyer).available, Money(8_900));
    assert_eq!(accounts.account(seller).available, Money(1_100));
}

#[test]
fn partial_fill_conserves_money_and_goods() {
    let buyer = EconomicActorId(1);
    let seller = EconomicActorId(2);
    let market = MarketId(1);
    let key = MarketGoodKey {
        market,
        good: GOOD_FOOD,
    };
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut goods = MarketGoods::default();
    goods.0.insert(key, seeded_market_state(market));
    accounts.deposit(buyer, Money(10_000)).unwrap();
    inventory.deposit(seller, GOOD_FOOD, Quantity(500)).unwrap();
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
        Money(1_500),
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
        Quantity(500),
        Money(1_000),
        10,
    )
    .unwrap();
    let before_money = accounts.total_money().unwrap();
    let before_goods = inventory.total_good(GOOD_FOOD).unwrap();

    clear_market_good(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut goods,
        key,
        2,
    )
    .unwrap();

    assert_eq!(accounts.total_money().unwrap(), before_money);
    assert_eq!(inventory.total_good(GOOD_FOOD).unwrap(), before_goods);
    assert_eq!(orders.bids.len(), 1);
    assert_eq!(
        orders.bids.values().next().unwrap().qty_remaining,
        Quantity(500)
    );
}
