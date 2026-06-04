use bevy_ecs::prelude::*;

use crate::economy::seed::seed_demo_economy;
use crate::economy::{
    AccountBook, DemandPools, InventoryBook, MarketChunks, MarketGoods, MarketId, Markets,
    SupplyPools,
};
use crate::ids::ChunkCoord;
use crate::routing::{Graph, Node, NodeId, NodeKind, NodeSpatialIndex};

fn node(id: u32, x: f32, y: f32) -> Node {
    Node {
        id: NodeId(id),
        position: (x, y),
        kind: NodeKind::Intersection,
        legacy_id: None,
    }
}

/// Build a fresh world with four footway nodes: two near the original seeder
/// reference points (2,3) and (13,3), and two near the flow-demo reference
/// points (16,48) and (208,48) for the dormant cross-market pair.
fn seed_world() -> World {
    let mut world = World::new();
    let nodes = vec![
        node(0, 2.0, 3.0),
        node(1, 13.0, 3.0),
        node(2, 16.0, 48.0),
        node(3, 208.0, 48.0),
    ];
    world.insert_resource(NodeSpatialIndex::from_nodes(&nodes));
    world.insert_resource(Graph::new(nodes, vec![]));
    world.insert_resource(Markets::default());
    world.insert_resource(MarketChunks::default());
    world.insert_resource(AccountBook::default());
    world.insert_resource(InventoryBook::default());
    world.insert_resource(SupplyPools::default());
    world.insert_resource(DemandPools::default());
    world.insert_resource(crate::economy::MarketDistances::default());
    world.insert_resource(MarketGoods::default());
    world.insert_resource(crate::economy::production::ProductionPools::default());
    world.insert_resource(crate::economy::production::RawDeposits::default());
    world
}

#[test]
fn seed_demo_economy_creates_four_markets() {
    let mut world = seed_world();

    seed_demo_economy(&mut world);

    assert_eq!(world.resource::<Markets>().0.len(), 4, "four demo markets");
    assert_eq!(world.resource::<MarketChunks>().0.len(), 4, "all anchored");

    // The original demo markets still exist.
    let distances = world.resource::<crate::economy::MarketDistances>();
    assert!(
        distances
            .0
            .contains_key(&(MarketId(9_001), MarketId(9_002))),
        "original demo pair baked"
    );

    // The flow-demo markets have distance entries in both directions.
    assert!(
        distances
            .0
            .contains_key(&(MarketId(9_003), MarketId(9_004))),
        "flow-demo A->B distance baked"
    );
    assert!(
        distances
            .0
            .contains_key(&(MarketId(9_004), MarketId(9_003))),
        "flow-demo B->A distance baked"
    );
}

#[test]
fn seed_adds_second_good_without_new_markets() {
    // After seeding, the live economy still has exactly 4 markets, and a
    // GOOD_FOOD supplier@m_a + consumer@m_b exists so the macro flow
    // produces a non-vacuous cross-market FOOD flow on the live stream.
    use crate::economy::GOOD_FOOD;
    let mut world = seed_world();
    seed_demo_economy(&mut world);
    assert_eq!(
        world.resource::<Markets>().0.len(),
        4,
        "still exactly 4 markets"
    );
    let has_food_supply = world
        .resource::<SupplyPools>()
        .0
        .values()
        .any(|p| p.good == GOOD_FOOD);
    let has_food_demand = world
        .resource::<DemandPools>()
        .0
        .values()
        .any(|p| p.good == GOOD_FOOD);
    assert!(
        has_food_supply && has_food_demand,
        "FOOD supplier@A + consumer@B added"
    );
}

