# Routing A* Multi-Modal Cache Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add deterministic A* routing, mode-aware route profiles, coordinate-to-node requests, and a bounded path cache on top of the Phase 8b routing graph.

**Architecture:** `sim_core::routing` gains three focused modules: `profile.rs` for mode legality/cost, `pathfinding.rs` for A* and request/result/error types, and `path_cache.rs` for cached successful paths. `routing/plugin.rs` gains `PathfindingPlugin`, which installs only `PathCache`; runtime construction installs it between `RoutingPlugin` and `MobilityPlugin`. Mobility state, proto schema, and JSONB snapshot shape stay unchanged.

**Tech Stack:** Rust 2024, `bevy_ecs 0.18`, existing dense `routing::Graph`, `NodeSpatialIndex`, `BinaryHeap`, `HashMap`, `VecDeque`, `Arc`.

---

## File Structure

### Create

- `backend/crates/sim-core/src/routing/profile.rs` - `RoutingProfileKey`, `RoutingProfile`, and `ModeState`.
- `backend/crates/sim-core/src/routing/pathfinding.rs` - A* request/result/error types and `AStarRouter`.
- `backend/crates/sim-core/src/routing/path_cache.rs` - `PathCache`, key, stats, bounded insertion-order eviction.

### Modify

- `backend/crates/sim-core/src/routing/mod.rs` - add modules and re-exports.
- `backend/crates/sim-core/src/routing/plugin.rs` - add `PathfindingPlugin` and plugin tests.
- `backend/crates/sim-server/src/runtime.rs` - install `PathfindingPlugin` after `RoutingPlugin` in both runtime construction paths; add runtime resource/path tests.

### Do Not Modify

- `backend/crates/protocol/proto/abutown.proto`
- `backend/crates/sim-core/src/mobility/records.rs`
- `backend/crates/sim-core/src/mobility/persist_snapshot.rs`
- Frontend source files

---

## Task 1: Routing Profiles

**Files:**
- Create: `backend/crates/sim-core/src/routing/profile.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs`

- [ ] **Step 1: Write failing profile tests**

Create `backend/crates/sim-core/src/routing/profile.rs` with the tests first:

```rust
use crate::routing::{Edge, EdgeKind, NodeKind};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{EdgeId, NodeId};

    fn edge(kind: EdgeKind, length: f32, speed_limit: f32) -> Edge {
        Edge {
            id: EdgeId(0),
            from: NodeId(0),
            to: NodeId(1),
            polyline: vec![(0.0, 0.0), (length, 0.0)],
            length,
            kind,
            speed_limit,
            capacity: 1,
            legacy_id: None,
        }
    }

    #[test]
    fn walk_profile_accepts_only_footway() {
        let profile = RoutingProfile::for_key(RoutingProfileKey::Walk);
        assert!(profile.transition(ModeState::Walking, NodeKind::Intersection, &edge(EdgeKind::Footway, 10.0, 1.0)).is_some());
        assert!(profile.transition(ModeState::Walking, NodeKind::Intersection, &edge(EdgeKind::Road, 10.0, 6.0)).is_none());
        assert!(profile.transition(ModeState::Walking, NodeKind::Intersection, &edge(EdgeKind::TramTrack, 10.0, 4.0)).is_none());
    }

    #[test]
    fn car_profile_accepts_only_road() {
        let profile = RoutingProfile::for_key(RoutingProfileKey::Car);
        assert!(profile.transition(ModeState::Driving, NodeKind::Intersection, &edge(EdgeKind::Road, 12.0, 6.0)).is_some());
        assert!(profile.transition(ModeState::Driving, NodeKind::Intersection, &edge(EdgeKind::Footway, 12.0, 1.0)).is_none());
        assert!(profile.transition(ModeState::Driving, NodeKind::Intersection, &edge(EdgeKind::TramTrack, 12.0, 4.0)).is_none());
    }

    #[test]
    fn walk_transit_boards_only_at_stops() {
        let profile = RoutingProfile::for_key(RoutingProfileKey::WalkTransit);
        let tram = edge(EdgeKind::TramTrack, 20.0, 4.0);
        assert!(profile.transition(ModeState::Walking, NodeKind::Intersection, &tram).is_none());
        let (mode, cost) = profile
            .transition(ModeState::Walking, NodeKind::TransitStop, &tram)
            .expect("boarding at stop is legal");
        assert_eq!(mode, ModeState::OnTram);
        assert!(cost > 0.0);
    }

    #[test]
    fn walk_transit_alights_only_at_stops() {
        let profile = RoutingProfile::for_key(RoutingProfileKey::WalkTransit);
        let foot = edge(EdgeKind::Footway, 5.0, 1.0);
        assert!(profile.transition(ModeState::OnTram, NodeKind::Intersection, &foot).is_none());
        let (mode, cost) = profile
            .transition(ModeState::OnTram, NodeKind::TransitStop, &foot)
            .expect("alighting at stop is legal");
        assert_eq!(mode, ModeState::Walking);
        assert!(cost > 5.0);
    }

    #[test]
    fn fastest_speed_is_positive_for_heuristic() {
        for key in [
            RoutingProfileKey::Walk,
            RoutingProfileKey::Car,
            RoutingProfileKey::Tram,
            RoutingProfileKey::WalkTransit,
        ] {
            assert!(RoutingProfile::for_key(key).fastest_speed() > 0.0);
        }
    }
}
```

