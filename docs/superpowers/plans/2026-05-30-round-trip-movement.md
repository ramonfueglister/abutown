# Round-Trip / Cyclic Movement Implementation Plan (minimal)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make the abutopia pedestrian walk purposefully between two waypoints and loop forever (instead of random wandering), via a cyclic `WalkPlan`.

**Architecture:** Add a `cyclic` flag to `WalkPlan`/`AgentRecord`; when the plan cursor advances past the last stage and `cyclic` is set, wrap it to 0. Register two activities (`home`, `destination`) at the pedestrian corridor's two endpoints (reachable footway nodes near the houses). Seed the abutopia pedestrian with a cyclic `[WalkToActivity(home), WalkToActivity(destination)]` plan; the existing HPA* routing handles the pathing.

**Tech Stack:** Rust (sim-core/mobility), builds on the merged abutopia + 8i + sidewalks.

**Spec:** `docs/superpowers/specs/2026-05-30-round-trip-movement-design.md`

**Branch / isolation:** worktree `/Users/ramonfuglister/Coding/abutown-rt` on `plan/round-trip-movement`. `export CARGO_TARGET_DIR=/tmp/abutown-rt-target`. Every cargo via `scripts/cargo-serial.sh`; `cargo fmt --check` in every task verify.

## Grounding (verified on this branch)
- `mobility_geometry::activity_geometry(id) -> Option<ActivityGeometry{coord:(f32,f32)}>` is a hardcoded `match`: `"activity:work" => …`, `_ => default`. Add `home`/`destination` arms.
- `WalkToActivity { activity_id }` → `activity_geometry(id).coord` → `destination_for_stage` → HPA* route (routing.rs). `Activity` → random `next_wander_footway_link`.
- Cursor advances at: `walking.rs` (route advance, `plan.cursor += 1`) and `routing.rs` (`plan.cursor += 1` at two sites). Past the end, `plan.stages.get(cursor)` → `None` → agent idles/skips.
- abutopia houses: `building:house-a` @ (2,3), `building:house-b` @ (13,3). Pedestrian corridors: `corridor:sidewalk:north`, `corridor:sidewalk:south`; the seeded pedestrian uses `corridor:sidewalk:south` (`data/worlds/abutopia/layers/transport.json` + `spawns.json`).
- Seed builds the pedestrian plan in `mobility/seed.rs` (`seed_pedestrians_from_bundle`) as `vec![PlanStage::Activity { activity_id: "activity:wander:N" }]`.
- `WalkPlan` is a Component (mobility/components.rs); `AgentRecord` has `plan`, `plan_cursor`, `sex`, `parent_id`, `birth_tick` (all serde-defaulted where added).

---

## Task 1: Cyclic `WalkPlan` + cursor-wrap helper

**Files:** `mobility/components.rs` (WalkPlan gains `cyclic`), `mobility/records.rs` (AgentRecord gains `cyclic`, threaded to the WalkPlan at spawn), `mobility/api.rs` (`spawn_agent_from_record` sets `WalkPlan.cyclic` from the record; extraction reads it back), `mobility/systems/walking.rs` + `mobility/systems/routing.rs` (use the wrap helper). Test: `mobility/systems/tests.rs`.

