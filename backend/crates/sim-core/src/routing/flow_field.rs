use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use bevy_ecs::prelude::*;

use crate::routing::{
    ClusterId, EdgeId, Graph, ModeState, NodeId, RoutingProfile, RoutingProfileKey,
};

#[derive(Debug, Clone, PartialEq)]
pub enum FlowFieldError {
    MissingNode(NodeId),
    MissingCluster(NodeId),
    CacheProfileMismatch {
        key: RoutingProfileKey,
        profile: RoutingProfileKey,
    },
    CacheScopeMismatch,
    CacheRequiresClusterLookup,
    NoCorridor {
        from: NodeId,
        to: NodeId,
        profile: RoutingProfileKey,
    },
    Unreachable {
        from: NodeId,
        to: NodeId,
        profile: RoutingProfileKey,
    },
    DestinationMismatch {
        requested: NodeId,
        actual: NodeId,
    },
    InvalidGraph(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlowFieldEntry {
    pub next_edge: Option<EdgeId>,
    pub next_mode: ModeState,
    pub cost_to_goal: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlowField {
    pub destination: NodeId,
    pub profile: RoutingProfileKey,
    entries: HashMap<(NodeId, ModeState), FlowFieldEntry>,
}

impl FlowField {
    pub fn entry(&self, node: NodeId, mode: ModeState) -> Option<&FlowFieldEntry> {
        self.entries.get(&(node, mode))
    }

    pub fn reachable_state_count(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowFieldScope {
    AllEdges,
    Corridor(HashSet<ClusterId>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlowFieldCacheKey {
    pub destination: NodeId,
    pub profile: RoutingProfileKey,
    pub graph_generation: u64,
    pub scope_discriminator: u64,
    pub corridor_hash: u64,
}

const FLOW_FIELD_SCOPE_ALL_EDGES: u64 = 0x5fbd_63ea_13c9_1201;
const FLOW_FIELD_SCOPE_CORRIDOR: u64 = 0xa419_2bd0_c7e5_94f3;
const FLOW_FIELD_ALL_EDGES_HASH: u64 = 0;
const FNV_OFFSET_BASIS: u64 = 1469598103934665603;
const FNV_PRIME: u64 = 1099511628211;

fn corridor_hash(corridor: &[ClusterId]) -> u64 {
    let mut sorted = corridor.to_vec();
    sorted.sort_unstable();

    let mut hash = FNV_OFFSET_BASIS;
    hash ^= FLOW_FIELD_SCOPE_CORRIDOR;
    hash = hash.wrapping_mul(FNV_PRIME);
    for cluster in sorted {
        hash ^= u64::from(cluster.0);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    (hash << 1) | 1
}

fn key_parts_for_scope(scope: &FlowFieldScope) -> (u64, u64) {
    match scope {
        FlowFieldScope::AllEdges => (FLOW_FIELD_SCOPE_ALL_EDGES, FLOW_FIELD_ALL_EDGES_HASH),
        FlowFieldScope::Corridor(corridor) => {
            let clusters: Vec<_> = corridor.iter().copied().collect();
            (FLOW_FIELD_SCOPE_CORRIDOR, corridor_hash(&clusters))
        }
    }
}

impl FlowFieldCacheKey {
    pub fn all_edges(
        destination: NodeId,
        profile: RoutingProfileKey,
        graph_generation: u64,
    ) -> Self {
        Self {
            destination,
            profile,
            graph_generation,
            scope_discriminator: FLOW_FIELD_SCOPE_ALL_EDGES,
            corridor_hash: FLOW_FIELD_ALL_EDGES_HASH,
        }
    }

    pub fn new(
        destination: NodeId,
        profile: RoutingProfileKey,
        graph_generation: u64,
        corridor: &[ClusterId],
    ) -> Self {
        Self {
            destination,
            profile,
            graph_generation,
            scope_discriminator: FLOW_FIELD_SCOPE_CORRIDOR,
            corridor_hash: corridor_hash(corridor),
        }
    }

    pub fn matches_scope(&self, scope: &FlowFieldScope) -> bool {
        let (scope_discriminator, corridor_hash) = key_parts_for_scope(scope);
        self.scope_discriminator == scope_discriminator && self.corridor_hash == corridor_hash
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FlowFieldCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub evictions: u64,
}

#[derive(Resource)]
pub struct FlowFieldCache {
    capacity: usize,
    entries: HashMap<FlowFieldCacheKey, Arc<FlowField>>,
    order: VecDeque<FlowFieldCacheKey>,
    stats: FlowFieldCacheStats,
}

impl Default for FlowFieldCache {
    fn default() -> Self {
        Self::with_capacity(4096)
    }
}

impl FlowFieldCache {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: HashMap::new(),
            order: VecDeque::new(),
            stats: FlowFieldCacheStats::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn stats(&self) -> FlowFieldCacheStats {
        self.stats
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    pub fn insert(&mut self, key: FlowFieldCacheKey, field: Arc<FlowField>) {
        if !self.entries.contains_key(&key) {
            self.order.push_back(key);
        }
        self.entries.insert(key, field);
        self.stats.inserts += 1;

        while self.entries.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                if self.entries.remove(&oldest).is_some() {
                    self.stats.evictions += 1;
                }
            } else {
                break;
            }
        }
    }

    pub fn get_or_build(
        &mut self,
        graph: &Graph,
        key: FlowFieldCacheKey,
        profile: RoutingProfile,
        scope: FlowFieldScope,
    ) -> Result<Arc<FlowField>, FlowFieldError> {
        validate_cache_request(key, profile.key, &scope)?;
        match scope {
            FlowFieldScope::AllEdges => {
                self.get_or_build_with_cluster_lookup(graph, key, profile, scope, |_| None)
            }
            FlowFieldScope::Corridor(_) => Err(FlowFieldError::CacheRequiresClusterLookup),
        }
    }

    pub fn get_or_build_with_cluster_lookup<F>(
        &mut self,
        graph: &Graph,
        key: FlowFieldCacheKey,
        profile: RoutingProfile,
        scope: FlowFieldScope,
        cluster_of_node: F,
    ) -> Result<Arc<FlowField>, FlowFieldError>
    where
        F: Fn(NodeId) -> Option<ClusterId>,
    {
        validate_cache_request(key, profile.key, &scope)?;
        if let Some(existing) = self.entries.get(&key) {
            self.stats.hits += 1;
            return Ok(Arc::clone(existing));
        }

        self.stats.misses += 1;
        let field = Arc::new(FlowFieldRouter::build_with_cluster_lookup(
            graph,
            key.destination,
            profile,
            scope,
            cluster_of_node,
        )?);
        self.insert(key, Arc::clone(&field));
        Ok(field)
    }
}

fn validate_cache_request(
    key: FlowFieldCacheKey,
    profile: RoutingProfileKey,
    scope: &FlowFieldScope,
) -> Result<(), FlowFieldError> {
    if key.profile != profile {
        return Err(FlowFieldError::CacheProfileMismatch {
            key: key.profile,
            profile,
        });
    }
    if !key.matches_scope(scope) {
        return Err(FlowFieldError::CacheScopeMismatch);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct QueueEntry {
    node: NodeId,
    mode: ModeState,
    cost: f32,
}

impl PartialEq for QueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost && self.node == other.node && self.mode == other.mode
    }
}

impl Eq for QueueEntry {}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
            .then_with(|| other.node.0.cmp(&self.node.0))
            .then_with(|| other.mode.cmp(&self.mode))
    }
}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub struct FlowFieldRouter;

impl FlowFieldRouter {
    pub fn build(
        graph: &Graph,
        destination: NodeId,
        profile: RoutingProfile,
        scope: FlowFieldScope,
    ) -> Result<FlowField, FlowFieldError> {
        Self::build_with_cluster_lookup(graph, destination, profile, scope, |_| None)
    }

    pub fn build_with_cluster_lookup<F>(
        graph: &Graph,
        destination: NodeId,
        profile: RoutingProfile,
        scope: FlowFieldScope,
        cluster_of_node: F,
    ) -> Result<FlowField, FlowFieldError>
    where
        F: Fn(NodeId) -> Option<ClusterId>,
    {
        validate_node(graph, destination)?;

        let mut open = BinaryHeap::new();
        let mut entries: HashMap<(NodeId, ModeState), FlowFieldEntry> = HashMap::new();

        for destination_mode in terminal_destination_modes(profile.key).iter().copied() {
            entries.insert(
                (destination, destination_mode),
                FlowFieldEntry {
                    next_edge: None,
                    next_mode: destination_mode,
                    cost_to_goal: 0.0,
                },
            );
            open.push(QueueEntry {
                node: destination,
                mode: destination_mode,
                cost: 0.0,
            });
        }

        while let Some(entry) = open.pop() {
            let best = entries
                .get(&(entry.node, entry.mode))
                .map(|entry| entry.cost_to_goal)
                .unwrap_or(f32::INFINITY);
            if entry.cost > best {
                continue;
            }

            for edge_id in graph.incoming(entry.node) {
                let edge = graph.edge(*edge_id);
                if !scope_allows_edge(&scope, edge.from, edge.to, &cluster_of_node)? {
                    continue;
                }

                let from_node = graph.node(edge.from);
                for prior_mode in [ModeState::Walking, ModeState::Driving, ModeState::OnTram] {
                    let Some((next_mode, edge_cost)) =
                        profile.transition(prior_mode, from_node.kind, edge)
                    else {
                        continue;
                    };
                    if next_mode != entry.mode {
                        continue;
                    }
                    if edge_cost < 0.0 || !edge_cost.is_finite() {
                        return Err(FlowFieldError::InvalidGraph(
                            "edge cost must be finite and non-negative",
                        ));
                    }

                    let next_cost = entry.cost + edge_cost;
                    let key = (edge.from, prior_mode);
                    let old_cost = entries
                        .get(&key)
                        .map(|existing| existing.cost_to_goal)
                        .unwrap_or(f32::INFINITY);
                    if next_cost < old_cost {
                        entries.insert(
                            key,
                            FlowFieldEntry {
                                next_edge: Some(*edge_id),
                                next_mode,
                                cost_to_goal: next_cost,
                            },
                        );
                        open.push(QueueEntry {
                            node: edge.from,
                            mode: prior_mode,
                            cost: next_cost,
                        });
                    }
                }
            }
        }

        Ok(FlowField {
            destination,
            profile: profile.key,
            entries,
        })
    }

    pub fn require_reachable(
        field: &FlowField,
        from: NodeId,
        to: NodeId,
        mode: ModeState,
    ) -> Result<(), FlowFieldError> {
        if to != field.destination {
            return Err(FlowFieldError::DestinationMismatch {
                requested: to,
                actual: field.destination,
            });
        }

        if field.entry(from, mode).is_some() {
            Ok(())
        } else {
            Err(FlowFieldError::Unreachable {
                from,
                to,
                profile: field.profile,
            })
        }
    }
}

fn terminal_destination_modes(profile: RoutingProfileKey) -> &'static [ModeState] {
    match profile {
        RoutingProfileKey::Walk => &[ModeState::Walking],
        RoutingProfileKey::Car => &[ModeState::Driving],
        RoutingProfileKey::Tram => &[ModeState::OnTram],
        RoutingProfileKey::WalkTransit => &[ModeState::Walking, ModeState::OnTram],
    }
}

fn validate_node(graph: &Graph, node: NodeId) -> Result<(), FlowFieldError> {
    if (node.0 as usize) < graph.node_count() {
        Ok(())
    } else {
        Err(FlowFieldError::MissingNode(node))
    }
}

fn scope_allows_edge<F>(
    scope: &FlowFieldScope,
    from: NodeId,
    to: NodeId,
    cluster_of_node: &F,
) -> Result<bool, FlowFieldError>
where
    F: Fn(NodeId) -> Option<ClusterId>,
{
    match scope {
        FlowFieldScope::AllEdges => Ok(true),
        FlowFieldScope::Corridor(clusters) => {
            let Some(from_cluster) = cluster_of_node(from) else {
                return Err(FlowFieldError::MissingCluster(from));
            };
            let Some(to_cluster) = cluster_of_node(to) else {
                return Err(FlowFieldError::MissingCluster(to));
            };
            Ok(clusters.contains(&from_cluster) && clusters.contains(&to_cluster))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::routing::{Edge, EdgeKind, Node, NodeKind};

    fn node(id: u32, x: f32, y: f32) -> Node {
        Node {
            id: NodeId(id),
            position: (x, y),
            kind: NodeKind::Intersection,
            legacy_id: None,
        }
    }

    fn typed_node(id: u32, x: f32, y: f32, kind: NodeKind) -> Node {
        Node {
            id: NodeId(id),
            position: (x, y),
            kind,
            legacy_id: None,
        }
    }

    fn edge(id: u32, from: u32, to: u32, kind: EdgeKind, legacy: &str) -> Edge {
        Edge {
            id: EdgeId(id),
            from: NodeId(from),
            to: NodeId(to),
            polyline: vec![(from as f32, 0.0), (to as f32, 0.0)],
            length: 1.0,
            kind,
            speed_limit: 1.0,
            capacity: 1,
            legacy_id: Some(legacy.to_string()),
        }
    }

    fn walk_graph() -> Graph {
        Graph::new(
            vec![node(0, 0.0, 0.0), node(1, 1.0, 0.0), node(2, 2.0, 0.0)],
            vec![
                edge(0, 0, 1, EdgeKind::Footway, "walk:0"),
                edge(1, 1, 2, EdgeKind::Footway, "walk:1"),
                edge(2, 0, 2, EdgeKind::Road, "road:shortcut"),
            ],
        )
    }

    #[test]
    fn field_points_multiple_origins_to_destination() {
        let graph = walk_graph();
        let field = FlowFieldRouter::build(
            &graph,
            NodeId(2),
            RoutingProfile::for_key(RoutingProfileKey::Walk),
            FlowFieldScope::AllEdges,
        )
        .expect("walk field should build");
        assert_eq!(
            field
                .entry(NodeId(0), ModeState::Walking)
                .unwrap()
                .next_edge,
            Some(EdgeId(0))
        );
        assert_eq!(
            field
                .entry(NodeId(1), ModeState::Walking)
                .unwrap()
                .next_edge,
            Some(EdgeId(1))
        );
        assert_eq!(
            field
                .entry(NodeId(2), ModeState::Walking)
                .unwrap()
                .next_edge,
            None
        );
    }

    #[test]
    fn field_respects_profile_edge_legality() {
        let graph = walk_graph();
        let walk_field = FlowFieldRouter::build(
            &graph,
            NodeId(2),
            RoutingProfile::for_key(RoutingProfileKey::Walk),
            FlowFieldScope::AllEdges,
        )
        .expect("walk field should build");
        assert_ne!(
            walk_field
                .entry(NodeId(0), ModeState::Walking)
                .unwrap()
                .next_edge,
            Some(EdgeId(2))
        );

        let car_field = FlowFieldRouter::build(
            &graph,
            NodeId(2),
            RoutingProfile::for_key(RoutingProfileKey::Car),
            FlowFieldScope::AllEdges,
        )
        .expect("car field should build over road edge");
        assert_eq!(
            car_field
                .entry(NodeId(0), ModeState::Driving)
                .unwrap()
                .next_edge,
            Some(EdgeId(2))
        );
    }

    #[test]
    fn corridor_scope_rejects_edges_outside_cluster_set() {
        let graph = walk_graph();
        let mut clusters = HashSet::new();
        clusters.insert(ClusterId(0));
        let result = FlowFieldRouter::build_with_cluster_lookup(
            &graph,
            NodeId(2),
            RoutingProfile::for_key(RoutingProfileKey::Walk),
            FlowFieldScope::Corridor(clusters),
            |node| match node.0 {
                0 | 1 => Some(ClusterId(0)),
                2 => Some(ClusterId(1)),
                _ => None,
            },
        );

        let field = result.expect("field can build even when origin is unreachable");
        assert_eq!(
            FlowFieldRouter::require_reachable(&field, NodeId(0), NodeId(2), ModeState::Walking,),
            Err(FlowFieldError::Unreachable {
                from: NodeId(0),
                to: NodeId(2),
                profile: RoutingProfileKey::Walk,
            })
        );
    }

    #[test]
    fn require_reachable_rejects_mismatched_destination() {
        let graph = walk_graph();
        let field = FlowFieldRouter::build(
            &graph,
            NodeId(2),
            RoutingProfile::for_key(RoutingProfileKey::Walk),
            FlowFieldScope::AllEdges,
        )
        .expect("walk field should build");

        assert_eq!(
            FlowFieldRouter::require_reachable(&field, NodeId(0), NodeId(1), ModeState::Walking,),
            Err(FlowFieldError::DestinationMismatch {
                requested: NodeId(1),
                actual: NodeId(2),
            })
        );
    }

    #[test]
    fn walk_transit_field_allows_tram_terminal_arrival() {
        let graph = Graph::new(
            vec![
                typed_node(0, 0.0, 0.0, NodeKind::TransitStop),
                typed_node(1, 1.0, 0.0, NodeKind::TransitStop),
            ],
            vec![edge(0, 0, 1, EdgeKind::TramTrack, "tram:0")],
        );

        let field = FlowFieldRouter::build(
            &graph,
            NodeId(1),
            RoutingProfile::for_key(RoutingProfileKey::WalkTransit),
            FlowFieldScope::AllEdges,
        )
        .expect("walk-transit field should build");

        assert_eq!(
            field
                .entry(NodeId(0), ModeState::Walking)
                .unwrap()
                .next_edge,
            Some(EdgeId(0))
        );
    }

    #[test]
    fn cache_tracks_miss_hit_insert_and_eviction() {
        let graph = walk_graph();
        let mut cache = FlowFieldCache::with_capacity(1);
        let key = FlowFieldCacheKey::all_edges(NodeId(2), RoutingProfileKey::Walk, 0);

        let first = cache
            .get_or_build(
                &graph,
                key,
                RoutingProfile::for_key(RoutingProfileKey::Walk),
                FlowFieldScope::AllEdges,
            )
            .expect("first field should build");
        let second = cache
            .get_or_build(
                &graph,
                key,
                RoutingProfile::for_key(RoutingProfileKey::Walk),
                FlowFieldScope::AllEdges,
            )
            .expect("second field should hit");

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            cache.stats(),
            FlowFieldCacheStats {
                hits: 1,
                misses: 1,
                inserts: 1,
                evictions: 0
            }
        );

        let other = FlowFieldCacheKey::all_edges(NodeId(1), RoutingProfileKey::Walk, 0);
        let _ = cache
            .get_or_build(
                &graph,
                other,
                RoutingProfile::for_key(RoutingProfileKey::Walk),
                FlowFieldScope::AllEdges,
            )
            .expect("other field should build");
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.stats().evictions, 1);
    }

    #[test]
    fn cache_rejects_profile_mismatch_at_runtime() {
        let graph = walk_graph();
        let mut cache = FlowFieldCache::with_capacity(1);
        let key = FlowFieldCacheKey::all_edges(NodeId(2), RoutingProfileKey::Walk, 0);

        let result = cache.get_or_build(
            &graph,
            key,
            RoutingProfile::for_key(RoutingProfileKey::Car),
            FlowFieldScope::AllEdges,
        );

        assert_eq!(
            result,
            Err(FlowFieldError::CacheProfileMismatch {
                key: RoutingProfileKey::Walk,
                profile: RoutingProfileKey::Car,
            })
        );
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.stats().misses, 0);
    }

    #[test]
    fn cache_rejects_scope_mismatch_at_runtime() {
        let graph = walk_graph();
        let mut cache = FlowFieldCache::with_capacity(2);
        let all_edges_key = FlowFieldCacheKey::all_edges(NodeId(2), RoutingProfileKey::Walk, 0);
        let corridor_key =
            FlowFieldCacheKey::new(NodeId(2), RoutingProfileKey::Walk, 0, &[ClusterId(0)]);
        let mut clusters = HashSet::new();
        clusters.insert(ClusterId(0));

        assert_eq!(
            cache.get_or_build(
                &graph,
                all_edges_key,
                RoutingProfile::for_key(RoutingProfileKey::Walk),
                FlowFieldScope::Corridor(clusters),
            ),
            Err(FlowFieldError::CacheScopeMismatch)
        );
        assert_eq!(
            cache.get_or_build(
                &graph,
                corridor_key,
                RoutingProfile::for_key(RoutingProfileKey::Walk),
                FlowFieldScope::AllEdges,
            ),
            Err(FlowFieldError::CacheScopeMismatch)
        );
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.stats().misses, 0);
    }

    #[test]
    fn cache_rejects_corridor_scope_with_different_cluster_set() {
        let graph = walk_graph();
        let mut cache = FlowFieldCache::with_capacity(1);
        let key = FlowFieldCacheKey::new(NodeId(2), RoutingProfileKey::Walk, 0, &[ClusterId(0)]);

        let result = cache.get_or_build_with_cluster_lookup(
            &graph,
            key,
            RoutingProfile::for_key(RoutingProfileKey::Walk),
            FlowFieldScope::Corridor(HashSet::from([ClusterId(1)])),
            |_| Some(ClusterId(1)),
        );

        assert_eq!(result, Err(FlowFieldError::CacheScopeMismatch));
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.stats().misses, 0);
    }

    #[test]
    fn simple_cache_api_rejects_corridor_scope_without_cluster_lookup() {
        let graph = walk_graph();
        let mut cache = FlowFieldCache::with_capacity(1);
        let key = FlowFieldCacheKey::new(NodeId(2), RoutingProfileKey::Walk, 0, &[ClusterId(0)]);
        let mut clusters = HashSet::new();
        clusters.insert(ClusterId(0));

        let result = cache.get_or_build(
            &graph,
            key,
            RoutingProfile::for_key(RoutingProfileKey::Walk),
            FlowFieldScope::Corridor(clusters),
        );

        assert_eq!(result, Err(FlowFieldError::CacheRequiresClusterLookup));
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.stats().misses, 0);
    }

    #[test]
    fn corridor_cache_builds_with_cluster_lookup() {
        let graph = walk_graph();
        let mut cache = FlowFieldCache::with_capacity(1);
        let key = FlowFieldCacheKey::new(NodeId(2), RoutingProfileKey::Walk, 0, &[ClusterId(0)]);
        let mut clusters = HashSet::new();
        clusters.insert(ClusterId(0));

        let field = cache
            .get_or_build_with_cluster_lookup(
                &graph,
                key,
                RoutingProfile::for_key(RoutingProfileKey::Walk),
                FlowFieldScope::Corridor(clusters),
                |_| Some(ClusterId(0)),
            )
            .expect("corridor field should build with cluster lookup");

        assert_eq!(
            field
                .entry(NodeId(0), ModeState::Walking)
                .unwrap()
                .next_edge,
            Some(EdgeId(0))
        );
        assert_eq!(
            cache.stats(),
            FlowFieldCacheStats {
                hits: 0,
                misses: 1,
                inserts: 1,
                evictions: 0
            }
        );
    }
}
