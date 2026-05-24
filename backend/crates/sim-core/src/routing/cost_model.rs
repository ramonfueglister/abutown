use crate::routing::graph::{Edge, EdgeKind};

pub trait CostModel: Send + Sync {
    /// Cost of traversing an edge. Return `f32::INFINITY` to disallow.
    fn cost(&self, edge: &Edge) -> f32;
}

pub struct DistanceCost;

impl CostModel for DistanceCost {
    fn cost(&self, edge: &Edge) -> f32 {
        edge.length
    }
}

pub struct TimeCost;

impl CostModel for TimeCost {
    fn cost(&self, edge: &Edge) -> f32 {
        edge.length / edge.speed_limit.max(0.001)
    }
}

pub struct ModeFilterCost<C: CostModel> {
    pub inner: C,
    pub allowed: &'static [EdgeKind],
}

impl<C: CostModel> CostModel for ModeFilterCost<C> {
    fn cost(&self, edge: &Edge) -> f32 {
        if self.allowed.contains(&edge.kind) {
            self.inner.cost(edge)
        } else {
            f32::INFINITY
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::graph::{EdgeId, NodeId};

    fn make_edge(kind: EdgeKind, length: f32, speed: f32) -> Edge {
        Edge {
            id: EdgeId(0),
            from: NodeId(0),
            to: NodeId(1),
            polyline: vec![],
            length,
            kind,
            speed_limit: speed,
            capacity: 1,
            legacy_id: None,
        }
    }

    #[test]
    fn distance_cost_returns_edge_length() {
        let e = make_edge(EdgeKind::Footway, 12.5, 1.0);
        assert_eq!(DistanceCost.cost(&e), 12.5);
    }

    #[test]
    fn time_cost_returns_length_over_speed() {
        let e = make_edge(EdgeKind::Road, 20.0, 4.0);
        assert_eq!(TimeCost.cost(&e), 5.0);
    }

    #[test]
    fn time_cost_clamps_zero_speed_to_finite() {
        let e = make_edge(EdgeKind::Road, 1.0, 0.0);
        assert!(
            TimeCost.cost(&e).is_finite(),
            "zero-speed must not produce NaN/inf"
        );
    }

    #[test]
    fn mode_filter_rejects_disallowed_kind() {
        let walking = ModeFilterCost {
            inner: DistanceCost,
            allowed: &[EdgeKind::Footway],
        };
        let foot = make_edge(EdgeKind::Footway, 5.0, 1.0);
        let road = make_edge(EdgeKind::Road, 5.0, 1.0);
        assert_eq!(walking.cost(&foot), 5.0);
        assert!(walking.cost(&road).is_infinite());
    }
}
