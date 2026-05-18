# Mobility Spawn-Time Position Init — Phase 7b Loose-End Fix

**Status:** Design approved, awaiting implementation plan
**Date:** 2026-05-18
**Predecessor:** Phase 7b WS multi-client scalability (commits `981ab72 … 2126bc6`)
**Surfaced by:** `scripts/smoke-7b.mjs` browser smoke

## Problem

Seeded agents (and any agent later respawned by `promote_warm_to_active_system`) are spawned with the default `Position { x: 0.0, y: 0.0 }`. The actual world coordinate is only computed on the FIRST `compute_world_coord_system` tick, which runs in the `Output` SystemSet — AFTER the `LOD` SystemSet has already classified the agents on the same tick.

The cascade:

1. `from_network` (or other seed entrypoint) spawns ~1011 agents, all with `Position(0,0)`.
2. Tick 1, `MobilitySet::LOD` runs first:
   - `track_chunk_populations_system` reads `Position(0,0)` for every agent → counts all 1011 into `chunk(0,0)`.
   - `classify_activity_system` classifies chunk(0,0): no subscribers, population > 0 → **Warm** (Asleep → Warm transition).
   - Subscribed chunks (e.g. (1..5, 1..5) under default camera) have zero population → Active (Asleep → Active transition).
   - `demote_active_to_warm_system` (added in commit `0f2bdb1`) fires on the Asleep → Warm transition for chunk(0,0) → **despawns all 1011 agents into the chunk's FlowCell aggregate**.
3. `Advance` set runs against the now-empty agent set → no work.
4. `Output` set finally runs `compute_world_coord_system` — but there are no agents to compute positions for.
5. `promote_warm_to_active_system` only fires on Warm → Active|Hot transitions, i.e. when a chunk gains a subscriber. chunk(0,0) is never subscribed by camera-driven viewport clients → stays Warm forever → agents sit in the FlowCell aggregate, never re-spawned, never moving, never rendered.

Browser smoke `scripts/smoke-7b.mjs` saw `mobility_chunk_snapshot` frames flow on subscribe (per-chunk wire format works) but every snapshot was effectively empty, and zero `mobility_chunk_delta` frames flowed per tick. The smoke's checks were too permissive — they verified frame presence and chunk coverage but not entity content. That's an additional defect this fix addresses.

## Goals

- Seeded agents and respawn-from-FlowCell agents are spawned with the correct `Position` derived from their initial `AgentMobilityState`.
- LOD classification on Tick 1 uses real-world positions, not (0,0). Chunks where agents actually live become Active (if subscribed) or Warm (if unsubscribed and populated) — without spurious mass-demote to a single chunk.
- Camera-driven clients receive `MobilityChunkSnapshot` frames containing real agent data, and `MobilityChunkDelta` frames flow per tick for chunks with moving agents in their subscription set.
- One source of truth for "given an agent's mobility state, what's the world coord?" — used by spawn, respawn, and tick-time recompute.

## Non-Goals

- Phase 7c (Arc-snapshot lock-free reads). Deferred.
- Refactoring the LOD systems to use derived-coord rather than Position-component. Position-component-driven LOD is fine once Position is correct from spawn.
- Fixing initial-Position for `InVehicle` or `AtActivity` agent states. These don't have an unambiguous origin coord at spawn time and aren't common in seeded data. Acceptable to keep them at (0,0) for this fix.

## Architecture

### Single source of truth

Two pure helper functions in `backend/crates/sim-core/src/mobility/mod.rs`:

```rust
pub fn agent_world_coord(
    state: &AgentMobilityState,
    routes: &Routes,
    stops: &Stops,
    link_polylines: &LinkPolylines,
) -> Option<(f32, f32)>;

pub fn vehicle_world_coord(
    route_position: &RoutePosition,
    routes: &Routes,
    link_polylines: &LinkPolylines,
) -> Option<(f32, f32)>;
```

The bodies are the existing logic that lives inline in `compute_world_coord_system`:

