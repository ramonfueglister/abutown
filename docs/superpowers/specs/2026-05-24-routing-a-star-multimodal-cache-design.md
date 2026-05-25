# Phase 8c - A* Routing + Multi-Modal Profiles + Path Cache

**Date:** 2026-05-24
**Status:** Design
**Phase in roadmap:** 8c (pathfinding layer on the Phase 8b routing graph)

## Goal

Add a deterministic routing engine on top of the Phase 8b `routing::Graph`: A* pathfinding, mode-aware profiles, coordinate-to-node planning through `NodeSpatialIndex`, and a bounded `PathCache` resource. This phase makes routing a reusable backend service for mobility, LOD promotion, future HPA*, and flow fields without changing wire bytes or persistence bytes.

8c is an engine-first phase. It does not migrate every live agent plan to dynamic multi-edge execution. The existing `AgentMobilityState` and `PlanStage` wire/storage shapes remain string-keyed and byte-compatible. Integration is limited to safe call sites that can consume a planned path without changing runtime semantics: tests, cache warming, seed validation, and LOD promotion helpers that already resolve graph edges.

## Why now

Phase 8b gave the simulation one graph substrate, but mobility still uses authored plan strings. The next blocker is not another data migration; it is a real route query API that can answer:

1. Which graph edges connect node A to node B for a walking agent?
2. Which path is legal for a car, and which edges are forbidden?
3. Which path can combine walking and tram travel without allowing boarding at arbitrary intersections?
4. How do we avoid recomputing the same route for thousands of agents?

8c answers those questions while keeping the behavior surface narrow.

## Scope

In scope:

- A* over `routing::Graph::outgoing`.
- Deterministic tie-breaking for equal-cost candidates.
- Euclidean heuristic from node positions.
- Mode profiles for `Walk`, `Car`, `Tram`, and `WalkTransit`.
- Transition rules for `WalkTransit`: boarding and alighting are legal only at `NodeKind::TransitStop`.
- Transfer penalties for entering or leaving tram mode.
- Coordinate planning via `NodeSpatialIndex::nearest`.
- A bounded `PathCache` resource keyed by origin, destination, profile, and graph generation.
- A `PathfindingPlugin` that installs routing resources but no behavior-changing systems.
- Unit tests on hand-authored graphs and one runtime integration test on the seeded Zurich graph.

Out of scope:

- No proto changes.
- No JSONB snapshot shape changes.
- No new frontend state.
- No live re-routing when congestion, construction, or player commands change the graph.
- No timetable, headway, waiting-time, capacity, or vehicle-arrival model for transit.
- No HPA*, contraction hierarchies, flow fields, or parallel routing worker pool.
- No replacement of `AgentMobilityState::Walking { link_id, progress }` with a multi-edge execution state.

## Design choice

Use the engine-first approach.

### Option A: Engine-first routing layer

Add pathfinding, mode profiles, and caching as reusable `sim_core::routing` APIs. Keep mobility behavior stable. Consume the engine in targeted places where the existing state model can safely use it.

This is the selected approach. It keeps 8c testable, preserves Phase 8b's zero-fallback stance, and gives later phases a clean API.

### Option B: Full mobility migration

Generate every agent plan dynamically and introduce a multi-edge walking execution state immediately.

This is too much blast radius for 8c. It touches persistence, plan cursors, wire compatibility, LOD promote/demote logic, snapshot hydration, and benchmark budgets in one phase.

### Option C: A* only, no cache or profiles

Implement a shortest-path function and defer mode legality/caching.

This is too shallow. It would be easy to merge but would not answer the 100k-agent scaling problem and would force the first real consumer to redesign the API.

## Public API

8c adds focused modules under `backend/crates/sim-core/src/routing/`:

```text
routing/
  pathfinding.rs
  path_cache.rs
  profile.rs
```

`routing/mod.rs` re-exports only the stable surface.

```rust
pub use path_cache::{PathCache, PathCacheKey, PathCacheStats};
pub use pathfinding::{
    AStarRouter, PathEdge, PathRequest, PlannedPath, RoutingError,
};
pub use plugin::PathfindingPlugin;
pub use profile::{ModeState, RoutingProfile, RoutingProfileKey};
```

### Routing profiles

`RoutingProfileKey` is the compact cache key and equality surface:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoutingProfileKey {
    Walk,
    Car,
    Tram,
    WalkTransit,
}
```

`RoutingProfile` holds the tunable costs:

```rust
#[derive(Debug, Clone, Copy)]
pub struct RoutingProfile {
    pub key: RoutingProfileKey,
    pub walk_speed: f32,
    pub car_speed_factor: f32,
    pub tram_speed_factor: f32,
    pub board_tram_penalty: f32,
    pub alight_tram_penalty: f32,
}
```

Defaults:

- `Walk`: `Footway` only, cost `edge.length / walk_speed`.
- `Car`: `Road` only, cost `edge.length / (edge.speed_limit * car_speed_factor)`.
- `Tram`: `TramTrack` only, cost `edge.length / (edge.speed_limit * tram_speed_factor)`.
- `WalkTransit`: `Footway` and `TramTrack`, with stateful transition penalties.

`WalkTransit` does not model vehicle schedules. It is a network-legality profile: walking to a stop, riding tram edges between stops, and walking away from a stop.

### Mode state

A* state includes both node and mode:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModeState {
    Walking,
    Driving,
    OnTram,
}
```

