use bevy_ecs::prelude::*;

use crate::economy::seed::seed_demo_economy;
use crate::economy::{
    AccountBook, DemandPools, InventoryBook, MarketChunks, MarketId, Markets, SupplyPools, Traders,
};
use crate::routing::{Graph, Node, NodeId, NodeKind, NodeSpatialIndex};

fn node(id: u32, x: f32, y: f32) -> Node {
    Node {
        id: NodeId(id),
        position: (x, y),
        kind: NodeKind::Intersection,
        legacy_id: None,
    }
}

/// Build the same fresh world the seed tests use, with the two footway nodes
/// near the seeder's reference points (2,3) and (13,3) and all economy
/// resources empty so `seed_demo_economy` performs its one-shot bootstrap.
fn seed_world() -> World {
    let mut world = World::new();
    let nodes = vec![node(0, 2.0, 3.0), node(1, 13.0, 3.0)];
    world.insert_resource(NodeSpatialIndex::from_nodes(&nodes));
    world.insert_resource(Graph::new(nodes, vec![]));
    world.insert_resource(Markets::default());
    world.insert_resource(MarketChunks::default());
    world.insert_resource(AccountBook::default());
    world.insert_resource(InventoryBook::default());
    world.insert_resource(SupplyPools::default());
    world.insert_resource(DemandPools::default());
    world.insert_resource(Traders::default());
    world.insert_resource(crate::economy::MarketDistances::default());
    world
}

#[test]
fn seed_demo_economy_creates_two_markets_and_one_trader() {
    let mut world = seed_world();

    seed_demo_economy(&mut world);

    assert_eq!(world.resource::<Markets>().0.len(), 2, "two demo markets");
    assert_eq!(world.resource::<MarketChunks>().0.len(), 2, "both anchored");
    assert_eq!(world.resource::<Traders>().0.len(), 1, "one demo trader");

    // Market nodes resolve to finite positions, and the two are distinct.
    let graph = world.resource::<Graph>();
    let markets = world.resource::<Markets>();
    let mut node_ids = Vec::new();
    for site in markets.0.values() {
        let p = graph.node(site.node_id).position;
        assert!(p.0.is_finite() && p.1.is_finite());
        node_ids.push(site.node_id);
    }
    assert_ne!(
        node_ids[0], node_ids[1],
        "source and dest are distinct nodes"
    );

    let distances = world.resource::<crate::economy::MarketDistances>();
    assert_eq!(distances.0.len(), 2, "both directed pairs baked");
    assert_eq!(
        distances
            .0
            .get(&(MarketId(9_001), MarketId(9_002)))
            .copied(),
        Some(11)
    );
    assert_eq!(
        distances
            .0
            .get(&(MarketId(9_002), MarketId(9_001)))
            .copied(),
        Some(11)
    );
}

#[test]
fn seed_adds_second_good_without_new_markets_or_traders() {
    // After seeding, the live economy still has exactly 2 markets and 1 trader,
    // but now a GOOD_FOOD supplier@m_a + consumer@m_b exists so the macro flow
    // produces a non-vacuous cross-market FOOD flow on the live stream.
    use crate::economy::GOOD_FOOD;
    let mut world = seed_world();
    seed_demo_economy(&mut world);
    assert_eq!(
        world.resource::<Markets>().0.len(),
        2,
        "still exactly 2 markets"
    );
    assert_eq!(
        world.resource::<Traders>().0.len(),
        1,
        "still exactly 1 trader"
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
