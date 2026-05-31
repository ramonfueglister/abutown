use std::collections::BTreeSet;

use bevy_ecs::prelude::*;

use crate::economy::{DormantMarkets, MarketChunks, MarketId, WarmMarkets, refresh_dormant_markets_system};
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
    world.insert_resource(WarmMarkets::default());

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
    world.insert_resource(WarmMarkets::default());

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

use crate::economy::{
    EconomyConfig, GOOD_TOOLS, MarketGoods, Trader, TraderState, Traders, run_traders_at_tick,
};

#[test]
fn dormant_trader_is_frozen_and_conserves() {
    let trader_actor = EconomicActorId(1);
    let source = MarketId(1);
    let dest = MarketId(2);

    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let goods = MarketGoods::default();
    let cfg = EconomyConfig::default();

    accounts.deposit(trader_actor, Money(1_000_000)).unwrap();
    let mut traders = Traders::default();
    traders.0.insert(
        trader_actor,
        Trader {
            actor: trader_actor,
            good: GOOD_TOOLS,
            source,
            dest,
            distance_tiles: 4,
            batch_qty: Quantity(100),
            buy_premium_bps: 500,
            sell_discount_bps: 500,
            order_ttl_ticks: 10,
            state: TraderState::Buying { order: None },
        },
    );

    let money_before = accounts.total_money();
    let trader_before = traders.0[&trader_actor].clone();

    // source market dormant -> trader frozen for many ticks
    let dormant: BTreeSet<MarketId> = [source].into_iter().collect();
    for tick in 0..20 {
        run_traders_at_tick(
            &mut accounts,
            &mut inventory,
            &mut orders,
            &mut ledger,
            &mut dirty,
            &mut next,
            &goods,
            &mut traders,
            &cfg,
            tick,
            &dormant,
        )
        .unwrap();
    }

    assert!(orders.bids.is_empty(), "frozen trader places no bids");
    assert_eq!(
        accounts.total_money(),
        money_before,
        "money conserved while frozen"
    );
    assert_eq!(
        traders.0[&trader_actor].state, trader_before.state,
        "frozen trader keeps its state",
    );
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
fn asleep_anchored_market_stays_frozen_end_to_end() {
    let market = MarketId(99);
    let coord = ChunkCoord { x: 5, y: 5 };
    let (mut world, mut schedule) = lod_world(market, coord, /*asleep=*/ true);

    for _ in 0..10 {
        schedule.run(&mut world);
        let mut t = world.resource_mut::<Tick>();
        t.0 += 1;
    }
    // No asks were ever placed because the supplier's market is dormant.
    assert!(world.resource::<OrderBook>().asks.is_empty());
    // Plugin installed the two new resources.
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
