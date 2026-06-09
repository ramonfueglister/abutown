# Stationary-Age Seed Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Seed founding agents at the stationary age distribution (∝ survivorship `l(a)`) instead of uniform `0..90`, suppressing the multi-generation population-wave transient.

**Architecture:** A pure `population::stationary_age_sample(u01, &cfg)` builds `l(a)` from the existing Gompertz–Makeham `mortality_hazard`, normalizes to a CDF, and inverse-samples. `mobility/seed.rs::seeded_birth_tick_for_agent_id` feeds it the existing per-agent hash (signature unchanged). Deterministic, replay-safe, mortality-only. Spec: `docs/superpowers/specs/2026-06-09-stationary-age-seed-design.md`.

**Tech Stack:** Rust, sim-core. Route ALL cargo through `scripts/cargo-serial.sh`; one cargo at a time; `pgrep -f cargo` first.

---

## File Structure
- Modify `backend/crates/sim-core/src/population/mod.rs` — `MAX_SEED_AGE_YEARS` const, `stationary_age_sample`, unit tests, `seed_ages` harness method, comparative test.
- Modify `backend/crates/sim-core/src/mobility/seed.rs` — wire `seeded_birth_tick_for_agent_id` to the sampler; remove the now-dead `MAX_SEED_AGENT_AGE_YEARS` const.

---

### Task 1: `stationary_age_sample` + `MAX_SEED_AGE_YEARS`

**Files:** Modify `backend/crates/sim-core/src/population/mod.rs`

- [ ] **Step 1: Write the failing unit tests** (in the `#[cfg(test)] mod tests`)

```rust
#[test]
fn stationary_age_sample_bounds_and_monotone() {
    let c = PopulationConfig::default();
    assert_eq!(stationary_age_sample(0.0, &c), 0, "u01=0 -> youngest");
    assert_eq!(stationary_age_sample(1.0, &c), MAX_SEED_AGE_YEARS, "u01->1 -> oldest bucket");
    let mut prev = 0u32;
    for k in 0..=100 {
        let u = (k as f64) / 100.0;
        let a = stationary_age_sample(u, &c);
        assert!(a >= prev, "inverse-CDF must be monotone non-decreasing in u01 (k={k})");
        prev = a;
    }
}

#[test]
fn stationary_age_sample_not_uniform_more_young_than_old() {
    // Stationary l(a) ~ 1 until ~50 then declines, so there is far more mass below 60
    // than at/above 70 — the opposite of a uniform 0..90 seed.
    let c = PopulationConfig::default();
    let n = 100_000usize;
    let (mut young, mut old) = (0u32, 0u32);
    for i in 0..n {
        let u = (i as f64 + 0.5) / n as f64;
        let a = stationary_age_sample(u, &c);
        if a < 60 { young += 1; } else if a >= 70 { old += 1; }
    }
    assert!(young > 5 * old, "stationary seed is young-skewed by survivorship; young={young} old={old}");
}

#[test]
fn stationary_age_sample_deterministic() {
    let c = PopulationConfig::default();
    assert_eq!(stationary_age_sample(0.37, &c), stationary_age_sample(0.37, &c));
}

#[test]
fn stationary_age_sample_empirical_mean_matches_l_a() {
    let c = PopulationConfig::default();
    let n = MAX_SEED_AGE_YEARS as usize;
    let mut l = vec![0.0f64; n + 1];
    l[0] = 1.0;
    for a in 1..=n {
        l[a] = l[a - 1] * (1.0 - death_probability_month((a - 1) as f32, &c) as f64).powi(12);
    }
    let total: f64 = l.iter().sum();
    let expected_mean: f64 = (0..=n).map(|a| a as f64 * l[a]).sum::<f64>() / total;
    let samples = 200_000usize;
    let emp: f64 = (0..samples)
        .map(|i| stationary_age_sample((i as f64 + 0.5) / samples as f64, &c) as f64)
        .sum::<f64>()
        / samples as f64;
    assert!((emp - expected_mean).abs() < 1.5, "empirical mean {emp} ≈ stationary mean {expected_mean}");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core population::tests::stationary_age_sample`
Expected: FAIL to compile — `MAX_SEED_AGE_YEARS` and `stationary_age_sample` don't exist.

- [ ] **Step 3: Add the const + function** (near `mortality_hazard`/`death_probability_month`, after line ~67)

