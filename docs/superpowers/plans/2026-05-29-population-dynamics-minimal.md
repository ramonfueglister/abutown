# Population Dynamics — Minimal Birth/Death Implementation Plan (8l, slice 1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Per-agent, deterministic birth + death for active agents — Gompertz–Makeham mortality and an age-specific fertility curve, evaluated once per sim-month, as a `PopulationPlugin` built on 8i.

**Architecture:** A `PopulationPlugin` registers a `Sex` component, a `PopulationConfig` resource, and a monthly-cadence system. Each sim-month, every active agent draws a deterministic hash-based unit value: women in the reproductive window may birth a new age-0 agent (spawned at the mother's position, with `parent_id`); any agent may die (despawn) by the Gompertz–Makeham hazard. Aggregate cohort dynamics for unobserved chunks and the tracked-lineage layer are deferred to later slices.

**Tech Stack:** Rust (sim-core, bevy_ecs), builds on 8i's `SimClock`/`birth_tick`/`DeterministicRng`.

**Spec:** `docs/superpowers/specs/2026-05-29-population-dynamics-minimal-design.md`

**Branch / isolation:** Work on `plan/population-dynamics` (off current `main`, which already has 8i + Codex's sidewalks). **Route every cargo through `scripts/cargo-serial.sh`.** Add `cargo fmt --check` to each task's verify (lesson from 8i: subagents must fmt).

## Grounding (verified on current main)
- `AgentRecord` (mobility/records.rs): `id, state, plan, plan_cursor, walk_speed_per_tick, birth_tick (#[serde(default)]), active_route`. `new_born_at(id, state, plan, walk_speed, birth_tick)`.
- Component pattern (mobility/components.rs): `pub struct BirthTick(pub u64);` (derive Component, …). `Position { x, y }` exists on agents.
- `SimClock` (time/mod.rs): `sim_seconds(tick)`, `age_years(now_tick, birth_tick)`, `SECONDS_PER_YEAR`, `SECONDS_PER_DAY`. `SimDate { year, day_of_year, … }` — **no month field**.
- `DeterministicRng` (world/resources.rs): sequential `from_world_id`, `next_u32/u64/f32` — order-dependent, so 8l uses a **hash-based** per-event draw instead (below).
- Authoritative tick via `mobility::api::tick(world)`; `Tick` resource.
- `SimPlugin` trait: `crate::world::schedule::SimPlugin` (`install(&self, world, schedule)`); plugins installed in `sim-server/src/runtime.rs` (two constructors) + `mobility::api::empty_world_and_schedule`.

---

## Task 1: `Sex` component + AgentRecord fields + deterministic sex at seed

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/components.rs` (add `Sex`)
- Modify: `backend/crates/sim-core/src/mobility/records.rs` (`AgentRecord` gains `sex`, `parent_id`)
- Modify: `backend/crates/sim-core/src/mobility/api.rs` (`spawn_agent_from_record` stamps `Sex`)
- Modify: `backend/crates/sim-core/src/mobility/seed.rs` (assign sex deterministically)
- Test: `backend/crates/sim-core/src/mobility/systems/tests.rs`

- [ ] **Step 1: Failing test** — append to `systems/tests.rs`:
```rust
#[test]
fn spawned_agent_carries_sex_and_parent() {
    use crate::mobility::components::Sex;
    let (mut world, _s) = crate::mobility::api::empty_world_and_schedule();
    let mut rec = crate::mobility::records::AgentRecord::new_born_at(
        crate::ids::AgentId("agent:f".into()),
        crate::mobility::records::AgentMobilityState::AtActivity { activity_id: "a".into() },
        vec![crate::mobility::records::PlanStage::Activity { activity_id: "a".into() }],
        0.05, 0,
    );
    rec.sex = Sex::Female;
    rec.parent_id = Some(crate::ids::AgentId("agent:mum".into()));
    let e = crate::mobility::api::spawn_agent_from_record(&mut world, rec);
    assert_eq!(*world.get::<Sex>(e).unwrap(), Sex::Female);
}
```
Run `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core spawned_agent_carries_sex_and_parent` → FAIL.

- [ ] **Step 2: Add `Sex`** in `components.rs`:
```rust
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Sex {
    Male,
    Female,
}
impl Default for Sex {
    fn default() -> Self {
        Sex::Male
    }
}
```

- [ ] **Step 3: Extend `AgentRecord`** (records.rs) — add fields with serde defaults (so old snapshots load); do NOT change `new`/`new_born_at` signatures (callers set fields after construction):
```rust
    #[serde(default)]
    pub sex: crate::mobility::components::Sex,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<AgentId>,
```
In `new_born_at`, initialise `sex: Sex::default(), parent_id: None`.

- [ ] **Step 4: Stamp on spawn** — in `spawn_agent_from_record`, destructure `sex`, `parent_id` and add `crate::mobility::components::Sex` (and an optional `ParentId(Option<AgentId>)` component if you want it queryable; minimal: store `parent_id` only in the record/persistence, add the component only if a system needs it) to the spawn tuple: add `sex` to the `world.spawn((...))`. Extract `Sex` back in the world→record path so it round-trips.

- [ ] **Step 5: Deterministic sex at seed** — in `seed.rs` where pedestrian `AgentRecord`s are built, set `sex` from a stable hash of the agent id so the founding population is ~50/50 and reproducible:
```rust
// rec.sex = if (stable_hash(agent_id) & 1) == 0 { Sex::Female } else { Sex::Male };
```
(Define `stable_hash(&AgentId) -> u64` once — see Task 2 Step 3's hashing helper, or a local `std::hash` of the id string.)

- [ ] **Step 6: Verify** — `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core` (pass) · `clippy -p sim-core --all-targets -- -D warnings` · `fmt --check`.

- [ ] **Step 7: Commit**
```
git add -A && git commit -m "feat(pop): Sex component + AgentRecord sex/parent_id, deterministic seed sex

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: PopulationPlugin scaffold — config, month cadence, deterministic draw

**Files:**
- Create: `backend/crates/sim-core/src/population/mod.rs`
- Modify: `backend/crates/sim-core/src/lib.rs` (`pub mod population;`)
- Modify: `backend/crates/sim-core/src/time/mod.rs` (add `month_index`)
- Modify: `runtime.rs` (install) + `mobility::api::empty_world_and_schedule` (install for tests)
- Test: inline `#[cfg(test)]` in `population/mod.rs`

- [ ] **Step 1: Add `month_index` to SimClock** (time/mod.rs):
```rust
pub const SECONDS_PER_MONTH: u64 = SECONDS_PER_YEAR / 12;
impl SimClock {
    pub fn month_index(&self, tick: u64) -> u64 {
        self.sim_seconds(tick) / SECONDS_PER_MONTH
    }
}
```

- [ ] **Step 2: Failing test** — in `population/mod.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unit_draw_is_in_range_and_deterministic() {
        let a = unit_draw(7, 3, 1);
        let b = unit_draw(7, 3, 1);
        assert_eq!(a, b);                       // reproducible
        assert!((0.0..1.0).contains(&a));       // in [0,1)
        assert!(unit_draw(7, 3, 1) != unit_draw(8, 3, 1)); // varies by key
    }
    #[test]
    fn config_has_sane_defaults() {
        let c = PopulationConfig::default();
        assert!(c.mort_c > 0.0 && c.tfr > 0.0 && c.fertile_min < c.fertile_max);
    }
}
```
Run → FAIL.

- [ ] **Step 3: Implement scaffold** (`population/mod.rs`):
```rust
//! Phase 8l (slice 1): per-agent birth/death for active agents. Aggregate cohort
//! and tracked-lineage are later slices. Deterministic + replay-safe.
use bevy_ecs::prelude::Resource;

/// Order-independent, reproducible unit draw in [0,1) for one event.
/// Keyed by a stable agent hash, the sim-month, and a salt (0=death, 1=birth).
pub fn unit_draw(agent_hash: u64, month: u64, salt: u64) -> f32 {
    // splitmix64 over the mixed key — no shared mutable RNG, so iteration order
    // never affects outcomes (unlike a sequential RNG).
    let mut z = agent_hash
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(month.wrapping_mul(0xD1B5_4A32_D192_ED03))
        .wrapping_add(salt.wrapping_mul(0xCA5A_8265_7BEE_9B3D));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // top 24 bits → [0,1)
    ((z >> 40) as f32) / ((1u64 << 24) as f32)
}

pub fn stable_agent_hash(id: &crate::ids::AgentId) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.0.hash(&mut h);
    h.finish()
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct PopulationConfig {
    // Gompertz–Makeham μ(age) = a + b·e^(c·age), age in years.
    pub mort_a: f32,
    pub mort_b: f32,
    pub mort_c: f32,
    // Fertility (Gaussian ASFR scaled to TFR).
    pub tfr: f32,
    pub fert_peak_age: f32,
    pub fert_spread: f32,
    pub fertile_min: f32,
    pub fertile_max: f32,
}
impl Default for PopulationConfig {
    fn default() -> Self {
        Self {
            // ~modern life table order of magnitude; tunable.
            mort_a: 0.0001,
            mort_b: 0.00002,
            mort_c: 0.0866, // doubles ~every 8 years
            tfr: 2.1,
            fert_peak_age: 28.0,
            fert_spread: 6.0,
            fertile_min: 15.0,
            fertile_max: 49.0,
        }
    }
}

/// Cadence bookkeeping: the last sim-month already processed.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct LastProcessedMonth(pub u64);

pub struct PopulationPlugin;
impl crate::world::schedule::SimPlugin for PopulationPlugin {
    fn name(&self) -> &'static str {
        "population"
    }
    fn install(&self, world: &mut bevy_ecs::world::World, schedule: &mut bevy_ecs::schedule::Schedule) {
        world.insert_resource(PopulationConfig::default());
        world.insert_resource(LastProcessedMonth::default());
        // mortality + fertility systems are added in Tasks 3 & 4.
        let _ = schedule;
    }
}
```
(Match the real `SimPlugin` trait path/signature — confirm against `CorePlugin`.)

- [ ] **Step 4: Register** — `pub mod population;` in lib.rs; install `PopulationPlugin` in `runtime.rs` (both constructors) and `empty_world_and_schedule`, AFTER mobility/time plugins.

- [ ] **Step 5: Verify** — `test -p sim-core population::` PASS · clippy · fmt --check.

- [ ] **Step 6: Commit**
```
git add -A && git commit -m "feat(pop): PopulationPlugin scaffold — config, month cadence, deterministic draw

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Mortality system (Gompertz–Makeham, monthly, deterministic)

**Files:**
- Modify: `backend/crates/sim-core/src/population/mod.rs` (hazard fn + `mortality_system`, add to schedule)
- Test: inline tests

- [ ] **Step 1: Failing tests**:
```rust
#[test]
fn gompertz_makeham_monotonic_and_probability_bounded() {
    let c = PopulationConfig::default();
    assert!(mortality_hazard(80.0, &c) > mortality_hazard(20.0, &c));
    let q = death_probability_month(80.0, &c);
    assert!(q > 0.0 && q < 1.0);
    assert!(death_probability_month(80.0, &c) > death_probability_month(20.0, &c));
}
#[test]
fn very_old_agent_dies_within_a_month_run() {
    // a 130-year-old has near-certain monthly death; the system despawns it.
    let (mut world, mut schedule) = crate::mobility::api::empty_world_and_schedule();
    // spawn an ancient agent (birth_tick far in the past relative to a large current tick)…
    // advance Tick so current month > LastProcessedMonth, run schedule, assert entity gone.
}
```
(The second test needs Task 2's cadence wired into the schedule; flesh it out once `mortality_system` is added.)

- [ ] **Step 2: Implement** in `population/mod.rs`:
```rust
pub fn mortality_hazard(age_years: f32, c: &PopulationConfig) -> f32 {
    c.mort_a + c.mort_b * (c.mort_c * age_years).exp()
}
/// Discrete-time monthly death probability: 1 − e^(−μ·Δt), Δt = 1/12 year.
pub fn death_probability_month(age_years: f32, c: &PopulationConfig) -> f32 {
    let mu = mortality_hazard(age_years, c);
    1.0 - (-mu / 12.0).exp()
}
```
And a `mortality_system` that, when `clock.month_index(tick) > last.0`, iterates active agents, computes age via `SimClock::age_years(tick, birth_tick)`, draws `unit_draw(stable_agent_hash(id), month, 0)`, and despawns if `draw < death_probability_month(age, cfg)`. (Run for each month in the gap `(last.0, current]` — normally one.) Update `LastProcessedMonth` at the end. Add the system to the schedule in `install` (it needs `Tick`, `SimClock`, `PopulationConfig`, `LastProcessedMonth`, and a query over agents with `BirthTick` + `StableAgentId` + `Commands` to despawn).

- [ ] **Step 3: Verify** — flesh out the integration test; `test -p sim-core` PASS · clippy · fmt --check.

- [ ] **Step 4: Commit**
```
git add -A && git commit -m "feat(pop): Gompertz-Makeham monthly mortality (deterministic despawn)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Fertility system (ASFR, monthly, deterministic) → birth

**Files:**
- Modify: `backend/crates/sim-core/src/population/mod.rs` (ASFR fn + `fertility_system`)
- Test: inline + a controlled integration test

- [ ] **Step 1: Failing tests**:
```rust
#[test]
fn asfr_peaks_in_window_and_scales_to_tfr() {
    let c = PopulationConfig::default();
    assert!(fertility_rate(28.0, &c) > fertility_rate(45.0, &c));
    assert_eq!(fertility_rate(10.0, &c), 0.0); // below window
    // sum of annual rates over integer ages in the window ≈ TFR (within 5%)
    let total: f32 = (15..=49).map(|a| fertility_rate(a as f32, &c)).sum();
    assert!((total - c.tfr).abs() < 0.05 * c.tfr, "got {total}");
}
#[test]
fn reproductive_female_eventually_gives_birth() {
    // spawn one Female aged ~28; run many sim-months; assert a child appears
    // with age 0, Sex set, parent_id == mother, and birth_tick == now.
}
```

- [ ] **Step 2: Implement** ASFR (Gaussian shape normalised to TFR over integer ages in the window):
```rust
fn asfr_shape(age: f32, c: &PopulationConfig) -> f32 {
    if age < c.fertile_min || age > c.fertile_max {
        return 0.0;
    }
    let z = (age - c.fert_peak_age) / c.fert_spread;
    (-0.5 * z * z).exp()
}
/// Annual age-specific fertility rate, scaled so Σ over the integer window = TFR.
pub fn fertility_rate(age: f32, c: &PopulationConfig) -> f32 {
    let shape = asfr_shape(age, c);
    if shape == 0.0 {
        return 0.0;
    }
    let norm: f32 = (c.fertile_min as i32..=c.fertile_max as i32)
        .map(|a| asfr_shape(a as f32, c))
        .sum();
    c.tfr * shape / norm
}
/// Monthly birth probability for a female of this age.
pub fn birth_probability_month(age_years: f32, c: &PopulationConfig) -> f32 {
    fertility_rate(age_years, c) / 12.0
}
```
And `fertility_system` (runs in the same monthly cadence): for each active **Female** in the window, draw `unit_draw(stable_agent_hash(id), month, 1)`; if `draw < birth_probability_month(age, cfg)`, build a child `AgentRecord::new_born_at(child_id, <idle state>, <minimal plan>, walk_speed, current_tick)` with `sex` from `unit_draw(child_hash, month, 2)` (≥0.5 → Female) and `parent_id = Some(mother_id)`, then spawn it **at the mother's `Position`** (read the mother's `Position` component and set the child's position directly — do NOT route through activity-geometry resolution, which can panic on an unknown activity). Child id e.g. `format!("agent:born:{mother}:{month}")` (unique, deterministic).

> **Verify in impl:** how `spawn_agent_from_record` derives `Position` — if it requires resolving the agent's state to a coord (activity/link lookup), add/);use a spawn path that accepts an explicit position, or set the child's state to one that resolves to the mother's tile. The newborn must not crash spawn. This is open-question #4 from the spec — resolve it here.

- [ ] **Step 3: Verify** — integration test shows a birth; `test -p sim-core` PASS · clippy · fmt --check.

- [ ] **Step 4: Commit**
```
git add -A && git commit -m "feat(pop): age-specific fertility → births (age-0 agent with parent_id)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Persistence round-trip for sex + parent_id

**Files:**
- Test: `backend/crates/sim-core/tests/mobility_persistence_round_trip.rs`

- [ ] **Step 1: Test** (mirror the existing `birth_tick_round_trips`): spawn an agent with `sex = Female`, `parent_id = Some("mum")`; `extract_from_world` → JSON → back → `apply_into_world` → assert `sex`/`parent_id` preserved. Run → expected PASS (serde-default fields flow automatically once Task 1 Step 4 extracts `Sex` back). If extraction drops them, fix the extraction path.

- [ ] **Step 2: Verify + Commit**
```
git add -A && git commit -m "test(pop): sex + parent_id survive snapshot round-trip

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Controlled-cohort integration test (the lifecycle works)

**Files:**
- Create: `backend/crates/sim-core/tests/population_lifecycle.rs`

- [ ] **Step 1: Test** — seed a small controlled cohort (e.g., 50 agents, mixed sex, a spread of ages incl. reproductive females and very old agents) in `empty_world_and_schedule`; advance the `Tick` by enough to cross many sim-months, running the schedule each month; assert: (a) at least one **death** occurred (population dropped for the old cohort), (b) at least one **birth** occurred (a child with `age == 0`, `parent_id` set, appeared), (c) re-running with the same world id/seed yields the **identical** final population (determinism). Keep `SIM_SECONDS_PER_TICK`/config explicit so the math is stable.

- [ ] **Step 2: Verify** — `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core` · clippy `--workspace --all-targets -- -D warnings` · `fmt --check`. Also `test --workspace` once to be safe.

- [ ] **Step 3: Commit**
```
git add -A && git commit -m "test(pop): controlled-cohort lifecycle — births, deaths, determinism

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Final verification
- [ ] `scripts/cargo-serial.sh fmt --check`, `clippy --workspace --all-targets -D warnings`, `test --workspace` — all green (run the FULL gate; don't repeat the 8i fmt slip).
- [ ] Determinism: the lifecycle test reproduces identically across runs.
- [ ] `superpowers:finishing-a-development-branch` → PR; confirm CI green **with `gh ... --exit-status`** (never merge on a misread).

## Deferred (later 8l slices, designed in the spec — not here)
- Aggregate cohort-component (Leslie) for warm/asleep chunks (so off-screen population lives at 1M scale).
- Tracked-lineage layer (player dynasties / notable figures persisted across LOD; importance score; culling).
- Sex-specific mortality, old-age logistic, partnerships/households, migration, economy/health hazard modifiers.
- Frontend exposure of sex/age/lineage in the inspector + any wire fields (none needed for the minimal backend lifecycle).
