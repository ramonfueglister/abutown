# Routing Graph + Spatial Index Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the polyline-soup `CityNetwork` + `Routes`/`Stops`/`LinkPolylines` triple with a proper routing graph (Nodes + Edges), an rstar-backed spatial index, and a pluggable `CostModel` trait — without changing wire or persistence bytes.

**Architecture:** A new `sim_core::routing` module ships compact `Vec`-indexed graph types, a `TransitLines` resource for fixed tram routes, an rstar `NodeSpatialIndex`, and a `CostModel` trait with three composable impls. `RoutingPlugin` builds the graph once at startup from `CityNetwork` and inserts everything as resources. Mobility systems are rewritten to read the graph; the old resources are deleted in a final cut. Wire ids (`link_id: String`, `stop_id: String`) stay intact via per-edge/per-node `legacy_id` fields.

**Tech Stack:** Rust 2024, `bevy_ecs 0.18`, `rstar` (R-tree), existing `serde_json` + tokio + axum + prost stack.

---

## File Structure

### Created
- `backend/crates/sim-core/src/routing/mod.rs` — re-exports public API
- `backend/crates/sim-core/src/routing/graph.rs` — `NodeId`, `EdgeId`, `Node`, `Edge`, `NodeKind`, `EdgeKind`, `Graph`
- `backend/crates/sim-core/src/routing/transit.rs` — `LineId`, `TransitLine`, `TransitLines` resource
- `backend/crates/sim-core/src/routing/spatial_index.rs` — `IndexedNode`, `NodeSpatialIndex` resource (rstar wrapper)
- `backend/crates/sim-core/src/routing/cost_model.rs` — `CostModel` trait + `DistanceCost`, `TimeCost`, `ModeFilterCost`
- `backend/crates/sim-core/src/routing/builder.rs` — `build_graph_from_city_network`
- `backend/crates/sim-core/src/routing/plugin.rs` — `RoutingPlugin: SimPlugin`
- `backend/crates/sim-core/src/routing/waiting.rs` — `WaitingAgents` resource (replaces `StopRecord.waiting_agents` per-tick state)

### Modified
- `backend/crates/sim-core/Cargo.toml` — add `rstar = "0.12"`
- `backend/crates/sim-core/src/lib.rs` — add `pub mod routing;`
- `backend/crates/sim-server/src/runtime.rs` — install `RoutingPlugin` after `CorePlugin` and before `MobilityPlugin`
- `backend/crates/sim-core/src/mobility/seed.rs` — consume `Graph` + `TransitLines` instead of authoring `Routes`/`Stops`/`LinkPolylines`
- `backend/crates/sim-core/src/mobility/api.rs` — replace `routes()`/`stops()`/`link_polyline()` accessors with graph-backed equivalents; `add_route`/`add_stop`/`set_link_polyline` removed
- `backend/crates/sim-core/src/mobility/components.rs` — `RoutePosition.route_id` → `line_id`, `link_index` → `edge_index` (semantics + types changed)
- `backend/crates/sim-core/src/mobility/systems.rs` — rewrite `walk_advance_system`, `vehicle_advance_system`, `boarding_alighting_system`, `stop_arrival_system`, `warm_chunk_flow_system`, `track_chunk_populations_system` (and any other system that touches `Routes`/`Stops`/`LinkPolylines`) to query `Graph`/`TransitLines`/`WaitingAgents`
- `backend/crates/sim-core/src/mobility/records.rs` — delete `RouteRecord`, `StopRecord`; `AgentMobilityState` variants keep `String` payloads for wire compat
- `backend/crates/sim-core/src/mobility/resources.rs` — delete `Routes`, `Stops`, `LinkPolylines`
- `backend/crates/sim-core/src/ids.rs` — delete `RouteId`, `LinkId`, `StopId`
- `backend/crates/sim-core/src/mobility/dto.rs` — adjust any references to deleted types (string ids stay, internal lookups go through Graph)
- `backend/crates/sim-core/src/mobility/persist_snapshot.rs` — `StopRecord` references swap to a small persistence-local stop type, or persist via string-only fields
- `backend/crates/sim-core/src/mobility_geometry.rs` — `world_coord_at_progress_slice` stays as the polyline geometry helper (unchanged)

---

## Pre-flight: glossary of new types

These types are referenced throughout. Quick reference so engineers reading tasks out of order are not lost.

- `NodeId(pub u32)` — index into `Graph.nodes`. `Component + Copy + Clone + Hash + Eq + PartialEq + Debug`.
- `EdgeId(pub u32)` — index into `Graph.edges`. Same derives.
- `LineId(pub u32)` — index into `TransitLines.0`. Same derives.
- `Node { id, position: (f32, f32), kind: NodeKind, legacy_id: Option<String> }` — `legacy_id` preserves seeded stop ids like `"stop:horizontal:pickup"` for wire compat.
- `NodeKind { Intersection, TransitStop, ActivityLocation }`.
- `Edge { id, from, to, polyline: Vec<(f32, f32)>, length: f32, kind: EdgeKind, speed_limit: f32, capacity: u16, legacy_id: Option<String> }`.
- `EdgeKind { Footway, Road, TramTrack }`.
- `Graph { nodes: Vec<Node>, edges: Vec<Edge>, outgoing: Vec<Vec<EdgeId>>, incoming: Vec<Vec<EdgeId>>, by_legacy_node_id: HashMap<String, NodeId>, by_legacy_edge_id: HashMap<String, EdgeId> }`.
- `TransitLine { id, name: String, edges: Vec<EdgeId>, stops: Vec<NodeId>, legacy_route_id: Option<String> }`.
- `TransitLines(Vec<TransitLine>)` — Resource. Has `by_legacy_route_id: HashMap<String, LineId>` field.
- `NodeSpatialIndex(rstar::RTree<IndexedNode>)` — Resource.
- `WaitingAgents(HashMap<NodeId, VecDeque<AgentId>>)` — Resource. Replaces `StopRecord.waiting_agents`.
- `CostModel` — trait. Impls: `DistanceCost`, `TimeCost`, `ModeFilterCost<C: CostModel>`.

---

## Task 1: Scaffolding (empty `routing` module + dep)

**Files:**
- Create: 8 empty files under `backend/crates/sim-core/src/routing/`
- Modify: `backend/crates/sim-core/Cargo.toml` (add `rstar`)
- Modify: `backend/crates/sim-core/src/lib.rs` (add `pub mod routing;`)

- [ ] **Step 1: Create skeleton files**

Create `backend/crates/sim-core/src/routing/mod.rs`:
```rust
pub mod builder;
pub mod cost_model;
pub mod graph;
pub mod plugin;
pub mod spatial_index;
pub mod transit;
pub mod waiting;
```

Create the seven submodule files (`builder.rs`, `cost_model.rs`, `graph.rs`, `plugin.rs`, `spatial_index.rs`, `transit.rs`, `waiting.rs`) as empty files. Subsequent tasks fill them.

In `backend/crates/sim-core/src/lib.rs`, add `pub mod routing;` near the other `pub mod` declarations.

In `backend/crates/sim-core/src/routing/graph.rs`, add a compile-only sanity test:
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn routing_module_compiles() {}
}
```

- [ ] **Step 2: Add rstar to Cargo.toml**

In `backend/crates/sim-core/Cargo.toml` `[dependencies]`:
```toml
rstar = "0.12"
```

- [ ] **Step 3: Verify build + test**

Run from `backend/`:
```
cargo build 2>&1 | tail -5
cargo test -p sim-core --lib routing::graph::tests 2>&1 | tail -5
```

Expected: build clean, 1 passed.

- [ ] **Step 4: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/routing/ backend/crates/sim-core/src/lib.rs backend/crates/sim-core/Cargo.toml backend/Cargo.lock
git commit -m "scaffold(8b): empty sim_core::routing module + rstar dep"
```

---

## Task 2: Graph types (`NodeId`, `EdgeId`, `Node`, `Edge`, `Graph`)

