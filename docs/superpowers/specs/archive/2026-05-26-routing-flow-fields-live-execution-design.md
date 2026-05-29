# Phase 8e - Flow Fields + Live Route Execution

**Date:** 2026-05-26
**Status:** Design
**Phase in roadmap:** 8e (batch routing and live mobility execution on top of 8c A* and 8d HPA*)

## Goal

Make the routing stack operational for live mobility at scale. Phase 8e adds reusable graph flow fields for many agents sharing the same destination or corridor, then wires walking agents to execute multi-edge planned routes from the graph instead of being limited to one legacy `link_id`.

This is the first routing phase that changes live agent execution. It remains deliberately narrow: walking execution becomes graph-backed and multi-edge; vehicles and transit boarding continue to use the existing route/stop machinery until a later phase can migrate them with the same rigor.

## Why now

Phase 8c added exact A* and a bounded path cache. Phase 8d added corridor-HPA* so long routes can be planned through deterministic cluster corridors. Those layers are still mostly query engines. The simulation still advances walking agents on `AgentMobilityState::Walking { link_id, progress }`, which means a live agent can only traverse one authored edge before its plan cursor changes.

8e closes that gap without taking on every mobility mode at once:

1. Many agents can consume a shared destination field instead of each running a full route query.
2. Walking agents can advance across multiple graph edges in one plan leg.
3. Persisted mobility state records enough route-execution data to survive restart.
4. Frontend rendering remains driven by server-computed `world_coord`, `direction`, and `sprite_key`.
5. Missing graph data fails explicitly instead of degrading to `(0, 0)`, empty activity state, synthetic link ids, or global-routing retries.

## Scope

In scope:

- New `routing::flow_field` module that builds reverse shortest-path fields over `Graph`.
- Profile-aware fields for at least `Walk` in production and test coverage for `Car` where the graph supports it.
- Corridor-constrained field construction using 8d HPA cluster corridors.
- `FlowFieldCache` Bevy resource keyed by destination node, profile, corridor identity, and graph generation.
- Route-execution types for agents:
  - `ActiveRoute` component for current graph path execution.
  - `RouteStep` records with `EdgeId`, traversal mode, edge length, and canonical edge key.
  - `RouteExecutionState` persisted alongside the existing mobility snapshot shape.
- Live walking execution over multiple graph edges.
- Seed integration that assigns route execution to walking agents when a graph path is resolvable.
- Snapshot extract/apply support for active route execution.
- Proto and TypeScript decoding updates only where needed to represent canonical graph walking state safely.
- Browser smoke and frontend tests proving backend-driven rendering still works.
- Acceptance greps for removed frontend/protocol fallback behavior and no routing fallback language.

Out of scope:

- No player road-edit invalidation.
- No dynamic congestion model.
- No timetable/headway model for transit.
- No vehicle execution migration from `RoutePosition` to arbitrary `PlannedPath`.
- No full replacement of `PlanStage::RideToStop`.
- No UI feature work.
- No global A* retry when HPA corridor or flow-field construction fails.
- No compatibility shim that invents route ids, link ids, stop ids, or coordinates.

## Design Choice

Use end-to-end walking execution with graph flow fields.

### Option A: Engine-only flow fields

Build fields and cache them, then stop before live mobility consumes them. This is low risk but does not satisfy the current goal because the app would still simulate walking through single legacy links.

### Option B: Flow fields plus controlled live walking consumer

Build the field engine, cache it, and migrate walking legs to multi-edge graph execution while leaving transit vehicles and boarding logic on the existing route/stop system. This is selected. It makes the routing stack real in the running app while keeping the blast radius bounded.

### Option C: Big-bang route migration

Move pedestrians, cars, trams, persisted records, wire DTOs, frontend state, route edits, and rerouting all at once. This is too broad for a single phase and would make correctness failures difficult to localize.

## Routing Flow Fields

8e adds:

```text
backend/crates/sim-core/src/routing/flow_field.rs
```

Stable API re-exported from `routing/mod.rs`:

```rust
pub use flow_field::{
    FlowField, FlowFieldCache, FlowFieldCacheKey, FlowFieldCacheStats,
    FlowFieldError, FlowFieldEntry, FlowFieldRouter,
};
```

### Flow Field Model

A flow field is a reverse shortest-path map to one destination:

```rust
pub struct FlowField {
    pub destination: NodeId,
    pub profile: RoutingProfileKey,
    entries: HashMap<(NodeId, ModeState), FlowFieldEntry>,
}

pub struct FlowFieldEntry {
    pub next_edge: Option<EdgeId>,
    pub next_mode: ModeState,
    pub cost_to_goal: f32,
}
```

The destination entry has `next_edge = None` and cost `0.0`. Every other reachable entry points to the next legal edge toward the destination.

