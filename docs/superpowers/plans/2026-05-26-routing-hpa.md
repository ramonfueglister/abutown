# Routing HPA* Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Archived/closed in the 2026-05-29 documentation cleanup. This checklist is historical; `progress.md` and later plans are authoritative for current implementation status.

**Goal:** Add deterministic corridor-HPA* routing over the existing 8c graph, A* profiles, and path cache without changing mobility execution, wire bytes, persistence bytes, or frontend state.

**Architecture:** Add a focused `routing/hpa.rs` module that builds a fixed-size cluster index from `Graph`, discovers real cross-cluster portals, searches an abstract cluster graph, and then delegates final path legality to 8c A* through a corridor edge constraint. Extend `AStarRouter` with constrained search while keeping the current unconstrained API as a thin `AllEdges` wrapper.

**Tech Stack:** Rust 2024, `bevy_ecs 0.18`, existing `sim_core::routing::{Graph, AStarRouter, RoutingProfile}`, `BinaryHeap`, `HashMap`, `HashSet`, `BTreeSet`, deterministic `NodeId`/`ClusterId` ordering.

---

## File Structure

### Create

- `backend/crates/sim-core/src/routing/hpa.rs` - HPA config, cluster identity, index, profile-aware adjacency, corridor router, plugin-facing resource, tests.

### Modify

- `backend/crates/sim-core/src/routing/pathfinding.rs` - add `EdgeConstraint`, `AllEdges`, and `AStarRouter::find_path_with_constraint`.
- `backend/crates/sim-core/src/routing/mod.rs` - add `pub mod hpa;` and re-export HPA public API.
- `backend/crates/sim-core/src/routing/plugin.rs` - add `HierarchicalRoutingPlugin` and plugin tests.
- `backend/crates/sim-server/src/runtime.rs` - install `HierarchicalRoutingPlugin` after `PathfindingPlugin`; add runtime integration tests.
- `progress.md` - add final 8d verification entry after all verification passes.

### Do Not Modify

- `backend/crates/protocol/proto/abutown.proto`
- `backend/crates/sim-core/src/mobility/records.rs`
- `backend/crates/sim-core/src/mobility/persist_snapshot.rs`
- `src/**`
- `tests/backend/**`

---

## Task 1: A* Edge Constraints

**Files:**
- Modify: `backend/crates/sim-core/src/routing/pathfinding.rs`

- [x] **Step 1: Write failing constraint tests**

Append these tests inside the existing `#[cfg(test)] mod tests` in `backend/crates/sim-core/src/routing/pathfinding.rs`:

```rust
    struct RejectEdge(EdgeId);

    impl EdgeConstraint for RejectEdge {
        fn allows(&self, _graph: &Graph, edge: &Edge) -> bool {
            edge.id != self.0
        }
    }

    #[test]
    fn edge_constraint_blocks_disallowed_edges() {
        let graph = Graph::new(
            vec![
                node(0, 0.0, 0.0, NodeKind::Intersection),
                node(1, 10.0, 0.0, NodeKind::Intersection),
                node(2, 20.0, 0.0, NodeKind::Intersection),
            ],
            vec![
                edge(0, 0, 1, EdgeKind::Footway, 10.0),
                edge(1, 1, 2, EdgeKind::Footway, 10.0),
            ],
        );

        let result = AStarRouter::find_path_with_constraint(
            &graph,
            PathRequest {
                from: NodeId(0),
                to: NodeId(2),
                profile: RoutingProfileKey::Walk,
            },
            RoutingProfile::for_key(RoutingProfileKey::Walk),
            &RejectEdge(EdgeId(1)),
        );

        assert_eq!(
            result,
            Err(RoutingError::NoPath {
                from: NodeId(0),
                to: NodeId(2),
                profile: RoutingProfileKey::Walk,
            })
        );
    }

    #[test]
    fn unconstrained_find_path_keeps_existing_behavior() {
        let graph = Graph::new(
            vec![
                node(0, 0.0, 0.0, NodeKind::Intersection),
                node(1, 10.0, 0.0, NodeKind::Intersection),
                node(2, 20.0, 0.0, NodeKind::Intersection),
            ],
            vec![
                edge(0, 0, 1, EdgeKind::Footway, 10.0),
                edge(1, 1, 2, EdgeKind::Footway, 10.0),
            ],
        );

        let path = AStarRouter::find_path(
            &graph,
            PathRequest {
                from: NodeId(0),
                to: NodeId(2),
                profile: RoutingProfileKey::Walk,
            },
            RoutingProfile::for_key(RoutingProfileKey::Walk),
        )
        .expect("unconstrained route should still work");

        assert_eq!(
            path.edges.iter().map(|edge| edge.edge_id).collect::<Vec<_>>(),
            vec![EdgeId(0), EdgeId(1)]
        );
    }
```

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa/backend
cargo test -p sim-core routing::pathfinding::tests::edge_constraint_blocks_disallowed_edges -- --nocapture
```

Expected: compile fails because `EdgeConstraint`, `AllEdges`, and `find_path_with_constraint` are not defined.

- [x] **Step 3: Add the constraint API**

In `backend/crates/sim-core/src/routing/pathfinding.rs`, extend the import and add the public trait near `pub struct AStarRouter;`:

```rust
use crate::routing::{
    Edge, EdgeId, Graph, ModeState, NodeId, NodeSpatialIndex, RoutingProfile, RoutingProfileKey,
};

