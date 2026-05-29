# Mobility Hot-Path on bevy_ecs

> **Phase 5 of the million-agent roadmap.** Parent spec: `docs/superpowers/specs/2026-05-16-million-agent-roadmap-design.md`.

## Purpose

After Phase 4 the backend simulates ~1015 entities and filters mobility deltas per connection. The next scaling bottleneck is the per-tick simulation loop itself: today `MobilityWorld` stores agents and vehicles in `HashMap<AgentId, AgentRecord>` / `HashMap<VehicleId, VehicleRecord>` and the tick function calls `for (id, agent) in self.agents.iter_mut() { … }`. HashMap iteration is cache-hostile, the per-tick branch-and-match overhead dominates, and the data layout precludes any later SIMD or parallel work.

This phase migrates the hot-path storage to `bevy_ecs::World` with proper Components and Systems orchestrated via `bevy_ecs::Schedule`. The migration is **API-preserving**: `MobilityWorld`'s public surface (`tick`, `agent_dto_for`, `vehicle_dto_for`, `serialize`/`deserialize`, etc.) stays unchanged so all current callers — `SimulationRuntime`, the WS task, the persistence loop, ~60 existing tests — compile without changes. The JSONB persistence shape stays byte-for-byte identical so existing snapshots remain readable.

A criterion benchmark `mobility_tick` is added to lock in the perf win and detect future regressions.

After Phase 5 the backend can sustainably tick ~100k entities (per the bevy_ecs literature in `docs/literature/agent-simulation/bevy-ecs-docs.html`); Phase 6 then layers chunk-LOD on top to reach 1M.

## Non-Goals

