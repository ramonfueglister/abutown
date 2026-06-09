# Stationary-Age Seed — Design

**Date:** 2026-06-09
**Status:** implemented (reframed mid-implementation — see §0 Findings)
**Goal:** Seed abutopia's founding agents at the **stationary age distribution** implied by the mortality schedule (∝ survivorship `l(a)`), instead of the current uniform `0..90` — a realistic founding age pyramid that starts the population near its demographic equilibrium. Deterministic, replay-safe, mortality-only. Bounded already by the carrying capacity (PR #89, K=300) + floor-at-0 guards.

> **Note (reframe):** the original goal framing — "suppress the multi-generation population *wave*" — was **empirically refuted during implementation** once the carrying capacity (PR #89) was already in place. The population *level* is now governed by the regulator, not the seed; the value of this change is the realistic initial age *structure*, which is a correctness/realism refinement. See **§0 Findings** before reading the wave argument in §1.

---

## 0. Findings (empirical, 2026-06-09)

Measured in `population::tests` (a deterministic harness: per-agent Gompertz–Makeham mortality + ASFR fertility, monthly steps):

1. **Population-level overshoot is governed by the carrying capacity, not the seed.** With the live config (K=300, PR #89), uniform-0..90 and stationary seeds are population-indistinguishable: peak 347 vs 348, final 313 vs 314 over 60 yr. The only signal is timing (stationary settles ~6 yr earlier). With K=0 both simply grow (no damped wave settles in this per-agent model). The 300→428/463 overshoot that motivated this slice was observed **before** #89; the carrying capacity already solved it. The spec's original comparative-overshoot test therefore **cannot** assert "stationary peak < uniform peak" — it isn't true — so it was replaced (see §3).
2. **The age-structure improvement is real but modest, because this schedule's `l(a)` is itself near-flat.** Mortality is light (annual survival ≈ 95% even at age 90), so survivorship barely declines across 0–90 and the stationary distribution differs only mildly from uniform: TV distance of each seed to the theoretical `l(a)` is ≈0.0144 (stationary) vs ≈0.0227 (uniform) — a ~1.6× improvement, not a dramatic one. Over one generation the stationary seed's age structure relaxes ~11% less than uniform's (0.105 vs 0.118 TV), an advantage that fades by two generations as both converge (strong ergodicity, Coale 1972).

**Conclusion:** this slice is a small, principled correctness refinement — the founding age pyramid now matches the mortality schedule (∝ `l(a)`) instead of an unphysical flat 0–90 — not a behavioral fix for population dynamics. It ships because it is cheap, deterministic, literature-grounded, and strictly more correct; its effect size is intentionally modest and documented here so the framing is honest.

---

## 1. Grounding

The population-balance assessment (literature-grounded, 2026-06-08) found abutopia seeds 300 agents at a **uniform** age distribution `0..90` (`mobility/seed.rs::seeded_birth_tick_for_agent_id`: `stable_seed_hash(id) % (MAX_SEED_AGENT_AGE_YEARS+1)`, `MAX_SEED_AGENT_AGE_YEARS=90`). A uniform distribution is NOT the stable/stationary age structure for the model's Gompertz–Makeham mortality (`A=0.0001, B=0.00002, C=0.0866`, life-exp ≈ 70–80) + ASFR fertility (TFR=2.1). A non-stationary initial age pyramid produces **damped population waves**: a fertility pulse when the over-represented young/uniform cohort hits the 15–49 window, an echo a generation later, decaying over several generations (Lotka/Euler renewal; Leslie-matrix subdominant-eigenvalue transient; Keyfitz). Seeding at the stationary structure removes this transient: the population starts at the shape it would relax to.

**Stationary vs stable.** The exact stable age distribution is `∝ e^{−r a} l(a)` (Lotka). Here the intrinsic growth `r ≈ +0.15%/yr` (NRR=1.044) is negligible, so the **stationary** structure `∝ l(a)` (survivorship; the `r=0` case) is used — mortality-only, no eigenvalue/`r` computation, and the carrying capacity regulates the level regardless. (Full `e^{−r a} l(a)` is out of scope — §5.)

---

## 2. Design

### 2.1 Demographic helper — `population/mod.rs`
Add a pure function next to `mortality_hazard`:

```rust
/// Sample an integer age (years) from the STATIONARY age distribution implied by
/// the mortality schedule: P(age = a) ∝ l(a), the survivorship to age a, for
/// a ∈ 0..=MAX_SEED_AGE_YEARS. `u01` is a uniform draw in [0,1); the function
/// returns the inverse-CDF age (the smallest a with cumulative weight ≥ u01).
/// Mortality-only (stationary, r=0) — see the stationary-age-seed spec.
pub fn stationary_age_sample(u01: f64, c: &PopulationConfig) -> u32
```

- **Survivorship** `l(a)` via the existing `mortality_hazard`: `l(0)=1`; for `a≥1`, accumulate `l(a) = l(a−1) · (1 − annual_death_prob(a−1))`, where annual death prob from the Gompertz–Makeham hazard at the bucket. (Reuse `mortality_hazard`; integrate over the year. A monthly-compounded annual survival `(1 − death_probability_month(age))^{12}` is acceptable and consistent with the rest of the model — pick one and document it.)
- **Buckets:** ages `0..=MAX_SEED_AGE_YEARS` (a `population`-owned const = 90, matching today's cap; `seed.rs::MAX_SEED_AGENT_AGE_YEARS` is removed/redirected to it — single source of truth).
- **CDF + inverse:** weights `w(a)=l(a)`; `cdf(a)=Σ_{0..a} w / Σ_all w`; return the smallest `a` with `cdf(a) ≥ u01`. Clamp `u01` to `[0, 1)`; the final bucket catches `u01→1`.
- **Determinism:** pure `f64` arithmetic over fixed integer buckets and fixed default params → deterministic per build. Returns a `u32` age (no float leaks downstream).

### 2.2 Wire into the seed — `mobility/seed.rs`
`seeded_birth_tick_for_agent_id` keeps its signature `(agent_id, now_tick, clock)`:
- `let h = stable_seed_hash(&agent_id.0, SEED_AGENT_AGE_HASH_SALT);`
- `let u01 = u01_from_hash(h);` — a splitmix64 avalanche finalizer over the top 53 bits, **not** the naive `h / u64::MAX`. (See the correctness note below.)
- `let age_years = crate::population::stationary_age_sample(u01, &crate::population::PopulationConfig::default()) as u64;`
- then the existing `age_years → age_seconds → age_ticks → now_tick − age_ticks`.

**Correctness fix (u01 derivation).** The spec originally prescribed `u01 = h / (u64::MAX + 1)`. That is **broken** for this hash: `stable_seed_hash` is FNV-1a, which has weak avalanche in its HIGH bits for short, sequential ids (`agent:walk:0`, `1`, …). The old `% 91` read the well-mixed LOW bits, but `h / u64::MAX` reads the high bits — empirically clustering 290 of 300 agents into u01 ∈ [0.9, 1.0] (→ age 90) with **none** in the 15–49 fertile band, a *worse* seed than uniform. The fix runs `h` through a splitmix64 finalizer (`u01_from_hash`) so every bit is well-mixed before taking the top 53 bits as a uniform double. A `seed.rs` test guards this: a 300-agent cohort must span ≥40 distinct ages (anti-clustering), not just "include a fertile-age agent".

Uses `PopulationConfig::default()` because mortality is not world-authored (only `carrying_capacity` is, and that's applied after seeding). A comment notes: thread the real config here if mortality ever becomes authored. The 4 call sites + the snapshot-generation path are unchanged.

### 2.3 Determinism / replay
Seed-age is computed once at world creation and persisted in the mobility snapshot, so replay (`LastProcessedMonth` catch-up) and hydrate use the persisted `birth_tick` — unaffected. Fresh-seed reproducibility holds: same `agent_id` → same hash → same `u01` → same age (the CDF is fixed-input `f64`). No new RNG state.

---

## 3. Testing

- **Distribution (`population` unit tests — `stationary_age_sample_*`):** the age histogram matches `l(a)` — **not uniform**: strictly more young-skewed than the uniform baseline (young/old ratio compared against uniform, *not* a magic constant); monotone non-decreasing CDF; `u01=0.0 → age 0`; `u01→1.0 → MAX (90)`; deterministic; empirical mean age ≈ the stationary mean `Σ a·l(a) / Σ l(a)` within tolerance.
- **Anti-clustering (`seed` test — `seeded_birth_ticks_are_deterministic_and_cover_life_stages`):** a 300-agent cohort is deterministic, spans `0..=90`, includes fertile (15–49) and elder (≥70) agents, **and spans ≥40 distinct ages** — the guard that caught the FNV high-bit clustering (see §2.2).
- **Age-structure relaxation (the reframed goal — `population` integration test `stationary_seed_relaxes_less_than_uniform_over_a_generation`):** seed N agents two ways (uniform `0..90` vs `stationary_age_sample`) with a realistic 50/50 sex ratio, K=0 (so the *shape* relaxes freely — count normalized away), and assert the stationary seed's normalized age histogram shifts (total-variation) at least 5% **less** over one generation than uniform's — it starts nearer the equilibrium structure (strong ergodicity, Coale 1972). This replaces the original comparative-overshoot test, which §0 shows is not assertable. Boundedness under the carrying capacity is the existing `carrying_capacity_bounds_population_in_a_band` regression (seed-agnostic; the K=300 probe confirms the stationary seed peaks at 348 ≤ `K_hard`=375).
- **Determinism/replay:** existing seed/replay tests still pass; a seeded world's `birth_tick`s are deterministic (hash-driven, no RNG).
- **Full gate:** Rust fmt/clippy/test, frontend typecheck/vitest/build, e2e render-smoke.

---

## 4. Deploy (optional, post-merge)

The change only affects a **fresh** seed; the live abutopia is already seeded + bounded + healthy, so a redeploy is not required for correctness. Optionally redeploy to Fly + fresh-seed (`DELETE mobility_snapshots WHERE world_id='abutopia'`) to start the live world at the stationary age structure. No DB migration (no schema change).

---

## 5. Out of scope

- Full Lotka **stable** distribution `e^{−r a} l(a)` (the `r` skew is negligible; stationary suffices).
- Aggregate **cohort model** (Leslie matrix for warm/asleep chunks / 1M-scale unobserved population) — 8l slice 2, separate.
- World-authoring of mortality params; sex-specific survivorship; immigration.

---

## 6. References (APA7)

- Lotka, A. J. (1907). Relation between birth rates and death rates. *Science, 26*(653), 21–22. (Renewal equation; stable/stationary age distribution.)
- Leslie, P. H. (1945). On the use of matrices in certain population mathematics. *Biometrika, 33*(3), 183–212. (Projection matrix; transient = subdominant eigenvalues; convergence to the dominant eigenvector.)
- Keyfitz, N., & Caswell, H. (2005). *Applied mathematical demography* (3rd ed.). Springer. (Stationary/stable population theory; `l(a)` age structure.)
- Coale, A. J. (1972). *The growth and structure of human populations*. Princeton University Press. (Convergence to the stable age distribution; population momentum from non-stable starts.)
- Gompertz, B. (1825) & Makeham, W. M. (1860). Mortality hazard `μ(a)=A+B·e^{Ca}` (the deployed survivorship schedule).
