# Phase 8b — Routing Graph + Spatial Index + Cost Model

**Date:** 2026-05-21
**Status:** Design
**Phase in roadmap:** 8b (routing substrate for 8c A*, 8d HPA*, 8e flow fields)

## Goal

Replace the polyline-soup `CityNetwork` + the seeded `Routes`/`Stops`/`LinkPolylines` resources with a proper routing graph (Nodes + Edges) and a spatial index, behind a `CostModel` trait. After 8b the mobility code queries one substrate; 8c A*, 8d HPA*, and 8e flow fields hang off the same data structure with no foundation changes.

This phase ships **no pathfinding** — only the data structure and the migration. 8c implements A*.

## Why now

The current world has three problems blocking the routing roadmap:

1. **No graph.** `data/city/zurich-network.json` is two polyline collections (`arterial_paths`, `pedestrian_corridors`). There is no connectivity, no edge metadata, no intersection set. A* has nothing to run on.

2. **Three parallel truths for the same data.** `Routes`, `Stops`, `LinkPolylines` are three separate resources that together describe the network. They were authored manually in `mobility/seed.rs` and have no derived relationship to `CityNetwork`. Any change ripples to all three.

3. **No spatial query path.** "Find the nearest stop to this world coord" today is a linear scan over `Stops`. At 100k agents and 10k stops, that is 1B operations per tick — the cost of every replanning event would dominate the budget.

Phase 8b fixes all three. After it, 8c A* is a pure pathfinding system on top of clean substrate.

## Reference models

- **`rstar` crate** for the spatial index. Active, well-benchmarked, supports nearest-neighbor + range queries + bulk-load.
- **`petgraph` was considered and rejected.** Petgraph is general-purpose, allocates `Vec<EdgeIndex>` per neighbor lookup, and forces double indirection. For 50k-node hot-path A* we want compact `Vec`-indexed nodes with cache-friendly adjacency.
- **Cities Skylines II road graph** for the dual-edge-per-segment pattern: roads carry both car-traffic Edge (`EdgeKind::Road`) and a parallel pedestrian Edge (`EdgeKind::Footway`) where applicable.
- **OpenStreetMap topology** for the "intersection-is-a-node, segment-between-intersections-is-an-edge" structural decision.

## Architectural principles (inherited from 8a)

1. **One substrate, no compat shims.** Old `Routes`/`Stops`/`LinkPolylines` resources are deleted; mobility queries the graph directly.
2. **Plugin composition.** New `RoutingPlugin: SimPlugin` registers everything. CorePlugin and MobilityPlugin stay untouched at the trait level; mobility gains internal calls into the routing module.
3. **Stable public API per module.** `routing/mod.rs` re-exports exactly the public surface; internals are `pub(crate)`.
4. **Wire stability.** WS protocol bytes do not change. Frontend keeps consuming `link_id: String`, `stop_id: String`. Internally these become string-encoded `EdgeId`/`NodeId`.
5. **No premature features.** No A*, no HPA*, no flow fields, no path cache, no dynamic re-routing. Substrate only.

## Graph data model

### Identifiers

```rust
#[derive(Component, Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct NodeId(pub u32);

#[derive(Component, Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct EdgeId(pub u32);

#[derive(Component, Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct LineId(pub u32);
```

`u32` indices into dense `Vec`s, not `HashMap` keys. Allocation-free lookup, cache-friendly iteration. Indices are stable for the lifetime of the graph (graph is built once at startup; if it ever rebuilds, callers re-resolve).

### Node

```rust
#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub position: (f32, f32),
    pub kind: NodeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Intersection,
    TransitStop,
    ActivityLocation,
}
```

Node carries its world position because (a) A* heuristic needs it, (b) the spatial index is built from it, (c) renderers need it. `NodeKind` is an exhaustive enum — adding a variant is a deliberate API change.

`ActivityLocation` is unused by 8b — it is the hook Phase 8g (Domain Tiles) attaches to. Including it now means 8g does not modify `NodeKind`.

### Edge

```rust
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    Footway,
    Road,
    TramTrack,
}
```

`polyline` includes both endpoint positions (so `polyline[0] == nodes[from].position` and `polyline[last] == nodes[to].position`). `length` is precomputed arc length so cost models do not re-walk geometry per query.

