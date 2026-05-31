//! Seed a tiny, data-driven demo economy into the live world so a trader is
//! actually visible. Seeded ONCE on fresh-world creation only — the economy
//! persists (`EconomyPersistSnapshot`), so a hydrated world restores the demo
//! economy from persistence (no re-seed, no double-seed guard). No hardcoded
//! coordinates: market nodes are snapped from the real footway graph to two
//! reference points near the default view.

use bevy_ecs::prelude::*;

use crate::economy::transport::manhattan_tiles;
use crate::economy::{
    AccountBook, DemandPool, DemandPools, EconomicActorId, GOOD_TOOLS, InventoryBook, MarketChunks,
    MarketId, MarketSite, Markets, Money, Quantity, SupplyPool, SupplyPools, Trader, TraderState,
    Traders,
};
use crate::routing::{Graph, NodeSpatialIndex};

/// Reference points near the abutopia default view (corridor ends). The seeder
/// snaps each to the nearest real footway node — no coordinate is baked into the
/// graph; we only express "near here".
const REF_A: (f32, f32) = (2.0, 3.0);
const REF_B: (f32, f32) = (13.0, 3.0);

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
}
