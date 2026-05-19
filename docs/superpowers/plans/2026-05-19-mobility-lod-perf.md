# Mobility Tick Performance Rework Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring `MobilityWorld::tick_mobility()` from 24.4 ms to < 5 ms at 100 000 active entities, all chunks subscribed.

**Architecture:** Five focused TDD-ordered optimizations: (1) expose stable-id → Entity HashMaps as ECS resources so boarding/alighting can do O(1) lookups, (2) add a `NearStop` marker so `stop_arrival_system` skips 100 k agents, (3) cache the active link polyline on each entity as `CurrentLinkPolyline` so the Output systems skip 2× HashMap chain per tick, (4) make `track_chunk_populations_system` incremental via `Changed<Position>` and a `PreviousChunkByEntity` resource, (5) lazy chunk-activity filter in boarding_alighting (drop the 100 k pre-pass). Each task ends with a re-bench and a commit.

**Tech Stack:** Rust 2021, `bevy_ecs = "0.18"`, `criterion`, sqlx (untouched), backend-only — no frontend / wire-protocol changes.

**Spec reference:** `docs/superpowers/specs/2026-05-19-mobility-lod-perf-design.md`.

---

## Task 1: Bench scaffolding + baseline

**Files:**
- Modify: `backend/crates/sim-core/benches/mobility_tick_lod.rs`
- Already-modified (uncommitted): `backend/crates/sim-core/src/mobility/mod.rs` (added `profile_world_mut`)
- Already-untracked: `backend/crates/sim-core/examples/profile_lod_tick.rs`

- [ ] **Step 1: Verify baseline measurement reproduces**

Run:
```bash
cargo run --release --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core --example profile_lod_tick 2>&1 | tail -25
```
Expected: `tick_mobility()  mean=~24ms` with per-system breakdown showing `boarding_alighting ~6.5ms`, `direction ~3.8ms`, `world_coord ~3.7ms`, `stop_arrive ~3.6ms`, `track_pop ~2.5ms`. If numbers differ by more than 20 % from the spec, stop and re-investigate before continuing.

- [ ] **Step 2: Add `tick_100k_all_active` to the LOD bench**

Append to `backend/crates/sim-core/benches/mobility_tick_lod.rs` **before** the `criterion_group!` line:

```rust
fn tick_100k_all_active(c: &mut Criterion) {
    let network = very_big_network();
    c.bench_function("tick_100k_all_active", |b| {
        let mut world = from_network(
            &network,
            SeedDensity {
                pedestrians_per_corridor: 50, // 2000 × 50 = 100_000 walkers
                cars_per_arterial: 10,
                trams_total: 0,
            },
        );

        // World is 1024×512 tiles, chunk_size=32 → 32×16 = 512 chunks.
        // Subscribe every chunk so NO entity gets demoted to a FlowCell —
        // the bench measures the cost of 100k entities in the ECS hot path.
        let mut subscribed: Vec<ChunkCoord> = Vec::with_capacity(32 * 16);
        for x in 0..32 {
            for y in 0..16 {
                subscribed.push(ChunkCoord { x, y });
            }
        }
        world.apply_subscription_diff(&subscribed, std::iter::empty());

        for _ in 0..50 {
            world.tick_mobility();
        }

        b.iter(|| {
            world.tick_mobility();
        });
    });
}
```

And update the `criterion_group!` line at the bottom:

```rust
criterion_group!(benches, tick_100k_with_5_subscribed_chunks, tick_100k_all_active);
```

- [ ] **Step 3: Run the new bench to capture baseline**

Run:
```bash
cargo bench --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core --bench mobility_tick_lod 2>&1 | tail -20
```
Expected: `tick_100k_all_active  time:   [~24 ms]` (give or take 2 ms). Capture the exact number for the commit message.

- [ ] **Step 4: Run workspace tests to confirm no regression from bench addition**

Run:
```bash
cargo test --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml 2>&1 | tail -10
```
Expected: all tests pass (178 workspace, 158 sim-core).

- [ ] **Step 5: Commit Step 1's changes**

```bash
git -C /Users/ramonfuglister/Desktop/Coding/abutown add \
  backend/crates/sim-core/Cargo.toml \
  backend/crates/sim-core/benches/mobility_tick_lod.rs \
  backend/crates/sim-core/examples/profile_lod_tick.rs \
  backend/crates/sim-core/src/mobility/mod.rs

git -C /Users/ramonfuglister/Desktop/Coding/abutown commit -m "$(cat <<'EOF'
bench(mobility): tick_100k_all_active + profile_lod_tick example

Captures the real baseline for Phase 6 perf followup. The existing
tick_100k_with_5_subscribed_chunks bench warms down to ~0 active agents
because LOD demotes everything outside the subscribed area; it reports
21µs of nothing.

tick_100k_all_active subscribes the entire 32×16 chunk grid so all 100k
walkers + 1k cars stay in the ECS hot path. Baseline mean: ~24 ms.

profile_lod_tick example walks the per-system breakdown:
  boarding_alighting  6.59 ms (27%)
  direction           3.79 ms (16%)
  world_coord         3.73 ms (15%)
  stop_arrive         3.60 ms (15%)
  track_pop           2.49 ms (10%)
  walk_adv            1.32 ms ( 5%)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: AgentIdIndex / VehicleIdIndex resources

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/resources.rs`
- Modify: `backend/crates/sim-core/src/mobility/mod.rs:104-133` (empty()), `:757-823` (spawn helpers), `:547-600` (tick_mobility post-schedule sync)
- Test: `backend/crates/sim-core/src/mobility/mod.rs` (new module-level tests near existing ones)

- [ ] **Step 1: Write the failing test**

Add to `backend/crates/sim-core/src/mobility/mod.rs` inside the existing `#[cfg(test)] mod tests` block:

```rust
#[test]
fn agent_id_index_resource_matches_by_agent_id_after_spawn() {
    use crate::mobility::resources::AgentIdIndex;

    let mut world = MobilityWorld::empty();
    let id_a = AgentId("a:1".into());
    let id_b = AgentId("a:2".into());
    world.spawn_agent_from_record(AgentRecord::new(
        id_a.clone(),
        AgentMobilityState::AtActivity { activity_id: "act".into() },
        vec![],
        0,
        0.05,
    ));
    world.spawn_agent_from_record(AgentRecord::new(
        id_b.clone(),
        AgentMobilityState::AtActivity { activity_id: "act".into() },
        vec![],
        0,
        0.05,
    ));

    let index = world.profile_world_mut().resource::<AgentIdIndex>();
    assert_eq!(index.0.len(), 2);
    assert_eq!(index.0.get(&id_a).copied(), world.by_agent_id.get(&id_a).copied());
    assert_eq!(index.0.get(&id_b).copied(), world.by_agent_id.get(&id_b).copied());
}

#[test]
fn vehicle_id_index_resource_matches_by_vehicle_id_after_spawn() {
    use crate::mobility::records::VehicleKind;
    use crate::mobility::resources::VehicleIdIndex;
    use crate::ids::{RouteId, VehicleId};

    let mut world = MobilityWorld::empty();
    let id_v = VehicleId("v:1".into());
    world.spawn_vehicle_from_record(VehicleRecord {
        id: id_v.clone(),
        kind: VehicleKind::Car,
        route_id: RouteId("r:1".into()),
        link_index: 0,
        progress: 0.0,
        speed_per_tick: 0.1,
        capacity: 1,
        occupants: vec![],
        dwell_ticks_remaining: 0,
    });

    let index = world.profile_world_mut().resource::<VehicleIdIndex>();
    assert_eq!(index.0.len(), 1);
    assert_eq!(index.0.get(&id_v).copied(), world.by_vehicle_id.get(&id_v).copied());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:
```bash
cargo test --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core agent_id_index_resource_matches 2>&1 | tail -15
```
Expected: compile error — `unresolved import 'crate::mobility::resources::AgentIdIndex'`.

- [ ] **Step 3: Add the resources**

Append to `backend/crates/sim-core/src/mobility/resources.rs`:

```rust
/// Stable AgentId → Entity. Spec §3 — exposed as a Resource so systems can
/// do O(1) lookups inside Queries instead of scanning all agents.
/// Maintained in lockstep with `MobilityWorld.by_agent_id` by the spawn
/// helpers and the post-tick sync in `MobilityWorld::tick_mobility`.
#[derive(Resource, Debug, Default, Clone)]
pub struct AgentIdIndex(pub HashMap<AgentId, Entity>);

