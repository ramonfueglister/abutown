use crate::economy::{
    CommuterTrip, CommuterTrips, EconomyConfig, MarketId, MarketSite, Markets, Money,
    NextCommuterId, WageTelemetry, capture_commuter_trips,
};
use crate::routing::NodeId;

#[test]
fn capture_spawns_commuters_proportional_to_wage_capped() {
    // wage 1_000 / per_unit 100 = 10, capped at 4 → 4 trips
    use std::collections::BTreeSet;

    let m = MarketId(1);
    let mut telemetry = WageTelemetry::default();
    telemetry.0.insert(m, Money(1_000));

    let mut markets = Markets::default();
    markets.0.insert(
        m,
        MarketSite {
            id: m,
            node_id: NodeId(10),
            name: "M".to_string(),
        },
    );

    let origins = |market_node: NodeId| -> Vec<(NodeId, i64)> {
        assert_eq!(market_node, NodeId(10));
        vec![
            (NodeId(1), 4),
            (NodeId(2), 8),
            (NodeId(3), 12),
            (NodeId(4), 16),
            (NodeId(5), 20),
        ]
    };

    let observed: BTreeSet<MarketId> = [m].into_iter().collect();
    let config = EconomyConfig {
        commuters_per_wage_unit: 100,
        max_commuters_per_market: 4,
        ..EconomyConfig::default()
    };
    let origins = &origins;

    let mut trips = CommuterTrips::default();
    let mut next = NextCommuterId::default();
    capture_commuter_trips(
        &telemetry, &observed, &markets, origins, &config, 0, &mut trips, &mut next,
    );

    assert_eq!(
        trips.0.len(),
        4,
        "wage 1_000 / per_unit 100 = 10, capped at 4"
    );
    let mut taken: Vec<NodeId> = trips.0.values().map(|t| t.origin_node).collect();
    taken.sort_by_key(|n| n.0);
    assert_eq!(
        taken,
        vec![NodeId(1), NodeId(2), NodeId(3), NodeId(4)],
        "first-N sorted origins are taken"
    );
    for t in trips.0.values() {
        assert_eq!(t.market, m);
        assert_eq!(t.start_tick, 0);
        assert!(t.travel_ticks >= 1);
    }
}

#[test]
fn capture_ignores_unobserved_and_zero_wage_markets() {
    use std::collections::BTreeSet;

    let m_obs = MarketId(1);
    let m_unobs = MarketId(2);
    let m_zero = MarketId(3);

    let mut telemetry = WageTelemetry::default();
    telemetry.0.insert(m_obs, Money(500));
    telemetry.0.insert(m_unobs, Money(500));
    telemetry.0.insert(m_zero, Money(0));

    let mut markets = Markets::default();
    for &mid in &[m_obs, m_unobs, m_zero] {
        markets.0.insert(
            mid,
            MarketSite {
                id: mid,
                node_id: NodeId(mid.0 * 10),
                name: format!("M{}", mid.0),
            },
        );
    }

    // Only m_obs is observed; m_unobs is not observed; m_zero has wage=0.
    let observed: BTreeSet<MarketId> = [m_obs].into_iter().collect();

    let config = EconomyConfig {
        commuters_per_wage_unit: 100,
        max_commuters_per_market: 4,
        ..EconomyConfig::default()
    };

    let origins = |_market_node: NodeId| -> Vec<(NodeId, i64)> { vec![(NodeId(99), 5)] };

    let mut trips = CommuterTrips::default();
    let mut next = NextCommuterId::default();
    capture_commuter_trips(
        &telemetry, &observed, &markets, origins, &config, 0, &mut trips, &mut next,
    );

    // Only m_obs with wage=500 and per_unit=100 → target=min(5,4)=4 → but only 1 origin → 1 trip
    assert!(
        trips.0.values().all(|t| t.market == m_obs),
        "all trips belong to the observed market"
    );
    assert!(
        !trips.0.values().any(|t| t.market == m_unobs),
        "unobserved market spawns no trips"
    );
    assert!(
        !trips.0.values().any(|t| t.market == m_zero),
        "zero-wage market spawns no trips"
    );
}

#[test]
fn capture_tops_up_only_the_shortfall() {
    use std::collections::BTreeSet;

    let m = MarketId(1);
    let mut telemetry = WageTelemetry::default();
    telemetry.0.insert(m, Money(400));

    let mut markets = Markets::default();
    markets.0.insert(
        m,
        MarketSite {
            id: m,
            node_id: NodeId(10),
            name: "M".to_string(),
        },
    );

    let config = EconomyConfig {
        commuters_per_wage_unit: 100,
        max_commuters_per_market: 4,
        ..EconomyConfig::default()
    };
    // wage=400 / per_unit=100 → target=min(4,4)=4

    // Pre-insert 3 trips for this market → shortfall = 1
    let mut trips = CommuterTrips::default();
    let mut next = NextCommuterId::default();
    for i in 0..3u64 {
        let id = next.next();
        trips.0.insert(
            id,
            CommuterTrip {
                id,
                market: m,
                origin_node: NodeId(100 + i as u32),
                start_tick: 0,
                travel_ticks: 10,
            },
        );
    }

    let observed: BTreeSet<MarketId> = [m].into_iter().collect();
    let origins =
        |_market_node: NodeId| -> Vec<(NodeId, i64)> { vec![(NodeId(200), 5), (NodeId(201), 6)] };

    capture_commuter_trips(
        &telemetry, &observed, &markets, origins, &config, 1, &mut trips, &mut next,
    );

    assert_eq!(trips.0.len(), 4, "topped up from 3 to 4 (shortfall=1)");
    // Exactly 1 new trip with start_tick=1
    let new_trips: Vec<_> = trips.0.values().filter(|t| t.start_tick == 1).collect();
    assert_eq!(new_trips.len(), 1, "exactly one new trip added");
}
