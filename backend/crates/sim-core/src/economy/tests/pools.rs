use crate::economy::{
    AccountBook, DemandPool, DemandPools, DirtyMarketGoods, EconomicActorId, EconomyEvent,
    GOOD_FOOD, InventoryBook, MarketId, Money, NextOrderId, OrderBook, Quantity, SupplyPool,
    SupplyPools, TradeLedger, generate_pool_orders_at_tick,
};

#[test]
fn demand_pool_caps_order_to_affordable_quantity() {
    let actor = EconomicActorId(1);
    let market = MarketId(1);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();
    accounts.deposit(actor, Money(1_500)).unwrap();
    demand.0.insert(
        actor,
        DemandPool {
            actor,
            market,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(5_000),
            max_price: Money(1_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );

    generate_pool_orders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut demand,
        &mut supply,
        10,
        5,
    )
    .unwrap();

    let bid = orders.bids.values().next().unwrap();
    assert_eq!(bid.qty_remaining, Quantity(1_500));
    assert_eq!(accounts.account(actor).locked, Money(1_500));
}

#[test]
fn supply_pool_caps_order_to_available_inventory() {
    let actor = EconomicActorId(2);
    let market = MarketId(1);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();
    inventory
        .deposit(actor, GOOD_FOOD, Quantity(1_500))
        .unwrap();
    supply.0.insert(
        actor,
        SupplyPool {
            actor,
            market,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(5_000),
            min_price: Money(1_000),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );

    generate_pool_orders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut demand,
        &mut supply,
        10,
        5,
    )
    .unwrap();

    let ask = orders.asks.values().next().unwrap();
    assert_eq!(ask.qty_remaining, Quantity(1_500));
    assert_eq!(inventory.balance(actor, GOOD_FOOD).locked, Quantity(1_500));
}

#[test]
fn rejected_pool_order_leaves_books_unchanged() {
    let actor = EconomicActorId(1);
    let market = MarketId(1);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();
    demand.0.insert(
        actor,
        DemandPool {
            actor,
            market,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(1_000),
            max_price: Money(1_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );

    generate_pool_orders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut demand,
        &mut supply,
        1,
        5,
    )
    .unwrap();

    assert!(orders.bids.is_empty());
    assert_eq!(accounts.account(actor).available, Money(0));
    assert!(
        matches!(ledger.0.last(), Some(EconomyEvent::OrderRejected { actor: rejected, .. }) if *rejected == actor)
    );
}