Also add the module line to `backend/crates/sim-core/src/routing/mod.rs` so the new test file is compiled:

```rust
pub mod profile;
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::profile -- --nocapture
```

Expected: compile fails because `RoutingProfileKey`, `RoutingProfile`, and `ModeState` are not defined.

- [ ] **Step 3: Implement profile types**

Replace `backend/crates/sim-core/src/routing/profile.rs` with:

```rust
use crate::routing::{Edge, EdgeKind, NodeKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ModeState {
    Walking,
    Driving,
    OnTram,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoutingProfileKey {
    Walk,
    Car,
    Tram,
    WalkTransit,
}

#[derive(Debug, Clone, Copy)]
pub struct RoutingProfile {
    pub key: RoutingProfileKey,
    pub walk_speed: f32,
    pub car_speed_factor: f32,
    pub tram_speed_factor: f32,
    pub board_tram_penalty: f32,
    pub alight_tram_penalty: f32,
}

impl RoutingProfile {
    pub fn for_key(key: RoutingProfileKey) -> Self {
        Self {
            key,
            walk_speed: 1.0,
            car_speed_factor: 1.0,
            tram_speed_factor: 1.0,
            board_tram_penalty: 10.0,
            alight_tram_penalty: 5.0,
        }
    }

    pub fn initial_mode(self) -> ModeState {
        match self.key {
            RoutingProfileKey::Walk | RoutingProfileKey::WalkTransit => ModeState::Walking,
            RoutingProfileKey::Car => ModeState::Driving,
            RoutingProfileKey::Tram => ModeState::OnTram,
        }
    }

    pub fn fastest_speed(self) -> f32 {
        match self.key {
            RoutingProfileKey::Walk => self.walk_speed,
            RoutingProfileKey::Car => 6.0 * self.car_speed_factor,
            RoutingProfileKey::Tram => 4.0 * self.tram_speed_factor,
            RoutingProfileKey::WalkTransit => (4.0 * self.tram_speed_factor).max(self.walk_speed),
        }
        .max(0.001)
    }

    pub fn transition(
        self,
        mode: ModeState,
        current_node_kind: NodeKind,
        edge: &Edge,
    ) -> Option<(ModeState, f32)> {
        let base = match self.key {
            RoutingProfileKey::Walk => {
                if mode == ModeState::Walking && edge.kind == EdgeKind::Footway {
                    Some((ModeState::Walking, edge.length / self.walk_speed.max(0.001)))
                } else {
                    None
                }
            }
            RoutingProfileKey::Car => {
                if mode == ModeState::Driving && edge.kind == EdgeKind::Road {
                    Some((
                        ModeState::Driving,
                        edge.length / (edge.speed_limit * self.car_speed_factor).max(0.001),
                    ))
                } else {
                    None
                }
            }
            RoutingProfileKey::Tram => {
                if mode == ModeState::OnTram && edge.kind == EdgeKind::TramTrack {
                    Some((
                        ModeState::OnTram,
                        edge.length / (edge.speed_limit * self.tram_speed_factor).max(0.001),
                    ))
                } else {
                    None
                }
            }
            RoutingProfileKey::WalkTransit => self.walk_transit_transition(
                mode,
                current_node_kind,
                edge,
            ),
        }?;
        base.1.is_finite().then_some(base)
    }

    fn walk_transit_transition(
        self,
        mode: ModeState,
        current_node_kind: NodeKind,
        edge: &Edge,
    ) -> Option<(ModeState, f32)> {
        match (mode, current_node_kind, edge.kind) {
            (ModeState::Walking, _, EdgeKind::Footway) => {
                Some((ModeState::Walking, edge.length / self.walk_speed.max(0.001)))
            }
            (ModeState::Walking, NodeKind::TransitStop, EdgeKind::TramTrack) => Some((
                ModeState::OnTram,
                edge.length / (edge.speed_limit * self.tram_speed_factor).max(0.001)
                    + self.board_tram_penalty,
            )),
            (ModeState::OnTram, _, EdgeKind::TramTrack) => Some((
                ModeState::OnTram,
                edge.length / (edge.speed_limit * self.tram_speed_factor).max(0.001),
            )),
            (ModeState::OnTram, NodeKind::TransitStop, EdgeKind::Footway) => Some((
                ModeState::Walking,
                edge.length / self.walk_speed.max(0.001) + self.alight_tram_penalty,
            )),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{EdgeId, NodeId};

    fn edge(kind: EdgeKind, length: f32, speed_limit: f32) -> Edge {
        Edge {
            id: EdgeId(0),
            from: NodeId(0),
            to: NodeId(1),
            polyline: vec![(0.0, 0.0), (length, 0.0)],
            length,
            kind,
            speed_limit,
            capacity: 1,
            legacy_id: None,
        }
    }

    #[test]
    fn walk_profile_accepts_only_footway() {
        let profile = RoutingProfile::for_key(RoutingProfileKey::Walk);
        assert!(profile.transition(ModeState::Walking, NodeKind::Intersection, &edge(EdgeKind::Footway, 10.0, 1.0)).is_some());
        assert!(profile.transition(ModeState::Walking, NodeKind::Intersection, &edge(EdgeKind::Road, 10.0, 6.0)).is_none());
        assert!(profile.transition(ModeState::Walking, NodeKind::Intersection, &edge(EdgeKind::TramTrack, 10.0, 4.0)).is_none());
    }

    #[test]
    fn car_profile_accepts_only_road() {
        let profile = RoutingProfile::for_key(RoutingProfileKey::Car);
        assert!(profile.transition(ModeState::Driving, NodeKind::Intersection, &edge(EdgeKind::Road, 12.0, 6.0)).is_some());
        assert!(profile.transition(ModeState::Driving, NodeKind::Intersection, &edge(EdgeKind::Footway, 12.0, 1.0)).is_none());
        assert!(profile.transition(ModeState::Driving, NodeKind::Intersection, &edge(EdgeKind::TramTrack, 12.0, 4.0)).is_none());
    }

    #[test]
    fn walk_transit_boards_only_at_stops() {
        let profile = RoutingProfile::for_key(RoutingProfileKey::WalkTransit);
        let tram = edge(EdgeKind::TramTrack, 20.0, 4.0);
        assert!(profile.transition(ModeState::Walking, NodeKind::Intersection, &tram).is_none());
        let (mode, cost) = profile
            .transition(ModeState::Walking, NodeKind::TransitStop, &tram)
            .expect("boarding at stop is legal");
        assert_eq!(mode, ModeState::OnTram);
        assert!(cost > 0.0);
    }

    #[test]
    fn walk_transit_alights_only_at_stops() {
        let profile = RoutingProfile::for_key(RoutingProfileKey::WalkTransit);
        let foot = edge(EdgeKind::Footway, 5.0, 1.0);
        assert!(profile.transition(ModeState::OnTram, NodeKind::Intersection, &foot).is_none());
        let (mode, cost) = profile
            .transition(ModeState::OnTram, NodeKind::TransitStop, &foot)
            .expect("alighting at stop is legal");
        assert_eq!(mode, ModeState::Walking);
        assert!(cost > 5.0);
    }

    #[test]
    fn fastest_speed_is_positive_for_heuristic() {
        for key in [
            RoutingProfileKey::Walk,
            RoutingProfileKey::Car,
            RoutingProfileKey::Tram,
            RoutingProfileKey::WalkTransit,
        ] {
            assert!(RoutingProfile::for_key(key).fastest_speed() > 0.0);
        }
    }
}
```

