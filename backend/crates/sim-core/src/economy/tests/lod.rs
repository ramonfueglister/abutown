use std::collections::BTreeSet;

use bevy_ecs::prelude::*;

use crate::economy::{DormantMarkets, MarketChunks, MarketId, refresh_dormant_markets_system};
use crate::ids::ChunkCoord;
use crate::world::components::{ActiveChunk, AsleepChunk, ChunkCoordComp, HotChunk, WarmChunk};

#[test]
fn refresh_dormant_markets_marks_only_anchored_inactive() {
    let mut world = World::new();
    // Four chunks, one per LOD level.
    world.spawn((ChunkCoordComp(ChunkCoord { x: 0, y: 0 }), AsleepChunk));
    world.spawn((ChunkCoordComp(ChunkCoord { x: 1, y: 0 }), WarmChunk));
    world.spawn((ChunkCoordComp(ChunkCoord { x: 2, y: 0 }), ActiveChunk));
    world.spawn((ChunkCoordComp(ChunkCoord { x: 3, y: 0 }), HotChunk));

    let mut anchors = MarketChunks::default();
    anchors.0.insert(MarketId(10), ChunkCoord { x: 0, y: 0 }); // asleep -> dormant
    anchors.0.insert(MarketId(11), ChunkCoord { x: 1, y: 0 }); // warm   -> dormant
    anchors.0.insert(MarketId(12), ChunkCoord { x: 2, y: 0 }); // active -> awake
    anchors.0.insert(MarketId(13), ChunkCoord { x: 3, y: 0 }); // hot    -> awake
    world.insert_resource(anchors);
    world.insert_resource(DormantMarkets::default());

    let mut schedule = bevy_ecs::schedule::Schedule::default();
    schedule.add_systems(refresh_dormant_markets_system);
    schedule.run(&mut world);

    let dormant = world.resource::<DormantMarkets>();
    let expected: BTreeSet<MarketId> = [MarketId(10), MarketId(11)].into_iter().collect();
    assert_eq!(dormant.0, expected);
}

#[test]
fn unanchored_market_is_never_dormant() {
    let mut world = World::new();
    // No active chunks at all, and the market is not anchored.
    world.insert_resource(MarketChunks::default());
    world.insert_resource(DormantMarkets::default());

    let mut schedule = bevy_ecs::schedule::Schedule::default();
    schedule.add_systems(refresh_dormant_markets_system);
    schedule.run(&mut world);

    assert!(world.resource::<DormantMarkets>().0.is_empty());
}

use crate::economy::{
    AccountBook, DemandPool, DemandPools, DirtyMarketGoods, EconomicActorId, GOOD_FOOD,
    InventoryBook, Money, NextOrderId, OrderBook, Quantity, SupplyPool, SupplyPools, TradeLedger,
    generate_pool_orders_at_tick,
};

fn seeded_demand_pool(actor: EconomicActorId, market: MarketId) -> DemandPool {
    DemandPool {
        actor,
        market,
        good: GOOD_FOOD,
        desired_qty_per_tick: Quantity(5),
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
fn dormant_market_generates_no_orders() {
    let actor = EconomicActorId(1);
    let market = MarketId(7);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();
    accounts.deposit(actor, Money(1_000_000)).unwrap();
    demand.0.insert(actor, seeded_demand_pool(actor, market));
    let before = accounts.total_money();

    let dormant: BTreeSet<MarketId> = [market].into_iter().collect();
    generate_pool_orders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut demand,
        &mut supply,
        0,
        5,
        &dormant,
    )
    .unwrap();

    assert!(orders.bids.is_empty(), "dormant market must not place bids");
    assert!(
        dirty.0.is_empty(),
        "dormant market must not dirty any market-good"
    );
    assert_eq!(
        accounts.total_money(),
        before,
        "no cash locked while dormant"
    );
}

#[test]
fn awake_market_still_generates_orders() {
    let actor = EconomicActorId(1);
    let market = MarketId(7);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();
    accounts.deposit(actor, Money(1_000_000)).unwrap();
    demand.0.insert(actor, seeded_demand_pool(actor, market));

    let dormant: BTreeSet<MarketId> = BTreeSet::new();
    generate_pool_orders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut demand,
        &mut supply,
        0,
        5,
        &dormant,
    )
    .unwrap();

    assert_eq!(orders.bids.len(), 1, "awake market places its bid");
}

