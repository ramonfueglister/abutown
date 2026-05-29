# Mobility Tick Performance Rework (Phase 6 Followup)

**Date:** 2026-05-19
**Status:** Spec
**Author:** Claude (with @ramonfueglister)

## §1 — Goal & Success Criterion

When **100 000 active mobility entities exist in subscribed-as-active chunks**,
`MobilityWorld::tick_mobility()` must complete in **≤ 5 ms** mean wall-clock
(release build).

The current cost — measured 2026-05-19 — is **24.4 ms / tick** with the same
population. The progress.md entry from 2026-05-17 reported 13.6 ms, but that
diagnosis ("per-tick chunk-activity filtering in Advance/Output is the
bottleneck") was wrong: the existing `tick_100k_with_5_subscribed_chunks`
bench warms down to ~0 active agents because LOD demotes everything outside
the 5 subscribed chunks. The 21 µs that bench reports today is unchanged —
filtering isn't the bottleneck. The real bottleneck is the per-entity work
each Advance/Output system does when entities ARE active.

### Acceptance

- A new bench `tick_100k_all_active` in `mobility_tick_lod.rs` reports
  mean < 5 ms in release mode.
- All existing tests pass: 158 sim-core, 178 workspace, 158 vitest, browser
  smoke `scripts/smoke-7b.mjs` 9/9 checks.
- Existing benches `mobility_tick` and `tick_100k_with_5_subscribed_chunks`
  do not regress by more than 2× their current values.

### Out of scope

- Phase 7c (Arc-snapshot lock-free reads) — separate spec.
- Pushing to 1M entities — needs sharding / parallel ticks, separate spec.
- Frontend changes; the wire protocol is not touched.
- Vehicle routing semantics; only performance.
- Full SoA denormalization of route data — the cached `Arc<Vec<(f32,f32)>>`
  variant captures most of the win without the complexity tail.

## §2 — Measurement Baseline

Captured via `cargo run --release -p sim-core --example profile_lod_tick`
(2026-05-19, 100 000 walkers + 1 000 cars + 0 trams, all 512 chunks
subscribed, 50 ticks warmup, 30 sample ticks).

Total `tick_mobility()` mean: **24.4 ms** (min 23.8 ms, max 25.1 ms).

Per-system breakdown (sibling-schedule measurement — sum exceeds total
because Commands flush 4× and resources rebuild redundantly, but the
relative ranking holds):

| System | Mean (ms) | % of 24 ms |
|---|---|---|
| `boarding_alighting_system` | 6.59 | 27 % |
| `compute_direction_system` | 3.79 | 16 % |
| `compute_world_coord_system` | 3.73 | 15 % |
| `stop_arrival_system` | 3.60 | 15 % |
| `track_chunk_populations_system` | 2.49 | 10 % |
| `walk_advance_system` | 1.32 | 5 % |
| classify / promote / demote / veh_adv / warm_flow / tick_inc | < 0.1 each | — |

Hot-path analysis:

- `boarding_alighting_system` has **O(N²) nested loops** in phases A.5, B.2,
  and B.3 — for each candidate it scans all 100 k agents to find the one
  with matching `StableAgentId`. `MobilityWorld.by_agent_id: HashMap<AgentId, Entity>`
  already exists for this purpose but is not exposed to systems.
- `compute_world_coord_system` and `compute_direction_system` each do
  2× HashMap lookup per entity (`routes.0.get(&route_id)` →
  `link_polylines.0.get(&link_id)`) every tick, even when the entity is
  still on the same link.
- `stop_arrival_system` iterates all 100 k agents to find the few whose
  walking progress just hit 1.0.
- `track_chunk_populations_system` rebuilds `AgentsByChunk` /
  `VehiclesByChunk` / `ChunkPopulations` from scratch every tick.

## §3 — Architecture

### New resources

```rust
/// AgentId → Entity. Replaces MobilityWorld.by_agent_id (which stays as a
/// thin wrapper that forwards to this resource). Exposing it as a Resource
/// lets systems do O(1) lookups inside ECS queries.
#[derive(Resource, Default)]
pub struct AgentIdIndex(pub HashMap<AgentId, Entity>);

/// Mirror for vehicles.
#[derive(Resource, Default)]
pub struct VehicleIdIndex(pub HashMap<VehicleId, Entity>);
```

`AgentIdIndex` and `VehicleIdIndex` are kept consistent by:

- `MobilityWorld::spawn_agent_from_record` / `spawn_vehicle_from_record`
  inserts into both `by_agent_id` AND the index resource.
- The post-tick sync block in `tick_mobility` that scans for newly spawned /
  despawned entities (promote / demote) updates both.
- (Long-term cleanup: collapse `by_agent_id` to a thin wrapper that reads
  from the resource. Out of scope for this spec — touch only the index
  consistency, leave the API surface.)

### New components

```rust
/// Cached resolved polyline for the link this entity currently traverses.
/// Refreshed by `update_link_polyline_cache_system` (runs first in Advance)
/// when the entity's link changes. Eliminates the per-tick HashMap chain
/// (RouteId → RouteRecord → LinkId → polyline) in compute_world_coord /
/// compute_direction.
#[derive(Component, Clone)]
pub struct CurrentLinkPolyline {
    pub link_id: LinkId,           // sentinel to detect link changes
    pub points: Arc<Vec<(f32, f32)>>,
}

/// Marker for agents whose walking progress reached 1.0 this tick. Only
/// entities with this marker are visited by stop_arrival_system. Added by
/// walk_advance_system (when progress saturates) and removed by
/// stop_arrival_system after the state transition completes.
#[derive(Component)]
pub struct NearStop;
```

`CurrentLinkPolyline` is added to:

- Walking agents (link_id from `AgentMobilityState::Walking`).
- Vehicles (link_id from `routes[route_id].links[link_index]`).

Other agent states (`WaitingAtStop`, `InVehicle`, `AtActivity`, etc.) do not
carry the cache — `compute_world_coord` falls back to the existing
`agent_world_coord` helper for those.

### Resource removed in spirit, kept in form

`MobilityWorld.by_agent_id` and `by_vehicle_id` continue to exist for the
external API (`MobilityWorld::agent`, `vehicle`, etc.) but are no longer the
authoritative index — they mirror the resource. This avoids touching every
external caller.

## §4 — Per-System Optimization Plan

| System | Aktuell | Change | Target |
|---|---|---|---|
| `boarding_alighting` | 6.59 ms | Phase A.5 / B.2 / B.3: replace `for agents.iter()` scans with `AgentIdIndex.get()` + `commands` / direct component access. Phase A.2: build a one-shot `HashMap<(RouteId, link_index, OrderedFloat<progress>), Vec<Entity>>` of vehicles at the start of the system. | ~0.4 ms |
| `compute_world_coord` | 3.73 ms | When `CurrentLinkPolyline` is present, read the cached `Arc<Vec<...>>` directly; only fall back to the existing helper for other agent states. | ~0.6 ms |
| `compute_direction` | 3.79 ms | Same `CurrentLinkPolyline` cache path. | ~0.4 ms |
| `stop_arrival` | 3.60 ms | Add `Query<..., With<NearStop>>` filter. `walk_advance` inserts `NearStop` when it clamps progress to 1.0; `stop_arrival` removes the marker after the state transition. | ~0.2 ms |
| `track_chunk_populations` | 2.49 ms | Incremental update via `Query<(Entity, &Position), (With<AgentMarker>, Changed<Position>)>` — only re-bucket entities whose Position changed this tick. Maintain a `PreviousChunkByEntity` resource to know which bucket to remove from. | ~0.4 ms |
| `walk_advance` | 1.32 ms | No change; cheap. | 1.32 ms |
| Remaining systems | < 0.1 ms each | No change. | < 0.1 ms |
| **Tick mobility total** | **24.4 ms** | | **~3.4 ms ✓** |

### Risks per optimization

- **`CurrentLinkPolyline` invalidation:** when a walker / vehicle moves to a
  new link, the cached `link_id` no longer matches. A small
  `update_link_polyline_cache_system` runs first in the Advance set,
  compares the entity's current `link_id` against the cached one, and
  refreshes the `Arc` from `LinkPolylines` on mismatch. The `Arc` clone is
  pointer-sized so the only expense is the comparison itself (~10 ns per
  entity, ≤ 1 ms total at 100 k).
- **`NearStop` marker:** correctness depends on `walk_advance` being the
  only system that pushes progress to 1.0. The existing code base satisfies
  this (verified: no other system mutates `progress`). A test enforces it
  going forward.
- **Incremental `track_chunk_populations`:** bevy_ecs 0.18 change detection
  is per-component-per-tick. After snapshot hydration, every Position is
  marked changed in the first tick → full rebuild path, which is correct.
  Edge case: if a system writes to `Position` without actually changing it,
  change detection still triggers — we'll absorb the extra bucketing work,
  but correctness is unaffected. A new resource `PreviousChunkByEntity:
  HashMap<Entity, ChunkCoord>` tracks where each entity was last bucketed
  so the system can remove from the old bucket before inserting into the
  new one. `FlowCells` aggregation in the same system keeps its full
  rebuild path — `FlowCells` is small (≤ chunk_count entries) and cheap.
- **`boarding_alighting` vehicle-index table:** building a HashMap keyed by
  `(RouteId, link_index, OrderedFloat<progress>)` over ~1 000 vehicles is
  ~30 µs — negligible. `OrderedFloat` from the `ordered-float` crate, or a
  manual `(i32, i32, u32)` key derived from the floats with a documented
  epsilon.

### Schedule changes

```
MobilitySet::Advance (new ordering)
  update_link_polyline_cache_system        ← NEW, first
  walk_advance_system
  boarding_alighting_system                ← uses AgentIdIndex
  stop_arrival_system                      ← filters With<NearStop>
  vehicle_advance_system
  warm_chunk_flow_system

MobilitySet::Output
  compute_world_coord_system               ← reads CurrentLinkPolyline
  compute_direction_system                 ← reads CurrentLinkPolyline

MobilitySet::LOD
  track_chunk_populations_system           ← incremental via Changed<Position>
  classify_activity_system
  promote_warm_to_active_system            ← also inserts AgentIdIndex
  demote_active_to_warm_system             ← also removes from AgentIdIndex
```

## §5 — Test Strategy

Existing tests must stay green. No semantic change, only performance.

New tests added TDD-first per implementation step:

1. `agent_id_index_stays_consistent_with_spawn_despawn` — index inserts on
   promote-spawn, removes on demote-despawn, contains the same `(AgentId,
   Entity)` pairs as `MobilityWorld.by_agent_id` after each tick.
2. `vehicle_id_index_stays_consistent_with_spawn_despawn` — mirror.
3. `current_link_polyline_invalidates_on_vehicle_link_change` — vehicle
   advances past end of link, cache refreshes next tick.
4. `current_link_polyline_invalidates_on_walker_link_change` — agent
   transitions from `Walking { link_id: A, .. }` to
   `Walking { link_id: B, .. }`, cache refreshes.
5. `near_stop_marker_added_when_walk_progress_reaches_one` — single agent,
   walk_advance saturates progress, marker exists after the tick.
6. `near_stop_marker_removed_after_stop_arrival` — same agent, next tick
   stop_arrival runs, marker removed regardless of which `PlanStage` matched.
7. `incremental_chunk_populations_matches_full_rebuild` — fuzz-style: seed
   1 000 agents, tick 100×, after each tick compare `AgentsByChunk` against
   a full re-build computed from the Query — must be byte-equal.

Bench expansion:

- `tick_100k_all_active` added to `mobility_tick_lod.rs` (new criterion
  function, same file). Subscribes all 512 chunks. Reports baseline first;
  must be < 5 ms after all steps complete.
- `tick_100k_with_5_subscribed_chunks` stays — confirms LOD effectiveness
  isn't regressed.

CLAUDE.md mandates browser smoke for changes that touch the frontend↔backend
boundary. This rework is backend-only and does NOT touch
`WorldSummaryDto`, the delta wire schema, or chunk subscription semantics.
Still, `scripts/smoke-7b.mjs` runs as the final gate before declaring
complete.

## §6 — Implementation Order (TDD)

```
Step 1  Bench scaffolding
        - tick_100k_all_active added to mobility_tick_lod.rs
        - profile_lod_tick example committed (created during spec, formal
          inclusion now)
        - Baseline 24 ms captured + recorded in commit message

Step 2  AgentIdIndex / VehicleIdIndex
        - Tests 1-2 fail
        - Resources added, sync in spawn / promote / demote
        - Tests 1-2 pass
        - Refactor boarding_alighting to use indexes
        - Bench: expect ~17 ms

Step 3  NearStop marker
        - Tests 5-6 fail
        - walk_advance sets marker; stop_arrival reads + clears
        - Tests 5-6 pass; existing tests still green
        - Bench: expect ~14 ms

Step 4  CurrentLinkPolyline cache
        - Tests 3-4 fail
        - update_link_polyline_cache_system added, components inserted at
          spawn, refreshed when link changes
        - compute_world_coord / compute_direction read cache first
        - Tests 3-4 pass; full byte-equal snapshot roundtrip test still
          green (cache is not serialized — derived state)
        - Bench: expect ~8 ms

Step 5  Incremental track_chunk_populations
        - Test 7 fails
        - PreviousChunkByEntity resource added
        - System rewritten to use Changed<Position>
        - Test 7 passes
        - Bench: expect ~5 ms ✓ goal met

Step 6  Stretch (only if needed)
        - compute_direction polyline-direction bucket-cache per link
        - Bench: < 5 ms confirmed across CI runs
```

Each step ends with: red test → green test → re-bench → commit. If a step
under-delivers, halt and re-profile before continuing.

## §7 — File-Level Touch List

- `backend/crates/sim-core/src/mobility/resources.rs` — add
  `AgentIdIndex`, `VehicleIdIndex`, `PreviousChunkByEntity`.
- `backend/crates/sim-core/src/mobility/components.rs` — add
  `CurrentLinkPolyline`, `NearStop`.
- `backend/crates/sim-core/src/mobility/systems.rs` — add
  `update_link_polyline_cache_system`; modify `walk_advance_system`,
  `boarding_alighting_system`, `stop_arrival_system`,
  `compute_world_coord_system`, `compute_direction_system`,
  `track_chunk_populations_system`, `install_systems`.
- `backend/crates/sim-core/src/mobility/mod.rs` — keep `by_agent_id` /
  `by_vehicle_id` mirroring the new resources; insert / remove in
  `spawn_agent_from_record` / `spawn_vehicle_from_record` and the post-tick
  sync.
- `backend/crates/sim-core/benches/mobility_tick_lod.rs` — add
  `tick_100k_all_active`.
- `backend/crates/sim-core/examples/profile_lod_tick.rs` — created during
  measurement (uncommitted at spec time); keep as a debugging tool, commit
  alongside Step 1.
- `progress.md` — append an entry describing baseline, real bottleneck,
  outcome.

No frontend, persistence, or wire-protocol files are touched.

## §8 — Rollback

If any step regresses correctness (cargo / vitest / smoke), revert that
step's commit. The TDD ordering ensures one optimization per commit, so
rollback granularity is fine. Snapshot format is unchanged, so DB rollback
is not needed.