Construction runs reverse Dijkstra from the destination over incoming graph edges. Legality uses the same `RoutingProfile` semantics as A*; when reverse traversal cannot infer a legal prior mode unambiguously, 8e enumerates the finite `ModeState` set and accepts only transitions that the forward profile would accept. This keeps the field behavior aligned with exact pathfinding and avoids a separate cost model.

### Corridor Constraint

Flow fields can be built in two modes:

```rust
pub enum FlowFieldScope {
    AllEdges,
    Corridor(HashSet<ClusterId>),
}
```

Production walking route assignment uses HPA-derived corridor scope for cross-cluster requests. Same-cluster requests use a one-cluster corridor, matching the 8d base-case semantics. Tests may build `AllEdges` fields only where the test is explicitly about raw field correctness.

If HPA cannot produce a cluster corridor, field construction returns `FlowFieldError::NoCorridor`. It does not retry globally.

### Errors

```rust
pub enum FlowFieldError {
    MissingNode(NodeId),
    MissingCluster(NodeId),
    NoCorridor { from: NodeId, to: NodeId, profile: RoutingProfileKey },
    Unreachable { from: NodeId, to: NodeId, profile: RoutingProfileKey },
    InvalidGraph(&'static str),
}
```

Errors are returned to callers and counted in route-assignment stats. They are not converted into dummy activity states or fallback links.

## Flow Field Cache

`FlowFieldCache` is a Bevy resource:

```rust
pub struct FlowFieldCacheKey {
    pub destination: NodeId,
    pub profile: RoutingProfileKey,
    pub graph_generation: u64,
    pub corridor_hash: u64,
}
```

The cache is bounded and insertion-order evicted, matching the existing `PathCache` style. `corridor_hash` is computed from sorted `ClusterId` values, so all agents sharing a destination and corridor reuse the same field.

Successful fields are cached. Failed fields are not cached in 8e, because graph and seed mistakes should remain visible during development.

## Live Walking Execution

8e introduces graph-route execution for agents without replacing every plan type.

### Components

New mobility components:

```rust
pub struct ActiveRoute {
    pub destination: NodeId,
    pub profile: RoutingProfileKey,
    pub steps: Vec<RouteStep>,
    pub cursor: usize,
}

pub struct RouteStep {
    pub edge_id: EdgeId,
    pub mode: ModeState,
    pub canonical_edge_key: String,
    pub length: f32,
}
```

`canonical_edge_key` is the stable external identifier for the step. If the graph edge has a legacy id, use it. Otherwise use `edge:<u32>`. This is not a fallback; it is the canonical id for graph-native edges.

### Agent State

`AgentMobilityState::Walking` remains the wire-visible walking state:

```rust
Walking { link_id: String, progress: f32 }
```

For graph-executed agents, `link_id` is the current `RouteStep.canonical_edge_key`, and `progress` is progress along that current edge. The authoritative multi-edge context lives in `ActiveRoute`.

When progress reaches `1.0`, `route_advance_system` moves to the next `RouteStep`, resets progress to `0.0`, and marks the agent dirty. When the route cursor reaches the destination, existing plan cursor semantics apply:

- `WalkToStop` transitions to `WaitingAtStop`.
- `WalkToActivity` transitions to `AtActivity`.
- Any missing expected destination returns an explicit route-execution error in tests; production does not silently continue.

### Route Assignment

Route assignment runs before `walk_advance_system`:

1. Find walking agents without `ActiveRoute`.
2. Resolve current graph node from current `Walking.link_id` and progress.
3. Resolve destination node from the current plan stage:
   - `WalkToStop` uses `Graph::node_by_legacy(stop_id)`.
   - `WalkToActivity` uses activity geometry and `NodeSpatialIndex::nearest`.
4. Ask HPA for the corridor.
5. Build or fetch a flow field for the destination and corridor.
6. Follow field entries from origin to destination to materialize `RouteStep`s.
7. Insert `ActiveRoute`.

If any resolution step fails, the agent stays in its current explicit state and route-assignment stats record the failure. It does not get teleported, parked in `AtActivity`, or assigned a synthetic link.

## Persistence

The existing `mobility_snapshots` table remains the durable row. The JSON payload is extended with route execution data. This is a JSONB payload evolution, not a table migration.

`AgentRecord` gains:

```rust
pub active_route: Option<PersistedActiveRoute>
```

`PersistedActiveRoute` stores:

```rust
pub struct PersistedActiveRoute {
    pub destination_node: u32,
    pub profile: RoutingProfileKey,
    pub cursor: usize,
    pub steps: Vec<PersistedRouteStep>,
}

pub struct PersistedRouteStep {
    pub edge_id: u32,
    pub mode: ModeState,
    pub canonical_edge_key: String,
    pub length: f32,
}
```

Serde uses `#[serde(default)]` for `active_route` so pre-8e snapshots hydrate. On hydration:

- Missing `active_route` is acceptable.
- Present `active_route` is validated against the current `Graph`.
- Invalid edge ids, mismatched canonical keys, missing nodes, or non-finite lengths cause hydration to fail explicitly.

