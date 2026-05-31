# Demographic persistence fix — design spec

**Date:** 2026-05-31
**Branch context:** `plan/persistence-liveness`
**Status:** design, awaiting implementation plan

## Problem

The monthly population system (`backend/crates/sim-core/src/population/mod.rs`,
`population_monthly_system`) replays the entire demographic history in a single
tick after every server restart, producing a mass die-off / baby-boom and
duplicate agent entities.

### Root cause

`LastProcessedMonth` (the resource that records which sim-month the population
system last advanced through) is **not part of the persisted snapshot**:

- `PopulationPlugin::install` (`population/mod.rs`) inserts
  `LastProcessedMonth::default()` (= 0) every time the world is built.
- `MobilityPersistSnapshot` (`mobility/persist_snapshot.rs`) persists `Tick`
  but not `LastProcessedMonth`; `apply_into_world` restores `Tick` and never
  touches `LastProcessedMonth`.

So after a reload, `Tick` is restored to its saved (large) value while
`LastProcessedMonth` is 0. On the first tick the catch-up loop
`for m in (last+1)..=current_month` runs from month 1 to `month_index(saved_tick)`
— potentially dozens of months — in one tick.

### Consequences

1. **Mass mortality + fertility burst** on every reload (months collapsed into
   one tick).
2. **Duplicate child entities.** Child id is deterministic
   (`agent:born:{mother}:{m}`). A child already persisted from the original run
   is "re-born" during replay; `spawn_agent_from_record` (`mobility/api.rs:454`)
   does a blank `AgentIdIndex.insert`, overwriting the index entry **without
   despawning** the previously-mapped entity → orphaned/duplicate entity.
3. **Non-idempotent reload** — the same snapshot loaded twice yields different
   populations, breaking the "Deterministic + replay-safe" promise in the
   module docstring.

### Secondary defects (in the catch-up loop itself)

- **Age uses `now_tick`, not the processed month `m`.** Mortality/fertility
  thresholds are computed from the agent's age at the current tick
  (`population/mod.rs` ~137 and ~176) while the random draw is keyed to month
  `m`. For multi-month catch-up every month wrongly uses the agent's *final*
  age.
- **Newborns are invisible to the same catch-up call.** `agent_entries` is
  snapshotted once before the month loop; agents born in month `m` are not
  reconsidered in months `m+1..` of the same call. (See "Accepted limitations".)

The `unit_draw` salt machinery, the Gompertz–Makeham / ASFR math, and the
display-side age calculation are all correct — they are **not** changed by this
work.

## Intended model (confirmed)

Frozen-time persistence: while the server runs, sim-time advances; while it is
down, nothing advances; on restart the world resumes from the saved `Tick`.
Everything is persistent. There is **no** offline catch-up (sim-time never
jumps to wall-clock on reload). Therefore a per-reload demographic replay is
purely a bug.

## Goals

- After a reload the population system resumes exactly where it left off: no
  replay, no die-off/birth burst, no duplicate entities.
- Idempotent reload: loading the same snapshot any number of times yields the
  same population.
- Existing (pre-fix) snapshots on disk load without a replay storm.
- The catch-up loop is made correct for arbitrary month spans (even though, in
  the frozen model, it runs ≤ 1 month per tick in practice).
- Regression coverage that would have caught the original bug.

## Non-goals

- No offline / wall-clock catch-up of sim-time.
- No change to the salt/PRNG, Gompertz–Makeham, or ASFR math.
- No change to the frontend or the frontend↔backend boundary (pure backend) —
  no browser smoke required.
- `PopulationConfig` is **not** persisted in this work (it is a deterministic
  default today). Noted as a future item if it ever becomes per-world tunable.

## Design

### 1. Persist `LastProcessedMonth` with a replay-guard on restore

All three edits live in `backend/crates/sim-core/src/mobility/persist_snapshot.rs`.

1. **Struct + wire format.** Add `pub last_processed_month: u64` to
   `MobilityPersistSnapshot`. Add the field to the `WorldRepr` serialize and
   deserialize structs; on deserialize mark it `#[serde(default)]` so existing
   JSON (without the field) still loads. Place it next to `tick` for clarity.
2. **Extract.** In `extract_from_world`, set
   `last_processed_month: world.resource::<crate::population::LastProcessedMonth>().0`.
