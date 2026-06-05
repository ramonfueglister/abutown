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
            last_consumed_tick: None,
            income_last_tick: Money::ZERO,
            mpc_bps: 8_000,
            autonomous: Money(5_000),
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
    // Buyer received 1000 goods (the trade); the same-tick consumption sink then consumed
    // them. Books conserved (bid locked 1000, traded at 900, 100 refunded). Assert
    // received = held + consumed (ledger-derived).
    let buyer_consumed: i64 = world
        .resource::<TradeLedger>()
        .0
        .iter()
        .filter_map(|e| match e {
            EconomyEvent::FinalConsumed { actor, good, qty }
                if *actor == buyer && *good == GOOD_FOOD =>
            {
                Some(qty.0)
            }
            _ => None,
        })
        .sum();
    assert_eq!(
        world
            .resource::<InventoryBook>()
            .balance(buyer, GOOD_FOOD)
            .available
            .0
            + buyer_consumed,
        1_000,
        "buyer received 1000 FOOD (held + consumed by the sink)"
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
            consumed_qty_last_tick: Quantity::ZERO,
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

#[test]
fn shopper_capture_set_runs_after_macro_flow_before_materialize() {
    // Pin the ShopperCapture ordering edge against the REAL `install_systems` set
    // chain: recorder systems placed into MacroFlow / ShopperCapture / Materialize
    // must fire in exactly that order (it must run after MacroFlow so unmet demand
    // is current, and before Materialize so shoppers render same-tick).
    use crate::economy::{EconomyPlugin, systems::EconomySet};
    use bevy_ecs::prelude::*;

    #[derive(Resource, Default)]
    struct OrderLog(Vec<&'static str>);

    fn rec_macro_flow(mut log: ResMut<OrderLog>) {
        log.0.push("macro_flow");
    }
    fn rec_shopper_capture(mut log: ResMut<OrderLog>) {
        log.0.push("shopper_capture");
    }
    fn rec_materialize(mut log: ResMut<OrderLog>) {
        log.0.push("materialize");
    }

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    world.insert_resource(OrderLog::default());
    // Recorders inherit each set's position in the `.chain()` configured by
    // `install_systems`, so their relative run order reflects the real set edges.
    schedule.add_systems((
        rec_macro_flow.in_set(EconomySet::MacroFlow),
        rec_shopper_capture.in_set(EconomySet::ShopperCapture),
        rec_materialize.in_set(EconomySet::Materialize),
    ));

    schedule.run(&mut world);

    let log = &world.resource::<OrderLog>().0;
    let i_mf = log.iter().position(|s| *s == "macro_flow").unwrap();
    let i_sc = log.iter().position(|s| *s == "shopper_capture").unwrap();
    let i_mat = log.iter().position(|s| *s == "materialize").unwrap();
    assert!(
        i_mf < i_sc,
        "ShopperCapture must run AFTER MacroFlow (so unmet_demand is current): {log:?}"
    );
    assert!(
        i_sc < i_mat,
        "ShopperCapture must run BEFORE Materialize (so shoppers render same-tick): {log:?}"
    );
}

#[test]
fn regenerate_set_runs_after_expire_before_production() {
    // Pin EconomySet::Regenerate's position against the REAL install_systems chain:
    // recorder systems placed into ExpireOrders / Regenerate / Production must fire in
    // exactly that order (RAW deposited before the recipe can consume it same-tick).
    use crate::economy::{EconomyPlugin, systems::EconomySet};
    use bevy_ecs::prelude::*;

    #[derive(Resource, Default)]
    struct OrderLog(Vec<&'static str>);
    fn rec_expire(mut log: ResMut<OrderLog>) {
        log.0.push("expire");
    }
    fn rec_regen(mut log: ResMut<OrderLog>) {
        log.0.push("regen");
    }
    fn rec_production(mut log: ResMut<OrderLog>) {
        log.0.push("production");
    }

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    world.insert_resource(OrderLog::default());
    schedule.add_systems((
        rec_expire.in_set(EconomySet::ExpireOrders),
        rec_regen.in_set(EconomySet::Regenerate),
        rec_production.in_set(EconomySet::Production),
    ));
    schedule.run(&mut world);

    let log = &world.resource::<OrderLog>().0;
    let i_e = log.iter().position(|s| *s == "expire").unwrap();
    let i_r = log.iter().position(|s| *s == "regen").unwrap();
    let i_p = log.iter().position(|s| *s == "production").unwrap();
    assert!(i_e < i_r, "Regenerate must run AFTER ExpireOrders: {log:?}");
    assert!(i_r < i_p, "Regenerate must run BEFORE Production: {log:?}");
}

#[test]
fn raw_deposits_resource_is_installed_by_plugin() {
    use crate::economy::EconomyPlugin;
    use crate::economy::production::RawDeposits;
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    EconomyPlugin.install(&mut world, &mut schedule);
    assert!(world.get_resource::<RawDeposits>().is_some());
}

#[test]
fn regenerate_system_feeds_input_gated_production_same_tick() {
    // EXTRACTOR_TOOLS has a RAW faucet + a RAW->TOOLS recipe. After one schedule run, the RAW
    // deposited this tick is immediately consumed by the recipe → TOOLS appears; net RAW
    // on hand is bounded (deposit qty minus what the recipe took).
    use crate::economy::production::{
        EXTRACTOR_TOOLS, ProductionPool, ProductionPools, RawDeposit, RawDeposits, Recipe,
    };
    use crate::economy::{EconomyPlugin, GOOD_RAW, GOOD_TOOLS, InventoryBook, Quantity};

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    world.resource_mut::<RawDeposits>().0.insert(
        EXTRACTOR_TOOLS,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(100),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        EXTRACTOR_TOOLS,
        ProductionPool {
            actor: EXTRACTOR_TOOLS,
            recipe: Recipe {
                inputs: vec![(GOOD_RAW, Quantity(100))],
                outputs: vec![(GOOD_TOOLS, Quantity(100))],
            },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );

    schedule.run(&mut world);

    let inv = world.resource::<InventoryBook>();
    assert_eq!(
        inv.balance(EXTRACTOR_TOOLS, GOOD_TOOLS).available,
        Quantity(100),
        "Regenerate deposited RAW before Production consumed it (same-tick ordering)"
    );
    assert_eq!(
        inv.balance(EXTRACTOR_TOOLS, GOOD_RAW).available,
        Quantity(0),
        "the recipe drained the freshly-deposited RAW this tick"
    );
}

#[test]
fn pay_wages_then_profit_then_rebate_order_within_schedule() {
    // Pin the intra/inter-set ordering: wage-credit (PayWages) → profit-credit (PayWages,
    // .after wage) → rebate-credit (TransportRebate set, after PayWages, before Consume).
    // Recorders are anchored to the REAL systems via .after edges so the recorded order
    // reflects the real .after edges, not just set membership.
    use crate::economy::EconomyPlugin;
    use crate::economy::systems::{
        EconomySet, run_distribute_profit_system, run_pay_wages_system, run_transport_rebate_system,
    };
    use bevy_ecs::prelude::*;

    #[derive(Resource, Default)]
    struct OrderLog(Vec<&'static str>);
    fn rec_wages(mut log: ResMut<OrderLog>) {
        log.0.push("wages");
    }
    fn rec_profit(mut log: ResMut<OrderLog>) {
        log.0.push("profit");
    }
    fn rec_rebate(mut log: ResMut<OrderLog>) {
        log.0.push("rebate");
    }
    fn rec_consume(mut log: ResMut<OrderLog>) {
        log.0.push("consume");
    }

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    // The rebate system is macro-flow-modulo gated; force tick 0 so the gate is open
    // (0 is a multiple of macro_flow_interval_ticks) and the rebate recorder still has a
    // deterministic position even though the recorder itself is unconditional.
    world.insert_resource(crate::mobility::resources::Tick(0));

    world.insert_resource(OrderLog::default());
    schedule.add_systems((
        rec_wages
            .in_set(EconomySet::PayWages)
            .after(run_pay_wages_system)
            .before(run_distribute_profit_system),
        rec_profit
            .in_set(EconomySet::PayWages)
            .after(run_distribute_profit_system),
        rec_rebate
            .in_set(EconomySet::TransportRebate)
            .after(run_transport_rebate_system),
        rec_consume.in_set(EconomySet::Consume),
    ));
    schedule.run(&mut world);

    let log = &world.resource::<OrderLog>().0;
    let i_w = log.iter().position(|s| *s == "wages").unwrap();
    let i_p = log.iter().position(|s| *s == "profit").unwrap();
    let i_r = log.iter().position(|s| *s == "rebate").unwrap();
    let i_c = log.iter().position(|s| *s == "consume").unwrap();
    assert!(i_w < i_p, "profit credit AFTER wage credit: {log:?}");
    assert!(i_p < i_r, "rebate AFTER profit: {log:?}");
    assert!(i_r < i_c, "rebate BEFORE consume: {log:?}");
}

#[test]
fn price_adjust_config_defaults_and_validation() {
    use crate::economy::Money;
    use crate::economy::systems::EconomyConfig;
    let c = EconomyConfig::default();
    assert_eq!(c.price_adjust_k_bps, 500);
    assert_eq!(c.price_adjust_max_step_bps, 100);
    assert_eq!(c.price_floor, Money(1));
    assert_eq!(c.price_ceiling, Money(100_000));
    // Validated getters accept the defaults...
    assert_eq!(c.validated_price_adjust_k_bps().unwrap(), 500);
    assert_eq!(c.validated_price_adjust_max_step_bps().unwrap(), 100);
    assert_eq!(c.validated_price_band().unwrap(), (1, 100_000));
    // Inclusive boundary == 10_000 PASSES (mirrors validated_labor_share_bps).
    let edge_k = EconomyConfig {
        price_adjust_k_bps: 10_000,
        ..c
    };
    assert_eq!(edge_k.validated_price_adjust_k_bps().unwrap(), 10_000);
    // ...and reject out-of-band config (NO-FALLBACK: honest Err, no silent clamp).
    let bad_k = EconomyConfig {
        price_adjust_k_bps: 10_001,
        ..c
    };
    assert!(bad_k.validated_price_adjust_k_bps().is_err());
    let bad_step = EconomyConfig {
        price_adjust_max_step_bps: 10_001,
        ..c
    };
    assert!(bad_step.validated_price_adjust_max_step_bps().is_err());
    let bad_floor0 = EconomyConfig {
        price_floor: Money(0),
        ..c
    };
    assert!(
        bad_floor0.validated_price_band().is_err(),
        "floor must be > 0 (else ZeroPrice)"
    );
    let bad_order = EconomyConfig {
        price_floor: Money(100_000),
        price_ceiling: Money(1),
        ..c
    };
    assert!(
        bad_order.validated_price_band().is_err(),
        "floor must be < ceiling"
    );
}

#[test]
fn adjust_reservation_prices_fires_on_cadence_boundary_only() {
    use crate::economy::EconomyPlugin;
    use crate::economy::systems::{EconomySet, run_adjust_reservation_prices_system};
    use crate::economy::{
        AccountBook, DemandPool, DemandPools, EconomicActorId, GOOD_TOOLS, HouseholdSector,
        InventoryBook, MarketGoodKey, MarketGoodState, MarketGoods, MarketId, MarketSite, Markets,
        Money, Quantity, SupplyPool, SupplyPools,
    };
    use crate::mobility::resources::Tick;
    use crate::world::plugin::CorePlugin;
    use bevy_ecs::prelude::*;
    use std::collections::BTreeMap;

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let m = MarketId(1);
    let consumer = EconomicActorId(8_002);
    let supplier = EconomicActorId(8_001);
    world.resource_mut::<DemandPools>().0.insert(
        consumer,
        DemandPool {
            actor: consumer,
            market: m,
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
    world.resource_mut::<SupplyPools>().0.insert(
        supplier,
        SupplyPool {
            actor: supplier,
            market: m,
            good: GOOD_TOOLS,
            offered_qty_per_tick: Quantity(5),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world
        .resource_mut::<AccountBook>()
        .deposit(consumer, Money(10_000_000))
        .unwrap();
    world
        .resource_mut::<InventoryBook>()
        .deposit(supplier, GOOD_TOOLS, Quantity(1_000_000))
        .unwrap();
    world.resource_mut::<Markets>().0.insert(
        m,
        MarketSite {
            id: m,
            node_id: crate::routing::NodeId(0),
            name: "M1".to_string(),
        },
    );
    world.insert_resource(HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(consumer, 1_i64)]),
    });
    {
        let key = MarketGoodKey {
            market: m,
            good: GOOD_TOOLS,
        };
        let mut g = world.resource_mut::<MarketGoods>();
        let st = g.0.entry(key).or_insert_with(|| MarketGoodState::new(key));
        st.ewma_reference_price = Money(1_000);
        st.last_settlement_price = Money(1_000);
    }

    world.insert_resource(Tick(0));
    schedule.run(&mut world);
    let after_fire = world.resource::<DemandPools>().0[&consumer].max_price.0;
    assert!(
        after_fire > 2_000,
        "nudge fired on cadence boundary (tick 0), read post-clear unmet: {after_fire}"
    );

    world.insert_resource(Tick(3));
    let before_noop = world.resource::<DemandPools>().0[&consumer].max_price.0;
    schedule.run(&mut world);
    let after_noop = world.resource::<DemandPools>().0[&consumer].max_price.0;
    assert_eq!(
        after_noop, before_noop,
        "no nudge off the cadence boundary (tick 3)"
    );

    let _ = (
        run_adjust_reservation_prices_system,
        EconomySet::AdjustReservationPrices,
    );
}
