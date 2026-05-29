# Phase 8d - Hierarchical Routing HPA*

**Date:** 2026-05-26
**Status:** Design
**Phase in roadmap:** 8d (hierarchical routing layer on top of 8c A* and cache)

## Goal

Add a deterministic hierarchical routing layer over the Phase 8c `routing::Graph`: cluster indexing, portal discovery, abstract cluster search, and corridor-constrained pathfinding. The result is a reusable HPA* substrate for large agent batches without changing wire bytes, persistence bytes, frontend state, or live mobility execution.

8d is still an engine-first phase. It does not replace `AgentMobilityState`, does not create dynamic multi-edge agent plans, and does not make live agents re-route. It gives later mobility, flow-field, and domain-tile phases a scalable route query API that can plan through a small cluster corridor instead of scanning the whole graph.

## Why now

Phase 8c added exact A* and a bounded path cache. Exact A* is correct, but it still has whole-graph search behavior on every cache miss. The next scaling layer is a graph abstraction that answers:

1. Which spatial clusters does a long route need to cross?
2. Which graph nodes act as cluster portals?
3. Can a route be planned through a deterministic corridor instead of the whole graph?
4. Can the hierarchical layer fail explicitly when the abstraction is insufficient, without silently falling back to a different algorithm?

8d answers those questions while preserving the zero-fallback policy established in 8b and 8c.

## Scope

In scope:

- Fixed-size spatial clusters derived from existing graph node positions.
- Deterministic `ClusterCoord` and `ClusterId` assignment.
- Portal detection from graph edges whose endpoints live in different clusters.
- A profile-aware cluster adjacency builder for `Walk`, `Car`, `Tram`, and `WalkTransit`.
- A hierarchical router that:
  - uses exact A* for same-cluster requests as the defined base case,
  - runs abstract cluster A* for cross-cluster requests,
  - runs exact A* constrained to the selected cluster corridor,
  - returns explicit hierarchical errors when no cluster path or corridor path exists.
- Optional extra search margin around the abstract cluster path, configured by `HpaConfig`.
- `HierarchicalRoutingPlugin` that installs `HpaIndex` from the existing `Graph`.
- Unit tests on hand-authored graphs and runtime integration tests on the seeded Zurich graph.
- Acceptance greps proving no fallback or synthetic path language landed.

Out of scope:

- No proto changes.
- No JSONB snapshot changes.
- No frontend changes.
- No live agent execution migration.
- No route invalidation from player road edits or graph mutations.
- No precomputed all-pairs portal path table.
- No contraction hierarchies.
- No flow-field integration. 8e owns flow fields.
- No parallel routing worker pool.
- No route optimality guarantee equal to global exact A*. 8d guarantees deterministic, legal routes through the selected corridor.

## Design choice

Use corridor-HPA*.

### Option A: Corridor-HPA* over fixed clusters

Build a deterministic cluster index from graph coordinates, search an abstract cluster graph, then run existing exact A* constrained to the cluster corridor. This is the selected approach. It is smaller than full HPA*, profile-aware enough for current modes, and gives a meaningful search-space reduction without introducing a second path representation.

### Option B: Full portal-to-portal HPA* with precomputed segment paths

Precompute portal-pair shortest paths per cluster and stitch those paths at query time. This is faster at runtime but too much blast radius for 8d because `WalkTransit` has stateful mode transitions and portal-pair precomputation would need profile-specific state handling.

### Option C: Cluster index only

Only build clusters and portals, then defer routing. This is safe but too shallow; it would not prove that the hierarchy can actually return legal `PlannedPath` values.

## Public API

8d adds one focused module under `backend/crates/sim-core/src/routing/`:

```text
routing/
  hpa.rs
```

`routing/mod.rs` re-exports the stable surface:

```rust
pub use hpa::{
    ClusterCoord, ClusterId, HierarchicalRoutingError, HierarchicalRoutingPlugin,
    HpaConfig, HpaIndex, HpaRouteStats, HpaRouter,
};
```

### Configuration

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HpaConfig {
    pub cluster_size_tiles: u16,
    pub corridor_margin_clusters: u16,
}
```

Defaults:

- `cluster_size_tiles = 32`, matching the existing chunk size and current Zurich world partition.
- `corridor_margin_clusters = 0`, so the first implementation searches exactly the abstract cluster path.

`HpaConfig::default()` is the production config. Tests may use smaller cluster sizes to create compact fixtures.

Validation:

- `cluster_size_tiles` must be greater than zero.
- `corridor_margin_clusters` is allowed to be zero.
- Invalid config is rejected during index construction with `HierarchicalRoutingError::InvalidConfig`.

### Cluster identity

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClusterCoord {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClusterId(pub u32);
```

`ClusterCoord` is derived from node position using Euclidean grid buckets:

```rust
pub fn cluster_coord_for(position: (f32, f32), config: HpaConfig) -> ClusterCoord;
```

The conversion uses `floor(position / cluster_size_tiles)` so negative coordinates remain deterministic. `ClusterId` is assigned by sorting all occupied `ClusterCoord` values lexicographically.

