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
fn capture_spawns_proportional_to_unmet_demand_deterministically() {
    use crate::economy::shoppers::capture_shopper_visits;
    use crate::economy::{
        EconomyConfig, MarketGoodKey, MarketGoodState, MarketGoods, MarketSite, Markets, Quantity,
    };
    use std::collections::BTreeSet;

    // One observed market m=1 (node 10) with unmet_demand 9 of good 0.
    // cap = 4, per_unit = 3 -> target = min(9/3, 4) = 3 visits.
    let m = MarketId(1);
    let good = GoodId(0);
    let key = MarketGoodKey { market: m, good };
    let mut mg = MarketGoods::default();
    let mut st = MarketGoodState::new(key);
    st.unmet_demand_last_tick = Quantity(9);
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
