//! Blocker-2 evidence: run the REAL abutopia economy long enough to see whether the
//! free-price tâtonnement keeps the SUPPLIED demand market 9002 healthy (consuming,
//! price in-band) or lets it collapse to the ceiling. Extends the free-prices spec's
//! stability Test #10 to a long-run abutopia scenario. If 9002 collapses, that
//! contradicts the spec's stability guarantee and is a genuine bug (escalate).

use bevy_ecs::prelude::*;

use crate::economy::{
    AccountBook, EconomyConfig, EconomyPlugin, GOOD_FOOD, GOOD_TOOLS, MarketGoods, MarketId,
};
use crate::routing::{Graph, Node, NodeId, NodeKind, NodeSpatialIndex};
use crate::world::plugin::CorePlugin;
use crate::world::schedule::SimPlugin;

fn node(id: u32, x: f32, y: f32) -> Node {
    Node {
        id: NodeId(id),
        position: (x, y),
        kind: NodeKind::Intersection,
        legacy_id: None,
    }
}

/// Build the real abutopia economy with a RUNNABLE schedule (seed_world recipe + the
/// capita run pattern: CorePlugin + MobilityPlugin + EconomyPlugin so the schedule
/// advances and Tick exists). 4-node graph at the market anchors (9002 @ 111.5,64.51).
fn build_abutopia_economy() -> (World, bevy_ecs::schedule::Schedule) {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let nodes = vec![
        node(0, 2.0, 3.0),
        node(1, 111.5, 64.51),
        node(2, 16.0, 48.0),
        node(3, 208.0, 48.0),
    ];
    world.insert_resource(NodeSpatialIndex::from_nodes(&nodes));
    world.insert_resource(Graph::new(nodes, vec![]));

    let bundle = crate::base_world::BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia")
        .expect("abutopia bundle loads");
    crate::economy::seed_from_markets_layer(&mut world, &bundle.markets);
    (world, schedule)
}

/// Sum market 9002's consumed_qty_last_tick across its two demand goods.
fn consumed_9002(world: &World) -> i64 {
    let goods = world.resource::<MarketGoods>();
    let mut total = 0i64;
    for g in [GOOD_TOOLS, GOOD_FOOD] {
        if let Some(st) = goods.0.get(&crate::economy::MarketGoodKey {
            market: MarketId(9002),
            good: g,
        }) {
            total += st.consumed_qty_last_tick.0;
        }
    }
    total
}

/// Max ewma_reference_price across 9002's demand goods (the divergence signal).
fn price_9002(world: &World) -> i64 {
    let goods = world.resource::<MarketGoods>();
    let mut max = 0i64;
    for g in [GOOD_TOOLS, GOOD_FOOD] {
        if let Some(st) = goods.0.get(&crate::economy::MarketGoodKey {
            market: MarketId(9002),
            good: g,
        }) {
            max = max.max(st.ewma_reference_price.0);
        }
    }
    max
}

/// Max ewma_reference_price across 9001's supply goods (the spatial LoOP source price).
fn price_9001(world: &World) -> i64 {
    let goods = world.resource::<MarketGoods>();
    let mut max = 0i64;
    for g in [GOOD_TOOLS, GOOD_FOOD] {
        if let Some(st) = goods.0.get(&crate::economy::MarketGoodKey {
            market: MarketId(9001),
            good: g,
        }) {
            max = max.max(st.ewma_reference_price.0);
        }
    }
    max
}

#[test]
fn abutopia_prices_stay_in_band_and_9002_consumes_over_long_run() {
    const N: u64 = 2000; // 200 tâtonnement cadences (macro_flow_interval_ticks = 10)
    let (mut world, mut schedule) = build_abutopia_economy();

    let money_before = world.resource::<AccountBook>().total_money().unwrap();
    let config = *world.resource::<EconomyConfig>();

    let mut consumed_first_half = 0i64;
    let mut consumed_last_quarter = 0i64;
    let mut peak_price_9002 = 0i64;

    for i in 0..N {
        schedule.run(&mut world);
        world.resource_mut::<crate::mobility::resources::Tick>().0 += 1;

        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            money_before,
            "total_money byte-invariant at tick {i}"
        );

        for (key, st) in world.resource::<MarketGoods>().0.iter() {
            assert!(
                st.ewma_reference_price >= config.price_floor
                    && st.ewma_reference_price <= config.price_ceiling,
                "price out of band at tick {i}: {:?} {:?} price={:?}",
                key.market,
                key.good,
                st.ewma_reference_price
            );
        }

        let c = consumed_9002(&world);
        let p = price_9002(&world);
        peak_price_9002 = peak_price_9002.max(p);
        if i < N / 2 {
            consumed_first_half += c;
        }
        if i >= N - N / 4 {
            consumed_last_quarter += c;
        }
    }

    let final_price_9002 = price_9002(&world);
    let p_src = price_9001(&world);
    let rate = config.transport_cost_per_tile_unit.0;
    let dist = 172i64; // rounded Manhattan of 9001 (2,3) ↔ 9002 (111.5,64.51): |2-112|+|3-65|
    let loop_target = p_src + rate * dist;
    println!(
        "ABUTOPIA STABILITY: consumed_first_half={consumed_first_half} \
         consumed_last_quarter={consumed_last_quarter} peak_price_9002={peak_price_9002} \
         final_price_9002={final_price_9002} ceiling={}",
        config.price_ceiling.0
    );
    println!(
        "CONVERGENCE: final_9002={final_price_9002} p_9001={p_src} loop_target={loop_target} \
         (rate*dist={})",
        rate * dist
    );

    assert!(
        consumed_last_quarter > 0,
        "market 9002 must keep consuming over the long run (no collapse); \
         consumed_last_quarter={consumed_last_quarter}, consumed_first_half={consumed_first_half}"
    );

    // Spatial LoOP convergence: 9002's final price must be near p_9001 + rate·dist
    // (Samuelson 1952). ±25% band is non-vacuous (the unnudged price would ratchet to
    // ~99_961, the ceiling; even peak=1938 shows the nudge is working; the band excludes
    // the failure mode and is loose enough not to be flaky across minor EWMA drift).
    assert!(
        (final_price_9002 - loop_target).abs() <= loop_target / 4,
        "9002 must CONVERGE to the spatial-LoOP target p_9001 + rate*dist (Samuelson 1952); \
         final_9002={final_price_9002}, loop_target={loop_target} (p_9001={p_src}, rate*dist={})",
        rate * dist
    );
    assert!(
        final_price_9002 < config.price_ceiling.0 / 10,
        "and stays far below the ceiling"
    );
}
