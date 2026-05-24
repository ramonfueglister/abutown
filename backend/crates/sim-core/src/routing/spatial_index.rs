use bevy_ecs::prelude::*;
use rstar::{AABB, PointDistance, RTree, RTreeObject};

use crate::routing::graph::{Node, NodeId};

#[derive(Debug, Clone)]
pub struct IndexedNode {
    pub id: NodeId,
    pub position: [f32; 2],
}

impl RTreeObject for IndexedNode {
    type Envelope = AABB<[f32; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_point(self.position)
    }
}

impl PointDistance for IndexedNode {
    fn distance_2(&self, point: &[f32; 2]) -> f32 {
        let dx = self.position[0] - point[0];
        let dy = self.position[1] - point[1];
        dx * dx + dy * dy
    }
}

#[derive(Resource, Default)]
pub struct NodeSpatialIndex(RTree<IndexedNode>);

impl NodeSpatialIndex {
    pub fn from_nodes(nodes: &[Node]) -> Self {
        let indexed: Vec<IndexedNode> = nodes
            .iter()
            .map(|n| IndexedNode {
                id: n.id,
                position: [n.position.0, n.position.1],
            })
            .collect();
        Self(RTree::bulk_load(indexed))
    }

    pub fn nearest(&self, point: (f32, f32)) -> Option<NodeId> {
        self.0.nearest_neighbor(&[point.0, point.1]).map(|n| n.id)
    }

    pub fn within_radius(&self, center: (f32, f32), radius: f32) -> Vec<NodeId> {
        let r2 = radius * radius;
        self.0
            .locate_within_distance([center.0, center.1], r2)
            .map(|n| n.id)
            .collect()
    }

    pub fn size(&self) -> usize {
        self.0.size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::graph::NodeKind;

    fn n(id: u32, x: f32, y: f32) -> Node {
        Node {
            id: NodeId(id),
            position: (x, y),
            kind: NodeKind::Intersection,
            legacy_id: None,
        }
    }

    #[test]
    fn nearest_returns_closest_node() {
        let idx = NodeSpatialIndex::from_nodes(&[n(0, 0.0, 0.0), n(1, 10.0, 0.0), n(2, 0.0, 10.0)]);
        assert_eq!(idx.nearest((0.5, 0.5)), Some(NodeId(0)));
        assert_eq!(idx.nearest((9.5, 0.5)), Some(NodeId(1)));
        assert_eq!(idx.nearest((0.5, 9.5)), Some(NodeId(2)));
    }

    #[test]
    fn nearest_on_empty_returns_none() {
        let idx = NodeSpatialIndex::from_nodes(&[]);
        assert_eq!(idx.nearest((0.0, 0.0)), None);
    }

    #[test]
    fn within_radius_returns_only_in_range() {
        let idx = NodeSpatialIndex::from_nodes(&[n(0, 0.0, 0.0), n(1, 3.0, 0.0), n(2, 10.0, 0.0)]);
        let mut got = idx.within_radius((0.0, 0.0), 5.0);
        got.sort_by_key(|n| n.0);
        assert_eq!(got, vec![NodeId(0), NodeId(1)]);
    }

    #[test]
    fn size_matches_node_count() {
        let idx = NodeSpatialIndex::from_nodes(&[n(0, 0.0, 0.0), n(1, 1.0, 1.0)]);
        assert_eq!(idx.size(), 2);
    }
}