- [ ] **Step 1: Failing test** (systems/tests.rs):
```rust
#[test]
fn cyclic_plan_cursor_wraps_to_zero_at_end() {
    use crate::mobility::components::WalkPlan;
    let mut p = WalkPlan { stages: vec![/* two stages */], cursor: 1, cyclic: true };
    crate::mobility::systems::advance_cursor(&mut p);
    assert_eq!(p.cursor, 0); // wrapped
    let mut q = WalkPlan { stages: vec![/* two */], cursor: 1, cyclic: false };
    crate::mobility::systems::advance_cursor(&mut q);
    assert_eq!(q.cursor, 2); // non-cyclic: no wrap
}
```
(Build the two-stage vecs with real `PlanStage`s — match the file's imports.) RUN → FAIL.

- [ ] **Step 2: Add `cyclic` to `WalkPlan`** (components.rs): add `pub cyclic: bool` to the struct. Fix every `WalkPlan { … }` literal in the crate (grep `rg -n "WalkPlan \{" backend`) to add `cyclic: false` (default).

- [ ] **Step 3: The wrap helper** — add to `mobility/systems/mod.rs` (or wherever shared system helpers live), `pub`:
```rust
/// Advance a plan cursor by one; cyclic plans wrap back to the start.
pub fn advance_cursor(plan: &mut crate::mobility::components::WalkPlan) {
    plan.cursor += 1;
    if plan.cyclic && plan.cursor >= plan.stages.len() && !plan.stages.is_empty() {
        plan.cursor = 0;
    }
}
```
Replace the bare `plan.cursor += 1;` sites in `walking.rs` and `routing.rs` (3 sites) with `crate::mobility::systems::advance_cursor(&mut plan);` (adjust the binding name to each call site).

- [ ] **Step 4: Thread `cyclic` through AgentRecord + spawn** — add `#[serde(default)] pub cyclic: bool` to `AgentRecord` (records.rs); in `new_born_at` init `cyclic: false`. In `spawn_agent_from_record` set `WalkPlan { …, cyclic: record.cyclic }`. In `agent_record_from_entity` read it back from the `WalkPlan` component (`cyclic: plan.cyclic`). Fix any struct-literal `AgentRecord` in tests.

- [ ] **Step 5: Verify** — `population::`/`cyclic_plan_cursor_wraps` pass; full `-p sim-core` pass; clippy `-p sim-core --all-targets -- -D warnings`; `fmt --check`; `build -p sim-server`.

- [ ] **Step 6: Commit**
```
git add -A && git commit -m "feat(move): cyclic WalkPlan — cursor wraps at plan end

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: `home` / `destination` activities at the corridor endpoints

**Files:** `backend/crates/sim-core/src/mobility_geometry.rs`. Test: inline.

- [ ] **Step 1: Read the real endpoint coords.** `data/worlds/abutopia/layers/transport.json` → `corridor:sidewalk:south` → `points[0]` and `points[last]`. Use those exact coords (they are footway-graph nodes, so reachable). The houses (2,3)/(13,3) sit at/next to these ends.

- [ ] **Step 2: Failing test** (mobility_geometry tests):
```rust
#[test]
fn home_and_destination_resolve_to_corridor_ends() {
    let home = activity_geometry("activity:home").unwrap().coord;
    let dest = activity_geometry("activity:destination").unwrap().coord;
    assert_ne!(home, dest);
    // assert they equal the south-corridor endpoints you read in Step 1
}
```
RUN → FAIL (falls into the wildcard default currently).

- [ ] **Step 3: Add match arms** in `activity_geometry`:
```rust
        "activity:home" => Some(ActivityGeometry { coord: (/* south-corridor point[0] */) }),
        "activity:destination" => Some(ActivityGeometry { coord: (/* south-corridor point[last] */) }),
```
(Fill the literal coords from Step 1; match the coord unit the other arms use — tile-space.)

- [ ] **Step 4: Verify** — tests pass; clippy; fmt --check.

- [ ] **Step 5: Commit**
```
git add -A && git commit -m "feat(move): home/destination activities at the abutopia corridor ends

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Seed the abutopia pedestrian with a cyclic round-trip plan

**Files:** `mobility/seed.rs` (`seed_pedestrians_from_bundle`). Test: `backend/crates/sim-core/tests/` (new `round_trip_movement.rs`) or systems/tests.

- [ ] **Step 1: Failing integration test** — `backend/crates/sim-core/tests/round_trip_movement.rs`: build the abutopia world via the existing base-world bundle test helper (mirror `abutopia_bundle.rs` / the seed path), or seed one pedestrian with a cyclic `[WalkToActivity("activity:home"), WalkToActivity("activity:destination")]` plan directly; run the mobility schedule for enough ticks; sample the agent's `world_coord` (or `Position`) over time and assert it **moves toward `destination`, then reverses toward `home`** (its x-extent spans both corridor ends — not a random drift), deterministically (same world id → same trajectory). RUN → FAIL (seed still produces the wander plan / no cyclic).

- [ ] **Step 2: Change the seed** — in `seed_pedestrians_from_bundle`, build the pedestrian as a **cyclic round-trip** instead of the wander Activity:
```rust
let plan = vec![
    PlanStage::WalkToActivity { link_id: /* corridor link */, activity_id: "activity:home".into() },
    PlanStage::WalkToActivity { link_id: /* corridor link */, activity_id: "activity:destination".into() },
];
let mut rec = AgentRecord::new_born_at(agent_id, AgentMobilityState::Walking { link_id, progress }, plan, 0.05, 0);
rec.cyclic = true;
// (keep sex assignment from the population work)
```
(Match the real `WalkToActivity` field set — it has `link_id` + `activity_id`; use the corridor's link id for `link_id`, the system re-routes via the activity coord anyway. Keep the existing initial `Walking { link_id, progress }` start state + the deterministic sex.)

> **Verify in impl (open Qs):** confirm HPA* finds a route from the corridor to each activity coord (if not, set the activity coords exactly to corridor endpoints that ARE footway nodes — Task 2 already does this). Confirm the `WalkToActivity.link_id` field value the seed should use.

- [ ] **Step 3: Verify** — the integration test passes (agent oscillates, deterministic); full `-p sim-core` pass; clippy; fmt --check; `build -p sim-server`.

- [ ] **Step 4: Commit**
```
git add -A && git commit -m "feat(move): abutopia pedestrian walks a cyclic home↔destination route

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Final gate
- [ ] `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check` (clean)
- [ ] `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings` (clean)
- [ ] `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace` (green — incl. the existing abutopia render-smoke expectations: the agent count is still 1; it just moves purposefully now)
- [ ] PR → confirm CI green with `gh ... --exit-status` (never merge on a misread) → `superpowers:finishing-a-development-branch`.

## Deferred (later)
- Dwell/pause at the endpoints.
- Time-of-day scheduling via the 8i `SimClock` (morning→work, night→home).
- Per-corridor / per-agent individual routines; multiple waypoints; activity selection.
- Generalising the seed (derive home/destination per corridor instead of two hardcoded activities).
