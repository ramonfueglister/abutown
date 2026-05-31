# Visible Economy Traders Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make economy traders **visible as real walking mobility agents** — they walk the real footway graph between their source and destination markets, replicate through the existing per-tick mobility delta path (so the client renders smooth motion), carry a distinct trader sprite, and a small demo economy is seeded into the live world so a trader is actually on screen.

**Architecture:** A render-only **materialization bridge** in the economy reads the authoritative (untouched, conserved, deterministic) `Trader` state machine and drives a dedicated `TraderAgent` entity along a real HPA*/flow-field footway route, at the progress implied by the trader's `ToDest`/`ToSource` countdown. The entity carries render components only (no `AgentMarker`), is fed into `DirtyAgents` each tick so the existing `tick_mobility` delta builder ships it to subscribed clients, and is materialized only while its current chunk is observed (Active/Hot) — the same LOD thesis the rest of the simulation uses. Economy core, conservation, and determinism are unchanged.

**Tech Stack:** Rust (`bevy_ecs`, `sim-core`/`sim-server` crates), TypeScript/Vite frontend, Playwright headless-chromium browser smoke. Cargo MUST run through `scripts/cargo-serial.sh` (build-lock serialization — see CLAUDE.md).

**Base branch:** `origin/main` (this `plan/economy-visible-traders` worktree). Task 0 resets the branch to discard the prior snapshot-only attempt and keep only the corrected spec.

**Spec:** `docs/superpowers/specs/2026-05-31-economy-visible-traders-design.md`

---

## Verified integration facts (grounding for every task)

- **Delta path carries no-`AgentMarker` entities.** `tick_mobility` (`mobility/api.rs:847`) does `std::mem::take` of `DirtyAgents.0` and iterates the raw `Entity` set (`for entity in &dirty_agents`); `agent_record_from_entity` (`api.rs:503`) reads `StableAgentId`/`AgentMobilityStateComponent`/`WalkPlan`/`WalkSpeed` via `world.get::<_>(entity)?` — **no `AgentMarker` check**. So an entity with those components, inserted into `DirtyAgents`, appears in `MobilityChunkDelta.changed_agents`. Cross-chunk `left_agents` fires when a dirty entity's current chunk differs from `PreviousAgentChunks` (`api.rs:891`).
- **Subscribe snapshot** (`build_mobility_chunk_snapshot`, `api.rs:236`) iterates `AgentIdIndex` keys (no `AgentMarker` filter) and filters by position→chunk, so a materialized trader appears in snapshots automatically.
- **Route recipe (Walk):** `hpa.corridor_between(from, to, RoutingProfileKey::Walk) -> HashSet<ClusterId>` (`routing/hpa.rs:208`) → `FlowFieldCacheKey::new(dest, Walk, 0, &sorted_corridor)` + `FlowFieldScope::Corridor(corridor)` → `cache.get_or_build_with_cluster_lookup(&graph, key, RoutingProfile::for_key(Walk), scope, |n| hpa.cluster_of_node(n)) -> Arc<FlowField>` (`routing/flow_field.rs:226`) → `materialize_route_steps(&graph, &field, origin, ModeState::Walking) -> Option<Vec<RouteStep>>` (`mobility/systems/routing.rs:50`). Each `RouteStep.edge_id` → `graph.edge(edge_id).polyline: Vec<(f32,f32)>`. Sample with `mobility_geometry::world_coord_at_progress_slice(&poly, t)` (`mobility_geometry.rs:23`).
- **Graph/Node/Edge:** `Graph::node(NodeId)->&Node{position:(f32,f32),kind:NodeKind}`, `Graph::edge(EdgeId)->&Edge{from,to,polyline,length,kind:EdgeKind}` (`routing/graph.rs:25-99`). `NodeKind::{Intersection,TransitStop,ActivityLocation}`; `EdgeKind::{Footway,Road,TramTrack}`. `Graph::node_by_legacy(&str)->Option<NodeId>` (`graph.rs:125`). `NodeSpatialIndex::nearest((f32,f32))->Option<NodeId>` (`routing/spatial_index.rs:42`). `chunk_of(x,y,32)->ChunkCoord` (`mobility/mod.rs:36`).
- **Routing type exports:** `crate::routing::{Graph, NodeId, EdgeId, RoutingProfile, RoutingProfileKey, ModeState, HpaIndex, FlowFieldCache, FlowFieldCacheKey, FlowFieldScope}`. The HPA cluster lookup is `hpa.cluster_of_node(node)`. `materialize_route_steps` is `crate::mobility::systems::routing::materialize_route_steps` (confirm it is `pub`; if it is private, make it `pub(crate)`).
- **Economy facts:** `Trader{actor,good,source,dest,distance_tiles,batch_qty,...,state:TraderState}` and `TraderState::{Buying{order},ToDest{remaining},Selling{order},ToSource{remaining}}` (`economy/traders.rs:14-34`). `Traders(BTreeMap<EconomicActorId,Trader>)` resource. `transport_ticks(distance_tiles,&config)->u64` (`traders.rs:56`). `transport::manhattan_tiles(&graph,from,to)->i64` (`transport.rs:7`). `MarketSite{id:MarketId,node_id:NodeId,name:String}`, `Markets(BTreeMap<MarketId,MarketSite>)`, `MarketChunks(BTreeMap<MarketId,ChunkCoord>)` (`economy/market.rs`). `DemandPool`/`SupplyPool` (full fields in `economy/pools.rs:12-33`), `DemandPools`/`SupplyPools` resources. `AccountBook::deposit(actor,Money)`, `InventoryBook::deposit(actor,good,qty)`. `GoodId` consts: `GOOD_FOOD(1)`,`GOOD_WOOD(2)`,`GOOD_IRON(3)`,`GOOD_TOOLS(4)`. `EconomyConfig{trader_tiles_per_tick:4, trader_default_ref_price:Money(1_000), transport_cost_per_tile_unit:Money(5), ...}`.
- **Economy schedule:** `EconomySet::{RefreshLod,ExpireOrders,Production,Traders,GeneratePoolOrders,ClearMarkets,WarmFlow,Telemetry}` chained (`economy/systems.rs:17,55`). `EconomyPlugin::install` inserts all economy resources + `install_systems(schedule)` (`economy/mod.rs:42`). `Tick` = `crate::mobility::resources::Tick`.
- **LOD test pattern:** `world.spawn((ChunkCoordComp(ChunkCoord{x,y}), ActiveChunk));` etc. Imports `crate::world::components::{ActiveChunk,AsleepChunk,ChunkCoordComp,HotChunk,WarmChunk}`, `crate::ids::ChunkCoord`.
- **Runtime seam:** `sim_core::economy::EconomyPlugin.install(&mut world,&mut schedule)` at `sim-server/src/runtime/mod.rs:217` (fresh) and `:340` (hydrate); the routing `Graph` + `NodeSpatialIndex` are already inserted (RoutingPlugin ran just before). `bundle.spawn_all_chunks` + mobility snapshot apply happen after.
- **Frontend:** `AgentMobilityDto.sprite_key: string` (already on the wire, `mobilityProtocol.ts:19`). `spriteIndexFromKey(key,modulus)` (`backendMobilityDrawables.ts:55`) splits on `:` and parses the last segment. `pedestriansFromMobilityState` picks `sprites[spriteIndexFromKey(agent.sprite_key, sprites.length)]` (`:80`). `drawPedestrian` fills a circle with `AGENT_COLOR='#343b43'` (`minimalMapRenderer.ts:474,121`). Dev stack: backend `http://127.0.0.1:8080`, vite `http://127.0.0.1:5175`, `npm run dev:stack`. Smoke template: `scripts/smoke-7b.mjs`.

---

## Task 0: Reset the branch to a clean base, keep the corrected spec

**Files:**
- Git branch `plan/economy-visible-traders` (this worktree)
- Keep: `docs/superpowers/specs/2026-05-31-economy-visible-traders-design.md` + this plan

