# Population Bounds — Floor-at-0 Guards + Carrying Capacity — Design

**Date:** 2026-06-08
**Status:** approved (brainstorming), pending spec review
**Goal:** Make the living abutopia population **bounded at both ends** so the server neither blanks out (lower) nor grows without limit (upper): (1) fix the persist + health guards to floor at 0 instead of the base-world seed count, so a legitimately-evolved population renders/persists; (2) add density-dependent fertility (carrying capacity) so the population self-regulates around a target and cannot grow unboundedly. After this ships and redeploys, the live public abutopia renders its current living population, and over long continuous runs the agent count stays in a bounded band.

---

## 1. Grounding (the bug + the demographic assessment)

A 4-agent assessment (model + spec + demographic theory, with citations) established:

- **The black screen is a guard bug, not a sim failure.** Two guards in `sim-server/src/app/mod.rs` key off `expected_base_world_agents` = the base-world **seed count** (300 = seed pedestrians + driver vehicles, `runtime/base_world_expectations.rs::expected_base_world_agent_count`):
  - **Persist guard** (`~:1352`): refuses to persist **and records a failure** when `mobility_world.agents.len() < expected_base_world_agents`.
  - **Health guard** (`~:536`): `health.ok = health.ok && view.mobility_full_dto.agents.len() >= expected_base_world_agents`.
  - The population-dynamics spec (`2026-05-29-population-dynamics-minimal-design.md` §125) explicitly says *"the agent count is no longer fixed"* — births/deaths make it dynamic. So below 300 the persist guard → persistence **Stale**, and the health guard → `health.ok=false` directly; the frontend `backendGate` then refuses the mobility WS → **black screen** (verified live on Fly at 285 agents, tick ~6000).
  - The guards' original (pre-demographics) intent was to avoid persisting/serving a **half-initialized** world. With the current architecture the runtime is fully seeded at construction before the persist loop runs, so `< 300` now only ever means demographic drift — the seed-count comparison is obsolete.

- **The population model itself is sound — mildly growing, not declining.** Computed from the deployed config (`mort_a=0.0001, mort_b=0.00002, mort_c=0.0866`; `tfr=2.1`, peak 28, σ 6, window 15–49; 50/50 child-sex draw): **NRR (net reproduction rate) = 1.044 > 1**, intrinsic `r ≈ +0.15%/sim-year`, mean generation `T ≈ 28 yr`. Stability is governed by **NRR=1**, not TFR=2 (Lotka/Euler renewal); TFR 2.1 with a 50/50 sex ratio and near-zero reproductive-age mortality lands slightly super-replacement.
- **The observed swings are transient + drift, not a calibration error.** The deterministic trend is *up* (~+0.46 agents/sim-yr at N=300); the remote's 300→285 is ~5.5 demographic-stochastic SDs over many sim-years (a random-walk dip), and the local 300→428/463 is the **population-wave overshoot** from seeding ages uniformly 0–90 (not the stable age distribution → echo cohorts; Leslie/renewal). Same rates, different points on the transient.
- **Unbounded is the real risk for a permanent server.** With NRR>1 and **no carrying capacity** (deferred per the spec), the mean population grows slowly but **without limit** — doubling ≈ every 460 sim-years, i.e. days of continuous wall-clock, but unbounded over an indefinitely-running public deploy. Agent count drives memory/CPU, so a permanent server needs an upper bound.

**Decisions (from brainstorming):** floor the guards at 0 (any non-empty world is valid); add a carrying capacity so the population is bounded above; document the balance finding (no fertility recalibration — the rates are sound).

---

## 2. Design

### 2.1 Floor-at-0 guards (`sim-server/src/app/mod.rs`)
Both guards drop the seed-count comparison and floor at 0:
- **Persist guard:** refuse to persist + `record_failure` only when `mobility_world.agents.len() == 0` (a genuinely empty/never-initialized runtime). Any population `> 0` persists. Update the error message accordingly.
- **Health guard:** `let runtime_agents_ok = !view.mobility_full_dto.agents.is_empty();` (i.e. `> 0`). `health.ok` stays gated on `runtime_agents_ok && persistence != Stale`.
- **Remove the now-dead `expected_base_world_agents`:** the `AppState` field, its computation in the constructor, `runtime::expected_base_world_agent_count`, and its test references — **iff** nothing else uses them. The seed helpers `expected_base_world_pedestrian_walks` / `expected_base_world_driver_vehicles` are KEPT if they are used by seeding/other code; only the agent-count guard plumbing is removed. (No-cruft: the guard defended an unreachable half-init state and false-positived on the reachable demographic one.)

### 2.2 Carrying capacity — density-dependent fertility (`sim-core/src/population/mod.rs`)
Add a configurable carrying capacity `K` and scale the per-female monthly birth probability by a density factor, so fertility is full well below `K` and suppressed as `N → K`:

- New `PopulationConfig` field `carrying_capacity: f32` (the regulation onset/target). Authorable per world (abutopia ⇒ its seed count, **300**); a sensible non-zero default. A value `<= 0` means **unbounded** (regulation disabled) for backward-compatible/test worlds.
- **Density factor** applied to `birth_probability_month(age)` in the fertility phase, using `N` = the live active-agent count at the month boundary:
  - `density_factor(N, K) = clamp((K_hard − N) / (K_hard − K), 0.0, 1.0)`, where `K_hard = K · capacity_overshoot` (a small band above `K`, e.g. `capacity_overshoot = 1.25`). Full fertility for `N ≤ K`; linear ramp 1→0 over `[K, K_hard]`; **zero births at/above `K_hard`** (a hard ceiling).
  - **Why not naïve `1 − N/K`:** because the base growth is tiny (NRR=1.044), a linear-from-zero suppression balances at only ~4% of `K` — it would *crash* the population to ~12. The ceiling form above keeps full fertility until `N` nears `K`, so the bounded equilibrium sits **just above `K`** (where the small reduction makes NRR=1) and is hard-capped at `K_hard`. This is the canonical density-regulation behavior (logistic/Verhulst; ceiling model of density dependence).
- **Determinism / replay-safety:** `N` is read at the start of each processed sim-month; the population system already processes months sequentially with the `LastProcessedMonth` cursor, updating `N` as agents die/spawn — so the factor is a pure function of the deterministic month-by-month state. No RNG-sequence state added. The per-agent birth draw (`unit_draw(hash, month, salt=1)`) is unchanged; only its acceptance threshold is scaled by `density_factor`.

This bounds the active population in roughly `(0, K_hard]`, self-regulating around `≈ K`. Combined with §2.1, the world can neither blank out (floor) nor explode (ceiling).

### 2.3 Population observability gauge (`sim-core/src/population/`)
A lightweight monthly log gauge, mirroring `economy::liveness`: once per processed sim-month, `tracing::info!(target: "population::liveness", month, n, births, deaths, "population month")`. Read-only, only when the clock/cursor is present. Makes the transient wave, drift band, and the carrying-capacity equilibrium observable in `fly logs` before any future rebalance.

---

## 3. Testing

- **Guard regression (`sim-server`):** a runtime/snapshot with `0 < agents < seed_count` (e.g. 285) **persists successfully and reports `health.ok = true`** (the live bug); a `0`-agent runtime is still refused/`record_failure` + `health.ok = false` (protection preserved).
- **Carrying-capacity bound (`sim-core`):** seed a population, run many sim-months (enough to pass the transient), assert the active count **stays within a bounded band** — never reaches `0` and never exceeds `K_hard` — and settles near `K` (e.g. within `[0.7·K, K_hard]` after the transient). A second case with `carrying_capacity <= 0` confirms regulation is disabled (unbounded, back-compat). Deterministic: same seed ⇒ identical trajectory.
- **Determinism/replay:** the existing per-month replay (`LastProcessedMonth` catch-up) yields the same population with the density factor applied (no divergence between continuous run and catch-up).
- **Full gate** (Rust fmt/clippy/test, frontend typecheck/vitest/build, e2e render-smoke) + the Fly image rebuilds.

---

## 4. Deploy

After merge, **redeploy to Fly** (the running machine has the old guards → still black). The new image: floor-at-0 guards → the current living population renders; carrying capacity → bounded going forward. Verify: `/health`=200 + `healthy`, the Vercel frontend renders the living world over `wss://`, and `population::liveness` logs a bounded `n`. No DB migration (no snapshot-schema change; `carrying_capacity` lives in the non-persisted `PopulationConfig`, re-applied from the authored world on each boot like `capita_baseline`).

---

## 5. Out of scope (deferred, per the population-dynamics spec §8l-2)

- Aggregate cohort / Leslie-matrix model for warm/asleep chunks (unobserved population at 1M scale).
- Seeding at the **stable age distribution** to suppress the multi-generation transient wave (we keep the intentional 0–90 hash seed + its initial death wave).
- Fertility/mortality recalibration (rates are demographically sound; NRR=1.044 is intentional mild growth, now bounded by the carrying capacity).
- Migration/immigration, sex-ratio realism (1.05), partnerships/households.

---

## 6. References (APA7)

- Verhulst, P.-F. (1838). Notice sur la loi que la population suit dans son accroissement. *Correspondance Mathématique et Physique, 10*, 113–121. (Logistic / carrying-capacity density regulation.)
- Lotka, A. J. (1907). Relation between birth rates and death rates. *Science, 26*(653), 21–22. (Renewal equation; stable population.)
- Leslie, P. H. (1945). On the use of matrices in certain population mathematics. *Biometrika, 33*(3), 183–212. (Age-structured projection; dominant eigenvalue λ = growth.)
- Keyfitz, N., & Caswell, H. (2005). *Applied mathematical demography* (3rd ed.). Springer. (NRR, replacement, stable population theory.)
- Lande, R. (1993). Risks of population extinction from demographic and environmental stochasticity. *The American Naturalist, 142*(6), 911–927. (Small-N demographic stochasticity; `log λs = r − ½σe² − (1/2N)σd²`; density regulation ⇒ exponential mean time to extinction.)
- Gompertz, B. (1825) & Makeham, W. M. (1860). Mortality hazard `μ(t)=A+B·e^{Ct}` (the deployed death model).
