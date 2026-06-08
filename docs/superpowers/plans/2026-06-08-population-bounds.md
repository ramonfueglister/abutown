# Population Bounds Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bound the living abutopia population at both ends — floor-at-0 persist/health guards (so a demographic population renders/persists) and density-dependent fertility (so it self-regulates around a carrying capacity and can't grow unbounded).

**Architecture:** Two crates. In `sim-server` the two `expected_base_world_agents` guards become floor-at-0 and the dead field is removed. In `sim-core` `PopulationConfig` gains a `carrying_capacity` (+ `capacity_overshoot`); a `fertility_density_factor(N,K)` scales birth acceptance in the monthly population system; the `sim-server` runtime sets `carrying_capacity` = the base-world seed count (reusing `expected_base_world_agent_count`) on both construct paths. A `population::liveness` monthly log gauge adds observability.

**Tech Stack:** Rust, bevy_ecs, tracing. Route ALL cargo through `scripts/cargo-serial.sh` (run slow ones in background). Spec: `docs/superpowers/specs/2026-06-08-population-bounds-design.md`.

---

## File Structure
- Modify `backend/crates/sim-core/src/population/mod.rs` — `PopulationConfig` fields, `fertility_density_factor`, apply in `population_monthly_system`, the gauge, unit + integration tests.
- Modify `backend/crates/sim-server/src/app/mod.rs` — floor-at-0 guards; remove the `expected_base_world_agents` field + plumbing.
- Modify `backend/crates/sim-server/src/app/tests.rs` — drop the two `expected_base_world_agents:` struct fields; add the floor-at-0 regression.
- Modify `backend/crates/sim-server/src/runtime/mod.rs` — set `PopulationConfig.carrying_capacity` from `expected_base_world_agent_count(base_world)` after both `PopulationPlugin.install` sites.
- Keep `backend/crates/sim-server/src/runtime/base_world_expectations.rs::expected_base_world_agent_count` (repurposed as the carrying-capacity source).

---

### Task 1: `fertility_density_factor` + `PopulationConfig` carrying capacity

**Files:** Modify `backend/crates/sim-core/src/population/mod.rs`

- [ ] **Step 1: Write failing unit tests** (add to the `#[cfg(test)] mod tests` in `population/mod.rs`)

```rust
#[test]
fn density_factor_unbounded_when_capacity_non_positive() {
    let c = PopulationConfig { carrying_capacity: 0.0, ..PopulationConfig::default() };
    assert_eq!(fertility_density_factor(0, &c), 1.0);
    assert_eq!(fertility_density_factor(100_000, &c), 1.0);
}

#[test]
fn density_factor_full_below_k_zero_at_hard_ceiling() {
    // K=100, overshoot 1.25 => K_hard=125.
    let c = PopulationConfig { carrying_capacity: 100.0, capacity_overshoot: 1.25, ..PopulationConfig::default() };
    assert_eq!(fertility_density_factor(50, &c), 1.0, "full fertility well below K");
    assert_eq!(fertility_density_factor(100, &c), 1.0, "full fertility at K");
    let mid = fertility_density_factor(112, &c); // ~halfway through [100,125]
    assert!(mid > 0.4 && mid < 0.6, "linear ramp in the band, got {mid}");
    assert_eq!(fertility_density_factor(125, &c), 0.0, "zero at K_hard");
    assert_eq!(fertility_density_factor(200, &c), 0.0, "zero above K_hard");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core population::tests::density_factor`
Expected: FAIL to compile — `carrying_capacity`/`capacity_overshoot` fields and `fertility_density_factor` don't exist.

- [ ] **Step 3: Add the config fields + the factor function**