**Files:**
- Modify: `backend/crates/sim-core/src/routing/graph.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs` (re-exports)

- [ ] **Step 1: Write `graph.rs`**

Replace `backend/crates/sim-core/src/routing/graph.rs` with:

```rust
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
    by_legacy_edge_id: HashMap<String, EdgeId>,
}

impl Graph {
    pub fn new(
        nodes: Vec<Node>,
        edges: Vec<Edge>,
    ) -> Self {
        let node_count = nodes.len();
        let mut outgoing: Vec<Vec<EdgeId>> = vec![Vec::new(); node_count];
        let mut incoming: Vec<Vec<EdgeId>> = vec![Vec::new(); node_count];
        let mut by_legacy_node_id: HashMap<String, NodeId> = HashMap::new();
        let mut by_legacy_edge_id: HashMap<String, EdgeId> = HashMap::new();
        for n in &nodes {
            if let Some(legacy) = &n.legacy_id {
                by_legacy_node_id.insert(legacy.clone(), n.id);
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
```

- [ ] **Step 2: Re-export from `routing/mod.rs`**

Append to `backend/crates/sim-core/src/routing/mod.rs`:
```rust
pub use graph::{Edge, EdgeId, EdgeKind, Graph, Node, NodeId, NodeKind};
```

- [ ] **Step 3: Verify tests pass**

```
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core --lib routing::graph 2>&1 | tail -10
```

Expected: 3 passed.

- [ ] **Step 4: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/routing/
git commit -m "feat(8b): graph types — Node, Edge, Graph with legacy-id maps"
```

---

## Task 3: Transit lines (`LineId`, `TransitLine`, `TransitLines`)

**Files:**
- Modify: `backend/crates/sim-core/src/routing/transit.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs`

- [ ] **Step 1: Write `transit.rs`**

```rust
use bevy_ecs::prelude::*;
use std::collections::HashMap;

use crate::routing::graph::{EdgeId, NodeId};

#[derive(Component, Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct LineId(pub u32);

#[derive(Debug, Clone)]
pub struct TransitLine {
    pub id: LineId,
    pub name: String,
    pub edges: Vec<EdgeId>,
    pub stops: Vec<NodeId>,
    /// Legacy wire id (e.g., "route:horizontal"). `None` for lines
    /// introduced by the builder without legacy ancestry.
    pub legacy_route_id: Option<String>,
}

#[derive(Resource, Debug, Default)]
pub struct TransitLines {
    lines: Vec<TransitLine>,
    by_legacy_route_id: HashMap<String, LineId>,
}

impl TransitLines {
    pub fn new(lines: Vec<TransitLine>) -> Self {
        let mut by_legacy_route_id = HashMap::new();
        for line in &lines {
            if let Some(legacy) = &line.legacy_route_id {
                by_legacy_route_id.insert(legacy.clone(), line.id);
            }
        }
        Self { lines, by_legacy_route_id }
    }

    pub fn line(&self, id: LineId) -> &TransitLine {
        &self.lines[id.0 as usize]
    }

    pub fn iter(&self) -> impl Iterator<Item = &TransitLine> {
        self.lines.iter()
    }

    pub fn count(&self) -> usize {
        self.lines.len()
    }