- [ ] **Step 1: Save the corrected spec + plan out of the worktree**

```bash
cp docs/superpowers/specs/2026-05-31-economy-visible-traders-design.md /tmp/visible-traders-spec.md
cp docs/superpowers/plans/2026-05-31-economy-visible-traders.md /tmp/visible-traders-plan.md
```

- [ ] **Step 2: Reset the branch to origin/main (discards the 2 prior commits + all uncommitted half-built code)**

This is a feature branch (NOT main); discarding its own WIP is the agreed cleanup. `reflog` keeps it recoverable.

```bash
git fetch origin
git reset --hard origin/main
git status --short   # expect: clean
git log --oneline -1 # expect: origin/main HEAD
```

- [ ] **Step 3: Restore the corrected spec + plan and commit on the clean base**

```bash
mkdir -p docs/superpowers/specs docs/superpowers/plans
cp /tmp/visible-traders-spec.md docs/superpowers/specs/2026-05-31-economy-visible-traders-design.md
cp /tmp/visible-traders-plan.md docs/superpowers/plans/2026-05-31-economy-visible-traders.md
git add docs/superpowers/specs/2026-05-31-economy-visible-traders-design.md docs/superpowers/plans/2026-05-31-economy-visible-traders.md
git commit -m "docs: corrected visible-traders spec + plan (real walking agents)"
```

Expected: a single clean commit on top of `origin/main`, no backend/frontend changes.

---