Why stateful search: the legality and cost of a `TramTrack` edge depends on whether the path is boarding, staying on the tram, or illegally entering from a non-stop intersection. Node-only A* cannot express that without hidden fallbacks.

### Path request

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PathRequest {
    pub from: NodeId,
    pub to: NodeId,
    pub profile: RoutingProfileKey,
}
```

Coordinate planning is a separate helper:

```rust
pub fn request_between_points(
    index: &NodeSpatialIndex,
    from: (f32, f32),
    to: (f32, f32),
    profile: RoutingProfileKey,
) -> Result<PathRequest, RoutingError>;
```

It returns an error if either endpoint cannot resolve to a graph node. It never falls back to `(0, 0)` or an arbitrary edge.

### Planned path

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedPath {
    pub from: NodeId,
    pub to: NodeId,
    pub profile: RoutingProfileKey,
    pub edges: Vec<PathEdge>,
    pub total_cost: f32,
    pub total_length: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathEdge {
    pub edge_id: EdgeId,
    pub mode: ModeState,
    pub cost: f32,
}
```

Empty path semantics: `from == to` returns an empty `edges` vector with zero cost and zero length.

### Errors

```rust
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
```

Errors are explicit. There are no compatibility shims, magic defaults, or best-effort path strings.

## A* algorithm

The router is stateless:

```rust
pub struct AStarRouter;

impl AStarRouter {
    pub fn find_path(
        graph: &Graph,
        request: PathRequest,
        profile: RoutingProfile,
    ) -> Result<PlannedPath, RoutingError>;
}
```

Search node:

```rust
struct SearchState {
    node: NodeId,
    mode: ModeState,
}
```

Priority queue ordering:

1. Lowest estimated total cost (`g + h`).
2. Lowest known cost (`g`) if estimate ties.
3. Lowest `NodeId`.
4. Lowest `ModeState` ordinal.

Tie-breaking is intentional. It keeps unit tests stable and avoids nondeterministic path choices across platforms.

Heuristic:

- Euclidean distance from current node to target node.
- Divided by the fastest speed the profile can legally use.
- This keeps the heuristic admissible for positive edge costs.

Edge expansion:

- `Walk`: accept `EdgeKind::Footway`, next mode `Walking`.
- `Car`: accept `EdgeKind::Road`, next mode `Driving`.
- `Tram`: accept `EdgeKind::TramTrack`, next mode `OnTram`.
- `WalkTransit`:
  - `Footway` is always legal from `Walking`.
  - `Footway` from `OnTram` is legal only if the current node is a transit stop; add `alight_tram_penalty`.
  - `TramTrack` from `Walking` is legal only if the current node is a transit stop; add `board_tram_penalty`.
  - `TramTrack` from `OnTram` is legal.
  - `Road` is illegal.

All accepted edges must have finite positive cost. Zero-length edges are legal only when the computed cost is finite and non-negative; negative costs are rejected with `InvalidGraph`.

## Path cache

`PathCache` is a Bevy resource:

```rust
#[derive(Resource)]
pub struct PathCache {
    capacity: usize,
    graph_generation: u64,
    entries: HashMap<PathCacheKey, Arc<PlannedPath>>,
    order: VecDeque<PathCacheKey>,
    stats: PathCacheStats,
}
```

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PathCacheKey {
    pub from: NodeId,
    pub to: NodeId,
    pub profile: RoutingProfileKey,
    pub graph_generation: u64,
}
```

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PathCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub evictions: u64,
}
```

`PathCache::get_or_plan` checks the cache first, then calls `AStarRouter::find_path`, then inserts successful paths. `NoPath` results are not cached in 8c; caching failures can mask graph fixes during development.

Eviction is insertion-order bounded eviction, not a full LRU. This is enough for 8c because graph routes are immutable and consumers batch many identical OD requests. A future phase can replace the internal policy without changing `PathCacheKey` or `get_or_plan`.

Graph generation is `0` in 8c because the graph is immutable post-build. It exists in the key now so later dynamic graph changes can invalidate cache entries without API churn.

## Plugin integration

`PathfindingPlugin` follows the 8a/8b plugin pattern:

```rust
pub struct PathfindingPlugin {
    pub cache_capacity: usize,
}
```

Install behavior:

- Insert `PathCache::with_capacity(cache_capacity)`.
- Do not install tick systems.
- Do not mutate `Graph`, `TransitLines`, or `NodeSpatialIndex`.
- Do not require frontend or server code changes beyond runtime resource installation.

Runtime installation order:

1. `CorePlugin`
2. `RoutingPlugin`
3. `PathfindingPlugin`
4. `MobilityPlugin`
5. `PersistencePlugin`

The plugin can tolerate tests that do not install `RoutingPlugin`; it only inserts its own cache resource. Actual route calls return explicit errors if graph inputs are invalid.

## Mobility integration boundary

8c does not add a new persisted walking state. That means an arbitrary multi-edge `PlannedPath` cannot yet be executed by existing agents. This is deliberate.

Allowed consumers in 8c:

- `mobility::seed` can call the router in tests to verify that seeded stop/activity pairs are connected.
- LOD promotion helpers can use cached path lookup only when the resolved path has a single legacy `Footway` edge compatible with the existing `PlanStage` shape.
- Runtime tests can assert the Zurich graph has legal walk, car, tram, and walk-transit paths.

Disallowed consumers in 8c:

- No broad replacement of `WalkPlan.stages`.
- No new `AgentMobilityState` variant.
- No plan cursor semantics change.
- No serialization of `PlannedPath`.

This keeps 8c small enough to verify rigorously. The later "multi-edge mobility execution" phase can add an internal route-following component and decide whether it remains transient or becomes persisted.

## Testing

Unit tests:

- Empty graph returns `MissingNode` or `NoNearestNode`.
- `from == to` returns an empty path.
- Walk profile rejects road and tram edges.
- Car profile rejects footway and tram edges.
- Tram profile rejects footway and road edges.
- WalkTransit boards only at `TransitStop`.
- WalkTransit can walk to a stop, ride tram edges, and walk away from a stop.
- A* picks the lower total cost over a shorter but slower/penalized path.
- Tie-breaking is deterministic.
- Cache hit/miss/insert/eviction counters are exact.
- Cache keys distinguish profile and graph generation.

Integration tests:

- Seeded Zurich runtime has `PathCache` after runtime construction.
- Seeded Zurich graph can produce at least one path for each profile that is expected to be connected.
- No route query returns a fake `(0, 0)` position or synthetic legacy link id.

Verification commands:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::pathfinding routing::path_cache -- --nocapture
cargo test -p sim-server runtime_has_pathfinding_resources -- --nocapture
cargo test --workspace -- --nocapture
cargo clippy --workspace --all-targets -- -D warnings
```

Frontend verification stays unchanged:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
./node_modules/.bin/tsc --noEmit --pretty false
./node_modules/.bin/vitest run --passWithNoTests --reporter=dot --pool=forks --fileParallelism=false
```

Smoke and perf gates remain:

```bash
node scripts/smoke-7b.mjs
cd backend && cargo bench -p sim-core tick_100k_all_active
```

## Acceptance criteria

8c is done when all of the following hold:

1. `routing::AStarRouter` returns deterministic `PlannedPath` values for hand-authored graphs.
2. `RoutingProfileKey::{Walk, Car, Tram, WalkTransit}` enforce edge legality with no fallback edges.
3. `WalkTransit` cannot board or alight at non-stop intersections.
4. `request_between_points` returns `RoutingError::NoNearestNode` instead of fabricating coordinates when the index is empty.
5. `PathCache` caches successful paths, tracks exact stats, and evicts at capacity.
6. `PathfindingPlugin` installs `PathCache` in runtime construction.
7. Existing wire protocol and proto files are unchanged.
8. Existing `mobility_snapshots` JSONB shape is unchanged.
9. All workspace cargo tests pass.
10. Clippy `-D warnings` is clean.
11. tsc and Vitest are clean.
12. Browser smoke remains 9/9 green with binary frames.
13. `tick_100k_all_active` stays within +5% of the Phase 8b median baseline of 11.773 ms, budget 12.362 ms.

## Risks

- **Footway connectivity is sparse.** Current footway edges are seeded corridors and are not split at intersections. Some arbitrary OD pairs may have no walking path. This is acceptable in 8c; the router must return `NoPath`, not invent a route.

- **Transit realism is limited.** `WalkTransit` models network legality, not schedules. That is still useful for route planning and keeps simulation behavior stable.

- **Stateful A* is more complex than node-only A*.** The complexity is justified because transit boarding rules are otherwise impossible to express cleanly.

- **Cache policy could be too simple.** Insertion-order eviction is intentionally replaceable. The key shape and caller API are the important 8c contract.

- **Full mobility execution remains future work.** This is an explicit scope line, not a miss. The next phase can build on `PlannedPath` without reworking routing.

## Open questions

None. The design intentionally chooses the narrow engine-first implementation and leaves multi-edge agent execution for a later phase.
