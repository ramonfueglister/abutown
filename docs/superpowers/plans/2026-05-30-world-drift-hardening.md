# World-Drift Hardening (data-driven activity waypoints) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Derive the round-trip `home`/`destination` waypoints from the loaded world's south-sidewalk corridor instead of hardcoding coordinates, so regenerating the world cannot leave production routing pointing at stale coords.

**Architecture:** A new `ActivityWaypoints(HashMap<String,(f32,f32)>)` resource is the authoritative source for resolvable activity coordinates; `destination_for_stage` consults it before falling back to the static `activity_geometry`. The seed populates it from the corridor endpoints. Each task leaves the branch green.

**Tech Stack:** Rust, bevy_ecs resources.

**Spec:** `docs/superpowers/specs/2026-05-30-world-drift-hardening-design.md`

**Branch / isolation:** worktree `/Users/ramonfuglister/Coding/abutown-drift` on `plan/world-drift-hardening` (from `origin/main` 152432a). `export CARGO_TARGET_DIR=/tmp/abutown-drift-target`. Every cargo via `scripts/cargo-serial.sh`; `cargo fmt --check` each task.

## Grounding (verified)
- Resource pattern (mobility/resources.rs): `#[derive(Resource, Debug, Default, Clone)] pub struct ChunkPopulations(pub HashMap<ChunkCoord, u32>);`
- `install_mobility` (mobility/api.rs:30) inserts the default resources (`ChunkPopulations::default()` at 48, `AgentsByChunk::default()` at 52). It is called by `MobilityPlugin`, which every world builder installs (`empty_world_and_schedule`, `from_network`, `from_base_world_bundle`). Inserting the default here makes `Res<ActivityWaypoints>` always present.
- `destination_for_stage` (mobility/systems/routing.rs:30) `fn(graph, stage, spatial) -> Option<NodeId>`; for `WalkToActivity { activity_id }` it does `graph.node_by_legacy(activity_id).or_else(|| spatial?.nearest(activity_geometry(activity_id)?.coord))`. Called at routing.rs:202 (in `route_assignment_system`) and :422 (in `route_advance_system`); both systems take `Res<...>` params.
- `seed_pedestrians_from_bundle` (mobility/seed.rs:459) has `world: &mut World` + `bundle`, finds `corridor_index` per pedestrian group, so `bundle.transport.pedestrian_corridors[corridor_index].points` first/last are the home/destination.
- `activity_geometry` (mobility_geometry.rs:106) currently hardcodes `"activity:home" => (106.0, 64.51)`, `"activity:destination" => (117.0, 64.51)`, plus `"activity:work"` and a default.
- The mobility_geometry.rs test `home_and_destination_resolve_to_south_corridor_ends` asserts those literals (will be updated in Task 3).

---

## Task 1: `ActivityWaypoints` resource + routing consults it (fallback to static)

**Files:** Modify `mobility/resources.rs` (new resource), `mobility/api.rs` (insert default in `install_mobility`), `mobility/systems/routing.rs` (`destination_for_stage` + the two systems). Test: routing.rs inline tests.

This task adds the resource + plumbing but leaves `activity_geometry`'s hardcoded arms in place as the fallback, so behaviour is unchanged (empty resource → falls back).

