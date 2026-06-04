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
        consumed_qty_last_tick: Quantity::ZERO,
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

#[test]
fn conservation_full_plugin_multi_tick() {
    use crate::economy::production::{
        EXTRACTOR_FOOD_A, EXTRACTOR_TOOLS, ProductionPool, ProductionPools, RawDeposit,
        RawDeposits, Recipe,
    };
    use crate::economy::{
        AccountBook, DemandPool, DemandPools, EconomicActorId, EconomyEvent, EconomyPlugin,
        GOOD_FOOD, GOOD_RAW, GOOD_TOOLS, GoodId, HouseholdSector, InventoryBook, MarketGoodKey,
        MarketGoodState, MarketGoods, MarketId, MarketSite, Markets, Money, Quantity, SupplyPool,
        SupplyPools, TradeLedger,
    };
    use crate::mobility::resources::Tick;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    use bevy_ecs::prelude::*;
    use std::collections::BTreeMap;

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let consumer = EconomicActorId(8_002);
    let market = MarketId(1);

    world.resource_mut::<RawDeposits>().0.insert(
        EXTRACTOR_TOOLS,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(10),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        EXTRACTOR_TOOLS,
        ProductionPool {
            actor: EXTRACTOR_TOOLS,
            recipe: Recipe {
                inputs: vec![(GOOD_RAW, Quantity(10))],
                outputs: vec![(GOOD_TOOLS, Quantity(10))],
            },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<SupplyPools>().0.insert(
        EXTRACTOR_TOOLS,
        SupplyPool {
            actor: EXTRACTOR_TOOLS,
            market,
            good: GOOD_TOOLS,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world
        .resource_mut::<AccountBook>()
        .deposit(consumer, Money(10_000_000))
        .unwrap();
    world.resource_mut::<DemandPools>().0.insert(
        consumer,
        DemandPool {
            actor: consumer,
            market,
            good: GOOD_TOOLS,
            desired_qty_per_tick: Quantity(10),
            max_price: Money(2_000),
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
    // FOOD self-sufficiency: a parallel FOOD extractor + FOOD consumer at the SAME market
    // (single-market auction), proving FOOD conserves and flows exactly like TOOLS.
    let food_consumer = EconomicActorId(8_012);
    world.resource_mut::<RawDeposits>().0.insert(
        EXTRACTOR_FOOD_A,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(10),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        EXTRACTOR_FOOD_A,
        ProductionPool {
            actor: EXTRACTOR_FOOD_A,
            recipe: Recipe {
                inputs: vec![(GOOD_RAW, Quantity(10))],
                outputs: vec![(GOOD_FOOD, Quantity(10))],
            },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<SupplyPools>().0.insert(
        EXTRACTOR_FOOD_A,
        SupplyPool {
            actor: EXTRACTOR_FOOD_A,
            market,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world
        .resource_mut::<AccountBook>()
        .deposit(food_consumer, Money(10_000_000))
        .unwrap();
    world.resource_mut::<DemandPools>().0.insert(
        food_consumer,
        DemandPool {
            actor: food_consumer,
            market,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(10),
            max_price: Money(2_000),
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
    world.resource_mut::<Markets>().0.insert(
        market,
        MarketSite {
            id: market,
            node_id: crate::routing::NodeId(0),
            name: "M1".to_string(),
        },
    );
    world.insert_resource(HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(consumer, 1_i64), (food_consumer, 1_i64)]),
    });
    {
        let key = MarketGoodKey {
            market,
            good: GOOD_TOOLS,
        };
        let mut goods = world.resource_mut::<MarketGoods>();
        let st = goods
            .0
            .entry(key)
            .or_insert_with(|| MarketGoodState::new(key));
        st.ewma_reference_price = Money(1_000);
        st.last_settlement_price = Money(1_000);
    }
    {
        let key = MarketGoodKey {
            market,
            good: GOOD_FOOD,
        };
        let mut goods = world.resource_mut::<MarketGoods>();
        let st = goods
            .0
            .entry(key)
            .or_insert_with(|| MarketGoodState::new(key));
        st.ewma_reference_price = Money(1_000);
        st.last_settlement_price = Money(1_000);
    }

    let goods_to_track = [GOOD_RAW, GOOD_TOOLS, GOOD_FOOD];
    let money_before = world.resource::<AccountBook>().total_money().unwrap();
    let mut good_before: BTreeMap<GoodId, i64> = BTreeMap::new();
    for g in goods_to_track {
        good_before.insert(
            g,
            world.resource::<InventoryBook>().total_good(g).unwrap().0,
        );
    }

    let mut net_ledger: BTreeMap<GoodId, i64> = BTreeMap::new();
    let mut last_seen = 0usize;
    let n = 60u64;
    for _ in 0..n {
        schedule.run(&mut world);
        world.resource_mut::<Tick>().0 += 1;
        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            money_before,
            "total_money byte-invariant every tick"
        );
        let ledger = world.resource::<TradeLedger>();
        for e in &ledger.0[last_seen..] {
            match e {
                EconomyEvent::Regenerated { good, qty, .. } => {
                    *net_ledger.entry(*good).or_insert(0) += qty.0
                }
                EconomyEvent::Produced { good, qty, .. } => {
                    *net_ledger.entry(*good).or_insert(0) += qty.0
                }
                EconomyEvent::Consumed { good, qty, .. } => {
                    *net_ledger.entry(*good).or_insert(0) -= qty.0
                }
                EconomyEvent::FinalConsumed { good, qty, .. } => {
                    *net_ledger.entry(*good).or_insert(0) -= qty.0
                }
                _ => {}
            }
        }
        last_seen = ledger.0.len();
    }

    for g in goods_to_track {
        let after = world.resource::<InventoryBook>().total_good(g).unwrap().0;
        let delta = after - good_before[&g];
        let from_events = *net_ledger.get(&g).unwrap_or(&0);
        assert_eq!(
            delta, from_events,
            "per-good balance for {g:?}: on-hand delta == Σ(Regen+Produced) − Σ(Consumed+FinalConsumed)"
        );
    }

    // Per-actor RAW balance: with two extractors sharing GOOD_RAW, each must have regenerated
    // exactly as much RAW as it consumed (no shared-RAW double-spend / cross-actor leak).
    for actor in [EXTRACTOR_TOOLS, EXTRACTOR_FOOD_A] {
        let regen: i64 = world
            .resource::<TradeLedger>()
            .0
            .iter()
            .filter_map(|e| match e {
                EconomyEvent::Regenerated {
                    actor: a,
                    good,
                    qty,
                } if *a == actor && *good == GOOD_RAW => Some(qty.0),
                _ => None,
            })
            .sum();
        let consumed: i64 = world
            .resource::<TradeLedger>()
            .0
            .iter()
            .filter_map(|e| match e {
                EconomyEvent::Consumed {
                    actor: a,
                    good,
                    qty,
                } if *a == actor && *good == GOOD_RAW => Some(qty.0),
                _ => None,
            })
            .sum();
        assert_eq!(
            regen, consumed,
            "actor {actor:?}: RAW regenerated == consumed over the run"
        );
    }
    // FOOD flowed (non-vacuity for the new good).
    assert!(
        world
            .resource::<TradeLedger>()
            .0
            .iter()
            .any(|e| matches!(e, EconomyEvent::Produced { good, .. } if *good == GOOD_FOOD)),
        "FOOD was produced"
    );

    // Non-vacuity: at least some ledger events must have been emitted (goods flowed).
    // Net-ledger values balance to 0 in a working system (Regenerated+Consumed cancel,
    // Produced+FinalConsumed cancel), so we check that ANY events were recorded, not
    // that the net is non-zero.
    assert!(
        last_seen > 0,
        "the conservation test must be non-vacuous (goods flowed)"
    );
    assert!(
        world
            .resource::<TradeLedger>()
            .0
            .iter()
            .any(|e| matches!(e, EconomyEvent::Regenerated { .. })),
        "EXTRACTOR_TOOLS regenerated RAW"
    );
    assert!(
        world
            .resource::<TradeLedger>()
            .0
            .iter()
            .any(|e| matches!(e, EconomyEvent::Produced { good, .. } if *good == GOOD_TOOLS)),
        "the recipe produced TOOLS"
    );
}

#[test]
fn steady_state_multi_tick() {
    use crate::economy::production::{
        EXTRACTOR_FOOD_A, EXTRACTOR_TOOLS, ProductionPool, ProductionPools, RawDeposit,
        RawDeposits, Recipe,
    };
    use crate::economy::{
        AccountBook, DemandPool, DemandPools, EconomicActorId, EconomyEvent, EconomyPlugin,
        GOOD_FOOD, GOOD_RAW, GOOD_TOOLS, HouseholdSector, InventoryBook, MarketGoodKey,
        MarketGoodState, MarketGoods, MarketId, MarketSite, Markets, Money, Quantity, SupplyPool,
        SupplyPools, TradeLedger,
    };
    use crate::mobility::resources::Tick;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    use bevy_ecs::prelude::*;
    use std::collections::BTreeMap;

    // §8 CAVEATS (also carried into the PR body):
    //  - Capped-price regulator: seed prices are static opening values; the auction EWMA
    //    smooths but does NOT self-correct chronic scarcity. The §15.2 Sizing-Sim (A5a)
    //    sized the faucet to cover aggregate demand precisely so scarcity does not arise.
    //  - Autonomous floor: each consumer pool has a non-zero `autonomous` demand term, so
    //    consumption never collapses to zero even at a transient income dip — this is what
    //    keeps the steady state "living" (lo > 0) rather than freezing.

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let consumer = EconomicActorId(8_002);
    let market = MarketId(1);

    // EXTRACTOR_TOOLS is the ONLY supplier (no finite 1M endowment to mask steady state).
    world.resource_mut::<RawDeposits>().0.insert(
        EXTRACTOR_TOOLS,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(10),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        EXTRACTOR_TOOLS,
        ProductionPool {
            actor: EXTRACTOR_TOOLS,
            recipe: Recipe {
                inputs: vec![(GOOD_RAW, Quantity(10))],
                outputs: vec![(GOOD_TOOLS, Quantity(10))],
            },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<SupplyPools>().0.insert(
        EXTRACTOR_TOOLS,
        SupplyPool {
            actor: EXTRACTOR_TOOLS,
            market,
            good: GOOD_TOOLS,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    // Consumer cash: ample but FINITE — the loop must recycle, not just drain a hoard.
    world
        .resource_mut::<AccountBook>()
        .deposit(consumer, Money(1_000_000))
        .unwrap();
    world.resource_mut::<DemandPools>().0.insert(
        consumer,
        DemandPool {
            actor: consumer,
            market,
            good: GOOD_TOOLS,
            desired_qty_per_tick: Quantity(0), // bootstrapped from autonomous at tick 0
            max_price: Money(2_000),
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
    let food_consumer = EconomicActorId(8_012);
    world.resource_mut::<RawDeposits>().0.insert(
        EXTRACTOR_FOOD_A,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(10),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        EXTRACTOR_FOOD_A,
        ProductionPool {
            actor: EXTRACTOR_FOOD_A,
            recipe: Recipe {
                inputs: vec![(GOOD_RAW, Quantity(10))],
                outputs: vec![(GOOD_FOOD, Quantity(10))],
            },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<SupplyPools>().0.insert(
        EXTRACTOR_FOOD_A,
        SupplyPool {
            actor: EXTRACTOR_FOOD_A,
            market,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world
        .resource_mut::<AccountBook>()
        .deposit(food_consumer, Money(1_000_000))
        .unwrap();
    world.resource_mut::<DemandPools>().0.insert(
        food_consumer,
        DemandPool {
            actor: food_consumer,
            market,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(0),
            max_price: Money(2_000),
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
    world.resource_mut::<Markets>().0.insert(
        market,
        MarketSite {
            id: market,
            node_id: crate::routing::NodeId(0),
            name: "M1".to_string(),
        },
    );
    world.insert_resource(HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(consumer, 1_i64), (food_consumer, 1_i64)]),
    });
    {
        let key = MarketGoodKey {
            market,
            good: GOOD_TOOLS,
        };
        let mut goods = world.resource_mut::<MarketGoods>();
        let st = goods
            .0
            .entry(key)
            .or_insert_with(|| MarketGoodState::new(key));
        st.ewma_reference_price = Money(1_000);
        st.last_settlement_price = Money(1_000);
    }
    {
        let key = MarketGoodKey {
            market,
            good: GOOD_FOOD,
        };
        let mut goods = world.resource_mut::<MarketGoods>();
        let st = goods
            .0
            .entry(key)
            .or_insert_with(|| MarketGoodState::new(key));
        st.ewma_reference_price = Money(1_000);
        st.last_settlement_price = Money(1_000);
    }

    let money_before = world.resource::<AccountBook>().total_money().unwrap();

    let n: usize = 240;
    let k: usize = 50; // tail window (iterations)
    let mut consumer_bal_tail: Vec<i64> = Vec::new();
    let mut ext_bal_tail: Vec<i64> = Vec::new();
    let mut traded_tail: Vec<i64> = Vec::new();
    let mut food_traded_tail: Vec<i64> = Vec::new();
    let mut tools_total_tail: Vec<i64> = Vec::new();
    let mut max_price_tail: Vec<i64> = Vec::new();

    for i in 0..n {
        schedule.run(&mut world);
        world.resource_mut::<Tick>().0 += 1;
        // (a) money constant EVERY tick.
        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            money_before,
            "total_money constant in steady state (iter {i})"
        );
        if i >= n - k {
            let accounts = world.resource::<AccountBook>();
            consumer_bal_tail.push(accounts.account(consumer).available.0);
            // EXTRACTOR_TOOLS is the sole firm seller. With full distribution it nets ~0 each tick;
            // the tail spread bounds "no unbounded retained earnings".
            ext_bal_tail.push(accounts.account(EXTRACTOR_TOOLS).available.0);

            let key = MarketGoodKey {
                market,
                good: GOOD_TOOLS,
            };
            let traded = world
                .resource::<MarketGoods>()
                .0
                .get(&key)
                .map(|s| s.traded_qty_last_tick.0)
                .unwrap_or(0);
            traded_tail.push(traded);

            let food_key = MarketGoodKey {
                market,
                good: GOOD_FOOD,
            };
            let food_traded = world
                .resource::<MarketGoods>()
                .0
                .get(&food_key)
                .map(|s| s.traded_qty_last_tick.0)
                .unwrap_or(0);
            food_traded_tail.push(food_traded);

            tools_total_tail.push(
                world
                    .resource::<InventoryBook>()
                    .total_good(GOOD_TOOLS)
                    .unwrap()
                    .0,
            );
            max_price_tail.push(world.resource::<DemandPools>().0[&consumer].max_price.0);
        }
    }

    let min = |v: &[i64]| *v.iter().min().unwrap();
    let max = |v: &[i64]| *v.iter().max().unwrap();

    // (b) EXTRACTOR_TOOLS balance bounded over the tail (no unbounded retained earnings).
    // Full distribution ⇒ the firm nets to ~zero each tick; allow a small epsilon for
    // intra-tick rounding remainders left by floor wage/dividend (never minted).
    let seller_eps: i64 = 1_000;
    assert!(
        max(&ext_bal_tail) - min(&ext_bal_tail) < seller_eps,
        "EXTRACTOR_TOOLS balance bounded over tail (max-min={} < {seller_eps}); tail={:?}",
        max(&ext_bal_tail) - min(&ext_bal_tail),
        ext_bal_tail
    );

    // (c1) consumer ACCOUNT balance lives in a committed band [lo,hi], lo>0, hi/lo < r.
    let cons_lo = min(&consumer_bal_tail);
    let cons_hi = max(&consumer_bal_tail);
    assert!(
        cons_lo > 0,
        "consumer never drains to zero (living loop); lo={cons_lo}"
    );
    assert!(
        (cons_hi as i128) < (cons_lo as i128) * 4,
        "consumer balance band ratio hi/lo < 4 (hi={cons_hi}, lo={cons_lo})"
    );

    // (c2) market traded_qty lives in a band [lo,hi], lo>0.
    let tr_lo = min(&traded_tail);
    let tr_hi = max(&traded_tail);
    assert!(
        tr_lo > 0,
        "market traded every tick in steady state (lo={tr_lo})"
    );
    assert!(
        (tr_hi as i128) < (tr_lo as i128) * 5,
        "traded_qty band ratio hi/lo < 5 (hi={tr_hi}, lo={tr_lo})"
    );

    // FOOD market also lives in a bounded band (lo > 0) — FOOD self-sustains identically.
    // PRECONDITION (spec §8): TOOLS and FOOD share identical opening prices (Money(1_000))
    // and equal pool_weights, so the two goods stay symmetric and these bands mirror TOOLS'.
    // If a band fails, a REAL asymmetry exists (e.g. divergent ewma_reference_price) — STOP
    // and investigate; do NOT loosen the band to force green.
    let f_lo = min(&food_traded_tail);
    let f_hi = max(&food_traded_tail);
    assert!(
        f_lo > 0,
        "FOOD market traded every tick in steady state (lo={f_lo})"
    );
    assert!(
        (f_hi as i128) < (f_lo as i128) * 5,
        "FOOD traded_qty band hi/lo < 5 (hi={f_hi}, lo={f_lo})"
    );

    // (d) total_good(GOOD_TOOLS) bounded (not monotonic growth/collapse). With regen=10 and
    // consumption ~10/tick the on-hand TOOLS stays small and bounded.
    let tools_lo = min(&tools_total_tail);
    let tools_hi = max(&tools_total_tail);
    assert!(
        tools_hi - tools_lo < 10_000,
        "TOOLS on-hand bounded over tail (hi={tools_hi}, lo={tools_lo})"
    );

    // Non-vacuity: regen + production + trades all occurred.
    let ev = &world.resource::<TradeLedger>().0;
    assert!(
        ev.iter()
            .any(|e| matches!(e, EconomyEvent::Regenerated { .. })),
        "regen fired"
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, EconomyEvent::Produced { good, .. } if *good == GOOD_TOOLS)),
        "tools produced"
    );
    assert!(
        ev.iter().any(|e| matches!(e, EconomyEvent::Trade { .. })),
        "trades cleared"
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, EconomyEvent::Produced { good, .. } if *good == GOOD_FOOD)),
        "food produced"
    );

    // Free-prices nudge is active in the default schedule. In the converged steady state the
    // excess-demand signal is ~0, so the reservation price must settle into a bounded band
    // (it neither runs to the ceiling nor oscillates wildly). This is the stability guard.
    let mp_lo = *max_price_tail.iter().min().unwrap();
    let mp_hi = *max_price_tail.iter().max().unwrap();
    assert!(mp_lo > 0, "reservation price stays positive (no ZeroPrice)");
    assert!(mp_hi - mp_lo < 2_000, "reservation-price tail bounded (no runaway/oscillation): lo={mp_lo} hi={mp_hi}");
}