pub trait EdgeConstraint {
    fn allows(&self, graph: &Graph, edge: &Edge) -> bool;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AllEdges;

impl EdgeConstraint for AllEdges {
    fn allows(&self, _graph: &Graph, _edge: &Edge) -> bool {
        true
    }
}
```

Replace the body of `AStarRouter::find_path` with a delegate:

```rust
    pub fn find_path(
        graph: &Graph,
        request: PathRequest,
        profile: RoutingProfile,
    ) -> Result<PlannedPath, RoutingError> {
        Self::find_path_with_constraint(graph, request, profile, &AllEdges)
    }
```

Add the constrained method immediately after `find_path` and move the existing search body into it. The only new line inside the outgoing-edge loop is the constraint check before profile transition:

```rust
    pub fn find_path_with_constraint<C: EdgeConstraint>(
        graph: &Graph,
        request: PathRequest,
        profile: RoutingProfile,
        constraint: &C,
    ) -> Result<PlannedPath, RoutingError> {
        validate_node(graph, request.from)?;
        validate_node(graph, request.to)?;

        if request.from == request.to {
            return Ok(PlannedPath {
                from: request.from,
                to: request.to,
                profile: request.profile,
                edges: Vec::new(),
                total_cost: 0.0,
                total_length: 0.0,
            });
        }

        let start = SearchState {
            node: request.from,
            mode: profile.initial_mode(),
        };
        let mut open = BinaryHeap::new();
        let mut best: HashMap<SearchState, f32> = HashMap::new();
        let mut came_from: HashMap<SearchState, (SearchState, PathEdge)> = HashMap::new();

        best.insert(start, 0.0);
        open.push(QueueEntry {
            state: start,
            known_cost: 0.0,
            estimated_total: heuristic(graph, request.from, request.to, profile),
        });

        while let Some(entry) = open.pop() {
            if entry.state.node == request.to {
                return reconstruct_path(graph, request, start, entry.state, &came_from);
            }

            if entry.known_cost > *best.get(&entry.state).unwrap_or(&f32::INFINITY) {
                continue;
            }

            let current_node = graph.node(entry.state.node);
            for edge_id in graph.outgoing(entry.state.node) {
                let edge = graph.edge(*edge_id);
                if !constraint.allows(graph, edge) {
                    continue;
                }
                let Some((next_mode, edge_cost)) =
                    profile.transition(entry.state.mode, current_node.kind, edge)
                else {
                    continue;
                };
                if edge_cost < 0.0 || !edge_cost.is_finite() {
                    return Err(RoutingError::InvalidGraph(
                        "edge cost must be finite and non-negative",
                    ));
                }

                let next_state = SearchState {
                    node: edge.to,
                    mode: next_mode,
                };
                let next_cost = entry.known_cost + edge_cost;
                if next_cost < *best.get(&next_state).unwrap_or(&f32::INFINITY) {
                    best.insert(next_state, next_cost);
                    came_from.insert(
                        next_state,
                        (
                            entry.state,
                            PathEdge {
                                edge_id: *edge_id,
                                mode: next_mode,
                                cost: edge_cost,
                            },
                        ),
                    );
                    open.push(QueueEntry {
                        state: next_state,
                        known_cost: next_cost,
                        estimated_total: next_cost + heuristic(graph, edge.to, request.to, profile),
                    });
                }
            }
        }

        Err(RoutingError::NoPath {
            from: request.from,
            to: request.to,
            profile: request.profile,
        })
    }
```

- [x] **Step 4: Run targeted tests**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa/backend
cargo test -p sim-core routing::pathfinding -- --nocapture
```

Expected: all `routing::pathfinding` tests pass.

- [x] **Step 5: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa
git add backend/crates/sim-core/src/routing/pathfinding.rs
git commit -m "feat(8d): add constrained pathfinding"
```

---

## Task 2: HPA Index and Portals

**Files:**
- Create: `backend/crates/sim-core/src/routing/hpa.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs`

- [x] **Step 1: Add the module and failing index tests**

Add this line to `backend/crates/sim-core/src/routing/mod.rs`:

```rust
pub mod hpa;
```

Add this re-export block to `backend/crates/sim-core/src/routing/mod.rs`:

```rust
pub use hpa::{
    ClusterCoord, ClusterId, HierarchicalRoutingError, HpaConfig, HpaIndex, HpaRouteStats,
    HpaRouter,
};
```

Create `backend/crates/sim-core/src/routing/hpa.rs` with tests first:

```rust
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use bevy_ecs::prelude::*;

use crate::routing::{
    Edge, EdgeConstraint, EdgeId, EdgeKind, Graph, Node, NodeId, NodeKind, PathRequest,
    PlannedPath, RoutingError, RoutingProfile, RoutingProfileKey,
};

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

