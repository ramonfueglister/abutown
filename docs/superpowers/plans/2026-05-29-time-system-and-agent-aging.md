# Time System + Agent Aging Implementation Plan (Phase 8i)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the one shared observer-world a deterministic, server-authoritative global clock at a single fixed (tunable) rate, expose a calendar + per-agent age on the wire, and make every agent age — with no player time controls.

**Architecture:** `sim_time` is *derived* from the existing authoritative mobility `Tick` resource (`sim_seconds = tick × SIM_SECONDS_PER_TICK`); no second counter, trivially deterministic/replayable. A `TimePlugin` inserts a `SimClock` resource holding the fixed rate + calendar/age helpers. Aging is the derived property `age = (now_tick − birth_tick) × rate`, with a durable `birth_tick` per agent. Pedestrian movement stays tick-paced (untouched), so it remains watchable while the calendar races — the genre-standard "fudged time". LOD stays interest-driven (untouched).

**Tech Stack:** Rust (sim-core/sim-server, bevy_ecs, prost), Protobuf, TypeScript (Vite/Vitest/Playwright).

**Spec:** `docs/superpowers/specs/2026-05-29-time-system-and-agent-aging-design.md`

**Branch / isolation (codex):** own worktree off latest `main`:
`git worktree add ../abutown-time -b codex/time-system main && cd ../abutown-time`, `export CARGO_TARGET_DIR=/tmp/abutown-time-target`. **Route every cargo through `scripts/cargo-serial.sh`.**

## Key facts (verified against main)
- Authoritative tick = mobility `Tick(pub u64)` (`mobility/resources.rs`), advanced once/run by `tick_increment_system` (`mobility/systems/bookkeeping.rs`, in `MobilitySet::Bookkeeping`). Read via `mobility::api::tick(world) -> u64`.
- Plugins implement `SimPlugin { fn install(&self, world, schedule) }`; registered in `sim-server/src/runtime.rs:203-219`. `empty_world_and_schedule()` (mobility/api.rs) is the test world builder.
- `AgentRecord` (`mobility/records.rs:81`) is `#[derive(Serialize, Deserialize)]` → any new field auto-persists through `MobilityPersistSnapshot`.
- Per-agent components live in `mobility/components.rs` (pattern: `pub struct DwellTicksRemaining(pub u16);`). Spawn in `spawn_agent_from_record` (`mobility/api.rs:398`).
- Wire DTOs: `WorldSummaryDto` + `AgentMobilityDto` in `backend/crates/protocol/src/lib.rs`; proto at `backend/crates/protocol/proto/abutown.proto` (regenerates via prost `build.rs` on cargo build; TS via `npm run generate:proto`). Per-tick agent DTOs built in `mobility/dto.rs` (`build_mobility_snapshot_dto`).

**Default rate:** `SIM_SECONDS_PER_TICK = 200`. At 10 Hz that is ~2000× (1 real day ≈ 5.48 sim-years) — the user's civilization aspiration, **tunable** via this one constant. Movement is unaffected (tick-paced).

---

## Task 1: SimClock + TimePlugin

**Files:**
- Create: `backend/crates/sim-core/src/time/mod.rs`
- Modify: `backend/crates/sim-core/src/lib.rs` (add `pub mod time;` + re-export)
- Modify: `backend/crates/sim-server/src/runtime.rs:203-219` (install TimePlugin) and the test builder `empty_world_and_schedule` path
- Test: inline `#[cfg(test)]` in `time/mod.rs`

- [ ] **Step 1: Write the failing test** — create `backend/crates/sim-core/src/time/mod.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_seconds_is_tick_times_rate() {
        let clock = SimClock { sim_seconds_per_tick: 200 };
        assert_eq!(clock.sim_seconds(0), 0);
        assert_eq!(clock.sim_seconds(10), 2000);
    }

    #[test]
    fn calendar_derives_from_seconds() {
        // 200 s/tick. One sim-year = 365*86400 = 31_536_000 s = 157_680 ticks.
        let clock = SimClock { sim_seconds_per_tick: 200 };
        let d = clock.calendar(157_680);
        assert_eq!(d.year, 1);
        assert_eq!(d.day_of_year, 0);
    }

    #[test]
    fn age_years_is_elapsed_ticks_times_rate() {
        let clock = SimClock { sim_seconds_per_tick: 200 };
        // born at tick 0, now at one sim-year of ticks → age ~1.0
        let years = clock.age_years(157_680, 0);
        assert!((years - 1.0).abs() < 1e-3, "got {years}");
        // born later ages less
        assert!(clock.age_years(157_680, 78_840) < clock.age_years(157_680, 0));
    }
}
```

