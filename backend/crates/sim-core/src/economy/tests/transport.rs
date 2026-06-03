use crate::economy::{
    EconomyError, Money, Quantity, manhattan_tiles, transport_cost, transport_cost_between,
};
use crate::routing::{Edge, EdgeId, EdgeKind, Graph, Node, NodeId, NodeKind};

fn two_node_graph(ax: f32, ay: f32, bx: f32, by: f32) -> Graph {
    let nodes = vec![
        Node {
            id: NodeId(0),
            position: (ax, ay),
            kind: NodeKind::Intersection,
            legacy_id: None,
        },
        Node {
            id: NodeId(1),
            position: (bx, by),
            kind: NodeKind::Intersection,
            legacy_id: None,
        },
    ];
    // one footway edge so the graph is well-formed (length unused by Manhattan cost)
    let edges = vec![Edge {
        id: EdgeId(0),
        from: NodeId(0),
        to: NodeId(1),
        kind: EdgeKind::Footway,
        length: 1.0,
        polyline: vec![(ax, ay), (bx, by)],
        speed_limit: 1.0,
        capacity: 1,
        legacy_id: None,
    }];
    Graph::new(nodes, edges)
}

#[test]
fn manhattan_tiles_is_integer_and_symmetric() {
    let g = two_node_graph(106.0, 64.51, 117.0, 64.51);
    assert_eq!(manhattan_tiles(&g, NodeId(0), NodeId(1)), 11); // |117-106| + |65-65| (64.51 rounds to 65 both)
    assert_eq!(manhattan_tiles(&g, NodeId(1), NodeId(0)), 11);
    assert_eq!(manhattan_tiles(&g, NodeId(0), NodeId(0)), 0);
}

#[test]
fn transport_cost_scales_with_distance_and_qty() {
    // rate 1.0/tile/unit, qty 1 unit, dist 10 -> 10.0
    assert_eq!(
        transport_cost(10, Quantity(1_000), Money(1_000)),
        Ok(Money(10_000))
    );
    assert_eq!(
        transport_cost(20, Quantity(1_000), Money(1_000)),
        Ok(Money(20_000))
    );
    assert_eq!(
        transport_cost(10, Quantity(2_000), Money(1_000)),
        Ok(Money(20_000))
    );
}

#[test]
fn transport_cost_zero_distance_is_zero() {
    assert_eq!(
        transport_cost(0, Quantity(5_000), Money(1_000)),
        Ok(Money(0))
    );
}

#[test]
fn transport_cost_rejects_negative_distance() {
    assert_eq!(
        transport_cost(-1, Quantity(1_000), Money(1_000)),
        Err(EconomyError::InvalidOrder)
    );
}

#[test]
fn transport_cost_overflow_returns_error() {
    assert_eq!(
        transport_cost(i64::MAX, Quantity(i64::MAX), Money(i64::MAX)),
        Err(EconomyError::Overflow)
    );
}

#[test]
fn transport_cost_between_uses_graph_node_positions() {
    let g = two_node_graph(0.0, 0.0, 3.0, 4.0);
    // manhattan = 3 + 4 = 7; rate 1.0/tile/unit, qty 1 unit -> 7.0
    assert_eq!(
        transport_cost_between(&g, NodeId(0), NodeId(1), Quantity(1_000), Money(1_000)),
        Ok(Money(7_000))
    );
}

#[test]
fn transport_rebate_drains_operator_to_zero_and_conserves() {
    use crate::economy::transport::run_transport_rebate_at_tick;
    use crate::economy::{
        AccountBook, DemandPool, DemandPools, EconomicActorId, EconomyEvent, GOOD_TOOLS,
        HOUSEHOLD_SECTOR, HouseholdSector, MarketId, Money, Quantity, TRANSPORT_OPERATOR,
        TradeLedger,
    };
    use std::collections::BTreeMap;

    fn pool(actor: EconomicActorId) -> DemandPool {
        DemandPool {
            actor,
            market: MarketId(9_002),
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
        }
    }

    let c1 = EconomicActorId(8_002);
    let c2 = EconomicActorId(8_012);
    let c3 = EconomicActorId(8_022);
    let mut accounts = AccountBook::default();
    accounts.deposit(TRANSPORT_OPERATOR, Money(301)).unwrap();
    let mut demand = DemandPools::default();
    for c in [c1, c2, c3] {
        demand.0.insert(c, pool(c));
    }
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(c1, 1), (c2, 1), (c3, 1)]),
    };

    let before = accounts.total_money().unwrap();
    let mut ledger = TradeLedger::default();
    run_transport_rebate_at_tick(&mut accounts, &mut demand, &household, &mut ledger).unwrap();

    assert_eq!(
        accounts.total_money().unwrap(),
        before,
        "byte-invariant total money"
    );
    assert_eq!(
        accounts.account(TRANSPORT_OPERATOR).available,
        Money::ZERO,
        "operator fully drained"
    );
    assert_eq!(
        accounts.account(HOUSEHOLD_SECTOR).available,
        Money::ZERO,
        "sentinel nets to zero"
    );
    // 301 across 3 equal weights, largest-remainder ⇒ 101/100/100 (lowest index wins extra).
    assert_eq!(demand.0[&c1].income_last_tick, Money(101));
    assert_eq!(demand.0[&c2].income_last_tick, Money(100));
    assert_eq!(demand.0[&c3].income_last_tick, Money(100));
    let total: i64 = demand.0.values().map(|p| p.income_last_tick.0).sum();
    assert_eq!(total, 301, "Σ income == rebated amount");
    assert!(
        ledger
            .0
            .contains(&EconomyEvent::TransportRebate { amount: Money(301) })
    );
}

#[test]
fn transport_rebate_zero_balance_is_noop() {
    use crate::economy::transport::run_transport_rebate_at_tick;
    use crate::economy::{
        AccountBook, DemandPool, DemandPools, EconomicActorId, GOOD_TOOLS, HouseholdSector,
        MarketId, Money, Quantity, TradeLedger,
    };
    use std::collections::BTreeMap;
    let c1 = EconomicActorId(8_002);
    let mut accounts = AccountBook::default();
    let mut demand = DemandPools::default();
    demand.0.insert(
        c1,
        DemandPool {
            actor: c1,
            market: MarketId(9_002),
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
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(c1, 1)]),
    };
    let before = accounts.total_money().unwrap();
    let mut ledger = TradeLedger::default();
    run_transport_rebate_at_tick(&mut accounts, &mut demand, &household, &mut ledger).unwrap();
    assert_eq!(accounts.total_money().unwrap(), before);
    assert_eq!(demand.0[&c1].income_last_tick, Money::ZERO);
    assert!(ledger.0.is_empty(), "no rebate event when nothing to drain");
}
