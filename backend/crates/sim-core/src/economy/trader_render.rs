//! Render-only helpers shared by flow shipment and shopper render paths.
//! Pure functions; no ECS. The bridge (materialize.rs) wires these to resources.

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
}