#[test]
fn seed_adds_flow_demo_markets_for_dormant_cross_flow() {
    // Task 7: two far-apart dormant markets (F_A @ chunk (0,1), F_B @ chunk
    // (6,1)) with a standing GOOD_FOOD imbalance → recurring MacroFlow whose
    // straight-line route at row y≈48 crosses the transit chunk (3,1).
    // Both market chunks must be ≥2 chunks from the transit chunk in x.
    use crate::economy::GOOD_FOOD;
    let mut world = seed_world();
    seed_demo_economy(&mut world);

    // The two flow-demo markets exist.
    let markets = world.resource::<Markets>();
    assert!(
        markets.0.contains_key(&MarketId(9_003)),
        "flow-demo market F_A seeded"
    );
    assert!(
        markets.0.contains_key(&MarketId(9_004)),
        "flow-demo market F_B seeded"
    );

    // Both have entries in MarketChunks.
    let chunks = world.resource::<MarketChunks>();
    let chunk_fa = *chunks.0.get(&MarketId(9_003)).expect("F_A chunk");
    let chunk_fb = *chunks.0.get(&MarketId(9_004)).expect("F_B chunk");

    // Chunks are distinct.
    assert_ne!(chunk_fa, chunk_fb, "F_A and F_B in different chunks");

    // Both chunks differ from the transit chunk (3,1) by ≥2 in x.
    let transit = ChunkCoord { x: 3, y: 1 };
    assert!(
        (chunk_fa.x - transit.x).unsigned_abs() >= 2,
        "F_A chunk {:?} must be ≥2 chunks from transit {:?}",
        chunk_fa,
        transit
    );
    assert!(
        (chunk_fb.x - transit.x).unsigned_abs() >= 2,
        "F_B chunk {:?} must be ≥2 chunks from transit {:?}",
        chunk_fb,
        transit
    );

    // Supplier pool at F_A and consumer pool at F_B for GOOD_FOOD.
    let supply = world.resource::<SupplyPools>();
    let demand = world.resource::<DemandPools>();
    assert!(
        supply
            .0
            .values()
            .any(|p| p.market == MarketId(9_003) && p.good == GOOD_FOOD),
        "FOOD supplier at F_A"
    );
    assert!(
        demand
            .0
            .values()
            .any(|p| p.market == MarketId(9_004) && p.good == GOOD_FOOD),
        "FOOD consumer at F_B"
    );

    // MarketDistances entry in both directions.
    let distances = world.resource::<crate::economy::MarketDistances>();
    assert!(
        distances
            .0
            .get(&(MarketId(9_003), MarketId(9_004)))
            .copied()
            .unwrap_or(0)
            > 0,
        "F_A->F_B distance > 0"
    );
    assert!(
        distances
            .0
            .get(&(MarketId(9_004), MarketId(9_003)))
            .copied()
            .unwrap_or(0)
            > 0,
        "F_B->F_A distance > 0"
    );
}

#[test]
fn seed_installs_three_extractors_tools_and_two_food_but_never_lists_raw() {
    use crate::economy::production::{
        EXTRACTOR_FOOD_A, EXTRACTOR_FOOD_FA, EXTRACTOR_TOOLS, ProductionPools, RawDeposits,
    };
    use crate::economy::{
        GOOD_FOOD, GOOD_RAW, GOOD_TOOLS, HouseholdSector, InventoryBook, MarketId,
    };

    let mut world = seed_world();
    seed_demo_economy(&mut world);

    // (market, output-good) expected for each extractor.
    let expected = [
        (EXTRACTOR_TOOLS, MarketId(9_001), GOOD_TOOLS),  // m_a
        (EXTRACTOR_FOOD_A, MarketId(9_001), GOOD_FOOD),  // m_a
        (EXTRACTOR_FOOD_FA, MarketId(9_003), GOOD_FOOD), // m_fa
    ];
    for (actor, market, out_good) in expected {
        let dep = world.resource::<RawDeposits>().0[&actor];
        assert_eq!(dep.good, GOOD_RAW, "{actor:?} faucets RAW");
        assert_eq!(dep.qty_per_interval.0, 10, "{actor:?} faucet rate 10");
        assert_eq!(dep.interval_ticks, 1);

        let pool = world.resource::<ProductionPools>().0[&actor].clone();
        assert_eq!(
            pool.recipe.inputs,
            vec![(GOOD_RAW, dep.qty_per_interval)],
            "{actor:?} consumes RAW"
        );
        assert_eq!(pool.recipe.outputs.len(), 1);
        assert_eq!(
            pool.recipe.outputs[0].0, out_good,
            "{actor:?} outputs the right good"
        );

        let sp = world.resource::<SupplyPools>().0[&actor];
        assert_eq!(sp.good, out_good, "{actor:?} sells its output good");
        assert_eq!(sp.market, market, "{actor:?} sells at the right market");
        assert_eq!(sp.offered_qty_per_tick.0, 10);

        assert!(
            world
                .resource::<InventoryBook>()
                .balance(actor, GOOD_RAW)
                .available
                .0
                > 0,
            "{actor:?} holds opening RAW so production fires on tick 0"
        );
        assert!(
            !world
                .resource::<HouseholdSector>()
                .pool_weights
                .contains_key(&actor),
            "{actor:?} is a firm, not a labor household"
        );
    }

    // GOOD_RAW is NEVER on any SupplyPool or DemandPool (structural non-tradability).
    assert!(
        world
            .resource::<SupplyPools>()
            .0
            .values()
            .all(|p| p.good != GOOD_RAW),
        "RAW never on a SupplyPool"
    );
    assert!(
        world
            .resource::<DemandPools>()
            .0
            .values()
            .all(|p| p.good != GOOD_RAW),
        "RAW never on a DemandPool"
    );
}