## Task 1: `TraderAgent` marker component + `world_coord_for_agent` intercept

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/components.rs`
- Modify: `backend/crates/sim-core/src/mobility/api.rs` (`world_coord_for_agent`, near line 589)
- Test: `backend/crates/sim-core/src/mobility/systems/tests.rs` (append)

- [ ] **Step 1: Write the failing test**

Append to `mobility/systems/tests.rs`:

```rust
#[test]
fn trader_agent_world_coord_reads_position_verbatim() {
    use crate::mobility::api::world_coord_for_agent;
    use crate::mobility::components::{
        AgentMobilityStateComponent, BirthTick, Direction, Position, SpriteKey, StableAgentId,
        TraderAgent, WalkPlan, WalkSpeed,
    };
    use crate::mobility::records::AgentMobilityState;
    use crate::mobility::resources::AgentIdIndex;
    use crate::ids::AgentId;

    let mut world = bevy_ecs::world::World::new();
    world.insert_resource(AgentIdIndex::default());
    let id = AgentId("trader:1".to_string());
    let entity = world
        .spawn((
            TraderAgent,
            StableAgentId(id.clone()),
            AgentMobilityStateComponent(AgentMobilityState::AtActivity {
                activity_id: "trader".to_string(),
            }),
            WalkPlan { stages: vec![], cursor: 0, cyclic: false },
            WalkSpeed(0.0),
            BirthTick(0),
            Position { x: 12.5, y: 34.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey("trader:3".to_string()),
        ))
        .id();
    world.resource_mut::<AgentIdIndex>().0.insert(id.clone(), entity);

    assert_eq!(world_coord_for_agent(&world, &id), Some((12.5, 34.0)));
}
```

- [ ] **Step 2: Run it and watch it fail (TraderAgent undefined)**

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core trader_agent_world_coord_reads_position_verbatim
```
Expected: FAIL — `cannot find type TraderAgent`.

- [ ] **Step 3: Add the `TraderAgent` marker**

In `mobility/components.rs`, near the other marker components (e.g. after `AgentMarker`):

```rust
/// Marks a render-only economy-trader agent. It carries NO `AgentMarker`, so no
/// mobility movement/bookkeeping system touches it; the economy materialization
/// bridge is its sole owner and writes its `Position` authoritatively.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraderAgent;
```

- [ ] **Step 4: Add the intercept to `world_coord_for_agent`**

In `mobility/api.rs`, at the top of `world_coord_for_agent` (right after resolving `entity` from `AgentIdIndex`, before reading `AgentMobilityStateComponent`):

```rust
    // Materialized trader-agents carry an authoritative Position written by the
    // economy materialize bridge; their mobility state is only a benign DTO filler.
    if world
        .get::<crate::mobility::components::TraderAgent>(entity)
        .is_some()
    {
        let pos = world.get::<crate::mobility::components::Position>(entity)?;
        return Some((pos.x, pos.y));
    }
```

- [ ] **Step 5: Run the test to verify it passes**

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core trader_agent_world_coord_reads_position_verbatim
```
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/mobility/components.rs backend/crates/sim-core/src/mobility/api.rs backend/crates/sim-core/src/mobility/systems/tests.rs
git commit -m "feat(mobility): TraderAgent marker + authoritative-Position world coord"
```

---

## Task 2: Route-polyline helpers (pure, testable without HPA)

The full HPA*/flow-field route is exercised in integration + the browser smoke (it needs a built graph + indices). Here we unit-test the two pure pieces the bridge needs: concatenating edge polylines into one route polyline, and mapping `TraderState` → travel progress.

**Files:**
- Create: `backend/crates/sim-core/src/economy/trader_render.rs`
- Modify: `backend/crates/sim-core/src/economy/mod.rs` (add `mod trader_render;` + re-export)
- Modify: `backend/crates/sim-core/src/routing/graph.rs` (add a `#[doc(hidden)] pub fn from_parts` test/seed constructor)
- Test: in `trader_render.rs` `#[cfg(test)]`

- [ ] **Step 1: Write the failing tests**

Create `backend/crates/sim-core/src/economy/trader_render.rs`:

```rust
//! Render-only helpers turning trader state + a footway route into a world coord.
//! Pure functions; no ECS. The bridge (materialize.rs) wires these to resources.

use crate::economy::traders::transport_ticks;
use crate::economy::{EconomyConfig, Trader, TraderState};
use crate::routing::{EdgeId, Graph};

/// Concatenate the polylines of `edges` into one route polyline, dropping the
/// duplicated shared endpoint between consecutive edges. Empty if no edges.
pub fn route_polyline(graph: &Graph, edges: &[EdgeId]) -> Vec<(f32, f32)> {
    let mut out: Vec<(f32, f32)> = Vec::new();
    for &edge_id in edges {
        let poly = &graph.edge(edge_id).polyline;
        if poly.is_empty() {
            continue;
        }
        let start = if out.last() == poly.first() { 1 } else { 0 };
        out.extend_from_slice(&poly[start..]);
    }
    out
}

/// Travel progress in [0,1] for a trader, given its travel-tick budget.
/// `Buying` => 0 (at source), `Selling` => 1 (at dest).
pub fn leg_progress(state: &TraderState, travel: u64) -> f32 {
    let travel = travel.max(1) as f32;
    match state {
        TraderState::Buying { .. } => 0.0,
        TraderState::Selling { .. } => 1.0,
        TraderState::ToDest { remaining } | TraderState::ToSource { remaining } => {
            let done = travel - (*remaining as f32);
            (done / travel).clamp(0.0, 1.0)
        }
    }
}

/// `Buying`/`ToDest` => outbound (source->dest); `Selling`/`ToSource` => return.
pub fn is_outbound(state: &TraderState) -> bool {
    matches!(state, TraderState::Buying { .. } | TraderState::ToDest { .. })
}

/// The travel-tick budget for a trader (so callers don't re-derive it).
pub fn trader_travel(trader: &Trader, config: &EconomyConfig) -> u64 {
    transport_ticks(trader.distance_tiles, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{Edge, EdgeId, EdgeKind, Graph, Node, NodeId, NodeKind};

    fn node(id: u32, x: f32, y: f32) -> Node {
        Node { id: NodeId(id), position: (x, y), kind: NodeKind::Intersection, legacy_id: None }
    }
    fn edge(id: u32, from: u32, to: u32, poly: Vec<(f32, f32)>) -> Edge {
        let length = poly
            .windows(2)
            .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
            .sum();
        Edge {
            id: EdgeId(id), from: NodeId(from), to: NodeId(to), polyline: poly,
            length, kind: EdgeKind::Footway, speed_limit: 1.0, capacity: 1, legacy_id: None,
        }
    }

    #[test]
    fn route_polyline_concats_and_dedupes_shared_endpoints() {
        let graph = Graph::from_parts(
            vec![node(0, 0.0, 0.0), node(1, 2.0, 0.0), node(2, 2.0, 3.0)],
            vec![
                edge(0, 0, 1, vec![(0.0, 0.0), (2.0, 0.0)]),
                edge(1, 1, 2, vec![(2.0, 0.0), (2.0, 3.0)]),
            ],
        );
        let poly = route_polyline(&graph, &[EdgeId(0), EdgeId(1)]);
        assert_eq!(poly, vec![(0.0, 0.0), (2.0, 0.0), (2.0, 3.0)]); // shared (2,0) once
    }

    #[test]
    fn leg_progress_maps_countdown_to_unit_interval() {
        assert_eq!(leg_progress(&TraderState::ToDest { remaining: 4 }, 4), 0.0);
        assert_eq!(leg_progress(&TraderState::ToDest { remaining: 1 }, 4), 0.75);
        assert_eq!(leg_progress(&TraderState::Buying { order: None }, 4), 0.0);
        assert_eq!(leg_progress(&TraderState::Selling { order: None }, 4), 1.0);
    }

    #[test]
    fn is_outbound_distinguishes_legs() {
        assert!(is_outbound(&TraderState::Buying { order: None }));
        assert!(is_outbound(&TraderState::ToDest { remaining: 2 }));
        assert!(!is_outbound(&TraderState::Selling { order: None }));
        assert!(!is_outbound(&TraderState::ToSource { remaining: 2 }));
    }
}
```

- [ ] **Step 2: Add the module + a test/seed `Graph::from_parts` constructor**

In `economy/mod.rs`, add: `mod trader_render;` and `pub use trader_render::{is_outbound, leg_progress, route_polyline, trader_travel};`.

Read the real `Graph` struct in `routing/graph.rs`, then add a constructor that fills **every** field (set any legacy/index maps to empty, recompute derived counts) — deterministic, no panics:

```rust
impl Graph {
    /// Build a graph directly from nodes + edges (test/seed helper). The index
    /// of each item in the vec must equal its `id.0`. Legacy maps start empty.
    #[doc(hidden)]
    pub fn from_parts(nodes: Vec<Node>, edges: Vec<Edge>) -> Self {
        // Fill the real struct's fields here (mirror the canonical builder).
        // e.g. Self { nodes, edges, legacy_node_index: HashMap::new(),
        //             legacy_edge_index: HashMap::new(), /* + any others */ }
        Self { /* … real fields … */ }
    }
}
```

- [ ] **Step 3: Run the tests and watch them fail**

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core trader_render
```
Expected: FAIL (compile error until `from_parts` + module exist).

- [ ] **Step 4: Make them pass**

Implement `from_parts` against the real struct until the 3 tests compile and pass.

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core trader_render
```
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/trader_render.rs backend/crates/sim-core/src/economy/mod.rs backend/crates/sim-core/src/routing/graph.rs
git commit -m "feat(economy): pure trader-render helpers (route polyline, leg progress)"
```

---

## Task 3: `MaterializedTraders` resource + the materialization bridge

**Files:**
- Create: `backend/crates/sim-core/src/economy/materialize.rs`
- Modify: `backend/crates/sim-core/src/economy/mod.rs`
- Test: `backend/crates/sim-core/src/economy/tests/materialize.rs` (+ register in `tests/mod.rs`)

> **Unit-test seam:** the `materialize_traders_system` needs `Res<HpaIndex>` + `ResMut<FlowFieldCache>` which only exist in the full runtime. To keep Task-3/5/6 unit tests HPA-free, `leg_polyline` returns `Some(vec![graph.node(from).position])` when `from == to`, and the unit tests seed the source and dest markets on the **same** node, with the trader in `Buying`/`Selling`/`ToDest{remaining}` so progress maps onto that single-point (or two-point) polyline without any flow-field build. The real multi-edge HPA route is covered by the integration test (Task 4) and the browser smoke (Task 11), where a built graph exists.

- [ ] **Step 1: Write the failing test (spawn at source when chunk is Active)**

Create `backend/crates/sim-core/src/economy/tests/materialize.rs`:

```rust
use bevy_ecs::prelude::*;

use crate::economy::materialize::{materialize_traders_at_tick, MaterializedTraders};
use crate::economy::{
    EconomyConfig, EconomicActorId, MarketChunks, MarketId, MarketSite, Markets, Quantity, Trader,
    TraderState, Traders, GOOD_TOOLS,
};
use crate::ids::ChunkCoord;
use crate::mobility::components::{Direction, Position, StableAgentId, TraderAgent};
use crate::mobility::resources::{AgentIdIndex, DirtyAgents};
use crate::routing::{Edge, EdgeId, EdgeKind, Graph, Node, NodeId, NodeKind};
use crate::world::components::{ActiveChunk, ChunkCoordComp};

fn node(id: u32, x: f32, y: f32) -> Node {
    Node { id: NodeId(id), position: (x, y), kind: NodeKind::Intersection, legacy_id: None }
}

/// Source and dest markets anchored to the SAME node (1,1) so `leg_polyline`
/// short-circuits (from==to) — no HPA needed. Chunk (0,0) is Active.
fn seed_world(state: TraderState) -> (World, EconomicActorId, std::collections::BTreeSet<ChunkCoord>) {
    use std::collections::BTreeSet;
    let mut world = World::new();
    let graph = Graph::from_parts(vec![node(0, 1.0, 1.0)], Vec::<Edge>::new());
    world.insert_resource(graph);
    world.insert_resource(AgentIdIndex::default());
    world.insert_resource(DirtyAgents::default());
    world.insert_resource(MaterializedTraders::default());
    world.insert_resource(EconomyConfig::default());

    let mut markets = Markets::default();
    markets.0.insert(MarketId(1), MarketSite { id: MarketId(1), node_id: NodeId(0), name: "A".into() });
    markets.0.insert(MarketId(2), MarketSite { id: MarketId(2), node_id: NodeId(0), name: "B".into() });
    world.insert_resource(markets);

    let mut anchors = MarketChunks::default();
    anchors.0.insert(MarketId(1), ChunkCoord { x: 0, y: 0 });
    anchors.0.insert(MarketId(2), ChunkCoord { x: 0, y: 0 });
    world.insert_resource(anchors);

    let actor = EconomicActorId(1);
    let mut traders = Traders::default();
    traders.0.insert(actor, Trader {
        actor, good: GOOD_TOOLS, source: MarketId(1), dest: MarketId(2), distance_tiles: 4,
        batch_qty: Quantity(1), buy_premium_bps: 0, sell_discount_bps: 0, order_ttl_ticks: 10, state,
    });
    world.insert_resource(traders);

    let observed: BTreeSet<ChunkCoord> = [ChunkCoord { x: 0, y: 0 }].into_iter().collect();
    (world, actor, observed)
}

/// Drive the bridge directly (no HPA): runs the system body with an in-test
/// observed set. The system itself is integration-tested in Task 4.
fn run_materialize(world: &mut World, observed: &std::collections::BTreeSet<ChunkCoord>) {
    // SAFETY/SIMPLICITY: build the QueryStates the helper needs and call it.
    let mut positions = world.query_filtered::<&mut Position, With<TraderAgent>>();
    let mut directions = world.query_filtered::<&mut Direction, With<TraderAgent>>();
    // For from==to legs `leg_polyline` ignores hpa/cache, so we can pass a fresh
    // default cache + a graph-derived HpaIndex stub is not needed: route is a
    // single point. Use the dedicated test entrypoint below instead.
    crate::economy::materialize::materialize_for_test(world, observed, &mut positions, &mut directions);
}

#[test]
fn materialize_spawns_trader_agent_at_source_in_active_chunk() {
    let (mut world, actor, observed) = seed_world(TraderState::Buying { order: None });
    run_materialize(&mut world, &observed);

    let mut q = world.query_filtered::<(Entity, &Position, &StableAgentId), With<TraderAgent>>();
    let hits: Vec<_> = q.iter(&world).map(|(e, p, s)| (e, (p.x, p.y), s.0.clone())).collect();
    assert_eq!(hits.len(), 1, "exactly one trader-agent");
    assert_eq!(hits[0].1, (1.0, 1.0), "buying => at source node");
    assert!(hits[0].2 .0.starts_with("trader:"));
    assert!(world.resource::<DirtyAgents>().0.contains(&hits[0].0), "fed into delta path");
    assert!(world.resource::<MaterializedTraders>().0.contains_key(&actor));
}
```

Register: in `economy/tests/mod.rs` add `mod materialize;`.

- [ ] **Step 2: Run and watch it fail**

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core materialize_spawns_trader_agent_at_source_in_active_chunk
```
Expected: FAIL — `materialize` module / functions undefined.

- [ ] **Step 3: Implement the bridge**

Create `backend/crates/sim-core/src/economy/materialize.rs`:

```rust
//! Render-only bridge: materialize each economy trader as a walking mobility
//! agent on the real footway graph while its current chunk is observed, feeding
//! the per-tick mobility delta. Never mutates economy state (conservation-safe).

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;

use crate::economy::trader_render::{is_outbound, leg_progress, route_polyline, trader_travel};
use crate::economy::{EconomyConfig, MarketChunks, Markets, Trader, Traders};
use crate::ids::{AgentId, ChunkCoord};
use crate::mobility::components::{
    AgentMobilityStateComponent, BirthTick, Direction, Position, SpriteKey, StableAgentId,
    TraderAgent, WalkPlan, WalkSpeed,
};
use crate::mobility::records::AgentMobilityState;
use crate::mobility::resources::{AgentIdIndex, DirtyAgents, Tick};
use crate::routing::{
    EdgeId, FlowFieldCache, FlowFieldCacheKey, FlowFieldScope, Graph, HpaIndex, ModeState, NodeId,
    RoutingProfile, RoutingProfileKey,
};
use crate::world::components::{ActiveChunk, ChunkCoordComp, HotChunk};

#[derive(Debug, Clone)]
pub struct MaterializedTrader {
    pub entity: Entity,
    pub leg_outbound: bool,
    pub polyline: Vec<(f32, f32)>,
}

#[derive(Resource, Default)]
pub struct MaterializedTraders(pub BTreeMap<crate::economy::EconomicActorId, MaterializedTrader>);

fn sprite_hash(id: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in id.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h % 8
}

fn dir_from_delta(dx: f32, dy: f32) -> abutown_protocol::DirectionDto {
    use abutown_protocol::DirectionDto as D;
    if dx == 0.0 && dy == 0.0 {
        return D::S;
    }
    let octant = (((dy.atan2(dx) / std::f32::consts::FRAC_PI_4).round() as i32) + 8) % 8;
    [D::E, D::Se, D::S, D::Sw, D::W, D::Nw, D::N, D::Ne][octant as usize]
}

/// Walk footway route polyline between two nodes. `from==to` short-circuits to a
/// single point (no flow-field build — keeps unit tests HPA-free).
fn leg_polyline(
    graph: &Graph,
    hpa: &HpaIndex,
    cache: &mut FlowFieldCache,
    from: NodeId,
    to: NodeId,
) -> Option<Vec<(f32, f32)>> {
    if from == to {
        return Some(vec![graph.node(from).position]);
    }
    let corridor = hpa.corridor_between(from, to, RoutingProfileKey::Walk).ok()?;
    let mut corridor_key: Vec<_> = corridor.iter().copied().collect();
    corridor_key.sort_unstable();
    let key = FlowFieldCacheKey::new(to, RoutingProfileKey::Walk, 0, &corridor_key);
    let scope = FlowFieldScope::Corridor(corridor);
    let profile = RoutingProfile::for_key(RoutingProfileKey::Walk);
    let field = cache
        .get_or_build_with_cluster_lookup(graph, key, profile, scope, |n| hpa.cluster_of_node(n))
        .ok()?;
    let steps = crate::mobility::systems::routing::materialize_route_steps(
        graph, &field, from, ModeState::Walking,
    )?;
    let edges: Vec<EdgeId> = steps.iter().map(|s| s.edge_id).collect();
    let poly = route_polyline(graph, &edges);
    if poly.is_empty() { None } else { Some(poly) }
}

fn endpoints(markets: &Markets, trader: &Trader) -> Option<(NodeId, NodeId)> {
    Some((markets.0.get(&trader.source)?.node_id, markets.0.get(&trader.dest)?.node_id))
}

/// Core bridge. `route` resolves a leg's polyline (real HPA in prod; the
/// from==to short-circuit covers unit tests).
#[allow(clippy::too_many_arguments)]
fn run_bridge(
    traders: &Traders,
    markets: &Markets,
    config: &EconomyConfig,
    observed: &BTreeSet<ChunkCoord>,
    materialized: &mut MaterializedTraders,
    index: &mut AgentIdIndex,
    dirty: &mut DirtyAgents,
    positions: &mut Query<&mut Position, With<TraderAgent>>,
    directions: &mut Query<&mut Direction, With<TraderAgent>>,
    commands: &mut Commands,
    current_tick: u64,
    mut route: impl FnMut(NodeId, NodeId) -> Option<Vec<(f32, f32)>>,
) {
    let mut alive: BTreeSet<crate::economy::EconomicActorId> = BTreeSet::new();

    for (actor, trader) in traders.0.iter() {
        let Some((src, dst)) = endpoints(markets, trader) else { continue };
        let outbound = is_outbound(&trader.state);
        let travel = trader_travel(trader, config);

        let need_route = materialized.0.get(actor).map(|m| m.leg_outbound != outbound).unwrap_or(true);
        let polyline = if need_route {
            let (a, b) = if outbound { (src, dst) } else { (dst, src) };
            match route(a, b) {
                Some(p) => p,
                None => continue,
            }
        } else {
            materialized.0.get(actor).unwrap().polyline.clone()
        };

        let t = leg_progress(&trader.state, travel);
        let (x, y) = crate::mobility_geometry::world_coord_at_progress_slice(&polyline, t);
        let chunk = crate::mobility::chunk_of(x, y, 32);
        let (nx, ny) = crate::mobility_geometry::world_coord_at_progress_slice(&polyline, (t + 0.02).min(1.0));
        let dir = dir_from_delta(nx - x, ny - y);

        if observed.contains(&chunk) {
            alive.insert(*actor);
            match materialized.0.get(actor).map(|m| m.entity) {
                Some(entity) => {
                    if let Ok(mut p) = positions.get_mut(entity) { p.x = x; p.y = y; }
                    if let Ok(mut d) = directions.get_mut(entity) { d.0 = dir; }
                    dirty.0.insert(entity);
                    let m = materialized.0.get_mut(actor).unwrap();
                    m.leg_outbound = outbound;
                    m.polyline = polyline;
                }
                None => {
                    let agent_id = AgentId(format!("trader:{}", actor.0));
                    let entity = commands
                        .spawn((
                            TraderAgent,
                            StableAgentId(agent_id.clone()),
                            AgentMobilityStateComponent(AgentMobilityState::AtActivity {
                                activity_id: "trader".to_string(),
                            }),
                            WalkPlan { stages: vec![], cursor: 0, cyclic: false },
                            WalkSpeed(0.0),
                            BirthTick(current_tick),
                            Position { x, y },
                            Direction(dir),
                            SpriteKey(format!("trader:{}", sprite_hash(&agent_id.0))),
                        ))
                        .id();
                    index.0.insert(agent_id, entity);
                    dirty.0.insert(entity);
                    materialized.0.insert(*actor, MaterializedTrader { entity, leg_outbound: outbound, polyline });
                }
            }
        } else if let Some(m) = materialized.0.get(actor) {
            // Unobserved: nudge to current pos + mark dirty once (so the old
            // chunk's delta emits `left_agents`), then despawn + drop the index.
            let entity = m.entity;
            if let Ok(mut p) = positions.get_mut(entity) { p.x = x; p.y = y; }
            dirty.0.insert(entity);
            commands.entity(entity).despawn();
            index.0.remove(&AgentId(format!("trader:{}", actor.0)));
            materialized.0.remove(actor);
        }
    }

    // Despawn agents whose trader vanished from `Traders`.
    let stale: Vec<_> = materialized
        .0
        .keys()
        .filter(|a| !alive.contains(a) && !traders.0.contains_key(a))
        .copied()
        .collect();
    for actor in stale {
        if let Some(m) = materialized.0.remove(&actor) {
            commands.entity(m.entity).despawn();
            index.0.remove(&AgentId(format!("trader:{}", actor.0)));
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn materialize_traders_system(
    tick: Res<Tick>,
    traders: Res<Traders>,
    markets: Res<Markets>,
    graph: Res<Graph>,
    hpa: Res<HpaIndex>,
    mut cache: ResMut<FlowFieldCache>,
    config: Res<EconomyConfig>,
    active: Query<&ChunkCoordComp, bevy_ecs::query::Or<(With<ActiveChunk>, With<HotChunk>)>>,
    mut materialized: ResMut<MaterializedTraders>,
    mut index: ResMut<AgentIdIndex>,
    mut dirty: ResMut<DirtyAgents>,
    mut positions: Query<&mut Position, With<TraderAgent>>,
    mut directions: Query<&mut Direction, With<TraderAgent>>,
    mut commands: Commands,
) {
    let observed: BTreeSet<ChunkCoord> = active.iter().map(|c| c.0).collect();
    let graph_ref = &*graph;
    let hpa_ref = &*hpa;
    run_bridge(
        &traders, &markets, &config, &observed, &mut materialized, &mut index, &mut dirty,
        &mut positions, &mut directions, &mut commands, tick.0,
        |a, b| leg_polyline(graph_ref, hpa_ref, &mut cache, a, b),
    );
}

/// Test entrypoint: drives the bridge using the `from==to` short-circuit route
/// (no HPA/flow-field resources required). Used by the materialize unit tests.
#[cfg(any(test, feature = "test-support"))]
pub fn materialize_for_test(
    world: &mut World,
    observed: &BTreeSet<ChunkCoord>,
    positions: &mut Query<&mut Position, With<TraderAgent>>,
    directions: &mut Query<&mut Direction, With<TraderAgent>>,
) {
    world.resource_scope(|world, traders: Mut<Traders>| {
        world.resource_scope(|world, markets: Mut<Markets>| {
            world.resource_scope(|world, config: Mut<EconomyConfig>| {
                world.resource_scope(|world, mut materialized: Mut<MaterializedTraders>| {
                    world.resource_scope(|world, mut index: Mut<AgentIdIndex>| {
                        world.resource_scope(|world, mut dirty: Mut<DirtyAgents>| {
                            let graph = world.resource::<Graph>();
                            let mut commands_queue = bevy_ecs::system::CommandQueue::default();
                            let mut commands = Commands::new(&mut commands_queue, world);
                            run_bridge(
                                &traders, &markets, &config, observed, &mut materialized,
                                &mut index, &mut dirty, positions, directions, &mut commands, 0,
                                |a, b| {
                                    if a == b { Some(vec![graph.node(a).position]) } else { None }
                                },
                            );
                            drop(commands);
                            commands_queue.apply(world);
                        });
                    });
                });
            });
        });
    });
}
```

> The `materialize_for_test` plumbing (resource_scope + manual CommandQueue) is the standard way to call a system-shaped fn against a `&mut World` in a test without a `Schedule`. If your bevy_ecs version exposes a simpler `world.run_system_once`, prefer that and delete `materialize_for_test`, calling `materialize_traders_system` directly after inserting stub `HpaIndex`/`FlowFieldCache` resources. Either way the assertions are unchanged.

- [ ] **Step 4: Wire the module + resource**

In `economy/mod.rs`: `pub mod materialize;` and `pub use materialize::MaterializedTraders;`.

- [ ] **Step 5: Run green**

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core materialize_spawns_trader_agent_at_source_in_active_chunk
```
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/economy/materialize.rs backend/crates/sim-core/src/economy/mod.rs backend/crates/sim-core/src/economy/tests/materialize.rs backend/crates/sim-core/src/economy/tests/mod.rs
git commit -m "feat(economy): trader materialization bridge — spawn at source, feed DirtyAgents"
```

---

## Task 4: Dematerialize when the current chunk is unobserved

**Files:**
- Test: `backend/crates/sim-core/src/economy/tests/materialize.rs` (append)

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn materialize_despawns_when_current_chunk_unobserved() {
    use std::collections::BTreeSet;
    let (mut world, actor, observed) = seed_world(TraderState::Buying { order: None });

    // Observed first tick: agent created.
    run_materialize(&mut world, &observed);
    assert!(world.resource::<MaterializedTraders>().0.contains_key(&actor));

    // Now unobserve (empty set) and re-run: agent despawned + dropped.
    let empty: BTreeSet<crate::ids::ChunkCoord> = BTreeSet::new();
    run_materialize(&mut world, &empty);

    let count = world.query_filtered::<Entity, With<TraderAgent>>().iter(&world).count();
    assert_eq!(count, 0, "trader-agent despawned when unobserved");
    assert!(!world.resource::<MaterializedTraders>().0.contains_key(&actor));
}
```

- [ ] **Step 2: Run — should already pass** (despawn branch implemented in Task 3).

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core materialize_despawns_when_current_chunk_unobserved
```
Expected: PASS. If FAIL, fix the unobserved branch in `run_bridge`.

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-core/src/economy/tests/materialize.rs
git commit -m "test(economy): trader dematerializes when its chunk is unobserved"
```

---

## Task 5: Prove the bridge is render-only (no economy mutation)

**Files:**
- Test: `backend/crates/sim-core/src/economy/tests/materialize.rs` (append)

- [ ] **Step 1: Write the test**

```rust
#[test]
fn materialize_does_not_touch_money_or_goods() {
    use crate::economy::{AccountBook, InventoryBook, Money};
    let (mut world, actor, observed) = seed_world(TraderState::Buying { order: None });
    let mut accounts = AccountBook::default();
    accounts.deposit(actor, Money(10_000)).unwrap();
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_TOOLS, Quantity(5)).unwrap();
    // sum helpers: iterate the maps (no auction runs here).
    let money_before: i64 = accounts_total(&accounts);
    let goods_before: i64 = inventory_total(&inv);
    world.insert_resource(accounts);
    world.insert_resource(inv);

    for _ in 0..5 { run_materialize(&mut world, &observed); }

    assert_eq!(accounts_total(world.resource::<AccountBook>()), money_before);
    assert_eq!(inventory_total(world.resource::<InventoryBook>()), goods_before);
}

