# Demographic Persistence Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the population system from replaying its entire demographic history (mass die-off / baby-boom, duplicate agents) on every server restart by persisting `LastProcessedMonth`, and make the catch-up loop month-accurate and duplicate-safe.

**Architecture:** The monthly population system advances `(LastProcessedMonth, current_month]` once per tick. `Tick` is persisted but `LastProcessedMonth` is not, so after a reload it resets to 0 while `Tick` is restored — the next tick then replays thousands of months at once. The fix persists `last_processed_month` in the mobility snapshot and, on restore, sets it to `max(persisted, month_index(tick))` so both new and legacy snapshots resume cleanly. Two supporting changes make the catch-up loop correct for arbitrary spans: per-month age in the probability thresholds, and a guard against re-spawning an already-existing deterministic child id.

**Tech Stack:** Rust, `bevy_ecs` (ECS World/Resource), `serde` + `serde_json` (snapshot format). All cargo MUST be run through `scripts/cargo-serial.sh` (see CLAUDE.md — never run two cargo processes at once). Pure backend change, no frontend wiring touched → no browser smoke required.

**Spec:** `docs/superpowers/specs/2026-05-31-demographic-persistence-fix-design.md`

---

## Execution context

This plan is executed in an **isolated git worktree** (created via `superpowers:using-git-worktrees` at execution start), branched off the current branch `plan/persistence-liveness`. At the end: run the full gate, commit everything, merge, and push to GitHub (see "Finishing" at the bottom).

Run all cargo commands scoped to the package and through the serial wrapper, e.g.:

```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core
```

Before launching any cargo command, clear orphans: `pgrep -f cargo` and kill stragglers if present.

---

## File Structure

Files touched, each with one clear responsibility:

- **`backend/crates/sim-core/src/time/mod.rs`** — owns `SimClock` and all time math. Add two pure helpers (`month_start_seconds`, `age_years_at`) + their unit tests. (Task 1)
- **`backend/crates/sim-core/src/mobility/persist_snapshot.rs`** — owns `MobilityPersistSnapshot` (the serialized mobility-world state), `extract_from_world`, `apply_into_world`. Add the `last_processed_month` field (wire format + extract + restore guard). (Task 2)
- **`backend/crates/sim-core/tests/population_persistence_reload.rs`** *(new)* — integration tests: reload regression, value round-trip, legacy snapshot. (Tasks 2 & 3)
- **`backend/crates/sim-core/src/population/mod.rs`** — owns `population_monthly_system`. Switch the catch-up loop to per-month age; add the double-spawn guard; add inline tests. (Tasks 4 & 5)
- **Existing snapshot struct-literal sites** — must gain the new field to keep compiling (compiler-driven, see Task 2 Step 5).

---

## Task 1: Add per-instant age helpers to `SimClock`

Pure, dependency-free functions. TDD start.

**Files:**
- Modify: `backend/crates/sim-core/src/time/mod.rs` (impl block ends at line 46; `#[cfg(test)] mod tests` ends at line 123)

- [ ] **Step 1: Write the failing tests**

Add these three tests inside the existing `#[cfg(test)] mod tests { ... }` block in `backend/crates/sim-core/src/time/mod.rs`, just before its closing `}` (after the existing `age_years_is_elapsed_ticks_times_rate` test at line 122):