- [ ] **Step 4: Wire module exports**

Modify `backend/crates/sim-core/src/routing/mod.rs`:

```rust
pub use profile::{ModeState, RoutingProfile, RoutingProfileKey};
```

- [ ] **Step 5: Verify tests pass**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::profile -- --nocapture
```

Expected: all `routing::profile` tests pass.

- [ ] **Step 6: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/routing/profile.rs backend/crates/sim-core/src/routing/mod.rs
git commit -m "feat(8c): add routing mode profiles"
```

---

## Task 2: A* Core for Single-Mode Profiles

**Files:**
- Create: `backend/crates/sim-core/src/routing/pathfinding.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs`

- [ ] **Step 1: Add failing pathfinding tests**

Create `backend/crates/sim-core/src/routing/pathfinding.rs` with tests that build a small graph:

```rust
use crate::routing::{
    Edge, EdgeId, EdgeKind, Graph, ModeState, Node, NodeId, NodeKind, RoutingProfile,
    RoutingProfileKey,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u32, x: f32, y: f32, kind: NodeKind) -> Node {
        Node { id: NodeId(id), position: (x, y), kind, legacy_id: None }
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
            legacy_id: Some(format!("edge:{id}")),
        }
    }

    fn graph_for_modes() -> Graph {
        Graph::new(
            vec![
                node(0, 0.0, 0.0, NodeKind::Intersection),
                node(1, 10.0, 0.0, NodeKind::Intersection),
                node(2, 20.0, 0.0, NodeKind::Intersection),
            ],
            vec![
                edge(0, 0, 1, EdgeKind::Footway, 10.0),
                edge(1, 1, 2, EdgeKind::Footway, 10.0),
                edge(2, 0, 2, EdgeKind::Road, 30.0),
            ],
        )
    }

    #[test]
    fn same_node_returns_empty_path() {
        let graph = graph_for_modes();
        let path = AStarRouter::find_path(
            &graph,
            PathRequest { from: NodeId(1), to: NodeId(1), profile: RoutingProfileKey::Walk },
            RoutingProfile::for_key(RoutingProfileKey::Walk),
        )
        .expect("same-node route should exist");
        assert_eq!(path.edges, Vec::<PathEdge>::new());
        assert_eq!(path.total_cost, 0.0);
    }

    #[test]
    fn walk_profile_uses_footways() {
        let graph = graph_for_modes();
        let path = AStarRouter::find_path(
            &graph,
            PathRequest { from: NodeId(0), to: NodeId(2), profile: RoutingProfileKey::Walk },
            RoutingProfile::for_key(RoutingProfileKey::Walk),
        )
        .expect("walk route should exist");
        assert_eq!(path.edges.iter().map(|e| e.edge_id).collect::<Vec<_>>(), vec![EdgeId(0), EdgeId(1)]);
        assert!(path.edges.iter().all(|e| e.mode == ModeState::Walking));
    }

    #[test]
    fn walk_profile_rejects_road_only_route() {
        let graph = Graph::new(
            vec![
                node(0, 0.0, 0.0, NodeKind::Intersection),
                node(1, 10.0, 0.0, NodeKind::Intersection),
            ],
            vec![edge(0, 0, 1, EdgeKind::Road, 10.0)],
        );
        let err = AStarRouter::find_path(
            &graph,
            PathRequest { from: NodeId(0), to: NodeId(1), profile: RoutingProfileKey::Walk },
            RoutingProfile::for_key(RoutingProfileKey::Walk),
        )
        .expect_err("road-only graph should not satisfy walk route");
        assert_eq!(
            err,
            RoutingError::NoPath {
                from: NodeId(0),
                to: NodeId(1),
                profile: RoutingProfileKey::Walk,
            }
        );
    }

    #[test]
    fn missing_nodes_are_errors() {
        let graph = graph_for_modes();
        assert_eq!(
            AStarRouter::find_path(
                &graph,
                PathRequest { from: NodeId(99), to: NodeId(1), profile: RoutingProfileKey::Walk },
                RoutingProfile::for_key(RoutingProfileKey::Walk),
            ),
            Err(RoutingError::MissingNode(NodeId(99)))
        );
    }
}
```