// Local sum helpers (or reuse the conservation-test helpers if they exist).
fn accounts_total(a: &crate::economy::AccountBook) -> i64 {
    a.iter_accounts().map(|acc| acc.available.0 + acc.locked.0).sum()
}
fn inventory_total(i: &crate::economy::InventoryBook) -> i64 {
    i.iter_balances().map(|b| b.available.0 + b.locked.0).sum()
}
```

If `iter_accounts`/`iter_balances` don't exist, reuse whatever the existing `tests/conservation.rs` uses to sum totals (grep it) — do not invent new public API just for the test; match the existing conservation-test helper.

- [ ] **Step 2: Run green**

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core materialize_does_not_touch_money_or_goods
```
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-core/src/economy/tests/materialize.rs
git commit -m "test(economy): materialization is render-only (money + goods conserved)"
```

---

## Task 6: Schedule wiring — `EconomySet::Materialize` + resource install

**Files:**
- Modify: `backend/crates/sim-core/src/economy/systems.rs` (enum + chain + add_systems)
- Modify: `backend/crates/sim-core/src/economy/mod.rs` (`EconomyPlugin::install` inserts `MaterializedTraders`)
- Test: `backend/crates/sim-core/src/economy/tests/plugin.rs` (append)

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn economy_plugin_installs_materialized_traders() {
    use bevy_ecs::prelude::*;
    use crate::economy::{EconomyPlugin, MaterializedTraders};
    use crate::world::schedule::SimPlugin;

    let mut world = World::new();
    let mut schedule = Schedule::default();
    EconomyPlugin.install(&mut world, &mut schedule);
    assert!(world.get_resource::<MaterializedTraders>().is_some());
}
```