Persisted `Walking.link_id` must resolve either as a graph legacy edge id or canonical `edge:<u32>`. Unresolvable walking links are errors, not defaults.

## Wire And Frontend

The frontend already renders authoritative `world_coord`, `direction`, and `sprite_key`, so most of 8e does not require new visual state.

Wire compatibility:

- `AgentMobilityStateDto::Walking` remains `{ link_id, progress }`.
- For graph-native edges, `link_id` can be `edge:<u32>`.
- A future diagnostic field may expose route cursor/length, but 8e does not need it for rendering.

Frontend cleanup:

- `agentStateFromProto` must not convert missing `AgentState` to `at_activity`.
- Missing `world_coord` must not become `(0, 0)` in accepted mobility frames.
- Invalid walking states must be decode errors in tests and ignored at the frame boundary with a server-error log path, not rendered as valid entities.

## Runtime Integration

Plugin order:

```text
RoutingPlugin
PathfindingPlugin
HierarchicalRoutingPlugin
FlowFieldPlugin
MobilityPlugin
```

`FlowFieldPlugin` installs `FlowFieldCache` and any route-assignment stats resources. It does not mutate the graph.

Runtime refresh rules:

- Any path that rebuilds or replaces `Graph` must rebuild `HpaIndex`, clear `PathCache`, and clear `FlowFieldCache`.
- Existing test helper paths such as `set_mobility_for_test` must refresh flow-field resources together with HPA.

## Tests

Required backend tests:

1. Flow-field construction on a hand-authored walking graph returns the expected next edge from multiple origins to one destination.
2. Flow-field construction respects `RoutingProfileKey` and rejects illegal edge kinds.
3. Corridor-scoped construction refuses paths outside the selected cluster set.
4. `FlowFieldCache` records miss, hit, insert, and eviction stats.
5. Route materialization follows a field into deterministic `RouteStep`s.
6. `route_assignment_system` inserts `ActiveRoute` for a walking agent with a resolvable `WalkToActivity`.
7. `route_advance_system` crosses from edge 1 to edge 2 without changing plan cursor prematurely.
8. Completing the final route step advances `WalkToStop` or `WalkToActivity` exactly once.
9. Snapshot extract/apply round-trips an agent with `ActiveRoute`.
10. Hydration rejects invalid persisted route steps.
11. Runtime seeded Zurich has `FlowFieldCache`.
12. Runtime helper graph replacement clears/rebuilds HPA and flow-field resources.

Required frontend tests:

1. Proto conversion rejects missing agent state instead of fabricating `at_activity`.
2. Proto conversion rejects missing world coordinates instead of fabricating `(0, 0)`.
3. Walking state accepts both legacy ids and `edge:<u32>` ids.
4. Mobility reducer still interpolates graph-executed walking entities by authoritative coordinates.

Required verification:

```bash
cargo test -p sim-core routing::flow_field -- --nocapture
cargo test -p sim-core mobility:: -- --nocapture
cargo test -p sim-server runtime_ -- --nocapture
cargo test --workspace -- --nocapture
cargo clippy --workspace --all-targets -- -D warnings
./node_modules/.bin/tsc --noEmit --pretty false
./node_modules/.bin/vitest run --passWithNoTests --reporter=dot --pool=forks --fileParallelism=false
node scripts/smoke-7b.mjs
```

Acceptance greps:

```bash
rg -n "fallback|fall back|unwrap_or\\(\\(0\\.0, 0\\.0\\)\\)|at_activity with empty|synthetic link|global A\\*" backend/crates/sim-core/src backend/crates/sim-server/src src tests
rg -n "FlowFieldCache|ActiveRoute|route_assignment_system|route_advance_system" backend/crates/sim-core/src
```

The first grep must return no production fallback language introduced by 8e. Existing historical docs are excluded from the acceptance grep.

## Performance Budget

8e must stay within the Phase 8d `tick_100k_all_active` +5% budget unless the benchmark is deliberately updated with a new baseline and a written rationale.

The important performance invariant is that route assignment is amortized:

- Per-tick route advancement is O(number of active walking agents).
- Field construction is O(edges in corridor log nodes in corridor).
- Many agents sharing destination/corridor hit `FlowFieldCache`.
- Route assignment does not scan all agents for every destination.

## Acceptance Criteria

8e is complete when:

1. Walking agents can execute multi-edge graph routes in the live mobility schedule.
2. Flow fields are used for batch route materialization and covered by cache stats.
3. HPA corridor failures and flow-field unreachable states are explicit errors.
4. Snapshot persistence round-trips graph route execution.
5. Existing frontend rendering continues to display moving agents from backend coordinates.
6. No proto, DTO, or frontend decoder creates silent valid-looking data for malformed mobility frames.
7. Full backend, frontend, smoke, and perf verification passes or any known caveat is documented with exact evidence.