### HPA index

```rust
#[derive(Resource)]
pub struct HpaIndex {
    pub config: HpaConfig,
    // private deterministic indexes
}
```

Public methods:

```rust
impl HpaIndex {
    pub fn build(graph: &Graph, config: HpaConfig) -> Result<Self, HierarchicalRoutingError>;
    pub fn cluster_count(&self) -> usize;
    pub fn portal_count(&self) -> usize;
    pub fn cluster_of_node(&self, node: NodeId) -> Option<ClusterId>;
    pub fn portals_in_cluster(&self, cluster: ClusterId) -> &[NodeId];
    pub fn cluster_coord(&self, cluster: ClusterId) -> ClusterCoord;
    pub fn cluster_id(&self, coord: ClusterCoord) -> Option<ClusterId>;
}
```

The index stores:

- Node-to-cluster mapping.
- Cluster-to-node members.
- Cluster-to-portal nodes.
- Profile-aware cluster adjacency summaries.
- Portal metadata for diagnostics and tests.

The index does not store planned paths. Planned paths remain `PlannedPath` from 8c.

## Portal detection

A node is a portal when at least one outgoing edge crosses from its cluster to another cluster. The opposite endpoint is also a portal for its own cluster.

Detection rules:

- Edges with invalid node ids make index build fail with `InvalidGraph`.
- Edges whose endpoints are in the same cluster do not create portals.
- Portal lists are sorted by `NodeId` and deduplicated.
- Empty graphs produce an empty `HpaIndex`; routing against it returns explicit route errors.

Profile-aware adjacency:

- `Walk` adjacency includes only cross-cluster `Footway` edges.
- `Car` adjacency includes only cross-cluster `Road` edges.
- `Tram` adjacency includes only cross-cluster `TramTrack` edges.
- `WalkTransit` adjacency includes:
  - cross-cluster `Footway` edges,
  - cross-cluster `TramTrack` edges when at least one endpoint is a `TransitStop`,
  - cluster transitions that may later be validated by the exact corridor search.

The abstract adjacency is a coarse guide, not final legality. Final legality is enforced by 8c `RoutingProfile::transition` during corridor-constrained exact search.

## Routing algorithm

`HpaRouter` exposes:

```rust
pub struct HpaRouter;

impl HpaRouter {
    pub fn find_path(
        graph: &Graph,
        index: &HpaIndex,
        request: PathRequest,
        profile: RoutingProfile,
    ) -> Result<(PlannedPath, HpaRouteStats), HierarchicalRoutingError>;
}
```

`HpaRouteStats` gives measurable evidence that the hierarchy was used:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HpaRouteStats {
    pub start_cluster: ClusterId,
    pub goal_cluster: ClusterId,
    pub abstract_clusters_visited: usize,
    pub corridor_cluster_count: usize,
    pub used_base_case: bool,
}
```

### Same-cluster base case

If `from` and `to` are in the same cluster, `HpaRouter` runs exact A* with a constraint that allows only that cluster. This is not a fallback. It is the defined base case for hierarchical routing, and `HpaRouteStats.used_base_case = true`.

If the constrained exact search fails, the route returns `HierarchicalRoutingError::NoCorridorPath`.

### Cross-cluster route

For different start and goal clusters:

1. Validate that both request nodes exist in `Graph`.
2. Resolve both nodes to `ClusterId`.
3. Run abstract A* over cluster adjacency for the request profile.
4. Expand the abstract cluster path by `corridor_margin_clusters`.
5. Run exact A* over the original graph constrained to edges whose endpoints are both inside the corridor cluster set.
6. Return the resulting `PlannedPath` and `HpaRouteStats`.

The final path is always produced by the 8c A* legality model. HPA* only narrows the candidate graph.

## Pathfinding extension

8d extends `AStarRouter` with a constrained search entry point:

```rust
pub trait EdgeConstraint {
    fn allows(&self, graph: &Graph, edge: &Edge) -> bool;
}

pub struct AllEdges;