- [ ] **Step 1: Failing test** (routing.rs `#[cfg(test)] mod tests` — mirror existing routing test setup, which builds a `Graph` + `NodeSpatialIndex`):
```rust
    #[test]
    fn destination_for_activity_prefers_waypoints_over_static_geometry() {
        // Build a graph + spatial index with a node at (5.0, 5.0).
        let (graph, spatial) = test_graph_with_node_at(5.0, 5.0); // adapt to existing helpers
        let mut wp = crate::mobility::resources::ActivityWaypoints::default();
        wp.0.insert("activity:home".to_string(), (5.0, 5.0));
        let stage = PlanStage::WalkToActivity { link_id: "l".into(), activity_id: "activity:home".into() };
        let got = super::destination_for_stage(&graph, &stage, Some(&spatial), &wp);
        assert_eq!(got, spatial.nearest((5.0, 5.0)));
    }
    #[test]
    fn destination_for_activity_falls_back_to_static_geometry_when_absent() {
        let (graph, spatial) = test_graph_with_node_at(/* near activity:work's coord */);
        let wp = crate::mobility::resources::ActivityWaypoints::default(); // empty
        let stage = PlanStage::WalkToActivity { link_id: "l".into(), activity_id: "activity:work".into() };
        // resolves via activity_geometry("activity:work") -> nearest node (non-None)
        assert!(super::destination_for_stage(&graph, &stage, Some(&spatial), &wp).is_some());
    }
```
(Adapt graph/spatial construction to the helpers already used in routing.rs tests — grep the existing `#[test]` setup there. `destination_for_stage` is module-private, so tests live in the same file.)
RUN: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core destination_for_activity` → FAIL (signature mismatch / fn doesn't take waypoints yet).

- [ ] **Step 2: Add the resource** (mobility/resources.rs):
```rust
/// World-derived coordinates for resolvable activities (e.g. round-trip
/// home/destination), populated at seed time from the loaded world's geometry.
/// Authoritative over the static `mobility_geometry::activity_geometry` fallback.
#[derive(Resource, Debug, Default, Clone)]
pub struct ActivityWaypoints(pub std::collections::HashMap<String, (f32, f32)>);
```
(Confirm `HashMap` import style in the file; reuse the existing `use std::collections::HashMap;` if present.)

- [ ] **Step 3: Insert the default** in `install_mobility` (mobility/api.rs, next to the other inserts ~line 48-52):
```rust
    world.insert_resource(crate::mobility::resources::ActivityWaypoints::default());
```

- [ ] **Step 4: Thread it through `destination_for_stage`** (routing.rs:30):
```rust
fn destination_for_stage(
    graph: &crate::routing::Graph,
    stage: &PlanStage,
    spatial: Option<&crate::routing::NodeSpatialIndex>,
    waypoints: &crate::mobility::resources::ActivityWaypoints,
) -> Option<crate::routing::NodeId> {
    match stage {
        PlanStage::WalkToStop { stop_id, .. } => graph.node_by_legacy(stop_id),
        PlanStage::WalkToActivity { activity_id, .. } => graph.node_by_legacy(activity_id).or_else(|| {
            let coord = waypoints
                .0
                .get(activity_id)
                .copied()
                .or_else(|| crate::mobility_geometry::activity_geometry(activity_id).map(|g| g.coord))?;
            spatial?.nearest(coord)
        }),
        _ => None,
    }
}
```

- [ ] **Step 5: Update the two call sites + system params.**
  - `route_assignment_system` (routing.rs:144): add param `waypoints: Res<crate::mobility::resources::ActivityWaypoints>,` and change the call at :202 to `destination_for_stage(&graph, &stage, spatial.as_deref(), &waypoints)`.
  - `route_advance_system` (routing.rs:340): add the same `waypoints: Res<...>` param and change the call at :422 likewise.

- [ ] **Step 6: Verify** — `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core` (new tests + all green; the existing round_trip via fallback still works) · `clippy -p sim-core --all-targets -- -D warnings` · `fmt --check`.

- [ ] **Step 7: Commit**
```
git add -A && git commit -m "feat(move): ActivityWaypoints resource — routing consults it before static geometry

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Populate `ActivityWaypoints` from the seeded corridor

**Files:** Modify `mobility/seed.rs` (`seed_pedestrians_from_bundle` ~459). Test: `backend/crates/sim-core/tests/round_trip_movement.rs` or seed.rs inline.