        assert_eq!(cluster_coord_for((0.0, 0.0), config), ClusterCoord { x: 0, y: 0 });
        assert_eq!(cluster_coord_for((9.9, 0.0), config), ClusterCoord { x: 0, y: 0 });
        assert_eq!(cluster_coord_for((10.0, 0.0), config), ClusterCoord { x: 1, y: 0 });
        assert_eq!(cluster_coord_for((-0.1, 0.0), config), ClusterCoord { x: -1, y: 0 });
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
        assert_eq!(index.cluster_coord(ClusterId(0)), ClusterCoord { x: 0, y: 0 });
        assert_eq!(index.cluster_coord(ClusterId(1)), ClusterCoord { x: 1, y: 0 });
        assert_eq!(index.cluster_coord(ClusterId(2)), ClusterCoord { x: 2, y: 0 });
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
```

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa/backend
cargo test -p sim-core routing::hpa -- --nocapture
```

Expected: compile fails because HPA types and functions are not implemented.

- [x] **Step 3: Implement config, ids, index storage, and portal detection**

Replace the top of `backend/crates/sim-core/src/routing/hpa.rs`, keeping the tests, with:

```rust
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

        let mut portal_sets: Vec<BTreeSet<NodeId>> = vec![BTreeSet::new(); cluster_coords.len()];
        let mut adjacency_sets: HashMap<(ClusterId, RoutingProfileKey), BTreeSet<ClusterId>> =
            HashMap::new();

        for edge in graph.edges() {
            let from_cluster = cluster_for_node(&node_clusters, edge.from)?;
            let to_cluster = cluster_for_node(&node_clusters, edge.to)?;
            if from_cluster == to_cluster {
                continue;
            }

            portal_sets[from_cluster.0 as usize].insert(edge.from);
            portal_sets[to_cluster.0 as usize].insert(edge.to);

            for profile in profiles_for_cross_cluster_edge(graph, edge) {
                adjacency_sets
                    .entry((from_cluster, profile))
                    .or_default()
                    .insert(to_cluster);
            }
        }

        let cluster_portals: Vec<Vec<NodeId>> = portal_sets
            .into_iter()
            .map(|set| set.into_iter().collect())
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
        self.node_clusters.get(node.0 as usize).and_then(|entry| *entry)
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

    fn adjacent_clusters(&self, cluster: ClusterId, profile: RoutingProfileKey) -> &[ClusterId] {
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
```

- [x] **Step 4: Run targeted tests**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa/backend
cargo test -p sim-core routing::hpa -- --nocapture
```

Expected: HPA index tests pass.

- [x] **Step 5: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa
git add backend/crates/sim-core/src/routing/hpa.rs backend/crates/sim-core/src/routing/mod.rs
git commit -m "feat(8d): build hierarchical routing index"
```

---

## Task 3: Profile-Aware Cluster Adjacency Tests

**Files:**
- Modify: `backend/crates/sim-core/src/routing/hpa.rs`

- [x] **Step 1: Add failing adjacency tests**

Append these tests inside `routing/hpa.rs`'s test module:

```rust
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
        assert!(index.adjacent_clusters(c0, RoutingProfileKey::Car).is_empty());
        assert_eq!(index.adjacent_clusters(c2, RoutingProfileKey::Car), &[c3]);
        assert_eq!(index.adjacent_clusters(c3, RoutingProfileKey::Tram), &[c4]);
        assert_eq!(
            index.adjacent_clusters(c3, RoutingProfileKey::WalkTransit),
            &[c4]
        );
    }

    #[test]
    fn walk_transit_tram_adjacency_requires_stop_endpoint() {
        let graph = Graph::new(
            vec![
                node(0, 0.0, 0.0, NodeKind::Intersection),
                node(1, 12.0, 0.0, NodeKind::Intersection),
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
        assert_eq!(
            index.adjacent_clusters(c0, RoutingProfileKey::Tram),
            &[ClusterId(1)]
        );
        assert!(
            index
                .adjacent_clusters(c0, RoutingProfileKey::WalkTransit)
                .is_empty()
        );
    }
```

- [x] **Step 2: Run tests to verify behavior**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa/backend
cargo test -p sim-core routing::hpa -- --nocapture
```

Expected: tests pass if Task 2 implemented adjacency correctly. If either test fails, fix only `profiles_for_cross_cluster_edge` or deterministic adjacency ordering.

- [x] **Step 3: Ensure deterministic adjacency order**

If the tests expose unstable ordering, ensure `adjacency_sets` remains a `BTreeSet<ClusterId>` until final collection:

```rust
let adjacency = adjacency_sets
    .into_iter()
    .map(|(key, values)| (key, values.into_iter().collect()))
    .collect();
```

- [x] **Step 4: Run all HPA tests**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa/backend
cargo test -p sim-core routing::hpa -- --nocapture
```

Expected: all HPA tests pass.

- [x] **Step 5: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa
git add backend/crates/sim-core/src/routing/hpa.rs
git commit -m "test(8d): cover profile-aware cluster adjacency"
```

---

## Task 4: HPA Router and Corridor Constraint

**Files:**
- Modify: `backend/crates/sim-core/src/routing/hpa.rs`

- [x] **Step 1: Add failing router tests**

Append these tests inside `routing/hpa.rs`'s test module:

```rust
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

