use std::cmp::Ordering;
use std::collections::{BTreeSet, BinaryHeap, HashMap, HashSet};

use bevy_ecs::prelude::*;

use crate::routing::pathfinding::EdgeConstraint;
use crate::routing::{
    AStarRouter, Edge, EdgeKind, Graph, NodeId, PathRequest, PlannedPath, RoutingError,
    RoutingProfile, RoutingProfileKey,
};

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

    pub fn corridor_between(
        &self,
        from: NodeId,
        to: NodeId,
        profile: RoutingProfileKey,
    ) -> Result<HashSet<ClusterId>, HierarchicalRoutingError> {
        let start_cluster = resolve_request_cluster(self, from)?;
        let goal_cluster = resolve_request_cluster(self, to)?;

        if start_cluster == goal_cluster {
            return Ok(HashSet::from([start_cluster]));
        }

        let (cluster_path, _) = abstract_cluster_path(self, start_cluster, goal_cluster, profile)?;
        Ok(expand_corridor(self, &cluster_path))
    }

    #[cfg(test)]
    fn force_cluster_adjacency_for_test(
        &mut self,
        from: ClusterId,
        to: ClusterId,
        profile: RoutingProfileKey,
    ) {
        self.adjacency.entry((from, profile)).or_default().push(to);
    }
}

impl HpaRouter {
    pub fn find_path(
        graph: &Graph,
        index: &HpaIndex,
        request: PathRequest,
        profile: RoutingProfile,
    ) -> Result<(PlannedPath, HpaRouteStats), HierarchicalRoutingError> {
        if request.profile != profile.key {
            return Err(HierarchicalRoutingError::InvalidGraph(
                "request profile must match routing profile",
            ));
        }
        validate_request_node(graph, request.from)?;
        validate_request_node(graph, request.to)?;

        let start_cluster = resolve_request_cluster(index, request.from)?;
        let goal_cluster = resolve_request_cluster(index, request.to)?;

        if start_cluster == goal_cluster {
            let corridor = HashSet::from([start_cluster]);
            let path = constrained_exact(graph, index, request, profile, &corridor)?;
            return Ok((
                path,
                HpaRouteStats {
                    start_cluster,
                    goal_cluster,
                    abstract_clusters_visited: 1,
                    corridor_cluster_count: 1,
                    used_base_case: true,
                },
            ));
        }

        let (cluster_path, visited) =
            abstract_cluster_path(index, start_cluster, goal_cluster, request.profile)?;
        let corridor = expand_corridor(index, &cluster_path);
        let path = constrained_exact(graph, index, request, profile, &corridor)?;

        Ok((
            path,
            HpaRouteStats {
                start_cluster,
                goal_cluster,
                abstract_clusters_visited: visited,
                corridor_cluster_count: corridor.len(),
                used_base_case: false,
            },
        ))
    }
}

fn validate_request_node(graph: &Graph, node: NodeId) -> Result<(), HierarchicalRoutingError> {
    if (node.0 as usize) < graph.node_count() {
        Ok(())
    } else {
        Err(HierarchicalRoutingError::MissingNode(node))
    }
}

fn resolve_request_cluster(
    index: &HpaIndex,
    node: NodeId,
) -> Result<ClusterId, HierarchicalRoutingError> {
    index
        .cluster_of_node(node)
        .ok_or(HierarchicalRoutingError::MissingCluster(node))
}

#[derive(Debug, Clone, Copy)]
struct ClusterQueueEntry {
    cluster: ClusterId,
    known_cost: u32,
    estimated_total: u32,
}

impl PartialEq for ClusterQueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.estimated_total == other.estimated_total
            && self.known_cost == other.known_cost
            && self.cluster == other.cluster
    }
}

impl Eq for ClusterQueueEntry {}

impl Ord for ClusterQueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .estimated_total
            .cmp(&self.estimated_total)
            .then_with(|| other.known_cost.cmp(&self.known_cost))
            .then_with(|| other.cluster.cmp(&self.cluster))
    }
}