- [ ] **Step 2: Run, watch fail**

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy_plugin_installs_materialized_traders
```
Expected: FAIL.

- [ ] **Step 3: Add `EconomySet::Materialize` + register the system + insert the resource**

In `economy/systems.rs`, add `Materialize` to the enum between `WarmFlow` and `Telemetry`:

```rust
pub enum EconomySet {
    RefreshLod,
    ExpireOrders,
    Production,
    Traders,
    GeneratePoolOrders,
    ClearMarkets,
    WarmFlow,
    Materialize,
    Telemetry,
}
```

In `configure_sets((...).chain())` insert `EconomySet::Materialize` between `WarmFlow` and `Telemetry`. In `add_systems((...))` add:

```rust
            crate::economy::materialize::materialize_traders_system.in_set(EconomySet::Materialize),
```

In `economy/mod.rs` `EconomyPlugin::install`, before `install_systems(schedule);`:

```rust
        world.insert_resource(crate::economy::materialize::MaterializedTraders::default());
```

- [ ] **Step 4: Run green**

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy_plugin_installs_materialized_traders
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/systems.rs backend/crates/sim-core/src/economy/mod.rs backend/crates/sim-core/src/economy/tests/plugin.rs
git commit -m "feat(economy): schedule EconomySet::Materialize + install MaterializedTraders"
```