- Parallel-system execution (single-threaded Schedule for now; Bevy supports parallel but it's YAGNI at 10k-100k scale).
- SIMD micro-tuning (dense storage already gives the big win; SIMD belongs to Phase 6+).
- LOD tiers / chunk activity states (Phase 6).
- Frontend changes (Phase 5 is backend-only; the Phase-4 DTOs and chunk subscription are unchanged).
- Splitting `AgentMobilityState` into per-state marker components (e.g. `Walking` / `InVehicle` as separate components). Stays as an `enum` stored in a single `AgentMobilityStateComponent`. Per-state markers can come in Phase 5.5 if profiling shows the enum-match is the bottleneck.
- Changing the JSON persistence shape (zero-migration constraint).

## Architecture

### Storage

```rust
pub struct MobilityWorld {
    world: bevy_ecs::world::World,
    schedule: bevy_ecs::schedule::Schedule,
    by_agent_id:   HashMap<AgentId,   bevy_ecs::entity::Entity>,
    by_vehicle_id: HashMap<VehicleId, bevy_ecs::entity::Entity>,
}
```

The `World` is the source of truth. The two `HashMap`s are pure index structures for `agent_dto_for(&AgentId) -> ...` style lookups — they get updated on spawn/despawn. They are NOT iterated in the hot tick path.

### Components

All `#[derive(Component, Debug, Clone, PartialEq)]` unless noted. Grouped by which entity class they live on:

**Shared (Agent + Vehicle):**
- `Position { x: f32, y: f32 }` — current tile-space coordinate.
- `Direction(DirectionDto)` — facing direction for sprite selection.
- `SpriteKey(String)` — sprite catalog key.
- `Dirty` — marker, present iff entity changed this tick (cleared at frame end).

**Agent-only:**
- `AgentMarker` — unit struct, identifies agent entities.
- `StableAgentId(AgentId)` — stable persistence id.
- `AgentMobilityStateComponent(AgentMobilityState)` — the existing enum (Walking/WaitingAtStop/Boarding/InVehicle/Alighting/AtActivity).
- `WalkPlan { stages: Vec<PlanStage>, cursor: usize }` — agent's MATSim-style plan.
- `WalkSpeed(f32)` — tiles per tick when walking.

**Vehicle-only:**
- `VehicleMarker` — unit struct.
- `StableVehicleId(VehicleId)`.
- `VehicleKindComponent(VehicleKind)` — `Car` / `Tram`.
- `RoutePosition { route_id: RouteId, link_index: usize, progress: f32, speed: f32 }`.
- `Capacity(u16)`.
- `Occupants(Vec<AgentId>)`.
- `DwellTicksRemaining(u16)`.

### Resources (Bevy-idiomatic singletons)

```rust
#[derive(Resource)] struct Tick(u64);
#[derive(Resource)] struct Routes(HashMap<RouteId, RouteRecord>);
#[derive(Resource)] struct Stops(HashMap<StopId, StopRecord>);
#[derive(Resource)] struct LinkPolylines(HashMap<LinkId, Vec<(f32, f32)>>);
#[derive(Resource, Default)] struct DirtyAgents(HashSet<Entity>);
#[derive(Resource, Default)] struct DirtyVehicles(HashSet<Entity>);
```

`Routes` / `Stops` / `LinkPolylines` are static-after-seed, so safe as resources. `DirtyAgents` / `DirtyVehicles` are written by advance-systems, read by `build_mobility_delta`, cleared at tick end.

### Systems & Schedule

```rust
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
enum MobilitySet { Advance, Output, Bookkeeping }
```

System ordering (each system is `fn name(...) -> ()` with `Query<...>` / `Res<...>` / `ResMut<...>` params):

```
MobilitySet::Advance:
  walk_advance_system
    Query: &mut AgentMobilityStateComponent, &WalkSpeed, &StableAgentId
    Reads: LinkPolylines, Routes (for activity transitions)
    Mutates: state.progress (if Walking), pushes Entity to DirtyAgents

  vehicle_advance_system
    Query: &mut RoutePosition, &VehicleKindComponent, &StableVehicleId, &mut DwellTicksRemaining
    Reads: Routes, LinkPolylines
    Mutates: progress; on link end, increments link_index; pushes to DirtyVehicles

  stop_arrival_system
    Query: agents with Walking state and pending transit-plan-step
    Reads: Stops
    Mutates: agent state → WaitingAtStop

  boarding_alighting_system
    Query: vehicles at stop (dwell > 0) and waiting agents
    Mutates: agent state → InVehicle / agent state → Walking;
             vehicle.occupants

MobilitySet::Output (.after(Advance)):
  compute_world_coord_system
    Query: &AgentMobilityStateComponent + &mut Position (agents)
           &RoutePosition + &mut Position (vehicles)
    Reads: LinkPolylines, Routes
    Mutates: Position
  
  compute_direction_system
    same shape, mutates Direction

MobilitySet::Bookkeeping (.after(Output)):
  tick_increment_system
    ResMut<Tick>: tick.0 += 1
```

A single `Schedule` runs all systems in the order above. `MobilityWorld::tick(&mut self) -> MobilityDelta` calls `self.schedule.run(&mut self.world)` then drains `DirtyAgents` / `DirtyVehicles` into the returned `MobilityDelta { changed_agents: Vec<AgentRecord>, changed_vehicles: Vec<VehicleRecord> }`. The records are reconstructed from the entity's components at delta-build time, not stored as plain Rust structs.

### Persistence boundary

`AgentRecord` and `VehicleRecord` become **DTO-like Boundary-Snapshot structs** used only at the persistence (and DTO-building) edge. They are not in the hot path.

Two helpers:

```rust
impl MobilityWorld {
    // For serde to JSONB.
    pub fn to_snapshot(&self) -> MobilitySnapshot {
        MobilitySnapshot {
            tick: self.tick(),
            agents:   query_all_agents(&self.world).map(|e| AgentRecord::from_entity(e)).collect(),
            vehicles: query_all_vehicles(&self.world).map(|e| VehicleRecord::from_entity(e)).collect(),
            routes:   self.routes_resource().clone(),
            stops:    self.stops_resource().clone(),
            link_polylines: self.link_polylines_resource().clone(),
        }
    }

    pub fn from_snapshot(snapshot: MobilitySnapshot) -> Self {
        let mut world = Self::empty();
        for record in snapshot.agents { world.spawn_agent_from_record(record); }
        for record in snapshot.vehicles { world.spawn_vehicle_from_record(record); }
        world.set_routes(snapshot.routes);
        // … etc
        world
    }
}
```

`MobilitySnapshot` has a `serde::{Serialize, Deserialize}` impl that produces the **exact same JSON shape** today's `MobilityWorld` serde impl produces. Verified by a round-trip test: load a Phase-3-era snapshot fixture from disk, deserialize, re-serialize, byte-compare.

### Seeding

`seed::tiny_world()` and `seed::from_network(network, density)` keep their signatures. Internally they construct a `MobilityWorld` via `MobilityWorld::empty() + spawn_agent + spawn_vehicle + add_route + add_stop + add_link_polyline`. No public API change for callers.

### Public API summary (unchanged signatures)

```rust
impl MobilityWorld {
    pub fn empty() -> Self;
    pub fn tick(&mut self) -> MobilityDelta;
    pub fn current_tick(&self) -> u64;

    pub fn agent(&self, id: &AgentId) -> Option<AgentRecord>;
    pub fn vehicle(&self, id: &VehicleId) -> Option<VehicleRecord>;
    pub fn agents(&self) -> impl Iterator<Item = AgentRecord> + '_;
    pub fn vehicles(&self) -> impl Iterator<Item = VehicleRecord> + '_;
    pub fn stops(&self) -> impl Iterator<Item = StopRecord> + '_;

    pub fn world_coord_for_agent(&self, id: &AgentId) -> Option<(f32, f32)>;
    pub fn world_coord_for_vehicle(&self, id: &VehicleId) -> Option<(f32, f32)>;
    pub fn direction_for_agent(&self, id: &AgentId) -> Option<DirectionDto>;
    pub fn direction_for_vehicle(&self, id: &VehicleId) -> Option<DirectionDto>;

    pub fn agent_dto_for(&self, id: &AgentId) -> Option<AgentMobilityDto>;
    pub fn vehicle_dto_for(&self, id: &VehicleId) -> Option<VehicleMobilityDto>;
}
```

Today's `pub` fields (`agents`, `vehicles`, `routes`, etc.) become private — accessors take their place. This is part of the migration: any test or external caller that grabbed `world.agents` directly must switch to `world.agents().collect()` etc. Most callers already use accessors; the affected sites get audited and updated in the implementation plan.

### Benchmark

`backend/crates/sim-core/benches/mobility_tick.rs` (criterion harness):

```rust
fn bench_tick_10k_walkers(c: &mut Criterion) {
    let mut world = build_world_with(10_000, 0);
    c.bench_function("tick_10k_walkers", |b| b.iter(|| { world.tick(); }));
}

fn bench_tick_mixed(c: &mut Criterion) {
    let mut world = build_world_with(8_000, 2_000);
    c.bench_function("tick_8k_walkers_2k_vehicles", |b| b.iter(|| { world.tick(); }));
}
```

Informal target: < 1 ms per tick at 10k entities on an Apple M-series. Criterion stores baselines per branch so regressions get flagged.

The benchmark is opt-in (`cargo bench --bench mobility_tick`) and not part of the standard `cargo test` cycle — it's a one-shot perf check, not a CI gate.

## Testing

**Unit tests (new, in `mobility/systems.rs` or co-located):**
- `walk_advance_progresses_walking_agent_by_walk_speed`
- `vehicle_advance_traverses_link_and_advances_index_at_progress_1`
- `vehicle_advance_decrements_dwell_when_at_stop`
- `stop_arrival_transitions_walking_to_waiting_at_stop`
- `boarding_moves_agent_into_vehicle_occupants_and_sets_in_vehicle_state`
- `alighting_removes_from_occupants_and_resumes_walking`
- `compute_world_coord_matches_polyline_interpolation` (sanity check vs Phase 4's existing geometry math)
- `compute_direction_returns_polyline_tangent`
- `tick_increment_advances_by_one`
- `dirty_agents_drained_after_tick`

**Migration tests (new):**
- `phase3_snapshot_round_trips_byte_for_byte` — load a frozen Phase-3 JSON fixture, deserialize into the new ECS `MobilityWorld`, serialize back, assert exact equality.
- `seed_from_network_produces_same_dto_set` — call `seed::from_network` before and after migration with the same network, assert `agents().sorted_by_id() == agents().sorted_by_id()` and same for vehicles.

**Existing tests:** All ~60 existing mobility tests stay green. Tests that built `MobilityWorld { agents: HashMap::new(), ... }` literals get updated to `MobilityWorld::empty()`. This is mechanical — no semantic change.

**Benchmark:**
- `cargo bench --bench mobility_tick` runs both bench functions, criterion prints throughput, baselines stored in `target/criterion/`.

**E2E:** Render-smoke spec continues to assert tick reception and entity-count thresholds. No spec change needed.

## Backward Compatibility

- **JSONB:** snapshot byte-identical to Phase-3-era. Postgres rows in `mobility_snapshots` remain readable. The implementation plan includes a one-time test that loads a committed Phase-3 snapshot fixture and asserts round-trip equality.
- **DTOs:** unchanged.
- **Frontend:** unchanged.
- **WS protocol:** unchanged.

## Risks

1. **Audit of `pub` field reads.** The Phase-4 Task-4 implementer noted that `MobilityWorld.agents` / `.vehicles` are private — but the Phase-3 implementation had them public. Need to verify the actual access pattern at start of implementation; if any external caller still does `world.agents.iter()` we must replace with `world.agents()` accessor first.
2. **Bevy 0.18 ECS API:** we depend on `bevy_ecs` only, not `bevy_app`. Schedule construction without `App` requires `Schedule::new(MobilitySchedule)` + `add_systems` + explicit `schedule.run(&mut world)`. Documented pattern in `bevy-ecs-docs.html`. Test compile-success early in the plan.
3. **System parameter borrow checker.** Systems that touch agents AND vehicles (e.g. boarding) need disjoint `Query`s with `ParamSet` or careful filtering. Plan reserves one task for boarding/alighting specifically because of this.
4. **Persistence round-trip drift.** A subtle bug where `from_snapshot → to_snapshot` reorders fields or changes float-precision would silently change the JSONB. Mitigation: `phase3_snapshot_round_trips_byte_for_byte` test on a checked-in fixture.
5. **Schedule re-invocation.** Bevy schedules can be run multiple times. Each `tick()` call invokes `schedule.run(&mut world)`. Verify with a multi-tick test that state advances correctly across N ticks.

## Success Criteria

- All current tests green (`cargo test --workspace`).
- Two new system unit tests + one round-trip test pass.
- `cargo bench --bench mobility_tick` reports a tick budget < 1 ms at 10k entities (informal; for the record, not a gate).
- Code review: no `HashMap<AgentId, AgentRecord>` and no `HashMap<VehicleId, VehicleRecord>` remain in `mobility.rs` (except the index `by_agent_id` / `by_vehicle_id` for stable-id lookup).
- `clippy --workspace --all-targets -- -D warnings` clean.
- Frontend renders correctly with ~960 walkers + 51 cars + 4 trams (verified in browser after deploy).
- The JSON byte-for-byte equality test passes on a checked-in Phase-3 snapshot fixture.
