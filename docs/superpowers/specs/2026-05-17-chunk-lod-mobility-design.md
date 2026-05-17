# Chunk-LOD Mobility (Phase 6)

> **Phase 6 of the million-agent roadmap.** Parent: `docs/superpowers/specs/2026-05-16-million-agent-roadmap-design.md`.

## Purpose

After Phase 5 `MobilityWorld` runs on bevy_ecs at 1.44 ms/tick for 10k entities. The next ceiling is the per-entity tick cost: at 100k entities the budget is ~14 ms, and at 1M it would be ~140 ms — far past the 100 ms slot of a 10 Hz tick. The fix is **chunk-LOD**: only entities in chunks observed by a connected client tick at full fidelity; chunks with population but no observer simulate via a coarse gravity-flow aggregate at 1 Hz; chunks with neither observer nor population skip entirely.

This phase wires four `MobilityActivity` states (`Hot`/`Active`/`Warm`/`Asleep`) to mobility ticks, adds a `FlowCell` aggregate per chunk, and implements promote/demote transitions that preserve total population across LOD boundaries. The architecture follows the SOTA pattern documented in `docs/literature/agent-simulation/dynamic-lod-large-scale-agent-urban-simulations-aamas2011.pdf` and the Citybound / Unity Mass / Unreal Mass design playbook.

After Phase 6 the backend can sustain ~1M entities total when typical client viewports cover ~10-20 chunks. A new criterion benchmark `mobility_tick_lod` measures 100k entities with 5 subscribed chunks; target <5 ms/tick.

## Non-Goals

