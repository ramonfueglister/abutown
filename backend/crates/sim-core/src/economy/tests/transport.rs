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
