# Time System + Agent Aging Design (Phase 8i)

Date: 2026-05-29

## Status

Approved direction in brainstorming. This is the **8i — Time + calendar** phase
from `2026-05-20-world-unification-foundation-design.md`. It adds a real
simulation clock and a **minimal agent-aging** slice. Corrected to match the
established game model in `2026-05-14-abutown-simulation-architecture-v2-design.md`.

## Game model (from architecture-v2 — drives this design)

> "Abutown is **one persistent, always-on browser game world** … aquarium-first:
> **players observe the same authoritative world, pan, zoom, and later submit
> validated indirect actions** … ~2000 connected players, 1M+ durable entities."

Consequences that shape the time system:
- **One server-authoritative global clock.** Same time for every observer.
- **No player time control.** No speed slider, no pause-for-one-player, no
  per-client time. (It's an MMO; you cannot fast-forward a shared world.)
- **LOD is interest-driven, not time-driven.** Chunk states
  `asleep/warm/active/hot` follow *player proximity / viewport*, per v2 — they
  are **not** coupled to any time-speed setting (this corrects the earlier draft).

## Goal

Give the one shared world a deterministic, server-authoritative clock that runs
at a single fixed rate, exposes a calendar, and makes **every agent age** as
simulation time passes.

The earlier "~2000 sim-years / real-year" is an **aspiration, not a fixed
constant** (the user is relaxed about the number). The clock rate is a **tunable
server config**, chosen so observed agents still read as a lively (slightly
time-lapsed) aquarium while the calendar and aging progress at a satisfying
civilization scale over the world's lifetime. No 2000×-hardwiring.

## Key insight — aging is a free, derived property

`age = global_sim_time − agent.birth_sim_time`. Zero per-tick compute, so "every
agent ages" costs only a durable birth-stamp per agent — it scales to 1M.
Because `birth_sim_time` is **durable** and the clock is global, an agent's age
is correct the instant it is observed, even after its chunk was `asleep` for a
long time (the v2 "waking state = prior state + events + **elapsed simulation
time**" invariant covers exactly this). So aging needs **no** per-tick work and
**no** aggregate-cohort modelling for individual correctness.

## Spec-conformance constraints (must honor)

- **Implement as a bevy `Plugin`** (`TimePlugin`) registering resources/systems/
  events against the public API of `CorePlugin`/`MobilityPlugin`, **without
  modifying foundation code** (foundation correctness criterion).
- **Deterministic, tick-derived** clock — no wall-clock on the sim side
  (required by planned 8n replay + the v2 "resumes deterministically" invariant).
- **Build on existing primitives:** `TickClock { tick, version, pulse_sequence }`
  (`world/resources.rs`), `DeterministicRng` (seeded from world id),
  `tick_period_ms` already in `WorldSummaryDto`, the existing frame
  interpolation, and the existing interest-driven LOD. Do not duplicate the tick
  counter; do not touch the LOD trigger logic.
- **Scope = clock + calendar + minimal aging.** No speed control (N/A for an
  MMO). Births/death/life-stages/demographics/weather/economy = later phases.

## Architecture

### 1. SimClock (resource, derived from `TickClock`, single fixed rate)

- The server advances `TickClock.tick` at the existing fixed tick rate (~10 Hz).
- `SimClock` maps ticks → sim-time: `elapsed_sim_seconds = tick × SIM_SECONDS_PER_TICK`
  where `SIM_SECONDS_PER_TICK` is the one **fixed, tunable** rate constant
  (server config). Linear in tick (no varying speed) → trivially deterministic
  and replayable; `elapsed_sim_seconds` is exactly recoverable from tick alone.
- Derived calendar accessors (`year/month/day/hour/minute`) as pure functions of
  `elapsed_sim_seconds` from a fixed epoch — no stored calendar state to drift.
- `u64` seconds is ample (e.g. 2000 sim-years ≈ 6.3e10 s ≪ u64).

### 2. Calendar boundary events

When a tick crosses a day/month/year boundary, emit `SimDayElapsed { day }` /
`SimMonthElapsed` / `SimYearElapsed`. Later phases (economy 8h, weather 8m,
population 8l) subscribe. 8i **emits**; the only in-phase consumer is aging
(which doesn't even need the events — it derives age directly).

### 3. Agent aging (the minimal slice)

- `AgentRecord` gains `birth_sim_time: u64` (durable; legacy/seed records default
  to 0 = "born at epoch"). Set in `spawn_agent_from_record` to the current
  `elapsed_sim_seconds`.
- A `BirthSimTime(u64)` component carries it on the live entity.
- **Age is derived on demand:** `age_seconds = clock.elapsed_sim_seconds −
  birth_sim_time`; expose `age_years: f32` in `AgentMobilityDto`.
- **Every agent ages** — active, hot, and (correctly, on observe) formerly
  `asleep`/`warm` ones, because age is derived from the global clock + durable
  birth stamp. No per-tick aging system is required.
- **No life stages, no death, no sprite/behaviour change by age** (those are 8l).

### 4. LOD — unchanged

The interest-driven `asleep/warm/active/hot` machinery is **not** modified by
this phase. Movement fidelity per chunk stays exactly as today. The clock only
*provides* `elapsed_sim_seconds` that the existing catch-up logic may read when a
chunk wakes.

### 5. Frontend

- Show the world calendar (a compact date / year) somewhere unobtrusive — it is
  the same for all observers, read from the wire.
- The entity inspector shows an agent's **age** (panel already exists).
- Extend `WorldSummaryDto` with `sim_time` (and the fixed rate, for the client to
  display/advance the clock between snapshots), and `AgentMobilityDto` with
  `age_years` (Protobuf change → regenerate). No speed UI.

## Testing

- **Deterministic clock:** tick N ⇒ exact `elapsed_sim_seconds` and calendar;
  pure function of tick (replay-safe).
- **Calendar boundaries:** crossing day/month/year emits exactly one event each.
- **Aging:** agent spawned at sim-time T has `age == 0`; after K sim-seconds its
  `age_years` matches; two agents born at different sim-times differ correctly.
- **Aging survives dormancy:** an agent whose chunk slept for a long sim-interval
  reports the correct (older) age when re-observed (derive-from-global-clock).
- **No speed surface:** assert there is no client-settable speed (guards against
  re-introducing a switch).

## Out of scope (later phases)

- Any player time control / speed / pause — not part of this MMO.
- Life stages, death/lifespan, births/natality, replacement — 8l (death will use
  the existing `DeterministicRng`).
- Aggregate demographic age distributions in warm/flow chunks — 8l. (Individual
  age is already correct via derivation; this is only about *population
  statistics* for unobserved regions.)
- Seasons/weather — 8m. Economy cadence — 8h. Replay *verification harness* — 8n.
- Round-trip / cyclic pedestrian plans — separate movement feature (parked; will
  read this clock later).

## Open questions (resolve during planning, against real code)

1. Pick `SIM_SECONDS_PER_TICK` (the one fixed rate) and where it's configured
   (server config/env). Tune so observed agents look like a lively time-lapse
   and the calendar/aging progress meaningfully over the world's lifetime.
2. Confirm the single place per tick where `TickClock.tick` is the authority
   (`runtime.tick` ↔ mobility `Tick`/`TickClock`) and derive `SimClock` from it
   without adding a second counter.
3. Wire format: extend `WorldSummaryDto` (`sim_time`, rate) and
   `AgentMobilityDto` (`age_years`); regenerate Protobuf.
4. Confirm `birth_sim_time` flows through persistence (snapshot write/read) so
   ages survive restart/hydration.
