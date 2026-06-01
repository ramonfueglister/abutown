//! Seed a tiny, data-driven demo economy into the live world so a trader is
//! actually visible. Seeded ONCE on fresh-world creation only — the economy
//! persists (`EconomyPersistSnapshot`), so a hydrated world restores the demo
//! economy from persistence (no re-seed, no double-seed guard). No hardcoded
//! coordinates: market nodes are snapped from the real footway graph to two
//! reference points near the default view.

use bevy_ecs::prelude::*;

use crate::economy::transport::manhattan_tiles;
use crate::economy::{
    AccountBook, DemandPool, DemandPools, EconomicActorId, GOOD_FOOD, GOOD_TOOLS, InventoryBook,
    MarketChunks, MarketDistances, MarketId, MarketSite, Markets, Money, Quantity, SupplyPool,
    SupplyPools, Trader, TraderState, Traders,
};
use crate::routing::{Graph, NodeSpatialIndex};

/// Reference points near the abutopia default view (corridor ends). The seeder
/// snaps each to the nearest real footway node — no coordinate is baked into the
/// graph; we only express "near here".
const REF_A: (f32, f32) = (2.0, 3.0);
const REF_B: (f32, f32) = (13.0, 3.0);

/// Reference points for the dormant flow-demo market pair (Task 7).  Both sit
/// at grass-row y≈48 (chunk row 1), far apart in x so their route crosses the
/// transit chunk (3,1) while both market chunks stay ≥2 chunks from it.
/// F_A @ tile (16,48) → chunk (0,1); F_B @ tile (208,48) → chunk (6,1).
const REF_FA: (f32, f32) = (16.0, 48.0);
const REF_FB: (f32, f32) = (208.0, 48.0);

