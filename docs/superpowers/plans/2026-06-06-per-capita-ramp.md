# Ramp + Live-Validate Per-Capita Density — Implementation Plan (Slice 2b)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `capita_baseline` authorable from `markets.json`, add a routed-citizen liveness gauge, then ramp abutopia and validate the visible economic density live — keeping the #78 audit byte-invariant, prices in-band, perf fine, no migration.

**Architecture:** `seed_from_markets_layer` reads an authored `capita_baseline` from the world's `household` block (serde-default `1_000_000` = identity) and writes it into `EconomyConfig` (which Slice 2's `refresh_capita_factor_system` already consumes each tick). Code default + all other worlds stay at identity; only abutopia's JSON carries the ramp. Validation is a deterministic backend test (density up + safety) plus a live run (screenshot + the routed-count gauge), tuning the JSON value.

**Tech Stack:** Rust (bevy_ecs, `sim-core`), JSON world data, Playwright browser-smoke. Cargo via `scripts/cargo-serial.sh`. Base: `feat/per-capita-ramp` off `origin/main` (`2961e94`), worktree `/Users/ramonfuglister/Coding/abutown-percapita-ramp`.

## File structure

- **Modify** `backend/crates/sim-core/src/base_world.rs` — `HouseholdSpec` gains `capita_baseline` (serde-default 1M) + a `default_capita_baseline()` fn.
- **Modify** `backend/crates/sim-core/src/economy/markets_layer.rs` — `seed_from_markets_layer` writes `EconomyConfig.capita_baseline` from the layer.
- **Modify** `backend/crates/sim-core/src/economy/attribution.rs` (or `capita.rs`) — a periodic `tracing::info!` liveness gauge of `CitizenEconomicTargets.len()`.
- **Modify** `data/worlds/abutopia/layers/markets.json` — author `household.capita_baseline` (Task 4, after tuning).
- **Tests:** `economy/tests/` — authoring wiring + density/safety; any `HouseholdSpec { .. }` struct-literal sites updated for the new field.

Cargo serial wrapper + `-p sim-core` scope per CLAUDE.md.

---

### Task 1: Authorable `capita_baseline` (identity-preserving)

Wire the mechanism; abutopia stays at identity (1M) until Task 4 tunes it.

**Files:** `base_world.rs`, `economy/markets_layer.rs` (+ any `HouseholdSpec` literals)

- [ ] **Step 1: Add the serde-default field to `HouseholdSpec` (`base_world.rs`)**

```rust
fn default_capita_baseline() -> i64 {
    1_000_000
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HouseholdSpec {
    pub population: u64,
    /// Per-capita scaling baseline (`capita_factor = max(1, live_count / capita_baseline)`).
    /// Authored per world; LOWER it to ramp economic throughput + visible density up
    /// (e.g. 10 → ~30x at 300 citizens). Defaults to 1_000_000 = identity for worlds that
    /// omit it. Loaded as world data (serde-default OK here — not a persisted snapshot).
    #[serde(default = "default_capita_baseline")]
    pub capita_baseline: i64,
}
```
Then `cargo build -p sim-core` and add `capita_baseline: 1_000_000` (or `default_capita_baseline()`) to every `HouseholdSpec { .. }` struct literal the compiler names (there is at least one in a `base_world.rs` test fixture ~line 831).

- [ ] **Step 2: Apply it in `seed_from_markets_layer` (`markets_layer.rs`)**

`seed_from_markets_layer` runs AFTER `EconomyPlugin::install` inserts `EconomyConfig::default()` (verified: runtime installs economy then seeds). In the household-seeding block (where it does `world.insert_resource(HouseholdSector { population: layer.household.population, .. })`), also write the baseline into the config:
```rust
    world.resource_mut::<crate::economy::EconomyConfig>().capita_baseline =
        layer.household.capita_baseline;
```
(If `EconomyConfig` might be absent in a narrow seed path, guard with `if let Some(mut cfg) = world.get_resource_mut::<EconomyConfig>() { cfg.capita_baseline = ...; }` — but in the runtime + economy test seed it is present.)