- [ ] **Step 2: Run, verify FAIL** — `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core time::` → FAIL (module/types missing).

- [ ] **Step 3: Implement `time/mod.rs`** (above the test module):

```rust
//! Deterministic, server-authoritative simulation clock for the single shared
//! observer-world. `sim_time` is derived from the mobility `Tick`; there is no
//! player-facing speed control (see the 8i spec).

use bevy_ecs::prelude::Resource;

pub const SECONDS_PER_DAY: u64 = 86_400;
pub const DAYS_PER_YEAR: u64 = 365;
pub const SECONDS_PER_YEAR: u64 = SECONDS_PER_DAY * DAYS_PER_YEAR;

/// Fixed-rate clock. `sim_seconds_per_tick` is the one tunable time-compression
/// knob; everything else is derived from the authoritative tick.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimClock {
    pub sim_seconds_per_tick: u64,
}

impl Default for SimClock {
    /// ~2000x at a 10 Hz tick (1 real day ≈ 5.48 sim-years). Tunable.
    fn default() -> Self {
        Self { sim_seconds_per_tick: 200 }
    }
}

impl SimClock {
    pub fn sim_seconds(&self, tick: u64) -> u64 {
        tick.saturating_mul(self.sim_seconds_per_tick)
    }
    pub fn calendar(&self, tick: u64) -> SimDate {
        SimDate::from_seconds(self.sim_seconds(tick))
    }
    pub fn age_seconds(&self, now_tick: u64, birth_tick: u64) -> u64 {
        now_tick.saturating_sub(birth_tick).saturating_mul(self.sim_seconds_per_tick)
    }
    pub fn age_years(&self, now_tick: u64, birth_tick: u64) -> f32 {
        self.age_seconds(now_tick, birth_tick) as f32 / SECONDS_PER_YEAR as f32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimDate {
    pub year: u64,
    pub day_of_year: u64,
    pub hour: u64,
    pub minute: u64,
    pub second: u64,
}

impl SimDate {
    pub fn from_seconds(s: u64) -> Self {
        let year = s / SECONDS_PER_YEAR;
        let rem = s % SECONDS_PER_YEAR;
        let day_of_year = rem / SECONDS_PER_DAY;
        let day_rem = rem % SECONDS_PER_DAY;
        Self {
            year,
            day_of_year,
            hour: day_rem / 3600,
            minute: (day_rem % 3600) / 60,
            second: day_rem % 60,
        }
    }
}

/// Plugin that installs the `SimClock` resource. No per-tick system: sim-time is
/// derived from the existing `Tick`. Future calendar-boundary events live here.
pub struct TimePlugin;

impl crate::SimPlugin for TimePlugin {
    fn name(&self) -> &'static str {
        "time"
    }
    fn install(&self, world: &mut bevy_ecs::world::World, _schedule: &mut bevy_ecs::schedule::Schedule) {
        world.insert_resource(SimClock::default());
    }
}
```

(Confirm the exact `SimPlugin` trait path/import by matching `CorePlugin` in `world/plugin.rs`; adjust the `impl crate::SimPlugin` path to the real trait location.)

- [ ] **Step 4: Register the module + plugin** — in `lib.rs` add `pub mod time;`. In `runtime.rs` after `CorePlugin::default().install(...)` add `sim_core::time::TimePlugin.install(&mut world, &mut schedule);`. Ensure `empty_world_and_schedule()` also ends up with a `SimClock` (either it builds via the same plugin path, or insert `SimClock::default()` there) — Task 2/3 tests depend on it.

- [ ] **Step 5: Run, verify PASS** — `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core time::` → PASS. Then `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml -p sim-core --all-targets -- -D warnings` clean.