Also add the module line to `backend/crates/sim-core/src/routing/mod.rs` so the new test file is compiled:

```rust
pub mod pathfinding;
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::pathfinding -- --nocapture
```

Expected: compile fails because pathfinding types are not implemented.

- [ ] **Step 3: Implement public pathfinding types**

At the top of `pathfinding.rs`, above the tests, add:

```rust
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::routing::{
    EdgeId, Graph, ModeState, NodeId, NodeSpatialIndex, RoutingProfile, RoutingProfileKey,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PathRequest {
    pub from: NodeId,
    pub to: NodeId,
    pub profile: RoutingProfileKey,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathEdge {
    pub edge_id: EdgeId,
    pub mode: ModeState,
    pub cost: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedPath {
    pub from: NodeId,
    pub to: NodeId,
    pub profile: RoutingProfileKey,
    pub edges: Vec<PathEdge>,
    pub total_cost: f32,
    pub total_length: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingError {
    MissingNode(NodeId),
    NoNearestNode,
    NoPath {
        from: NodeId,
        to: NodeId,
        profile: RoutingProfileKey,
    },
    InvalidGraph(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SearchState {
    node: NodeId,
    mode: ModeState,
}

#[derive(Debug, Clone, Copy)]
struct QueueEntry {
    state: SearchState,
    known_cost: f32,
    estimated_total: f32,
}

impl PartialEq for QueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.estimated_total == other.estimated_total
            && self.known_cost == other.known_cost
            && self.state.node == other.state.node
            && self.state.mode == other.state.mode
    }
}

impl Eq for QueueEntry {}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .estimated_total
            .partial_cmp(&self.estimated_total)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                other
                    .known_cost
                    .partial_cmp(&self.known_cost)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| other.state.node.0.cmp(&self.state.node.0))
            .then_with(|| other.state.mode.cmp(&self.state.mode))
    }
}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub struct AStarRouter;
```