In `PopulationConfig` (struct at `population/mod.rs:70`) add two fields after `fertile_max`:
```rust
    /// Active-population carrying capacity K. Fertility is full at/below K and
    /// ramps to zero across [K, K*capacity_overshoot]. `<= 0.0` disables
    /// regulation (unbounded growth). Set per-world by the runtime.
    pub carrying_capacity: f32,
    /// Upper band as a multiple of K: hard fertility ceiling K_hard = K*overshoot.
    pub capacity_overshoot: f32,
```
In `impl Default for PopulationConfig` (at `:80`) add (after `fertile_max: 49.0,`):
```rust
            carrying_capacity: 0.0, // unbounded by default; the runtime sets it per-world
            capacity_overshoot: 1.25,
```
Add the factor function (near `birth_probability_month`, after `:65`):
```rust
/// Density-dependent fertility multiplier in `[0,1]`. Full fertility (1.0) while
/// the active population `n` is at or below the carrying capacity `K`; linear ramp
/// 1→0 across `[K, K_hard]` where `K_hard = K * capacity_overshoot`; 0 at/above
/// `K_hard`. `K <= 0` disables regulation (returns 1.0 — unbounded).
///
/// NOTE: deliberately NOT `1 - n/K`. The base schedule is only mildly
/// super-replacement (NRR≈1.044), so a linear-from-zero suppression would balance
/// at ~4% of K and collapse the population; the ceiling form keeps full fertility
/// until `n` nears `K`, so the bounded equilibrium sits just above `K`.
pub fn fertility_density_factor(n: usize, c: &PopulationConfig) -> f32 {
    let k = c.carrying_capacity;
    if k <= 0.0 {
        return 1.0;
    }
    let k_hard = k * c.capacity_overshoot.max(1.0);
    let n = n as f32;
    ((k_hard - n) / (k_hard - k)).clamp(0.0, 1.0)
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core population::tests::density_factor`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/population/mod.rs
git commit -m "feat(population): carrying-capacity config + fertility_density_factor (ceiling form)"
```

---

### Task 2: Apply the density factor + the monthly population gauge

**Files:** Modify `backend/crates/sim-core/src/population/mod.rs` (`population_monthly_system`)

- [ ] **Step 1: Write the failing integration test** (in the same `mod tests`)

This mirrors the existing harnesses (e.g. around `:415`) that insert a `PopulationConfig`, spawn agents, and advance months. It asserts the bounded band.

```rust
#[test]
fn carrying_capacity_bounds_population_in_a_band() {
    // Seed a clearly-fertile young population well below K, run many sim-months,
    // and assert the active count never hits 0 and never exceeds K_hard, settling
    // near K rather than growing unbounded.
    let k = 80.0_f32;
    let overshoot = 1.25_f32;
    let k_hard = (k * overshoot).ceil() as usize; // 100
    let mut h = PopulationTestHarness::new(PopulationConfig {
        carrying_capacity: k,
        capacity_overshoot: overshoot,
        ..PopulationConfig::default()
    });
    h.seed_agents(60, /*ages*/ 20..35); // 60 fertile-age agents, below K
    let mut max_n = 0usize;
    for _ in 0..(40 * 12) { // 40 sim-years of monthly steps
        h.advance_one_month();
        let n = h.active_agent_count();
        assert!(n > 0, "population must never reach 0 within the band");
        assert!(n <= k_hard, "population must never exceed K_hard={k_hard}, got {n}");
        max_n = max_n.max(n);
    }
    assert!(h.active_agent_count() >= 40, "should settle near K, not collapse");
    assert!(max_n >= 70, "should grow up toward K from the seed (saw max {max_n})");
}

#[test]
fn zero_capacity_is_unbounded() {
    // With regulation disabled, the mildly super-replacement schedule grows past K_hard-equivalent.
    let mut h = PopulationTestHarness::new(PopulationConfig {
        carrying_capacity: 0.0,
        ..PopulationConfig::default()
    });
    h.seed_agents(60, 20..35);
    for _ in 0..(60 * 12) { h.advance_one_month(); }
    assert!(h.active_agent_count() > 60, "unbounded schedule should grow above seed");
}
```

If a reusable `PopulationTestHarness` does not already exist, add a minimal one in `mod tests` that: builds a `World`, inserts `SimClock`, `PopulationConfig`, `LastProcessedMonth`, `AgentIdIndex`; `seed_agents(n, age_range)` spawns `n` agents (deterministic ids, `Sex::Female` for the fertile ones, `BirthTick` set so age ∈ range, via `crate::mobility::api::spawn_agent_from_record_at_position`); `advance_one_month()` bumps `SimClock` by one month and runs `population_monthly_system`; `active_agent_count()` returns `world.resource::<AgentIdIndex>().0.len()`. Model it on the existing month-advancing tests near `population/mod.rs:340-430`.

- [ ] **Step 2: Run to verify it fails**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core population::tests::carrying_capacity`
Expected: FAIL — population grows past `K_hard` (density factor not applied yet).

- [ ] **Step 3: Apply the density factor in the fertility phase**

In `population_monthly_system`, inside the `for m in (last + 1)..=current_month` loop, AFTER the mortality despawn block and BEFORE the fertility candidate loop, compute the live count and density once:
```rust
        // ---- Fertility (density-regulated) ----
        let live_n = world.resource::<crate::mobility::resources::AgentIdIndex>().0.len();
        let density = fertility_density_factor(live_n, &cfg);
```
Then change the birth-acceptance test (currently `if draw >= birth_probability_month(age, &cfg) { continue; }`) to scale by `density`:
```rust
            if draw >= birth_probability_month(age, &cfg) * density {
                continue;
            }
```
(`density` is a pure function of `live_n` — the deterministic post-mortality month state — so replay is unaffected; the per-agent `unit_draw(hash, m, 1)` is unchanged.)

