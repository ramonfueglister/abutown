use bevy_ecs::prelude::*;

use crate::economy::{
    AccountBook, DemandPool, DemandPools, EconomicActorId, EconomyConfig, EconomyEvent,
    EconomyPlugin, GOOD_FOOD, GOOD_IRON, GOOD_TOOLS, GOOD_WOOD, InventoryBook, MarketGoodKey,
    MarketGoodState, MarketGoods, MarketId, Money, Quantity, SupplyPool, SupplyPools, TradeLedger,
};
use crate::world::plugin::CorePlugin;
use crate::world::schedule::SimPlugin;

/// Seed a buyer pool (with cash) and a seller pool (with inventory) for one
/// market-good whose prices overlap, so one schedule run clears one trade.
fn seed_trading_pair(
    world: &mut World,
    buyer: EconomicActorId,
    seller: EconomicActorId,
    market: MarketId,
) {
    world
        .resource_mut::<AccountBook>()
        .deposit(buyer, Money(10_000))
        .unwrap();
    world
        .resource_mut::<InventoryBook>()
        .deposit(seller, GOOD_FOOD, Quantity(2_000))
        .unwrap();
    world.resource_mut::<DemandPools>().0.insert(
        buyer,
        DemandPool {
            actor: buyer,
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
    world.resource_mut::<SupplyPools>().0.insert(
        seller,
        SupplyPool {
            actor: seller,
            market,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(1_000),
            min_price: Money(900),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
}

#[test]
fn economy_clears_a_trade_end_to_end() {
    // End-to-end: pool -> order -> clear -> Trade, with NO pre-seeded
    // MarketGoodState (it is get-or-created). Guards the gap where the wired
    // system path would otherwise fail InvalidOrder and never trade.
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let buyer = EconomicActorId(1);
    let seller = EconomicActorId(2);
    seed_trading_pair(&mut world, buyer, seller, MarketId(1));

    schedule.run(&mut world);

    let trades: Vec<_> = world
        .resource::<TradeLedger>()
        .0
        .iter()
        .filter_map(|e| match e {
            EconomyEvent::Trade {
                buyer: b,
                seller: s,
                qty,
                price,
                market,
                ..
            } => Some((*b, *s, *qty, *price, *market)),
            _ => None,
        })
        .collect();
    assert_eq!(trades.len(), 1, "exactly one trade should clear");
    let (b, s, qty, price, market) = trades[0];
    assert_eq!(b, buyer);
    assert_eq!(s, seller);
    assert_eq!(qty, Quantity(1_000));
    assert_eq!(price, Money(900)); // first trade: last=ZERO clamps up to the ask
    assert_eq!(market, MarketId(1));
    // Buyer received goods; books conserved (bid locked 1000, traded at 900,
    // 100 refunded).
    assert_eq!(
        world
            .resource::<InventoryBook>()
            .balance(buyer, GOOD_FOOD)
            .available,
        Quantity(1_000)
    );
    assert_eq!(
        world.resource::<AccountBook>().account(buyer).locked,
        Money(0)
    );
    assert_eq!(
        world.resource::<AccountBook>().account(seller).available,
        Money(900)
    );
}

#[test]
fn dirty_market_keys_are_processed_in_stable_order() {
    // Two markets dirty in one tick must clear in (market, good) order — proving
    // the BTreeSet dirty-key iteration drives deterministic processing.
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    // Insert the higher market id first to prove ordering is by key, not insertion.
    seed_trading_pair(
        &mut world,
        EconomicActorId(21),
        EconomicActorId(22),
        MarketId(2),
    );
    seed_trading_pair(
        &mut world,
        EconomicActorId(11),
        EconomicActorId(12),
        MarketId(1),
    );

    schedule.run(&mut world);

    let trade_markets: Vec<MarketId> = world
        .resource::<TradeLedger>()
        .0
        .iter()
        .filter_map(|e| match e {
            EconomyEvent::Trade { market, .. } => Some(*market),
            _ => None,
        })
        .collect();
    assert_eq!(trade_markets, vec![MarketId(1), MarketId(2)]);
}

#[test]
fn telemetry_updates_ewma_with_basis_points() {
    let mut goods = MarketGoods::default();
    let key = MarketGoodKey {
        market: MarketId(1),
        good: GOOD_FOOD,
    };
    goods.0.insert(
        key,
        MarketGoodState {
            key,
            last_settlement_price: Money(2_000),
            ewma_reference_price: Money(1_000),
            traded_qty_last_tick: Quantity(1_000),
            unmet_demand_last_tick: Quantity(0),
            unsold_supply_last_tick: Quantity(0),
            dirty: false,
            last_cleared_tick: 1,
        },
    );
    let config = EconomyConfig {
        ewma_alpha_bps: 2_500,
        ..EconomyConfig::default()
    };
    crate::economy::update_market_telemetry(&mut goods, config).unwrap();

    assert_eq!(goods.0[&key].ewma_reference_price, Money(1_250));
}

#[test]
fn economy_config_default_transport_cost_per_tile_unit() {
    assert_eq!(
        EconomyConfig::default().transport_cost_per_tile_unit,
        Money(5)
    );
}

#[test]
fn production_runs_through_schedule() {
    use crate::economy::{ProductionPool, ProductionPools, Recipe};
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let actor = EconomicActorId(7);
    world
        .resource_mut::<InventoryBook>()
        .deposit(actor, GOOD_WOOD, Quantity(2_000))
        .unwrap();
    world
        .resource_mut::<InventoryBook>()
        .deposit(actor, GOOD_IRON, Quantity(1_000))
        .unwrap();
    world.resource_mut::<ProductionPools>().0.insert(
        actor,
        ProductionPool {
            actor,
            recipe: Recipe {
                inputs: vec![(GOOD_WOOD, Quantity(2_000)), (GOOD_IRON, Quantity(1_000))],
                outputs: vec![(GOOD_TOOLS, Quantity(1_000))],
            },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );

    schedule.run(&mut world);

    assert_eq!(
        world
            .resource::<InventoryBook>()
            .balance(actor, GOOD_TOOLS)
            .available,
        Quantity(1_000)
    );
    assert_eq!(
        world
            .resource::<InventoryBook>()
            .balance(actor, GOOD_WOOD)
            .available,
        Quantity(0)
    );
}

#[test]
fn trader_arbitrages_between_markets_end_to_end() {
    use crate::economy::{
        DemandPool, DemandPools, SupplyPool, SupplyPools, TRANSPORT_OPERATOR, Trader, TraderState,
        Traders,
    };
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let trader = EconomicActorId(1);
    let supplier = EconomicActorId(2); // sells cheap at source M1
    let consumer = EconomicActorId(3); // buys dear at dest M2
    let src = MarketId(1);
    let dst = MarketId(2);

    world
        .resource_mut::<AccountBook>()
        .deposit(trader, Money(1_000_000))
        .unwrap();
    world
        .resource_mut::<AccountBook>()
        .deposit(consumer, Money(1_000_000))
        .unwrap();
    world
        .resource_mut::<InventoryBook>()
        .deposit(supplier, GOOD_TOOLS, Quantity(100_000))
        .unwrap();
    // supplier offers TOOLS cheap at M1
    world.resource_mut::<SupplyPools>().0.insert(
        supplier,
        SupplyPool {
            actor: supplier,
            market: src,
            good: GOOD_TOOLS,
            offered_qty_per_tick: Quantity(1_000),
            min_price: Money(800),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    // consumer demands TOOLS dear at M2
    world.resource_mut::<DemandPools>().0.insert(
        consumer,
        DemandPool {
            actor: consumer,
            market: dst,
            good: GOOD_TOOLS,
            desired_qty_per_tick: Quantity(1_000),
            max_price: Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<Traders>().0.insert(
        trader,
        Trader {
            actor: trader,
            good: GOOD_TOOLS,
            source: src,
            dest: dst,
            distance_tiles: 4,
            batch_qty: Quantity(1_000),
            buy_premium_bps: 5_000,
            sell_discount_bps: 5_000,
            order_ttl_ticks: 50,
            state: TraderState::Buying { order: None },
        },
    );

    let total_money_before = {
        let a = world.resource::<AccountBook>();
        a.total_money().unwrap()
    };
    for _ in 0..40 {
        schedule.run(&mut world);
    }

    // The trader paid transport at least once (completed a buy->travel leg).
    assert!(
        world
            .resource::<AccountBook>()
            .account(TRANSPORT_OPERATOR)
            .available
            .0
            > 0
    );
    // A trade happened at the destination (trader sold to consumer): consumer holds TOOLS.
    assert!(
        world
            .resource::<InventoryBook>()
            .balance(consumer, GOOD_TOOLS)
            .available
            .0
            > 0
    );
    // Money conserved across all accounts (incl. operator).
    assert_eq!(
        world.resource::<AccountBook>().total_money().unwrap(),
        total_money_before
    );
}

#[test]
fn refresh_lod_observes_post_reclassify_lod_not_stale_active() {
    // Discriminating test for the EconomySet::RefreshLod.after(CoreSet::LodReclassify)
    // ordering edge.
    //
    // Setup: spawn a chunk entity with ActiveChunk but zero subscribers and no
    // population. reclassify_chunk_lod_system will demote it Active→Asleep in the
    // same tick. refresh_dormant_markets_system must run AFTER that demotion so it
    // sees the post-reclassify (non-active) state and marks the market dormant.
    //
    // Ordering matters: if RefreshLod ran BEFORE LodReclassify it would observe the
    // still-Active marker and the market would NOT be marked dormant (the assertion
    // would fail), proving the .after() edge is load-bearing.
    use crate::economy::{DormantMarkets, EconomyPlugin, MarketChunks, MarketId};
    use crate::ids::ChunkCoord;
    use crate::mobility::resources::Tick;
    use crate::world::components::{
        ActiveChunk, ChunkCoordComp, ChunkSubscriberCount, LodCooldown,
    };
    use crate::world::plugin::CorePlugin;
    use bevy_ecs::prelude::*;

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let market = MarketId(77);
    let coord = ChunkCoord { x: 9, y: 9 };

    // Anchor market to the chunk.
    world.resource_mut::<MarketChunks>().0.insert(market, coord);

    // Spawn an Active chunk entity with zero subscribers and cooldown=0. No
    // population entry → reclassify target is Asleep. With cooldown=0 (no
    // hysteresis) the demotion fires immediately: Active→Asleep.
    world.spawn((
        ChunkCoordComp(coord),
        ActiveChunk,
        ChunkSubscriberCount(0),
        LodCooldown(0),
    ));

    world.insert_resource(Tick(0));

    // Run one full tick. reclassify demotes Active→Asleep first (CoreSet::LodReclassify),
    // then refresh_dormant_markets_system (EconomySet::RefreshLod, ordered after) sees no
    // Active/Hot chunk at coord → market is dormant.
    schedule.run(&mut world);

    assert!(
        world.resource::<DormantMarkets>().0.contains(&market),
        "RefreshLod must observe the reclassified (Asleep) chunk state; \
         if the ordering edge were removed it would see ActiveChunk and market \
         would NOT be dormant"
    );
}