- [ ] **Step 4: Implement A* search**

Add this implementation below the type definitions:

```rust
impl AStarRouter {
    pub fn find_path(
        graph: &Graph,
        request: PathRequest,
        profile: RoutingProfile,
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
                return reconstruct_path(graph, request, entry.state, &came_from, entry.known_cost);
            }

            if entry.known_cost > *best.get(&entry.state).unwrap_or(&f32::INFINITY) {
                continue;
            }

            let current_node = graph.node(entry.state.node);
            for edge_id in graph.outgoing(entry.state.node) {
                let edge = graph.edge(*edge_id);
                let Some((next_mode, edge_cost)) =
                    profile.transition(entry.state.mode, current_node.kind, edge)
                else {
                    continue;
                };
                if edge_cost < 0.0 || !edge_cost.is_finite() {
                    return Err(RoutingError::InvalidGraph("edge cost must be finite and non-negative"));
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
}

fn validate_node(graph: &Graph, id: NodeId) -> Result<(), RoutingError> {
    if (id.0 as usize) < graph.node_count() {
        Ok(())
    } else {
        Err(RoutingError::MissingNode(id))
    }
}

fn heuristic(graph: &Graph, from: NodeId, to: NodeId, profile: RoutingProfile) -> f32 {
    let a = graph.node(from).position;
    let b = graph.node(to).position;
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    (dx * dx + dy * dy).sqrt() / profile.fastest_speed()
}

fn reconstruct_path(
    graph: &Graph,
    request: PathRequest,
    mut current: SearchState,
    came_from: &HashMap<SearchState, (SearchState, PathEdge)>,
    total_cost: f32,
) -> Result<PlannedPath, RoutingError> {
    let mut edges = Vec::new();
    while current.node != request.from {
        let Some((previous, path_edge)) = came_from.get(&current).copied() else {
            return Err(RoutingError::InvalidGraph("path reconstruction missing predecessor"));
        };
        edges.push(path_edge);
        current = previous;
    }
    edges.reverse();
    let total_length = edges
        .iter()
        .map(|edge| graph.edge(edge.edge_id).length)
        .sum();
    Ok(PlannedPath {
        from: request.from,
        to: request.to,
        profile: request.profile,
        edges,
        total_cost,
        total_length,
    })
}
```

- [ ] **Step 5: Wire module exports**

Modify `backend/crates/sim-core/src/routing/mod.rs`:

```rust
pub use pathfinding::{AStarRouter, PathEdge, PathRequest, PlannedPath, RoutingError};
```

- [ ] **Step 6: Verify tests pass**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::pathfinding -- --nocapture
```

Expected: all `routing::pathfinding` tests pass.

- [ ] **Step 7: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/routing/pathfinding.rs backend/crates/sim-core/src/routing/mod.rs
git commit -m "feat(8c): add deterministic astar routing"
```

---

## Task 3: Coordinate Requests and Walk-Transit Tests

**Files:**
- Modify: `backend/crates/sim-core/src/routing/pathfinding.rs`

- [ ] **Step 1: Add failing tests**

Append these tests to the existing `routing::pathfinding::tests` module:

```rust
fn walk_transit_graph() -> Graph {
    Graph::new(
        vec![
            node(0, 0.0, 0.0, NodeKind::Intersection),
            node(1, 10.0, 0.0, NodeKind::TransitStop),
            node(2, 20.0, 0.0, NodeKind::TransitStop),
            node(3, 30.0, 0.0, NodeKind::Intersection),
        ],
        vec![
            edge(0, 0, 1, EdgeKind::Footway, 10.0),
            edge(1, 1, 2, EdgeKind::TramTrack, 10.0),
            edge(2, 2, 3, EdgeKind::Footway, 10.0),
        ],
    )
}

#[test]
fn walk_transit_combines_walk_tram_walk() {
    let graph = walk_transit_graph();
    let path = AStarRouter::find_path(
        &graph,
        PathRequest { from: NodeId(0), to: NodeId(3), profile: RoutingProfileKey::WalkTransit },
        RoutingProfile::for_key(RoutingProfileKey::WalkTransit),
    )
    .expect("walk-transit route should exist");
    assert_eq!(path.edges.iter().map(|e| e.edge_id).collect::<Vec<_>>(), vec![EdgeId(0), EdgeId(1), EdgeId(2)]);
    assert_eq!(path.edges.iter().map(|e| e.mode).collect::<Vec<_>>(), vec![ModeState::Walking, ModeState::OnTram, ModeState::Walking]);
}

#[test]
fn walk_transit_cannot_board_at_intersection() {
    let graph = Graph::new(
        vec![
            node(0, 0.0, 0.0, NodeKind::Intersection),
            node(1, 10.0, 0.0, NodeKind::Intersection),
        ],
        vec![edge(0, 0, 1, EdgeKind::TramTrack, 10.0)],
    );
    let err = AStarRouter::find_path(
        &graph,
        PathRequest { from: NodeId(0), to: NodeId(1), profile: RoutingProfileKey::WalkTransit },
        RoutingProfile::for_key(RoutingProfileKey::WalkTransit),
    )
    .expect_err("boarding at intersection must be illegal");
    assert_eq!(
        err,
        RoutingError::NoPath {
            from: NodeId(0),
            to: NodeId(1),
            profile: RoutingProfileKey::WalkTransit,
        }
    );
}

#[test]
fn request_between_points_resolves_nearest_nodes() {
    let graph = graph_for_modes();
    let index = crate::routing::NodeSpatialIndex::from_nodes(graph.nodes());
    let request = request_between_points(
        &index,
        (1.0, 0.0),
        (19.0, 0.0),
        RoutingProfileKey::Walk,
    )
    .expect("points should resolve to nearest graph nodes");
    assert_eq!(request.from, NodeId(0));
    assert_eq!(request.to, NodeId(2));
}

#[test]
fn request_between_points_errors_on_empty_index() {
    let index = crate::routing::NodeSpatialIndex::default();
    assert_eq!(
        request_between_points(&index, (1.0, 0.0), (19.0, 0.0), RoutingProfileKey::Walk),
        Err(RoutingError::NoNearestNode)
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::pathfinding -- --nocapture
```