---

## Task 7: `seed_demo_economy` — data-driven markets + a trader

**Files:**
- Create: `backend/crates/sim-core/src/economy/seed.rs`
- Modify: `backend/crates/sim-core/src/economy/mod.rs` (`pub mod seed;`)
- Test: `backend/crates/sim-core/src/economy/tests/seed.rs` (+ register)

- [ ] **Step 1: Write the failing test**

Create `backend/crates/sim-core/src/economy/tests/seed.rs`:

```rust
use crate::economy::seed::{seed_demo_economy, tests_support};
use crate::economy::{Markets, MarketChunks, Traders};
use crate::routing::Graph;

#[test]
fn seed_demo_economy_creates_two_markets_and_one_trader() {
    let mut world = tests_support::world_with_base_graph();
    seed_demo_economy(&mut world);

    assert_eq!(world.resource::<Markets>().0.len(), 2, "two demo markets");
    assert_eq!(world.resource::<MarketChunks>().0.len(), 2, "both anchored");
    assert_eq!(world.resource::<Traders>().0.len(), 1, "one demo trader");

    let graph = world.resource::<Graph>();
    for site in world.resource::<Markets>().0.values() {
        let p = graph.node(site.node_id).position;
        assert!(p.0.is_finite() && p.1.is_finite());
    }
}
```

- [ ] **Step 2: Run, watch fail**

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core seed_demo_economy_creates_two_markets_and_one_trader
```
Expected: FAIL — `seed` module undefined.

- [ ] **Step 3: Implement `seed_demo_economy`**

Create `backend/crates/sim-core/src/economy/seed.rs`:

```rust
//! Seed a tiny, data-driven demo economy into the live world so a trader is
//! actually visible. No hardcoded coordinates: market nodes are snapped from the
//! real footway graph to two reference points near the default view.

use bevy_ecs::prelude::*;

use crate::economy::transport::manhattan_tiles;
use crate::economy::{
    AccountBook, DemandPool, DemandPools, EconomicActorId, InventoryBook, MarketChunks, MarketId,
    MarketSite, Markets, Money, Quantity, SupplyPool, SupplyPools, Trader, TraderState, GOOD_TOOLS,
};
use crate::routing::Graph;

/// Reference points near the abutopia default view (corridor ends). The seeder
/// snaps each to the nearest real footway node — no coordinate is baked into the
/// graph; we only express "near here".
const REF_A: (f32, f32) = (2.0, 3.0);
const REF_B: (f32, f32) = (13.0, 3.0);