impl PartialOrd for ClusterQueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn abstract_cluster_path(
    index: &HpaIndex,
    start: ClusterId,
    goal: ClusterId,
    profile: RoutingProfileKey,
) -> Result<(Vec<ClusterId>, usize), HierarchicalRoutingError> {
    let mut open = BinaryHeap::new();
    let mut best: HashMap<ClusterId, u32> = HashMap::new();
    let mut came_from: HashMap<ClusterId, ClusterId> = HashMap::new();
    let mut visited = 0usize;

    best.insert(start, 0);
    open.push(ClusterQueueEntry {
        cluster: start,
        known_cost: 0,
        estimated_total: cluster_heuristic(index, start, goal),
    });

    while let Some(entry) = open.pop() {
        visited += 1;
        if entry.cluster == goal {
            return Ok((reconstruct_cluster_path(start, goal, &came_from), visited));
        }
        if entry.known_cost > *best.get(&entry.cluster).unwrap_or(&u32::MAX) {
            continue;
        }

        for next in index.adjacent_clusters(entry.cluster, profile) {
            let next_cost = entry.known_cost + 1;
            if next_cost < *best.get(next).unwrap_or(&u32::MAX) {
                best.insert(*next, next_cost);
                came_from.insert(*next, entry.cluster);
                open.push(ClusterQueueEntry {
                    cluster: *next,
                    known_cost: next_cost,
                    estimated_total: next_cost + cluster_heuristic(index, *next, goal),
                });
            }
        }
    }

    Err(HierarchicalRoutingError::NoClusterPath {
        from: start,
        to: goal,
        profile,
    })
}

fn cluster_heuristic(index: &HpaIndex, from: ClusterId, to: ClusterId) -> u32 {
    let a = index.cluster_coord(from);
    let b = index.cluster_coord(to);
    a.x.abs_diff(b.x) + a.y.abs_diff(b.y)
}

fn reconstruct_cluster_path(
    start: ClusterId,
    goal: ClusterId,
    came_from: &HashMap<ClusterId, ClusterId>,
) -> Vec<ClusterId> {
    let mut current = goal;
    let mut out = vec![current];
    while current != start {
        current = came_from[&current];
        out.push(current);
    }
    out.reverse();
    out
}

fn expand_corridor(index: &HpaIndex, cluster_path: &[ClusterId]) -> HashSet<ClusterId> {
    let mut corridor: HashSet<ClusterId> = cluster_path.iter().copied().collect();
    let margin = i32::from(index.config.corridor_margin_clusters);
    if margin == 0 {
        return corridor;
    }

    for cluster in cluster_path {
        let center = index.cluster_coord(*cluster);
        for dx in -margin..=margin {
            for dy in -margin..=margin {
                if let Some(id) = index.cluster_id(ClusterCoord {
                    x: center.x + dx,
                    y: center.y + dy,
                }) {
                    corridor.insert(id);
                }
            }
        }
    }
    corridor
}

fn constrained_exact(
    graph: &Graph,
    index: &HpaIndex,
    request: PathRequest,
    profile: RoutingProfile,
    corridor: &HashSet<ClusterId>,
) -> Result<PlannedPath, HierarchicalRoutingError> {
    let constraint = ClusterCorridorConstraint { index, corridor };
    AStarRouter::find_path_with_constraint(graph, request, profile, &constraint).map_err(|error| {
        match error {
            RoutingError::NoPath { from, to, profile } => {
                HierarchicalRoutingError::NoCorridorPath { from, to, profile }
            }
            RoutingError::MissingNode(node) => HierarchicalRoutingError::MissingNode(node),
            other => HierarchicalRoutingError::Exact(other),
        }
    })
}

struct ClusterCorridorConstraint<'a> {
    index: &'a HpaIndex,
    corridor: &'a HashSet<ClusterId>,
}