- [ ] **Step 6: Commit**
```
git add -A && git commit -m "feat(time): deterministic SimClock + TimePlugin (8i)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Per-agent birth stamp + aging

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/records.rs` (`AgentRecord` + `new`)
- Modify: `backend/crates/sim-core/src/mobility/components.rs` (new `BirthTick`)
- Modify: `backend/crates/sim-core/src/mobility/api.rs` (`spawn_agent_from_record` stamps it)
- Modify: any runtime spawn site that builds `AgentRecord`s at a non-zero tick (LOD promotion in `mobility/systems/lod.rs`) to set `birth_tick = current Tick`
- Test: `mobility/systems/tests.rs`

- [ ] **Step 1: Write the failing test** — append to `mobility/systems/tests.rs`:

```rust
#[test]
fn spawned_agent_carries_birth_tick_and_ages() {
    use crate::mobility::components::BirthTick;
    use crate::time::SimClock;
    let (mut world, _schedule) = crate::mobility::api::empty_world_and_schedule();

    let rec = crate::mobility::records::AgentRecord::new_born_at(
        crate::ids::AgentId("agent:test".into()),
        crate::mobility::records::AgentMobilityState::AtActivity { activity_id: "a".into() },
        vec![crate::mobility::records::PlanStage::Activity { activity_id: "a".into() }],
        0.05,
        100, // birth_tick
    );
    let entity = crate::mobility::api::spawn_agent_from_record(&mut world, rec);
    assert_eq!(world.get::<BirthTick>(entity).unwrap().0, 100);

    let clock = SimClock { sim_seconds_per_tick: 200 };
    // now at tick 100 + one sim-year of ticks → age ~1.0
    assert!((clock.age_years(100 + 157_680, 100) - 1.0).abs() < 1e-3);
}
```

- [ ] **Step 2: Run, verify FAIL** — `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core spawned_agent_carries_birth_tick` → FAIL.

- [ ] **Step 3: Add the `BirthTick` component** — in `mobility/components.rs`:
```rust
/// Simulation tick at which this agent was born (spawned). Age is derived from
/// it via `SimClock`. Durable: mirrors `AgentRecord.birth_tick`.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BirthTick(pub u64);
```

- [ ] **Step 4: Add `birth_tick` to `AgentRecord`** — in `mobility/records.rs`, add the field (serde-default so legacy/older snapshots load as born-at-epoch) and a `new_born_at` constructor; keep `new` delegating with `birth_tick: 0`:
```rust
pub struct AgentRecord {
    pub id: AgentId,
    pub state: AgentMobilityState,
    pub plan: Vec<PlanStage>,
    pub plan_cursor: usize,
    pub walk_speed_per_tick: f32,
    #[serde(default)]
    pub birth_tick: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_route: Option<PersistedActiveRoute>,
}

impl AgentRecord {
    pub fn new(id: AgentId, state: AgentMobilityState, plan: Vec<PlanStage>, walk_speed_per_tick: f32) -> Self {
        Self::new_born_at(id, state, plan, walk_speed_per_tick, 0)
    }
    pub fn new_born_at(id: AgentId, state: AgentMobilityState, plan: Vec<PlanStage>, walk_speed_per_tick: f32, birth_tick: u64) -> Self {
        Self { id, state, plan, plan_cursor: 0, walk_speed_per_tick, birth_tick, active_route: None }
    }
}
```

- [ ] **Step 5: Stamp it on spawn + extract it back** — in `spawn_agent_from_record` (api.rs), destructure `birth_tick` and add `BirthTick(birth_tick)` to the spawn tuple. In the world→record extraction (`agents()` / wherever `AgentRecord` is rebuilt from the entity for persistence), read the `BirthTick` component back into `birth_tick` so it round-trips.

- [ ] **Step 6: Stamp current tick at runtime spawn sites** — in `mobility/systems/lod.rs` promotion (and any other place that builds an `AgentRecord` after world start), use `new_born_at(..., current_tick)` reading `world.resource::<Tick>().0`. Initial seed (`seed.rs`) stays `new(...)` (tick 0 = born at epoch — correct for the founding population). Grep to be exhaustive: `rg -n "AgentRecord::new\b" backend/crates`.

- [ ] **Step 7: Run, verify PASS** — `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core` → PASS (new test + existing). clippy clean.