        assert!(matches!(error, HierarchicalRoutingError::NoClusterPath { .. }));
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

        assert!(matches!(error, HierarchicalRoutingError::NoCorridorPath { .. }));
    }
```

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa/backend
cargo test -p sim-core routing::hpa::tests::same_cluster_route_uses_base_case -- --nocapture
```

Expected: compile fails because `HpaRouter` and route helpers are not implemented.

- [x] **Step 3: Implement router structs and abstract cluster search**

First extend the imports at the top of `routing/hpa.rs`:

```rust
use std::cmp::Ordering;
use std::collections::{BTreeSet, BinaryHeap, HashMap, HashSet};

use crate::routing::{
    AStarRouter, Edge, EdgeConstraint, EdgeKind, Graph, NodeId, NodeKind, PathRequest,
    PlannedPath, RoutingError, RoutingProfile, RoutingProfileKey,
};
```

Add this implementation above the test module in `routing/hpa.rs`:

```rust
pub struct HpaRouter;

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
```

- [x] **Step 4: Implement corridor expansion and constrained exact search**

Add these helpers below the abstract search helpers:

```rust
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
```

Add the test-only helper inside `impl HpaIndex`:

```rust
    #[cfg(test)]
    fn force_cluster_adjacency_for_test(
        &mut self,
        from: ClusterId,
        to: ClusterId,
        profile: RoutingProfileKey,
    ) {
        self.adjacency.entry((from, profile)).or_default().push(to);
    }
```

