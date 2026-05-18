# Mobility Spawn-Time Position Init Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the Phase-6 LOD-vs-spawn-order bug exposed by the Phase-7b browser smoke: seeded agents start with `Position(0,0)`, get classified into chunk(0,0) by LOD, then mass-demoted into FlowCells where they sit forever. Make Position correct from spawn time via a shared `agent_world_coord` / `vehicle_world_coord` helper used by both the per-tick `compute_world_coord_system` and the spawn paths.

**Architecture:** Extract two pure free functions on `MobilityWorld`'s module that map `(state, resources) -> Option<(f32, f32)>` for agents and `(route_position, resources) -> Option<(f32, f32)>` for vehicles. Call them from `compute_world_coord_system` (behaviour-preserving refactor), from `spawn_agent_from_record` / `spawn_vehicle_from_record` (set Position at spawn), and inline in `promote_warm_to_active_system` (LOD respawn).

**Tech Stack:** Rust, `bevy_ecs` resources + components.

---

## Spec

This plan implements `docs/superpowers/specs/2026-05-18-mobility-spawn-time-position-init-design.md`. Re-read that spec if any task is unclear.

## File Structure

**Modified files:**
- `backend/crates/sim-core/src/mobility/mod.rs` — new `agent_world_coord` + `vehicle_world_coord` pure helpers; `spawn_agent_from_record` + `spawn_vehicle_from_record` compute Position at spawn; new unit test.
- `backend/crates/sim-core/src/mobility/systems.rs` — `compute_world_coord_system` delegates to helpers (behaviour-preserving); `promote_warm_to_active_system` computes Position before `commands.spawn`.
- `backend/crates/sim-server/tests/websocket.rs` — simplify `subscribed_chunk_receives_mobility_chunk_delta_each_tick` (drop the chunk(0,0) workaround).
- `scripts/smoke-7b.mjs` — re-enable `got_chunk_deltas_per_tick` check, add `snapshots_contain_entities` check, remove stale comment.

**No new files.**

---

## Task 1: Extract pure helpers + refactor `compute_world_coord_system`

Behaviour-preserving extraction. No tests change, no behaviour change, just refactor.

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/mod.rs`
- Modify: `backend/crates/sim-core/src/mobility/systems.rs`

- [ ] **Step 1: Add `agent_world_coord` + `vehicle_world_coord` to mod.rs**

In `backend/crates/sim-core/src/mobility/mod.rs`, at module top-level (near the existing `chunk_of` free function), add:

```rust
/// World coord for an agent given its mobility state. Returns `None` for
/// states where there is no unambiguous spawn-time coord (`InVehicle`,
/// `AtActivity`).
///
/// Used by both `compute_world_coord_system` (per-tick) and
/// `spawn_agent_from_record` (one-shot at spawn time) so LOD systems see
/// the real position immediately on Tick 1 instead of the default `(0,0)`.
pub fn agent_world_coord(
    state: &crate::mobility::records::AgentMobilityState,
    routes: &crate::mobility::resources::Routes,
    stops: &crate::mobility::resources::Stops,
    link_polylines: &crate::mobility::resources::LinkPolylines,
) -> Option<(f32, f32)> {
    use crate::mobility::records::AgentMobilityState;
    match state {
        AgentMobilityState::Walking { link_id, progress } => link_polylines
            .0
            .get(link_id)
            .map(|points| crate::mobility_geometry::world_coord_at_progress_slice(points, *progress)),
        AgentMobilityState::WaitingAtStop { stop_id }
        | AgentMobilityState::Boarding { stop_id, .. }
        | AgentMobilityState::Alighting { stop_id, .. } => stops.0.get(stop_id).and_then(|stop| {
            let route = routes.0.get(&stop.route_id)?;
            let link_id = route.links.get(stop.link_index)?;
            let points = link_polylines.0.get(link_id)?;
            Some(crate::mobility_geometry::world_coord_at_progress_slice(points, stop.progress))
        }),
        _ => None,
    }
}