/// Seed two anchored markets, a supplier, a consumer, and one trader cycling
/// between them. Requires `Graph` + `NodeSpatialIndex` (the spatial seeder runs
/// after `RoutingPlugin`). No-ops only if the graph is too small to host two
/// distinct reachable nodes.
pub fn seed_demo_economy(world: &mut World) {
    // Idempotent bootstrap: seed only when the world has no economy yet — a
    // brand-new world, or one created before the economy existed. Once seeded it
    // persists, so subsequent hydrates find markets and skip. This (not a heal-on-
    // restore shim) is what puts the demo trader into the always-hydrated live
    // server; re-seeding would duplicate markets and reset trader progress.
    if !world.resource::<Markets>().0.is_empty() {
        return;
    }
    let (node_a, node_b) = {
        let spatial = world.resource::<NodeSpatialIndex>();
        match (spatial.nearest(REF_A), spatial.nearest(REF_B)) {
            (Some(a), Some(b)) if a != b => (a, b),
            _ => return, // graph too small to host a demo economy
        }
    };
    let (chunk_a, chunk_b, dist) = {
        let graph = world.resource::<Graph>();
        let pa = graph.node(node_a).position;
        let pb = graph.node(node_b).position;
        (
            crate::mobility::chunk_of(pa.0, pa.1, 32),
            crate::mobility::chunk_of(pb.0, pb.1, 32),
            manhattan_tiles(graph, node_a, node_b),
        )
    };

    let (m_a, m_b) = (MarketId(9_001), MarketId(9_002));
    {
        let mut markets = world.resource_mut::<Markets>();
        markets.0.insert(
            m_a,
            MarketSite {
                id: m_a,
                node_id: node_a,
                name: "Demo A".to_string(),
            },
        );
        markets.0.insert(
            m_b,
            MarketSite {
                id: m_b,
                node_id: node_b,
                name: "Demo B".to_string(),
            },
        );
    }
    {
        let mut anchors = world.resource_mut::<MarketChunks>();
        anchors.0.insert(m_a, chunk_a);
        anchors.0.insert(m_b, chunk_b);
    }
    {
        let mut distances = world.resource_mut::<MarketDistances>();
        distances.0.insert((m_a, m_b), dist);
        distances.0.insert((m_b, m_a), dist);
    }

    let supplier = EconomicActorId(8_001);
    let consumer = EconomicActorId(8_002);
    let trader_actor = EconomicActorId(8_003);
    {
        let mut accounts = world.resource_mut::<AccountBook>();
        accounts
            .deposit(consumer, Money(1_000_000))
            .expect("seed: consumer cash");
        accounts
            .deposit(trader_actor, Money(1_000_000))
            .expect("seed: trader cash");
    }
    world
        .resource_mut::<InventoryBook>()
        .deposit(supplier, GOOD_TOOLS, Quantity(1_000_000))
        .expect("seed: supplier goods");

    world.resource_mut::<SupplyPools>().0.insert(
        supplier,
        SupplyPool {
            actor: supplier,
            market: m_a,
            good: GOOD_TOOLS,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<DemandPools>().0.insert(
        consumer,
        DemandPool {
            actor: consumer,
            market: m_b,
            good: GOOD_TOOLS,
            desired_qty_per_tick: Quantity(10),
            max_price: Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    // Second good (FOOD): a cheap supplier at m_a and a dear consumer at m_b,
    // reusing the SAME two markets. This adds POOLS, not markets/traders, so the
    // live macro flow shows a non-vacuous cross-market FOOD MacroFlow without
    // enlarging the world (Markets.len()==2, Traders.len()==1 still hold).
    let food_supplier = EconomicActorId(8_011);
    let food_consumer = EconomicActorId(8_012);
    world
        .resource_mut::<InventoryBook>()
        .deposit(food_supplier, GOOD_FOOD, Quantity(1_000_000))
        .expect("seed: food supplier goods");
    world
        .resource_mut::<AccountBook>()
        .deposit(food_consumer, Money(1_000_000))
        .expect("seed: food consumer cash");
    world.resource_mut::<SupplyPools>().0.insert(
        food_supplier,
        SupplyPool {
            actor: food_supplier,
            market: m_a,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<DemandPools>().0.insert(
        food_consumer,
        DemandPool {
            actor: food_consumer,
            market: m_b,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(10),
            max_price: Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<Traders>().0.insert(
        trader_actor,
        Trader {
            actor: trader_actor,
            good: GOOD_TOOLS,
            source: m_a,
            dest: m_b,
            distance_tiles: dist,
            batch_qty: Quantity(5),
            buy_premium_bps: 500,
            sell_discount_bps: 500,
            order_ttl_ticks: 20,
            state: TraderState::Buying { order: None },
        },
    );

    // ── Task 7: dormant flow-demo market pair ────────────────────────────────
    // F_A @ tile ≈(16,48) chunk (0,1); F_B @ tile ≈(208,48) chunk (6,1).
    // Both are ≥3 chunks from the transit chunk (3,1) → never pulled Active
    // by a 3×3+ring subscription centred on (3,1). The straight-line grass
    // route at y≈48 crosses (3,1), so a flow-shipment is visible there.
    // Avoids the pinned chunk (3,2). Standing GOOD_FOOD imbalance (supply@FA,
    // demand@FB) → recurring MacroFlow every macro_flow_interval_ticks.
    let (node_fa, node_fb) = {
        let spatial = world.resource::<NodeSpatialIndex>();
        match (spatial.nearest(REF_FA), spatial.nearest(REF_FB)) {
            (Some(fa), Some(fb)) if fa != fb => (fa, fb),
            _ => return, // graph too small; skip flow-demo markets
        }
    };
    let (chunk_fa, chunk_fb, dist_fa_fb) = {
        let graph = world.resource::<Graph>();
        let pfa = graph.node(node_fa).position;
        let pfb = graph.node(node_fb).position;
        (
            crate::mobility::chunk_of(pfa.0, pfa.1, 32),
            crate::mobility::chunk_of(pfb.0, pfb.1, 32),
            manhattan_tiles(graph, node_fa, node_fb),
        )
    };
    let (m_fa, m_fb) = (MarketId(9_003), MarketId(9_004));
    {
        let mut markets = world.resource_mut::<Markets>();
        markets.0.insert(
            m_fa,
            MarketSite {
                id: m_fa,
                node_id: node_fa,
                name: "Flow Demo A".to_string(),
            },
        );
        markets.0.insert(
            m_fb,
            MarketSite {
                id: m_fb,
                node_id: node_fb,
                name: "Flow Demo B".to_string(),
            },
        );
    }
    {
        let mut anchors = world.resource_mut::<MarketChunks>();
        anchors.0.insert(m_fa, chunk_fa);
        anchors.0.insert(m_fb, chunk_fb);
    }
    {
        let mut distances = world.resource_mut::<MarketDistances>();
        distances.0.insert((m_fa, m_fb), dist_fa_fb);
        distances.0.insert((m_fb, m_fa), dist_fa_fb);
    }
    // Flow actors: supplier at F_A, consumer at F_B for GOOD_FOOD.
    // Reuses the second good (GOOD_FOOD) introduced in Slice 1 (ids 8_021/8_022
    // are fresh — existing pools use 8_011/8_012 at m_a/m_b).
    let flow_supplier = EconomicActorId(8_021);
    let flow_consumer = EconomicActorId(8_022);
    world
        .resource_mut::<InventoryBook>()
        .deposit(flow_supplier, GOOD_FOOD, Quantity(1_000_000))
        .expect("seed: flow-demo food supplier goods");
    world
        .resource_mut::<AccountBook>()
        .deposit(flow_consumer, Money(1_000_000))
        .expect("seed: flow-demo food consumer cash");
    world.resource_mut::<SupplyPools>().0.insert(
        flow_supplier,
        SupplyPool {
            actor: flow_supplier,
            market: m_fa,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<DemandPools>().0.insert(
        flow_consumer,
        DemandPool {
            actor: flow_consumer,
            market: m_fb,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(10),
            max_price: Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
}