- [x] **Step 5: Run HPA tests**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa/backend
cargo test -p sim-core routing::hpa -- --nocapture
```

Expected: all HPA tests pass.

- [x] **Step 6: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa
git add backend/crates/sim-core/src/routing/hpa.rs
git commit -m "feat(8d): route through hierarchical corridors"
```

---

## Task 5: Plugin and Runtime Integration

**Files:**
- Modify: `backend/crates/sim-core/src/routing/plugin.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [x] **Step 1: Add plugin tests first**

In `backend/crates/sim-core/src/routing/plugin.rs`, extend imports:

```rust
use crate::routing::hpa::{HpaConfig, HpaIndex};
```

Append tests inside the existing test module:

```rust
    #[test]
    fn hierarchical_routing_plugin_installs_hpa_index() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        RoutingPlugin::default().install(&mut world, &mut schedule);
        PathfindingPlugin::default().install(&mut world, &mut schedule);
        HierarchicalRoutingPlugin::default().install(&mut world, &mut schedule);

        assert!(world.contains_resource::<crate::routing::HpaIndex>());
        assert_eq!(world.resource::<crate::routing::HpaIndex>().cluster_count(), 0);
    }

    #[test]
    fn hierarchical_routing_plugin_uses_custom_config() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        RoutingPlugin::default().install(&mut world, &mut schedule);
        HierarchicalRoutingPlugin {
            config: HpaConfig {
                cluster_size_tiles: 16,
                corridor_margin_clusters: 1,
            },
        }
        .install(&mut world, &mut schedule);

        assert_eq!(
            world.resource::<crate::routing::HpaIndex>().config,
            HpaConfig {
                cluster_size_tiles: 16,
                corridor_margin_clusters: 1,
            }
        );
    }
```

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa/backend
cargo test -p sim-core routing::plugin::tests::hierarchical_routing_plugin_installs_hpa_index -- --nocapture
```

Expected: compile fails because `HierarchicalRoutingPlugin` is not implemented or not re-exported.

- [x] **Step 3: Implement plugin and exports**

In `backend/crates/sim-core/src/routing/plugin.rs`, add:

```rust
pub struct HierarchicalRoutingPlugin {
    pub config: HpaConfig,
}

impl Default for HierarchicalRoutingPlugin {
    fn default() -> Self {
        Self {
            config: HpaConfig::default(),
        }
    }
}

impl SimPlugin for HierarchicalRoutingPlugin {
    fn name(&self) -> &'static str {
        "hierarchical_routing"
    }

    fn install(&self, world: &mut World, _schedule: &mut Schedule) {
        let index = {
            let graph = world.resource::<Graph>();
            HpaIndex::build(graph, self.config)
                .expect("hierarchical routing index must build from routing graph")
        };
        world.insert_resource(index);
    }
}
```

In `backend/crates/sim-core/src/routing/mod.rs`, update the plugin re-export:

```rust
pub use plugin::{HierarchicalRoutingPlugin, PathfindingPlugin, RoutingPlugin};
```

- [x] **Step 4: Add runtime installation**

In both runtime construction paths in `backend/crates/sim-server/src/runtime.rs`, install hierarchical routing immediately after `PathfindingPlugin`:

```rust
        sim_core::routing::PathfindingPlugin::default().install(&mut world, &mut schedule);
        sim_core::routing::HierarchicalRoutingPlugin::default().install(&mut world, &mut schedule);

        MobilityPlugin.install(&mut world, &mut schedule);
```

- [x] **Step 5: Add runtime tests**

Append these tests near the existing routing runtime tests in `backend/crates/sim-server/src/runtime.rs`:

```rust
    #[test]
    fn runtime_installs_hpa_index_for_seeded_graph() {
        let network_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../data/city/zurich-network.json");
        let network = sim_core::city_network::CityNetwork::load_from_path(&network_path)
            .expect("zurich fixture network must load");
        let runtime = SimulationRuntime::new_from_network(&network);
        let graph = runtime.world.resource::<sim_core::routing::Graph>();
        let hpa = runtime.world.resource::<sim_core::routing::HpaIndex>();

        assert!(hpa.cluster_count() > 0);
        assert!(hpa.portal_count() > 0);
        assert!(hpa.cluster_count() <= graph.node_count());
    }

    #[test]
    fn runtime_can_find_seeded_hierarchical_path() {
        let network_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../data/city/zurich-network.json");
        let network = sim_core::city_network::CityNetwork::load_from_path(&network_path)
            .expect("zurich fixture network must load");
        let runtime = SimulationRuntime::new_from_network(&network);
        let graph = runtime.world.resource::<sim_core::routing::Graph>();
        let hpa = runtime.world.resource::<sim_core::routing::HpaIndex>();
        let transit_lines = runtime.world.resource::<sim_core::routing::TransitLines>();
        let line = transit_lines
            .iter()
            .find(|line| !line.edges.is_empty())
            .expect("seeded runtime should contain a non-empty transit line");
        let tram_edge = graph.edge(*line.edges.first().expect("line has first edge"));

        let (path, stats) = sim_core::routing::HpaRouter::find_path(
            graph,
            hpa,
            sim_core::routing::PathRequest {
                from: tram_edge.from,
                to: tram_edge.to,
                profile: sim_core::routing::RoutingProfileKey::Tram,
            },
            sim_core::routing::RoutingProfile::for_key(sim_core::routing::RoutingProfileKey::Tram),
        )
        .expect("seeded tram edge endpoints should route through HPA");

        assert!(!path.edges.is_empty());
        assert!(stats.corridor_cluster_count >= 1);
        assert!(path
            .edges
            .iter()
            .all(|edge| graph.edge(edge.edge_id).kind == sim_core::routing::EdgeKind::TramTrack));
    }
```

- [x] **Step 6: Run targeted plugin and runtime tests**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa/backend
cargo test -p sim-core routing::plugin -- --nocapture
cargo test -p sim-server runtime_ -- --nocapture
```

Expected: plugin tests and runtime HPA tests pass.

- [x] **Step 7: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa
git add backend/crates/sim-core/src/routing/plugin.rs backend/crates/sim-core/src/routing/mod.rs backend/crates/sim-server/src/runtime.rs
git commit -m "feat(8d): install hierarchical routing plugin"
```

---

## Task 6: Full Verification and Progress Record

**Files:**
- Modify: `progress.md`

- [x] **Step 1: Run targeted backend tests**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa/backend
cargo test -p sim-core routing::pathfinding -- --nocapture
cargo test -p sim-core routing::hpa -- --nocapture
cargo test -p sim-core routing::plugin -- --nocapture
cargo test -p sim-server runtime_ -- --nocapture
```

Expected: all targeted tests pass.

- [x] **Step 2: Run full backend verification**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa/backend
cargo test --workspace -- --nocapture
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: workspace tests pass; clippy has zero warnings.

- [x] **Step 3: Run frontend verification**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa
./node_modules/.bin/tsc --noEmit --pretty false
./node_modules/.bin/vitest run --passWithNoTests --reporter=dot --pool=forks --fileParallelism=false
```

Expected: TypeScript and Vitest pass unchanged.

- [x] **Step 4: Run smoke and perf gates**

Start the dev stack if it is not already running for this worktree:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa
PATH="/Users/ramonfuglister/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" \
RUSTC="/Users/ramonfuglister/.rustup/toolchains/stable-aarch64-apple-darwin/bin/rustc" \
CARGO_NET_OFFLINE=true \
CARGO_TARGET_DIR=/tmp/abutown-target-8d \
npm run dev:stack
```

In another shell, run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa
node scripts/smoke-7b.mjs
cd backend && cargo bench -p sim-core tick_100k_all_active
```