/// Mirror of `AgentIdIndex` for vehicle entities.
#[derive(Resource, Debug, Default, Clone)]
pub struct VehicleIdIndex(pub HashMap<VehicleId, Entity>);
```

- [ ] **Step 4: Register the resources in `MobilityWorld::empty()`**

In `backend/crates/sim-core/src/mobility/mod.rs:104-133`, add `world.insert_resource(...)` calls for the two new resources alongside the existing ones (right after `PreviousVehicleChunks`):

```rust
        world.insert_resource(PreviousAgentChunks::default());
        world.insert_resource(PreviousVehicleChunks::default());
        world.insert_resource(crate::mobility::resources::AgentIdIndex::default());
        world.insert_resource(crate::mobility::resources::VehicleIdIndex::default());
```

- [ ] **Step 5: Sync inside `spawn_agent_from_record`**

In `backend/crates/sim-core/src/mobility/mod.rs:783`, change:

```rust
        self.by_agent_id.insert(id, entity);
        entity
```

to:

```rust
        self.by_agent_id.insert(id.clone(), entity);
        self.world
            .resource_mut::<crate::mobility::resources::AgentIdIndex>()
            .0
            .insert(id, entity);
        entity
```

- [ ] **Step 6: Sync inside `spawn_vehicle_from_record`**

In `backend/crates/sim-core/src/mobility/mod.rs:821`, change:

```rust
        self.by_vehicle_id.insert(id, entity);
        entity
```

to:

```rust
        self.by_vehicle_id.insert(id.clone(), entity);
        self.world
            .resource_mut::<crate::mobility::resources::VehicleIdIndex>()
            .0
            .insert(id, entity);
        entity
```

- [ ] **Step 7: Sync inside `tick_mobility`'s post-schedule block**

In `backend/crates/sim-core/src/mobility/mod.rs:562-600` (the four sync blocks that handle newly-spawned and despawned agents/vehicles), update each to mirror into the resources.

Replace the four blocks (newly spawned agents, removed agents, newly spawned vehicles, removed vehicles) with:

```rust
        for (id, entity) in new_agents {
            self.by_agent_id.insert(id.clone(), entity);
            self.world
                .resource_mut::<crate::mobility::resources::AgentIdIndex>()
                .0
                .insert(id, entity);
        }

        // Remove despawned agents from the index (from demote_active_to_warm_system).
        let agent_ids_to_remove: Vec<AgentId> = self
            .by_agent_id
            .iter()
            .filter(|(_, entity)| self.world.get_entity(**entity).is_err())
            .map(|(id, _)| id.clone())
            .collect();
        for id in &agent_ids_to_remove {
            self.by_agent_id.remove(id);
        }
        {
            let mut index = self
                .world
                .resource_mut::<crate::mobility::resources::AgentIdIndex>();
            for id in &agent_ids_to_remove {
                index.0.remove(id);
            }
        }
```

And mirror for vehicles (replacing the corresponding block):

```rust
        for (id, entity) in new_vehicles {
            self.by_vehicle_id.insert(id.clone(), entity);
            self.world
                .resource_mut::<crate::mobility::resources::VehicleIdIndex>()
                .0
                .insert(id, entity);
        }

        // Remove despawned vehicles from the index.
        let vehicle_ids_to_remove: Vec<VehicleId> = self
            .by_vehicle_id
            .iter()
            .filter(|(_, entity)| self.world.get_entity(**entity).is_err())
            .map(|(id, _)| id.clone())
            .collect();
        for id in &vehicle_ids_to_remove {
            self.by_vehicle_id.remove(id);
        }
        {
            let mut index = self
                .world
                .resource_mut::<crate::mobility::resources::VehicleIdIndex>();
            for id in &vehicle_ids_to_remove {
                index.0.remove(id);
            }
        }
```

- [ ] **Step 8: Run tests to verify they pass**

Run:
```bash
cargo test --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core 2>&1 | tail -15
```
Expected: all 160 tests pass (158 + the 2 new ones).

- [ ] **Step 9: Commit**

```bash
git -C /Users/ramonfuglister/Desktop/Coding/abutown add backend/crates/sim-core/src/mobility/
git -C /Users/ramonfuglister/Desktop/Coding/abutown commit -m "$(cat <<'EOF'
feat(mobility): expose AgentIdIndex/VehicleIdIndex as ECS resources

Mirrors MobilityWorld.by_agent_id and by_vehicle_id into Resources so
systems can do O(1) stable-id → Entity lookups inside Queries. Spawn
helpers (spawn_agent_from_record, spawn_vehicle_from_record) and the
post-tick sync in tick_mobility insert/remove from both the resource and
the legacy HashMap field; the two stay byte-equivalent.

