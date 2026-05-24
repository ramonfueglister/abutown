use bevy_ecs::prelude::*;
use std::collections::HashMap;

#[derive(Component, Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct NodeId(pub u32);

#[derive(Component, Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct EdgeId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Intersection,
    TransitStop,
    ActivityLocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    Footway,
    Road,
    TramTrack,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub position: (f32, f32),
    pub kind: NodeKind,
    /// Legacy wire id (e.g., "stop:horizontal:pickup"). `None` for nodes
    /// introduced by the builder (pure intersections without legacy ancestry).
    pub legacy_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Edge {
    pub id: EdgeId,
    pub from: NodeId,
    pub to: NodeId,
    pub polyline: Vec<(f32, f32)>,
    pub length: f32,
    pub kind: EdgeKind,
    pub speed_limit: f32,
    pub capacity: u16,
    /// Legacy wire id (e.g., "link:walk:corridor:22"). `None` for edges
    /// introduced by the builder for pure topology.
    pub legacy_id: Option<String>,
}

#[derive(Resource, Debug, Default)]
pub struct Graph {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    outgoing: Vec<Vec<EdgeId>>,
    incoming: Vec<Vec<EdgeId>>,
    by_legacy_node_id: HashMap<String, NodeId>,
    legacy_node_ids: Vec<Vec<String>>,
    by_legacy_edge_id: HashMap<String, EdgeId>,
}

impl Graph {
    pub fn new(nodes: Vec<Node>, edges: Vec<Edge>) -> Self {
        let node_count = nodes.len();
        let mut outgoing: Vec<Vec<EdgeId>> = vec![Vec::new(); node_count];
        let mut incoming: Vec<Vec<EdgeId>> = vec![Vec::new(); node_count];
        let mut by_legacy_node_id: HashMap<String, NodeId> = HashMap::new();
        let mut by_legacy_edge_id: HashMap<String, EdgeId> = HashMap::new();
        let mut legacy_node_ids: Vec<Vec<String>> = vec![Vec::new(); node_count];
        for n in &nodes {
            if let Some(legacy) = &n.legacy_id {
                by_legacy_node_id.insert(legacy.clone(), n.id);
                legacy_node_ids[n.id.0 as usize].push(legacy.clone());
            }
        }
        for e in &edges {
            outgoing[e.from.0 as usize].push(e.id);
            incoming[e.to.0 as usize].push(e.id);
            if let Some(legacy) = &e.legacy_id {
                by_legacy_edge_id.insert(legacy.clone(), e.id);
            }
        }
        Self {
            nodes,
            edges,
            outgoing,
            incoming,
            by_legacy_node_id,
            legacy_node_ids,
            by_legacy_edge_id,
        }
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    pub fn edge(&self, id: EdgeId) -> &Edge {
        &self.edges[id.0 as usize]
    }

    pub fn outgoing(&self, id: NodeId) -> &[EdgeId] {
        &self.outgoing[id.0 as usize]
    }

    pub fn incoming(&self, id: NodeId) -> &[EdgeId] {
        &self.incoming[id.0 as usize]
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn node_by_legacy(&self, legacy_id: &str) -> Option<NodeId> {
        self.by_legacy_node_id.get(legacy_id).copied()
    }

    pub fn add_legacy_node_alias(&mut self, legacy_id: String, id: NodeId) {
        self.by_legacy_node_id.insert(legacy_id.clone(), id);
        let aliases = &mut self.legacy_node_ids[id.0 as usize];
        if !aliases.contains(&legacy_id) {
            aliases.push(legacy_id);
        }
    }

    pub fn legacy_node_ids(&self, id: NodeId) -> &[String] {
        &self.legacy_node_ids[id.0 as usize]
    }

    pub fn edge_by_legacy(&self, legacy_id: &str) -> Option<EdgeId> {
        self.by_legacy_edge_id.get(legacy_id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_two_node_graph() -> Graph {
        let n0 = Node {
            id: NodeId(0),
            position: (0.0, 0.0),
            kind: NodeKind::Intersection,
            legacy_id: None,
        };
        let n1 = Node {
            id: NodeId(1),
            position: (10.0, 0.0),
            kind: NodeKind::TransitStop,
            legacy_id: Some("stop:test".into()),
        };
        let e0 = Edge {
            id: EdgeId(0),
            from: NodeId(0),
            to: NodeId(1),
            polyline: vec![(0.0, 0.0), (10.0, 0.0)],
            length: 10.0,
            kind: EdgeKind::Footway,
            speed_limit: 1.0,
            capacity: 1,
            legacy_id: Some("link:test".into()),
        };
        Graph::new(vec![n0, n1], vec![e0])
    }

    #[test]
    fn routing_module_compiles() {}

    #[test]
    fn graph_indexes_nodes_edges_and_adjacency() {
        let g = build_two_node_graph();
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.edge_count(), 1);
        assert_eq!(g.node(NodeId(1)).kind, NodeKind::TransitStop);
        assert_eq!(g.edge(EdgeId(0)).length, 10.0);
        assert_eq!(g.outgoing(NodeId(0)), &[EdgeId(0)]);
        assert!(g.outgoing(NodeId(1)).is_empty());
        assert_eq!(g.incoming(NodeId(1)), &[EdgeId(0)]);
        assert!(g.incoming(NodeId(0)).is_empty());
    }

    #[test]
    fn graph_resolves_legacy_ids() {
        let g = build_two_node_graph();
        assert_eq!(g.node_by_legacy("stop:test"), Some(NodeId(1)));
        assert_eq!(g.edge_by_legacy("link:test"), Some(EdgeId(0)));
        assert_eq!(g.node_by_legacy("missing"), None);
    }
}
