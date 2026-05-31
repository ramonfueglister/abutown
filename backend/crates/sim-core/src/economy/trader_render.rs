//! Render-only helpers turning trader state + a footway route into a world coord.
//! Pure functions; no ECS. The bridge (materialize.rs) wires these to resources.

use crate::economy::traders::transport_ticks;
use crate::economy::{EconomyConfig, Trader, TraderState};
use crate::routing::{EdgeId, Graph};

/// Concatenate the polylines of `edges` into one route polyline, dropping the
/// duplicated shared endpoint between consecutive edges. Empty if no edges.
pub fn route_polyline(graph: &Graph, edges: &[EdgeId]) -> Vec<(f32, f32)> {
    let mut out: Vec<(f32, f32)> = Vec::new();
    for &edge_id in edges {
        let poly = &graph.edge(edge_id).polyline;
        if poly.is_empty() {
            continue;
        }
        let start = if out.last() == poly.first() { 1 } else { 0 };
        out.extend_from_slice(&poly[start..]);
    }
    out
}

/// Travel progress in [0,1] for a trader, given its travel-tick budget.
/// `Buying` => 0 (at source), `Selling` => 1 (at dest).
pub fn leg_progress(state: &TraderState, travel: u64) -> f32 {
    let travel = travel.max(1) as f32;
    match state {
        TraderState::Buying { .. } => 0.0,
        TraderState::Selling { .. } => 1.0,
        TraderState::ToDest { remaining } | TraderState::ToSource { remaining } => {
            let done = travel - (*remaining as f32);
            (done / travel).clamp(0.0, 1.0)
        }
    }
}

/// `Buying`/`ToDest` => outbound (source->dest); `Selling`/`ToSource` => return.
pub fn is_outbound(state: &TraderState) -> bool {
    matches!(
        state,
        TraderState::Buying { .. } | TraderState::ToDest { .. }
    )
}

/// The travel-tick budget for a trader (so callers don't re-derive it).
pub fn trader_travel(trader: &Trader, config: &EconomyConfig) -> u64 {
    transport_ticks(trader.distance_tiles, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{Edge, EdgeId, EdgeKind, Graph, Node, NodeId, NodeKind};

    fn node(id: u32, x: f32, y: f32) -> Node {
        Node {
            id: NodeId(id),
            position: (x, y),
            kind: NodeKind::Intersection,
            legacy_id: None,
        }
    }
    fn edge(id: u32, from: u32, to: u32, poly: Vec<(f32, f32)>) -> Edge {
        let length = poly
            .windows(2)
            .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
            .sum();
        Edge {
            id: EdgeId(id),
            from: NodeId(from),
            to: NodeId(to),
            polyline: poly,
            length,
            kind: EdgeKind::Footway,
            speed_limit: 1.0,
            capacity: 1,
            legacy_id: None,
        }
    }

    #[test]
    fn route_polyline_concats_and_dedupes_shared_endpoints() {
        let graph = Graph::new(
            vec![node(0, 0.0, 0.0), node(1, 2.0, 0.0), node(2, 2.0, 3.0)],
            vec![
                edge(0, 0, 1, vec![(0.0, 0.0), (2.0, 0.0)]),
                edge(1, 1, 2, vec![(2.0, 0.0), (2.0, 3.0)]),
            ],
        );
        let poly = route_polyline(&graph, &[EdgeId(0), EdgeId(1)]);
        assert_eq!(poly, vec![(0.0, 0.0), (2.0, 0.0), (2.0, 3.0)]); // shared (2,0) once
    }

    #[test]
    fn leg_progress_maps_countdown_to_unit_interval() {
        assert_eq!(leg_progress(&TraderState::ToDest { remaining: 4 }, 4), 0.0);
        assert_eq!(leg_progress(&TraderState::ToDest { remaining: 1 }, 4), 0.75);
        assert_eq!(leg_progress(&TraderState::Buying { order: None }, 4), 0.0);
        assert_eq!(leg_progress(&TraderState::Selling { order: None }, 4), 1.0);
    }

    #[test]
    fn is_outbound_distinguishes_legs() {
        assert!(is_outbound(&TraderState::Buying { order: None }));
        assert!(is_outbound(&TraderState::ToDest { remaining: 2 }));
        assert!(!is_outbound(&TraderState::Selling { order: None }));
        assert!(!is_outbound(&TraderState::ToSource { remaining: 2 }));
    }
}