- Per-chunk persistence partitioning (Phase 7). `MobilitySnapshot` stays a single JSONB row; the LOD state lives inside that row.
- Exposing chunk-activity to clients (server-internal LOD; clients only see fewer entities in non-subscribed chunks via Phase 4's AoI filter).
- Asleep → Active direct transitions (always Asleep → Warm → Active via population threshold + viewport activation).
- Continuous LOD (we use the existing four discrete states, no interpolation).
- TilePulse-side activity changes — the existing `scheduler::classify_chunk_activity(player_count, dirty_tile_pressure)` for tile pulses stays untouched. Phase 6 adds a parallel mobility classifier.
- Plan-driven warm simulation (MATSim activity schedules in warm state) — deferred.

## Architecture

### MobilityActivity enum

```rust
pub enum MobilityActivity { Hot, Active, Warm, Asleep }
```

Driven from two signals plus hysteresis:

```rust
pub fn classify_chunk_mobility_activity(
    subscribers: u8,
    population: u32,
    previous: MobilityActivity,
    cooldown_remaining: u8,
) -> MobilityActivity {
    let target = if subscribers >= 2 {
        MobilityActivity::Hot
    } else if subscribers == 1 {
        MobilityActivity::Active
    } else if population > 0 {
        MobilityActivity::Warm
    } else {
        MobilityActivity::Asleep
    };
    if target == previous || cooldown_remaining == 0 { target } else { previous }
}
```

Hysteresis: when a chunk transitions, cooldown is set to **30 ticks** (~3 seconds at 10 Hz). During cooldown the chunk stays in the previous state. Prevents thrashing on rapid camera pans.

### Resources

All new resources are inserted in `MobilityWorld::empty()` alongside the existing six (Phase 5):

```rust
#[derive(Resource, Default)] pub struct ChunkActivities(pub HashMap<ChunkCoord, MobilityActivity>);
#[derive(Resource, Default)] pub struct ChunkActivityCooldowns(pub HashMap<ChunkCoord, u8>);
#[derive(Resource, Default)] pub struct FlowCells(pub HashMap<ChunkCoord, FlowCell>);
#[derive(Resource, Default)] pub struct ChunkSubscribers(pub HashMap<ChunkCoord, u8>);

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FlowCell {
    pub population: f32,
    pub outflow: HashMap<ChunkCoord, f32>,
    pub attractiveness: f32,
    pub last_tick: u64,
}
```

`ChunkSubscribers` is populated by the WS task at the moment of each `ChunkSubscribe`/`ChunkUnsubscribe`; the resource is the running tally across all connections. `FlowCells` is per-chunk aggregate state for Warm chunks.

### System schedule additions

The existing Phase-5 schedule has `MobilitySet::Advance` → `Output` → `Bookkeeping`. Add a new set `MobilitySet::LOD` that runs FIRST:

```
MobilitySet::LOD:
  classify_activity_system           — write ChunkActivities + decrement cooldowns
  promote_warm_to_active_system      — on transitions, spawn agents from FlowCell
  demote_active_to_warm_system       — on transitions, drain agents into FlowCell

MobilitySet::Advance (after LOD):
  walk_advance_system        — filter: only agents in Active/Hot chunks
  vehicle_advance_system     — filter: only vehicles in Active/Hot chunks
  stop_arrival_system        — same
  boarding_alighting_system  — same
  warm_chunk_flow_system     — every 10th tick, updates FlowCells per gravity model

MobilitySet::Output (after Advance):
  compute_world_coord_system  — only for entities in Active/Hot chunks
  compute_direction_system    — same

MobilitySet::Bookkeeping (after Output):
  tick_increment_system  — unchanged
```

#### `classify_activity_system`

For each chunk in the union of `ChunkSubscribers.keys()` ∪ chunks containing any agent/vehicle:
- Look up `subscribers` from `ChunkSubscribers`.
- Compute `population` by querying agents + vehicles whose `Position` is in this chunk.
- Read previous state from `ChunkActivities`.
- Read cooldown from `ChunkActivityCooldowns` (default 0).
- Call `classify_chunk_mobility_activity`, get new state.
- If state changed: set `cooldown_remaining = 30`, write new state to `ChunkActivities`, also record the transition in a per-tick `ChunkTransitions(Vec<(ChunkCoord, MobilityActivity, MobilityActivity)>)` resource (transient, drained by promote/demote systems).
- Else: decrement cooldown if > 0.

#### `promote_warm_to_active_system`

For each transition in `ChunkTransitions` where (prev = Warm, new ∈ {Active, Hot}):
- Get the `FlowCell` for that chunk.
- Take `floor(cell.population)` agents to spawn.
- Use a deterministic seed: `seed = hash(world_id, chunk_coord, current_tick)`.
- For each agent:
  - Pick a random corridor or arterial that passes through this chunk (from `LinkPolylines` indexed by chunks — see below).
  - Spawn with `AgentMobilityState::Walking { link_id, progress: rand_in_chunk(link_id, chunk) }`.
  - Generate plan: `vec![PlanStage::Activity { activity_id: format!("activity:lod:{chunk}:{seed_idx}") }]` (one-stage filler — agents that emerge from LOD don't have rich plans).
- Subtract spawned count from `cell.population` (the fractional remainder stays).
- Clear `cell.outflow` (it had per-tick outflow rates assuming aggregate; now we have individuals).

#### `demote_active_to_warm_system`

For each transition where (prev ∈ {Active, Hot}, new = Warm):
- Find all agents whose `Position` is in this chunk.
- `cell.population += count_of_agents`.
- For each agent, determine the **destination chunk** from their state:
  - `Walking { link_id, progress }` → end-chunk of `link_id` (computed from polyline last point).
  - `WaitingAtStop` → chunk containing the stop.
  - `InVehicle` → chunk of the vehicle's current position.
  - `AtActivity` → this chunk (no outflow).
- Increment `cell.outflow[destination]` by `1 / activity_threshold_ticks` (so the outflow rate is "1 agent per N ticks").
- Despawn the agents and remove from `by_agent_id`.
- Same treatment for vehicles in the chunk (count + outflow + despawn).

#### `warm_chunk_flow_system`

Runs every **10th** tick (gated by `Tick % 10 == 0`). For each warm chunk with `cell.population > 0`:
- For each `(destination, rate)` in `cell.outflow`:
  - Amount flowing this 10-tick window: `delta = rate * 10`.
  - Transfer: `cell.population -= delta; flow_cells[destination].population += delta`.
- Update `cell.last_tick = current_tick`.
- If `cell.population` drops below 0.001 and `cell.outflow` is empty, the chunk is logically Asleep — but the `classify_activity_system` next tick handles the actual transition.

Gravity-model formula (used to initialize outflow when populations are added on demote, not redone every tick):

```
flow(A → B) ∝ population(A) * attractiveness(B) / distance(A, B)^2
```

`attractiveness` is initialized from the static city network: chunks with arterials or stops have higher attractiveness. Computed once at network load.

#### `Asleep` handling

No system runs against an Asleep chunk. The chunk's `FlowCell` stays at zero population. When wake to Warm (i.e. a transition where new = Warm and `cell.population` was already > 0 due to a flow inbound — this happens when a neighbor warm chunk pushes population into us), `warm_chunk_flow_system` picks it up.

A direct Asleep → Active is impossible by construction: the chunk must first get population from somewhere (a neighbor warm chunk's flow OR a client subscribes and sees zero entities, in which case the chunk goes Active-empty). The latter case: Active with population 0 is fine — `walk_advance_system` has nothing to process.

### Population query

`classify_activity_system` needs O(chunk_count) population lookups per tick. Building it from `agents()` + `vehicles()` (Phase-5 accessors) is O(entity_count × log entity_count) every tick — too expensive at 100k.

Add a derived resource `ChunkPopulations(HashMap<ChunkCoord, u32>)` updated incrementally by a system that runs in `MobilitySet::Output` (after Position is up-to-date):

```rust
fn track_chunk_populations_system(
    agents: Query<&Position, With<AgentMarker>>,
    vehicles: Query<&Position, With<VehicleMarker>>,
    mut populations: ResMut<ChunkPopulations>,
) {
    populations.0.clear();
    for pos in agents.iter() {
        let chunk = chunk_of(pos.x, pos.y, 32);
        *populations.0.entry(chunk).or_insert(0) += 1;
    }
    for pos in vehicles.iter() {
        let chunk = chunk_of(pos.x, pos.y, 32);
        *populations.0.entry(chunk).or_insert(0) += 1;
    }
}
```

Cost: O(entity_count) per tick — acceptable since we only iterate Active/Hot entities (filtered by Position-belongs-to-active-chunk), which is bounded by viewport size × density.

Actually correction: at the moment `track_chunk_populations_system` runs, agents in Warm chunks have already been despawned (demote pass). So we should ALSO add FlowCell populations:

```rust
for (chunk, cell) in &flow_cells.0 {
    *populations.0.entry(*chunk).or_insert(0) += cell.population.floor() as u32;
}
```

### `ChunkSubscribers` resource population

The WS task currently maintains per-connection `subscription: HashSet<ChunkCoord>` (Phase 4). On `ChunkSubscribe`/`ChunkUnsubscribe`, after updating per-connection state, also update the world-level `ChunkSubscribers`:

```rust
fn update_global_subscribers(runtime: &mut SimulationRuntime, before: &HashSet<ChunkCoord>, after: &HashSet<ChunkCoord>) {
    let mut subs = runtime.mobility.world.resource_mut::<ChunkSubscribers>();
    for added in after.difference(before) {
        *subs.0.entry(*added).or_insert(0) += 1;
    }
    for removed in before.difference(after) {
        let entry = subs.0.entry(*removed).or_insert(0);
        *entry = entry.saturating_sub(1);
        if *entry == 0 {
            subs.0.remove(removed);
        }
    }
}
```

On client disconnect (WS close), decrement all subscriptions held by that connection.

### Persistence

`MobilitySnapshot` (the JSON-boundary struct) gains two fields with `#[serde(default)]` for backward compat:

```rust
pub struct MobilitySnapshot {
    pub tick: u64,
    pub agents: HashMap<AgentId, AgentRecord>,
    pub vehicles: HashMap<VehicleId, VehicleRecord>,
    pub stops: HashMap<StopId, StopRecord>,
    pub routes: HashMap<RouteId, RouteRecord>,
    pub link_polylines: HashMap<LinkId, Vec<(f32, f32)>>,
    #[serde(default)] pub flow_cells: HashMap<ChunkCoord, FlowCell>,
    #[serde(default)] pub chunk_activities: HashMap<ChunkCoord, MobilityActivity>,
}
```

Phase-5 snapshots (no `flow_cells` / `chunk_activities` fields) deserialize fine: defaults to empty maps → all chunks classified as `Asleep` on first tick → the first `track_chunk_populations_system` + `classify_activity_system` pass transitions populated chunks to Warm/Active. Zero-migration.

### Bevy parallel execution (deferred)

The schedule remains single-threaded as in Phase 5. Phase 6 keeps the systems' borrow-set narrow enough to enable multi-threaded execution later (Phase 8 production hardening), but does not switch the executor here.

## Testing

**Unit:**
- `classify_chunk_mobility_activity` × the (subscribers, population) matrix.
- Hysteresis: rapid transition is suppressed during cooldown.
- `warm_chunk_flow_system` advances populations per gravity formula.
- `promote_warm_to_active_system` spawns N agents from `floor(population)`, with deterministic IDs.
- `demote_active_to_warm_system` collapses agents into FlowCell, populates outflow correctly.
- `track_chunk_populations_system` counts agents + vehicles + flow cells correctly.

**Integration:**
- Tick cycle: chunk goes Hot → (lose subscriber) → Active → (lose subscriber) → Warm (population still > 0) → (population evaporates via outflow) → Asleep.
- Promote/demote round-trip: spawn 5 agents in Active chunk; demote to Warm; promote back; assert ~5 agents respawned (modulo fractional rounding).
- Round-trip persistence: snapshot with `flow_cells` + `chunk_activities` populated, deserialize, re-serialize, byte equal.
- Backward-compat: existing `phase3-mobility-snapshot.json` parses and re-serializes (with empty added fields).

**Benchmark:**
- New `mobility_tick_lod` bench: 100k entities seeded across 100 chunks, only 5 subscribed (Active), all others Warm. Target <5 ms/tick.

## Risks

1. **Determinism across promote/demote.** Sampling agents on promote uses RNG seeded by `(world_id, chunk_coord, tick)`. Different replays of the same sim must produce the same agents. Tested.
2. **Population drift.** Floating-point flow with `f32` accumulators may drift over thousands of ticks. Mitigation: snap `cell.population` to `f64` internally if drift bites; for Phase 6 acceptance, accept `f32` and assert in tests that population is conserved to 0.1% over 1000 ticks.
3. **Subscriber-state mismatch.** If `ChunkSubscribers` falls out of sync with per-connection state (e.g. due to a WS-task bug), Hot/Active misclassification. Defense: an invariant assertion in debug mode that the global tally equals the sum of per-connection sets.
4. **Bevy filter cost.** Adding `Filter<...>` to every advance/output system has a per-tick query-setup cost. Profile after Phase 6 ships — if it dominates, switch to a `MarkerInActiveChunk` component sparsely updated by `classify_activity_system`.

## Success criteria

- All Phase-5 tests stay green.
- New unit tests for activity classifier + flow system + promote/demote pass.
- Round-trip test for new persistence fields passes.
- Backward-compat round-trip for Phase-5 fixture passes.
- `mobility_tick_lod` benchmark reports < 5 ms/tick at 100k entities with 5 subscribed chunks.
- Clippy clean across workspace.
- Live verification: launch backend, subscribe to one chunk, observe Warm flows in `/mobility` JSON output.