- [ ] **Step 4: Run to verify the bounded-band tests pass**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core population::tests::carrying_capacity population::tests::zero_capacity`
Expected: PASS (both).

- [ ] **Step 5: Add the monthly gauge**

At the end of the `for m in ...` loop body (after the child-spawn loop, before the loop closes), add a read-only gauge. `victims` and `candidates` are in scope (deaths = `victims.len()` — capture it before `victims` is moved by the despawn loop, so store `let deaths = victims.len();` right after building `victims`; births = `candidates.len()`):
```rust
            let live_after = world.resource::<crate::mobility::resources::AgentIdIndex>().0.len();
            tracing::info!(
                target: "population::liveness",
                month = m,
                n = live_after,
                births = candidates.len(),
                deaths = deaths,
                "population month"
            );
```
Add `let deaths = victims.len();` immediately after the `victims` vector is fully built (before the despawn `for` loop consumes it).

- [ ] **Step 6: Run the full sim-core population suite**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core population`
Expected: PASS (existing population tests + the new ones; existing tests use default `carrying_capacity=0.0` → unbounded → unchanged behavior).

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-core/src/population/mod.rs
git commit -m "feat(population): density-regulate fertility by carrying capacity + population::liveness gauge"
```

---

### Task 3: Floor-at-0 persist + health guards; remove the dead field

**Files:** Modify `backend/crates/sim-server/src/app/mod.rs`, `backend/crates/sim-server/src/app/tests.rs`

- [ ] **Step 1: Write the failing regression test** (in `app/tests.rs`)

Find the nearest existing test that builds an `AppState` + a mobility view and drives `health_response_for_state` / the persist path (the tests around `tests.rs:540-560` and `:790-800` construct `AppState` literals). Add a test asserting a **non-empty but below-seed** population is healthy and persists, and an **empty** one is not:

```rust
#[tokio::test]
async fn below_seed_count_population_is_healthy_and_persists() {
    // A living world that has dropped below its seed count (demographic deaths)
    // must still report health.ok and must persist — only an EMPTY world is invalid.
    let base_world = base_world_fixture();
    let state = app_state_with_agent_count(&base_world, 285).await; // helper: build AppState + a view with 285 agents
    let health = health_response_for_state(&state);
    assert!(health.ok, "285 (<300 seed) but >0 must be healthy");

    let empty = app_state_with_agent_count(&base_world, 0).await;
    let health0 = health_response_for_state(&empty);
    assert!(!health0.ok, "0 agents must be unhealthy");
}
```
Implement `app_state_with_agent_count` as a small test helper that constructs the `AppState` and sets `view.mobility_full_dto.agents` to a vector of `n` placeholder agent DTOs (reuse the existing view-construction pattern in `app/tests.rs`; the agents only need a length). If a persist-path test exists, add the parallel assertion that `n=285` writes a snapshot and `n=0` is refused; otherwise the health assertions plus the unit-level guard suffice.

- [ ] **Step 2: Run to verify it fails / doesn't compile**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server below_seed_count`
Expected: FAIL — with the current guard, `285 < expected_base_world_agents(300)` makes `health.ok=false`.

- [ ] **Step 3: Change the two guards + remove the field**

In `app/mod.rs` health guard (`:536`):
```rust
    let runtime_agents_ok = !view.mobility_full_dto.agents.is_empty();
```
In `app/mod.rs` persist guard (`:1352`), replace the `< state.expected_base_world_agents` block with an empty check:
```rust
        if mobility_world.agents.is_empty() {
            let error = "refusing to persist empty mobility snapshot (0 agents)".to_string();
            mobility_liveness.record_failure(mobility_attempt, error.clone(), SystemTime::now());
            tracing::warn!(%error, "refusing to persist invalid mobility snapshot");
            return Ok(written);
        }
```
Remove the now-unused `expected_base_world_agents` field from the `AppState` struct (`:91`), its computation (`:152-153`), and its initializer (`:168`). In `app/tests.rs` remove the two `expected_base_world_agents:` initializers (`:558`, `:799`). Leave `crate::runtime::expected_base_world_agent_count` defined (Task 4 reuses it).

- [ ] **Step 4: Run to verify pass + no dead-code warning**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server below_seed_count`
Then: `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml -p sim-server -- -D warnings`
Expected: test PASS; clippy clean (no "field never read" for the removed field; `expected_base_world_agent_count` still used by Task 4 — if Task 4 not yet done, temporarily allow: do Task 4 before clippy, or expect a transient unused-fn warning that Task 4 resolves).

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-server/src/app/mod.rs backend/crates/sim-server/src/app/tests.rs
git commit -m "fix(server): floor persist+health guards at 0 (not base-world seed count)"
```

---

### Task 4: Wire carrying capacity from the base world (both runtime paths)

**Files:** Modify `backend/crates/sim-server/src/runtime/mod.rs`

- [ ] **Step 1: Write the failing test** (in `runtime/tests.rs`)