```rust
/// Maximum seeded founding-agent age in years (single source of truth; `mobility::seed`
/// uses this via `stationary_age_sample`).
pub const MAX_SEED_AGE_YEARS: u32 = 90;

/// Sample an integer age (years, `0..=MAX_SEED_AGE_YEARS`) from the STATIONARY age
/// distribution implied by the mortality schedule: `P(age = a) ∝ l(a)`, the
/// survivorship to age `a`. `u01` ∈ `[0,1]` → inverse-CDF age (the smallest `a` whose
/// cumulative weight ≥ `u01`). Mortality-only (stationary, `r = 0`) — the model's
/// intrinsic growth is negligible and the carrying capacity sets the level. Pure and
/// deterministic.
pub fn stationary_age_sample(u01: f64, c: &PopulationConfig) -> u32 {
    let n = MAX_SEED_AGE_YEARS as usize;
    // Survivorship l(a): l(0)=1; l(a)=l(a-1)*annual_survival(a-1), where annual
    // survival = (1 - monthly death prob)^12 — consistent with the monthly model.
    let mut l = vec![0.0f64; n + 1];
    l[0] = 1.0;
    for a in 1..=n {
        let annual_survival = (1.0 - death_probability_month((a - 1) as f32, c) as f64).powi(12);
        l[a] = l[a - 1] * annual_survival;
    }
    let total: f64 = l.iter().sum();
    if !(total > 0.0) {
        return 0; // degenerate schedule — fall back to youngest
    }
    let threshold = u01.clamp(0.0, 1.0) * total;
    let mut acc = 0.0;
    for a in 0..=n {
        acc += l[a];
        if acc >= threshold {
            return a as u32;
        }
    }
    MAX_SEED_AGE_YEARS
}
```

(Note: `u01 = 1.0` gives `threshold = total`; floating accumulation reaches `total` only at the last bucket, returning `MAX_SEED_AGE_YEARS` — matching the test.)

- [ ] **Step 4: Run to verify they pass**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core population::tests::stationary_age_sample`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/population/mod.rs
git commit -m "feat(population): stationary_age_sample (inverse-CDF of survivorship l(a))"
```

---

### Task 2: Wire the seed to the stationary sampler

**Files:** Modify `backend/crates/sim-core/src/mobility/seed.rs`

