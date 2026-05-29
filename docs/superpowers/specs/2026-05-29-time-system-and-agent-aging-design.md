# Time System + Agent Aging Design (Phase 8i)

Date: 2026-05-29

## Status

Approved direction in brainstorming. This is the **8i — Time + calendar** phase
listed in `2026-05-20-world-unification-foundation-design.md`. It is the first
real consumer-driver of a simulation clock, and it includes a **minimal agent
aging** slice at the user's request. It builds on `abutopia` (now the only
world) and the merged `mobility/systems/*` split.

## Goal

Give the simulation a real, deterministic notion of time, a player-controllable
speed, a derived calendar, and make **every (active) agent age** as simulation
time passes — without breaking pedestrian movement at high speed.

Two headline requirements from the user:
1. A real time system. The aspiration "~2000 sim-years over 1 real year" is the
   *top speed tier*, **tunable, not hardwired** (the user is relaxed about the
   exact number). Compression factor at that tier ≈ 2000× (1 real second ≈ 2000
   sim-seconds ≈ 33 sim-minutes).
2. **All agents age.** No life stages, no death yet — just an age that grows
   with sim-time, simulated for every agent.

## Key design insight (why this resolves the macro/micro tension)

SimCity / Cities: Skylines run **two decoupled, intentionally-inconsistent
clocks** (agents in ~real-time, calendar fudged faster). We do better by
exploiting what abutown already has: **couple the time-scale to the existing
LOD system**.

- **Slow / "watch" speed → chunks Active/Hot → individual pedestrians** tick and
  move in human-watchable time (existing frame interpolation smooths it).
- **Fast / "epoch" speed → chunks Warm/Asleep → movement becomes the existing
  population-flow abstraction.** Pedestrians never teleport; they *degrade to
  flow* at high compression and re-materialize when you slow down/zoom in.

This is mandated, not just nice: the v2 invariant already says a waking chunk's
state "must be explainable from prior state plus accepted events plus **elapsed
simulation time**." The SimClock is the home of "elapsed simulation time."

**Aging is free.** Age is a *derived* property (`age = sim_now − birth_sim_time`),
zero per-tick compute, so "every agent ages" costs only a birth-stamp per agent
— it scales to the million-agent target. The expensive thing is movement, not
aging.

## Spec-conformance constraints (from existing specs — must honor)

- **Implement as a bevy `Plugin`** (`TimePlugin`) registering resources/systems/
  events against the public API of `CorePlugin`/`MobilityPlugin`, **without
  modifying foundation code** (foundation correctness criterion).
- **Deterministic, tick-derived** — no wall-clock on the sim side; required by
  the planned 8n determinism+replay phase and the v2 "resumes deterministically"
  invariant. Speed changes are **commands/events** (so replay reproduces them).
- **Build on existing primitives:** `TickClock { tick, version, pulse_sequence }`
  (`world/resources.rs`), `DeterministicRng` (seeded from world id),
  `tick_period_ms` already in `WorldSummaryDto`, and the existing frame
  interpolation. Do not duplicate the tick counter.
- **Scope = clock + calendar + speed + minimal aging.** Births/death-rates,
  demographic age in flow, generations, migration, weather/economy coupling stay
  in their own later phases (8h economy, 8l population dynamics, 8m weather).

## Architecture

### 1. SimClock (resource, derived from `TickClock`)

A new resource that turns ticks into sim-time:

- `elapsed_sim_seconds: u64` — the canonical sim-time. Each tick adds
  `sim_seconds_per_tick(current_speed)`. Accumulated (not `tick × const`) so a
  varying speed still yields a monotonic, deterministic sim-time.
- Derived calendar accessors: `year / month / day / hour / minute` from
  `elapsed_sim_seconds` (fixed epoch start, e.g. year 0 day 0). Simple,
  pure functions — no stored calendar state to drift.

### 2. TimeScale / speed (resource, command-driven)

- Discrete speed tiers (tunable), e.g.: `Paused · Watch(1×) · Fast · Faster ·
  Epoch`. Each maps to a `sim_seconds_per_tick`.
- A `SetSpeed` **command/event** changes it (deterministic + replayable).
- One **LOD-coupling threshold**: at/above tier *Faster*, individual mobility
  systems yield to flow (chunks treated warm) — encoded as a single predicate
  the mobility schedule and the LOD scheduler both read. (Exact wiring to the
  current LOD states verified during planning — see Open Questions.)