impl AStarRouter {
    pub fn find_path_with_constraint<C: EdgeConstraint>(
        graph: &Graph,
        request: PathRequest,
        profile: RoutingProfile,
        constraint: &C,
    ) -> Result<PlannedPath, RoutingError>;
}
```

The existing `AStarRouter::find_path` delegates to `find_path_with_constraint` using `AllEdges`, so 8c behavior remains unchanged.

The HPA module provides a private `ClusterCorridorConstraint` that allows an edge only if both endpoint clusters are in the selected corridor.

## Error handling

```rust
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
```

Rules:

- No error path calls unconstrained global A* as a fallback.
- No error path creates synthetic nodes, synthetic edges, or `(0, 0)` coordinates.
- `RoutingError::NoPath` from the constrained search is mapped to `NoCorridorPath`.
- Other `RoutingError` values are wrapped as `Exact`.

## Plugin integration

Add:

```rust
pub struct HierarchicalRoutingPlugin {
    pub config: HpaConfig,
}
```

Install order in `SimulationRuntime`:

1. `RoutingPlugin`
2. `PathfindingPlugin`
3. `HierarchicalRoutingPlugin`
4. `MobilityPlugin`

The plugin reads the existing `Graph` resource and inserts `HpaIndex`. It registers no behavior-changing systems. If the graph is empty, it installs an empty index so test and no-network startup paths stay explicit and deterministic.

## Testing strategy

Unit tests in `routing/hpa.rs`:

1. `cluster_coord_uses_floor_division` covers positive and negative positions.
2. `index_assigns_deterministic_cluster_ids` verifies sorted coord assignment.
3. `index_detects_cross_cluster_portals` verifies portal nodes from crossing edges.
4. `profile_adjacency_filters_edge_kinds` verifies Walk, Car, Tram cluster edges.
5. `same_cluster_route_uses_base_case` verifies constrained exact route and stats.
6. `cross_cluster_route_uses_corridor` verifies abstract search plus exact corridor path.
7. `no_cluster_path_is_not_replanned_globally` verifies an explicit `NoClusterPath`.
8. `no_corridor_path_is_not_replanned_globally` verifies an explicit `NoCorridorPath`.

Unit tests in `routing/pathfinding.rs`:

1. `edge_constraint_blocks_disallowed_edges` proves `find_path_with_constraint` respects constraints.
2. `unconstrained_find_path_keeps_existing_behavior` proves the original API still works.

Plugin tests in `routing/plugin.rs`:

1. `hierarchical_routing_plugin_installs_hpa_index`.
2. `hierarchical_routing_plugin_accepts_empty_graph`.

Runtime tests in `sim-server/src/runtime.rs`:

1. `runtime_installs_hpa_index_for_seeded_graph`.
2. `runtime_can_find_seeded_hierarchical_path`.

Verification commands:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::hpa -- --nocapture
cargo test -p sim-core routing::pathfinding -- --nocapture
cargo test -p sim-server runtime_ -- --nocapture
cargo test --workspace -- --nocapture
cargo clippy --workspace --all-targets -- -D warnings
```

Frontend verification remains required even though no frontend files change:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
./node_modules/.bin/tsc --noEmit --pretty false
./node_modules/.bin/vitest run --passWithNoTests --reporter=dot --pool=forks --fileParallelism=false
```

Smoke and perf:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
node scripts/smoke-7b.mjs
cd backend && cargo bench -p sim-core tick_100k_all_active
```

Perf acceptance: `tick_100k_all_active` must stay within the Phase 8c median +5% budget. Phase 8c median was 11.047 ms, so the 8d budget is 11.599 ms on this host.

## Acceptance criteria

1. `HpaIndex` is deterministic for identical graph input.
2. Portal detection is based only on real graph edges.
3. HPA route results are legal `PlannedPath` values produced by the 8c profile transition model.
4. Same-cluster routing uses the constrained base case.
5. Cross-cluster routing uses abstract cluster search before exact corridor search.
6. No hierarchical failure path calls unconstrained global A*.
7. No synthetic path, synthetic coordinate, or fake edge id is introduced.
8. No proto files change.
9. No mobility persistence files change.
10. No frontend files change.
11. Full backend tests pass.
12. Clippy is clean with `-D warnings`.
13. TypeScript and Vitest pass.
14. Browser smoke remains 9/9 green with binary frames.
15. `tick_100k_all_active` remains inside the 8d perf budget.

Acceptance grep:

```bash
rg -n "fallback|synthetic|unwrap_or\\(\\(0\\.0, 0\\.0\\)\\)|global A\\* fallback|fake edge|fake node" \
  backend/crates/sim-core/src/routing backend/crates/sim-server/src/runtime.rs
```

Expected: no matches in routing implementation. If comments need to discuss forbidden behavior, they must use precise wording such as "returns an explicit error" rather than "fallback".

## Risks

### Corridor misses a valid exact path

The abstract cluster path can choose a corridor that excludes a valid exact path. This is acceptable in 8d because the contract is hierarchical routing, not global optimal routing. The error is explicit (`NoCorridorPath`), and callers can choose whether to widen `corridor_margin_clusters` in future phases.

### WalkTransit abstraction is coarse

`WalkTransit` is stateful. Cluster adjacency can only approximate boarding legality; exact legality remains in the final constrained A* pass. This keeps the index simple and prevents profile-specific portal precomputation from becoming the real project in 8d.

### Current graph has sparse pedestrian topology

Phase 8b intentionally preserved the seeded graph shape. HPA tests must include compact hand-authored graphs so correctness does not depend on the seeded Zurich graph being fully connected across every mode.

## Non-goals for later phases

8d does not decide the shape of 8e flow fields. It only provides a cluster/corridor abstraction that 8e can reuse if useful. 8e may choose to build flow fields per cluster, per destination group, or per activity domain without changing the 8d API.

8d also does not migrate live agents. The first live mobility consumer should be a separate phase with its own persistence and execution-state design.
