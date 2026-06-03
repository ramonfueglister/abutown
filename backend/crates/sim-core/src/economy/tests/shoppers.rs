use crate::economy::{EconomyPlugin, apply_into_world, extract_from_world};
use crate::economy::{GoodId, MarketId, NextShopperId, ShopperVisit, ShopperVisits};
use crate::routing::NodeId;

#[test]
fn id_prefix_distinguishes_shoppers_from_traders() {
    use crate::economy::EconomicActorId;
    use crate::economy::flow_shipments::SHIPMENT_ACTOR_OFFSET;
    use crate::economy::materialize::id_prefix;
    use crate::economy::shoppers::SHOPPER_ACTOR_OFFSET;
    assert_eq!(id_prefix(EconomicActorId(8003)), "trader:");
    assert_eq!(
        id_prefix(EconomicActorId(SHIPMENT_ACTOR_OFFSET + 1)),
        "trader:"
    );
    assert_eq!(
        id_prefix(EconomicActorId(SHOPPER_ACTOR_OFFSET + 1)),
        "shopper:"
    );
}

#[test]
fn economy_config_has_shopper_tuning_defaults() {
    let c = crate::economy::EconomyConfig::default();
    assert!(c.shoppers_per_unit >= 1);
    assert!(c.max_shoppers_per_market >= 1);
    assert!(c.shopper_radius_tiles > 0.0);
}

#[test]
fn shopper_progress_arrival_and_id() {
    let v = ShopperVisit {
        id: 0,
        market: MarketId(1),
        good: GoodId(0),
        origin_node: NodeId(7),
        start_tick: 100,
        travel_ticks: 10,
    };
    assert_eq!(v.progress(105), 0.5);
    assert!(!v.arrived(109));
    assert!(v.arrived(110));
    let mut n = NextShopperId::default();
    assert_eq!((n.next(), n.next()), (0, 1));
    assert_eq!(ShopperVisits::default().0.len(), 0);
}

#[test]
fn capture_spawns_proportional_to_consumption_deterministically() {
    use crate::economy::shoppers::capture_shopper_visits;
    use crate::economy::{
        EconomyConfig, MarketGoodKey, MarketGoodState, MarketGoods, MarketSite, Markets, Quantity,
    };
    use std::collections::BTreeSet;

    // One observed market m=1 (node 10) that consumed 9 of good 0 last tick.
    // cap = 4, per_unit = 3 -> target = min(9/3, 4) = 3 visits.
    let m = MarketId(1);
    let good = GoodId(0);
    let key = MarketGoodKey { market: m, good };
    let mut mg = MarketGoods::default();
    let mut st = MarketGoodState::new(key);
    st.consumed_qty_last_tick = Quantity(9);
    mg.0.insert(key, st);

    let mut markets = Markets::default();
    markets.0.insert(
        m,
        MarketSite {
            id: m,
            node_id: NodeId(10),
            name: "M".to_string(),
        },
    );

    // Deterministic sorted origin provider (already sorted by NodeId, market-node
    // excluded, walkable). Returns (origin_node, manhattan_dist_to_market). The
    // system wrapper builds this from NodeSpatialIndex::within_radius (sorted) +
    // manhattan_tiles + a Walk-route filter; the unit test pins it directly.
    let origins = |market_node: NodeId| -> Vec<(NodeId, i64)> {
        assert_eq!(market_node, NodeId(10));
        vec![
            (NodeId(1), 4),
            (NodeId(2), 8),
            (NodeId(3), 12),
            (NodeId(4), 16),
        ]
    };

    let observed: BTreeSet<MarketId> = [m].into_iter().collect();
    let config = EconomyConfig::default();
    // `&F where F: Fn` itself implements `Fn`; binding the borrow once lets both
    // calls reuse the same provider (the closure is non-Copy).
    let origins = &origins;

    let mut visits = ShopperVisits::default();
    let mut next = NextShopperId::default();
    capture_shopper_visits(
        &mg,
        &observed,
        &markets,
        origins,
        &config,
        0,
        &mut visits,
        &mut next,
    );
    assert_eq!(visits.0.len(), 3, "min(9/3, 4) = 3 visits");
    // every visit walks toward the observed market, with a positive travel time.
    for v in visits.0.values() {
        assert_eq!(v.market, m);
        assert_eq!(v.good, good);
        assert_eq!(v.start_tick, 0);
        assert!(v.travel_ticks >= 1);
        assert_ne!(
            v.origin_node,
            NodeId(10),
            "origin must not be the market node"
        );
    }

    // Determinism: same inputs -> same (market, origin_node) visit set.
    let mut visits2 = ShopperVisits::default();
    let mut next2 = NextShopperId::default();
    capture_shopper_visits(
        &mg,
        &observed,
        &markets,
        origins,
        &config,
        0,
        &mut visits2,
        &mut next2,
    );
    assert_eq!(
        visits
            .0
            .values()
            .map(|v| (v.market, v.origin_node))
            .collect::<Vec<_>>(),
        visits2
            .0
            .values()
            .map(|v| (v.market, v.origin_node))
            .collect::<Vec<_>>(),
    );
    // First-N sorted candidates were taken (1,2,3), not (2,3,4) or unsorted.
    let mut taken: Vec<NodeId> = visits.0.values().map(|v| v.origin_node).collect();
    taken.sort_by_key(|n| n.0);
    assert_eq!(taken, vec![NodeId(1), NodeId(2), NodeId(3)]);
}