#[test]
fn market_resumes_with_single_order_no_burst() {
    let actor = EconomicActorId(1);
    let market = MarketId(7);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();
    accounts.deposit(actor, Money(1_000_000)).unwrap();
    demand.0.insert(actor, seeded_demand_pool(actor, market));

    let dormant: BTreeSet<MarketId> = [market].into_iter().collect();
    // Dormant for 100 ticks: no orders accrue.
    for tick in 0..100 {
        generate_pool_orders_at_tick(
            &mut accounts,
            &mut inventory,
            &mut orders,
            &mut ledger,
            &mut dirty,
            &mut next,
            &mut demand,
            &mut supply,
            tick,
            5,
            &dormant,
        )
        .unwrap();
    }
    assert!(orders.bids.is_empty());

    // Wake on tick 100: exactly ONE order, not a 100-order backlog burst.
    let awake: BTreeSet<MarketId> = BTreeSet::new();
    generate_pool_orders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut demand,
        &mut supply,
        100,
        5,
        &awake,
    )
    .unwrap();
    assert_eq!(orders.bids.len(), 1, "wake emits exactly one order");
}

use crate::economy::EconomyPlugin;
use crate::mobility::resources::Tick;
use crate::world::plugin::CorePlugin;
use crate::world::schedule::SimPlugin;

// Build a world with Core + Mobility + Economy, one supply pool selling FOOD at
// `market`, the trader/markets un-touched. Anchor `market` to `coord`, and spawn a
// chunk entity at `coord` with the given marker. Returns the assembled world+schedule.
fn lod_world(
    market: MarketId,
    coord: ChunkCoord,
    asleep: bool,
) -> (World, bevy_ecs::schedule::Schedule) {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let supplier = EconomicActorId(50);
    {
        let mut inv = world.resource_mut::<InventoryBook>();
        inv.deposit(supplier, GOOD_FOOD, Quantity(1_000_000))
            .unwrap();
    }
    {
        let mut supply = world.resource_mut::<SupplyPools>();
        supply.0.insert(
            supplier,
            SupplyPool {
                actor: supplier,
                market,
                good: GOOD_FOOD,
                offered_qty_per_tick: Quantity(10),
                min_price: Money(1_000),
                interval_ticks: 1,
                last_generated_tick: None,
            },
        );
    }
    {
        let mut anchors = world.resource_mut::<MarketChunks>();
        anchors.0.insert(market, coord);
    }
    if asleep {
        world.spawn((ChunkCoordComp(coord), AsleepChunk));
    } else {
        world.spawn((ChunkCoordComp(coord), ActiveChunk));
    }
    world.insert_resource(Tick(0));
    (world, schedule)
}