```rust
    #[test]
    fn month_start_seconds_is_month_times_month_length() {
        let clock = SimClock {
            sim_seconds_per_tick: 200,
        };
        assert_eq!(clock.month_start_seconds(0), 0);
        assert_eq!(clock.month_start_seconds(1), SECONDS_PER_MONTH);
        // 12 months is exactly one year (SECONDS_PER_MONTH = SECONDS_PER_YEAR / 12, exact).
        assert_eq!(clock.month_start_seconds(12), SECONDS_PER_YEAR);
    }

    #[test]
    fn age_years_at_uses_the_given_instant_not_now() {
        let clock = SimClock {
            sim_seconds_per_tick: 200,
        };
        // Agent born at tick 0. Age queried at the 1-year and 2-year marks.
        let one_year = clock.age_years_at(SECONDS_PER_YEAR, 0);
        assert!((one_year - 1.0).abs() < 1e-3, "got {one_year}");
        let two_years = clock.age_years_at(2 * SECONDS_PER_YEAR, 0);
        assert!((two_years - 2.0).abs() < 1e-3, "got {two_years}");
    }

    #[test]
    fn age_years_at_saturates_to_zero_before_birth() {
        let clock = SimClock {
            sim_seconds_per_tick: 200,
        };
        // Born at tick 1000 (sim-second 200_000); queried at sim-second 0 → not yet born.
        assert_eq!(clock.age_years_at(0, 1000), 0.0);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run:
```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core time::tests
```
Expected: FAIL — `no method named month_start_seconds`/`age_years_at found for struct SimClock`.

- [ ] **Step 3: Implement the helpers**

In `backend/crates/sim-core/src/time/mod.rs`, add the two methods to `impl SimClock`, immediately after the `month_index` method (after line 45, before the closing `}` of the impl on line 46):

```rust
    /// Absolute sim-seconds at the start of `month` (month 0 begins at second 0).
    pub fn month_start_seconds(&self, month: u64) -> u64 {
        month.saturating_mul(SECONDS_PER_MONTH)
    }

    /// Age in years at an absolute sim-second `at_sim_second`, for an agent born
    /// at `birth_tick`. Saturates to 0 if the agent is born after that instant.
    pub fn age_years_at(&self, at_sim_second: u64, birth_tick: u64) -> f32 {
        at_sim_second.saturating_sub(self.sim_seconds(birth_tick)) as f32
            / SECONDS_PER_YEAR as f32
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run:
```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core time::tests
```
Expected: PASS (all `time::tests` green, including the three new ones).

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/time/mod.rs
git commit -m "feat(time): add month_start_seconds and age_years_at helpers"
```

---

## Task 2: Persist `last_processed_month` with a restore replay-guard

This is the headline bug fix. Red-first via an integration test that reproduces the replay storm, then the persistence change makes it green.

**Files:**
- Create: `backend/crates/sim-core/tests/population_persistence_reload.rs`
- Modify: `backend/crates/sim-core/src/mobility/persist_snapshot.rs` (struct lines 60-70; Serialize impl lines 72-108; Deserialize impl lines 110-137; `extract_from_world` return literal lines 188-197; `apply_into_world` lines 845-869)
- Modify (compiler-driven): every other `MobilityPersistSnapshot { .. }` struct literal (see Step 5)

- [ ] **Step 1: Write the failing reload-regression test**

Create `backend/crates/sim-core/tests/population_persistence_reload.rs` with exactly this content:

```rust
//! Regression: demographic state must survive a snapshot save+reload without
//! replaying months. Before the fix, `LastProcessedMonth` was not persisted and
//! reset to 0 on reload while `Tick` was restored, so the first post-reload tick
//! replayed every month from 1..=month_index(tick) in one go (mass die-off /
//! baby boom, duplicate agents). These tests pin the fixed behaviour.

use std::collections::HashSet;

use sim_core::ids::AgentId;
use sim_core::mobility::api::{empty_world_and_schedule, spawn_agent_from_record};
use sim_core::mobility::components::Sex;
use sim_core::mobility::resources::{AgentIdIndex, Tick};
use sim_core::mobility::{
    AgentMobilityState, AgentRecord, MobilityPersistSnapshot, PlanStage, apply_into_world,
    extract_from_world,
};
use sim_core::population::{LastProcessedMonth, PopulationConfig, population_monthly_system};
use sim_core::time::{SECONDS_PER_MONTH, SECONDS_PER_YEAR, SimClock};

/// Build a world seeded with a small cohort and aged forward several months so
/// that `LastProcessedMonth` and `Tick` are both well past 0. Returns the world.
fn aged_world() -> bevy_ecs::world::World {
    let (mut world, _schedule) = empty_world_and_schedule();

    let clock = *world.resource::<SimClock>();
    let ticks_per_year = SECONDS_PER_YEAR / clock.sim_seconds_per_tick;
    let ticks_per_month = SECONDS_PER_MONTH / clock.sim_seconds_per_tick;

    // "Now" ≈ sim-year 50.
    let now_tick: u64 = 50 * ticks_per_year;
    world.resource_mut::<Tick>().0 = now_tick;
    let now_month = clock.month_index(now_tick);
    // Start one month behind "now" so each call processes exactly one month.
    world.resource_mut::<LastProcessedMonth>().0 = now_month.saturating_sub(1);

    // Use defaults (TFR 2.1, realistic mortality): a handful of 30-year-olds.
    world.insert_resource(PopulationConfig::default());

    for i in 0..6 {
        let id = AgentId(format!("agent:seed:{i}"));
        let birth_tick = now_tick - 30 * ticks_per_year;
        let mut rec = AgentRecord::new_born_at(
            id.clone(),
            AgentMobilityState::AtActivity {
                activity_id: "home".to_string(),
            },
            vec![PlanStage::Activity {
                activity_id: "home".to_string(),
            }],
            1.0,
            birth_tick,
        );
        rec.sex = if i % 2 == 0 { Sex::Female } else { Sex::Male };
        spawn_agent_from_record(&mut world, rec);
    }

    // Advance a few months so LastProcessedMonth advances past its start value.
    for _ in 0..3 {
        population_monthly_system(&mut world);
        let cur = world.resource::<Tick>().0;
        world.resource_mut::<Tick>().0 = cur + ticks_per_month;
    }
    population_monthly_system(&mut world);

    world
}

fn living_ids(world: &bevy_ecs::world::World) -> HashSet<String> {
    world
        .resource::<AgentIdIndex>()
        .0
        .keys()
        .map(|id| id.0.clone())
        .collect()
}

#[test]
fn reload_does_not_replay_months_or_change_population() {
    let world = aged_world();
    let clock = *world.resource::<SimClock>();
    let snap = extract_from_world(&world);
    let before_ids = living_ids(&world);
    let saved_tick = snap.tick;

    // Round-trip through JSON exactly like the persistence layer does.
    let json = serde_json::to_string(&snap).expect("serialize");
    let restored: MobilityPersistSnapshot = serde_json::from_str(&json).expect("deserialize");

    let (mut reloaded, _schedule) = empty_world_and_schedule();
    apply_into_world(&mut reloaded, restored);

    // The resume month must match the saved tick — NOT reset to 0.
    assert_eq!(
        reloaded.resource::<LastProcessedMonth>().0,
        clock.month_index(saved_tick),
        "LastProcessedMonth must be restored to the saved month, not 0"
    );

    // Running the monthly system at the restored tick must be a no-op: there is
    // no uncrossed month, so no births/deaths and no duplicate spawns.
    population_monthly_system(&mut reloaded);
    let after_ids = living_ids(&reloaded);

    assert_eq!(
        after_ids, before_ids,
        "reload + one population tick must not change the living-agent set \
         (no replay storm, no duplicates)"
    );
}
```

- [ ] **Step 2: Run the regression test to verify it fails**

Run:
```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --test population_persistence_reload
```
Expected: FAIL — `LastProcessedMonth` comes back as 0 (asserted `== month_index(saved_tick)`), and/or the population set changes after the replay storm. (If it fails to *compile* because of an unrelated missing symbol, fix the import, not the assertion.)

- [ ] **Step 3: Add the `last_processed_month` field to the snapshot wire format**

In `backend/crates/sim-core/src/mobility/persist_snapshot.rs`:

(a) Struct definition — add the field right after `pub tick: u64,` (line 62):

```rust
pub struct MobilityPersistSnapshot {
    pub tick: u64,
    pub last_processed_month: u64,
    pub agents: HashMap<AgentId, AgentRecord>,
```

(b) `Serialize` impl — in the inner `struct WorldRepr<'a>` (after `tick: u64,` at line 76) add `last_processed_month: u64,`; and in the `WorldRepr { ... }` construction (after `tick: self.tick,` at line 97) add `last_processed_month: self.last_processed_month,`. Result:

```rust
        #[derive(Serialize)]
        struct WorldRepr<'a> {
            tick: u64,
            last_processed_month: u64,
            agents: &'a HashMap<AgentId, AgentRecord>,
            // ... unchanged ...
        }
        // ... unchanged sorting ...
        WorldRepr {
            tick: self.tick,
            last_processed_month: self.last_processed_month,
            agents: &self.agents,
            // ... unchanged ...
        }
        .serialize(ser)
```

(c) `Deserialize` impl — in the inner `struct WorldRepr` (after `tick: u64,` at line 114) add the field with a serde default so legacy snapshots (without it) still load; and in the `Ok(Self { ... })` (after `tick: repr.tick,` at line 127) add `last_processed_month: repr.last_processed_month,`. Result:

```rust
        #[derive(Deserialize)]
        struct WorldRepr {
            tick: u64,
            #[serde(default)]
            last_processed_month: u64,
            agents: HashMap<AgentId, AgentRecord>,
            // ... unchanged ...
        }
        let repr = WorldRepr::deserialize(de)?;
        Ok(Self {
            tick: repr.tick,
            last_processed_month: repr.last_processed_month,
            agents: repr.agents,
            // ... unchanged ...
        })
```

- [ ] **Step 4: Populate the field on extract and restore it with the replay-guard**

In `backend/crates/sim-core/src/mobility/persist_snapshot.rs`:

(a) `extract_from_world` — in the returned literal (lines 188-197), add the field right after `tick: world.resource::<Tick>().0,`:

```rust
    MobilityPersistSnapshot {
        tick: world.resource::<Tick>().0,
        last_processed_month: world
            .resource::<crate::population::LastProcessedMonth>()
            .0,
        agents: agents_map,
        // ... unchanged ...
    }
```

(b) `apply_into_world` — immediately after `world.resource_mut::<Tick>().0 = snap.tick;` (line 846), insert the restore guard:

```rust
    world.resource_mut::<Tick>().0 = snap.tick;
    // Restore the demographic cursor. Use max(persisted, month_index(tick)) so:
    //  - a valid snapshot (invariant: LastProcessedMonth == month_index(tick))
    //    restores its exact value; the max is a no-op.
    //  - a legacy snapshot (field absent → serde default 0) derives the correct
    //    resume month from the already-persisted tick, so the population system
    //    never replays the whole history. Frozen-time model: sim-time resumes
    //    from the saved tick, there is no offline catch-up.
    let restored_month = snap
        .last_processed_month
        .max(world.resource::<crate::time::SimClock>().month_index(snap.tick));
    world
        .resource_mut::<crate::population::LastProcessedMonth>()
        .0 = restored_month;
```

(Both `SimClock` and `LastProcessedMonth` are guaranteed present: `empty_world_and_schedule` installs `TimePlugin` + `PopulationPlugin` for tests, and production hydration in `sim-server/src/runtime/mod.rs` installs them before calling `apply_into_world`. This matches the existing pattern where `apply_into_world` already assumes `Tick`/`FlowCells` are installed.)

- [ ] **Step 5: Fix every other snapshot struct literal (compiler-driven)**

Adding a field breaks all explicit `MobilityPersistSnapshot { .. }` literals. Let the compiler enumerate them exhaustively:

Run:
```bash
scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-core
```
Expected at first: errors `missing field 'last_processed_month' in initializer of MobilityPersistSnapshot`.

For each reported site, add `last_processed_month: 0,` right after the `tick: ..,` line. Known sites to update (verify against compiler output — there may be more, and some are in `#[cfg(test)]` blocks only surfaced by the test build):
- `backend/crates/sim-core/tests/mobility_persistence_round_trip.rs` — `active_route_snapshot()` (~line 54) and `multi_step_active_route_snapshot()` (~line 145): add `last_processed_month: 0,` after `tick: 7,` and `tick: 9,` respectively.
- `backend/crates/sim-core/src/mobility/snapshot_provider.rs` — any literal in its test module (~line 96).
- `backend/crates/sim-core/src/mobility/persist_snapshot.rs` — any literal in its own `#[cfg(test)]` module.

Then also compile the test target to surface test-only literals:
```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --no-run
```
Fix any further `missing field` errors the same way (`last_processed_month: 0,`).

Also check the server crate (it constructs `authored_snap` in `runtime/tests.rs`):
```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server --no-run
```
If `runtime/tests.rs` mutates a snapshot via `authored_snap.tick = ..` it came from `extract_from_world`/a constructor and needs no change; only fix actual `MobilityPersistSnapshot { .. }` literals the compiler flags.

- [ ] **Step 6: Run the regression test — it must now pass**

Run:
```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --test population_persistence_reload
```
Expected: PASS — `LastProcessedMonth` restored to `month_index(saved_tick)`, population unchanged after reload + one tick.

- [ ] **Step 7: Run the full sim-core test suite to confirm nothing regressed**

Run:
```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core
```
Expected: PASS (all green, including existing `mobility_persistence_round_trip`, `population_lifecycle`, and the in-crate `mobility::tests` round-trip/determinism tests — the new field is 0 in fresh/equal worlds so equality assertions still hold).

- [ ] **Step 8: Commit**

```bash
git add backend/crates/sim-core/src/mobility/persist_snapshot.rs \
        backend/crates/sim-core/tests/population_persistence_reload.rs \
        backend/crates/sim-core/tests/mobility_persistence_round_trip.rs \
        backend/crates/sim-core/src/mobility/snapshot_provider.rs
# include any other files the compiler made you touch in Step 5:
git add -A
git commit -m "fix(population): persist last_processed_month to stop reload replay storm

LastProcessedMonth was not in the snapshot, so it reset to 0 on reload
while Tick was restored — the next tick replayed months 1..=now in one go
(mass die-off / baby boom, duplicate agents, non-idempotent reload). Persist
the field and restore it as max(persisted, month_index(tick)) so valid and
legacy snapshots both resume cleanly."
```

---

## Task 3: Lock in the wire-format behaviour with focused unit tests

Characterization tests for the exact value round-trip and the legacy (field-absent) path. Both are green on write because Task 2 already landed the fix; they pin the contract against future regressions.

**Files:**
- Modify: `backend/crates/sim-core/tests/population_persistence_reload.rs`

- [ ] **Step 1: Write the round-trip + legacy tests**

Append these two tests to `backend/crates/sim-core/tests/population_persistence_reload.rs`:

```rust
#[test]
fn last_processed_month_round_trips_through_json() {
    let world = aged_world();
    let snap = extract_from_world(&world);
    assert!(
        snap.last_processed_month > 0,
        "precondition: aged world has a non-zero last_processed_month"
    );

    let json = serde_json::to_string(&snap).expect("serialize");
    let back: MobilityPersistSnapshot = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(
        back.last_processed_month, snap.last_processed_month,
        "last_processed_month must survive JSON serialize → deserialize"
    );
}

#[test]
fn legacy_snapshot_without_field_resumes_from_tick() {
    // A pre-fix snapshot has no `last_processed_month` key. Build one by taking a
    // real snapshot's JSON and removing the field, proving #[serde(default)]
    // plus the restore guard recover the resume month from `tick`.
    let world = aged_world();
    let clock = *world.resource::<SimClock>();
    let snap = extract_from_world(&world);
    let saved_tick = snap.tick;

    let mut value = serde_json::to_value(&snap).expect("to value");
    value
        .as_object_mut()
        .expect("snapshot serializes as a JSON object")
        .remove("last_processed_month");
    let legacy: MobilityPersistSnapshot =
        serde_json::from_value(value).expect("legacy snapshot deserializes");
    assert_eq!(
        legacy.last_processed_month, 0,
        "absent field must default to 0 on deserialize"
    );

    let (mut reloaded, _schedule) = empty_world_and_schedule();
    apply_into_world(&mut reloaded, legacy);
    assert_eq!(
        reloaded.resource::<LastProcessedMonth>().0,
        clock.month_index(saved_tick),
        "legacy snapshot must resume from month_index(tick), never replay from 0"
    );
}
```

- [ ] **Step 2: Run the tests to verify they pass**

Run:
```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --test population_persistence_reload
```
Expected: PASS (all four tests in the file green).

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-core/tests/population_persistence_reload.rs
git commit -m "test(population): pin last_processed_month round-trip and legacy-snapshot resume"
```

---

## Task 4: Use per-month age in the catch-up loop

Make the mortality/fertility probability thresholds use the agent's age at the *processed month* `m`, matching the month-keyed random draw. Observable only across multi-month catch-up; tested with a forced multi-month span where now-age leaves the fertile window but per-month age does not.

**Files:**
- Modify: `backend/crates/sim-core/src/population/mod.rs` (mortality age at line 137; fertility age at line 176; add a test to the `#[cfg(test)] mod tests` block)

- [ ] **Step 1: Write the failing test**

Add this test inside the `#[cfg(test)] mod tests { ... }` block in `backend/crates/sim-core/src/population/mod.rs` (after the existing `female_agent_gives_birth_deterministically` test, before the module's closing `}`):

```rust
    /// Per-month age: in a multi-month catch-up, fertility must be judged by the
    /// agent's age *in each processed month*, not her age at the final tick.
    /// Here the mother is past `fertile_max` at `now_tick` but was squarely in
    /// the fertile window during the early processed months. With per-month age
    /// she gives birth; with now-tick age she would be skipped every month.
    #[test]
    fn catch_up_judges_fertility_by_per_month_age() {
        use crate::ids::AgentId;
        use crate::mobility::components::{
            AgentMarker, AgentMobilityStateComponent, BirthTick, Sex, StableAgentId, WalkPlan,
            WalkSpeed,
        };
        use crate::mobility::resources::{AgentIdIndex, Tick};
        use crate::mobility::{AgentMobilityState, PlanStage};
        use crate::time::{SECONDS_PER_YEAR, SimClock};
        use bevy_ecs::prelude::*;
        use bevy_ecs::schedule::Schedule;

        let mut world = World::new();
        let mut schedule = Schedule::default();
        world.insert_resource(SimClock::default());
        // Zero mortality (mother cannot die mid-catch-up) + huge TFR so every
        // fertile month is a guaranteed birth (monthly prob ≥ 1).
        world.insert_resource(PopulationConfig {
            mort_a: 0.0,
            mort_b: 0.0,
            tfr: 1000.0,
            ..PopulationConfig::default()
        });
        world.insert_resource(LastProcessedMonth::default());
        world.insert_resource(Tick(0));
        world.insert_resource(AgentIdIndex::default());
        schedule.add_systems(population_monthly_system);

        let clock = *world.resource::<SimClock>();
        let ticks_per_year: u64 = SECONDS_PER_YEAR / clock.sim_seconds_per_tick;

        // now_tick → mother is 55 (past fertile_max=49).
        let now_tick = 200 * ticks_per_year;
        let mother_birth_tick = now_tick - 55 * ticks_per_year;

        // Catch-up window starts when she was 28 (peak fertility): process months
        // (last+1 ..= now_month) in a single system call.
        let age28_tick = mother_birth_tick + 28 * ticks_per_year;
        let start_month = clock.month_index(age28_tick);
        world.resource_mut::<LastProcessedMonth>().0 = start_month.saturating_sub(1);
        world.resource_mut::<Tick>().0 = now_tick;

        let mother_id = AgentId("agent:mother:permonth".to_string());
        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(mother_id.clone()),
                BirthTick(mother_birth_tick),
                Sex::Female,
                AgentMobilityStateComponent(AgentMobilityState::AtActivity {
                    activity_id: "home".to_string(),
                }),
                WalkPlan {
                    stages: vec![PlanStage::Activity {
                        activity_id: "home".to_string(),
                    }],
                    cursor: 0,
                    cyclic: false,
                },
                WalkSpeed(1.0),
                crate::mobility::components::Position { x: 16.0, y: 16.0 },
            ))
            .id();
        world
            .resource_mut::<AgentIdIndex>()
            .0
            .insert(mother_id.clone(), entity);

        // One scheduled run = one multi-month catch-up over (start_month-1, now_month].
        schedule.run(&mut world);

        let born = world
            .resource::<AgentIdIndex>()
            .0
            .keys()
            .any(|id| id.0.starts_with("agent:born:"));
        assert!(
            born,
            "mother must give birth in an early (fertile) processed month when \
             age is computed per-month; got no agent:born:* in the index"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run:
```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core catch_up_judges_fertility_by_per_month_age
```
Expected: FAIL — with now-tick age (55 > 49) the mother is skipped every month, so no `agent:born:*` exists.

- [ ] **Step 3: Switch both age computations to per-month age**

In `backend/crates/sim-core/src/population/mod.rs`:

(a) Mortality phase — replace line 137:

```rust
            let age = clock.age_years(now_tick, birth_tick.0);
```
with:
```rust
            let age = clock.age_years_at(clock.month_start_seconds(m), birth_tick.0);
```

(b) Fertility phase — replace line 176:

```rust
            let age = clock.age_years(now_tick, birth_tick.0);
```
with:
```rust
            let age = clock.age_years_at(clock.month_start_seconds(m), birth_tick.0);
```

(Leave the newborn `birth_tick` stamping at `now_tick` unchanged — only the probability-threshold age changes. `now_tick` remains used elsewhere in the function.)

- [ ] **Step 4: Run the test to verify it passes**

Run:
```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core catch_up_judges_fertility_by_per_month_age
```
Expected: PASS.

- [ ] **Step 5: Run the population + persistence suites to confirm no regression**

Run:
```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core population
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --test population_lifecycle
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --test population_persistence_reload
```
Expected: PASS (single-month operation is unchanged in practice — per-month age at the current month differs from now-age by < 1 month, so existing cohort tests stay green).

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/population/mod.rs
git commit -m "fix(population): compute catch-up age per processed month, not at now_tick"
```

---

## Task 5: Guard against re-spawning an existing child id

Defense in depth: the deterministic child id `agent:born:{mother}:{m}` must never spawn twice. `spawn_agent_from_record` does a blank `AgentIdIndex` insert that would overwrite the index and orphan the prior entity. Skip the birth if the id already exists.

**Files:**
- Modify: `backend/crates/sim-core/src/population/mod.rs` (birth loop around lines 215-251; add a test to the `#[cfg(test)] mod tests` block)

- [ ] **Step 1: Write the failing test**

Add this test inside the `#[cfg(test)] mod tests { ... }` block in `backend/crates/sim-core/src/population/mod.rs` (after the Task 4 test):

```rust
    /// Double-spawn guard: if the deterministic child id already exists in the
    /// index (e.g. it was persisted from a prior run), the birth must be skipped
    /// rather than overwriting the index entry and orphaning the prior entity.
    #[test]
    fn birth_skips_when_child_id_already_exists() {
        use crate::ids::AgentId;
        use crate::mobility::api::spawn_agent_from_record;
        use crate::mobility::components::{
            AgentMarker, AgentMobilityStateComponent, BirthTick, Sex, StableAgentId, WalkPlan,
            WalkSpeed,
        };
        use crate::mobility::resources::{AgentIdIndex, Tick};
        use crate::mobility::{AgentMobilityState, AgentRecord, PlanStage};
        use crate::time::{SECONDS_PER_YEAR, SimClock};
        use bevy_ecs::prelude::*;
        use bevy_ecs::schedule::Schedule;

        let mut world = World::new();
        let mut schedule = Schedule::default();
        world.insert_resource(SimClock::default());
        // Zero mortality + huge TFR ⇒ the mother gives birth in the single
        // processed month with certainty.
        world.insert_resource(PopulationConfig {
            mort_a: 0.0,
            mort_b: 0.0,
            tfr: 1000.0,
            ..PopulationConfig::default()
        });
        world.insert_resource(LastProcessedMonth::default());
        world.insert_resource(Tick(0));
        world.insert_resource(AgentIdIndex::default());
        schedule.add_systems(population_monthly_system);

        let clock = *world.resource::<SimClock>();
        let ticks_per_year: u64 = SECONDS_PER_YEAR / clock.sim_seconds_per_tick;
        let now_tick = 100 * ticks_per_year;
        let mother_birth_tick = now_tick - 28 * ticks_per_year; // peak fertility
        let now_month = clock.month_index(now_tick);
        world.resource_mut::<LastProcessedMonth>().0 = now_month.saturating_sub(1);
        world.resource_mut::<Tick>().0 = now_tick;

        let mother_id = AgentId("agent:mother:dup".to_string());
        let mother_entity = world
            .spawn((
                AgentMarker,
                StableAgentId(mother_id.clone()),
                BirthTick(mother_birth_tick),
                Sex::Female,
                AgentMobilityStateComponent(AgentMobilityState::AtActivity {
                    activity_id: "home".to_string(),
                }),
                WalkPlan {
                    stages: vec![PlanStage::Activity {
                        activity_id: "home".to_string(),
                    }],
                    cursor: 0,
                    cyclic: false,
                },
                WalkSpeed(1.0),
                crate::mobility::components::Position { x: 16.0, y: 16.0 },
            ))
            .id();
        world
            .resource_mut::<AgentIdIndex>()
            .0
            .insert(mother_id.clone(), mother_entity);

        // Pre-seed a sentinel under the exact child id the system would generate
        // this month, simulating an already-persisted child.
        let child_id = AgentId(format!("agent:born:{}:{}", mother_id.0, now_month));
        let sentinel = spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                child_id.clone(),
                AgentMobilityState::AtActivity {
                    activity_id: "home".to_string(),
                },
                vec![PlanStage::Activity {
                    activity_id: "home".to_string(),
                }],
                1.0,
            ),
        );

        schedule.run(&mut world);

        // The index must still point at the sentinel (not overwritten), and only
        // one entity may carry that StableAgentId.
        assert_eq!(
            world.resource::<AgentIdIndex>().0.get(&child_id).copied(),
            Some(sentinel),
            "existing child id must not be overwritten by a duplicate birth"
        );
        let mut q = world.query::<&StableAgentId>();
        let dup_count = q
            .iter(&world)
            .filter(|sid| sid.0 == child_id)
            .count();
        assert_eq!(
            dup_count, 1,
            "there must be exactly one entity for the child id (no orphaned duplicate)"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run:
```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core birth_skips_when_child_id_already_exists
```
Expected: FAIL — without the guard the birth overwrites the index with a new entity (sentinel orphaned); `get(child_id) != sentinel` and/or `dup_count == 2`.

- [ ] **Step 3: Add the guard in the birth loop**

In `backend/crates/sim-core/src/population/mod.rs`, in the `for candidate in candidates { ... }` loop, immediately after the `child_id` is computed (currently lines 216-217):

```rust
        for candidate in candidates {
            let child_id =
                crate::ids::AgentId(format!("agent:born:{}:{}", candidate.mother_id.0, m));
```
insert the guard right after that `let child_id = ...;` statement:

```rust
            // Never re-spawn a deterministic child id that already exists (e.g.
            // restored from a prior run): spawn_agent_from_record would overwrite
            // the AgentIdIndex entry and orphan the existing entity.
            if world
                .resource::<crate::mobility::resources::AgentIdIndex>()
                .0
                .contains_key(&child_id)
            {
                continue;
            }
```

- [ ] **Step 4: Run the test to verify it passes**

Run:
```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core birth_skips_when_child_id_already_exists
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/population/mod.rs
git commit -m "fix(population): skip births whose deterministic child id already exists"
```

---

## Task 6: Full gate

Confirm the whole backend is green and formatted before integration.

- [ ] **Step 1: Clear any cargo orphans**

```bash
pgrep -f cargo || echo "no cargo running"
```
If any stragglers are listed, kill them before proceeding.

- [ ] **Step 2: Format check**

Run:
```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
```
Expected: PASS (no diff). If it reports diffs, run without `--check` to apply, then re-run the check:
```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all
```
(Per memory: CI uses `@stable`; if local fmt and CI disagree later, `rustup update stable` and re-run.)

- [ ] **Step 3: Clippy (workspace, scoped, no parallel cargo)**

Run:
```bash
scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```
Expected: PASS (no warnings).

- [ ] **Step 4: Full test suite for the affected crates**

Run (sequentially — never two cargo at once):
```bash
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server
```
Expected: PASS for both.

- [ ] **Step 5: Commit any formatting-only changes**

```bash
git add -A
git diff --cached --quiet || git commit -m "style: cargo fmt"
```

---

## Finishing (worktree → merge → GitHub)

After Task 6 is fully green, use `superpowers:finishing-a-development-branch` to integrate. The user's intent: **commit everything, merge, and push to GitHub.** Concretely:

1. Verify the working tree is clean and all tasks are committed (`git status`).
2. Push the worktree branch to `origin` and open a PR (or fast-forward into the base branch, per the finishing skill's guidance and the repo convention of delivering via worktree → PR → origin).
3. **Verify CI is green before merge** (per memory): use `gh pr checks --watch` / `gh ... --exit-status`; never `--admin`-merge over a red check. If only the fmt step is red while local fmt passed, the local toolchain is behind CI `@stable` — `rustup update stable`, reformat, re-push.
4. Merge once CI is green; confirm the merge on `origin/main` (do **not** reset local `main` — the user commits there in parallel).
5. Clean up the worktree.

---

## Self-review notes (already reconciled against the spec)

- **Spec §Design.1 (persist + restore guard)** → Task 2 (field, extract, `max(field, month_index(tick))` restore) + Task 3 (round-trip + legacy tests).
- **Spec §Design.2 (per-month age)** → Task 1 (helpers) + Task 4 (loop swap + boundary test).
- **Spec §Design.3 (double-spawn guard)** → Task 5.
- **Spec §Testing.1 (reload regression)** → Task 2 Step 1. **§Testing.2 (round-trip)** → Task 3. **§Testing.3 (legacy)** → Task 3. **§Testing.4 (helper units)** → Task 1. **§Testing.5 (guard)** → Task 5. **§Testing.6 (existing green)** → Task 2 Step 7, Task 4 Step 5, Task 6.
- **Spec §Accepted limitations (newborns not reconsidered mid-catch-up)** → intentionally NOT implemented; no task, by design.
- **Spec §Non-goals (no offline catch-up; no salt/Gompertz/ASFR change; no frontend)** → respected; no task touches those.
- **Type consistency:** `month_start_seconds(u64)->u64`, `age_years_at(u64,u64)->f32`, field `last_processed_month: u64`, guard via `AgentIdIndex.0.contains_key(&AgentId)` — names used identically across Tasks 1, 2, 4, 5.