/// World coord for a vehicle given its route position. Returns `None` if
/// the route or link is missing from resources.
pub fn vehicle_world_coord(
    route_position: &crate::mobility::components::RoutePosition,
    routes: &crate::mobility::resources::Routes,
    link_polylines: &crate::mobility::resources::LinkPolylines,
) -> Option<(f32, f32)> {
    let route = routes.0.get(&route_position.route_id)?;
    let link_id = route.links.get(route_position.link_index)?;
    let points = link_polylines.0.get(link_id)?;
    Some(crate::mobility_geometry::world_coord_at_progress_slice(points, route_position.progress))
}
```

Adjust the import paths (`crate::mobility::records::`, etc.) to match the conventions already used in `mod.rs` — many of these types are re-exported and the existing functions may use shorter paths.

- [ ] **Step 2: Refactor `compute_world_coord_system` to delegate**

In `backend/crates/sim-core/src/mobility/systems.rs`, replace the existing `compute_world_coord_system` body:

```rust
pub fn compute_world_coord_system(
    mut agents: Query<
        (&AgentMobilityStateComponent, &mut Position),
        (With<AgentMarker>, Without<VehicleMarker>),
    >,
    mut vehicles: Query<
        (&RoutePosition, &mut Position),
        (With<VehicleMarker>, Without<AgentMarker>),
    >,
    activities: Res<ChunkActivities>,
    routes: Res<Routes>,
    stops: Res<Stops>,
    link_polylines: Res<LinkPolylines>,
) {
    for (rp, mut pos) in vehicles.iter_mut() {
        if !chunk_is_simulated(&pos, &activities) {
            continue;
        }
        if let Some((x, y)) =
            crate::mobility::vehicle_world_coord(rp, &routes, &link_polylines)
        {
            pos.x = x;
            pos.y = y;
        }
    }
    for (state, mut pos) in agents.iter_mut() {
        if !chunk_is_simulated(&pos, &activities) {
            continue;
        }
        if let Some((x, y)) =
            crate::mobility::agent_world_coord(&state.0, &routes, &stops, &link_polylines)
        {
            pos.x = x;
            pos.y = y;
        }
    }
}
```

The old inline `match` block is replaced by the helper call. The `coord_at_progress` local helper at the top of `systems.rs` is now unused — delete it.

- [ ] **Step 3: Run existing sim-core tests**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core 2>&1 | grep -E "test result|FAILED" | tail -5
```

Expected: same green count as before (no behaviour change). If `coord_at_progress` was used elsewhere in `systems.rs` than just `compute_world_coord_system`, the compiler will tell you; restore it as needed.

- [ ] **Step 4: Workspace + clippy**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml --workspace 2>&1 | grep -E "test result|FAILED" | tail -10
cargo clippy --locked --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings 2>&1 | tail -5
```

Expected: all green, clippy clean.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/mobility/mod.rs backend/crates/sim-core/src/mobility/systems.rs
git commit -m "refactor(mobility): extract agent/vehicle_world_coord helpers"
```

---

## Task 2: Spawn-time Position init (the actual fix)

TDD: write failing test, then update both spawn functions + the LOD respawn to use the helpers.

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/mod.rs`
- Modify: `backend/crates/sim-core/src/mobility/systems.rs`

- [ ] **Step 1: Write the failing TDD test**

Append to the existing test module in `backend/crates/sim-core/src/mobility/mod.rs`:

```rust
#[test]
fn spawn_agent_from_record_initializes_position_from_link_polyline() {
    use crate::ids::{AgentId, LinkId};
    use crate::mobility::components::Position;

    let mut world = MobilityWorld::empty();
    world.set_link_polyline(LinkId("l".into()), vec![(10.0, 20.0), (30.0, 40.0)]);
    world.spawn_agent_from_record(AgentRecord::new(
        AgentId("a".into()),
        AgentMobilityState::Walking {
            link_id: LinkId("l".into()),
            progress: 0.0,
        },
        vec![PlanStage::Activity { activity_id: "act".into() }],
        0.0,
    ));

    // Before fix: Position is (0, 0) until compute_world_coord_system runs.
    // After fix: Position is (10, 20) — start of the polyline — immediately.
    let entity = *world.by_agent_id.get(&AgentId("a".into())).unwrap();
    let pos = world.world.entity(entity).get::<Position>().unwrap();
    assert_eq!((pos.x, pos.y), (10.0, 20.0));
}