3. **Restore (replay-guard).** In `apply_into_world`, after restoring `Tick`:

   ```rust
   let restored_month = snapshot
       .last_processed_month
       .max(world.resource::<crate::time::SimClock>().month_index(snapshot.tick));
   world.resource_mut::<crate::population::LastProcessedMonth>().0 = restored_month;
   ```

   Rationale for `max(field, month_index(tick))`:
   - **Valid snapshot:** the invariant after every tick is
     `LastProcessedMonth == month_index(tick)` (the system runs every tick and
     sets `LastProcessedMonth = current_month` at the end), so the `max` is a
     no-op and the exact persisted value is used.
   - **Legacy snapshot (field absent → default 0):** `max(0, month_index(tick))`
     derives the correct resume month from the already-persisted `Tick`, so no
     replay storm ever occurs — including for spielstände created before this
     fix.

   The guard relies only on `month_index(tick)` never *exceeding* the true last
   processed month for a valid snapshot. That holds because the system never
   skips a crossed month boundary.

`LastProcessedMonth` lives in `crate::population`; `apply_into_world` already
takes `&mut World` and the world has `SimClock` + `LastProcessedMonth` installed
(PopulationPlugin runs before snapshot apply), so both resources are reachable.

### 2. Per-month age in the catch-up loop

Make the catch-up loop month-accurate so that, when it ever processes more than
one month (e.g. a future cadence change), each month uses the agent's age *as of
that month*, consistent with the month-keyed random draw.

Add a small, tested helper to `SimClock` (`backend/crates/sim-core/src/time/mod.rs`):

```rust
/// Absolute sim-seconds at the start of `month`.
pub fn month_start_seconds(&self, month: u64) -> u64 {
    month.saturating_mul(SECONDS_PER_MONTH)
}

/// Age in years at an absolute sim-second, for an agent born at `birth_tick`.
pub fn age_years_at(&self, at_sim_second: u64, birth_tick: u64) -> f32 {
    at_sim_second.saturating_sub(self.sim_seconds(birth_tick)) as f32
        / SECONDS_PER_YEAR as f32
}
```

In `population_monthly_system`, replace both
`clock.age_years(now_tick, birth_tick.0)` calls (mortality and fertility) with:

```rust
let age = clock.age_years_at(clock.month_start_seconds(m), birth_tick.0);
```

`now_tick` is still used to stamp `birth_tick` on newborns; only the
probability-threshold age changes.

### 3. Double-spawn guard (defense in depth)

In the fertility birth loop in `population_monthly_system`, skip a birth whose
deterministic `child_id` already exists in `AgentIdIndex`:

```rust
if world.resource::<AgentIdIndex>().0.contains_key(&child_id) {
    continue;
}
```

This prevents the orphaned-entity class entirely, independent of the persistence
fix. Keep the guard in the population module (the caller owns the birth
semantics), leaving `spawn_agent_from_record` unchanged.

## Accepted limitations (explicit, by design)

- **Newborns born mid-catch-up are not reconsidered within the same call.**
  `agent_entries` is collected once per call. In the frozen model the catch-up
  loop processes ≤ 1 month per tick, and newborns are age 0 with effectively
  zero mortality/fertility that month, so the gap has no observable effect.
  Documented here as a conscious boundary rather than fixed, to keep the change
  focused. Revisit only if true multi-month catch-up is ever introduced.

## Testing

All cargo via `scripts/cargo-serial.sh` (per CLAUDE.md). Pure backend change →
no browser smoke.

1. **Reload regression test (would have caught the bug).** Build a world, age it
   several sim-months (so agents age and `LastProcessedMonth` advances), record
   the population count, `extract_from_world` → serialize → deserialize →
   `apply_into_world` into a fresh world, run one tick. Assert: population count
   unchanged (no mass die-off/birth), `LastProcessedMonth == month_index(tick)`,
   no duplicate agent ids.
2. **Round-trip.** `LastProcessedMonth` survives
   extract→serialize→deserialize→apply with its exact value.
3. **Legacy snapshot.** Deserialize a JSON string without `last_processed_month`,
   apply, assert `LastProcessedMonth == month_index(tick)` and no replay.
4. **Per-month age helper.** Unit-test `month_start_seconds` and `age_years_at`
   (including birth after the queried month → 0 via saturating sub).
5. **Double-spawn guard.** Force the birth path to produce an already-existing
   child id; assert no duplicate entity and the index still maps to the original.
6. **Existing `population_lifecycle` and persistence round-trip tests stay green.**

## Affected files

- `backend/crates/sim-core/src/mobility/persist_snapshot.rs` — snapshot field,
  extract, restore guard.
- `backend/crates/sim-core/src/time/mod.rs` — `month_start_seconds`,
  `age_years_at` helpers + tests.
- `backend/crates/sim-core/src/population/mod.rs` — per-month age, double-spawn
  guard.
- Tests: helper unit tests inline in `time/mod.rs` and the double-spawn-guard
  test inline in `population/mod.rs` (`#[cfg(test)]`); reload-regression,
  round-trip, and legacy-snapshot tests in a new
  `backend/crates/sim-core/tests/population_persistence_reload.rs`.