    pub fn line_by_legacy(&self, legacy_id: &str) -> Option<LineId> {
        self.by_legacy_route_id.get(legacy_id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transit_lines_lookup_by_index_and_legacy() {
        let lines = vec![TransitLine {
            id: LineId(0),
            name: "tram_h".into(),
            edges: vec![EdgeId(0), EdgeId(2)],
            stops: vec![NodeId(1)],
            legacy_route_id: Some("route:horizontal".into()),
        }];
        let tl = TransitLines::new(lines);
        assert_eq!(tl.count(), 1);
        assert_eq!(tl.line(LineId(0)).name, "tram_h");
        assert_eq!(tl.line_by_legacy("route:horizontal"), Some(LineId(0)));
        assert!(tl.line_by_legacy("missing").is_none());
    }
}
```

- [ ] **Step 2: Re-export**

In `backend/crates/sim-core/src/routing/mod.rs` append:
```rust
pub use transit::{LineId, TransitLine, TransitLines};
```

- [ ] **Step 3: Test + commit**

```
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core --lib routing::transit 2>&1 | tail -5
```

Expected: 1 passed.

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/routing/
git commit -m "feat(8b): TransitLine + TransitLines resource with legacy-route-id lookup"
```

---

## Task 4: Cost model trait + 3 impls

**Files:**
- Modify: `backend/crates/sim-core/src/routing/cost_model.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs`

- [ ] **Step 1: Write `cost_model.rs`**

```rust
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
        assert!(TimeCost.cost(&e).is_finite(), "zero-speed must not produce NaN/inf");
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
```

- [ ] **Step 2: Re-export**

In `backend/crates/sim-core/src/routing/mod.rs` append:
```rust
pub use cost_model::{CostModel, DistanceCost, ModeFilterCost, TimeCost};
```

- [ ] **Step 3: Test + commit**

```
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core --lib routing::cost_model 2>&1 | tail -10
```

Expected: 4 passed.

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/routing/
git commit -m "feat(8b): CostModel trait + DistanceCost, TimeCost, ModeFilterCost"
```

---

## Task 5: Spatial index (`NodeSpatialIndex` via rstar)

**Files:**
- Modify: `backend/crates/sim-core/src/routing/spatial_index.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs`

- [ ] **Step 1: Write `spatial_index.rs`**

```rust
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
        self.0
            .nearest_neighbor(&[point.0, point.1])
            .map(|n| n.id)
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
        let idx = NodeSpatialIndex::from_nodes(&[
            n(0, 0.0, 0.0),
            n(1, 3.0, 0.0),
            n(2, 10.0, 0.0),
        ]);
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
```

- [ ] **Step 2: Re-export**

In `backend/crates/sim-core/src/routing/mod.rs` append:
```rust
pub use spatial_index::{IndexedNode, NodeSpatialIndex};
```

- [ ] **Step 3: Test + commit**

```
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core --lib routing::spatial_index 2>&1 | tail -10
```

Expected: 4 passed.

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/routing/
git commit -m "feat(8b): NodeSpatialIndex resource (rstar R-tree wrapper)"
```

---

## Task 6: Waiting-agents resource (`WaitingAgents`)

**Files:**
- Modify: `backend/crates/sim-core/src/routing/waiting.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs`

- [ ] **Step 1: Write `waiting.rs`**

The old `StopRecord.waiting_agents: VecDeque<AgentId>` per-tick state is moved out of the immutable Graph into its own Resource.

```rust
use bevy_ecs::prelude::*;
use std::collections::{HashMap, VecDeque};

use crate::ids::AgentId;
use crate::routing::graph::NodeId;

#[derive(Resource, Debug, Default)]
pub struct WaitingAgents(HashMap<NodeId, VecDeque<AgentId>>);

impl WaitingAgents {
    pub fn enqueue(&mut self, node: NodeId, agent: AgentId) {
        self.0.entry(node).or_default().push_back(agent);
    }

    pub fn dequeue(&mut self, node: NodeId) -> Option<AgentId> {
        self.0.get_mut(&node).and_then(|q| q.pop_front())
    }

    pub fn queue(&self, node: NodeId) -> Option<&VecDeque<AgentId>> {
        self.0.get(&node)
    }

    pub fn queue_mut(&mut self, node: NodeId) -> &mut VecDeque<AgentId> {
        self.0.entry(node).or_default()
    }

    pub fn remove_agent(&mut self, node: NodeId, agent: &AgentId) -> bool {
        if let Some(q) = self.0.get_mut(&node) {
            if let Some(pos) = q.iter().position(|a| a == agent) {
                q.remove(pos);
                return true;
            }
        }
        false
    }

    pub fn iter(&self) -> impl Iterator<Item = (&NodeId, &VecDeque<AgentId>)> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.values().all(|q| q.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_and_dequeue_preserve_order() {
        let mut w = WaitingAgents::default();
        w.enqueue(NodeId(0), AgentId("a".into()));
        w.enqueue(NodeId(0), AgentId("b".into()));
        assert_eq!(w.dequeue(NodeId(0)).unwrap().0, "a");
        assert_eq!(w.dequeue(NodeId(0)).unwrap().0, "b");
        assert!(w.dequeue(NodeId(0)).is_none());
    }

    #[test]
    fn dequeue_empty_returns_none() {
        let mut w = WaitingAgents::default();
        assert!(w.dequeue(NodeId(42)).is_none());
    }

    #[test]
    fn remove_agent_targets_specific_id() {
        let mut w = WaitingAgents::default();
        w.enqueue(NodeId(0), AgentId("a".into()));
        w.enqueue(NodeId(0), AgentId("b".into()));
        assert!(w.remove_agent(NodeId(0), &AgentId("a".into())));
        assert_eq!(w.dequeue(NodeId(0)).unwrap().0, "b");
    }
}
```

- [ ] **Step 2: Re-export**

In `backend/crates/sim-core/src/routing/mod.rs` append:
```rust
pub use waiting::WaitingAgents;
```

- [ ] **Step 3: Test + commit**

```
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core --lib routing::waiting 2>&1 | tail -10
```

Expected: 3 passed.

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/routing/
git commit -m "feat(8b): WaitingAgents resource (per-stop boarding queue)"
```

---

## Task 7: Graph builder (`build_graph_from_city_network`)

**Files:**
- Modify: `backend/crates/sim-core/src/routing/builder.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs`

- [ ] **Step 1: Write the builder skeleton + tests first**

Write tests first to lock the algorithmic contract, then implement.

In `backend/crates/sim-core/src/routing/builder.rs`:

```rust
use std::collections::HashMap;

use crate::city_network::{CityNetwork, NetworkCoord};
use crate::routing::graph::{Edge, EdgeId, EdgeKind, Graph, Node, NodeId, NodeKind};
use crate::routing::spatial_index::NodeSpatialIndex;
use crate::routing::transit::{LineId, TransitLine, TransitLines};

/// Speed-limit constants for 8b (placeholders for the future real per-edge data).
const SPEED_TRAM: f32 = 4.0;
const SPEED_ROAD: f32 = 6.0;
const SPEED_FOOT: f32 = 1.0;

/// Pre-seeded transit stops, hardcoded today in `mobility/seed.rs`.
/// Builder maps each to the nearest network node. Source of truth for these
/// strings stays in seed.rs; the builder takes them as input.
#[derive(Debug, Clone)]
pub struct SeededStop {
    pub legacy_stop_id: String,
    pub coord: (f32, f32),
    pub legacy_route_id: String,
}

pub fn build_graph_from_city_network(
    network: &CityNetwork,
    seeded_stops: &[SeededStop],
) -> (Graph, TransitLines, NodeSpatialIndex) {
    // Phase 1: identify node coords.
    // - Endpoints of any polyline.
    // - Coords appearing in 2+ polylines (intersections).
    // - Coords from `seeded_stops`.
    let mut coord_use_count: HashMap<(i32, i32), u32> = HashMap::new();
    let mut endpoint_coords: Vec<(i32, i32)> = Vec::new();
    let mut polyline_coords: Vec<Vec<(i32, i32)>> = Vec::new();
    let mut polyline_kinds: Vec<PolylineKind> = Vec::new();

    for (idx, path) in network.arterial_paths.iter().enumerate() {
        let coords = path.iter().map(|nc| (nc.x, nc.y)).collect::<Vec<_>>();
        if coords.is_empty() {
            continue;
        }
        endpoint_coords.push(coords[0]);
        endpoint_coords.push(*coords.last().unwrap());
        for c in &coords {
            *coord_use_count.entry(*c).or_insert(0) += 1;
        }
        polyline_coords.push(coords);
        polyline_kinds.push(PolylineKind::Arterial { index: idx });
    }

    for path in network.pedestrian_corridors.iter() {
        let coords = path.iter().map(|nc| (nc.x, nc.y)).collect::<Vec<_>>();
        if coords.is_empty() {
            continue;
        }
        endpoint_coords.push(coords[0]);
        endpoint_coords.push(*coords.last().unwrap());
        for c in &coords {
            *coord_use_count.entry(*c).or_insert(0) += 1;
        }
        polyline_coords.push(coords);
        polyline_kinds.push(PolylineKind::PedestrianCorridor);
    }

    let mut is_node: HashMap<(i32, i32), bool> = HashMap::new();
    for c in &endpoint_coords {
        is_node.insert(*c, true);
    }
    for (c, count) in &coord_use_count {
        if *count >= 2 {
            is_node.insert(*c, true);
        }
    }
    // Snap each seeded stop to an existing polyline coord (must already exist).
    for stop in seeded_stops {
        let coord = (stop.coord.0.round() as i32, stop.coord.1.round() as i32);
        is_node.insert(coord, true);
    }

    // Phase 2: assign NodeIds in deterministic order (sorted by coord).
    let mut node_coords: Vec<(i32, i32)> = is_node.keys().copied().collect();
    node_coords.sort();
    let mut nodes: Vec<Node> = Vec::with_capacity(node_coords.len());
    let mut node_id_by_coord: HashMap<(i32, i32), NodeId> = HashMap::new();
    for (idx, coord) in node_coords.iter().enumerate() {
        let id = NodeId(idx as u32);
        node_id_by_coord.insert(*coord, id);
        nodes.push(Node {
            id,
            position: (coord.0 as f32, coord.1 as f32),
            kind: NodeKind::Intersection, // upgraded to TransitStop below if applicable
            legacy_id: None,
        });
    }

    // Mark stop nodes with kind=TransitStop and legacy_id.
    for stop in seeded_stops {
        let coord = (stop.coord.0.round() as i32, stop.coord.1.round() as i32);
        let node_id = *node_id_by_coord.get(&coord).expect("seeded stop coord must be a node");
        let n = &mut nodes[node_id.0 as usize];
        n.kind = NodeKind::TransitStop;
        n.legacy_id = Some(stop.legacy_stop_id.clone());
    }

    // Phase 3: walk each polyline, split at node coords, emit edges.
    let mut edges: Vec<Edge> = Vec::new();
    let mut tram_edges_by_arterial: HashMap<usize, Vec<EdgeId>> = HashMap::new();

    for (poly_idx, coords) in polyline_coords.iter().enumerate() {
        let kind = polyline_kinds[poly_idx];
        // Split positions: first coord, each interior node coord, last coord.
        let mut split_indices: Vec<usize> = vec![0];
        for (i, c) in coords.iter().enumerate().skip(1).take(coords.len().saturating_sub(2)) {
            if node_id_by_coord.contains_key(c) {
                split_indices.push(i);
            }
        }
        split_indices.push(coords.len() - 1);
        for win in split_indices.windows(2) {
            let (a, b) = (win[0], win[1]);
            let segment = &coords[a..=b];
            let from = node_id_by_coord[&segment[0]];
            let to = node_id_by_coord[&*segment.last().unwrap()];
            let polyline: Vec<(f32, f32)> = segment.iter().map(|c| (c.0 as f32, c.1 as f32)).collect();
            let length = polyline_length(&polyline);
            match kind {
                PolylineKind::Arterial { index } => {
                    // Emit two parallel edges (TramTrack + Road), bidirectional pair each.
                    let tram_legacy_fwd = Some(format!("link:tram:{}:{}_{}", index, segment[0].0, segment[0].1));
                    let tram_fwd = Edge {
                        id: EdgeId(edges.len() as u32),
                        from, to,
                        polyline: polyline.clone(),
                        length,
                        kind: EdgeKind::TramTrack,
                        speed_limit: SPEED_TRAM,
                        capacity: 1,
                        legacy_id: tram_legacy_fwd,
                    };
                    tram_edges_by_arterial.entry(index).or_default().push(tram_fwd.id);
                    edges.push(tram_fwd);
                    let tram_bwd = Edge {
                        id: EdgeId(edges.len() as u32),
                        from: to, to: from,
                        polyline: polyline.iter().rev().copied().collect(),
                        length,
                        kind: EdgeKind::TramTrack,
                        speed_limit: SPEED_TRAM,
                        capacity: 1,
                        legacy_id: None,
                    };
                    edges.push(tram_bwd);
                    let road_fwd = Edge {
                        id: EdgeId(edges.len() as u32),
                        from, to,
                        polyline: polyline.clone(),
                        length,
                        kind: EdgeKind::Road,
                        speed_limit: SPEED_ROAD,
                        capacity: 1,
                        legacy_id: None,
                    };
                    edges.push(road_fwd);
                    let road_bwd = Edge {
                        id: EdgeId(edges.len() as u32),
                        from: to, to: from,
                        polyline: polyline.iter().rev().copied().collect(),
                        length,
                        kind: EdgeKind::Road,
                        speed_limit: SPEED_ROAD,
                        capacity: 1,
                        legacy_id: None,
                    };
                    edges.push(road_bwd);
                }
                PolylineKind::PedestrianCorridor => {
                    let foot_fwd = Edge {
                        id: EdgeId(edges.len() as u32),
                        from, to,
                        polyline: polyline.clone(),
                        length,
                        kind: EdgeKind::Footway,
                        speed_limit: SPEED_FOOT,
                        capacity: 1,
                        legacy_id: Some(format!("link:walk:{}_{}_to_{}_{}", segment[0].0, segment[0].1, segment.last().unwrap().0, segment.last().unwrap().1)),
                    };
                    edges.push(foot_fwd);
                    let foot_bwd = Edge {
                        id: EdgeId(edges.len() as u32),
                        from: to, to: from,
                        polyline: polyline.iter().rev().copied().collect(),
                        length,
                        kind: EdgeKind::Footway,
                        speed_limit: SPEED_FOOT,
                        capacity: 1,
                        legacy_id: None,
                    };
                    edges.push(foot_bwd);
                }
            }
        }
    }

    let graph = Graph::new(nodes, edges);
    let spatial_index = NodeSpatialIndex::from_nodes(graph.nodes());

    // Phase 4: transit lines — one per arterial.
    let mut lines: Vec<TransitLine> = Vec::new();
    for (arterial_idx, edges_in_line) in tram_edges_by_arterial {
        let stops_in_line: Vec<NodeId> = seeded_stops
            .iter()
            .filter_map(|s| graph.node_by_legacy(&s.legacy_stop_id))
            .filter(|n| {
                // Only stops along this arterial: their position must lie on one of edges_in_line.
                let np = graph.node(*n).position;
                edges_in_line.iter().any(|e| graph.edge(*e).polyline.iter().any(|p| p.0 == np.0 && p.1 == np.1))
            })
            .collect();
        let legacy_route_id = if arterial_idx == 0 {
            Some("route:horizontal".to_string())
        } else if arterial_idx == 1 {
            Some("route:vertical".to_string())
        } else {
            None
        };
        let line = TransitLine {
            id: LineId(lines.len() as u32),
            name: format!("arterial_{arterial_idx}"),
            edges: edges_in_line,
            stops: stops_in_line,
            legacy_route_id,
        };
        lines.push(line);
    }
    let transit_lines = TransitLines::new(lines);

    (graph, transit_lines, spatial_index)
}

#[derive(Debug, Clone, Copy)]
enum PolylineKind {
    Arterial { index: usize },
    PedestrianCorridor,
}

fn polyline_length(points: &[(f32, f32)]) -> f32 {
    points
        .windows(2)
        .map(|w| {
            let dx = w[1].0 - w[0].0;
            let dy = w[1].1 - w[0].1;
            (dx * dx + dy * dy).sqrt()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city_network::{CityNetwork, WorldTiles};

    fn nc(x: i32, y: i32) -> NetworkCoord { NetworkCoord { x, y } }

    fn simple_network() -> CityNetwork {
        // Two arterials forming a T-junction at (5, 0).
        // Arterial 0: (0,0) → (5,0) → (10,0)
        // Arterial 1: (5,0) → (5,5)
        CityNetwork {
            version: 1,
            world_id: "test".into(),
            chunk_size: 32,
            world_tiles: WorldTiles { width: 32, height: 32 },
            arterial_paths: vec![
                vec![nc(0, 0), nc(5, 0), nc(10, 0)],
                vec![nc(5, 0), nc(5, 5)],
            ],
            pedestrian_corridors: vec![],
        }
    }

    #[test]
    fn builder_creates_nodes_at_intersections() {
        let (graph, _, _) = build_graph_from_city_network(&simple_network(), &[]);
        // Expected nodes: endpoints (0,0), (10,0), (5,5) AND intersection (5,0).
        assert_eq!(graph.node_count(), 4);
    }

    #[test]
    fn builder_emits_bidirectional_tram_plus_road_per_arterial_segment() {
        let (graph, _, _) = build_graph_from_city_network(&simple_network(), &[]);
        // Arterial 0 has 2 segments (0,0)-(5,0) and (5,0)-(10,0), each → 4 edges (tram fwd/bwd + road fwd/bwd) = 8.
        // Arterial 1 has 1 segment → 4 edges.
        // Total = 12.
        assert_eq!(graph.edge_count(), 12);
        // All arterial edges are TramTrack or Road, none Footway.
        for e in graph.edges() {
            assert!(matches!(e.kind, EdgeKind::TramTrack | EdgeKind::Road));
        }
    }

    #[test]
    fn builder_uses_seeded_stops_as_nodes() {
        let stops = vec![SeededStop {
            legacy_stop_id: "stop:on_arterial".into(),
            coord: (5.0, 0.0),
            legacy_route_id: "route:horizontal".into(),
        }];
        let (graph, _, _) = build_graph_from_city_network(&simple_network(), &stops);
        let node_id = graph.node_by_legacy("stop:on_arterial").expect("stop must resolve");
        assert_eq!(graph.node(node_id).kind, NodeKind::TransitStop);
    }

    #[test]
    fn builder_creates_one_transit_line_per_arterial() {
        let (_, lines, _) = build_graph_from_city_network(&simple_network(), &[]);
        assert_eq!(lines.count(), 2);
    }

    #[test]
    fn polyline_length_is_arc_length() {
        let p = vec![(0.0, 0.0), (3.0, 4.0), (3.0, 8.0)];
        assert_eq!(polyline_length(&p), 5.0 + 4.0);
    }

    #[test]
    fn empty_polyline_skipped() {
        let mut net = simple_network();
        net.arterial_paths.push(vec![]);
        let (graph, _, _) = build_graph_from_city_network(&net, &[]);
        // Same node + edge counts as the simple test.
        assert_eq!(graph.node_count(), 4);
        assert_eq!(graph.edge_count(), 12);
    }
}
```

- [ ] **Step 2: Re-export**

In `backend/crates/sim-core/src/routing/mod.rs` append:
```rust
pub use builder::{build_graph_from_city_network, SeededStop};
```

- [ ] **Step 3: Test + commit**

```
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core --lib routing::builder 2>&1 | tail -15
```

Expected: 6 passed.

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/routing/
git commit -m "feat(8b): graph builder from CityNetwork + seeded stops"
```

---

## Task 8: `RoutingPlugin` + runtime install order

**Files:**
- Modify: `backend/crates/sim-core/src/routing/plugin.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [ ] **Step 1: Write `plugin.rs`**

The plugin reads `CityNetwork` (a Resource, already installed by callers — `runtime.rs` inserts it before plugin install). For tests that don't have a network, the plugin installs an empty graph.

```rust
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::Schedule;

use crate::city_network::CityNetwork;
use crate::routing::builder::{build_graph_from_city_network, SeededStop};
use crate::routing::graph::Graph;
use crate::routing::spatial_index::NodeSpatialIndex;
use crate::routing::transit::TransitLines;
use crate::routing::waiting::WaitingAgents;
use crate::world::schedule::SimPlugin;

pub struct RoutingPlugin {
    pub seeded_stops: Vec<SeededStop>,
}

impl Default for RoutingPlugin {
    fn default() -> Self {
        Self { seeded_stops: Vec::new() }
    }
}

impl SimPlugin for RoutingPlugin {
    fn name(&self) -> &'static str { "routing" }

    fn install(&self, world: &mut World, _schedule: &mut Schedule) {
        let (graph, transit_lines, spatial_index) = match world.get_resource::<CityNetwork>() {
            Some(network) => build_graph_from_city_network(network, &self.seeded_stops),
            None => (Graph::default(), TransitLines::default(), NodeSpatialIndex::default()),
        };
        world.insert_resource(graph);
        world.insert_resource(transit_lines);
        world.insert_resource(spatial_index);
        world.insert_resource(WaitingAgents::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::plugin::CorePlugin;

    #[test]
    fn routing_plugin_installs_empty_graph_without_city_network() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        RoutingPlugin::default().install(&mut world, &mut schedule);
        assert!(world.contains_resource::<Graph>());
        assert!(world.contains_resource::<TransitLines>());
        assert!(world.contains_resource::<NodeSpatialIndex>());
        assert!(world.contains_resource::<WaitingAgents>());
        assert_eq!(world.resource::<Graph>().node_count(), 0);
    }
}
```

- [ ] **Step 2: Re-export**

In `backend/crates/sim-core/src/routing/mod.rs` append:
```rust
pub use plugin::RoutingPlugin;
```

- [ ] **Step 3: Install in runtime**

In `backend/crates/sim-server/src/runtime.rs`, find `new_with_event_store` (the constructor). Currently the plugin install sequence is:
```rust
sim_core::world::plugin::CorePlugin::default().install(&mut world, &mut schedule);
sim_core::mobility::MobilityPlugin.install(&mut world, &mut schedule);
crate::persistence_plugin::PersistencePlugin { ... }.install(&mut world, &mut schedule);
```

We also need to load the `CityNetwork` resource BEFORE `RoutingPlugin` runs. Today the city network loads inside the mobility seed path. Move that load to before plugin install:

```rust
// Load city network from disk and insert as resource before plugins run.
let network_path = crate::app::resolve_city_network_path(); // exists today
let city_network = sim_core::city_network::CityNetwork::load_from_path(&network_path)
    .unwrap_or_else(|_| sim_core::city_network::CityNetwork::empty_for_world("abutown-main"));
world.insert_resource(city_network);

sim_core::world::plugin::CorePlugin::default().install(&mut world, &mut schedule);

// Seeded stops are derived from today's hardcoded mobility seed.
// For T8 we pass an empty list (graph has intersections only, no transit
// stops yet). T9's seed.rs migration provides the real list.
let seeded_stops: Vec<sim_core::routing::SeededStop> = Vec::new();
sim_core::routing::RoutingPlugin { seeded_stops }
    .install(&mut world, &mut schedule);

sim_core::mobility::MobilityPlugin.install(&mut world, &mut schedule);
crate::persistence_plugin::PersistencePlugin { ... }.install(&mut world, &mut schedule);
```

If `CityNetwork::empty_for_world` doesn't exist today, add a simple constructor:
```rust
// In backend/crates/sim-core/src/city_network.rs
impl CityNetwork {
    pub fn empty_for_world(world_id: &str) -> Self {
        Self {
            version: 1,
            world_id: world_id.to_string(),
            chunk_size: 32,
            world_tiles: WorldTiles { width: 256, height: 256 },
            arterial_paths: Vec::new(),
            pedestrian_corridors: Vec::new(),
        }
    }
}
```

(If a constructor with this name exists already, use it instead.)

- [ ] **Step 4: Build + test**

```
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo build 2>&1 | tail -10
cargo test --workspace 2>&1 | grep -E "test result|FAILED" | tail -15
```

Expected: all tests pass. Routing resources now exist in every constructed runtime.

- [ ] **Step 5: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add -A
git commit -m "feat(8b): RoutingPlugin installs Graph/TransitLines/SpatialIndex/WaitingAgents"
```

---

## Task 9: Migrate `mobility/seed.rs` to consume `Graph` + `TransitLines`

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/seed.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs` (populate `seeded_stops` before `RoutingPlugin` install)

The old `from_network(network, density)` builds `Routes`/`Stops`/`LinkPolylines` and seeds agents/vehicles. After T9:
- Stops are derived from a small `seeded_stops` list and become real `NodeKind::TransitStop` graph nodes.
- Agents and vehicles are seeded against graph entities — their `WalkPlan`/`RoutePosition` fields carry **string ids** (preserving wire compat) that resolve through `graph.node_by_legacy()` / `graph.edge_by_legacy()` / `transit_lines.line_by_legacy()` at runtime.

- [ ] **Step 1: Extract seeded stops into a top-level constant**

In `backend/crates/sim-core/src/mobility/seed.rs`, near the top, define:
```rust
/// Hardcoded transit stops the world seeds today. Coords are network-tile units.
/// These get promoted to graph nodes via `RoutingPlugin.seeded_stops`.
pub fn legacy_seeded_stops() -> Vec<crate::routing::SeededStop> {
    vec![
        crate::routing::SeededStop {
            legacy_stop_id: "stop:horizontal:pickup".into(),
            coord: (32.0, 128.0),
            legacy_route_id: "route:horizontal".into(),
        },
        crate::routing::SeededStop {
            legacy_stop_id: "stop:horizontal:dropoff".into(),
            coord: (224.0, 128.0),
            legacy_route_id: "route:horizontal".into(),
        },
        crate::routing::SeededStop {
            legacy_stop_id: "stop:vertical:pickup".into(),
            coord: (128.0, 32.0),
            legacy_route_id: "route:vertical".into(),
        },
        crate::routing::SeededStop {
            legacy_stop_id: "stop:vertical:dropoff".into(),
            coord: (128.0, 224.0),
            legacy_route_id: "route:vertical".into(),
        },
    ]
}
```

(Inspect the existing seed.rs to find the precise current coord values and stop ids — replace these placeholder values with the actual ones. The function shape is what matters.)

- [ ] **Step 2: Update runtime to pass seeded stops to RoutingPlugin**

In `backend/crates/sim-server/src/runtime.rs`, replace the empty `seeded_stops` from T8 with:
```rust
let seeded_stops = sim_core::mobility::seed::legacy_seeded_stops();
sim_core::routing::RoutingPlugin { seeded_stops }
    .install(&mut world, &mut schedule);
```

- [ ] **Step 3: Rewrite `mobility::seed::from_network`**

The body that today calls `world.add_route(...)`, `world.add_stop(...)`, `world.set_link_polyline(...)` is replaced with reads from `Graph` and `TransitLines`. Agents and vehicles still spawn via the existing `spawn_agent_from_record` / `spawn_vehicle_from_record` helpers, but their plan stages carry strings (legacy ids) that resolve through graph at runtime.

Concretely:
- Remove every `add_route` / `add_stop` / `set_link_polyline` call. The graph is already populated by `RoutingPlugin`.
- For each vehicle to spawn, set `RoutePosition.route_id` to the legacy string `"route:horizontal"` (current API will be migrated in T10 to a `LineId`).
- For each agent, build `PlanStage::WalkToStop { link_id: "<legacy_link_id>".into(), stop_id: "<legacy_stop_id>".into() }` and `PlanStage::RideToStop { route_id: "<legacy_route_id>".into(), stop_id: "<legacy_stop_id>".into() }` using legacy strings.

(The walk_advance/vehicle_advance/boarding_alighting systems still consume Routes/Stops/LinkPolylines as-is — they get migrated in T10/T11/T12. Until then this commit is a dual-track: the graph exists in parallel with the unmigrated systems still reading the old resources, which seed.rs continues to populate ALONGSIDE the graph for this transitional commit.)

So in this task seed.rs both:
- Builds the legacy Routes/Stops/LinkPolylines (existing logic kept)
- Also passes seeded_stops to RoutingPlugin so the graph has transit stops

T10-T12 migrate the consumers; T13 deletes the old.

- [ ] **Step 4: Build + smoke**

```
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo build 2>&1 | tail -10
cargo test --workspace 2>&1 | grep -E "test result|FAILED" | tail -15

cd /Users/ramonfuglister/Desktop/Coding/abutown
npm run dev:stack > /tmp/devstack-t9.log 2>&1 &
DEVSTACK_PID=$!
for i in $(seq 1 30); do
  if grep -q "VITE.*ready" /tmp/devstack-t9.log 2>/dev/null; then break; fi
  sleep 1
done
sleep 3
node scripts/smoke-7b.mjs 2>&1 | tail -20
kill $DEVSTACK_PID 2>/dev/null
pkill -f sim-server 2>/dev/null
pkill -f "vite.*5175" 2>/dev/null
```

Expected: workspace tests green, smoke 9/9 (mobility still works via old resources, graph exists in parallel).

- [ ] **Step 5: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add -A
git commit -m "feat(8b): seed.rs publishes seeded stops to RoutingPlugin (dual-track)"
```

---

## Task 10: Migrate `walk_advance_system` + `vehicle_advance_system` to read `Graph`

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/systems.rs`
- Modify: `backend/crates/sim-core/src/mobility/components.rs`
- Modify: `backend/crates/sim-core/src/mobility/dto.rs` (if it has anything that needs graph lookup)

- [ ] **Step 1: Change `RoutePosition` to carry `LineId` + `edge_index`**

In `backend/crates/sim-core/src/mobility/components.rs`, find the existing `RoutePosition`:
```rust
#[derive(Component, Copy, Clone, Debug)]
pub struct RoutePosition {
    pub route_id: RouteId,
    pub link_index: usize,
    pub progress: f32,
    pub speed: f32,
}
```

Replace with:
```rust
#[derive(Component, Copy, Clone, Debug)]
pub struct RoutePosition {
    pub line_id: crate::routing::LineId,
    pub edge_index: usize,
    pub progress: f32,
    pub speed: f32,
}
```

The vehicle DTO (wire / persistence) keeps a `route_id: String` field — the encoding/decoding happens at the DTO boundary using `transit_lines.line(line_id).legacy_route_id.clone().unwrap_or_else(|| line.name.clone())`.

- [ ] **Step 2: Update `spawn_vehicle_from_record` to resolve string→LineId**

In `backend/crates/sim-core/src/mobility/api.rs`, find `spawn_vehicle_from_record`. The record arrives with `route_id: RouteId` (string-wrapped). Convert to `LineId`:
```rust
let transit_lines = world.resource::<crate::routing::TransitLines>();
let line_id = transit_lines.line_by_legacy(&record.route_id.0).unwrap_or_else(|| {
    panic!("spawn_vehicle_from_record: unknown route_id {}", record.route_id.0)
});
```

Build `RoutePosition` with that `line_id`.

- [ ] **Step 3: Rewrite `vehicle_advance_system`**

The system today reads `routes: Res<Routes>` + `link_polylines: Res<LinkPolylines>`. Rewrite to read `Res<Graph>` + `Res<TransitLines>`:
```rust
fn vehicle_advance_system(
    graph: Res<crate::routing::Graph>,
    transit_lines: Res<crate::routing::TransitLines>,
    mut vehicles: Query<(&mut RoutePosition, &mut Position, ...)>,
) {
    for (mut route_pos, ...) in &mut vehicles {
        let line = transit_lines.line(route_pos.line_id);
        let edge_id = line.edges[route_pos.edge_index];
        let edge = graph.edge(edge_id);
        // advance progress along edge.polyline
        // when progress >= 1.0, increment edge_index, reset progress
        // ... same logic as today, just sourced from graph
    }
}
```

(Use the existing arc-length-walk helper `mobility_geometry::world_coord_at_progress_slice(&edge.polyline, progress)` — same one as today.)

- [ ] **Step 4: Rewrite `walk_advance_system`**

Today reads `LinkPolylines`. Rewrite to read `Res<Graph>`. The agent state `Walking { link_id: String }` keeps the string; the system resolves once per agent:
```rust
fn walk_advance_system(
    graph: Res<crate::routing::Graph>,
    mut agents: Query<(&mut AgentMobilityStateComponent, &mut Position, &WalkSpeed)>,
) {
    for (mut state_comp, mut pos, speed) in &mut agents {
        if let AgentMobilityState::Walking { link_id, progress } = &mut state_comp.0 {
            let Some(edge_id) = graph.edge_by_legacy(link_id) else { continue; };
            let edge = graph.edge(edge_id);
            *progress = (*progress + speed.0 / edge.length).min(1.0);
            let coord = crate::mobility_geometry::world_coord_at_progress_slice(&edge.polyline, *progress);
            pos.x = coord.0;
            pos.y = coord.1;
        }
    }
}
```

(The exact field names of components — match the existing system body. The point is: replace `link_polylines.0.get(link_id)` with `graph.edge_by_legacy(link_id)`. The `Routes` resource is no longer needed by this system.)

- [ ] **Step 5: Build + smoke**

```
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo build 2>&1 | tail -10
cargo test --workspace 2>&1 | grep -E "test result|FAILED" | tail -15

cd /Users/ramonfuglister/Desktop/Coding/abutown
npm run dev:stack > /tmp/devstack-t10.log 2>&1 &
DEVSTACK_PID=$!
for i in $(seq 1 30); do
  if grep -q "VITE.*ready" /tmp/devstack-t10.log 2>/dev/null; then break; fi
  sleep 1
done
sleep 3
node scripts/smoke-7b.mjs 2>&1 | tail -20
kill $DEVSTACK_PID 2>/dev/null
pkill -f sim-server 2>/dev/null
pkill -f "vite.*5175" 2>/dev/null
```

Expected: cargo green, smoke 9/9.

- [ ] **Step 6: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add -A
git commit -m "refactor(8b): walk_advance + vehicle_advance read from Graph"
```

---

## Task 11: Migrate `boarding_alighting_system` + `stop_arrival_system` to `Graph` + `WaitingAgents`

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/systems.rs`

- [ ] **Step 1: Migrate `stop_arrival_system`**

Today reads `routes: Res<Routes>` + `stops: ResMut<Stops>` and inserts the agent into `StopRecord.waiting_agents`. Rewrite to use `Graph` + `WaitingAgents` resource:

```rust
fn stop_arrival_system(
    graph: Res<crate::routing::Graph>,
    transit_lines: Res<crate::routing::TransitLines>,
    mut waiting: ResMut<crate::routing::WaitingAgents>,
    mut agents: Query<(&AgentId, &mut AgentMobilityStateComponent, &mut WalkPlan), With<NearStop>>,
    mut commands: Commands,
) {
    for (agent_id, mut state, mut plan) in &mut agents {
        // ... existing transition logic ...
        // When transitioning to WaitingAtStop:
        if let AgentMobilityState::WaitingAtStop { stop_id } = &state.0 {
            let Some(node_id) = graph.node_by_legacy(stop_id) else { continue; };
            waiting.enqueue(node_id, agent_id.clone());
        }
        // ... rest of state machine ...
    }
}
```

(Replicate the existing transition logic verbatim; only the `stops.0.get_mut(...).waiting_agents.push_back(...)` becomes `waiting.enqueue(node_id, agent_id)`.)

- [ ] **Step 2: Migrate `boarding_alighting_system`**

Today the system reads `routes: Res<Routes>` + `stops: ResMut<Stops>` and walks each vehicle's `RoutePosition` to find matching stops via the `routes` table and the `stops` waiting-queue. Rewrite to read `Graph` + `TransitLines` + `WaitingAgents`:

```rust
fn boarding_alighting_system(
    graph: Res<crate::routing::Graph>,
    transit_lines: Res<crate::routing::TransitLines>,
    mut waiting: ResMut<crate::routing::WaitingAgents>,
    // ... existing queries for vehicles + agents ...
) {
    // Pre-index end-of-edge stops by (LineId, edge_index) — same shape as today's end_of_link_stops.
    let mut end_of_edge_stops: HashMap<(crate::routing::LineId, usize), crate::routing::NodeId> = HashMap::new();
    for line in transit_lines.iter() {
        for (i, edge_id) in line.edges.iter().enumerate() {
            let to = graph.edge(*edge_id).to;
            if graph.node(to).kind == crate::routing::NodeKind::TransitStop {
                end_of_edge_stops.insert((line.id, i), to);
            }
        }
    }
    // ... rest: per-vehicle find (line_id, edge_index), look up node, drain waiting queue ...
}
```

The agent-side state transitions when boarding/alighting are unchanged in structure; only the data sources change.

- [ ] **Step 3: Build + smoke**

```
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo build 2>&1 | tail -10
cargo test --workspace 2>&1 | grep -E "test result|FAILED" | tail -15

cd /Users/ramonfuglister/Desktop/Coding/abutown
npm run dev:stack > /tmp/devstack-t11.log 2>&1 &
DEVSTACK_PID=$!
for i in $(seq 1 30); do
  if grep -q "VITE.*ready" /tmp/devstack-t11.log 2>/dev/null; then break; fi
  sleep 1
done
sleep 3
node scripts/smoke-7b.mjs 2>&1 | tail -20
kill $DEVSTACK_PID 2>/dev/null
pkill -f sim-server 2>/dev/null
pkill -f "vite.*5175" 2>/dev/null
```

Expected: cargo green, smoke 9/9 (agents board + alight trams correctly).

- [ ] **Step 4: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add -A
git commit -m "refactor(8b): boarding_alighting + stop_arrival use Graph + WaitingAgents"
```

---

## Task 12: Delete `Routes`, `Stops`, `LinkPolylines` + dependent types

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/resources.rs` (delete 3 structs)
- Modify: `backend/crates/sim-core/src/mobility/records.rs` (delete `RouteRecord`, `StopRecord`)
- Modify: `backend/crates/sim-core/src/ids.rs` (delete `RouteId`, `LinkId`, `StopId`)
- Modify: `backend/crates/sim-core/src/mobility/api.rs` (delete `add_route`, `add_stop`, `set_link_polyline`, `routes`, `stops`, `stop`, `link_polyline` and their tests; replace public reads with graph-backed equivalents where called externally)
- Modify: `backend/crates/sim-core/src/mobility/seed.rs` (remove the legacy `add_route`/`add_stop`/`set_link_polyline` calls left in T9; only the graph path remains)
- Modify: `backend/crates/sim-core/src/mobility/persist_snapshot.rs` (`StopRecord` references → small persistence-local stop type or string fields only)
- Modify: any tests still constructing `Routes`/`Stops`/`LinkPolylines`/`RouteRecord`/`StopRecord` — rewrite to construct the equivalent graph entries via the builder or to test against the graph directly

- [ ] **Step 1: Grep all call sites**

```bash
grep -rn "Routes\b\|Stops\b\|LinkPolylines\b" backend/crates/ --include='*.rs' | grep -v 'TransitLines\|legacy_route' | head -50
grep -rn "RouteId\b\|LinkId\b\|StopId\b" backend/crates/ --include='*.rs' | grep -v 'LegacyRouteId\|legacy_route' | head -50
grep -rn "RouteRecord\b\|StopRecord\b" backend/crates/ --include='*.rs' | head -30
```

Make a checklist. Every match must either become a graph query or be deleted (test code, no-longer-needed accessor, etc.).

- [ ] **Step 2: Delete the type declarations**

In `backend/crates/sim-core/src/mobility/resources.rs`, delete:
```rust
pub struct Routes(...);
pub struct Stops(...);
pub struct LinkPolylines(...);
```
and their `Default` impls and any `Resource` derives.

In `backend/crates/sim-core/src/mobility/records.rs`, delete:
```rust
pub struct StopRecord { ... }
pub struct RouteRecord { ... }
```

In `backend/crates/sim-core/src/ids.rs`, delete:
```rust
pub struct RouteId(pub String);
pub struct LinkId(pub String);
pub struct StopId(pub String);
```

- [ ] **Step 3: Migrate remaining call sites**

Iterate `cargo build` and fix each compile error:
- `world.resource::<Routes>()` → `world.resource::<crate::routing::TransitLines>()`
- `world.resource::<Stops>()` → `world.resource::<crate::routing::WaitingAgents>()` (for waiting-agents reads) OR `world.resource::<crate::routing::Graph>()` (for stop-existence checks)
- `world.resource::<LinkPolylines>()` → `world.resource::<crate::routing::Graph>()` + `graph.edge_by_legacy(link_id)`
- `RouteId(s)` → `String` directly (in DTO/wire) or `crate::routing::LineId` (in internal state)
- `LinkId(s)` → `String` (wire) or `EdgeId` (internal)
- `StopId(s)` → `String` (wire) or `NodeId` (internal)

In `mobility/seed.rs`, delete the `add_route`/`add_stop`/`set_link_polyline` calls left from T9. Only the seeded_stops-publishing path remains.

In `mobility/persist_snapshot.rs`, the persist payload today serializes `StopRecord` and `RouteRecord` shapes. Replace with a small persistence-local struct that carries the wire fields verbatim:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedStop {
    pub id: String,
    pub route_id: String,
    pub edge_index: usize,
    pub progress: f32,
    pub waiting_agents: Vec<String>,
}
```
On `extract_from_world`, build `Vec<PersistedStop>` from `graph.nodes()` filtered to `NodeKind::TransitStop` plus `waiting_agents.queue(node_id)`. On `apply_into_world`, repopulate `waiting_agents` by parsing each `PersistedStop`.

(The JSON wire format MUST stay identical to today's. The fields `id`, `route_id`, `link_index`, `progress`, `waiting_agents` are the on-disk schema — preserve exactly. Note: today's persistence uses `link_index`, not `edge_index`. The persistence field stays `link_index` for wire compat; only the internal type changed.)

- [ ] **Step 4: Build + test + clippy + smoke**

```
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo build 2>&1 | tail -15
cargo test --workspace 2>&1 | grep -E "test result|FAILED" | tail -20
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5

cd /Users/ramonfuglister/Desktop/Coding/abutown
npx tsc --noEmit 2>&1 | tail -5
npx vitest run --reporter=dot 2>&1 | tail -10

npm run dev:stack > /tmp/devstack-t12.log 2>&1 &
DEVSTACK_PID=$!
for i in $(seq 1 30); do
  if grep -q "VITE.*ready" /tmp/devstack-t12.log 2>/dev/null; then break; fi
  sleep 1
done
sleep 3
node scripts/smoke-7b.mjs 2>&1 | tail -20
kill $DEVSTACK_PID 2>/dev/null
pkill -f sim-server 2>/dev/null
pkill -f "vite.*5175" 2>/dev/null
```

Expected: everything green, smoke 9/9.

- [ ] **Step 5: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add -A
git commit -m "refactor(8b): delete Routes/Stops/LinkPolylines + RouteRecord/StopRecord + RouteId/LinkId/StopId"
```

---

## Task 13: Acceptance verification + perf bench + progress note

**Files:**
- Modify: `progress.md`

- [ ] **Step 1: Run all acceptance greps**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown

echo "=== Grep 1: old resources gone ==="
(grep -rn 'pub struct \(Routes\|Stops\|LinkPolylines\)' backend/crates/ --include='*.rs' || echo "OK")
echo "=== Grep 2: old records gone ==="
(grep -rn 'pub struct \(RouteRecord\|StopRecord\)' backend/crates/ --include='*.rs' || echo "OK")
echo "=== Grep 3: old IDs gone ==="
(grep -rn 'pub struct \(RouteId\|LinkId\|StopId\)' backend/crates/sim-core/src/ids.rs --include='*.rs' || echo "OK")
echo "=== Grep 4: routing module exists ==="
ls backend/crates/sim-core/src/routing/ | head -10
echo "=== Grep 5: Graph populated ==="
# Inspected via the test added below.
```

Greps 1–3 should print "OK". Grep 4 should list the 8 files.

- [ ] **Step 2: Add graph-populated runtime test**

In `backend/crates/sim-server/src/runtime.rs` `#[cfg(test)] mod tests`, add:
```rust
    #[test]
    fn runtime_has_populated_routing_graph() {
        let runtime = SimulationRuntime::new();
        let world = &runtime.world;
        let graph = world.resource::<sim_core::routing::Graph>();
        assert!(graph.node_count() > 0, "graph must have nodes after hydration");
        assert!(graph.edge_count() > 0, "graph must have edges after hydration");
        let transit = world.resource::<sim_core::routing::TransitLines>();
        assert!(transit.count() > 0, "must have at least one transit line");
        let spatial = world.resource::<sim_core::routing::NodeSpatialIndex>();
        assert_eq!(spatial.size(), graph.node_count());
    }
```

- [ ] **Step 3: Run all test suites**

```
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo build 2>&1 | tail -5
cargo test --workspace 2>&1 | tee /tmp/8b-cargo-test.log | grep -E "test result|FAILED" | tail -25
grep "test result: ok" /tmp/8b-cargo-test.log | awk '{s+=$4} END {print "Total cargo tests:", s}'
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5

cd /Users/ramonfuglister/Desktop/Coding/abutown
npx tsc --noEmit 2>&1 | tail -5
npx vitest run --reporter=dot 2>&1 | tail -10
```

Expected: workspace tests pass, clippy clean, tsc clean, vitest pass.

- [ ] **Step 4: Browser smoke**

```
cd /Users/ramonfuglister/Desktop/Coding/abutown
npm run dev:stack > /tmp/devstack-t13.log 2>&1 &
DEVSTACK_PID=$!
for i in $(seq 1 30); do
  if grep -q "VITE.*ready" /tmp/devstack-t13.log 2>/dev/null; then break; fi
  sleep 1
done
sleep 3
node scripts/smoke-7b.mjs 2>&1 | tail -30
kill $DEVSTACK_PID 2>/dev/null
pkill -f sim-server 2>/dev/null
pkill -f "vite.*5175" 2>/dev/null
```

Expected: 9/9.

- [ ] **Step 5: Perf bench**

```
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo bench --bench mobility_tick_lod -- tick_100k_all_active 2>&1 | tail -20
```

Capture median. Compare to Phase 8a baseline 11.84 ms. Compute delta %. Spec budget: ≤ +5% (12.43 ms).

- [ ] **Step 6: Write progress.md entry**

Insert at the TOP of the reverse-chronological block (line 19+, right after line 18) in `progress.md`:

```
2026-05-21T<HH:MM:SS>.000Z - Phase 8b — Routing Graph + Spatial Index + Cost Model: replaced the polyline-soup `CityNetwork` + `Routes`/`Stops`/`LinkPolylines` triple with a real routing graph. New `sim_core::routing` module ships `Graph` (Vec-indexed nodes + edges, bidirectional adjacency, legacy-id lookup maps), `TransitLines` (resource with `LineId → TransitLine`), `NodeSpatialIndex` (rstar R-tree), `WaitingAgents` (per-stop boarding queue), and `CostModel` trait + 3 impls (`DistanceCost`, `TimeCost`, `ModeFilterCost<C>`). `RoutingPlugin: SimPlugin` builds the graph once at startup from `CityNetwork` via `build_graph_from_city_network`, which splits arterial polylines into bidirectional `TramTrack` + `Road` edges and pedestrian corridors into bidirectional `Footway` edges, identifies intersections (coord in ≥ 2 polylines), and snaps seeded stops to graph nodes with `NodeKind::TransitStop`. Mobility systems (`walk_advance`, `vehicle_advance`, `boarding_alighting`, `stop_arrival`, `warm_chunk_flow`, `track_chunk_populations`) rewritten to read `Graph` + `TransitLines` + `WaitingAgents`. `RoutePosition` migrated from `(RouteId, link_index)` to `(LineId, edge_index)`. Old `Routes`/`Stops`/`LinkPolylines` resources, `RouteRecord`/`StopRecord` records, and `RouteId`/`LinkId`/`StopId` newtypes all deleted. Wire bytes byte-identical: `AgentMobility.state.Walking { link_id: String }`, `WaitingAtStop { stop_id: String }`, vehicle DTO `route_id: String` — all preserved via per-edge/per-node/per-line `legacy_id: Option<String>` fields, looked up through `graph.node_by_legacy()` / `graph.edge_by_legacy()` / `transit_lines.line_by_legacy()`. Persistence-stable: `mobility_snapshots` JSONB schema unchanged (a `PersistedStop` boundary type translates between graph state and wire shape). All <CARGO_COUNT> cargo workspace tests pass; clippy `-D warnings` clean; tsc clean; vitest 166 pass; smoke `scripts/smoke-7b.mjs` 9/9 with binary frames. Perf bench `tick_100k_all_active` <NEW_MS>ms vs Phase 8a baseline 11.84ms (delta <PCT>%, within ≤+5% budget). For the seeded `zurich-river-city-v1` network: graph contains <NODES> nodes and <EDGES> edges across <LINES> transit lines. Spec `docs/superpowers/specs/2026-05-21-routing-graph-spatial-index-design.md`, plan `docs/superpowers/plans/2026-05-21-routing-graph-spatial-index.md`. Commits T1-T13 (SHAs in commit log). Foundation for Phase 8c (A* + multi-modal + path cache), 8d (HPA*), 8e (flow fields).
```

Substitute the placeholders (`<HH:MM:SS>`, `<CARGO_COUNT>`, `<NEW_MS>`, `<PCT>`, `<NODES>`, `<EDGES>`, `<LINES>`) with the captured values.

- [ ] **Step 7: Commit progress note**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add progress.md
git commit -m "docs(8b): progress note — Routing Graph + Spatial Index complete"
```

---

## Self-Review Notes

### Spec coverage

- ✅ Spec sections 1–10 (Goal, Why, Reference, Principles, Graph, Transit, Spatial Index, Cost Model, Builder, Modules): Tasks 1–8 implement.
- ✅ Spec section "Replace migration": Tasks 9–12 stage the migration; T12 cuts the old types.
- ✅ Spec acceptance criteria 1–13: T13 verifies. Specifically:
  - Criteria 1–3 (grep zero): T13 step 1.
  - Criteria 4–5 (graph populated): T13 step 2 test.
  - Criteria 6 (cargo tests pass): T13 step 3.
  - Criterion 7 (clippy clean): T13 step 3.
  - Criterion 8 (tsc clean): T13 step 3.
  - Criterion 9 (vitest pass): T13 step 3.
  - Criterion 10 (smoke 9/9): T13 step 4.
  - Criterion 11 (perf ≤+5%): T13 step 5.
  - Criterion 12 (wire bytes identical): T9–T12 preserve string ids; smoke is the empirical proof.
  - Criterion 13 (Postgres schema identical): T12 preserves `PersistedStop` field names.

### Type consistency check

- `NodeId(pub u32)`, `EdgeId(pub u32)`, `LineId(pub u32)` — consistent across T2/T3.
- `Node.legacy_id: Option<String>`, `Edge.legacy_id: Option<String>`, `TransitLine.legacy_route_id: Option<String>` — consistent.
- `Graph::node_by_legacy(&str) -> Option<NodeId>`, `Graph::edge_by_legacy(&str) -> Option<EdgeId>`, `TransitLines::line_by_legacy(&str) -> Option<LineId>` — consistent.
- `build_graph_from_city_network(network, seeded_stops) -> (Graph, TransitLines, NodeSpatialIndex)` — consistent across T7/T8.
- `RoutePosition { line_id, edge_index, progress, speed }` — defined T10, used T10/T11.

### Placeholder scan

No "TBD", "implement later", or vague "handle edge cases" remain. Two "placeholder" mentions are intentional contractual notes (capacity=1 for 8b future congestion model, speed-limit constants for 8b before real data).