- `agent_world_coord` for `Walking { link_id, progress }` reads `link_polylines` and calls the existing `coord_at_progress` helper.
- For `WaitingAtStop` / `Boarding { stop_id, .. }` / `Alighting { stop_id, .. }` it reads the stop, follows its route → link → polyline.
- For `InVehicle { .. }` and `AtActivity { .. }` it returns `None` (no unambiguous spawn coord; tick-time computation can fill from the vehicle later).

- `vehicle_world_coord` reads route → link → polyline at `progress`.

Both functions are pure: no `&mut`, no ECS world access, just resource borrows.

### Call sites — three places update

**1. `compute_world_coord_system` (existing, tick-time)** — refactor to delegate to the helpers. Behaviour-preserving.

**2. `spawn_agent_from_record` (existing, spawn-time)** — currently spawns the entity with `Position { x: 0.0, y: 0.0 }`. Change to compute via `agent_world_coord(&record.state, …)` and use that result (falling back to (0,0) on `None`).

**3. `spawn_vehicle_from_record` (existing, spawn-time)** — analogous: compute via `vehicle_world_coord` from the record's `RoutePosition`-equivalent fields, fall back to (0,0) on `None`.

**4. `promote_warm_to_active_system` (existing, LOD-respawn-time)** — currently does `commands.spawn((... Position { x: 0.0, y: 0.0 } ...))`. Either:
- (a) refactor to call `spawn_agent_from_record(record)` after building a synthetic `AgentRecord`, OR
- (b) inline the `agent_world_coord` call at the spawn site.

(b) is the more minimal change. The system already builds the spawn closure inline; adding two lines to compute coord and replace the Position literal is the cleanest diff. The implementation plan should pick (b).

### Data Flow After Fix

```
seed:
  for each agent:
    record = AgentRecord { state: Walking { link, 0.0 }, … }
    spawn_agent_from_record(record):
      (x, y) = agent_world_coord(&record.state, routes, stops, link_polylines)
                 .unwrap_or((0.0, 0.0))
      world.spawn((..., Position { x, y }, ...))

tick 1:
  LOD reads each agent's real Position → classifies by real chunk
  chunk(real) with subscriber → Active → agents stay alive & tick
  chunk(real) without subscriber, but populated → Warm → demote into FlowCell (fine; that's the LOD design)
  Advance moves the alive agents
  Output recomputes Position from new progress (compute_world_coord_system uses same helper)

per-tick:
  per-chunk delta produced for chunks with movement
  MobilityChunkDelta flows to clients subscribed to those chunks
```

Camera-driven subscribers see snapshots containing the agents in their visible area, plus per-tick deltas as those agents walk.

## Components

**Modified files:**
- `backend/crates/sim-core/src/mobility/mod.rs` — new `agent_world_coord` + `vehicle_world_coord` pure functions; `spawn_agent_from_record` + `spawn_vehicle_from_record` compute Position at spawn.
- `backend/crates/sim-core/src/mobility/systems.rs` — `compute_world_coord_system` delegates to helpers (behaviour-preserving refactor); `promote_warm_to_active_system` computes Position before `commands.spawn`.
- `scripts/smoke-7b.mjs` — re-enable `got_chunk_deltas_per_tick` check; add `snapshots_non_empty` check that asserts total-agents-across-snapshots > 0.

**Test files:**
- `backend/crates/sim-core/src/mobility/mod.rs` test module — new unit test `spawn_agent_from_record_initializes_position_from_link_polyline`.
- `backend/crates/sim-server/tests/websocket.rs` — simplify the existing `subscribed_chunk_receives_mobility_chunk_delta_each_tick` to subscribe only to the chunk where agents actually live (no longer needs the chunk(0,0) workaround that worked around this bug).

## Error Handling