/// Seed once, on FRESH world creation only. The economy (Markets, Traders,
/// pools, accounts, inventory) fully persists via `EconomyPersistSnapshot`, so a
/// hydrated world restores the demo economy from persistence — this fn is NOT
/// called on the hydrate path (no double-seed guard, no heal-on-restore shim).
pub fn seed_demo_economy(world: &mut World) {
    let (node_a, node_b) = {
        let spatial = world.resource::<crate::routing::NodeSpatialIndex>();
        match (spatial.nearest(REF_A), spatial.nearest(REF_B)) {
            (Some(a), Some(b)) if a != b => (a, b),
            _ => return, // graph too small for a demo economy
        }
    };
    let (chunk_a, chunk_b, dist) = {
        let graph = world.resource::<Graph>();
        let pa = graph.node(node_a).position;
        let pb = graph.node(node_b).position;
        (
            crate::mobility::chunk_of(pa.0, pa.1, 32),
            crate::mobility::chunk_of(pb.0, pb.1, 32),
            manhattan_tiles(graph, node_a, node_b),
        )
    };

    let (m_a, m_b) = (MarketId(9_001), MarketId(9_002));
    {
        let mut markets = world.resource_mut::<Markets>();
        markets.0.insert(m_a, MarketSite { id: m_a, node_id: node_a, name: "Demo A".into() });
        markets.0.insert(m_b, MarketSite { id: m_b, node_id: node_b, name: "Demo B".into() });
    }
    {
        let mut anchors = world.resource_mut::<MarketChunks>();
        anchors.0.insert(m_a, chunk_a);
        anchors.0.insert(m_b, chunk_b);
    }

    let supplier = EconomicActorId(8_001);
    let consumer = EconomicActorId(8_002);
    let trader_actor = EconomicActorId(8_003);
    {
        let mut accounts = world.resource_mut::<AccountBook>();
        accounts.deposit(consumer, Money(1_000_000)).expect("seed: consumer cash");
        accounts.deposit(trader_actor, Money(1_000_000)).expect("seed: trader cash");
    }
    world
        .resource_mut::<InventoryBook>()
        .deposit(supplier, GOOD_TOOLS, Quantity(1_000_000))
        .expect("seed: supplier goods");

    // NOTE: confirm DemandPools/SupplyPools are Vec-backed (.0: Vec<_>). If they
    // are BTreeMap-keyed, insert by the appropriate key instead of `.push`.
    world.resource_mut::<SupplyPools>().0.push(SupplyPool {
        actor: supplier, market: m_a, good: GOOD_TOOLS, offered_qty_per_tick: Quantity(10),
        min_price: Money(500), interval_ticks: 1, last_generated_tick: None,
    });
    world.resource_mut::<DemandPools>().0.push(DemandPool {
        actor: consumer, market: m_b, good: GOOD_TOOLS, desired_qty_per_tick: Quantity(10),
        max_price: Money(2_000), urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1,
        last_generated_tick: None,
    });
    world.resource_mut::<Traders>().0.insert(trader_actor, Trader {
        actor: trader_actor, good: GOOD_TOOLS, source: m_a, dest: m_b, distance_tiles: dist,
        batch_qty: Quantity(5), buy_premium_bps: 500, sell_discount_bps: 500, order_ttl_ticks: 20,
        state: TraderState::Buying { order: None },
    });
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests_support {
    use bevy_ecs::prelude::*;
    /// Build a world with the base-world routing graph + indices installed,
    /// mirroring `sim-server/src/runtime` (RoutingPlugin → Pathfinding → HPA →
    /// FlowField). Provides `Graph`, `NodeSpatialIndex`, `HpaIndex`,
    /// `FlowFieldCache`. Reuse the exact base-world loader used by runtime tests.
    pub fn world_with_base_graph() -> World {
        // Implement against the existing base-world test fixture (grep
        // `seeded_walks_from_base_world` / the runtime test builder) so the
        // graph has real footway nodes near REF_A/REF_B.
        unimplemented!("wire to the existing base-world test fixture")
    }
}
```

Add `pub mod seed;` to `economy/mod.rs`. Implement `tests_support::world_with_base_graph()` against the existing base-world fixture (grep `seeded_walks_from_base_world`, `BaseWorldBundle`, and how `runtime/tests.rs` builds a routed world).

- [ ] **Step 4: Run green**

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core seed_demo_economy_creates_two_markets_and_one_trader
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/seed.rs backend/crates/sim-core/src/economy/mod.rs backend/crates/sim-core/src/economy/tests/seed.rs backend/crates/sim-core/src/economy/tests/mod.rs
git commit -m "feat(economy): data-driven seed_demo_economy (2 markets + 1 trader)"
```

---

## Task 8: Wire the seed into the live runtime (fresh path only; hydrate restores from persistence)

**Files:**
- Modify: `backend/crates/sim-server/src/runtime/mod.rs` (after the FRESH-path EconomyPlugin install at ~217 only)
- Test: `backend/crates/sim-server/src/runtime/tests.rs` (append)

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn live_runtime_seeds_demo_markets_and_trader() {
    // Use the SAME base-world runtime constructor runtime/tests.rs already uses.
    let rt = build_base_world_runtime_for_test();
    assert_eq!(rt.world().resource::<sim_core::economy::Markets>().0.len(), 2);
    assert_eq!(rt.world().resource::<sim_core::economy::Traders>().0.len(), 1);
}
```

(Use the real constructor + `world()` accessor names from `runtime/tests.rs`; if no `world()` accessor exists, add a `#[cfg(test)] pub fn world(&self) -> &World`.)

- [ ] **Step 2: Run, watch fail**

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server live_runtime_seeds_demo_markets_and_trader
```
Expected: FAIL — no markets seeded.

- [ ] **Step 3: Call the seed after the FRESH-path EconomyPlugin install only**

In `runtime/mod.rs`, immediately after the fresh-path `sim_core::economy::EconomyPlugin.install(&mut world, &mut schedule);` (≈217) add:

```rust
    sim_core::economy::seed::seed_demo_economy(&mut world);
```

Do **not** add it to the hydrate path (≈340): the economy fully round-trips through `EconomyPersistSnapshot` (`extract_from_world`/`apply_into_world` persist Markets, MarketChunks, DemandPools, SupplyPools, Traders, accounts, inventory), so a hydrated world restores the demo economy — re-seeding would duplicate markets and reset trader progress (the demographic-replay failure class). The `Graph`/`NodeSpatialIndex` the seed reads are already inserted by `RoutingPlugin` just above line 217.

- [ ] **Step 4: Run green**

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server live_runtime_seeds_demo_markets_and_trader
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-server/src/runtime/mod.rs backend/crates/sim-server/src/runtime/tests.rs
git commit -m "feat(server): seed demo economy on fresh world (hydrate restores from persistence)"
```

---

## Task 9: End-to-end — seeded trader walks the route and conserves (integration with real HPA)

**Files:**
- Test: `backend/crates/sim-server/src/runtime/tests.rs` (append) — runs the full schedule, so `Graph`/`HpaIndex`/`FlowFieldCache` exist and the multi-edge route path is exercised.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn seeded_trader_walks_and_conserves() {
    let mut rt = build_base_world_runtime_for_test();

    let money0 = economy_money_total(rt.world());
    let goods0 = economy_goods_total(rt.world());

    // The demo markets are near the default view; pin/observe their chunks so the
    // trader materializes (mirror how runtime tests mark chunks Active, or use the
    // base-world pinned chunks). Then run enough ticks to leave `Buying`.
    let mut positions: Vec<(f32, f32)> = Vec::new();
    for _ in 0..60 {
        rt.tick_once();
        if let Some(p) = trader_world_coord(rt.world(), "trader:") {
            positions.push(p);
        }
    }

    assert_eq!(economy_money_total(rt.world()), money0, "money conserved");
    assert_eq!(economy_goods_total(rt.world()), goods0, "goods conserved");
    assert!(positions.len() >= 5, "trader materialized over time");
    let first = positions[0];
    assert!(
        positions.iter().any(|p| (p.0 - first.0).abs() + (p.1 - first.1).abs() > 0.5),
        "trader's world_coord changes (it walks the route)"
    );
}

// Helpers: sum AccountBook/InventoryBook totals (reuse conservation-test helpers);
// read a TraderAgent's world_coord via mobility::api::world_coord_for_agent over
// AgentIdIndex keys starting with the prefix.
fn trader_world_coord(world: &sim_core::bevy_ecs::world::World, prefix: &str) -> Option<(f32, f32)> {
    let index = world.resource::<sim_core::mobility::resources::AgentIdIndex>();
    let id = index.0.keys().find(|k| k.0.starts_with(prefix))?.clone();
    sim_core::mobility::api::world_coord_for_agent(world, &id)
}
```

- [ ] **Step 2: Run, watch fail/iterate**

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server seeded_trader_walks_and_conserves
```
Expected: initially may FAIL if the demo chunks aren't observed in the test (mark them Active like the LOD tests do) or if `Buying` never completes (ensure the supply pool at A trades — the seed funds it). Iterate until green: trader buys at A, walks to B (positions vary), conserves.

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-server/src/runtime/tests.rs
git commit -m "test(server): e2e seeded trader walks the footway route and conserves"
```

---

## Task 10: Distinct trader sprite on the frontend

**Files:**
- Modify: `src/render/backendMobilityDrawables.ts`
- Modify: `src/render/minimalMapRenderer.ts`
- Test: `src/render/backendMobilityDrawables.test.ts` (vitest)

- [ ] **Step 1: Write the failing vitest**

Create/append `src/render/backendMobilityDrawables.test.ts`:

```ts
import { describe, it, expect } from 'vitest';
import { isTraderSpriteKey } from './backendMobilityDrawables';

describe('trader sprite selection', () => {
  it('detects trader sprite keys', () => {
    expect(isTraderSpriteKey('trader:3')).toBe(true);
    expect(isTraderSpriteKey('pedestrian:3')).toBe(false);
  });
});
```

- [ ] **Step 2: Run, watch fail**

```bash
npx vitest run src/render/backendMobilityDrawables.test.ts
```
Expected: FAIL — `isTraderSpriteKey` not exported.

- [ ] **Step 3: Implement the trader-kind branch**

In `backendMobilityDrawables.ts`, add the predicate, a `kind` field on `BackendPedestrian`, and set it in `pedestriansFromMobilityState`:

```ts
export function isTraderSpriteKey(key: string): boolean {
  return key.startsWith('trader:');
}
```

In the loop at ~line 80:

```ts
    const sprite = sprites[spriteIndexFromKey(agent.sprite_key, sprites.length)];
    out.push({
      id: agent.id,
      path: syntheticPath(agent.world_coord, agent.direction),
      offset: 0, speed: 0, laneOffset: 0,
      sprite,
      kind: isTraderSpriteKey(agent.sprite_key) ? 'trader' : 'pedestrian',
      direction: agent.direction,
      ageSeconds: agent.age_seconds,
    });
```

Add `kind: 'trader' | 'pedestrian'` to the `BackendPedestrian` type definition.

In `minimalMapRenderer.ts`, add a `TRADER_COLOR` near `AGENT_COLOR` and branch in `drawPedestrian` (replace the single fill block):

```ts
const TRADER_COLOR = '#c0392b'; // distinct trader red
// inside drawPedestrian, where it currently fills the circle:
  if (pedestrian.kind === 'trader') {
    ctx.fillStyle = TRADER_COLOR;
    ctx.globalAlpha *= 0.95;
    ctx.beginPath();
    ctx.arc(0, 0, style.radius * 1.4, 0, Math.PI * 2);
    ctx.fill();
  } else {
    ctx.fillStyle = AGENT_COLOR;
    ctx.globalAlpha *= 0.78;
    ctx.beginPath();
    ctx.arc(0, 0, style.radius, 0, Math.PI * 2);
    ctx.fill();
  }
```

- [ ] **Step 4: Run green + typecheck**

```bash
npx vitest run src/render/backendMobilityDrawables.test.ts
npx tsc --noEmit
```
Expected: PASS + no type errors.

- [ ] **Step 5: Commit**

```bash
git add src/render/backendMobilityDrawables.ts src/render/backendMobilityDrawables.test.ts src/render/minimalMapRenderer.ts
git commit -m "feat(render): distinct trader sprite via trader: sprite_key prefix"
```

---

## Task 11: MANDATORY browser smoke

**Files:**
- Create: `scripts/smoke-visible-traders.mjs` (adapt `scripts/smoke-7b.mjs`)
- Modify: `package.json` (add `smoke:visible-traders`)

- [ ] **Step 1: Adapt the smoke script**

Create `scripts/smoke-visible-traders.mjs` from `smoke-7b.mjs`. Keep its dev-stack/browser/proto-import/frame-capture/decode scaffolding verbatim (lines 12–111 of smoke-7b). Keep the pan step. Replace the assertion block with trader checks:

```js
// Gather trader agent samples from snapshots + deltas over time.
const traderSamples = new Map(); // id -> [{x,y}]
let frame = 0;
for (const m of receivedMessages) {
  frame += 1;
  const collect = (agents) => {
    for (const a of agents) {
      if (!a.id.startsWith('trader:')) continue;
      const arr = traderSamples.get(a.id) ?? [];
      arr.push({ x: a.worldCoord.x, y: a.worldCoord.y, f: frame });
      traderSamples.set(a.id, arr);
    }
  };
  if (m.body.case === 'mobilityChunkSnapshot') collect(m.body.value.agents);
  if (m.body.case === 'mobilityChunkDelta') collect(m.body.value.changedAgents);
}
const traderIds = [...traderSamples.keys()];
const traderMoved = traderIds.some((id) => {
  const s = traderSamples.get(id);
  if (s.length < 2) return false;
  const first = s[0];
  return s.some((p) => Math.abs(p.x - first.x) + Math.abs(p.y - first.y) > 0.5);
});

const checks = {
  page_loaded: receivedBinary.length > 0,
  trader_agent_present: traderIds.length > 0,
  trader_agent_moves: traderMoved,
  got_chunk_deltas_per_tick: recv.mobility_chunk_delta.count > 0,
  no_text_frames: textFramesReceived === 0 && textFramesSent === 0,
  no_console_errors: consoleErrors.length === 0,
};
const summary = { status: Object.values(checks).every(Boolean) ? 'ok' : 'failed', traderIds, checks, console_errors: consoleErrors };
console.log(JSON.stringify(summary, null, 2));
process.exit(summary.status === 'ok' ? 0 : 1);
```

Increase the idle window to ~4000ms so the trader visibly moves.

- [ ] **Step 2: Add the npm script**

In `package.json` scripts (after `smoke:mobility-persistence`):

```json
    "smoke:visible-traders": "tsx scripts/smoke-visible-traders.mjs",
```

- [ ] **Step 3: Run the smoke against the live stack**

```bash
scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server
npm run dev:stack &
STACK_PID=$!
until curl -sf http://127.0.0.1:8080/health >/dev/null; do sleep 1; done
sleep 3
node scripts/smoke-visible-traders.mjs; SMOKE=$?
kill $STACK_PID 2>/dev/null
echo "smoke exit: $SMOKE"
```
Expected: `"status": "ok"`, `trader_agent_present: true`, `trader_agent_moves: true`, `no_console_errors: true`, exit 0.

> If `trader_agent_present` is false: the demo markets aren't in the observed view — adjust `REF_A/REF_B` to nodes inside the default-loaded chunks (read `data/worlds/abutopia/layers/*`), or pan toward them. If present but not moving: the bridge isn't feeding `DirtyAgents` each tick, or the trader is stuck `Buying` (no supply trading at A) — verify the seed's supply pool clears.

- [ ] **Step 4: Commit**

```bash
git add scripts/smoke-visible-traders.mjs package.json
git commit -m "test(smoke): mandatory browser smoke for visible walking traders"
```

---

## Task 12: Full gate + finish the branch

- [ ] **Step 1: Format + lint (CI-matching stable toolchain)**

```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all
scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```
Expected: no fmt diff; clippy clean.

- [ ] **Step 2: Full workspace test (serialized; run in background + poll if slow)**

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace --all-targets
```
Expected: all green.

- [ ] **Step 3: Frontend tests + typecheck**

```bash
npx vitest run
npx tsc --noEmit
```
Expected: green.

- [ ] **Step 4: Re-run the browser smoke (acceptance gate)** — Task 11 Step 3. Expected: `status: ok`.

- [ ] **Step 5: Commit any fixups, then finish the branch**

```bash
git add -A && git commit -m "chore: fmt/clippy fixups for visible traders" || true
```

Invoke **superpowers:finishing-a-development-branch** to decide merge/PR. Verify CI green before any merge (never `--admin-merge` a red check; the CI stable toolchain may be newer than local — reformat if only fmt is red; see memories `verify-ci-green-before-merge`, `local-green-ci-red-rustfmt-skew`).

---

## Self-review (run before executing)

**Spec coverage:** trader = real walking agent on footway route → Tasks 2/3/9 ✓ · delta-path feed (smooth movement) → Task 3 (DirtyAgents) + Task 9 (e2e moves) + Task 11 (smoke moves) ✓ · LOD materialize/dematerialize → Task 3 + Task 4 ✓ · economy untouched / conservation → Task 5 + Task 9 ✓ · distinct trader sprite (sprite_key prefix, zero proto) → Task 10 ✓ · live demo economy seed (data-driven) → Tasks 7/8 ✓ · mandatory browser smoke → Task 11 ✓ · base branch origin/main, discard prior attempt → Task 0 ✓.

**Known fiddly spots (flagged, not placeholders):** (1) the unit-test seam in Tasks 3–5 avoids HPA via the `from==to` route short-circuit + `materialize_for_test`; if `run_system_once` is available, prefer it. (2) `Graph::from_parts` must fill the real struct's fields. (3) `DemandPools`/`SupplyPools` `.push` assumes `Vec`-backed — confirm and adapt. (4) The base-world test fixture (`tests_support::world_with_base_graph`, runtime test constructor, conservation sum helpers) must reuse existing fixtures — grep before writing new ones. (5) `materialize_route_steps` must be reachable (`pub(crate)`).

**Type consistency:** `MaterializedTraders` holds `MaterializedTrader{entity, leg_outbound, polyline}` throughout; `run_bridge`/`materialize_traders_system`/`materialize_for_test` signatures align; `is_outbound`/`leg_progress`/`route_polyline`/`trader_travel` names consistent (Tasks 2/3); `seed_demo_economy(&mut World)` identical in Tasks 7/8; frontend `isTraderSpriteKey` + `kind` field consistent (Task 10).
