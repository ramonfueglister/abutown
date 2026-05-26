use std::collections::{BTreeSet, HashMap};

use bevy_ecs::prelude::*;

use crate::routing::{Edge, EdgeKind, Graph, NodeId, NodeKind, RoutingError, RoutingProfileKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HpaConfig {
    pub cluster_size_tiles: u16,
    pub corridor_margin_clusters: u16,
}

impl Default for HpaConfig {
    fn default() -> Self {
        Self {
            cluster_size_tiles: 32,
            corridor_margin_clusters: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClusterCoord {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClusterId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HierarchicalRoutingError {
    InvalidConfig(&'static str),
    InvalidGraph(&'static str),
    MissingNode(NodeId),
    MissingCluster(NodeId),
    NoClusterPath {
        from: ClusterId,
        to: ClusterId,
        profile: RoutingProfileKey,
    },
    NoCorridorPath {
        from: NodeId,
        to: NodeId,
        profile: RoutingProfileKey,
    },
    Exact(RoutingError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HpaRouteStats {
    pub start_cluster: ClusterId,
    pub goal_cluster: ClusterId,
    pub abstract_clusters_visited: usize,
    pub corridor_cluster_count: usize,
    pub used_base_case: bool,
}

#[derive(Resource, Debug, Clone)]
pub struct HpaIndex {
    pub config: HpaConfig,
    cluster_coords: Vec<ClusterCoord>,
    cluster_ids_by_coord: HashMap<ClusterCoord, ClusterId>,
    node_clusters: Vec<Option<ClusterId>>,
    cluster_nodes: Vec<Vec<NodeId>>,
    cluster_portals: Vec<Vec<NodeId>>,
    adjacency: HashMap<(ClusterId, RoutingProfileKey), Vec<ClusterId>>,
}

pub struct HpaRouter;

pub fn cluster_coord_for(position: (f32, f32), config: HpaConfig) -> ClusterCoord {
    let size = f32::from(config.cluster_size_tiles.max(1));
    ClusterCoord {
        x: (position.0 / size).floor() as i32,
        y: (position.1 / size).floor() as i32,
    }
}

impl HpaIndex {
    pub fn build(graph: &Graph, config: HpaConfig) -> Result<Self, HierarchicalRoutingError> {
        if config.cluster_size_tiles == 0 {
            return Err(HierarchicalRoutingError::InvalidConfig(
                "cluster_size_tiles must be greater than zero",
            ));
        }

        let mut coords = BTreeSet::new();
        for node in graph.nodes() {
            coords.insert(cluster_coord_for(node.position, config));
        }

        let cluster_coords: Vec<ClusterCoord> = coords.into_iter().collect();
        let cluster_ids_by_coord: HashMap<ClusterCoord, ClusterId> = cluster_coords
            .iter()
            .enumerate()
            .map(|(index, coord)| (*coord, ClusterId(index as u32)))
            .collect();

        let mut node_clusters = vec![None; graph.node_count()];
        let mut cluster_nodes = vec![Vec::new(); cluster_coords.len()];
        for node in graph.nodes() {
            let coord = cluster_coord_for(node.position, config);
            let cluster = *cluster_ids_by_coord
                .get(&coord)
                .ok_or(HierarchicalRoutingError::MissingNode(node.id))?;
            node_clusters[node.id.0 as usize] = Some(cluster);
            cluster_nodes[cluster.0 as usize].push(node.id);
        }
        for nodes in &mut cluster_nodes {
            nodes.sort_by_key(|node| node.0);
        }

        let mut portal_sets: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); cluster_coords.len()];
        let mut adjacency_sets: HashMap<(ClusterId, RoutingProfileKey), BTreeSet<ClusterId>> =
            HashMap::new();

        for edge in graph.edges() {
            let from_cluster = cluster_for_node(&node_clusters, edge.from)?;
            let to_cluster = cluster_for_node(&node_clusters, edge.to)?;
            if from_cluster == to_cluster {
                continue;
            }

            portal_sets[from_cluster.0 as usize].insert(edge.from.0);
            portal_sets[to_cluster.0 as usize].insert(edge.to.0);

            for profile in profiles_for_cross_cluster_edge(graph, edge) {
                adjacency_sets
                    .entry((from_cluster, profile))
                    .or_default()
                    .insert(to_cluster);
            }
        }

        let cluster_portals: Vec<Vec<NodeId>> = portal_sets
            .into_iter()
            .map(|set| set.into_iter().map(NodeId).collect())
            .collect();
        let adjacency = adjacency_sets
            .into_iter()
            .map(|(key, values)| (key, values.into_iter().collect()))
            .collect();

        Ok(Self {
            config,
            cluster_coords,
            cluster_ids_by_coord,
            node_clusters,
            cluster_nodes,
            cluster_portals,
            adjacency,
        })
    }

    pub fn cluster_count(&self) -> usize {
        self.cluster_coords.len()
    }

    pub fn portal_count(&self) -> usize {
        self.cluster_portals.iter().map(Vec::len).sum()
    }

    pub fn cluster_of_node(&self, node: NodeId) -> Option<ClusterId> {
        self.node_clusters
            .get(node.0 as usize)
            .and_then(|entry| *entry)
    }

    pub fn portals_in_cluster(&self, cluster: ClusterId) -> &[NodeId] {
        self.cluster_portals
            .get(cluster.0 as usize)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn nodes_in_cluster(&self, cluster: ClusterId) -> &[NodeId] {
        self.cluster_nodes
            .get(cluster.0 as usize)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn cluster_coord(&self, cluster: ClusterId) -> ClusterCoord {
        self.cluster_coords[cluster.0 as usize]
    }

    pub fn cluster_id(&self, coord: ClusterCoord) -> Option<ClusterId> {
        self.cluster_ids_by_coord.get(&coord).copied()
    }

    pub fn adjacent_clusters(
        &self,
        cluster: ClusterId,
        profile: RoutingProfileKey,
    ) -> &[ClusterId] {
        self.adjacency
            .get(&(cluster, profile))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

fn cluster_for_node(
    node_clusters: &[Option<ClusterId>],
    node: NodeId,
) -> Result<ClusterId, HierarchicalRoutingError> {
    node_clusters
        .get(node.0 as usize)
        .and_then(|entry| *entry)
        .ok_or(HierarchicalRoutingError::MissingNode(node))
}

fn profiles_for_cross_cluster_edge(graph: &Graph, edge: &Edge) -> Vec<RoutingProfileKey> {
    let mut profiles = Vec::new();
    match edge.kind {
        EdgeKind::Footway => {
            profiles.push(RoutingProfileKey::Walk);
            profiles.push(RoutingProfileKey::WalkTransit);
        }
        EdgeKind::Road => {
            profiles.push(RoutingProfileKey::Car);
        }
        EdgeKind::TramTrack => {
            profiles.push(RoutingProfileKey::Tram);
            let from_is_stop = graph.node(edge.from).kind == NodeKind::TransitStop;
            let to_is_stop = graph.node(edge.to).kind == NodeKind::TransitStop;
            if from_is_stop || to_is_stop {
                profiles.push(RoutingProfileKey::WalkTransit);
            }
        }
    }
    profiles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{EdgeId, Node};

    fn node(id: u32, x: f32, y: f32, kind: NodeKind) -> Node {
        Node {
            id: NodeId(id),
            position: (x, y),
            kind,
            legacy_id: None,
        }
    }

    fn edge(id: u32, from: u32, to: u32, kind: EdgeKind, length: f32) -> Edge {
        Edge {
            id: EdgeId(id),
            from: NodeId(from),
            to: NodeId(to),
            polyline: vec![(from as f32, 0.0), (to as f32, 0.0)],
            length,
            kind,
            speed_limit: match kind {
                EdgeKind::Footway => 1.0,
                EdgeKind::Road => 6.0,
                EdgeKind::TramTrack => 4.0,
            },
            capacity: 1,
            legacy_id: None,
        }
    }

    fn three_cluster_walk_graph() -> Graph {
        Graph::new(
            vec![
                node(0, 0.0, 0.0, NodeKind::Intersection),
                node(1, 5.0, 0.0, NodeKind::Intersection),
                node(2, 12.0, 0.0, NodeKind::Intersection),
                node(3, 25.0, 0.0, NodeKind::Intersection),
            ],
            vec![
                edge(0, 0, 1, EdgeKind::Footway, 5.0),
                edge(1, 1, 2, EdgeKind::Footway, 7.0),
                edge(2, 2, 3, EdgeKind::Footway, 13.0),
            ],
        )
    }

    #[test]
    fn cluster_coord_uses_floor_division() {
        let config = HpaConfig {
            cluster_size_tiles: 10,
            corridor_margin_clusters: 0,
        };

        assert_eq!(
            cluster_coord_for((0.0, 0.0), config),
            ClusterCoord { x: 0, y: 0 }
        );
        assert_eq!(
            cluster_coord_for((9.9, 0.0), config),
            ClusterCoord { x: 0, y: 0 }
        );
        assert_eq!(
            cluster_coord_for((10.0, 0.0), config),
            ClusterCoord { x: 1, y: 0 }
        );
        assert_eq!(
            cluster_coord_for((-0.1, 0.0), config),
            ClusterCoord { x: -1, y: 0 }
        );
    }

    #[test]
    fn index_assigns_deterministic_cluster_ids() {
        let graph = Graph::new(
            vec![
                node(0, 25.0, 0.0, NodeKind::Intersection),
                node(1, 0.0, 0.0, NodeKind::Intersection),
                node(2, 12.0, 0.0, NodeKind::Intersection),
            ],
            Vec::new(),
        );
        let index = HpaIndex::build(
            &graph,
            HpaConfig {
                cluster_size_tiles: 10,
                corridor_margin_clusters: 0,
            },
        )
        .expect("index builds");

        assert_eq!(index.cluster_count(), 3);
        assert_eq!(
            index.cluster_coord(ClusterId(0)),
            ClusterCoord { x: 0, y: 0 }
        );
        assert_eq!(
            index.cluster_coord(ClusterId(1)),
            ClusterCoord { x: 1, y: 0 }
        );
        assert_eq!(
            index.cluster_coord(ClusterId(2)),
            ClusterCoord { x: 2, y: 0 }
        );
    }

    #[test]
    fn index_detects_cross_cluster_portals() {
        let graph = three_cluster_walk_graph();
        let index = HpaIndex::build(
            &graph,
            HpaConfig {
                cluster_size_tiles: 10,
                corridor_margin_clusters: 0,
            },
        )
        .expect("index builds");

        assert_eq!(index.cluster_count(), 3);
        assert_eq!(index.portal_count(), 3);
        assert_eq!(index.portals_in_cluster(ClusterId(0)), &[NodeId(1)]);
        assert_eq!(index.portals_in_cluster(ClusterId(1)), &[NodeId(2)]);
        assert_eq!(index.portals_in_cluster(ClusterId(2)), &[NodeId(3)]);
    }
}