#[test]
#[allow(non_snake_case)]
fn asleep_anchored_market_DOES_flow() {
    use crate::economy::{
        EconomyConfig, MarketDistances, MarketGoodKey, MarketGoods, TRANSPORT_OPERATOR,
    };

    // Market A (99): the lod_world supplier sells FOOD. Market B (100): a buyer
    // demands FOOD. BOTH are anchored to ASLEEP chunks (dormant). The macro flow
    // is the *only* path that touches dormant markets, and unlike the retired
    // warm-flow it spans markets: with a positive A->B price gap that strictly
    // exceeds transport, FOOD flows surplus(A)->deficit(B). This proves the LOD
    // path is NOT a hollow world — dormant markets still equilibrate.
    let a = MarketId(99);
    let coord_a = ChunkCoord { x: 5, y: 5 };
    let (mut world, mut schedule) = lod_world(a, coord_a, /*asleep=*/ true);

    // lod_world offers only 10/tick; at rate 50 that floors transport to 0
    // (50*10/1000 == 0). Bump the supplier's offer to 100 so transport > 0 and
    // a clean by-hand quantity exists. The supplier already holds 1_000_000 FOOD.
    let supplier = EconomicActorId(50);
    {
        let mut supply = world.resource_mut::<SupplyPools>();
        supply.0.get_mut(&supplier).unwrap().offered_qty_per_tick = Quantity(100);
    }

    // Market B: a second asleep-anchored market with a FOOD demand pool.
    let b = MarketId(100);
    let coord_b = ChunkCoord { x: 6, y: 6 };
    let buyer = EconomicActorId(60);
    {
        let mut accounts = world.resource_mut::<AccountBook>();
        accounts.deposit(buyer, Money(1_000_000)).unwrap();
    }
    {
        let mut demand = world.resource_mut::<DemandPools>();
        demand.0.insert(
            buyer,
            DemandPool {
                actor: buyer,
                market: b,
                good: GOOD_FOOD,
                desired_qty_per_tick: Quantity(100),
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
    }
    {
        let mut anchors = world.resource_mut::<MarketChunks>();
        anchors.0.insert(b, coord_b);
    }
    world.spawn((ChunkCoordComp(coord_b), AsleepChunk));

    // transport > 0 requires a finite distance AND rate 50 (with q=100 ->
    // per_tile = 50*100/1000 = 5, transport = 5 * dist).
    {
        let mut distances = world.resource_mut::<MarketDistances>();
        distances.0.insert((a, b), 2);
        distances.0.insert((b, a), 2);
    }
    {
        let mut config = world.resource_mut::<EconomyConfig>();
        config.transport_cost_per_tile_unit = Money(50);
        // macro_flow_interval_ticks defaults to 10; leave it.
    }

    let money_before = world.resource::<AccountBook>().total_money().unwrap();
    let supplier_food_before = world
        .resource::<InventoryBook>()
        .balance(supplier, GOOD_FOOD)
        .available;

    // The schedule's own tick_increment_system advances Tick by 1 per run, and
    // the manual bump below adds another (the established idiom in this file's
    // end-to-end tests), so Tick steps by 2 per iteration. The macro flow reads
    // the PRE-increment tick (it is ordered .before tick_increment_system), so
    // across 6 iterations it observes ticks 0, 2, 4, 6, 8, 10. The interval gate
    // (macro_flow_interval_ticks = 10) fires on multiples of 10 -> tick 0 AND
    // tick 10 -> exactly 2 flows.
    for _ in 0..6 {
        schedule.run(&mut world);
        let mut t = world.resource_mut::<Tick>();
        t.0 += 1;
    }

    // Hand-computed flow per fire (q = min(surplus 100, deficit 100, q_cap 100)):
    //   p_src (A, supply-only) = ask_floor = 1000; p_dst (B, demand-only) =
    //   bid_ceiling = 2000. src_revenue = 1000*100/1000 = 100; transport =
    //   (50*100/1000)*2 = 10; dst_payment = 110; net_gain = 200-100-10 = 90 > 0.
    // Two fires -> 200 FOOD moved A->B, B traded 200 @ 2000, A traded 200 @ 1000.
    let goods = &world.resource::<MarketGoods>().0;
    let state_a = &goods[&MarketGoodKey {
        market: a,
        good: GOOD_FOOD,
    }];
    let state_b = &goods[&MarketGoodKey {
        market: b,
        good: GOOD_FOOD,
    }];
    assert_eq!(
        state_b.last_settlement_price,
        Money(2_000),
        "B's price was discovered by the macro flow (changed from ZERO default)"
    );
    assert_eq!(
        state_b.traded_qty_last_tick,
        Quantity(200),
        "B imported 100 FOOD per fire across the two interval boundaries"
    );
    assert_eq!(state_a.last_settlement_price, Money(1_000));
    assert_eq!(state_a.traded_qty_last_tick, Quantity(200));

    // Goods MOVED A->B (traded_qty=200 above proves the flow). The buyer received 200
    // FOOD; the consumption sink then consumed part — assert received = held + consumed.
    let buyer_consumed: i64 = world
        .resource::<TradeLedger>()
        .0
        .iter()
        .filter_map(|e| match e {
            crate::economy::EconomyEvent::FinalConsumed { actor, good, qty }
                if *actor == buyer && *good == GOOD_FOOD =>
            {
                Some(qty.0)
            }
            _ => None,
        })
        .sum();
    let inv = world.resource::<InventoryBook>();
    assert_eq!(
        inv.balance(buyer, GOOD_FOOD).available.0 + buyer_consumed,
        200,
        "buyer received 200 imported FOOD (held + consumed by the sink)"
    );
    assert_eq!(
        inv.balance(supplier, GOOD_FOOD).available,
        Quantity(supplier_food_before.0 - 200),
        "supplier at the asleep source shipped the FOOD out",
    );

    // Conservation: transport is a TRANSFER to TRANSPORT_OPERATOR, never minted
    // or destroyed, so total money is exactly preserved.
    let money_after = world.resource::<AccountBook>().total_money().unwrap();
    assert_eq!(money_after, money_before, "cash exactly conserved");
    assert_eq!(
        world
            .resource::<AccountBook>()
            .account(TRANSPORT_OPERATOR)
            .available,
        Money(20),
        "operator collected transport (10 per fire) across two fires",
    );

    // Both new resources are still installed by the plugin.
    assert!(world.contains_resource::<MarketChunks>());
    assert!(world.contains_resource::<DormantMarkets>());
}

#[test]
fn active_anchored_market_trades_end_to_end() {
    let market = MarketId(99);
    let coord = ChunkCoord { x: 5, y: 5 };
    let (mut world, mut schedule) = lod_world(market, coord, /*asleep=*/ false);

    let mut saw_ask = false;
    for _ in 0..10 {
        schedule.run(&mut world);
        if !world.resource::<OrderBook>().asks.is_empty() {
            saw_ask = true;
        }
        let mut t = world.resource_mut::<Tick>();
        t.0 += 1;
    }
    assert!(saw_ask, "active market must place asks");
}

#[test]
fn market_distances_stores_directed_pairs_both_ways() {
    use crate::economy::MarketDistances;
    let mut d = MarketDistances::default();
    d.0.insert((MarketId(1), MarketId(2)), 7);
    d.0.insert((MarketId(2), MarketId(1)), 7);
    assert_eq!(d.0.get(&(MarketId(1), MarketId(2))).copied(), Some(7));
    assert_eq!(d.0.get(&(MarketId(2), MarketId(1))).copied(), Some(7));
    assert_eq!(d.0.get(&(MarketId(1), MarketId(3))).copied(), None);
}

#[test]
fn active_to_dormant_handoff_conserves() {
    use crate::economy::{EconomyConfig, MarketDistances};

    // m_b starts ACTIVE with a live consumer bid (locks cash); m_a is a dormant
    // surplus. S3: while m_b is active the flow DRAINS its residual bid into available
    // and imports goods from m_a directly — it no longer waits for the order to TTL-expire
    // (that was the pre-S3 dormant-only behavior). We then demote m_b's chunk to Asleep
    // and keep running. Conservation MUST hold every tick across the LOD handoff (the real
    // atomicity guard), and the consumer ends up served.
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let m_a = MarketId(9_201); // dormant surplus
    let m_b = MarketId(9_202); // active->demoted deficit
    let coord_a = ChunkCoord { x: 5, y: 5 };
    let coord_b = ChunkCoord { x: 9, y: 5 };
    let supplier = EconomicActorId(50);
    let consumer = EconomicActorId(60);
    {
        let mut inv = world.resource_mut::<InventoryBook>();
        inv.deposit(supplier, GOOD_FOOD, Quantity(1_000_000))
            .unwrap();
    }
    {
        let mut acc = world.resource_mut::<AccountBook>();
        acc.deposit(consumer, Money(1_000_000_000)).unwrap();
    }
    {
        let mut supply = world.resource_mut::<SupplyPools>();
        supply.0.insert(
            supplier,
            SupplyPool {
                actor: supplier,
                market: m_a,
                good: GOOD_FOOD,
                offered_qty_per_tick: Quantity(200),
                min_price: Money(500),
                interval_ticks: 1,
                last_generated_tick: None,
            },
        );
    }
    {
        let mut demand = world.resource_mut::<DemandPools>();
        demand.0.insert(
            consumer,
            DemandPool {
                actor: consumer,
                market: m_b,
                good: GOOD_FOOD,
                desired_qty_per_tick: Quantity(50),
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
    }
    {
        let mut anchors = world.resource_mut::<MarketChunks>();
        anchors.0.insert(m_a, coord_a);
        anchors.0.insert(m_b, coord_b);
    }
    {
        let mut dist = world.resource_mut::<MarketDistances>();
        dist.0.insert((m_a, m_b), 4);
        dist.0.insert((m_b, m_a), 4);
    }
    {
        let mut cfg = world.resource_mut::<EconomyConfig>();
        cfg.transport_cost_per_tile_unit = Money(50);
    }
    // m_a asleep (dormant) from the start; m_b ACTIVE so its bid is placed+locked.
    let chunk_b = world.spawn((ChunkCoordComp(coord_b), ActiveChunk)).id();
    world.spawn((ChunkCoordComp(coord_a), AsleepChunk));
    world.insert_resource(Tick(0));

    let money_total = world.resource::<AccountBook>().total_money().unwrap();
    let good_total = world
        .resource::<InventoryBook>()
        .total_good(GOOD_FOOD)
        .unwrap();
    // Goods are no longer invariant (the consumption sink drains delivered goods). Assert
    // the ledger-derived conservation: Δtotal_good == ΣProduced − Σ(Consumed+FinalConsumed).
    let food_net = |w: &World| -> i64 {
        w.resource::<TradeLedger>()
            .0
            .iter()
            .filter_map(|e| match e {
                crate::economy::EconomyEvent::Produced { good, qty, .. } if *good == GOOD_FOOD => {
                    Some(qty.0)
                }
                crate::economy::EconomyEvent::Consumed { good, qty, .. } if *good == GOOD_FOOD => {
                    Some(-qty.0)
                }
                crate::economy::EconomyEvent::FinalConsumed { good, qty, .. }
                    if *good == GOOD_FOOD =>
                {
                    Some(-qty.0)
                }
                _ => None,
            })
            .sum()
    };

    // Tick 0: m_b active -> the consumer bids, then the flow drains that residual bid and
    // imports goods from m_a directly. Conservation must hold.
    schedule.run(&mut world);
    assert_eq!(
        world.resource::<AccountBook>().total_money().unwrap(),
        money_total,
        "money conserved while m_b is active"
    );
    assert_eq!(
        world
            .resource::<InventoryBook>()
            .total_good(GOOD_FOOD)
            .unwrap()
            .0,
        good_total.0 + food_net(&world),
        "goods conserved vs ledger while m_b is active"
    );
    {
        let mut t = world.resource_mut::<Tick>();
        t.0 += 1;
    }

    // Demote m_b to Asleep and keep running through the old TTL window; conservation must
    // hold EVERY tick across the LOD handoff (the atomicity regression guard).
    world
        .entity_mut(chunk_b)
        .remove::<ActiveChunk>()
        .insert(AsleepChunk);
    for _ in 0..15 {
        schedule.run(&mut world);
        let m = world.resource::<AccountBook>().total_money().unwrap();
        let g = world
            .resource::<InventoryBook>()
            .total_good(GOOD_FOOD)
            .unwrap()
            .0;
        assert_eq!(m, money_total, "money conserved across handoff");
        assert_eq!(
            g,
            good_total.0 + food_net(&world),
            "goods conserved vs ledger across handoff"
        );
        let mut t = world.resource_mut::<Tick>();
        t.0 += 1;
    }

    // The consumer was served — the flow moved goods into m_b (while active via the
    // residual-bid drain, and via the dormant-pool path after demotion), and the
    // consumption sink then USED them. "Served" now means consumed > 0 (the goods no
    // longer just pile up in the consumer's inventory — they flow through to consumption).
    let consumer_consumed: i64 = world
        .resource::<TradeLedger>()
        .0
        .iter()
        .filter_map(|e| match e {
            crate::economy::EconomyEvent::FinalConsumed { actor, good, qty }
                if *actor == consumer && *good == GOOD_FOOD =>
            {
                Some(qty.0)
            }
            _ => None,
        })
        .sum();
    assert!(
        consumer_consumed > 0,
        "the flow served the observed/demoted market's demand (goods delivered + consumed)"
    );
}