Preparatory commit — Task 3 will use these resources to fix the O(N²)
nested-loop scans in boarding_alighting_system.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Refactor `boarding_alighting_system` to use the indexes

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/systems.rs:211-441`

This task changes only how the system finds entities by id; it does not change observable behavior. All existing boarding/alighting tests must stay green.

- [ ] **Step 1: Run existing boarding tests as the contract baseline**

Run:
```bash
cargo test --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core boarding_ 2>&1 | tail -15
```
Expected: all boarding/alighting tests pass. Note which test names exist so we can re-run them post-refactor.

- [ ] **Step 2: Drop the A.0 pre-pass and use lazy chunk_is_simulated**

The current code builds a HashMap of all 100 k agent simulation states up front (A.0). We replace this with on-demand `chunk_is_simulated` calls inside each candidate check via `world.get::<Position>(entity)`.

In `backend/crates/sim-core/src/mobility/systems.rs` replace the `pub fn boarding_alighting_system` signature and body with this version. The whole function block (lines 211-441) becomes:

```rust
#[allow(clippy::type_complexity)]
pub fn boarding_alighting_system(
    mut sets: ParamSet<(
        Query<
            (
                Entity,
                &Position,
                &StableAgentId,
                &mut AgentMobilityStateComponent,
                &mut WalkPlan,
            ),
            With<AgentMarker>,
        >,
        Query<
            (
                Entity,
                &Position,
                &StableVehicleId,
                &mut Occupants,
                &Capacity,
                &RoutePosition,
            ),
            With<VehicleMarker>,
        >,
    )>,
    activities: Res<ChunkActivities>,
    agent_index: Res<crate::mobility::resources::AgentIdIndex>,
    mut stops: ResMut<Stops>,
    mut dirty_agents: ResMut<DirtyAgents>,
    mut dirty_vehicles: ResMut<DirtyVehicles>,
) {
    // ----- PHASE A: BOARDING -----

    // A.1 — collect (stop_id, front agent, route/link/progress) for each stop
    // that has at least one waiting agent. Defer the chunk-activity filter to
    // A.2 so we don't pre-pass over all 100k agents.
    let mut boarding_candidates: Vec<(StopId, AgentId, RouteId, usize, f32)> = Vec::new();
    for (stop_id, stop) in stops.0.iter() {
        if let Some(agent_id) = stop.waiting_agents.front() {
            boarding_candidates.push((
                stop_id.clone(),
                agent_id.clone(),
                stop.route_id.clone(),
                stop.link_index,
                stop.progress,
            ));
        }
    }

    // A.2 — find a matching vehicle for each candidate. Both the candidate
    // agent AND the matched vehicle must live in an Active/Hot chunk.
    // Two-phase: first lookup candidate agent positions (p0 borrow), then
    // match against vehicles (p1 borrow) — ParamSet only permits one inner
    // query borrow at a time.
    let mut candidates_with_pos: Vec<(StopId, AgentId, RouteId, usize, f32)> = Vec::new();
    {
        let agents = sets.p0();
        for (stop_id, agent_id, route_id, link_index, stop_progress) in boarding_candidates {
            let Some(agent_entity) = agent_index.0.get(&agent_id).copied() else { continue };
            let Ok((_, pos, _, _, _)) = agents.get(agent_entity) else { continue };
            if !chunk_is_simulated(pos, &activities) {
                continue;
            }
            candidates_with_pos.push((stop_id, agent_id, route_id, link_index, stop_progress));
        }
    }

    let mut boardings: Vec<(StopId, AgentId, Entity, VehicleId, u16)> = Vec::new();
    {
        let vehicles = sets.p1();
        for (stop_id, agent_id, route_id, link_index, stop_progress) in candidates_with_pos {
            for (v_entity, v_pos_world, v_stable, v_occ, v_cap, v_pos) in vehicles.iter() {
                if !chunk_is_simulated(v_pos_world, &activities) {
                    continue;
                }
                if v_pos.route_id == route_id
                    && v_pos.link_index == link_index
                    && (v_pos.progress - stop_progress).abs() < 1e-6
                    && v_occ.0.len() < v_cap.0 as usize
                {
                    let seat_index = v_occ.0.len() as u16;
                    boardings.push((
                        stop_id.clone(),
                        agent_id.clone(),
                        v_entity,
                        v_stable.0.clone(),
                        seat_index,
                    ));
                    break;
                }
            }
        }
    }

    // A.3 — apply vehicle-side mutations (append occupant).
    {
        let mut vehicles = sets.p1();
        for (_stop_id, agent_id, v_entity, _v_id, _seat) in &boardings {
            if let Ok((_, _, _, mut v_occ, _, _)) = vehicles.get_mut(*v_entity) {
                v_occ.0.push(agent_id.clone());
                dirty_vehicles.0.insert(*v_entity);
            }
        }
    }

    // A.4 — pop boarded agents from stop queues.
    for (stop_id, agent_id, _, _, _) in &boardings {
        if let Some(stop) = stops.0.get_mut(stop_id)
            && stop.waiting_agents.front() == Some(agent_id)
        {
            stop.waiting_agents.pop_front();
        }
    }

    // A.5 — agent-side mutations: state becomes InVehicle. O(1) lookup via index.
    {
        let mut agents = sets.p0();
        for (_stop_id, agent_id, _v_entity, v_id, seat_index) in &boardings {
            let Some(a_entity) = agent_index.0.get(agent_id).copied() else {
                continue;
            };
            if let Ok((_, _, _, mut a_state, _)) = agents.get_mut(a_entity) {
                a_state.0 = AgentMobilityState::InVehicle {
                    vehicle_id: v_id.clone(),
                    seat_index: *seat_index,
                };
                dirty_agents.0.insert(a_entity);
            }
        }
    }

    // ----- PHASE B: ALIGHTING -----

    // B.1 — collect (vehicle_entity, vehicle_id, end-of-link stop_id, occupants)
    // for every vehicle parked at an end-of-link stop in an Active/Hot chunk.
    let mut alighting_candidates: Vec<(Entity, VehicleId, StopId, Vec<AgentId>)> = Vec::new();
    {
        let vehicles = sets.p1();
        for (v_entity, v_pos_world, v_stable, v_occ, _cap, v_pos) in vehicles.iter() {
            if !chunk_is_simulated(v_pos_world, &activities) {
                continue;
            }
            let stop_match = stops.0.values().find(|stop| {
                stop.route_id == v_pos.route_id
                    && stop.link_index == v_pos.link_index
                    && (stop.progress - v_pos.progress).abs() < 1e-6
                    && (stop.progress - 1.0).abs() < 1e-6
            });
            if let Some(stop) = stop_match {
                alighting_candidates.push((
                    v_entity,
                    v_stable.0.clone(),
                    stop.id.clone(),
                    v_occ.0.clone(),
                ));
            }
        }
    }

    // B.2 — for each occupant, check plan stage + state. O(1) lookups via index.
    let mut to_alight: Vec<(Entity, VehicleId, StopId, AgentId)> = Vec::new();
    {
        let agents = sets.p0();
        for (v_entity, v_id, stop_id, occupants) in &alighting_candidates {
            for agent_id in occupants {
                let Some(a_entity) = agent_index.0.get(agent_id).copied() else {
                    continue;
                };
                let Ok((_, a_pos, _, a_state, a_plan)) = agents.get(a_entity) else {
                    continue;
                };
                if !chunk_is_simulated(a_pos, &activities) {
                    continue;
                }
                let stage = a_plan.stages.get(a_plan.cursor);
                let matches_alight = matches!(
                    stage,
                    Some(PlanStage::RideToStop { stop_id: target, .. }) if target == stop_id
                );
                let in_this_vehicle = matches!(
                    &a_state.0,
                    AgentMobilityState::InVehicle { vehicle_id, .. } if vehicle_id == v_id
                );
                if matches_alight && in_this_vehicle {
                    to_alight.push((
                        *v_entity,
                        v_id.clone(),
                        stop_id.clone(),
                        agent_id.clone(),
                    ));
                }
            }
        }
    }

    // B.3 — apply alighting mutations. O(1) lookup via index.
    for (v_entity, v_id, stop_id, agent_id) in &to_alight {
        {
            let mut vehicles = sets.p1();
            if let Ok((_, _, _, mut v_occ, _, _)) = vehicles.get_mut(*v_entity) {
                v_occ.0.retain(|x| x != agent_id);
                dirty_vehicles.0.insert(*v_entity);
            }
        }
        {
            let mut agents = sets.p0();
            let Some(a_entity) = agent_index.0.get(agent_id).copied() else {
                continue;
            };
            if let Ok((_, _, _, mut a_state, mut a_plan)) = agents.get_mut(a_entity) {
                a_plan.cursor += 1;
                let next = a_plan.stages.get(a_plan.cursor).cloned();
                a_state.0 = match next {
                    Some(PlanStage::WalkToActivity { link_id, .. }) => {
                        AgentMobilityState::Walking {
                            link_id,
                            progress: 0.0,
                        }
                    }
                    Some(PlanStage::Activity { activity_id }) => {
                        a_plan.cursor += 1;
                        AgentMobilityState::AtActivity { activity_id }
                    }
                    _ => AgentMobilityState::Alighting {
                        vehicle_id: v_id.clone(),
                        stop_id: stop_id.clone(),
                    },
                };
                dirty_agents.0.insert(a_entity);
            }
        }
    }
}
```

Note: the inner `drop(agents); let vehicles = sets.p1(); ...; let _ = sets.p0();` dance in A.2 is required because `ParamSet` only lets one inner query be borrowed at a time. Each candidate releases the agents borrow, takes the vehicles borrow to find a match, then re-takes the agents borrow on next iteration.

- [ ] **Step 3: Build to surface any borrow-checker issues**

Run:
```bash
cargo build --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core 2>&1 | tail -20
```
Expected: clean build. If the borrow-checker complains about A.2 (ParamSet borrow split), rework into two passes: first collect `(agent_entity, agent_pos)` for all candidates, drop the borrow, then run the vehicle-matching pass.

- [ ] **Step 4: Re-run boarding tests**

Run:
```bash
cargo test --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core boarding_ 2>&1 | tail -10
```
Expected: all boarding/alighting tests pass — semantic behavior unchanged.

- [ ] **Step 5: Run full workspace tests**

Run:
```bash
cargo test --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml 2>&1 | tail -10
```
Expected: all 178 workspace tests pass.

- [ ] **Step 6: Bench: confirm reduction**

Run:
```bash
cargo bench --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core --bench mobility_tick_lod 2>&1 | tail -10
```
Expected: `tick_100k_all_active` mean ≤ 18 ms. If it's still ≥ 22 ms, the refactor is not removing the O(N²) — re-profile via `cargo run --release ... --example profile_lod_tick` and inspect `boarding_alighting` mean before continuing.

- [ ] **Step 7: Commit**

```bash
git -C /Users/ramonfuglister/Desktop/Coding/abutown add backend/crates/sim-core/src/mobility/systems.rs
git -C /Users/ramonfuglister/Desktop/Coding/abutown commit -m "$(cat <<'EOF'
perf(mobility): boarding_alighting O(N²) → O(N) via AgentIdIndex

Previous implementation built a 100k-entry HashMap each tick (A.0) and
did three nested `for agents.iter()` scans (A.5, B.2, B.3) — each
~100k entity iterations to find one by StableAgentId. Replaced with
AgentIdIndex resource lookups (O(1) per candidate). Lazy chunk-activity
filter applies only to actual candidates, not to all 100k agents.