- [ ] **Step 1: Write the failing test** (in `seed.rs`'s `#[cfg(test)] mod tests`)

```rust
#[test]
fn seeded_ages_are_stationary_distributed_and_deterministic() {
    use crate::ids::AgentId;
    let clock = crate::time::SimClock::default();
    let now: u64 = 10_000_000; // large enough that all seed ages are in the past
    // Determinism: same id -> same birth_tick.
    let a = seeded_birth_tick_for_agent_id(&AgentId("agent:walk:7".into()), now, &clock);
    let b = seeded_birth_tick_for_agent_id(&AgentId("agent:walk:7".into()), now, &clock);
    assert_eq!(a, b, "seed birth tick must be deterministic per id");
    // Distribution: over many ids, ages are young-skewed (stationary), NOT uniform.
    let ticks_per_year = (crate::time::SECONDS_PER_YEAR / clock.sim_seconds_per_tick.max(1)) as i64;
    let (mut young, mut old) = (0u32, 0u32);
    for i in 0..20_000u32 {
        let bt = seeded_birth_tick_for_agent_id(&AgentId(format!("agent:walk:{i}")), now, &clock);
        let age_years = ((now as i64 - bt) / ticks_per_year) as u32;
        if age_years < 60 { young += 1; } else if age_years >= 70 { old += 1; }
    }
    assert!(young > 5 * old, "seed ages young-skewed (stationary), not uniform; young={young} old={old}");
}
```

- [ ] **Step 2: Run to verify it fails** (current uniform seed → roughly equal young/old per decade, so `young > 5*old` is false)

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core seeded_ages_are_stationary`
Expected: FAIL on the distribution assertion (uniform seed isn't young-skewed).

- [ ] **Step 3: Rewire `seeded_birth_tick_for_agent_id` + remove the dead const**

Replace the body's age computation. The function (currently at `seed.rs:222`) becomes:
```rust
pub fn seeded_birth_tick_for_agent_id(
    agent_id: &AgentId,
    now_tick: u64,
    clock: &crate::time::SimClock,
) -> i64 {
    // Stationary-age seed: map the per-agent hash to u01, then draw an age from the
    // STATIONARY age distribution (∝ survivorship l(a)) of the DEFAULT mortality
    // schedule. Mortality is not world-authored (only carrying_capacity is); thread
    // the real PopulationConfig here if that ever changes.
    let h = stable_seed_hash(&agent_id.0, SEED_AGENT_AGE_HASH_SALT);
    let u01 = (h as f64) / (u64::MAX as f64 + 1.0);
    let age_years = crate::population::stationary_age_sample(
        u01,
        &crate::population::PopulationConfig::default(),
    ) as u64;
    let age_seconds = age_years.saturating_mul(crate::time::SECONDS_PER_YEAR);
    let age_ticks = age_seconds / clock.sim_seconds_per_tick.max(1);
    let age_ticks = i64::try_from(age_ticks).unwrap_or(i64::MAX);
    let now_tick = i64::try_from(now_tick).unwrap_or(i64::MAX);
    now_tick.saturating_sub(age_ticks)
}
```
Then remove the now-unused `const MAX_SEED_AGENT_AGE_YEARS: u64 = 90;` (at `seed.rs:210`). Grep first to confirm it has no other use: `git grep -n MAX_SEED_AGENT_AGE_YEARS -- backend/crates/sim-core/src/mobility/seed.rs` — if the only hit was the (now-rewritten) seed fn, delete the const. `stable_seed_hash` + `SEED_AGENT_AGE_HASH_SALT` are still used (keep them).

- [ ] **Step 4: Check the existing seed.rs tests** that call `seeded_birth_tick_for_agent_id` (around `seed.rs:698-706`). If any asserts a SPECIFIC age/birth_tick VALUE derived from the old uniform `% 91` mapping, update it to the new stationary value (or relax it to a property: deterministic + age in `0..=90`). Tests that only check determinism or range still pass.

- [ ] **Step 5: Run to verify the new test passes + the crate compiles**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core seeded_ages_are_stationary`
Then the seed module: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core seed`
Expected: PASS (new test + existing seed tests).

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/mobility/seed.rs
git commit -m "feat(mobility): seed founding ages from the stationary distribution"
```

---

### Task 3: Comparative-overshoot test (the goal: wave suppressed)

**Files:** Modify `backend/crates/sim-core/src/population/mod.rs` (extend `PopulationTestHarness` + add the test)

- [ ] **Step 1: Add a `seed_ages` harness method** (in the existing `PopulationTestHarness` impl, ~line 729)

Refactor so an explicit age list can be seeded. Add:
```rust
/// Spawn one agent per age in `ages` (all `Sex::Female`, fertile if in window),
/// mirroring `seed_agents`' component bundle.
fn seed_ages(&mut self, ages: &[u32]) {
    let owned: Vec<u32> = ages.to_vec();
    for age in owned {
        self.seed_one_agent(age); // extract the per-agent spawn from seed_agents into seed_one_agent(age: u32)
    }
}
```
Extract the per-agent spawn body of the existing `seed_agents` into `fn seed_one_agent(&mut self, age_years: u32)` (same `AgentMarker + StableAgentId + BirthTick + Sex::Female + AgentMobilityStateComponent + WalkPlan + WalkSpeed + Position` bundle + `AgentIdIndex` insert + `next_seed_id`), and have both `seed_agents` (cyclic ages) and `seed_ages` call it. This is a pure refactor — `seed_agents`' existing behavior is unchanged (verify the PR #89 carrying-capacity tests still pass).

- [ ] **Step 2: Write the comparative test**

```rust
#[test]
fn stationary_seed_suppresses_population_wave_vs_uniform() {
    // UNBOUNDED (carrying_capacity = 0) so the raw transient wave is visible (the
    // carrying capacity would otherwise clamp both at K_hard and mask the difference).
    let cfg = PopulationConfig { carrying_capacity: 0.0, ..PopulationConfig::default() };
    let n = 300usize;
    let cap = MAX_SEED_AGE_YEARS;
    // Uniform 0..=90 spread (today's seed shape) vs the stationary sampler.
    let uniform_ages: Vec<u32> = (0..n).map(|i| (i as u32 * (cap + 1)) / n as u32).collect();
    let stationary_ages: Vec<u32> =
        (0..n).map(|i| stationary_age_sample((i as f64 + 0.5) / n as f64, &cfg)).collect();

    let peak_over_window = |ages: &[u32]| -> usize {
        let mut h = PopulationTestHarness::new(cfg);
        h.seed_ages(ages);
        let mut max_n = h.active_agent_count();
        for _ in 0..(45 * 12) {
            h.advance_one_month();
            max_n = max_n.max(h.active_agent_count());
        }
        max_n
    };
    let uniform_peak = peak_over_window(&uniform_ages);
    let stationary_peak = peak_over_window(&stationary_ages);
    assert!(
        stationary_peak < uniform_peak,
        "stationary seed should suppress the wave (lower peak): stationary={stationary_peak} uniform={uniform_peak}"
    );
}
```

- [ ] **Step 3: Run; verify the direction holds (it is deterministic)**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core stationary_seed_suppresses`
Expected: PASS. The seeds are deterministic, so this is reproducible, not flaky. If the direction does NOT hold or the margin is zero, do NOT weaken it silently — STOP and report DONE_WITH_CONCERNS with both peak numbers and the window/N used, so the controller can reconsider (the literature predicts the uniform's young-heavy seed produces a larger synchronized birth pulse → higher peak; tune the window length `45*12` or `n` only to expose that, not to force a false pass).

- [ ] **Step 4: Commit**

```bash
git add backend/crates/sim-core/src/population/mod.rs
git commit -m "test(population): stationary seed suppresses the population-wave transient vs uniform"
```

---

### Task 4: Full gate

- [ ] **Step 1: Rust gate** (background slow ones; `pgrep -f cargo` first)
```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server
```
Expected: fmt clean; clippy 0 warnings; sim-core (incl. new tests + existing population/seed) + sim-server pass. (sim-server exercises `seeded_birth_tick_for_agent_id` via runtime seeding — confirm its seed/agent tests still pass with the stationary ages; update any sim-server test that asserted a specific old-uniform age value, mirroring Task 2 Step 4.)

- [ ] **Step 2: Frontend gate** (symlink `node_modules` if needed: `ln -sfn /Users/ramonfuglister/Coding/abutown/node_modules node_modules`)
```bash
npm run typecheck && npm test && npm run build
```
Expected: clean (no frontend change).

- [ ] **Step 3: e2e render-smoke**
```bash
CORS_ALLOWED_ORIGINS="http://127.0.0.1:5173" npm run test:e2e
```
Expected: render-smoke 2/2. NOTE: the smoke pins 300 backend-driven pedestrians at boot — seed ages changed but the COUNT (300) is unchanged, so it should hold. If the smoke asserts anything about agent ages, update it.

---

### Task 5: PR (finishing-a-development-branch)

- [ ] **Step 1:** Use **superpowers:finishing-a-development-branch** → push + PR against `main`.
- [ ] **Step 2:** PR body: stationary-age seed (∝ `l(a)`) replaces uniform 0–90, suppressing the population-wave transient; deterministic + replay-safe; reference the spec + the population-balance assessment. No DB migration (seed-time only; live world unaffected until a fresh seed).
- [ ] **Step 3:** Wait for ALL CI checks green, squash-merge, clean up the worktree + branch.

**Post-merge (OPTIONAL, controller-run — NOT a code task):** only affects a fresh seed; the live abutopia is already seeded + bounded. Optionally redeploy to Fly + fresh-seed (`DELETE mobility_snapshots WHERE world_id='abutopia'`) to start the live world at the stationary age structure.

---

## Self-Review

**Spec coverage:** §2.1 `stationary_age_sample` + `MAX_SEED_AGE_YEARS` → Task 1; §2.2 wiring + dead-const removal → Task 2; §2.3 determinism → asserted in Tasks 1+2; §3 distribution tests → Task 1, comparative-overshoot → Task 3, full gate → Task 4; §4 deploy → Task 5 post-merge note. All covered. The spec's "annual survival compounding" ambiguity is pinned in Task 1 Step 3 to `(1 - death_probability_month)^12`.

**Placeholder scan:** No TBD/TODO. `seed_one_agent` extraction is concrete (refactor the existing `seed_agents` body); test thresholds (`young > 5*old`, mean tol `1.5`, window `45*12`) are explicit. Task 2 Step 4 / Task 4 Step 1 flag the contingency (update tests asserting old uniform age values) with the concrete action.

**Type consistency:** `stationary_age_sample(u01: f64, c: &PopulationConfig) -> u32` and `MAX_SEED_AGE_YEARS: u32` used identically in Tasks 1, 2, 3. `seed_ages(&[u32])`/`seed_one_agent(u32)` consistent in Task 3. `seeded_birth_tick_for_agent_id` signature unchanged (verified against the 4 call sites).
