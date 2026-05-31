# Demographic Persistence Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the population system from replaying its entire demographic history (mass die-off / baby-boom, duplicate agents) on every server restart by persisting `LastProcessedMonth`, and make the catch-up loop age-accurate.

**Architecture:** The monthly population system advances `(LastProcessedMonth, current_month]` once per tick. `Tick` is persisted but `LastProcessedMonth` is not, so after a reload it resets to 0 while `Tick` is restored — the next tick replays thousands of months at once. The fix persists `last_processed_month` as a required snapshot field and restores it directly (no legacy shim, no fallback guard). One supporting change makes the catch-up loop age-correct: per-month age in the probability thresholds.

**Tech Stack:** Rust, `bevy_ecs`, `serde` + `serde_json`. All cargo MUST run through `scripts/cargo-serial.sh` (never two cargo at once). Pure backend change → no browser smoke.

**Spec:** `docs/superpowers/specs/2026-05-31-demographic-persistence-fix-design.md`

**Directive (2026-05-31):** No legacy-snapshot compatibility, no defensive fallback guards. Required field, direct restore, fix the root cause only.

---

## Status: implemented

All tasks below were implemented in this worktree (branched off `origin/main`,
where the bug was verified present). Each task is TDD (failing test first, then
the production change), committed separately. The double-spawn guard from an
earlier draft was dropped — once the cursor is persisted, each `(mother, month)`
is processed exactly once, so the duplicate child id is unreachable.

---

## File Structure

- **`backend/crates/sim-core/src/time/mod.rs`** — `SimClock` time math. Two pure helpers + unit tests. (Task 1)
- **`backend/crates/sim-core/src/mobility/persist_snapshot.rs`** — `MobilityPersistSnapshot`, extract, apply. Required `last_processed_month` field + direct restore. (Task 2)
- **`backend/crates/sim-core/tests/population_persistence_reload.rs`** *(new)* — reload regression + value round-trip. (Tasks 2 & 3)
- **`backend/crates/sim-core/tests/mobility_persistence_round_trip.rs`** — add required field to 2 fixtures. (Task 2, compiler-driven)
- **`backend/crates/sim-core/src/population/mod.rs`** — per-month age in catch-up loop + test. (Task 4)

---

## Task 1: Add per-instant age helpers to `SimClock`

`month_start_seconds(month)` and `age_years_at(at_sim_second, birth_tick)` on
`impl SimClock`, with three unit tests (`month_start_seconds_is_month_times_month_length`,
`age_years_at_uses_the_given_instant_not_now`, `age_years_at_saturates_to_zero_before_birth`).

```rust
/// Absolute sim-seconds at the start of `month` (month 0 begins at second 0).
pub fn month_start_seconds(&self, month: u64) -> u64 {
    month.saturating_mul(SECONDS_PER_MONTH)
}

/// Age in years at an absolute sim-second `at_sim_second`, for an agent born
/// at `birth_tick`. Saturates to 0 if the agent is born after that instant.
pub fn age_years_at(&self, at_sim_second: u64, birth_tick: u64) -> f32 {
    at_sim_second.saturating_sub(self.sim_seconds(birth_tick)) as f32 / SECONDS_PER_YEAR as f32
}
```

Verify: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core time::tests`

---

## Task 2: Persist `last_processed_month` (required field, direct restore)

Headline fix. Red-first via a behavioral reload test that reproduces the replay
storm using only the public API (so it compiles against pre-fix code and fails
there).

**Files:**
- Create: `backend/crates/sim-core/tests/population_persistence_reload.rs`
- Modify: `backend/crates/sim-core/src/mobility/persist_snapshot.rs`
- Modify (compiler-driven): `backend/crates/sim-core/tests/mobility_persistence_round_trip.rs` (2 fixtures)

The reload-regression test (`reload_does_not_replay_months_or_change_population`):
seed a 6-agent cohort with the `"home"` activity waypoint registered (so
`spawn_agent_from_record` resolves `AtActivity` positions), age it ~4 months,
extract → JSON → deserialize → apply into a fresh world, run one population tick,
and assert (a) `LastProcessedMonth` equals its pre-reload value and (b) the
living-agent set is unchanged.

Production changes in `persist_snapshot.rs`:
1. Struct: add `pub last_processed_month: u64` after `pub tick: u64`.
2. `Serialize` `WorldRepr<'a>`: add `last_processed_month: u64` and populate from `self`.
3. `Deserialize` `WorldRepr`: add `last_processed_month: u64` (NO `#[serde(default)]`) and read into `Self`.
4. `extract_from_world`: `last_processed_month: world.resource::<crate::mobility::resources::LastProcessedMonth>().0`.
5. `apply_into_world`, right after `world.resource_mut::<Tick>().0 = snap.tick;`:
   ```rust
   world
       .resource_mut::<crate::mobility::resources::LastProcessedMonth>()
       .0 = snap.last_processed_month;
   ```

(Note: `LastProcessedMonth` is defined in `mobility::resources` and installed by
`install_mobility`; `population` re-exports it via `pub use`. See Task 2b.)

Compiler-driven: the two `MobilityPersistSnapshot { .. }` fixtures in
`mobility_persistence_round_trip.rs` (`tick: 7`, `tick: 9`) each gain
`last_processed_month: 0,`.

Verify: `population_persistence_reload`, `mobility_persistence_round_trip`,
`population_lifecycle`, then the full `-p sim-core` suite.

---

## Task 3: Value round-trip test

Append `last_processed_month_round_trips_through_json` to
`population_persistence_reload.rs`: extract from the aged world, assert the
cursor is non-zero, serialize → deserialize, assert the value is preserved.

---

## Task 4: Per-month age in the catch-up loop

**Files:** `backend/crates/sim-core/src/population/mod.rs` (mortality + fertility age; inline test).

Replace BOTH occurrences of
```rust
let age = clock.age_years(now_tick, birth_tick.0);
```
with
```rust
let age = clock.age_years_at(clock.month_start_seconds(m), birth_tick.0);
```
Newborn `birth_tick` stamping at `now_tick` is unchanged.

Test (`catch_up_judges_fertility_by_per_month_age`): a mother spawned manually
with explicit `Position` (no waypoint dependency), 55 years old at `now_tick`
(past `fertile_max`) but fertile in the early processed months; zero mortality,
`tfr: 1000.0`. One multi-month catch-up run must produce an `agent:born:*`.
Verified red against the `now_tick` version.

---

## Task 5: Full gate

- `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check`
- `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings`
- `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core`
- `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server`

(If CI fmt disagrees with local later: `rustup update stable`, reformat, re-push.)

---

## Finishing (worktree → merge → GitHub)

Use `superpowers:finishing-a-development-branch`:
1. `git status` clean, all committed.
2. Push the worktree branch to `origin`, open a PR.
3. **Verify CI green before merge** (`gh pr checks --watch` / `--exit-status`); never `--admin`-merge a red check.
4. Merge once green; confirm on `origin/main`. Do NOT reset local `main`.
5. Clean up the worktree.