/// 6.2 — Two independent runs of the capture logic with identical inputs produce
/// identical ShopperVisits and identical NextShopperId state.  The Task-4 test
/// already verifies the (market, origin_node) pairs; this test additionally
/// confirms the full visit structs (including ids and travel_ticks) are equal.
#[test]
fn shopper_capture_is_deterministic() {
    use crate::economy::shoppers::capture_shopper_visits;
    use crate::economy::{
        EconomyConfig, MarketGoodKey, MarketGoodState, MarketGoods, MarketSite, Markets, Quantity,
    };
    use std::collections::BTreeSet;

    let m = MarketId(5);
    let good = GoodId(1);
    let key = MarketGoodKey { market: m, good };
    let mut mg = MarketGoods::default();
    let mut st = MarketGoodState::new(key);
    st.consumed_qty_last_tick = Quantity(6);
    mg.0.insert(key, st);

    let mut markets = Markets::default();
    markets.0.insert(
        m,
        MarketSite {
            id: m,
            node_id: NodeId(20),
            name: "Det".to_string(),
        },
    );

    let origins = |_market_node: NodeId| -> Vec<(NodeId, i64)> {
        vec![(NodeId(11), 5), (NodeId(12), 10), (NodeId(13), 15)]
    };
    let origins = &origins;

    let config = EconomyConfig::default(); // shoppers_per_unit=3, max=4 => target=2
    let observed: BTreeSet<MarketId> = [m].into_iter().collect();

    let mut visits1 = ShopperVisits::default();
    let mut next1 = NextShopperId::default();
    capture_shopper_visits(
        &mg,
        &observed,
        &markets,
        origins,
        &config,
        10,
        &mut visits1,
        &mut next1,
    );

    let mut visits2 = ShopperVisits::default();
    let mut next2 = NextShopperId::default();
    capture_shopper_visits(
        &mg,
        &observed,
        &markets,
        origins,
        &config,
        10,
        &mut visits2,
        &mut next2,
    );

    // Full visit-struct equality (ids, travel_ticks, origin_node, start_tick).
    assert_eq!(
        visits1.0.values().cloned().collect::<Vec<_>>(),
        visits2.0.values().cloned().collect::<Vec<_>>(),
        "capture produces identical ShopperVisits on repeated runs with same inputs"
    );
    assert_eq!(next1, next2, "NextShopperId advances identically");
}

/// 6.3 — ShopperVisits are ephemeral: inserting an active visit into a world,
/// extracting the economy snapshot, serializing, deserializing, and applying into
/// a fresh world yields an EMPTY ShopperVisits.  The snapshot must also be
/// byte-identical to one taken from a world without any shoppers.
#[test]
fn shoppers_not_persisted() {
    use crate::world::schedule::SimPlugin;
    fn install_economy() -> bevy_ecs::world::World {
        let mut world = bevy_ecs::world::World::new();
        let mut schedule = bevy_ecs::schedule::Schedule::default();
        EconomyPlugin.install(&mut world, &mut schedule);
        world
    }

    // World A: has an active shopper visit.
    let mut world_a = install_economy();
    world_a.resource_mut::<ShopperVisits>().0.insert(
        0,
        ShopperVisit {
            id: 0,
            market: MarketId(99),
            good: GoodId(2),
            origin_node: NodeId(42),
            start_tick: 0,
            travel_ticks: 100,
        },
    );

    // World B: identical but no shoppers.
    let world_b = install_economy();

    // Snapshots must be byte-identical (ShopperVisits not persisted).
    let snap_a = extract_from_world(&world_a);
    let snap_b = extract_from_world(&world_b);
    let bytes_a = serde_json::to_vec(&snap_a).unwrap();
    let bytes_b = serde_json::to_vec(&snap_b).unwrap();
    assert_eq!(
        bytes_a, bytes_b,
        "economy snapshot is byte-identical with or without active ShopperVisits"
    );
    assert!(
        snap_a == snap_b,
        "EconomyPersistSnapshot does not include ShopperVisits field"
    );

    // Restoring from the snapshot into a fresh world yields empty ShopperVisits.
    let decoded: crate::economy::EconomyPersistSnapshot = serde_json::from_slice(&bytes_a).unwrap();
    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &decoded);
    assert!(
        fresh.resource::<ShopperVisits>().0.is_empty(),
        "fresh ShopperVisits is empty after apply_into_world (ephemeral, not persisted)"
    );
}