- [ ] **Step 1: Failing test** (round_trip_movement.rs — it already loads the abutopia bundle + reads `south_corridor_ends`):
```rust
#[test]
fn seed_populates_activity_waypoints_from_corridor() {
    let bundle = BaseWorldBundle::load_from_dir(abutopia_root()).expect("bundle loads");
    let (home, dest) = south_corridor_ends(&bundle);
    let (world, _s) = seed::from_base_world_bundle(&bundle).expect("seed ok");
    let wp = world.resource::<sim_core::mobility::resources::ActivityWaypoints>();
    assert_eq!(wp.0.get("activity:home").copied(), Some(home));
    assert_eq!(wp.0.get("activity:destination").copied(), Some(dest));
}
```
RUN → FAIL (resource is empty/default; seed does not populate it yet).

- [ ] **Step 2: Populate at seed.** In `seed_pedestrians_from_bundle`, after resolving `corridor_index` for the pedestrian group, insert the corridor endpoints into the resource:
```rust
        let corridor = &bundle.transport.pedestrian_corridors[corridor_index];
        if let (Some(first), Some(last)) = (corridor.points.first(), corridor.points.last()) {
            let mut wp = world.resource_mut::<crate::mobility::resources::ActivityWaypoints>();
            wp.0.insert("activity:home".to_string(), (first.x, first.y));
            wp.0.insert("activity:destination".to_string(), (last.x, last.y));
        }
```
Place it where `corridor_index`/`corridor` is in scope (the existing code already binds `corridor_index` and accesses `bundle.transport.pedestrian_corridors[corridor_index]` for the agent's link). Keep the existing agent-seeding logic.

- [ ] **Step 3: Verify** — the new test passes; the existing `round_trip_movement` tests still pass (now home/destination flow through the resource); full `-p sim-core` green; `clippy`; `fmt --check`.

- [ ] **Step 4: Commit**
```
git add -A && git commit -m "feat(move): seed populates ActivityWaypoints from the pedestrian corridor ends

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Remove the hardcoded waypoints from `activity_geometry` (drift guard)

**Files:** Modify `mobility_geometry.rs` (remove the home/destination arms + update its test). Test: mobility_geometry.rs inline.

Now that the seed populates the resource (Task 2), the static hardcoded arms are redundant and are exactly the drift hazard — remove them.

- [ ] **Step 1: Update the test to a drift guard.** Replace `home_and_destination_resolve_to_south_corridor_ends` (mobility_geometry.rs ~205) with:
```rust
    #[test]
    fn activity_geometry_does_not_hardcode_round_trip_waypoints() {
        // home/destination are world-derived via the ActivityWaypoints resource,
        // NOT hardcoded here (hardcoding caused a world-drift bug). They must fall
        // through to the default, identical to any unknown activity.
        let default = activity_geometry("activity:unknown").unwrap().coord;
        assert_eq!(activity_geometry("activity:home").unwrap().coord, default);
        assert_eq!(activity_geometry("activity:destination").unwrap().coord, default);
    }
```
RUN → FAIL (the arms still return the bespoke coords).

- [ ] **Step 2: Remove the arms.** Delete the `"activity:home" => …` and `"activity:destination" => …` match arms (and their comment block) from `activity_geometry`, leaving `"activity:work"` and the `_ => default`.

- [ ] **Step 3: Verify** — the drift-guard test passes; **`cargo test --manifest-path backend/Cargo.toml -p sim-core --test round_trip_movement` still GREEN** (the pedestrian oscillates via the resource, not the static fn); full `-p sim-core`; `clippy`; `fmt --check`. If round_trip breaks, the resource is not being consulted on the route path — fix Task 1/2 wiring, do not re-add the arms.

- [ ] **Step 4: Commit**
```
git add -A && git commit -m "refactor(move): drop hardcoded round-trip waypoints from activity_geometry

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Final gate + PR
- [ ] `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check`
- [ ] `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings`
- [ ] `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace`
- [ ] `scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server`
- [ ] PR → confirm CI green with `gh run watch <id> --exit-status` → merge → `superpowers:finishing-a-development-branch`.

## Deferred
- god-file splits of runtime.rs + app.rs (stream ③).
- CHUNK_SIZE parametrization.