`capacity` is set to a constant 1 for 8b — a placeholder for the future congestion model.

Edges are **directed**. A bidirectional segment becomes two edges (`from → to` and `to → from`) that share polyline data. The builder generates both.

### Graph

```rust
pub struct Graph {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    outgoing: Vec<Vec<EdgeId>>,
    incoming: Vec<Vec<EdgeId>>,
}

impl Graph {
    pub fn node(&self, id: NodeId) -> &Node;
    pub fn edge(&self, id: EdgeId) -> &Edge;
    pub fn outgoing(&self, id: NodeId) -> &[EdgeId];
    pub fn incoming(&self, id: NodeId) -> &[EdgeId];
    pub fn nodes(&self) -> &[Node];
    pub fn edges(&self) -> &[Edge];
    pub fn node_count(&self) -> usize;
    pub fn edge_count(&self) -> usize;
}
```

Both `outgoing` and `incoming` are stored (they share `EdgeId`s, no edge duplication). Reverse search is on the 8d HPA* hot path; precomputing now is free.

The `Graph` is immutable post-build for 8b. Mutation hooks (add/remove edge for road construction in a future phase) are out of scope.

## Transit lines

Trams follow fixed routes — they do not pathfind. They need a separate sequenced edge list:

```rust
#[derive(Debug, Clone)]
pub struct TransitLine {
    pub id: LineId,
    pub name: String,
    pub edges: Vec<EdgeId>,
    pub stops: Vec<NodeId>,
}

#[derive(Resource, Default)]
pub struct TransitLines(Vec<TransitLine>);

impl TransitLines {
    pub fn line(&self, id: LineId) -> &TransitLine;
    pub fn iter(&self) -> impl Iterator<Item = &TransitLine>;
    pub fn count(&self) -> usize;
}
```

Vehicle `RoutePosition` is rewritten from `(RouteId, link_index, progress, speed)` to `(LineId, edge_index, progress, speed)`. Semantically identical, IDs are now graph-rooted.

## Spatial index

```rust
use rstar::{RTree, RTreeObject, AABB, PointDistance};

pub struct IndexedNode {
    pub id: NodeId,
    pub position: [f32; 2],
}

impl RTreeObject for IndexedNode {
    type Envelope = AABB<[f32; 2]>;
    fn envelope(&self) -> Self::Envelope { AABB::from_point(self.position) }
}

impl PointDistance for IndexedNode {
    fn distance_2(&self, point: &[f32; 2]) -> f32 {
        let dx = self.position[0] - point[0];
        let dy = self.position[1] - point[1];
        dx * dx + dy * dy
    }
}

#[derive(Resource)]
pub struct NodeSpatialIndex(RTree<IndexedNode>);

impl NodeSpatialIndex {
    pub fn new(nodes: &[Node]) -> Self {
        let indexed: Vec<IndexedNode> = nodes
            .iter()
            .map(|n| IndexedNode { id: n.id, position: [n.position.0, n.position.1] })
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
}
```

`rstar` because it is the actively-maintained Rust R-tree, supports bulk-load (faster initial build), and handles both nearest-neighbor and range queries through one structure.

## Cost model

```rust
pub trait CostModel: Send + Sync {
    /// Cost of traversing an edge. Return `f32::INFINITY` to disallow.
    fn cost(&self, edge: &Edge) -> f32;
}

pub struct DistanceCost;
impl CostModel for DistanceCost {
    fn cost(&self, edge: &Edge) -> f32 { edge.length }
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
```

Composition over inheritance. Phase 8c constructs `ModeFilterCost { inner: TimeCost, allowed: &[EdgeKind::Footway] }` for walking A*, etc.

The trait is `Send + Sync` so future systems can hold cost models behind `Arc<dyn CostModel>` for sharing across threads (parallel routing, eventually).

## Graph builder

```rust
pub fn build_graph_from_city_network(
    network: &CityNetwork,
) -> (Graph, TransitLines, NodeSpatialIndex) { ... }
```

Algorithm:

1. **Collect coords.** Walk every coord in `arterial_paths[*]` and `pedestrian_corridors[*]`. Count occurrences per coord. Note polyline endpoints.

2. **Identify nodes.** A coord is a node if it is (a) an endpoint of any polyline, OR (b) appears in ≥ 2 polylines (intersection), OR (c) listed as a stop in the seeded stop set (see step 6).