- [ ] **Step 3: Test — authored value reaches `EconomyConfig`; default is identity**

Add a `#[cfg(test)]` test (in `economy/tests/`, reusing the `seed.rs` bundle/seed fixture): seed a world from a `MarketLayer` whose `household.capita_baseline = 10`, assert `world.resource::<EconomyConfig>().capita_baseline == 10`. Second case: a `MarketLayer` built/deserialized WITHOUT the field → `capita_baseline == 1_000_000` (serde-default identity). If the fixture builds `MarketLayer` via a struct literal (not JSON), test the serde-default separately by `serde_json::from_str` of a `household` object lacking `capita_baseline` → `1_000_000`.

- [ ] **Step 4: Confirm identity unchanged + commit**

abutopia's `markets.json` is NOT changed yet (still no `capita_baseline` → serde-default 1M → identity). Run `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy -- --nocapture` (all green, unchanged). fmt + scoped clippy clean.
```bash
git add backend/crates/sim-core/src/base_world.rs backend/crates/sim-core/src/economy/markets_layer.rs
git commit -m "feat(economy): capita_baseline authorable from markets.json household (serde-default identity)"
```

---

### Task 2: Routed-citizen liveness gauge

A small, periodic log so the live run reports how many citizens are economically routed (and a permanent observability gauge).

**Files:** `economy/attribution.rs` (end of `run_citizen_attribution_system`)

- [ ] **Step 1: Add a periodic `tracing::info!` of the routed count**

At the END of `run_citizen_attribution_system` (after `CitizenEconomicTargets` is written), log its size every `macro_flow_interval_ticks` (or every 60 ticks) to avoid spam. Read the tick (the system is exclusive — `world.resource::<crate::mobility::resources::Tick>().0`) and the targets len:
```rust
    let tick = world.resource::<crate::mobility::resources::Tick>().0;
    let routed = world.resource::<CitizenEconomicTargets>().0.len();
    if tick % 60 == 0 {
        tracing::info!(target: "economy::liveness", tick, routed, "citizens economically routed this tick");
    }
```
(Place after the final write; keep it read-only. If `Tick` isn't already imported/available in the system, read it via `world.get_resource::<Tick>()`. Gate on `% 60` so it's a heartbeat, not per-tick spam.)

- [ ] **Step 2: Test + commit**

The existing attribution tests still pass (the log is side-effect-free). Add a one-line assertion is NOT needed (logging); just run `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core attribution -- --nocapture` green. fmt + scoped clippy clean.
```bash
git add backend/crates/sim-core/src/economy/attribution.rs
git commit -m "feat(economy): periodic routed-citizen liveness gauge (economy::liveness log)"
```

---

### Task 3: Backend density + safety test at the ramped factor

Prove deterministically that a ramped `capita_baseline` is denser AND safe.

**Files:** `economy/tests/capita.rs` (or a new test)

- [ ] **Step 1: Density-scales test**

Add a test that runs the full economy schedule for ~60 ticks twice — once at identity (`capita_baseline = 1_000_000`, or no citizens spawned for ramp) and once ramped (spawn N `AgentMarker` citizens + `capita_baseline` low enough for factor ~30, the Slice-2 pattern) — and asserts the ramped run's routed cohort (`CitizenEconomicTargets.len()` summed/maxed over ticks, or the attributed-per-market count) is materially larger than identity. Reuse `economy/tests/capita.rs`'s existing factor-30 harness (`run_solvency_scenario` / `run_conservation_with_factor`) which already spawns citizens + drives the factor.

- [ ] **Step 2: Safety over a longer run**

In the same (or an adjacent) test at factor ~30 over ~60 ticks, assert: (a) the #78 audit is byte-invariant every tick (the harness already does `total_money` invariance); (b) **prices stay within band** — every `MarketGoodState.ewma_reference_price` stays within `[config.price_floor, config.price_ceiling]` (no tâtonnement blow-up); (c) **no demand-collapse** — `FinalConsumed`/`Trade` events keep firing across the run (not an all-`InsufficientFunds` tail). This pins price-stability + solvency at the ramped factor before it ships.