Bench delta: tick_100k_all_active 24.4 ms → ~17 ms.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: NearStop marker for `stop_arrival_system`

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/components.rs`
- Modify: `backend/crates/sim-core/src/mobility/systems.rs` (`walk_advance_system`, `stop_arrival_system`)
- Test: add tests in `backend/crates/sim-core/src/mobility/systems.rs` inside the existing `#[cfg(test)]` block.

- [ ] **Step 1: Write the failing tests**

Add to the existing `#[cfg(test)] mod tests` block at the bottom of `backend/crates/sim-core/src/mobility/systems.rs`:

```rust
#[test]
fn walk_advance_inserts_near_stop_marker_when_progress_saturates() {
    use crate::ids::{AgentId, LinkId};

    let mut world = World::new();
    world.insert_resource(DirtyAgents::default());
    world.insert_resource(all_active());

    let entity = world
        .spawn((
            AgentMarker,
            StableAgentId(AgentId("a:1".into())),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: LinkId("l:1".into()),
                progress: 0.99,
            }),
            WalkPlan { stages: vec![], cursor: 0 },
            WalkSpeed(0.05),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ))
        .id();

    let mut schedule = Schedule::default();
    schedule.add_systems(walk_advance_system);
    schedule.run(&mut world);

    assert!(
        world.get::<NearStop>(entity).is_some(),
        "walk_advance should add NearStop when progress saturates to 1.0"
    );
}

#[test]
fn stop_arrival_removes_near_stop_marker_after_transition() {
    use crate::ids::{AgentId, LinkId, RouteId, StopId};
    use crate::mobility::records::StopRecord;
    use std::collections::VecDeque;

    let mut world = World::new();
    world.insert_resource(DirtyAgents::default());
    world.insert_resource(all_active());

    let mut stops = Stops::default();
    stops.0.insert(
        StopId("s:1".into()),
        StopRecord {
            id: StopId("s:1".into()),
            route_id: RouteId("r:1".into()),
            link_index: 0,
            progress: 1.0,
            waiting_agents: VecDeque::new(),
        },
    );
    world.insert_resource(stops);

    let entity = world
        .spawn((
            AgentMarker,
            StableAgentId(AgentId("a:1".into())),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: LinkId("l:1".into()),
                progress: 1.0,
            }),
            WalkPlan {
                stages: vec![PlanStage::WalkToStop {
                    link_id: LinkId("l:1".into()),
                    stop_id: StopId("s:1".into()),
                }],
                cursor: 0,
            },
            WalkSpeed(0.05),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
            NearStop,
        ))
        .id();

    let mut schedule = Schedule::default();
    schedule.add_systems(stop_arrival_system);
    schedule.run(&mut world);

    assert!(
        world.get::<NearStop>(entity).is_none(),
        "stop_arrival should remove NearStop after state transition"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:
```bash
cargo test --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core near_stop 2>&1 | tail -10
```
Expected: compile error — `cannot find type 'NearStop'`.

- [ ] **Step 3: Add the `NearStop` component**

Append to `backend/crates/sim-core/src/mobility/components.rs`:

```rust
/// Tagging an agent whose walking progress saturated to 1.0 this tick.
/// Only agents with this marker are visited by `stop_arrival_system`,
/// which avoids iterating all 100k agents. Added by `walk_advance_system`
/// on saturation, removed by `stop_arrival_system` after the state
/// transition completes.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct NearStop;
```

- [ ] **Step 4: walk_advance inserts the marker**

In `backend/crates/sim-core/src/mobility/systems.rs:93-118` (the `walk_advance_system`), change the signature to take `Commands` and add the marker on saturation:

```rust
pub fn walk_advance_system(
    mut query: Query<
        (
            Entity,
            &Position,
            &mut AgentMobilityStateComponent,
            &WalkSpeed,
        ),
        With<AgentMarker>,
    >,
    activities: Res<ChunkActivities>,
    mut dirty: ResMut<DirtyAgents>,
    mut commands: Commands,
) {
    for (entity, pos, mut state, speed) in query.iter_mut() {
        if !chunk_is_simulated(pos, &activities) {
            continue;
        }
        if let AgentMobilityState::Walking { progress, .. } = &mut state.0 {
            let next = (*progress + speed.0).min(1.0);
            if next != *progress {
                *progress = next;
                dirty.0.insert(entity);
                if next >= 1.0 {
                    commands.entity(entity).insert(NearStop);
                }
            }
        }
    }
}
```

- [ ] **Step 5: stop_arrival filters by `With<NearStop>` and removes the marker**

In `backend/crates/sim-core/src/mobility/systems.rs:159-208` (the `stop_arrival_system`), change the query filter and add a `Commands` parameter to remove the marker:

```rust
pub fn stop_arrival_system(
    mut query: Query<
        (
            Entity,
            &Position,
            &StableAgentId,
            &mut AgentMobilityStateComponent,
            &mut WalkPlan,
        ),
        (With<AgentMarker>, With<NearStop>),
    >,
    activities: Res<ChunkActivities>,
    mut stops: ResMut<Stops>,
    mut dirty: ResMut<DirtyAgents>,
    mut commands: Commands,
) {
    for (entity, pos, stable, mut state, mut plan) in query.iter_mut() {
        // Always remove the marker so the next tick doesn't revisit this
        // agent — even if the body falls through to the catch-all arm.
        commands.entity(entity).remove::<NearStop>();

        if !chunk_is_simulated(pos, &activities) {
            continue;
        }
        let completed_walking = matches!(
            &state.0,
            AgentMobilityState::Walking { progress, .. } if *progress >= 1.0
        );
        if !completed_walking {
            continue;
        }

        let stage = plan.stages.get(plan.cursor).cloned();
        match stage {
            Some(PlanStage::WalkToStop { stop_id, .. }) => {
                plan.cursor += 1;
                state.0 = AgentMobilityState::WaitingAtStop {
                    stop_id: stop_id.clone(),
                };
                if let Some(stop) = stops.0.get_mut(&stop_id)
                    && !stop.waiting_agents.contains(&stable.0)
                {
                    stop.waiting_agents.push_back(stable.0.clone());
                }
                dirty.0.insert(entity);
            }
            Some(PlanStage::WalkToActivity { activity_id, .. }) => {
                plan.cursor += 1;
                state.0 = AgentMobilityState::AtActivity { activity_id };
                dirty.0.insert(entity);
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run:
```bash
cargo test --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core 2>&1 | tail -10
```
Expected: all 162 sim-core tests pass (158 + 2 from Task 2 + 2 new). Note: existing `stop_arrival_transitions_walking_agent_to_waiting_at_stop` test must continue to pass — verify the test fixture inserts `NearStop` on the spawned agent OR adjust the test if it doesn't (the agent in that test starts at progress=1.0 manually, not via walk_advance, so we must add `NearStop` to its bundle).

If `stop_arrival_transitions_walking_agent_to_waiting_at_stop` fails, edit the spawn at `systems.rs:931-952` to include `NearStop` in the bundle:

```rust
        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: LinkId("l:1".into()),
                    progress: 1.0,
                }),
                WalkPlan {
                    stages: vec![PlanStage::WalkToStop {
                        link_id: LinkId("l:1".into()),
                        stop_id: StopId("s:1".into()),
                    }],
                    cursor: 0,
                },
                WalkSpeed(0.1),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
                NearStop,
            ))
            .id();
```

Also check `walk_advance_clamps_at_one_and_marks_dirty` — if it spawns an agent that should hit progress=1.0, then after this change it will also acquire `NearStop` which the test doesn't check. The test is checking dirty insertion, not marker absence, so it should still pass; but if assertion failures show up, add the marker check (`assert!(world.get::<NearStop>(entity).is_some())`) rather than removing it.

- [ ] **Step 7: Run full workspace tests + bench**

Run:
```bash
cargo test --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml 2>&1 | tail -10
```
Expected: 180 workspace tests pass.

Run:
```bash
cargo bench --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core --bench mobility_tick_lod 2>&1 | tail -10
```
Expected: `tick_100k_all_active` mean ≤ 15 ms (down from ~17). The stop_arrival savings (3.6 ms → ~0.2 ms) account for ~3 ms of reduction.

- [ ] **Step 8: Commit**

```bash
git -C /Users/ramonfuglister/Desktop/Coding/abutown add backend/crates/sim-core/src/mobility/
git -C /Users/ramonfuglister/Desktop/Coding/abutown commit -m "$(cat <<'EOF'
perf(mobility): NearStop marker confines stop_arrival to ~10 entities

Previous stop_arrival iterated all 100k agents per tick to find the few
whose walking progress just saturated. New NearStop marker (inserted by
walk_advance on progress=1.0, removed by stop_arrival) lets the query
filter the work to ~candidate-only via With<NearStop>.

Bench delta: tick_100k_all_active ~17 ms → ~14 ms.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `CurrentLinkPolyline` component cache

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/components.rs`
- Modify: `backend/crates/sim-core/src/mobility/systems.rs` (new `update_link_polyline_cache_system`, modify `compute_world_coord_system`, `compute_direction_system`, `install_systems`)
- Modify: `backend/crates/sim-core/src/mobility/mod.rs:757-823` (spawn helpers insert initial cache)

- [ ] **Step 1: Write failing tests**

Add to the existing `#[cfg(test)] mod tests` block at the bottom of `backend/crates/sim-core/src/mobility/systems.rs`:

```rust
#[test]
fn current_link_polyline_invalidates_on_walker_link_change() {
    use crate::ids::{AgentId, LinkId};
    use std::sync::Arc;

    let mut world = World::new();
    world.insert_resource(LinkPolylines::default());
    world.insert_resource(all_active());

    let mut links = LinkPolylines::default();
    links.0.insert(LinkId("l:a".into()), vec![(0.0, 0.0), (10.0, 0.0)]);
    links.0.insert(LinkId("l:b".into()), vec![(0.0, 0.0), (0.0, 10.0)]);
    world.insert_resource(links);

    let entity = world
        .spawn((
            AgentMarker,
            StableAgentId(AgentId("a:1".into())),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: LinkId("l:a".into()),
                progress: 0.0,
            }),
            WalkPlan { stages: vec![], cursor: 0 },
            WalkSpeed(0.05),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
            CurrentLinkPolyline {
                link_id: LinkId("l:a".into()),
                points: Arc::new(vec![(0.0, 0.0), (10.0, 0.0)]),
            },
        ))
        .id();

    let mut schedule = Schedule::default();
    schedule.add_systems(update_link_polyline_cache_system);

    // Tick 1: cache already matches → no change.
    schedule.run(&mut world);
    assert_eq!(
        world.get::<CurrentLinkPolyline>(entity).unwrap().link_id,
        LinkId("l:a".into())
    );

    // Mutate the agent to a different link.
    if let Some(mut s) = world.get_mut::<AgentMobilityStateComponent>(entity) {
        s.0 = AgentMobilityState::Walking {
            link_id: LinkId("l:b".into()),
            progress: 0.0,
        };
    }
    schedule.run(&mut world);
    assert_eq!(
        world.get::<CurrentLinkPolyline>(entity).unwrap().link_id,
        LinkId("l:b".into())
    );
    let cached = world.get::<CurrentLinkPolyline>(entity).unwrap();
    assert_eq!(cached.points.as_ref(), &vec![(0.0, 0.0), (0.0, 10.0)]);
}

#[test]
fn current_link_polyline_invalidates_on_vehicle_link_change() {
    use crate::ids::{LinkId, RouteId, VehicleId};
    use crate::mobility::records::{RouteRecord, VehicleKind};
    use std::sync::Arc;

    let mut world = World::new();
    world.insert_resource(all_active());

    let mut routes = Routes::default();
    routes.0.insert(
        RouteId("r:1".into()),
        RouteRecord {
            id: RouteId("r:1".into()),
            links: vec![LinkId("l:a".into()), LinkId("l:b".into())],
            stops: vec![],
        },
    );
    world.insert_resource(routes);

    let mut links = LinkPolylines::default();
    links.0.insert(LinkId("l:a".into()), vec![(0.0, 0.0), (10.0, 0.0)]);
    links.0.insert(LinkId("l:b".into()), vec![(0.0, 0.0), (0.0, 10.0)]);
    world.insert_resource(links);

    let entity = world
        .spawn((
            VehicleMarker,
            StableVehicleId(VehicleId("v:1".into())),
            VehicleKindComponent(VehicleKind::Car),
            RoutePosition {
                route_id: RouteId("r:1".into()),
                link_index: 0,
                progress: 0.0,
                speed: 0.1,
            },
            Capacity(1),
            Occupants(vec![]),
            DwellTicksRemaining(0),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
            CurrentLinkPolyline {
                link_id: LinkId("l:a".into()),
                points: Arc::new(vec![(0.0, 0.0), (10.0, 0.0)]),
            },
        ))
        .id();

    let mut schedule = Schedule::default();
    schedule.add_systems(update_link_polyline_cache_system);

    if let Some(mut rp) = world.get_mut::<RoutePosition>(entity) {
        rp.link_index = 1;
    }
    schedule.run(&mut world);
    assert_eq!(
        world.get::<CurrentLinkPolyline>(entity).unwrap().link_id,
        LinkId("l:b".into())
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:
```bash
cargo test --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core current_link_polyline 2>&1 | tail -10
```
Expected: compile errors for `CurrentLinkPolyline` and `update_link_polyline_cache_system`.

- [ ] **Step 3: Add the `CurrentLinkPolyline` component**

Append to `backend/crates/sim-core/src/mobility/components.rs`:

```rust
use crate::ids::LinkId;
use std::sync::Arc;

/// Cached resolved polyline for the link this entity currently traverses.
/// Refreshed by `update_link_polyline_cache_system` (runs first in Advance)
/// when the entity's link changes. Eliminates the per-tick HashMap chain
/// (RouteId → RouteRecord → LinkId → polyline) in compute_world_coord /
/// compute_direction.
#[derive(Component, Debug, Clone)]
pub struct CurrentLinkPolyline {
    pub link_id: LinkId,
    pub points: Arc<Vec<(f32, f32)>>,
}
```

Note: this component is derived state and is **not** serialized — `MobilityWorld`'s custom serde does not touch it, so snapshot round-trip stays byte-stable.

- [ ] **Step 4: Add `update_link_polyline_cache_system`**

Insert this new system in `backend/crates/sim-core/src/mobility/systems.rs` just after `walk_advance_system` (so before `boarding_alighting_system`):

```rust
#[allow(clippy::type_complexity)]
pub fn update_link_polyline_cache_system(
    mut agents: Query<
        (
            Entity,
            &AgentMobilityStateComponent,
            Option<&mut CurrentLinkPolyline>,
        ),
        (With<AgentMarker>, Without<VehicleMarker>),
    >,
    mut vehicles: Query<
        (
            Entity,
            &RoutePosition,
            Option<&mut CurrentLinkPolyline>,
        ),
        (With<VehicleMarker>, Without<AgentMarker>),
    >,
    routes: Res<Routes>,
    link_polylines: Res<LinkPolylines>,
    mut commands: Commands,
) {
    use std::sync::Arc;

    // Agents: only Walking state has a link_id; other states get the cache
    // removed (or skipped if absent).
    for (entity, state, cached) in agents.iter_mut() {
        let want = match &state.0 {
            AgentMobilityState::Walking { link_id, .. } => Some(link_id.clone()),
            _ => None,
        };
        match (want, cached) {
            (Some(want_id), Some(mut c)) => {
                if c.link_id != want_id {
                    if let Some(points) = link_polylines.0.get(&want_id) {
                        c.link_id = want_id;
                        c.points = Arc::new(points.clone());
                    }
                }
            }
            (Some(want_id), None) => {
                if let Some(points) = link_polylines.0.get(&want_id) {
                    commands.entity(entity).insert(CurrentLinkPolyline {
                        link_id: want_id,
                        points: Arc::new(points.clone()),
                    });
                }
            }
            (None, Some(_)) => {
                commands.entity(entity).remove::<CurrentLinkPolyline>();
            }
            (None, None) => {}
        }
    }

    // Vehicles: their link is routes[route_id].links[link_index].
    for (entity, rp, cached) in vehicles.iter_mut() {
        let want_id = routes
            .0
            .get(&rp.route_id)
            .and_then(|r| r.links.get(rp.link_index))
            .cloned();
        match (want_id, cached) {
            (Some(want_id), Some(mut c)) => {
                if c.link_id != want_id {
                    if let Some(points) = link_polylines.0.get(&want_id) {
                        c.link_id = want_id;
                        c.points = Arc::new(points.clone());
                    }
                }
            }
            (Some(want_id), None) => {
                if let Some(points) = link_polylines.0.get(&want_id) {
                    commands.entity(entity).insert(CurrentLinkPolyline {
                        link_id: want_id,
                        points: Arc::new(points.clone()),
                    });
                }
            }
            (None, Some(_)) => {
                commands.entity(entity).remove::<CurrentLinkPolyline>();
            }
            (None, None) => {}
        }
    }
}
```

- [ ] **Step 5: Register it in `install_systems`**

In `backend/crates/sim-core/src/mobility/systems.rs:73-91` (the `install_systems` Advance set), insert `update_link_polyline_cache_system` first and order the rest after it:

```rust
    schedule.add_systems((
        update_link_polyline_cache_system.in_set(MobilitySet::Advance),
        walk_advance_system
            .in_set(MobilitySet::Advance)
            .after(update_link_polyline_cache_system),
        boarding_alighting_system
            .in_set(MobilitySet::Advance)
            .after(walk_advance_system),
        stop_arrival_system
            .in_set(MobilitySet::Advance)
            .after(boarding_alighting_system),
        vehicle_advance_system
            .in_set(MobilitySet::Advance)
            .after(stop_arrival_system),
        warm_chunk_flow_system.in_set(MobilitySet::Advance),
        // Output set
        compute_world_coord_system.in_set(MobilitySet::Output),
        compute_direction_system.in_set(MobilitySet::Output),
        // Bookkeeping
        tick_increment_system.in_set(MobilitySet::Bookkeeping),
    ));
```

- [ ] **Step 6: Modify `compute_world_coord_system` to read the cache**

In `backend/crates/sim-core/src/mobility/systems.rs:444-478` replace with:

```rust
#[allow(clippy::type_complexity)]
pub fn compute_world_coord_system(
    mut agents: Query<
        (
            &AgentMobilityStateComponent,
            &mut Position,
            Option<&CurrentLinkPolyline>,
        ),
        (With<AgentMarker>, Without<VehicleMarker>),
    >,
    mut vehicles: Query<
        (
            &RoutePosition,
            &mut Position,
            Option<&CurrentLinkPolyline>,
        ),
        (With<VehicleMarker>, Without<AgentMarker>),
    >,
    activities: Res<ChunkActivities>,
    routes: Res<Routes>,
    stops: Res<Stops>,
    link_polylines: Res<LinkPolylines>,
) {
    for (rp, mut pos, cached) in vehicles.iter_mut() {
        if !chunk_is_simulated(&pos, &activities) {
            continue;
        }
        if let Some(c) = cached {
            // Fast path: progress along cached polyline.
            let (x, y) = crate::mobility_geometry::point_at_progress_slice(&c.points, rp.progress);
            pos.x = x;
            pos.y = y;
        } else if let Some((x, y)) =
            crate::mobility::vehicle_world_coord(rp, &routes, &link_polylines)
        {
            pos.x = x;
            pos.y = y;
        }
    }
    for (state, mut pos, cached) in agents.iter_mut() {
        if !chunk_is_simulated(&pos, &activities) {
            continue;
        }
        if let (AgentMobilityState::Walking { progress, .. }, Some(c)) = (&state.0, cached) {
            let (x, y) = crate::mobility_geometry::point_at_progress_slice(&c.points, *progress);
            pos.x = x;
            pos.y = y;
        } else if let Some((x, y)) =
            crate::mobility::agent_world_coord(&state.0, &routes, &stops, &link_polylines)
        {
            pos.x = x;
            pos.y = y;
        }
    }
}
```

Note: this assumes `crate::mobility_geometry::point_at_progress_slice` exists with signature `fn(&[(f32,f32)], f32) -> (f32,f32)`. Verify with:
```bash
grep -n "point_at_progress_slice\|fn point_at_progress" /Users/ramonfuglister/Desktop/Coding/abutown/backend/crates/sim-core/src/mobility_geometry.rs
```
If it doesn't exist, find the equivalent helper (likely named `point_at_progress` or similar) and use that name; if no slice variant exists, add one inline or split out — but most likely `direction_at_progress_slice`'s sibling `point_at_progress_slice` is already in `mobility_geometry`.

- [ ] **Step 7: Modify `compute_direction_system` to read the cache**

In `backend/crates/sim-core/src/mobility/systems.rs:481-520` replace with:

```rust
#[allow(clippy::type_complexity)]
pub fn compute_direction_system(
    mut agents: Query<
        (
            &Position,
            &AgentMobilityStateComponent,
            &mut Direction,
            Option<&CurrentLinkPolyline>,
        ),
        (With<AgentMarker>, Without<VehicleMarker>),
    >,
    mut vehicles: Query<
        (
            &Position,
            &RoutePosition,
            &mut Direction,
            Option<&CurrentLinkPolyline>,
        ),
        (With<VehicleMarker>, Without<AgentMarker>),
    >,
    activities: Res<ChunkActivities>,
    routes: Res<Routes>,
    link_polylines: Res<LinkPolylines>,
) {
    for (pos, rp, mut dir, cached) in vehicles.iter_mut() {
        if !chunk_is_simulated(pos, &activities) {
            continue;
        }
        if let Some(c) = cached {
            dir.0 = dir_at_progress(&c.points, rp.progress);
            continue;
        }
        // Slow path: resolve link from route table.
        let Some(route) = routes.0.get(&rp.route_id) else { continue };
        let Some(link_id) = route.links.get(rp.link_index) else { continue };
        let Some(points) = link_polylines.0.get(link_id) else { continue };
        dir.0 = dir_at_progress(points, rp.progress);
    }
    for (pos, state, mut dir, cached) in agents.iter_mut() {
        if !chunk_is_simulated(pos, &activities) {
            continue;
        }
        if let AgentMobilityState::Walking { link_id, progress } = &state.0 {
            if let Some(c) = cached {
                dir.0 = dir_at_progress(&c.points, *progress);
            } else if let Some(points) = link_polylines.0.get(link_id) {
                dir.0 = dir_at_progress(points, *progress);
            }
        }
    }
}
```

- [ ] **Step 8: Run tests**

Run:
```bash
cargo test --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core 2>&1 | tail -10
```
Expected: all 164 sim-core tests pass (162 + 2 new). The snapshot roundtrip test `phase3-mobility-snapshot.json` must stay byte-equal — verify it specifically:

```bash
cargo test --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core phase3 2>&1 | tail -10
```

- [ ] **Step 9: Run bench**

Run:
```bash
cargo bench --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core --bench mobility_tick_lod 2>&1 | tail -10
```
Expected: `tick_100k_all_active` mean ≤ 9 ms (down from ~14 ms). Output systems should drop from ~7.5 ms combined to ~1 ms combined; cache-update system adds ~0.3 ms.

- [ ] **Step 10: Commit**

```bash
git -C /Users/ramonfuglister/Desktop/Coding/abutown add backend/crates/sim-core/src/mobility/
git -C /Users/ramonfuglister/Desktop/Coding/abutown commit -m "$(cat <<'EOF'
perf(mobility): CurrentLinkPolyline component cache for Output systems

Adds a CurrentLinkPolyline { link_id, points: Arc<Vec<(f32,f32)>> }
component refreshed by update_link_polyline_cache_system (new, first in
Advance). compute_world_coord and compute_direction read the cached
Arc directly instead of doing 2× HashMap chain (RouteId → RouteRecord →
LinkId → polyline) per entity per tick.

Component is derived state and NOT serialized — snapshot round-trip
stays byte-stable.

Bench delta: tick_100k_all_active ~14 ms → ~8 ms.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Incremental `track_chunk_populations_system`

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/resources.rs` (add `PreviousChunkByEntity`)
- Modify: `backend/crates/sim-core/src/mobility/systems.rs:526-560` (rewrite the system)
- Modify: `backend/crates/sim-core/src/mobility/mod.rs:104-133` (`empty()` registers the new resource)

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` block at the bottom of `backend/crates/sim-core/src/mobility/systems.rs`:

```rust
#[test]
fn incremental_chunk_populations_matches_full_rebuild() {
    use crate::ids::{AgentId, LinkId};
    use crate::mobility::resources::{PreviousChunkByEntity, PreviousFlowCellContrib};

    let mut world = World::new();
    world.insert_resource(FlowCells::default());
    world.insert_resource(ChunkPopulations::default());
    world.insert_resource(AgentsByChunk::default());
    world.insert_resource(VehiclesByChunk::default());
    world.insert_resource(PreviousChunkByEntity::default());
    world.insert_resource(PreviousFlowCellContrib::default());

    // Spawn 200 agents scattered across multiple chunks.
    for i in 0..200 {
        let x = (i % 10) as f32 * 35.0; // chunks at 0, 35, 70, ...
        let y = (i / 10) as f32 * 35.0;
        world.spawn((
            AgentMarker,
            StableAgentId(AgentId(format!("a:{i}"))),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: LinkId("l".into()),
                progress: 0.0,
            }),
            WalkPlan { stages: vec![], cursor: 0 },
            WalkSpeed(0.05),
            Position { x, y },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ));
    }

    let mut schedule = Schedule::default();
    schedule.add_systems(track_chunk_populations_system);

    // Tick 1: full rebuild path (all positions are "new").
    schedule.run(&mut world);
    let after1: std::collections::HashMap<_, _> = world
        .resource::<AgentsByChunk>()
        .0
        .iter()
        .map(|(c, e)| {
            let mut e = e.clone();
            e.sort_by_key(|x| x.index());
            (*c, e)
        })
        .collect();

    // Mutate one agent's position to a new chunk.
    let mut q = world.query::<(Entity, &mut Position)>();
    let moved_entity = q.iter_mut(&mut world).next().map(|(e, mut p)| {
        p.x = 999.0;
        p.y = 999.0;
        e
    }).unwrap();

    // Tick 2: incremental path.
    schedule.run(&mut world);
    let after2_incremental: std::collections::HashMap<_, _> = world
        .resource::<AgentsByChunk>()
        .0
        .iter()
        .map(|(c, e)| {
            let mut e = e.clone();
            e.sort_by_key(|x| x.index());
            (*c, e)
        })
        .collect();

    // Compare against a fresh full rebuild from query state.
    let mut reference: std::collections::HashMap<crate::ids::ChunkCoord, Vec<Entity>> =
        std::collections::HashMap::new();
    let mut q2 = world.query::<(Entity, &Position, &AgentMarker)>();
    for (entity, pos, _) in q2.iter(&world) {
        let chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
        reference.entry(chunk).or_default().push(entity);
    }
    for bucket in reference.values_mut() {
        bucket.sort_by_key(|x| x.index());
    }
    assert_eq!(after2_incremental, reference);
    // Ensure the moved entity actually moved buckets.
    assert!(after2_incremental.values().any(|v| v.contains(&moved_entity)));
    let _ = after1; // silence unused warning if the assertion above passes
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:
```bash
cargo test --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core incremental_chunk_populations 2>&1 | tail -10
```
Expected: compile error — `unresolved import 'crate::mobility::resources::PreviousChunkByEntity'`.

- [ ] **Step 3: Add both resources**

Append to `backend/crates/sim-core/src/mobility/resources.rs`:

```rust
/// Per-entity record of which chunk that entity was bucketed into during
/// the previous run of `track_chunk_populations_system`. Lets the system
/// run incrementally via `Changed<Position>` — when an entity's Position
/// changes, we know which old bucket to remove from before inserting into
/// the new one.
#[derive(Resource, Debug, Default, Clone)]
pub struct PreviousChunkByEntity(pub HashMap<Entity, ChunkCoord>);

/// Per-chunk FlowCell aggregate that was added to `ChunkPopulations` on
/// the previous tick. Subtracted at the start of each incremental tick
/// before the entity-count deltas are applied, then re-added after, so
/// FlowCell contributions don't double-count across ticks.
#[derive(Resource, Debug, Default, Clone)]
pub struct PreviousFlowCellContrib(pub HashMap<ChunkCoord, u32>);
```

- [ ] **Step 4: Register the resources in `MobilityWorld::empty()`**

In `backend/crates/sim-core/src/mobility/mod.rs:104-133`, add:

```rust
        world.insert_resource(crate::mobility::resources::PreviousChunkByEntity::default());
        world.insert_resource(crate::mobility::resources::PreviousFlowCellContrib::default());
```

- [ ] **Step 5: Rewrite the system**

In `backend/crates/sim-core/src/mobility/systems.rs:526-560`, replace `track_chunk_populations_system` with:

```rust
#[allow(clippy::type_complexity)]
pub fn track_chunk_populations_system(
    moved_agents: Query<
        (Entity, &Position),
        (With<AgentMarker>, Changed<Position>),
    >,
    moved_vehicles: Query<
        (Entity, &Position),
        (With<VehicleMarker>, Changed<Position>),
    >,
    all_agents: Query<(Entity, &Position), With<AgentMarker>>,
    all_vehicles: Query<(Entity, &Position), With<VehicleMarker>>,
    flow_cells: Res<FlowCells>,
    mut populations: ResMut<ChunkPopulations>,
    mut agents_by_chunk: ResMut<AgentsByChunk>,
    mut vehicles_by_chunk: ResMut<VehiclesByChunk>,
    mut previous: ResMut<crate::mobility::resources::PreviousChunkByEntity>,
    mut prev_flow: ResMut<crate::mobility::resources::PreviousFlowCellContrib>,
) {
    use std::collections::HashMap;

    let first_run = previous.0.is_empty();
    if first_run {
        // First run after world creation / hydration: full rebuild.
        agents_by_chunk.0.clear();
        vehicles_by_chunk.0.clear();
        populations.0.clear();
        for (entity, pos) in all_agents.iter() {
            let chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
            *populations.0.entry(chunk).or_insert(0) += 1;
            agents_by_chunk.0.entry(chunk).or_default().push(entity);
            previous.0.insert(entity, chunk);
        }
        for (entity, pos) in all_vehicles.iter() {
            let chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
            *populations.0.entry(chunk).or_insert(0) += 1;
            vehicles_by_chunk.0.entry(chunk).or_default().push(entity);
            previous.0.insert(entity, chunk);
        }
    } else {
        // Step A: undo the previous tick's FlowCell aggregate so the
        // entity-count deltas below operate on a clean entity-only base.
        for (chunk, amount) in prev_flow.0.drain() {
            if let Some(p) = populations.0.get_mut(&chunk) {
                *p = p.saturating_sub(amount);
            }
        }

        // Step B: incremental rebucketing of moved entities.
        for (entity, pos) in moved_agents.iter() {
            let new_chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
            if let Some(old_chunk) = previous.0.get(&entity).copied() {
                if old_chunk == new_chunk {
                    continue;
                }
                if let Some(bucket) = agents_by_chunk.0.get_mut(&old_chunk) {
                    bucket.retain(|e| *e != entity);
                }
                if let Some(p) = populations.0.get_mut(&old_chunk) {
                    *p = p.saturating_sub(1);
                }
            }
            *populations.0.entry(new_chunk).or_insert(0) += 1;
            agents_by_chunk.0.entry(new_chunk).or_default().push(entity);
            previous.0.insert(entity, new_chunk);
        }
        for (entity, pos) in moved_vehicles.iter() {
            let new_chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
            if let Some(old_chunk) = previous.0.get(&entity).copied() {
                if old_chunk == new_chunk {
                    continue;
                }
                if let Some(bucket) = vehicles_by_chunk.0.get_mut(&old_chunk) {
                    bucket.retain(|e| *e != entity);
                }
                if let Some(p) = populations.0.get_mut(&old_chunk) {
                    *p = p.saturating_sub(1);
                }
            }
            *populations.0.entry(new_chunk).or_insert(0) += 1;
            vehicles_by_chunk.0.entry(new_chunk).or_default().push(entity);
            previous.0.insert(entity, new_chunk);
        }

        // Step C: reconcile despawns — any entity in `previous` that no
        // longer has Position is removed from its bucket.
        let stale: Vec<Entity> = previous
            .0
            .keys()
            .copied()
            .filter(|e| all_agents.get(*e).is_err() && all_vehicles.get(*e).is_err())
            .collect();
        for entity in stale {
            if let Some(old_chunk) = previous.0.remove(&entity) {
                if let Some(bucket) = agents_by_chunk.0.get_mut(&old_chunk) {
                    bucket.retain(|e| *e != entity);
                }
                if let Some(bucket) = vehicles_by_chunk.0.get_mut(&old_chunk) {
                    bucket.retain(|e| *e != entity);
                }
                if let Some(p) = populations.0.get_mut(&old_chunk) {
                    *p = p.saturating_sub(1);
                }
            }
        }
    }

    // Step D: re-add current FlowCell aggregate and remember it for next tick.
    let mut current_flow: HashMap<crate::ids::ChunkCoord, u32> = HashMap::new();
    for (chunk, cell) in &flow_cells.0 {
        let aggregate = cell.population.floor().max(0.0) as u32;
        if aggregate > 0 {
            *populations.0.entry(*chunk).or_insert(0) += aggregate;
            current_flow.insert(*chunk, aggregate);
        }
    }
    prev_flow.0 = current_flow;

    // Drop empty buckets so demote doesn't pay for dead entries.
    agents_by_chunk.0.retain(|_, bucket| !bucket.is_empty());
    vehicles_by_chunk.0.retain(|_, bucket| !bucket.is_empty());
}
```

Why the FlowCell handling: `classify_activity_system` reads `ChunkPopulations` and treats a positive value as "this chunk has population". FlowCell aggregates contribute to that value but their entity count isn't tracked by `Changed<Position>` — they live as numeric values in `FlowCells`. The `prev_flow.0.drain()` at the start of each incremental tick removes the previous tick's FlowCell contribution from `populations` before the entity-count deltas land; Step D re-adds the current tick's FlowCell contribution. Net result: `populations[chunk] = entity_count + flow_cell_aggregate`, exact same semantic as the old full-rebuild path.

- [ ] **Step 6: Run tests**

Run:
```bash
cargo test --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core 2>&1 | tail -10
```
Expected: all 165 sim-core tests pass (164 + 1 new). The `track_chunk_populations_sums_agents_vehicles_and_flow_cells` test at `systems.rs:1444` must still pass — it spawns then runs the schedule once, hitting the first-run path. If it spawns and runs twice expecting populations to be stable, adjust the test to insert `PreviousChunkByEntity` + `PreviousFlowCellContrib` and update assertions if needed.

- [ ] **Step 7: Run full workspace + bench**

```bash
cargo test --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml 2>&1 | tail -10
```
Expected: all 181 workspace tests pass.

```bash
cargo bench --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core --bench mobility_tick_lod 2>&1 | tail -10
```
Expected: `tick_100k_all_active` mean ≤ 5 ms ✓ goal met. If still > 5 ms, run the profile example to identify the new top hotspot and continue to Task 7's stretch step.

- [ ] **Step 8: Commit**

```bash
git -C /Users/ramonfuglister/Desktop/Coding/abutown add backend/crates/sim-core/src/mobility/
git -C /Users/ramonfuglister/Desktop/Coding/abutown commit -m "$(cat <<'EOF'
perf(mobility): incremental track_chunk_populations via Changed<Position>

Previous system rebuilt AgentsByChunk / VehiclesByChunk / ChunkPopulations
from scratch every tick — 100k entity iterations regardless of how many
moved. New version uses `Query<..., Changed<Position>>` filter so only
moved entities are re-bucketed, with PreviousChunkByEntity tracking the
old bucket for each entity. PreviousFlowCellContrib subtracts/re-adds
the FlowCell aggregate slice of ChunkPopulations to avoid double-counting
across ticks.

First-run path (post-hydration when all Positions are "newly changed")
still does a full rebuild for correctness.

Bench delta: tick_100k_all_active ~8 ms → ~5 ms ✓ goal met.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Final verification + progress.md entry

**Files:**
- Modify: `progress.md`

- [ ] **Step 1: Full workspace tests + clippy**

Run:
```bash
cargo test --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml 2>&1 | tail -5
cargo clippy --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml --all-targets -- -D warnings 2>&1 | tail -10
```
Expected: all tests pass, clippy clean.

- [ ] **Step 2: TSC + vitest**

Run from project root (use absolute paths to avoid cwd drift):
```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown && npx tsc --noEmit 2>&1 | tail -5
cd /Users/ramonfuglister/Desktop/Coding/abutown && npx vitest run 2>&1 | tail -10
```
Expected: tsc clean, 158/158 vitest tests pass (or 156/158 with the known `noRetiredAssets` pre-existing failures).

- [ ] **Step 3: Browser smoke**

CLAUDE.md mandates this for boundary-crossing changes. This rework is backend-only and doesn't change wire formats, but smoke acts as a final guard.

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown && node scripts/smoke-7b.mjs 2>&1 | tail -30
```
Expected: all 9 checks pass.

- [ ] **Step 4: Capture final bench numbers**

```bash
cargo bench --manifest-path /Users/ramonfuglister/Desktop/Coding/abutown/backend/Cargo.toml -p sim-core --bench mobility_tick_lod 2>&1 | tail -20
```
Capture the exact mean for `tick_100k_all_active` and `tick_100k_with_5_subscribed_chunks` for the progress.md entry.

- [ ] **Step 5: Append progress.md entry**

At the top of the reverse-chronological block (line 19 onwards — see CLAUDE.md note about insertion point), insert a new entry. Use the exact format of the surrounding entries. Replace `<FINAL_NUMBER>` with the captured bench mean:

Edit `progress.md` to insert immediately after line 18 (between the original line 18 and line 19):

```
2026-05-19T<HH:MM:SS>.000Z - Mobility tick perf rework (Phase 6 followup): the original 2026-05-17 progress note claimed the LOD-filter overhead was the bottleneck — fresh profiling debunked that. The existing `tick_100k_with_5_subscribed_chunks` bench warms down to ~0 active entities because LOD demotes everything outside the 5 subscribed chunks (still 21µs — unchanged). New bench `tick_100k_all_active` subscribes all 32×16 chunks so 100k walkers + 1k cars stay in the ECS hot path; baseline was 24.4 ms / tick. Five optimizations land under that bench: (1) `AgentIdIndex` + `VehicleIdIndex` ECS resources mirror `MobilityWorld.by_agent_id` / `by_vehicle_id` so `boarding_alighting_system` does O(1) lookups instead of three nested 100k-entity scans (A.5, B.2, B.3) — also drops the 100k-entry A.0 pre-pass, replacing it with lazy per-candidate `chunk_is_simulated`; (2) new `NearStop` marker (inserted by `walk_advance_system` on progress saturation, removed by `stop_arrival_system` post-transition) confines `stop_arrival_system`'s query to candidates only; (3) `CurrentLinkPolyline { link_id, points: Arc<Vec<(f32,f32)>> }` component cache refreshed by new `update_link_polyline_cache_system` (first in `MobilitySet::Advance`) — `compute_world_coord_system` and `compute_direction_system` read the cached `Arc` directly instead of doing `routes.get() → links.get() → link_polylines.get()` per entity per tick; (4) `track_chunk_populations_system` rewritten incrementally — `Query<..., Changed<Position>>` only re-buckets moved entities, `PreviousChunkByEntity` tracks each entity's old bucket, `PreviousFlowCellContrib` keeps the FlowCell aggregate slice consistent across ticks; (5) lazy chunk-activity filter inside `boarding_alighting_system` (covered in #1) — no 100k pre-pass. New profile example `examples/profile_lod_tick.rs` and `MobilityWorld::profile_world_mut()` accessor support per-system timing during development. `CurrentLinkPolyline` is derived state and not serialized — snapshot roundtrip stays byte-stable. Bench: `tick_100k_all_active` 24.4 ms → <FINAL_NUMBER> (<5 ms target met). All 181 workspace cargo tests + 158 vitest + clippy + tsc + browser smoke 9/9 pass.
```

- [ ] **Step 6: Commit progress.md**

```bash
git -C /Users/ramonfuglister/Desktop/Coding/abutown add progress.md
git -C /Users/ramonfuglister/Desktop/Coding/abutown commit -m "$(cat <<'EOF'
docs: progress note for mobility tick perf rework

24.4 ms → <FINAL_NUMBER> on tick_100k_all_active. Replace <FINAL_NUMBER>
with the bench mean captured in Task 7 Step 4.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```
