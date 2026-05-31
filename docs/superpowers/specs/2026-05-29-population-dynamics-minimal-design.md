# Population Dynamics — Minimal Birth/Death Design (Phase 8l, slice 1)

Date: 2026-05-29

## Status

Approved direction in brainstorming. This is the **first, minimal slice** of
**8l — Population dynamics** (the phase listed in
`2026-05-20-world-unification-foundation-design.md`). It is grounded in real
demographic literature (citations below) but deliberately scoped to the
**per-agent (individual) half only**. The aggregate cohort model and the
tracked-lineage layer are designed conceptually but **deferred to later slices**.

It builds on the merged **8i** (SimClock + derived age) and lands on a `main`
that already contains 8i and Codex's sidewalk/crosswalk work.

## Goal

Give agents a real, deterministic **birth and death** lifecycle: agents die by
an age-dependent mortality hazard, and women in the reproductive window give
birth to new age-0 agents. Demonstrable on **abutopia**: the pedestrian ages
(8i) and can die; a reproductive female bears children → the population grows and
shrinks over sim-time — a real, paper-grounded life cycle.

## Spec-conformance (verified against existing specs)

- **8l is the designated phase**; each phase is a bevy **`Plugin`** that
  registers its own components/events/resources/systems and never reaches into
  another plugin's internals → this ships a **`PopulationPlugin`**.
- 8i explicitly defers to 8l: "Births/death/life-stages/demographics = later
  phases", "death will use the existing `DeterministicRng`", "aggregate
  demographic age in flow = 8l". We honor exactly that boundary.
- Build on the **actually-implemented** 8i names: the durable per-agent field is
  **`birth_tick`** (+ `BirthTick` component); age comes from `SimClock`
  (`age_years(now_tick, birth_tick)` / `age_seconds`). (The 8i spec text said
  `birth_sim_time`; the implementation used `birth_tick` — 8l uses the real one.)
- 8i's calendar boundary events were **deferred** (no consumer then), so **8l
  brings its own sim-month cadence** — it is the first consumer.
- Deterministic + replay-able (8n): all stochastic draws are deterministic and
  reproducible.
  - **Implementation note (as built):** the shipped code does **not** draw from
    the mutable `DeterministicRng` stream. Each event uses a stateless hash,
    `unit_draw(stable_agent_hash(agent_id), sim_month, salt)` in
    `population/mod.rs` (salt `0` = death, `1` = birth, `2` = child sex). This is
    a deliberate improvement over a shared `StdRng`: a stateless hash is
    order-independent (immune to `AgentIdIndex`/HashMap iteration order and to
    population size), needs no RNG sequence-position state in the snapshot, and
    supports random access per `(agent, month)` for multi-month catch-up. A
    mutable global RNG would reintroduce exactly the reload-reset bug class fixed
    for `LastProcessedMonth` (see
    `2026-05-31-demographic-persistence-fix-design.md`). Do not "spec-align" the
    code back onto `DeterministicRng`. The references to `DeterministicRng` below
    describe the original intent (per-agent keyed independent draws), which
    `unit_draw` satisfies directly.

## Architecture (minimal slice)

### `PopulationPlugin`
Registers: the `Sex` component, mortality/fertility config resources, and a
**monthly cadence system**. Installed alongside the other plugins in the runtime
(and in the test world builder).

### Cadence — sim-month, not per tick
A system detects when the `SimClock` crosses a **sim-month boundary**
(Δt = 1/12 sim-year) and runs mortality + fertility once per crossing. Per-tick
evaluation is both wrong (Δt tiny) and wasteful; monthly is the standard
demographic-microsimulation step.

### Mortality (per active agent)
Age-dependent hazard, **Gompertz–Makeham**:

```
μ(age) = A + B · e^(C · age)
```

- `C ≈ ln(2)/8 ≈ 0.0866` per year (mortality roughly doubles every 8 years —
  the well-known regularity).
- `A` is the age-independent (Makeham) background term; `B` the baseline level.
- Discrete-time death probability over the month:
  `q = 1 − e^(−μ(age) · Δt)` (piecewise-exponential, constant rate per interval).
- Each active agent each month draws `DeterministicRng` (stream keyed by
  `agent_id` + sim-month index); `r < q` → the agent **dies** (despawn).

### Fertility (per active female in the reproductive window)
- New **`Sex`** component (`Male`/`Female`), assigned at birth ~50/50 via
  `DeterministicRng`; seed/founding agents get a deterministic split.