Expected: smoke remains 9/9 green with binary frames. `tick_100k_all_active` median stays at or below 11.599 ms, the Phase 8c +5% budget.

- [x] **Step 5: Add progress entry**

Run:

```bash
date -u +"%Y-%m-%dT%H:%M:%S.000Z"
```

Add a new top entry to `progress.md`. Start the line with the exact timestamp printed by that command, then append this text:

```markdown
 - Phase 8d verification pass: corridor-HPA* routing is implemented on top of the Phase 8c graph and A* layer. `HpaIndex` builds deterministic fixed-size clusters, detects real cross-cluster portals, records profile-aware abstract adjacency, and `HpaRouter` routes through explicit cluster corridors before delegating final legality to constrained 8c A*. `HierarchicalRoutingPlugin` is installed after `PathfindingPlugin`; runtime seeded Zurich tests verify the HPA resource and a seeded tram route. Targeted routing tests pass, sim-server runtime tests pass, workspace cargo tests pass, clippy `-D warnings` is clean, tsc is clean, Vitest is clean, browser smoke `scripts/smoke-7b.mjs` is 9/9 green with binary frames, and `tick_100k_all_active` stays within the Phase 8c +5% budget. Wire protocol, frontend state, and mobility JSONB snapshot schema remain unchanged.
```

- [x] **Step 6: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa
git add progress.md
git commit -m "docs(8d): record hierarchical routing verification"
```

---

## Task 7: Final Acceptance Sweep

**Files:**
- No code changes unless a verification command exposes a real defect.

- [x] **Step 1: Confirm no forbidden surfaces changed**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa
git diff --stat main...HEAD
git diff --name-only main...HEAD
git diff -- backend/crates/protocol/proto/abutown.proto
git diff -- backend/crates/sim-core/src/mobility/records.rs
git diff -- backend/crates/sim-core/src/mobility/persist_snapshot.rs
git diff -- src
```

Expected: proto, mobility persistence/state, and frontend diffs are empty.

- [x] **Step 2: Confirm no forbidden routing language landed in implementation**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa
rg -n "fallback|synthetic|unwrap_or\\(\\(0\\.0, 0\\.0\\)\\)|global A\\* fallback|fake edge|fake node" \
  backend/crates/sim-core/src/routing backend/crates/sim-server/src/runtime.rs
```

Expected: no matches in implementation. If this matches only spec or plan prose, do not change code. If it matches routing implementation comments, rewrite the comments to say "explicit error" or remove the comment.

- [x] **Step 3: Confirm branch is clean**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa
git status -sb
```

Expected: clean working tree on `codex/phase-8d-routing-hpa`.

- [x] **Step 4: Commit any doc-only acceptance cleanup**

If Task 7 required doc-only cleanup, commit it:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/.worktrees/phase-8d-routing-hpa
git add docs/superpowers/plans/2026-05-26-routing-hpa.md docs/superpowers/specs/2026-05-26-routing-hpa-design.md
git commit -m "docs(8d): finalize hierarchical routing acceptance plan"
```

Skip this commit if Task 7 made no file changes.

---

## Self-Review

- Spec coverage: Tasks 1-4 implement constrained A*, HPA index, portal detection, profile-aware adjacency, abstract cluster search, and corridor exact search. Task 5 installs the plugin and runtime integration. Task 6 covers full verification and progress recording. Task 7 covers forbidden surfaces and zero silent replanning.
- Placeholder scan: The plan contains no TBD markers and every task has concrete files, commands, expected results, and commit boundaries.
- Type consistency: `HpaConfig`, `ClusterCoord`, `ClusterId`, `HpaIndex`, `HpaRouter`, `HpaRouteStats`, and `HierarchicalRoutingError` match the 8d spec names.
- Scope check: The plan is one implementation slice. It does not modify proto, frontend, mobility persistence, or live agent execution.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-26-routing-hpa.md`. Two execution options:

1. Subagent-Driven (recommended) - dispatch a fresh subagent per task, review between tasks, fast iteration.
2. Inline Execution - execute tasks in this session using executing-plans, batch execution with checkpoints.

Recommended choice: Subagent-Driven.