### 3. Calendar boundary events

When the derived calendar crosses a day/month/year boundary during a tick, emit
`SimDayElapsed { day }` / `SimMonthElapsed` / `SimYearElapsed` events. Future
phases (economy, weather, population) subscribe; **8i emits, does not consume**
(except aging, below).

### 4. Agent aging (the minimal slice)

- New component `BirthSimTime(u64)` — the `elapsed_sim_seconds` at spawn. Set in
  `spawn_agent_from_record`; persisted via a new `AgentRecord.birth_sim_time`
  field (default 0 / "born at epoch" for legacy records).
- **Age is derived on demand:** `age_seconds = clock.elapsed_sim_seconds −
  birth_sim_time`; expose `age_years: f32` in `AgentMobilityDto`.
- **No life stages, no death** in 8i. No sprite/behavior change by age.
- All **active/hot** agents age (it's derived, so automatically). Warm/flow
  chunks do not carry per-individual age — aggregate cohort aging is **8l**,
  documented here as the deliberate 8i boundary. (For abutopia — a single
  always-active chunk — this means literally every agent ages.)

### 5. Frontend

- Show the calendar (year/season/day or a compact date) and a **speed control**
  (the tiers) in the UI.
- The entity inspector shows an agent's **age** (already has an inspector panel).
- Reuse existing `tick_period_ms` + interpolation; the calendar/speed come over
  the wire in `WorldSummaryDto` (extend it with `sim_time` + `speed`).

## Speed tiers (proposal — numbers tunable)

| Tier | sim-time / real-time | feel | movement LOD |
|---|---|---|---|
| Paused | 0 | frozen | — |
| Watch (1×) | ~60× (1 real s ≈ 1 sim-min; a sim-day in ~24 real-min) | watch pedestrians | individuals |
| Fast | ~600× | hours fly | individuals |
| Faster | ~7 200× (a sim-day in ~12 real-s) | days fly | flow |
| Epoch | ~2 000 sim-years / real-year (≈ the user's aspiration) | civilizations | flow |

(At 10 Hz, `sim_seconds_per_tick = (sim/real) / 10`. Exact tiers tuned during
implementation so movement stays smooth up to the flow threshold.)

## Testing

- **Deterministic clock:** N ticks at a fixed speed ⇒ exact `elapsed_sim_seconds`
  and calendar; same seed/speed-history ⇒ identical clock (replay-friendly).
- **Calendar boundaries:** crossing day/month/year emits exactly one event each.
- **Speed change is an event** and reproduces under replay.
- **Aging:** an agent spawned at tick T has `age == 0`; after K sim-seconds its
  derived `age_years` matches; two agents born at different sim-times have the
  expected age difference. Abutopia: the pedestrian's age grows visibly at Epoch
  speed.
- **LOD coupling:** above the threshold, the individual-mobility predicate is
  false (movement handed to flow); below, individuals tick. (Asserted against
  the real LOD states confirmed in planning.)

## Out of scope (later phases)

- Life stages, death/lifespan, births/natality, replacement — 8l population
  dynamics (death uses the existing `DeterministicRng` when it lands).
- Aggregate demographic age in warm/flow chunks — 8l.
- Seasons/weather effects — 8m. Economy cadence — 8h.
- Determinism/replay *verification harness* — 8n (this phase only keeps the
  clock replay-*able*).
- Round-trip / cyclic pedestrian plans — separate movement feature (was paused;
  will consume this clock later).

## Open questions (resolve during planning, against real code)

1. Confirm the **real LOD states** in the current code (`Asleep/Warm/Active/Hot`
   per spec vs. the `active/warm/cold` the assessment saw) and wire the
   time-scale↔LOD threshold to whatever actually exists.
2. Confirm where the authoritative tick advances (`runtime.tick` vs the mobility
   `Tick`/`TickClock` resource) and make the SimClock advance in exactly one
   place per tick.
3. Decide `elapsed_sim_seconds` integer unit (seconds vs a finer fixed-point) so
   that low tiers don't lose sub-second movement resolution while high tiers
   don't overflow over 2000 sim-years (2000 yr ≈ 6.3e10 s — fits u64 easily).
4. Wire format: extend `WorldSummaryDto` with `sim_time` + `speed`, and
   `AgentMobilityDto` with `age_years` (Protobuf change → regenerate).