- [ ] **Step 8: Commit**
```
git add -A && git commit -m "feat(time): durable per-agent birth_tick; age derived via SimClock

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Expose sim_time + age on the wire

**Files:**
- Modify: `backend/crates/protocol/proto/abutown.proto` (WorldSummary, AgentMobility)
- Modify: `backend/crates/protocol/src/lib.rs` (`WorldSummaryDto`, `AgentMobilityDto`)
- Modify: `backend/crates/sim-core/src/mobility/dto.rs` (compute `age_seconds` per agent) and the `WorldSummaryDto` build site (compute `sim_time`)
- Test: `backend/crates/sim-server/tests/http.rs` or `websocket.rs`

- [ ] **Step 1: Write the failing test** — in `tests/http.rs` (or websocket), assert the world summary carries `sim_time` and an agent carries a non-`None` age after some ticks. Use the existing test harness pattern (poll `/world` and `/mobility`). Concretely, assert `world.sim_time` advances after ticks and an agent's `age_seconds` is `>= 0`. (Mirror existing `world_id`/`agents.len()` assertions in that file.)

- [ ] **Step 2: Run, verify FAIL** — fields don't exist yet → compile error.

- [ ] **Step 3: Proto fields** — in `abutown.proto`:
```protobuf
message WorldSummary {
  // …existing fields 1-5…
  uint64 sim_time = 6;          // elapsed sim-seconds
}
message AgentMobility {
  // …existing fields 1-6…
  uint64 age_seconds = 7;       // derived: (now_tick - birth_tick) * rate
}
```

- [ ] **Step 4: Rust DTO fields** — in `protocol/src/lib.rs` add `pub sim_time: u64` to `WorldSummaryDto` and `pub age_seconds: u64` to `AgentMobilityDto`. Fix every constructor of these structs (grep: `rg -n "WorldSummaryDto \{|AgentMobilityDto \{" backend`), defaulting to `0` where a real value isn't available (tests/placeholders).

- [ ] **Step 5: Compute the real values** —
  - In the `WorldSummaryDto` build site: `sim_time: world.resource::<SimClock>().sim_seconds(mobility::api::tick(world))`.
  - In `mobility/dto.rs::build_mobility_snapshot_dto` per-agent path: read the agent's `BirthTick` + the `SimClock` + current `Tick`, set `age_seconds: clock.age_seconds(now_tick, birth_tick)`.

- [ ] **Step 6: Regenerate + verify** —
  - Rust proto regenerates on build (prost `build.rs`).
  - `npm run generate:proto` to refresh the TS proto.
  - `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace` → PASS. clippy clean.

- [ ] **Step 7: Commit**
```
git add -A && git commit -m "feat(time): expose sim_time (world) and age_seconds (agent) on the wire

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Persistence round-trip for birth_tick

**Files:**
- Test: `backend/crates/sim-core/tests/mobility_persistence_round_trip.rs`

- [ ] **Step 1: Write the test** — `birth_tick` survives extract→serialize→deserialize→apply. Mirror the existing `active_route_round_trip_preserves_valid_graph_ids` style:
```rust
#[test]
fn birth_tick_round_trips() {
    use sim_core::mobility::components::BirthTick;
    let (mut world, _s) = sim_core::mobility::api::empty_world_and_schedule();
    let rec = sim_core::mobility::AgentRecord::new_born_at(
        sim_core::ids::AgentId("agent:born".into()),
        sim_core::mobility::AgentMobilityState::AtActivity { activity_id: "a".into() },
        vec![sim_core::mobility::PlanStage::Activity { activity_id: "a".into() }],
        0.05, 4242,
    );
    sim_core::mobility::api::spawn_agent_from_record(&mut world, rec);
    let snap = sim_core::mobility::extract_from_world(&world);
    let json = serde_json::to_string(&snap).unwrap();
    let back: sim_core::mobility::MobilityPersistSnapshot = serde_json::from_str(&json).unwrap();
    let (mut w2, _s2) = sim_core::mobility::api::empty_world_and_schedule();
    sim_core::mobility::apply_into_world(&mut w2, back);
    let e = sim_core::mobility::api::agent_entity(&w2, &sim_core::ids::AgentId("agent:born".into())).unwrap();
    assert_eq!(w2.get::<BirthTick>(e).unwrap().0, 4242);
}
```
(Adjust `agent_entity` to the real lookup helper; if none, assert via `extract_from_world(&w2)` finding the record with `birth_tick == 4242`.)