#[test]
fn spawn_vehicle_from_record_initializes_position_from_route() {
    use crate::ids::{LinkId, RouteId, VehicleId};
    use crate::mobility::components::Position;
    use crate::mobility::records::{RouteRecord, VehicleKind, VehicleRecord};
    use std::collections::VecDeque;

    let mut world = MobilityWorld::empty();
    world.set_link_polyline(LinkId("v".into()), vec![(100.0, 200.0), (300.0, 400.0)]);
    // Register a route that uses the link above.
    world
        .world
        .resource_mut::<crate::mobility::resources::Routes>()
        .0
        .insert(
            RouteId("r".into()),
            RouteRecord {
                id: RouteId("r".into()),
                kind: VehicleKind::Tram,
                links: vec![LinkId("v".into())],
            },
        );
    world.spawn_vehicle_from_record(VehicleRecord {
        id: VehicleId("v1".into()),
        kind: VehicleKind::Tram,
        route_id: RouteId("r".into()),
        link_index: 0,
        progress: 0.0,
        speed_per_tick: 0.0,
        capacity: 0,
        occupants: vec![],
        dwell_ticks_remaining: 0,
    });

    let entity = *world.by_vehicle_id.get(&VehicleId("v1".into())).unwrap();
    let pos = world.world.entity(entity).get::<Position>().unwrap();
    assert_eq!((pos.x, pos.y), (100.0, 200.0));
}
```

Adjust the `RouteRecord` / `VehicleRecord` field shapes to match what's in `records.rs` (the field names above are best-guess — read the actual struct first).

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core spawn_agent_from_record_initializes_position spawn_vehicle_from_record_initializes_position 2>&1 | tail -15
```

Expected: FAIL with `(0.0, 0.0) != (10.0, 20.0)` (and similar for vehicle).

- [ ] **Step 3: Update `spawn_agent_from_record` to compute Position**

In `backend/crates/sim-core/src/mobility/mod.rs`, find `spawn_agent_from_record`. It currently spawns the entity with `Position { x: 0.0, y: 0.0 }` (or similar literal). Locate the spawn call.

Add at the top of the function body:

```rust
let (px, py) = {
    let routes = self.world.resource::<Routes>();
    let stops = self.world.resource::<Stops>();
    let link_polylines = self.world.resource::<LinkPolylines>();
    crate::mobility::agent_world_coord(&record.state, routes, stops, link_polylines)
        .unwrap_or((0.0, 0.0))
};
```

Then in the actual `self.world.spawn(...)` call, replace `Position { x: 0.0, y: 0.0 }` with `Position { x: px, y: py }`.

- [ ] **Step 4: Update `spawn_vehicle_from_record` to compute Position**

In the same file, find `spawn_vehicle_from_record`. Locate the spawn call (also has `Position { x: 0.0, y: 0.0 }`). Add at the top:

```rust
let (px, py) = {
    let routes = self.world.resource::<Routes>();
    let link_polylines = self.world.resource::<LinkPolylines>();
    let rp = crate::mobility::components::RoutePosition {
        route_id: record.route_id.clone(),
        link_index: record.link_index,
        progress: record.progress,
        speed: record.speed_per_tick,
    };
    crate::mobility::vehicle_world_coord(&rp, routes, link_polylines)
        .unwrap_or((0.0, 0.0))
};
```

(Adjust `RoutePosition` field names to match the actual struct — they may differ slightly. Read the struct first.)

Replace `Position { x: 0.0, y: 0.0 }` with `Position { x: px, y: py }` in the spawn call.

- [ ] **Step 5: Update `promote_warm_to_active_system` to compute Position**

In `backend/crates/sim-core/src/mobility/systems.rs`, find `promote_warm_to_active_system`. The function does something like `commands.spawn((AgentMarker, ..., Position { x: 0.0, y: 0.0 }, ...))`.