- Age-specific fertility rate `f(age)` from a simple parametric ASFR curve
  (Schmertmann quadratic-spline or a gamma shape), peaking ~28–30, **scaled so
  its integral over the reproductive window equals the target TFR**.
- Reproductive window (e.g., 15–49).
- Monthly birth probability `p = f(age) · Δt`; `r < p` → **birth**:
  `spawn_agent_from_record(AgentRecord::new_born_at(child_id, …, now_tick))` with
  a new `parent_id = mother`, random `Sex`. **No partnership required** (single-
  parent birth — partnerships are out of scope this slice).

### Calibration (config resource)
- target life expectancy → `(A, B, C)` (or load from a life table);
- target TFR → ASFR scale; reproductive window; cadence length.
Exposed as a config resource so the numbers are tunable, not hardcoded.

## What this is NOT (deferred slices, designed but not built here)

- **Aggregate cohort model (cohort-component / Leslie matrix)** for warm/asleep
  chunks. This minimal slice simulates **only active agents**, so it is
  **complete for abutopia** (a single always-active chunk) but **under-simulates
  unobserved population at Zurich/1M scale**. The aggregate model is 8l slice 2.
  *(Leslie, 1945.)*
- **Tracked-lineage layer** (player dynasties / notable figures kept as
  persistent individuals across LOD, importance scoring, culling — the
  Crusader-Kings / Dwarf-Fortress "historical figures" pattern). Designed in the
  brainstorm; a later additive layer that changes the demote/promote rules.
- Sex-specific mortality differential, old-age logistic deceleration
  (Thatcher–Kannisto), partnerships/households, migration, cause-specific
  mortality, economy/health hazard modifiers.

## Testing

- **Deterministic mortality:** an agent of a given age has the exact `q`; a fixed
  seed reproduces the same death/no-death across runs and replays.
- **Calibration sanity:** with target life expectancy E, a synthetic cohort's
  simulated mean age at death ≈ E (within tolerance); with target TFR, mean
  completed births per female ≈ TFR.
- **Birth wiring:** a reproductive female eventually bears a child with
  `age == 0`, `parent_id == mother`, a `Sex`, and a fresh `birth_tick == now`.
- **abutopia lifecycle:** over enough sim-months the population changes (births
  and/or deaths occur); the agent count is no longer fixed at 1.
- **Persistence:** `Sex` and `parent_id` survive snapshot round-trip.
- Cadence: mortality/fertility run exactly once per sim-month crossing (not per
  tick).

## References (APA7)

- Gompertz, B. (1825). On the nature of the function expressive of the law of
  human mortality. *Philosophical Transactions of the Royal Society of London,
  115*, 513–583.
- Makeham, W. M. (1860). On the law of mortality and the construction of annuity
  tables. *Journal of the Institute of Actuaries, 8*(6), 301–310.
- Heligman, L., & Pollard, J. H. (1980). The age pattern of mortality. *Journal
  of the Institute of Actuaries, 107*(1), 49–80.
- Thatcher, A. R., Kannisto, V., & Vaupel, J. W. (1998). *The force of mortality
  at ages 80 to 120*. Odense University Press.
- Schmertmann, C. P. (2003). A system of model fertility schedules with
  graphically intuitive parameters. *Demographic Research, 9*(5), 81–110.
- Peristera, P., & Kostaki, A. (2007). Modeling fertility in modern populations.
  *Demographic Research, 16*(6), 141–194.
- Leslie, P. H. (1945). On the use of matrices in certain population mathematics.
  *Biometrika, 33*(3), 183–212.
- Willekens, F. (2009). Continuous-time microsimulation in longitudinal analysis.
  In A. Zaidi, A. Harding, & P. Williamson (Eds.), *New frontiers in
  microsimulation modelling* (pp. 413–436). Ashgate.

## Open questions (resolve during planning, against real code)

1. The exact ASFR functional form for slice 1 (a 3–4-parameter gamma vs a
   Schmertmann quadratic spline) — pick the simplest that calibrates to TFR.
2. Where the monthly cadence system slots into the schedule relative to the
   mobility/aging systems, and how it reads the `SimClock` month index.
3. Default `(A, B, C)` + TFR values for abutopia (a plausible pre-industrial vs
   modern life table) — a tuning choice, documented as config.
4. Confirm `spawn_agent_from_record` + the seed give children a valid initial
   position/plan (a newborn needs somewhere to be) without a full activity plan.
5. `Sex` + `parent_id` persistence: add to `AgentRecord` (serde-default) so they
   round-trip like `birth_tick`.