- [ ] **Step 3: Run + commit**

`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core capita -- --nocapture` green; fmt + scoped clippy clean.
```bash
git add backend/crates/sim-core/src/economy/tests/capita.rs
git commit -m "test(economy): ramped capita_baseline is denser + price-stable + solvent over a long run"
```

---

### Task 4: Live validation + tune (controller-run, with the user)

Run the real abutopia stack on this branch, observe density, tune the JSON value. (This task is run by the controller, not a subagent — it needs the dev stack, the browser, and the user's visual "looks alive" judgment.)

- [ ] **Step 1: Switch the dev stack to this branch (clean restart, no migration)**

Stop the running dev backend (it's on a different worktree); fast-forward / point the dev worktree to `feat/per-capita-ramp` (or run the stack from this worktree). Slice 2b needs NO DB migration, so a clean restart suffices. Confirm the backend comes up healthy (no panic, audit not tripping) via `/tmp/abutown-backend.log`.

- [ ] **Step 2: Author the candidate + restart**

Set `data/worlds/abutopia/layers/markets.json` `household.capita_baseline = 10` (≈30× at ~300 citizens). Restart the backend. Confirm the `economy::liveness` log shows `routed` climbing well above the identity baseline.

- [ ] **Step 3: Browser-smoke + screenshot + count**

Drive the headless browser against the dev stack (adapt `scripts/smoke-7a.mjs` / the render-smoke), capture a screenshot of the city, and read the `economy::liveness` routed count from the backend log. Present both to the user: does it look alive (citizens clustered at / heading to markets)? Is the routed share reasonable?

- [ ] **Step 4: Tune**

Adjust `household.capita_baseline` (raise = less dense, lower = denser) and restart until the density looks right and prices/perf hold (watch the log for audit panics, price-ceiling clamps, slow-tick warnings). Settle on the final value with the user.

- [ ] **Step 5: Commit the chosen value**

```bash
git add data/worlds/abutopia/layers/markets.json
git commit -m "feat(world): ramp abutopia capita_baseline to <value> (per-capita density on)"
```

---

### Task 5: Whole-system verification + ship

- [ ] **Step 1: Full Rust gate** — `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check`; `clippy --workspace --all-targets -- -D warnings`; `test --workspace`. All green. (#78 audit/conservation green at the authored factor.)
- [ ] **Step 2: Frontend gate** — `npm ci` (fresh worktree); `npm run typecheck`; `npm test`; `npm run build`. Green (no frontend code changed; render-smoke 300-pin holds — citizens keep `agent:walk:*` ids).
- [ ] **Step 3: Browser-smoke** — covered live in Task 4; CI re-runs it in an isolated fresh DB.
- [ ] **Step 4: PR** — finishing-a-development-branch → PR to `origin/main`. **Deploy note:** no migration (world-data change + serde-default; no snapshot field; `HouseholdSector.population` untouched).

---

## Self-Review (author checklist — completed)

**Spec coverage:** authorable `capita_baseline` (Task 1) · routed-count gauge (Task 2) · backend density+safety incl. price-band + solvency (Task 3) · live validation + tune incl. screenshot + count (Task 4) · full gate + no-migration ship (Task 5). All spec sections covered.

**Placeholder scan:** No TBD/handle-errors. Task 1 Step 1 says "every `HouseholdSpec` literal the compiler names" — concrete (compile-driven), with the known site flagged. Task 4 is deliberately controller-run/empirical (the value is chosen by observation) — the candidate (10) and the tuning direction (raise=less dense) are explicit, not placeholders.

**Type consistency:** `capita_baseline: i64` consistent (HouseholdSpec field, the `EconomyConfig.capita_baseline` it writes, the Slice-2 `capita_factor` consumer). Serde-default `1_000_000` == the existing `EconomyConfig::default()` value → identity is preserved end-to-end for worlds omitting the field. `default_capita_baseline()` returns `i64`.
