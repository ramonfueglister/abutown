use std::collections::BTreeSet;

use crate::economy::{
    AccountBook, DemandPool, DemandPools, DirtyMarketGoods, EconomicActorId, EconomyEvent,
    GOOD_FOOD, InventoryBook, MarketGoodKey, MarketGoodState, MarketGoods, MarketId, Money,
    NextOrderId, OrderBook, Quantity, SupplyPool, SupplyPools, TradeLedger,
    generate_pool_orders_at_tick, run_consumption_at_tick,
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
            last_consumed_tick: None,
            income_last_tick: Money::ZERO,
            mpc_bps: 8_000,
            autonomous: Money(5_000),
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
        &BTreeSet::new(),
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
        &BTreeSet::new(),
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
            last_consumed_tick: None,
            income_last_tick: Money::ZERO,
            mpc_bps: 8_000,
            autonomous: Money(5_000),
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
        &BTreeSet::new(),
    )
    .unwrap();

    assert!(orders.bids.is_empty());
    assert_eq!(accounts.account(actor).available, Money(0));
    assert!(
        matches!(ledger.0.last(), Some(EconomyEvent::OrderRejected { actor: rejected, .. }) if *rejected == actor)
    );
}

fn consume_pool(actor: u64, market: u32, want: i64) -> DemandPool {
    DemandPool {
        actor: EconomicActorId(actor),
        market: MarketId(market),
        good: GOOD_FOOD,
        desired_qty_per_tick: Quantity(want),
        max_price: Money(1_000),
        urgency_bps: 0,
        elasticity_bps: 0,
        interval_ticks: 1,
        last_generated_tick: None,
        last_consumed_tick: None,
        income_last_tick: Money::ZERO,
        mpc_bps: 8_000,
        autonomous: Money(5_000),
    }
}

#[test]
fn consumption_removes_min_held_want_and_emits_finalconsumed() {
    let owner = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(owner, GOOD_FOOD, Quantity(4)).unwrap(); // held 4 < want 10
    let mut ledger = TradeLedger::default();
    let mut demand = DemandPools::default();
    demand.0.insert(owner, consume_pool(1, 10, 10));
    let good_before = inv.total_good(GOOD_FOOD).unwrap();
    let mut mg = MarketGoods::default();

    run_consumption_at_tick(&mut inv, &mut ledger, &mut demand, &mut mg, 0).unwrap();

    assert_eq!(inv.balance(owner, GOOD_FOOD).available, Quantity(0));
    assert_eq!(
        inv.total_good(GOOD_FOOD).unwrap().0,
        good_before.0 - 4,
        "goods removed by exactly consumed (clamped to held)"
    );
    assert!(
        ledger.0.iter().any(|e| matches!(e,
            EconomyEvent::FinalConsumed { actor, qty, .. }
            if *actor == owner && *qty == Quantity(4))),
        "a clamped FinalConsumed(4) event is pushed"
    );
    assert_eq!(
        demand.0[&owner].last_consumed_tick,
        Some(0),
        "cursor advanced"
    );
}

#[test]
fn consumption_conserves_money_and_is_deterministic() {
    let build = || {
        let mut inv = InventoryBook::default();
        for a in [1_u64, 2] {
            inv.deposit(EconomicActorId(a), GOOD_FOOD, Quantity(10))
                .unwrap();
        }
        let mut acc = AccountBook::default();
        acc.deposit(EconomicActorId(1), Money(500)).unwrap();
        let mut ledger = TradeLedger::default();
        let mut demand = DemandPools::default();
        demand.0.insert(EconomicActorId(1), consume_pool(1, 10, 3));
        demand.0.insert(EconomicActorId(2), consume_pool(2, 11, 5));
        let mut mg = MarketGoods::default();
        let m0 = acc.total_money().unwrap();
        run_consumption_at_tick(&mut inv, &mut ledger, &mut demand, &mut mg, 0).unwrap();
        assert_eq!(
            acc.total_money().unwrap(),
            m0,
            "money invariant across consume"
        );
        ledger.0
    };
    assert_eq!(build(), build(), "consumption is deterministic");
}

#[test]
fn consumption_respects_interval_cursor() {
    let owner = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(owner, GOOD_FOOD, Quantity(100)).unwrap();
    let mut ledger = TradeLedger::default();
    let mut demand = DemandPools::default();
    let mut p = consume_pool(1, 10, 4);
    p.interval_ticks = 5;
    demand.0.insert(owner, p);
    let mut mg = MarketGoods::default();

    run_consumption_at_tick(&mut inv, &mut ledger, &mut demand, &mut mg, 0).unwrap(); // cursor None -> consumes 4
    run_consumption_at_tick(&mut inv, &mut ledger, &mut demand, &mut mg, 2).unwrap(); // 2 < interval 5 -> skip
    assert_eq!(
        inv.balance(owner, GOOD_FOOD).available,
        Quantity(96),
        "only one interval consumed"
    );
    run_consumption_at_tick(&mut inv, &mut ledger, &mut demand, &mut mg, 5).unwrap(); // elapsed -> consumes 4
    assert_eq!(inv.balance(owner, GOOD_FOOD).available, Quantity(92));
}

#[test]
fn consumption_attributes_to_market_and_resets_stale() {
    let owner = EconomicActorId(1);
    let m = MarketId(7);
    let mut inv = InventoryBook::default();
    inv.deposit(owner, GOOD_FOOD, Quantity(10)).unwrap();
    let mut ledger = TradeLedger::default();
    let mut demand = DemandPools::default();
    demand.0.insert(owner, consume_pool(1, 7, 4));

    // A stale market-good carrying last tick's consumption MUST be reset (the sink is the
    // sole writer; reset-all-then-accumulate avoids phantom carry-over).
    let mut mg = MarketGoods::default();
    let stale_key = MarketGoodKey {
        market: MarketId(99),
        good: GOOD_FOOD,
    };
    let mut stale = MarketGoodState::new(stale_key);
    stale.consumed_qty_last_tick = Quantity(123);
    mg.0.insert(stale_key, stale);

    run_consumption_at_tick(&mut inv, &mut ledger, &mut demand, &mut mg, 0).unwrap();

    let key = MarketGoodKey {
        market: m,
        good: GOOD_FOOD,
    };
    assert_eq!(
        mg.0[&key].consumed_qty_last_tick,
        Quantity(4),
        "consumption attributed to its (market, good)"
    );
    assert_eq!(
        mg.0[&stale_key].consumed_qty_last_tick,
        Quantity(0),
        "stale market-good's consumed_qty reset to 0"
    );
}