3. **Generate edges from arterials.** For each `arterial_paths[i]`:
   - Split the polyline at node-coords.
   - Each segment becomes two `Edge`s: one `EdgeKind::TramTrack` (used by `TransitLine`) and one `EdgeKind::Road` (used by cars + future foot-on-road).
   - Both directions: emit `from→to` and `to→from` edges.
   - `length` = sum of segment distances.
   - `speed_limit` = constant (tram: 4 tiles/tick, road: 6 tiles/tick) for 8b. Real data comes later.
   - `capacity` = 1 placeholder.

4. **Generate edges from pedestrian corridors.** Same as arterials but `EdgeKind::Footway`, no parallel road. `speed_limit` = 1 tile/tick (walking pace).

5. **Build adjacency.** Walk all edges once to populate `outgoing[from]` and `incoming[to]`.

6. **Resolve transit stops.** The legacy seeded stops (hardcoded in `mobility/seed.rs`) are mapped to the nearest existing node. If no node within 1 tile distance exists, panic with a clear error — the network is misconfigured.

7. **Build transit lines.** For each `arterial_paths[i]`, build a `TransitLine` with:
   - `name` = `"arterial_{i}"`
   - `edges` = the ordered `TramTrack` edges generated from this polyline
   - `stops` = the stop-node sequence along the polyline (filtered to nodes with kind=TransitStop)

8. **Bulk-load the spatial index** from `nodes`.

For the current `zurich-river-city-v1` network (260 arterials + 260 pedestrian corridors, ~520 polylines): expect ~2000 nodes, ~6000 edges. Builder runs in well under 100 ms once at startup.

## Module structure

```
backend/crates/sim-core/src/routing/
  mod.rs            re-exports (CostModel, Graph, TransitLines, NodeSpatialIndex,
                    Node, Edge, NodeId, EdgeId, LineId, NodeKind, EdgeKind,
                    TransitLine, DistanceCost, TimeCost, ModeFilterCost)
  graph.rs          Node, Edge, Graph, NodeId, EdgeId, NodeKind, EdgeKind
  transit.rs        LineId, TransitLine, TransitLines (resource)
  spatial_index.rs  IndexedNode, NodeSpatialIndex (resource)
  cost_model.rs     CostModel trait + 3 impls
  builder.rs        build_graph_from_city_network
  plugin.rs         RoutingPlugin: SimPlugin
```

`RoutingPlugin` registers `Graph`, `TransitLines`, `NodeSpatialIndex` as resources. It does **not** install any systems — the graph is read-only data for 8b. Future phases (8c, 8d, 8e) add systems via their own plugins.

`RoutingPlugin::install` requires `CityNetwork` resource to already be present (CorePlugin installs it). The plugin reads it, calls `build_graph_from_city_network`, inserts the three resources.

In `SimulationRuntime::new_with_event_store`, install order becomes:
```rust
CorePlugin::default().install(&mut world, &mut schedule);
RoutingPlugin.install(&mut world, &mut schedule);   // NEW
MobilityPlugin.install(&mut world, &mut schedule);
PersistencePlugin { world_id }.install(&mut world, &mut schedule);
```

`MobilityPlugin` install (and `mobility::seed`) now consumes `Graph`/`TransitLines` instead of building `Routes`/`Stops`/`LinkPolylines`.

## Replace migration

Delete:
- `mobility::resources::Routes`
- `mobility::resources::Stops`
- `mobility::resources::LinkPolylines`
- `mobility::records::RouteRecord`
- `mobility::records::StopRecord`
- `ids::RouteId`, `ids::LinkId`, `ids::StopId` — replaced by `routing::{LineId, EdgeId, NodeId}` (compat re-exports may be kept temporarily under `mobility` for a transitional commit)

