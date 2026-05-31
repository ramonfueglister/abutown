# Demographic persistence fix — design spec

**Date:** 2026-05-31
**Branch context:** `plan/persistence-liveness`
**Status:** implemented

## Problem

The monthly population system (`backend/crates/sim-core/src/population/mod.rs`,
`population_monthly_system`) replays the entire demographic history in a single
tick after every server restart, producing a mass die-off / baby-boom and
duplicate agent entities.

### Root cause

`LastProcessedMonth` (the resource that records which sim-month the population
system last advanced through) is **not part of the persisted snapshot**:

- `PopulationPlugin::install` inserts `LastProcessedMonth::default()` (= 0)
  every time the world is built.
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
   is "re-born" during replay; `spawn_agent_from_record` does a blank
   `AgentIdIndex.insert`, overwriting the index entry without despawning the
   previously-mapped entity → orphaned/duplicate entity.
3. **Non-idempotent reload** — the same snapshot loaded twice yields different
   populations, breaking the "Deterministic + replay-safe" promise in the
   module docstring.

### Secondary defect (in the catch-up loop itself)

- **Age uses `now_tick`, not the processed month `m`.** Mortality/fertility
  thresholds are computed from the agent's age at the current tick while the
  random draw is keyed to month `m`. For any multi-month span every month
  wrongly uses the agent's *final* age.

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
- The catch-up loop is age-correct for arbitrary month spans (even though, in
  the frozen model, it runs ≤ 1 month per tick in practice).
- Regression coverage that would have caught the original bug.

## Non-goals

- No offline / wall-clock catch-up of sim-time.
- No change to the salt/PRNG, Gompertz–Makeham, or ASFR math.
- No change to the frontend or the frontend↔backend boundary (pure backend) —
  no browser smoke required.
- **No legacy-snapshot compatibility.** `last_processed_month` is a required
  field; pre-fix snapshots without it are intentionally not supported (frozen
  worlds are regenerated; no `#[serde(default)]` shim).
- **No defensive fallback guards.** The fix removes the root cause; no
  belt-and-suspenders guards are added for states that can no longer occur
  (specifically, no double-spawn guard — a persisted cursor means each
  `(mother, month)` is processed exactly once, so the duplicate child id is
  unreachable).
- `PopulationConfig` is **not** persisted in this work (it is a deterministic
  default today). A future item if it ever becomes per-world tunable.

## Design

### 1. Own the cursor in `mobility::resources`, persist it, restore it directly

The persistence functions live in `mobility` and must read/write the cursor on
**every** persistable world — including the minimal worlds built by the snapshot
provider, the seed builders, and the sim-server in-memory runtimes, which call
`install_mobility` without `PopulationPlugin`. So the resource must be owned by
`mobility`, not `population`:

0. **Resource home.** Define `pub struct LastProcessedMonth(pub u64)` in
   `backend/crates/sim-core/src/mobility/resources.rs` (next to `Tick`) and
   insert it in `install_mobility`. Re-export it from `population` via
   `pub use crate::mobility::resources::LastProcessedMonth;` and drop the insert
   from `PopulationPlugin::install`. This also avoids a `mobility → population`
   dependency.
1. **Struct + wire format** (`persist_snapshot.rs`). Add
   `pub last_processed_month: u64` to `MobilityPersistSnapshot`, next to `tick`,
   and to the `WorldRepr` serialize and deserialize structs. The field is
   **required** on both sides (no serde default — legacy snapshots are not
   supported).
2. **Extract.** In `extract_from_world`, set
   `last_processed_month: world.resource::<crate::mobility::resources::LastProcessedMonth>().0`.