- [ ] **Step 2: Run** — `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core birth_tick_round_trips`. It should PASS already (serde auto-flows birth_tick) once Task 2 Step 5 extracts the component back into the record. If it fails because extraction drops `birth_tick`, fix the extraction path. This task is the guard that persistence is wired.

- [ ] **Step 3: Commit**
```
git add -A && git commit -m "test(time): birth_tick survives snapshot round-trip

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Frontend — show the calendar + agent age

**Files:**
- Modify: `src/backend/mobilityClient.ts` (read `sim_time` from world summary; thread into state)
- Modify: the entity-inspector content builder (where agent inspector rows are assembled) + `src/render/inspectorPanelPainter.ts` consumers — add an "Age" row
- Modify: wherever a small world HUD/overlay is drawn (or add a minimal date label) — show the calendar
- Test: `tests/backend/*` (sim_time parsed) + a render/inspector test

- [ ] **Step 1: Write failing tests** — a vitest that a mocked world-summary proto with `sim_time` is parsed into state, and that an agent inspector includes an "Age" row when `age_seconds` is present. Mirror `tests/backend/mobilityClient.test.ts` proto-mock style (`create(WorldSummarySchema, { … simTime })`, `toBinary`).

- [ ] **Step 2: Run, verify FAIL.**

- [ ] **Step 3: Implement** —
  - Parse `simTime` from the world summary proto into the client state; format `simTime` seconds → a compact date (`Year N, Day D`) using the same constants (`SECONDS_PER_YEAR=31_536_000`, `SECONDS_PER_DAY=86_400`).
  - In the agent inspector content, add a row `{ label: 'Age', value: \`${(age_seconds/31_536_000).toFixed(1)} yr\` }`.
  - Draw the date label in the existing HUD/overlay layer (keep it unobtrusive).

- [ ] **Step 4: Run, verify PASS** — `npm run typecheck && npm test`.

- [ ] **Step 5: Commit**
```
git add -A && git commit -m "feat(time): frontend shows world date + agent age

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Browser smoke (mandatory — crosses the wire)

**Files:**
- Modify: `tests/e2e/render-smoke.spec.ts` (extend) or a new `tests/e2e/time-smoke.spec.ts`

Per CLAUDE.md, a frontend↔backend boundary change (new wire fields + display) **requires a real browser smoke**.

- [ ] **Step 1: Write the smoke** — boot the dev stack (abutopia world), read `window.render_game_to_text?.()` (or the existing test hook) twice a few seconds apart, and assert: (a) the world `sim_time`/date **advances** between samples, and (b) the pedestrian's reported **age increases** (or is present and ≥ 0). Reuse the existing render-smoke harness/structure.

- [ ] **Step 2: Run** —
```
CORS_ALLOWED_ORIGINS=http://127.0.0.1:5173 npm run build && CORS_ALLOWED_ORIGINS=http://127.0.0.1:5173 npx playwright test tests/e2e/
```
Expected: PASS — the clock visibly advances and the agent ages.

- [ ] **Step 3: Commit**
```
git add -A && git commit -m "test(e2e): smoke the live clock + agent aging over the wire

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Final verification
- [ ] `scripts/cargo-serial.sh fmt … --check` · `clippy --workspace --all-targets -D warnings` · `test --workspace` — all green.
- [ ] `npm run typecheck && npm test` green; browser smoke green.
- [ ] No player-facing speed/pause control exists anywhere (grep the frontend for any speed UI — there must be none; this is the observer-MMO guard).
- [ ] `superpowers:finishing-a-development-branch`.

## Deferred (not in 8i)
- Calendar-boundary **events** (`SimDayElapsed`/`…Month`/`…Year`) — add when a consumer phase needs them (8h economy, 8m weather, 8l population). The `SimClock` already makes them trivial to add in `TimePlugin`.
- Life stages, death/lifespan, births, demographic age-in-flow — 8l.
- Tuning the final `SIM_SECONDS_PER_TICK` (default 200 ≈ 2000×).