- **Walking agent's `link_id` not in `link_polylines`**: `agent_world_coord` returns `None`, spawn falls back to `Position(0,0)`. Same as today's tick-time behaviour for the same condition.
- **Stop in `WaitingAtStop`/`Boarding`/`Alighting` not in `stops`**: same fallback.
- **Vehicle's route or link missing**: same fallback.
- **InVehicle / AtActivity state at spawn**: `None` → spawn at (0,0). For InVehicle this is corrected at first tick by either compute_world_coord_system (which checks for vehicle position) or by the agent being filtered out of broadcasts anyway. For AtActivity it stays at (0,0) until the agent transitions to Walking; out of scope for this fix.

## Testing

### Unit test (TDD)

In `backend/crates/sim-core/src/mobility/mod.rs` test module:

```rust
#[test]
fn spawn_agent_from_record_initializes_position_from_link_polyline() {
    let mut world = MobilityWorld::empty();
    world.set_link_polyline(LinkId("l".into()), vec![(10.0, 20.0), (30.0, 40.0)]);
    world.spawn_agent_from_record(AgentRecord::new(
        AgentId("a".into()),
        AgentMobilityState::Walking { link_id: LinkId("l".into()), progress: 0.0 },
        vec![PlanStage::Activity { activity_id: "act".into() }],
        0.0,
    ));
    // Before fix: world_coord_for_agent returns (10, 20) via the geometry helper,
    // but the Position component itself is (0, 0). After fix: Position == (10, 20)
    // at spawn time, before any tick.
    let entity = *world.by_agent_id.get(&AgentId("a".into())).unwrap();
    let pos = world.world.entity(entity).get::<Position>().unwrap();
    assert_eq!((pos.x, pos.y), (10.0, 20.0));
}
```

(Adjust the field-access pattern if the test module already has a cleaner accessor.)

### Integration test simplification

`backend/crates/sim-server/tests/websocket.rs::subscribed_chunk_receives_mobility_chunk_delta_each_tick` currently subscribes to BOTH chunk(0,0) AND chunk(4,4) to work around this bug. Simplify to subscribe only to (4,4) (or whichever chunk the seeded agents actually live in after the fix). Verify the delta still arrives.

### Browser smoke (strengthened)

In `scripts/smoke-7b.mjs`:

1. Re-enable check that was disabled due to this bug:
   ```js
   got_chunk_deltas_per_tick: recv.mobility_chunk_delta.count > 0,
   ```
2. Add a new check that prevents the bug from re-surfacing — snapshots must contain real entities, not be empty:
   ```js
   snapshots_contain_entities:
     receivedFrames.some((f) => {
       try {
         const m = JSON.parse(f);
         return m.type === 'mobility_chunk_snapshot' && (m.agents.length > 0 || m.vehicles.length > 0);
       } catch { return false; }
     }),
   ```
3. Remove the comment block that explained the limitation — it's no longer accurate after this fix.

After the fix all checks pass without the previous caveat. Smoke remains the regression guard.

## Risks

- **Spawn cost goes up**: each `spawn_agent_from_record` now does one HashMap lookup + polyline indexing. For seeded 1011 agents this is fine. For repeated mid-game spawns via `promote_warm_to_active`, same per-spawn cost. Negligible.
- **Backward compat for snapshot deserialize**: `MobilityWorld` deserialization rebuilds agents via `spawn_agent_from_record`. The fix changes spawn-time Position. Any persisted snapshot will produce DIFFERENT Position values on rehydrate than what was originally serialized. The custom serde does NOT serialize Position (it serializes records only) — Position is always derived from state. So rehydrate produces the SAME Position the snapshot would have at the moment it was taken. No backward-compat break.
- **AtActivity agents at default Position**: per spec they remain at (0,0). They don't currently appear in seeded `from_network` (which only seeds Walking / InVehicle / WaitingAtStop). If they ever do, they'd land in chunk(0,0) and could trigger the same demote-cascade — but that's a separate issue not covered here.
- **promote_warm_to_active_system minimal-diff vs refactor**: picking (b) inline keeps the diff small but slightly duplicates the spawn shape. Acceptable for a focused fix; can be cleaned up later.

## Open questions

None for this fix. After it lands, the natural follow-up is Phase 7c (Arc-snapshot lock-free reads) per the existing roadmap.
