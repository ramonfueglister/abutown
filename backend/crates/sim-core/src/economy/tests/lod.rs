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
        &mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next,
        &mut demand, &mut supply, 0, 5, &dormant,
    )
    .unwrap();

    assert!(orders.bids.is_empty(), "dormant market must not place bids");
    assert!(dirty.0.is_empty(), "dormant market must not dirty any market-good");
    assert_eq!(accounts.total_money(), before, "no cash locked while dormant");
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
        &mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next,
        &mut demand, &mut supply, 0, 5, &dormant,
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
            &mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next,
            &mut demand, &mut supply, tick, 5, &dormant,
        )
        .unwrap();
    }
    assert!(orders.bids.is_empty());

    // Wake on tick 100: exactly ONE order, not a 100-order backlog burst.
    let awake: BTreeSet<MarketId> = BTreeSet::new();
    generate_pool_orders_at_tick(
        &mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next,
        &mut demand, &mut supply, 100, 5, &awake,
    )
    .unwrap();
    assert_eq!(orders.bids.len(), 1, "wake emits exactly one order");
}

use crate::economy::{
    GOOD_TOOLS, Trader, TraderState, Traders, run_traders_at_tick, EconomyConfig, MarketGoods,
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
            &mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next,
            &goods, &mut traders, &cfg, tick, &dormant,
        )
        .unwrap();
    }

    assert!(orders.bids.is_empty(), "frozen trader places no bids");
    assert_eq!(accounts.total_money(), money_before, "money conserved while frozen");
    assert_eq!(
        traders.0[&trader_actor].state,
        trader_before.state,
        "frozen trader keeps its state",
    );
}