Compute the spawn position right before the spawn call. The system already has `routes`, `link_polylines`, `stops` as `Res<...>` (or it doesn't — check; if it needs more resources, add them as `Res<…>` parameters). The respawned agent has a synthesized `AgentMobilityState::Walking { link_id, progress: 0.0 }` where `link_id` is one of the chunk's known links. Compute:

```rust
let (px, py) = crate::mobility::agent_world_coord(
    &spawned_state,
    &routes,
    &stops,
    &link_polylines,
).unwrap_or((0.0, 0.0));
```

Replace the `Position { x: 0.0, y: 0.0 }` literal in the spawn call with `Position { x: px, y: py }`.

If the system already constructs the `AgentMobilityState` in a local var (it should — it builds it for the `AgentMobilityStateComponent`), reuse that var. If it constructs the component inline, refactor to a local var first.

- [ ] **Step 6: Run TDD tests to verify they pass**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core spawn_agent_from_record_initializes_position spawn_vehicle_from_record_initializes_position 2>&1 | tail -10
```

Expected: PASS.

- [ ] **Step 7: Full workspace + clippy**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml --workspace 2>&1 | grep -E "test result|FAILED" | tail -10
cargo clippy --locked --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings 2>&1 | tail -5
```

Expected: all green, clippy clean.

- [ ] **Step 8: Commit**

```bash
git add backend/crates/sim-core/src/mobility/mod.rs backend/crates/sim-core/src/mobility/systems.rs
git commit -m "fix(mobility): compute Position at spawn so LOD classifies into real chunk"
```

---

## Task 3: Simplify integration test + strengthen browser smoke

**Files:**
- Modify: `backend/crates/sim-server/tests/websocket.rs`
- Modify: `scripts/smoke-7b.mjs`

- [ ] **Step 1: Simplify `subscribed_chunk_receives_mobility_chunk_delta_each_tick`**

In `backend/crates/sim-server/tests/websocket.rs`, find the test added in Phase 7b T7. It currently subscribes to BOTH `(0, 0)` AND another chunk to work around the bug now fixed. Simplify to subscribe ONLY to the chunk where seeded agents actually live (the second one in the original list — most likely `(4, 4)` per the Phase-6 / Phase-7 work, but verify by reading the existing test).

The old test body:
```rust
send_chunk_subscribe(&mut client, &[
    ChunkCoordDto { x: 0, y: 0 },
    ChunkCoordDto { x: 4, y: 4 },
]).await;
```

becomes:
```rust
send_chunk_subscribe(&mut client, &[ChunkCoordDto { x: 4, y: 4 }]).await;
```

The rest of the test (read frames, assert MobilityChunkDelta arrives for chunk(4,4)) stays unchanged.

- [ ] **Step 2: Run the simplified test**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server --test websocket subscribed_chunk_receives_mobility_chunk_delta_each_tick 2>&1 | tail -10
```

Expected: PASS without the chunk(0,0) crutch.

- [ ] **Step 3: Strengthen `scripts/smoke-7b.mjs`**

In `scripts/smoke-7b.mjs`, find the `checks` object. Replace it:

```js
const checks = {
  page_loaded: receivedFrames.length > 0,
  got_chunk_snapshots_on_subscribe: recv.mobility_chunk_snapshot.count > 0,
  one_snapshot_per_subscribed_chunk:
    Object.keys(recv.mobility_chunk_snapshot.chunks).length >= 9,
  // Re-enabled after the spawn-time Position init fix: seeded agents are
  // now LOD-classified into their real chunks (not all into chunk(0,0))
  // so subscribed chunks actually receive per-tick deltas.
  got_chunk_deltas_per_tick: recv.mobility_chunk_delta.count > 0,
  // Snapshots must contain real entity data — the previous bug let empty
  // snapshots flow undetected because we only counted frames, not content.
  snapshots_contain_entities: receivedFrames.some((f) => {
    try {
      const m = JSON.parse(f);
      return m.type === 'mobility_chunk_snapshot'
        && (m.agents.length > 0 || m.vehicles.length > 0);
    } catch { return false; }
  }),
  no_legacy_mobility_delta: recv.mobility_delta_LEGACY === 0,
  client_sent_chunk_subscribe: sent.chunk_subscribe > 0,
  pan_added_more_frames: afterPanReceivedCount > initialReceivedCount,
  no_console_errors: consoleErrors.length === 0,
};
```

Delete the stale comment block at the bottom of the previous `checks` object (the one explaining why per-tick deltas weren't asserted).

- [ ] **Step 4: Start dev stack**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
pkill -f run-dev-stack 2>/dev/null; pkill -f sim-server 2>/dev/null; pkill -f "vite --host" 2>/dev/null
sleep 2
nohup npm run dev:stack > /tmp/abutown-stack.log 2>&1 & disown
until curl -sf http://127.0.0.1:8080/health > /dev/null; do sleep 3; done
echo BACKEND_UP
```

- [ ] **Step 5: Run smoke**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
node scripts/smoke-7b.mjs 2>&1 | tail -25
```

Expected: all 9 checks (was 7) pass. If `got_chunk_deltas_per_tick` is false, the spawn-time fix didn't reach the real seed path — investigate before continuing (likely: `seed::from_network` uses a different spawn path that bypasses `spawn_agent_from_record`).

If `snapshots_contain_entities` is false, the snapshots are still empty — agents may be in some chunk other than the visible ones. Check the city_network seed to see where agents land.

- [ ] **Step 6: Stop dev stack**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
pkill -f run-dev-stack 2>/dev/null
pkill -f sim-server 2>/dev/null
pkill -f "vite --host" 2>/dev/null
sleep 1
pgrep -fl "run-dev-stack|sim-server|vite --host" || echo all-stopped
```

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-server/tests/websocket.rs scripts/smoke-7b.mjs
git commit -m "test(ws): drop chunk(0,0) workaround + strengthen 7b smoke after spawn-fix"
```

---

## Task 4: Final quality gate + progress note + push

**Files:**
- Modify: `progress.md`

- [ ] **Step 1: Full gates**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo fmt --all --manifest-path backend/Cargo.toml
cargo test --locked --manifest-path backend/Cargo.toml --workspace 2>&1 | grep -E "test result|FAILED" | tail -12
cargo clippy --locked --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings 2>&1 | tail -5
npx vitest run 2>&1 | tail -6
npx tsc --noEmit
npm run build 2>&1 | tail -5
```

Expected: all green.

- [ ] **Step 2: Add progress entry**

Get timestamp:
```bash
date -u +%Y-%m-%dT%H:%M:%S.000Z
```

Insert at the top of the reverse-chronological tail in `progress.md` (immediately before the Phase 7b entry):

```
<TIMESTAMP> - Spawn-time Position init (Phase 7b loose-end fix): the Phase-7b browser smoke surfaced that seeded agents have `Position(0,0)` until `compute_world_coord_system` runs in tick 1's Output phase — but the LOD systems run BEFORE Output in the same tick, so they mass-classify all ~1011 seeded agents into chunk(0,0). chunk(0,0) is unsubscribed by camera-driven viewports → `demote_active_to_warm_system` collapses everyone into a FlowCell → agents never tick again and the frontend renders nothing. Fix: extract two pure helpers `agent_world_coord(state, routes, stops, link_polylines) -> Option<(f32, f32)>` and `vehicle_world_coord(route_position, routes, link_polylines) -> Option<(f32, f32)>` in `mobility::mod`, then use them from `compute_world_coord_system` (behaviour-preserving refactor), `spawn_agent_from_record`/`spawn_vehicle_from_record` (set Position at spawn time so LOD classifies correctly on tick 1), and `promote_warm_to_active_system` (LOD respawn). The simplified `subscribed_chunk_receives_mobility_chunk_delta_each_tick` integration test no longer needs to subscribe to chunk(0,0) as a workaround. `scripts/smoke-7b.mjs` re-enables `got_chunk_deltas_per_tick` and adds `snapshots_contain_entities` — both gaps that let the original bug ship.
```

- [ ] **Step 3: Commit + push**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add progress.md
git commit -m "chore: quality gate + progress note for spawn-time Position init fix"
git push origin main
```

---

## Self-Review

**1. Spec coverage:**

| Spec requirement | Task |
|---|---|
| `agent_world_coord` / `vehicle_world_coord` helpers | Task 1 |
| `compute_world_coord_system` delegates to helpers | Task 1 |
| `spawn_agent_from_record` computes Position | Task 2 |
| `spawn_vehicle_from_record` computes Position | Task 2 |
| `promote_warm_to_active_system` computes Position | Task 2 |
| Unit test: spawn-time Position init for agents | Task 2 |
| Unit test: spawn-time Position init for vehicles | Task 2 |
| Integration test simplification (drop chunk(0,0) workaround) | Task 3 |
| Smoke re-enables `got_chunk_deltas_per_tick` | Task 3 |
| Smoke adds `snapshots_contain_entities` | Task 3 |
| Final gate + progress | Task 4 |

All covered.

**2. Placeholder scan:** No "TBD" / "implement later". Two places have read-the-actual-struct-first qualifiers (Task 2 Step 1 for RouteRecord/VehicleRecord shapes; Task 2 Step 5 for promote_warm_to_active_system's existing structure). Both have concrete fallback instructions, not vague TODOs.

**3. Type consistency:**
- `agent_world_coord(state, routes, stops, link_polylines)` and `vehicle_world_coord(route_position, routes, link_polylines)` consistent in Tasks 1, 2.
- `(f32, f32)` return type with `Option<…>` wrapping consistent.
- `Position { x, y }` struct shape preserved across all call sites.
- The `RoutePosition` synthesis in Task 2 Step 4 must match the real struct — flagged with "adjust field names" qualifier.

**Order rationale:** Refactor (T1, behaviour-preserving) before the fix (T2) so the fix is a tiny diff. Test simplification + smoke (T3) verifies the fix end-to-end. Final gate (T4) closes out. Each task produces a passing-tests commit.

**Scope check:** 4 tasks, ~4-5 commits. Each task is bite-sized and self-contained. The fix is genuinely focused.
