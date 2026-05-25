use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use bevy_ecs::prelude::*;

use crate::routing::{
    AStarRouter, Graph, NodeId, PathRequest, PlannedPath, RoutingError, RoutingProfile,
    RoutingProfileKey,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PathCacheKey {
    pub from: NodeId,
    pub to: NodeId,
    pub profile: RoutingProfileKey,
    pub graph_generation: u64,
}

impl PathCacheKey {
    pub fn new(request: PathRequest, graph_generation: u64) -> Self {
        Self {
            from: request.from,
            to: request.to,
            profile: request.profile,
            graph_generation,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PathCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub evictions: u64,
}

#[derive(Resource)]
pub struct PathCache {
    capacity: usize,
    graph_generation: u64,
    entries: HashMap<PathCacheKey, Arc<PlannedPath>>,
    order: VecDeque<PathCacheKey>,
    stats: PathCacheStats,
}

impl Default for PathCache {
    fn default() -> Self {
        Self::with_capacity(8192)
    }
}

impl PathCache {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            graph_generation: 0,
            entries: HashMap::new(),
            order: VecDeque::new(),
            stats: PathCacheStats::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn stats(&self) -> PathCacheStats {
        self.stats
    }

    pub fn insert(&mut self, key: PathCacheKey, path: Arc<PlannedPath>) {
        if !self.entries.contains_key(&key) {
            self.order.push_back(key);
        }
        self.entries.insert(key, path);
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

    pub fn get_or_plan(
        &mut self,
        graph: &Graph,
        request: PathRequest,
        profile: RoutingProfile,
    ) -> Result<Arc<PlannedPath>, RoutingError> {
        debug_assert_eq!(request.profile, profile.key);
        let key = PathCacheKey::new(request, self.graph_generation);
        if let Some(path) = self.entries.get(&key) {
            self.stats.hits += 1;
            return Ok(Arc::clone(path));
        }

        self.stats.misses += 1;
        let planned = Arc::new(AStarRouter::find_path(graph, request, profile)?);
        self.insert(key, Arc::clone(&planned));
        Ok(planned)
    }

    pub fn clear_for_generation(&mut self, graph_generation: u64) {
        self.graph_generation = graph_generation;
        self.entries.clear();
        self.order.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{Edge, EdgeId, EdgeKind, Node, NodeKind};

    fn graph() -> Graph {
        Graph::new(
            vec![
                Node {
                    id: NodeId(0),
                    position: (0.0, 0.0),
                    kind: NodeKind::Intersection,
                    legacy_id: None,
                },
                Node {
                    id: NodeId(1),
                    position: (10.0, 0.0),
                    kind: NodeKind::Intersection,
                    legacy_id: None,
                },
            ],
            vec![Edge {
                id: EdgeId(0),
                from: NodeId(0),
                to: NodeId(1),
                polyline: vec![(0.0, 0.0), (10.0, 0.0)],
                length: 10.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: None,
            }],
        )
    }

    fn request(profile: RoutingProfileKey) -> PathRequest {
        PathRequest {
            from: NodeId(0),
            to: NodeId(1),
            profile,
        }
    }

    #[test]
    fn cache_tracks_miss_then_hit() {
        let graph = graph();
        let mut cache = PathCache::with_capacity(8);
        let first = cache
            .get_or_plan(
                &graph,
                request(RoutingProfileKey::Walk),
                RoutingProfile::for_key(RoutingProfileKey::Walk),
            )
            .expect("first route should plan");
        let second = cache
            .get_or_plan(
                &graph,
                request(RoutingProfileKey::Walk),
                RoutingProfile::for_key(RoutingProfileKey::Walk),
            )
            .expect("second route should hit cache");
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            cache.stats(),
            PathCacheStats {
                hits: 1,
                misses: 1,
                inserts: 1,
                evictions: 0,
            }
        );
    }

    #[test]
    fn cache_key_distinguishes_profile() {
        let walk = PathCacheKey::new(request(RoutingProfileKey::Walk), 0);
        let car = PathCacheKey::new(request(RoutingProfileKey::Car), 0);
        assert_ne!(walk, car);
    }

    #[test]
    fn cache_evicts_at_capacity() {
        let path = Arc::new(PlannedPath {
            from: NodeId(0),
            to: NodeId(1),
            profile: RoutingProfileKey::Walk,
            edges: Vec::new(),
            total_cost: 0.0,
            total_length: 0.0,
        });
        let mut cache = PathCache::with_capacity(1);
        cache.insert(
            PathCacheKey::new(request(RoutingProfileKey::Walk), 0),
            Arc::clone(&path),
        );
        cache.insert(PathCacheKey::new(request(RoutingProfileKey::Car), 0), path);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.stats().evictions, 1);
    }

    #[test]
    fn no_path_errors_are_not_cached() {
        let graph = Graph::default();
        let mut cache = PathCache::with_capacity(8);
        let result = cache.get_or_plan(
            &graph,
            request(RoutingProfileKey::Walk),
            RoutingProfile::for_key(RoutingProfileKey::Walk),
        );
        assert_eq!(result, Err(RoutingError::MissingNode(NodeId(0))));
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.stats().misses, 1);
    }
}