Expected: compile fails because `request_between_points` is not implemented, or walk-transit assertions fail before the Task 2 implementation is corrected.

- [ ] **Step 3: Implement coordinate request helper**

Add this function to `pathfinding.rs`:

```rust
pub fn request_between_points(
    index: &NodeSpatialIndex,
    from: (f32, f32),
    to: (f32, f32),
    profile: RoutingProfileKey,
) -> Result<PathRequest, RoutingError> {
    let Some(from_node) = index.nearest(from) else {
        return Err(RoutingError::NoNearestNode);
    };
    let Some(to_node) = index.nearest(to) else {
        return Err(RoutingError::NoNearestNode);
    };
    Ok(PathRequest {
        from: from_node,
        to: to_node,
        profile,
    })
}
```

Update `routing/mod.rs` export:

```rust
pub use pathfinding::{
    request_between_points, AStarRouter, PathEdge, PathRequest, PlannedPath, RoutingError,
};
```

- [ ] **Step 4: Verify tests pass**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::pathfinding -- --nocapture
```

Expected: all pathfinding tests pass, including walk-transit transition tests.

- [ ] **Step 5: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/routing/pathfinding.rs backend/crates/sim-core/src/routing/mod.rs
git commit -m "feat(8c): support point routing and walk transit"
```

---

## Task 4: Path Cache Resource

**Files:**
- Create: `backend/crates/sim-core/src/routing/path_cache.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs`

- [ ] **Step 1: Write failing cache tests**

Create `backend/crates/sim-core/src/routing/path_cache.rs` with tests:

```rust
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use bevy_ecs::prelude::*;

use crate::routing::{
    AStarRouter, Graph, PathRequest, PlannedPath, RoutingError, RoutingProfile, RoutingProfileKey,
};
use crate::routing::{NodeId};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{Edge, EdgeId, EdgeKind, Node, NodeKind};

    fn graph() -> Graph {
        Graph::new(
            vec![
                Node { id: NodeId(0), position: (0.0, 0.0), kind: NodeKind::Intersection, legacy_id: None },
                Node { id: NodeId(1), position: (10.0, 0.0), kind: NodeKind::Intersection, legacy_id: None },
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
        PathRequest { from: NodeId(0), to: NodeId(1), profile }
    }

    #[test]
    fn cache_tracks_miss_then_hit() {
        let graph = graph();
        let mut cache = PathCache::with_capacity(8);
        let first = cache
            .get_or_plan(&graph, request(RoutingProfileKey::Walk), RoutingProfile::for_key(RoutingProfileKey::Walk))
            .expect("first route should plan");
        let second = cache
            .get_or_plan(&graph, request(RoutingProfileKey::Walk), RoutingProfile::for_key(RoutingProfileKey::Walk))
            .expect("second route should hit cache");
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(cache.stats(), PathCacheStats { hits: 1, misses: 1, inserts: 1, evictions: 0 });
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
        cache.insert(PathCacheKey::new(request(RoutingProfileKey::Walk), 0), Arc::clone(&path));
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
```

Also add the module line to `backend/crates/sim-core/src/routing/mod.rs` so the new test file is compiled:

```rust
pub mod path_cache;
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::path_cache -- --nocapture
```

Expected: compile fails because cache types are not implemented.

- [ ] **Step 3: Implement path cache**