```rust
#[test]
fn runtime_sets_population_carrying_capacity_from_base_world_seed_count() {
    let runtime = SimulationRuntime::new(); // fresh path, abutopia (300 seed agents)
    let cfg = runtime.world.resource::<sim_core::population::PopulationConfig>();
    let expected = expected_base_world_agent_count(&base_world_fixture()) as f32;
    assert!(expected > 0.0);
    assert_eq!(cfg.carrying_capacity, expected, "carrying capacity = base-world seed count");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server runtime_sets_population_carrying_capacity`
Expected: FAIL — `carrying_capacity` is the default `0.0`.

- [ ] **Step 3: Set carrying_capacity after both PopulationPlugin installs**

In `runtime/mod.rs`, immediately after EACH `sim_core::population::PopulationPlugin.install(&mut world, &mut schedule);` (the fresh path `:220` and the hydrate path `:364`), add (re-applied every boot, so it survives restart like the EconomyConfig pattern):
```rust
        if let Some(mut pcfg) = world.get_resource_mut::<sim_core::population::PopulationConfig>() {
            pcfg.carrying_capacity = expected_base_world_agent_count(base_world_or_bundle) as f32;
        }
```
Use the base-world value in scope: in the fresh path that is `&bundle` (the local `bundle: BaseWorldBundle`); in `hydrate_from_stores` that is `base_world` (the `&BaseWorldBundle` parameter). `expected_base_world_agent_count` is already imported at `runtime/mod.rs:66`.

- [ ] **Step 4: Run to verify pass**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server runtime_sets_population_carrying_capacity`
Expected: PASS (`carrying_capacity == 300`).

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-server/src/runtime/mod.rs
git commit -m "feat(server): set population carrying capacity = base-world seed count (both boot paths)"
```

---

### Task 5: Full gate

- [ ] **Step 1: Rust gate** (background the slow ones; `pgrep -f cargo` first)
```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server
```
Expected: fmt clean; clippy 0 warnings (the removed field + repurposed fn leave no dead code); sim-core + sim-server tests pass incl. the new ones.

- [ ] **Step 2: Frontend gate** (symlink `node_modules` → main repo if the worktree lacks it: `ln -sfn /Users/ramonfuglister/Coding/abutown/node_modules node_modules`)
```bash
npm run typecheck && npm test && npm run build
```
Expected: clean (no frontend change).

- [ ] **Step 3: e2e render-smoke**
```bash
CORS_ALLOWED_ORIGINS="http://127.0.0.1:5173" npm run test:e2e
```
Expected: render-smoke 2/2 (no wire change).

---

### Task 6: PR (finishing-a-development-branch)

- [ ] **Step 1:** Use **superpowers:finishing-a-development-branch** → push + PR against `main`.
- [ ] **Step 2:** PR body: floor-at-0 guards (fixes the live black screen once a demographic population drops below the seed count) + carrying-capacity density regulation (bounds growth) + the gauge; reference the spec + the NRR=1.044 grounding; note no DB migration (config-only, re-applied each boot).
- [ ] **Step 3:** Wait for ALL CI checks green, squash-merge, clean up the worktree + branch.

**Post-merge (operational, controller + user — NOT a code task):** redeploy to Fly (`cd` an origin/main checkout, `fly deploy --ha=false --remote-only --app abutown-abutopia`); verify `/health`=200 + `healthy`, the Vercel frontend renders the living world over `wss://` (black screen cleared), and `fly logs` shows a bounded `population::liveness` `n`.

---

## Self-Review

**Spec coverage:** §2.1 floor-at-0 guards → Task 3; §2.2 carrying-capacity density-dependent fertility → Tasks 1+2 (factor) + Task 4 (wiring K=seed count); §2.3 gauge → Task 2 Step 5; §3 tests → Tasks 1/2/3/4 + Task 5; §4 deploy → Task 6 post-merge. All covered. The spec's "authorable per world" is realized as "runtime sets K = base-world seed count" (cleaner than a new authored field; K auto-scales with the world and reuses `expected_base_world_agent_count`).

**Placeholder scan:** No TBD/TODO. `app_state_with_agent_count` and `PopulationTestHarness` are specified as "build it mirroring existing patterns at <line>"; both are concrete (length-only agent DTOs; the existing month-advancing harness). Constants (`K=80/100`, `overshoot=1.25`, `K=300`) are explicit.

**Type consistency:** `fertility_density_factor(n: usize, c: &PopulationConfig) -> f32` used identically in Tasks 1 + 2. `carrying_capacity: f32` / `capacity_overshoot: f32` consistent across config (Task 1), apply (Task 2), wiring (Task 4). Guard floor `is_empty()` / `== 0` consistent in Task 3. `expected_base_world_agent_count` kept (Task 3) and reused (Task 4) — no dangling reference.