3. **Restore.** In `apply_into_world`, after restoring `Tick`, restore the
   cursor directly:

   ```rust
   world.resource_mut::<Tick>().0 = snap.tick;
   world
       .resource_mut::<crate::mobility::resources::LastProcessedMonth>()
       .0 = snap.last_processed_month;
   ```

   No guard, no derivation. The system's invariant is that after every tick
   `LastProcessedMonth == month_index(tick)` (it runs each tick and sets
   `LastProcessedMonth = current_month` at the end), and a snapshot is taken
   between ticks — so the persisted value is exactly the resume point.
   Frozen-time: sim-time resumes from the saved tick, there is no catch-up.

Both `Tick` and `LastProcessedMonth` are guaranteed installed before
`apply_into_world` runs — `install_mobility` inserts both (via
`empty_world_and_schedule` for tests, and `MobilityPlugin` before snapshot apply
in `sim-server/src/runtime/mod.rs`), matching how `apply_into_world` already
assumes `Tick`/`FlowCells` exist.

### 2. Per-month age in the catch-up loop

Make the catch-up loop month-accurate so each processed month uses the agent's
age *as of that month*, consistent with the month-keyed random draw.

Two pure helpers on `SimClock` (`backend/crates/sim-core/src/time/mod.rs`):

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

In `population_monthly_system`, both
`clock.age_years(now_tick, birth_tick.0)` calls (mortality and fertility) become:

```rust
let age = clock.age_years_at(clock.month_start_seconds(m), birth_tick.0);
```

`now_tick` is still used to stamp `birth_tick` on newborns; only the
probability-threshold age changes.

## Accepted limitation (explicit, by design)

Newborns born mid-catch-up are not reconsidered within the same call
(`agent_entries` is collected once). In the frozen model the loop processes
≤ 1 month per tick and newborns are age 0 with ~zero mortality/fertility that
month, so this has no observable effect. A conscious scope boundary, not a bug.

## Testing

All cargo via `scripts/cargo-serial.sh` (per CLAUDE.md). Pure backend change →
no browser smoke.

1. **Reload regression (would have caught the bug).** Age a world several
   sim-months, record the living set and `LastProcessedMonth`,
   `extract_from_world` → JSON → deserialize → `apply_into_world` into a fresh
   world, run one population tick. Assert: `LastProcessedMonth` restored to its
   pre-reload value (not 0) and the living-agent set is unchanged. Uses only the
   public API, so it compiles against the pre-fix code and fails there (replay
   storm) — true red→green.
2. **Value round-trip.** `last_processed_month` survives
   extract→serialize→deserialize with its exact value.
3. **Per-month age helpers.** Unit-test `month_start_seconds` and `age_years_at`
   (including birth after the queried instant → 0 via saturating sub).
4. **Per-month catch-up age.** A forced multi-month catch-up where the mother is
   past `fertile_max` at `now_tick` but fertile in the early processed months:
   she gives birth only when age is computed per-month. Red→green for the loop
   change.
5. **Existing `population_lifecycle`, `mobility_persistence_round_trip`, and the
   in-crate round-trip/determinism tests stay green** (new field round-trips, so
   equality holds).

## Affected files

- `backend/crates/sim-core/src/mobility/resources.rs` — `LastProcessedMonth`
  resource definition (moved here from `population`, next to `Tick`).
- `backend/crates/sim-core/src/mobility/api.rs` — `install_mobility` inserts
  `LastProcessedMonth`.
- `backend/crates/sim-core/src/time/mod.rs` — `month_start_seconds`,
  `age_years_at` helpers + tests.
- `backend/crates/sim-core/src/mobility/persist_snapshot.rs` — required snapshot
  field, extract, direct restore.
- `backend/crates/sim-core/src/population/mod.rs` — `pub use` re-export of the
  cursor, per-month age in the catch-up loop + per-month-age test
  (`PopulationPlugin` no longer inserts the cursor).
- `backend/crates/sim-core/tests/mobility_persistence_round_trip.rs` — the two
  snapshot fixtures gain the required field.
- `backend/crates/sim-core/tests/population_persistence_reload.rs` *(new)* —
  reload-regression + value round-trip tests.