Rewrite, in order:
- `mobility::seed::from_network` — consumes Graph + TransitLines instead of constructing Routes/Stops/LinkPolylines
- `mobility::api::*` accessors — `link_polyline()` becomes `edge_polyline()` taking `EdgeId`, etc.
- `mobility::components::RoutePosition` — fields change from `(RouteId, link_index)` to `(LineId, edge_index)`
- `mobility::systems::walk_advance_system` — reads `graph.edge(edge_id)` instead of `link_polylines.0.get(link_id)`
- `mobility::systems::vehicle_advance_system` — reads `transit_lines.line(line_id)` + `graph.edge(...)`
- `mobility::systems::boarding_alighting_system` — `waiting_agents` queue: a new `Resource<WaitingAgents>(HashMap<NodeId, VecDeque<AgentId>>)` (per-tick mutable state; not on the immutable Graph)
- `mobility::systems::stop_arrival_system` — reads `graph.node(node_id)` to confirm it is a TransitStop

Wire-stability:
- `AgentMobility.state.Walking { link_id: String }` keeps `String`. Internally `link_id` parses to `EdgeId` via a stable encoding (e.g., `"edge:42"`).
- `WaitingAtStop { stop_id: String }` likewise. Internal: `"node:17"`.
- The proto `.proto` file does not change in 8b.

Persistence-stability:
- `mobility_snapshots` JSONB stays string-keyed. `MobilityPersistSnapshot` carries strings; load/store helpers parse them at the boundary.

## Acceptance criteria

8b is "done" when all of the following hold:

1. `grep -rn 'pub struct \(Routes\|Stops\|LinkPolylines\)' backend/crates/` returns zero matches.
2. `grep -rn 'pub struct \(RouteRecord\|StopRecord\)' backend/crates/` returns zero matches.
3. `grep -rn 'pub struct \(RouteId\|LinkId\|StopId\)' backend/crates/sim-core/src/ids.rs` returns zero matches (newtypes deleted or re-exported from routing).
4. `routing::Graph`, `routing::TransitLines`, `routing::NodeSpatialIndex` resources are present in the world after runtime construction.
5. `Graph::node_count() > 0` and `Graph::edge_count() > 0` for the seeded `zurich-river-city-v1` network.
6. All workspace cargo tests pass (target: 205+ tests).
7. Clippy `-D warnings` clean.
8. tsc clean.
9. vitest 166+ pass.
10. Browser smoke `scripts/smoke-7b.mjs` 9/9 with binary frames — trams move, agents walk.
11. Perf bench `tick_100k_all_active` ≤ +5% vs Phase 8a baseline (11.84 ms ⇒ budget 12.43 ms).
12. Wire bytes byte-identical to Phase 8a — no proto schema changes.
13. Postgres `mobility_snapshots` JSONB schema byte-identical to Phase 8a.

## Risks

- **Graph builder correctness.** Misidentifying intersections (e.g., off-by-one in coord matching) silently produces disconnected components. Mitigation: builder emits a count summary at startup (`tracing::info!`), and a `debug_assert!(graph.connected_component_count() == 1)` (or a documented expectation if multiple components are legitimate).

- **Polyline-to-edges split correctness.** If a polyline crosses itself or passes through an intersection without sharing the exact coord, edges get the wrong topology. Mitigation: dedicated unit tests for the splitting algorithm with hand-authored polylines covering the edge cases (T-junction, X-junction, self-loop, terminal stub).

- **Wire-id string encoding stability.** Frontends compare `link_id` strings for tracking; changing the encoding ("link:walk:corridor:22" → "edge:42") breaks delta-application. Mitigation: keep the legacy string IDs intact on the wire by maintaining a `String → EdgeId` map and a `EdgeId → String` reverse for serialization. The graph stores both for stops/edges that map to legacy IDs.

- **Mobility-system rewrite churn.** Touches the four core systems (`walk_advance`, `vehicle_advance`, `boarding_alighting`, `stop_arrival`) plus seed.rs. High blast radius. Mitigation: each system migrated in its own commit; smoke test run after every system migration; revert any commit that drops smoke.

- **Transit-line construction.** Tram routes today are 2 hardcoded routes. After 8b, every arterial becomes a TransitLine, so ~260 lines (one per arterial). That is far more transit than today's seed expects. Mitigation: 8b emits TransitLines but does not change the *vehicle seeding*. Trams are still seeded on a fixed subset of lines (the original 2). Other 258 lines exist but have no vehicles. This is fine — empty lines are inert.

## Open questions

None. Implementation choices (exact string-encoding scheme for legacy IDs, exact speed_limit constants, whether `waiting_agents` lives as a Resource or as a per-stop-entity component) are deferred to the writing-plans phase, which has freedom within these constraints.