Add these definitions above the tests in `path_cache.rs`:

```rust
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
```

- [ ] **Step 4: Wire module exports**

Modify `backend/crates/sim-core/src/routing/mod.rs`:

```rust
pub use path_cache::{PathCache, PathCacheKey, PathCacheStats};
```

- [ ] **Step 5: Verify tests pass**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::path_cache -- --nocapture
```

Expected: all path cache tests pass.

- [ ] **Step 6: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/routing/path_cache.rs backend/crates/sim-core/src/routing/mod.rs
git commit -m "feat(8c): add bounded routing path cache"
```

---

## Task 5: Pathfinding Plugin and Runtime Install

**Files:**
- Modify: `backend/crates/sim-core/src/routing/plugin.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [ ] **Step 1: Add failing plugin tests**

In `backend/crates/sim-core/src/routing/plugin.rs`, add this test inside the existing `tests` module:

```rust
#[test]
fn pathfinding_plugin_installs_path_cache() {
    let mut world = World::new();
    let mut schedule = Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    RoutingPlugin::default().install(&mut world, &mut schedule);
    PathfindingPlugin::default().install(&mut world, &mut schedule);
    assert!(world.contains_resource::<crate::routing::PathCache>());
    assert_eq!(world.resource::<crate::routing::PathCache>().len(), 0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::plugin::tests::pathfinding_plugin_installs_path_cache -- --nocapture
```

Expected: compile fails because `PathfindingPlugin` is missing.

- [ ] **Step 3: Implement plugin**

In `backend/crates/sim-core/src/routing/plugin.rs`, add:

```rust
use crate::routing::path_cache::PathCache;
```

Then add below `RoutingPlugin`:

```rust
pub struct PathfindingPlugin {
    pub cache_capacity: usize,
}

impl Default for PathfindingPlugin {
    fn default() -> Self {
        Self {
            cache_capacity: 8192,
        }
    }
}

impl SimPlugin for PathfindingPlugin {
    fn name(&self) -> &'static str {
        "pathfinding"
    }

    fn install(&self, world: &mut World, _schedule: &mut Schedule) {
        world.insert_resource(PathCache::with_capacity(self.cache_capacity));
    }
}
```

Modify `backend/crates/sim-core/src/routing/mod.rs`:

```rust
pub use plugin::{PathfindingPlugin, RoutingPlugin};
```

- [ ] **Step 4: Install plugin in runtime**

In `backend/crates/sim-server/src/runtime.rs`, add this immediately after each `sim_core::routing::RoutingPlugin { ... }.install(...)` block:

```rust
        sim_core::routing::PathfindingPlugin::default().install(&mut world, &mut schedule);
```

There are two runtime construction paths in this file. Install the plugin in both.

- [ ] **Step 5: Verify plugin test passes**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::plugin::tests::pathfinding_plugin_installs_path_cache -- --nocapture
```

Expected: one test passes.

- [ ] **Step 6: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/routing/plugin.rs backend/crates/sim-core/src/routing/mod.rs backend/crates/sim-server/src/runtime.rs
git commit -m "feat(8c): install pathfinding cache plugin"
```

---

## Task 6: Runtime Integration Tests

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [ ] **Step 1: Add runtime resource test**

In the `#[cfg(test)]` module of `backend/crates/sim-server/src/runtime.rs`, add:

```rust
#[test]
fn runtime_has_pathfinding_resources() {
    let runtime = SimulationRuntime::new();
    assert!(runtime.world.contains_resource::<sim_core::routing::PathCache>());
}
```

- [ ] **Step 2: Add seeded graph path test**

In the same test module, add:

```rust
#[test]
fn runtime_can_find_seeded_tram_path() {
    let runtime = SimulationRuntime::new();
    let graph = runtime.world.resource::<sim_core::routing::Graph>();
    let transit_lines = runtime.world.resource::<sim_core::routing::TransitLines>();
    let line = transit_lines
        .iter()
        .find(|line| !line.edges.is_empty())
        .expect("seeded runtime should contain a non-empty transit line");
    let first_edge = graph.edge(*line.edges.first().expect("line has first edge"));
    let last_edge = graph.edge(*line.edges.last().expect("line has last edge"));
    let path = sim_core::routing::AStarRouter::find_path(
        graph,
        sim_core::routing::PathRequest {
            from: first_edge.from,
            to: last_edge.to,
            profile: sim_core::routing::RoutingProfileKey::Tram,
        },
        sim_core::routing::RoutingProfile::for_key(
            sim_core::routing::RoutingProfileKey::Tram,
        ),
    )
    .expect("seeded transit line endpoints should be connected by tram edges");
    assert!(!path.edges.is_empty());
    assert!(path
        .edges
        .iter()
        .all(|edge| graph.edge(edge.edge_id).kind == sim_core::routing::EdgeKind::TramTrack));
}
```

- [ ] **Step 3: Run runtime tests**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-server runtime_ -- --nocapture
```

Expected: both runtime tests pass.

- [ ] **Step 4: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-server/src/runtime.rs
git commit -m "test(8c): verify runtime pathfinding resources"
```

---

## Task 7: Full Verification and Progress Record

**Files:**
- Modify: `progress.md`

- [ ] **Step 1: Run targeted backend tests**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing:: -- --nocapture
cargo test -p sim-server runtime_ -- --nocapture
```

Expected: all targeted routing/pathfinding tests pass.

- [ ] **Step 2: Run full backend verification**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test --workspace -- --nocapture
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: workspace tests pass; clippy has zero warnings.

- [ ] **Step 3: Run frontend verification**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
./node_modules/.bin/tsc --noEmit --pretty false
./node_modules/.bin/vitest run --passWithNoTests --reporter=dot --pool=forks --fileParallelism=false
```

Expected: TypeScript and Vitest pass unchanged.

- [ ] **Step 4: Run smoke and perf gates**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
node scripts/smoke-7b.mjs
cd backend && cargo bench -p sim-core tick_100k_all_active
```

Expected: smoke remains 9/9 green with binary frames. `tick_100k_all_active` median stays at or below 12.362 ms, the Phase 8b +5% budget.

- [ ] **Step 5: Add progress entry**

Run:

```bash
date -u +"%Y-%m-%dT%H:%M:%S.000Z"
```

Add a new top entry to `progress.md`. Start the line with the exact timestamp printed by that command, then append this text:

```markdown
 - Phase 8c Task 7 verification pass: A* routing, mode-aware profiles, point-to-node requests, PathCache, and PathfindingPlugin are implemented on top of the Phase 8b graph. Targeted sim-core routing tests pass, sim-server runtime pathfinding resource tests pass, workspace cargo tests pass, clippy `-D warnings` is clean, tsc is clean, Vitest is clean, browser smoke `scripts/smoke-7b.mjs` is 9/9 green with binary frames, and `tick_100k_all_active` stays within the Phase 8b +5% budget. Wire protocol and `mobility_snapshots` JSONB schema remain unchanged.
```

- [ ] **Step 6: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add progress.md
git commit -m "docs(8c): record pathfinding verification"
```

---

## Task 8: Final Acceptance Sweep

**Files:**
- No code changes unless a verification command exposes a real defect.

- [ ] **Step 1: Confirm no forbidden surface changed**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git diff --stat main...HEAD
git diff --name-only main...HEAD
git diff -- backend/crates/protocol/proto/abutown.proto
git diff -- backend/crates/sim-core/src/mobility/records.rs
git diff -- backend/crates/sim-core/src/mobility/persist_snapshot.rs
```

Expected: proto and mobility persistence/state diffs are empty.

- [ ] **Step 2: Confirm no fallback language landed in routing implementation**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
rg -n "fallback|unwrap_or\\(\\(0\\.0, 0\\.0\\)\\)|NoNearestNode.*unwrap|synthetic" backend/crates/sim-core/src/routing backend/crates/sim-server/src/runtime.rs
```

Expected: no matches that implement fake paths, fake coordinates, or synthetic link ids. Matches in comments explaining rejected fallback behavior must be removed or rewritten.

- [ ] **Step 3: Confirm branch is clean**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git status -sb
```

Expected: clean working tree on the 8c branch.

- [ ] **Step 4: Commit any acceptance-doc cleanup**

If Task 8 required doc-only cleanup, commit it:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add docs/superpowers/plans/2026-05-24-routing-a-star-multimodal-cache.md docs/superpowers/specs/2026-05-24-routing-a-star-multimodal-cache-design.md
git commit -m "docs(8c): finalize pathfinding acceptance plan"
```

Skip this commit if Task 8 made no file changes.

---

## Self-Review

- Spec coverage: Tasks 1-4 implement profiles, A*, point requests, and cache. Task 5 installs the plugin. Task 6 covers runtime integration. Task 7 covers verification and perf. Task 8 covers forbidden surface checks and fallback removal.
- Placeholder scan: No marker words or unspecified code tasks remain.
- Type consistency: `RoutingProfileKey`, `ModeState`, `PathRequest`, `PlannedPath`, `PathCacheKey`, and `PathfindingPlugin` names match the 8c spec.
- Scope check: No task changes proto, JSONB shape, or mobility state variants.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-24-routing-a-star-multimodal-cache.md`. Two execution options:

1. Subagent-Driven (recommended) - dispatch a fresh subagent per task, review between tasks, fast iteration.
2. Inline Execution - execute tasks in this session using executing-plans, batch execution with checkpoints.

Recommended choice: Subagent-Driven.