impl EdgeConstraint for ClusterCorridorConstraint<'_> {
    fn allows(&self, _graph: &Graph, edge: &Edge) -> bool {
        let Some(from_cluster) = self.index.cluster_of_node(edge.from) else {
            return false;
        };
        let Some(to_cluster) = self.index.cluster_of_node(edge.to) else {
            return false;
        };
        self.corridor.contains(&from_cluster) && self.corridor.contains(&to_cluster)
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

fn profiles_for_cross_cluster_edge(_graph: &Graph, edge: &Edge) -> Vec<RoutingProfileKey> {
    let mut profiles = Vec::new();
    match edge.kind {
        EdgeKind::Footway => {
            profiles.push(RoutingProfileKey::Walk);
            profiles.push(RoutingProfileKey::WalkTransit);
        }
        EdgeKind::Road => {
            profiles.push(RoutingProfileKey::Car);
        }
        EdgeKind::TramTrack => {}
    }
    profiles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{EdgeId, Node, NodeKind};

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

    #[test]
    fn profile_adjacency_filters_edge_kinds() {
        let graph = Graph::new(
            vec![
                node(0, 0.0, 0.0, NodeKind::Intersection),
                node(1, 12.0, 0.0, NodeKind::Intersection),
                node(2, 0.0, 20.0, NodeKind::Intersection),
                node(3, 12.0, 20.0, NodeKind::TransitStop),
                node(4, 25.0, 20.0, NodeKind::TransitStop),
            ],
            vec![
                edge(0, 0, 1, EdgeKind::Footway, 12.0),
                edge(1, 2, 3, EdgeKind::Road, 12.0),
                edge(2, 3, 4, EdgeKind::TramTrack, 13.0),
            ],
        );
        let index = HpaIndex::build(
            &graph,
            HpaConfig {
                cluster_size_tiles: 10,
                corridor_margin_clusters: 0,
            },
        )
        .expect("index builds");

        let c0 = index.cluster_id(ClusterCoord { x: 0, y: 0 }).unwrap();
        let c1 = index.cluster_id(ClusterCoord { x: 1, y: 0 }).unwrap();
        let c2 = index.cluster_id(ClusterCoord { x: 0, y: 2 }).unwrap();
        let c3 = index.cluster_id(ClusterCoord { x: 1, y: 2 }).unwrap();
        let c4 = index.cluster_id(ClusterCoord { x: 2, y: 2 }).unwrap();

        assert_eq!(index.adjacent_clusters(c0, RoutingProfileKey::Walk), &[c1]);
        assert!(
            index
                .adjacent_clusters(c0, RoutingProfileKey::Car)
                .is_empty()
        );
        assert_eq!(index.adjacent_clusters(c2, RoutingProfileKey::Car), &[c3]);
        assert!(
            index
                .adjacent_clusters(c3, RoutingProfileKey::Tram)
                .is_empty()
        );
        assert!(
            index
                .adjacent_clusters(c3, RoutingProfileKey::WalkTransit)
                .is_empty()
        );
    }

    #[test]
    fn rail_edges_are_not_route_adjacencies() {
        let graph = Graph::new(
            vec![
                node(0, 0.0, 0.0, NodeKind::TransitStop),
                node(1, 12.0, 0.0, NodeKind::TransitStop),
            ],
            vec![edge(0, 0, 1, EdgeKind::TramTrack, 12.0)],
        );
        let index = HpaIndex::build(
            &graph,
            HpaConfig {
                cluster_size_tiles: 10,
                corridor_margin_clusters: 0,
            },
        )
        .expect("index builds");

        let c0 = index.cluster_id(ClusterCoord { x: 0, y: 0 }).unwrap();
        assert!(
            index
                .adjacent_clusters(c0, RoutingProfileKey::Tram)
                .is_empty()
        );
        assert!(
            index
                .adjacent_clusters(c0, RoutingProfileKey::WalkTransit)
                .is_empty()
        );
    }

    #[test]
    fn same_cluster_route_uses_base_case() {
        let graph = Graph::new(
            vec![
                node(0, 0.0, 0.0, NodeKind::Intersection),
                node(1, 5.0, 0.0, NodeKind::Intersection),
            ],
            vec![edge(0, 0, 1, EdgeKind::Footway, 5.0)],
        );
        let index = HpaIndex::build(
            &graph,
            HpaConfig {
                cluster_size_tiles: 10,
                corridor_margin_clusters: 0,
            },
        )
        .expect("index builds");

        let (path, stats) = HpaRouter::find_path(
            &graph,
            &index,
            PathRequest {
                from: NodeId(0),
                to: NodeId(1),
                profile: RoutingProfileKey::Walk,
            },
            RoutingProfile::for_key(RoutingProfileKey::Walk),
        )
        .expect("same-cluster route should plan");

        assert_eq!(path.edges.len(), 1);
        assert!(stats.used_base_case);
        assert_eq!(stats.corridor_cluster_count, 1);
    }

    #[test]
    fn cross_cluster_route_uses_corridor() {
        let graph = three_cluster_walk_graph();
        let index = HpaIndex::build(
            &graph,
            HpaConfig {
                cluster_size_tiles: 10,
                corridor_margin_clusters: 0,
            },
        )
        .expect("index builds");

        let (path, stats) = HpaRouter::find_path(
            &graph,
            &index,
            PathRequest {
                from: NodeId(0),
                to: NodeId(3),
                profile: RoutingProfileKey::Walk,
            },
            RoutingProfile::for_key(RoutingProfileKey::Walk),
        )
        .expect("cross-cluster route should plan");

        assert_eq!(path.edges.len(), 3);
        assert!(!stats.used_base_case);
        assert_eq!(stats.corridor_cluster_count, 3);
        assert!(stats.abstract_clusters_visited >= 3);
    }

    #[test]
    fn corridor_between_cross_cluster_includes_intermediate_clusters() {
        let graph = three_cluster_walk_graph();
        let index = HpaIndex::build(
            &graph,
            HpaConfig {
                cluster_size_tiles: 10,
                corridor_margin_clusters: 0,
            },
        )
        .expect("index builds");

        let corridor = index
            .corridor_between(NodeId(0), NodeId(3), RoutingProfileKey::Walk)
            .expect("corridor should resolve through abstract cluster path");

        assert_eq!(
            corridor,
            HashSet::from([ClusterId(0), ClusterId(1), ClusterId(2)])
        );
    }

    #[test]
    fn missing_request_nodes_are_reported_before_cluster_resolution() {
        let graph = Graph::new(
            vec![
                node(0, 0.0, 0.0, NodeKind::Intersection),
                node(1, 5.0, 0.0, NodeKind::Intersection),
            ],
            vec![edge(0, 0, 1, EdgeKind::Footway, 5.0)],
        );
        let index = HpaIndex::build(
            &graph,
            HpaConfig {
                cluster_size_tiles: 10,
                corridor_margin_clusters: 0,
            },
        )
        .expect("index builds");

        let missing_from = HpaRouter::find_path(
            &graph,
            &index,
            PathRequest {
                from: NodeId(99),
                to: NodeId(1),
                profile: RoutingProfileKey::Walk,
            },
            RoutingProfile::for_key(RoutingProfileKey::Walk),
        )
        .unwrap_err();
        assert_eq!(
            missing_from,
            HierarchicalRoutingError::MissingNode(NodeId(99))
        );

        let missing_to = HpaRouter::find_path(
            &graph,
            &index,
            PathRequest {
                from: NodeId(0),
                to: NodeId(99),
                profile: RoutingProfileKey::Walk,
            },
            RoutingProfile::for_key(RoutingProfileKey::Walk),
        )
        .unwrap_err();
        assert_eq!(
            missing_to,
            HierarchicalRoutingError::MissingNode(NodeId(99))
        );
    }

    #[test]
    fn no_cluster_path_is_not_replanned_globally() {
        let graph = Graph::new(
            vec![
                node(0, 0.0, 0.0, NodeKind::Intersection),
                node(1, 12.0, 0.0, NodeKind::Intersection),
            ],
            vec![edge(0, 0, 1, EdgeKind::Road, 12.0)],
        );
        let index = HpaIndex::build(
            &graph,
            HpaConfig {
                cluster_size_tiles: 10,
                corridor_margin_clusters: 0,
            },
        )
        .expect("index builds");

        let error = HpaRouter::find_path(
            &graph,
            &index,
            PathRequest {
                from: NodeId(0),
                to: NodeId(1),
                profile: RoutingProfileKey::Walk,
            },
            RoutingProfile::for_key(RoutingProfileKey::Walk),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            HierarchicalRoutingError::NoClusterPath { .. }
        ));
    }

    #[test]
    fn no_corridor_path_is_not_replanned_globally() {
        let graph = Graph::new(
            vec![
                node(0, 0.0, 0.0, NodeKind::Intersection),
                node(1, 12.0, 0.0, NodeKind::Intersection),
                node(2, 25.0, 0.0, NodeKind::Intersection),
                node(3, 25.0, 12.0, NodeKind::Intersection),
                node(4, 0.0, 12.0, NodeKind::Intersection),
            ],
            vec![
                edge(0, 0, 1, EdgeKind::Footway, 12.0),
                edge(1, 1, 2, EdgeKind::Footway, 13.0),
                edge(2, 4, 3, EdgeKind::Footway, 25.0),
            ],
        );
        let mut index = HpaIndex::build(
            &graph,
            HpaConfig {
                cluster_size_tiles: 10,
                corridor_margin_clusters: 0,
            },
        )
        .expect("index builds");
        let start = index.cluster_of_node(NodeId(0)).unwrap();
        let goal = index.cluster_of_node(NodeId(3)).unwrap();
        index.force_cluster_adjacency_for_test(start, goal, RoutingProfileKey::Walk);

        let error = HpaRouter::find_path(
            &graph,
            &index,
            PathRequest {
                from: NodeId(0),
                to: NodeId(3),
                profile: RoutingProfileKey::Walk,
            },
            RoutingProfile::for_key(RoutingProfileKey::Walk),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            HierarchicalRoutingError::NoCorridorPath { .. }
        ));
    }
}
