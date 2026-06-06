# Per-Capita Economic Scaling + Visible Density — Implementation Plan (Slice 2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire a live-count-driven `capita_factor` into the economy so wage/consumption throughput AND the visible attribution cohort track the citizen count — keeping the #78 money-conservation audit byte-invariant and i64 arithmetic overflow-safe.

**Architecture:** A derived `CapitaFactor` resource (`= max(1, live_count / capita_baseline)`, recomputed at the monthly boundary) scales demand `autonomous` and supply `offered_qty` at **runtime** (transfers only — no runtime mint, so the audit is unchanged), and makes the attribution cap **population-aware** for visible density. Default `capita_baseline` keeps the factor at **1 (identity → byte-identical to today)** until deliberately ramped (by *lowering* the baseline). Wages stay labor-share of the now-larger revenue (never an artificial multiply). Delivered as audited sub-slices 2a–2f.

**Tech Stack:** Rust (bevy_ecs, `sim-core`). All cargo via `scripts/cargo-serial.sh` (CLAUDE.md). Base: `feat/per-capita-scaling` off `origin/main` (`d79f85c`, Slice 1 merged), worktree `/Users/ramonfuglister/Coding/abutown-percapita`.

---

## Design decisions baked in (from spec + code-map)

- **`CapitaFactor(pub i64)`** resource, value `= max(1, live_count / capita_baseline)` (integer floor). Default `capita_baseline = 1_000_000` → at ~300 live citizens the factor is `1` (identity). **Ramp by *lowering* `capita_baseline`** (e.g. `10` → factor `30`); never raise it. This reconciles the inert `household.population=1M` into a live tuning knob.
- **Runtime scaling, not seed scaling, is the primary lever** (economy seeds *before* citizens exist, so the live count isn't available at seed). `autonomous` and `offered_qty` scale per-tick by `capita_factor`. Because every flow stays a transfer, **the #78 audit is byte-invariant and needs no change**.
- **`opening_cash` stays FIXED at the realistic ~10–30× scale** (it circulates via the wage loop; `1M` is ample). Therefore **Slice 2 ships at identity with NO migration**. Seed-side `opening_cash`/`opening_inventory` scaling (sub-slice 2b) is implemented behind the same factor but is a no-op at identity; a one-time `DELETE FROM economy_snapshots` is required **only** if that seed scaling is active *and* the factor is ramped. The solvency check in 2b decides whether seed scaling is even needed.
- **Wages unchanged in form** — `labor_share·revenue`; revenue grows because quantities grow. `wage ≤ revenue` and the `HOUSEHOLD_SECTOR`/`TRANSPORT_OPERATOR` net-zero sentinels hold.
- **Cohort cap population-aware** (2d): replace the absolute `max_shoppers_per_market`/`max_commuters_per_market = 4` with a cap that scales with `capita_factor`; viewport-independence preserved (cap derives from population, not observation).

## File structure

**Create:**
- `backend/crates/sim-core/src/economy/capita.rs` — `CapitaFactor` resource + the pure `capita_factor(live_count, capita_baseline) -> i64` helper + `refresh_capita_factor_system` (monthly recompute reading `AgentIdIndex`) + unit tests.

**Modify:**
- `economy/systems.rs` — add `capita_baseline: i64` to `EconomyConfig` + `Default`; add `EconomySet::RefreshCapita` phase + register `refresh_capita_factor_system`; thread `CapitaFactor` where consumption/supply run.
- `economy/mod.rs` — `pub mod capita;`; insert `CapitaFactor::default()` in `EconomyPlugin::install`.
- `economy/pools.rs` — `target_spend` takes a `capita_factor` and scales `autonomous`; `generate_pool_orders_at_tick` scales supply `offered_qty` by the factor; thread the factor from the calling systems.
- `economy/markets_layer.rs` — (2b) optionally scale `opening_cash`/`opening_inventory`/seed `qty` by a seed-time factor.
- `economy/attribution.rs` — (2d) replace the absolute cap args with a population-aware `pop_cap` derived from `CapitaFactor`.
- `economy/tests/` — per-sub-slice tests (audit byte-invariance at factors, solvency, monthly recompute, pop-cap, overflow).

Conventions: new tests are `#[cfg(test)]` modules co-located in the file under test (or `capita.rs`'s own module). Single-test run:
`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core <FILTER> -- --nocapture`

---

### Task 2a: `CapitaFactor` resource + demand-side scaling (identity default)

Add the factor (default identity) and scale demand `autonomous`. At `capita_factor == 1` everything is byte-identical to today.

**Files:**
- Create: `backend/crates/sim-core/src/economy/capita.rs`
- Modify: `economy/mod.rs`, `economy/systems.rs` (`EconomyConfig` + `Default`), `economy/pools.rs` (`target_spend` + its caller `run_consumption_update_at_tick`)

- [ ] **Step 1: Pure factor helper + resource + failing tests (`capita.rs`)**

```rust
//! Per-capita scaling factor: economic throughput (and the visible attribution
//! cohort) track the live citizen count. `capita_factor = max(1, live_count /
//! capita_baseline)` (integer floor). Default `capita_baseline` keeps the factor
//! at 1 (identity) until deliberately ramped by LOWERING the baseline.

use bevy_ecs::prelude::Resource;

/// Per-tick scaling multiplier applied to real-quantity flows + cohort caps.
/// `1` = identity (byte-identical to the un-scaled economy).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapitaFactor(pub i64);

impl Default for CapitaFactor {
    fn default() -> Self {
        CapitaFactor(1)
    }
}

/// Derive the factor from the live citizen count and the configured baseline.
/// Floor division, clamped to `>= 1`. `capita_baseline <= 0` is treated as the
/// neutral 1 (never divide by zero, never invert the meaning).
pub fn capita_factor(live_count: u64, capita_baseline: i64) -> i64 {
    if capita_baseline <= 0 {
        return 1;
    }
    let raw = (live_count as i128) / (capita_baseline as i128);
    i64::try_from(raw).unwrap_or(i64::MAX).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_when_baseline_exceeds_count() {
        // default baseline 1_000_000 with ~300 live → factor 1 (identity).
        assert_eq!(capita_factor(300, 1_000_000), 1);
    }

    #[test]
    fn scales_up_when_baseline_lowered() {
        assert_eq!(capita_factor(300, 10), 30);
        assert_eq!(capita_factor(1000, 10), 100);
    }

    #[test]
    fn floor_and_min_one() {
        assert_eq!(capita_factor(9, 10), 1, "floor(0.9)=0 clamped to 1");
        assert_eq!(capita_factor(0, 10), 1, "zero citizens still 1, never 0");
    }

    #[test]
    fn nonpositive_baseline_is_neutral() {
        assert_eq!(capita_factor(300, 0), 1);
        assert_eq!(capita_factor(300, -5), 1);
    }
}
```

- [ ] **Step 2: Declare module + insert resource**

`economy/mod.rs`: add `pub mod capita;` near the other `pub mod` lines, and in `EconomyPlugin::install`'s resource-insert block add:
```rust
        world.insert_resource(crate::economy::capita::CapitaFactor::default());
```

- [ ] **Step 3: Add `capita_baseline` to `EconomyConfig` + Default (`systems.rs`)**

In the `EconomyConfig` struct (after `price_ceiling: Money,`):
```rust
    /// Per-capita scaling baseline: `capita_factor = max(1, live_count / capita_baseline)`.
    /// Default 1_000_000 keeps the factor at 1 (identity) at the ~300-citizen seed scale;
    /// LOWER it to ramp throughput up (e.g. 10 → ~30x at 300 citizens). Never raise it.
    pub capita_baseline: i64,
```
In `impl Default for EconomyConfig` (after `price_ceiling: Money(100_000),`):
```rust
            capita_baseline: 1_000_000,
```

- [ ] **Step 4: Scale `autonomous` in `target_spend` (`pools.rs`)**

`target_spend` currently is `(autonomous, mpc_bps, income_last_tick) -> Result<Money>` computing `C = autonomous + floor(mpc·income/10_000)`. Add a `capita_factor: i64` parameter and scale ONLY the autonomous floor (the induced `mpc·income` term already scales because income scales with the larger revenue):
```rust
pub(crate) fn target_spend(
    autonomous: Money,
    mpc_bps: i32,
    income_last_tick: Money,
    capita_factor: i64,
) -> Result<Money, EconomyError> {
    if !(0..=10_000).contains(&mpc_bps) {
        return Err(EconomyError::InvalidOrder);
    }
    let scaled_autonomous = i64::try_from((autonomous.0 as i128) * (capita_factor.max(1) as i128))
        .map_err(|_| EconomyError::Overflow)?;
    let induced = i64::try_from((income_last_tick.0 as i128) * (mpc_bps as i128) / 10_000)
        .map_err(|_| EconomyError::Overflow)?;
    Money(scaled_autonomous).checked_add(Money(induced))
}
```
Update the sole caller `run_consumption_update_at_tick` to take + pass the factor:
```rust
pub fn run_consumption_update_at_tick(
    demand: &mut DemandPools,
    market_goods: &MarketGoods,
    capita_factor: i64,
) -> Result<(), EconomyError> {
    for pool in demand.0.values_mut() {
        let key = MarketGoodKey { market: pool.market, good: pool.good };
        let spend = target_spend(pool.autonomous, pool.mpc_bps, pool.income_last_tick, capita_factor)?;
        let state = market_goods.0.get(&key).ok_or(EconomyError::ZeroPrice)?;
        pool.desired_qty_per_tick = spend_to_qty(spend, state.ewma_reference_price)?;
    }
    Ok(())
}
```
Then thread `capita_factor` from the system that calls `run_consumption_update_at_tick` (find it in `systems.rs` — it runs in the `UpdateConsumption` set; add `CapitaFactor` to its params/`world.resource` read and pass `.0`). Update every other caller (tests) to pass `1`.

- [ ] **Step 5: Tests — pure helper + identity invariance**

Run the `capita.rs` unit tests:
`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core capita -- --nocapture` → 4 pass.

Add an economy test (in `economy/tests/`) asserting **byte-invariance at multiple factors**: build the standard economy fixture (reuse `economy/tests/seed.rs` / the conservation-test harness), run N ticks with `CapitaFactor(1)`, `CapitaFactor(2)`, `CapitaFactor(10)`, and assert `run_tick_audit_at_tick` never reports a `ConservationViolation` (total_money byte-invariant tick-over-tick) at each factor. At factor 1, also assert the per-tick ledger/quantities are identical to a baseline run with the factor code absent-equivalent (i.e. factor 1 == today).

- [ ] **Step 6: Run + gate + commit**

`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core capita -- --nocapture`
`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy -- --nocapture` (all green; factor defaults to 1 → existing tests unchanged)
fmt + scoped clippy clean.
```bash
git add backend/crates/sim-core/src/economy/capita.rs backend/crates/sim-core/src/economy/mod.rs backend/crates/sim-core/src/economy/systems.rs backend/crates/sim-core/src/economy/pools.rs
git commit -m "feat(economy): CapitaFactor resource + demand-side per-capita scaling (identity default)"
```

---

### Task 2b: supply-side scaling + (conditional) seed scaling + solvency check

Scale supply `offered_qty` by the factor so revenue grows with demand (keeping the loop balanced and prices stable), then empirically decide whether `opening_cash` needs scaling.

**Files:**
- Modify: `economy/pools.rs` (`generate_pool_orders_at_tick` supply branch), `systems.rs` (thread factor), `economy/markets_layer.rs` (conditional seed scaling)

- [ ] **Step 1: Scale supply `offered_qty` (`pools.rs`)**

In `generate_pool_orders_at_tick`, add a `capita_factor: i64` parameter and scale the supply offer before the inventory clamp:
```rust
        // supply branch:
        let available = inventory.balance(actor, pool.good).available;
        let scaled_offer = i64::try_from((pool.offered_qty_per_tick.0 as i128) * (capita_factor.max(1) as i128))
            .map_err(|_| EconomyError::Overflow)?;
        let capped = Quantity(scaled_offer.min(available.0));
```
Thread `capita_factor` from the calling system (the `GeneratePoolOrders` set) reading `CapitaFactor`; pass `1` in tests not exercising scaling. (Demand bids are already scaled via `desired_qty_per_tick` from 2a; leave the demand branch's `affordable_qty` clamp as-is — it correctly bounds bids by cash.)

- [ ] **Step 2: Solvency test at the target factor (`economy/tests/`)**

Add a test that runs the full economy schedule for ~50 ticks at `CapitaFactor(30)` (the realistic ramp target) with **`opening_cash` unchanged** and asserts: (a) the audit stays byte-invariant; (b) demand does NOT collapse — i.e. `FinalConsumed`/`Trade` events keep firing (no run of all-`OrderRejected{InsufficientFunds}`), proving the circulating wage loop keeps the fixed `opening_cash` solvent at 30x. Capture the steady-state consumed-qty to confirm it scaled ~30x vs factor 1.

- [ ] **Step 3: Conditional seed scaling (only if Step 2 shows starvation)**

If and only if Step 2 shows cash-starvation/demand-collapse at the target factor, scale the seed in `markets_layer.rs` by a **seed-time factor** (the live count is unavailable at seed, so use `capita_factor(expected_population, config.capita_baseline)` where `expected_population` is read from the markets layer `household.population`, now repurposed as the expected headcount). Multiply `spec.opening_cash`, `spec.opening_inventory`, and seed `spec.qty` by that factor with checked arithmetic. **Document the one-time `DELETE FROM economy_snapshots`** this requires (changed seed money stock). If Step 2 shows solvency WITHOUT this, SKIP this step and record "seed scaling unnecessary at target factor — no economy_snapshots migration." Prefer skipping (no destructive migration) when the loop is solvent.

- [ ] **Step 4: Run + gate + commit**

Run the solvency test + full economy suite + the audit/conservation tests; fmt + clippy clean.
```bash
git add backend/crates/sim-core/src/economy/pools.rs backend/crates/sim-core/src/economy/systems.rs backend/crates/sim-core/src/economy/markets_layer.rs
git commit -m "feat(economy): supply-side per-capita scaling + solvency-verified (seed scaling conditional)"
```

---

### Task 2c: drive `CapitaFactor` from the live count (monthly)

Make the factor emergent: recompute it from `AgentIdIndex.len()` at the monthly boundary.

**Files:**
- Modify: `economy/capita.rs` (`refresh_capita_factor_system` + `LastCapitaMonth` state), `economy/systems.rs` (`EconomySet::RefreshCapita` phase + registration)

- [ ] **Step 1: Monthly refresh system (`capita.rs`)**

Add an exclusive system that, once per sim-month, reads the live count and the baseline and updates `CapitaFactor`. Gate on month change via a small `LastCapitaMonth(Option<u64>)` resource so it recomputes only when the month advances (deterministic, no per-tick churn). Read the month via the same `month_of(tick)` helper the population system uses (find it in `population`/`time`), and the live count via `world.resource::<crate::mobility::resources::AgentIdIndex>().0.len()`. No-op cleanly if `AgentIdIndex` is absent (economy-only tests):
```rust
pub fn refresh_capita_factor_system(world: &mut bevy_ecs::world::World) {
    use crate::economy::EconomyConfig;
    use crate::economy::capita::{CapitaFactor, capita_factor};
    // month gating + AgentIdIndex read; update CapitaFactor only on month change.
    // (Exact month_of + tick source: mirror population_monthly_system.)
    let Some(index) = world.get_resource::<crate::mobility::resources::AgentIdIndex>() else { return };
    let live = index.0.len() as u64;
    let baseline = world.resource::<EconomyConfig>().capita_baseline;
    let f = capita_factor(live, baseline);
    world.resource_mut::<CapitaFactor>().0 = f;
}
```
(Implement the month-gating with `LastCapitaMonth`; if mirroring the exact `month_of`/`Tick` plumbing is unclear, recompute every tick — it reads a `len()` and the count only changes monthly, so the value is stable between births; note which you chose. Determinism holds either way.)

- [ ] **Step 2: Register the phase (`systems.rs`)**

Add `EconomySet::RefreshCapita` as the FIRST phase in the `EconomySet` chain (before `ResetReceipts`), so the factor is current before any scaled flow reads it this tick. Register `refresh_capita_factor_system` in that set (exclusive, like the attribution/audit systems).

- [ ] **Step 3: Test the monthly recompute**

Add a test: spawn a known live count (insert an `AgentIdIndex` with N entries, or use the mobility fixture), set `capita_baseline` low enough for factor > 1, run the refresh, assert `CapitaFactor` == `capita_factor(N, baseline)`. Add births (grow the count), advance a month, re-run, assert the factor stepped. Assert determinism (same inputs → same factor) and that the audit holds across the factor step.

- [ ] **Step 4: Run + gate + commit**

```bash
git add backend/crates/sim-core/src/economy/capita.rs backend/crates/sim-core/src/economy/systems.rs
git commit -m "feat(economy): derive CapitaFactor from the live citizen count (monthly)"
```

---

### Task 2d: population-aware cohort cap (visible density)

Replace the absolute attribution caps with a population-aware `pop_cap` so more citizens are visibly economic as the population grows.

**Files:**
- Modify: `economy/attribution.rs` (the two `attribute_channel` call sites in `run_citizen_attribution_system`)

- [ ] **Step 1: Compute `pop_cap` from the factor**

In `run_citizen_attribution_system`, read `CapitaFactor` (`world.resource::<CapitaFactor>()` — it's inserted by the plugin) alongside the existing `EconomyConfig`. Derive a population-aware cap that scales with the factor but never below the configured absolute floor:
```rust
    let capita = world.resource::<crate::economy::capita::CapitaFactor>().0.max(1) as usize;
    let shopper_cap = config.max_shoppers_per_market.saturating_mul(capita);
    let commuter_cap = config.max_commuters_per_market.saturating_mul(capita);
```
Pass `shopper_cap` / `commuter_cap` instead of `config.max_shoppers_per_market` / `config.max_commuters_per_market` at the two `attribute_channel` call sites (`attribution.rs:171-176` shop, `:189-194` wage). The cohort stays `min(realized/per_unit, pop_cap, candidates)` — so at identity (factor 1) it is byte-identical to Slice 1; at factor 30 the cap is 120/market, letting `candidates` (the bound, observed citizens) become the binding term → visible density. Viewport-independence preserved: `pop_cap` derives from the population factor, not from observation.

> Note: this widening only matters once `realized` is large enough that `realized/per_unit` exceeds the old cap of 4 — which is exactly what 2a/2b's throughput scaling produces. The two levers are co-dependent by design.

- [ ] **Step 2: Test**

Extend the attribution system tests: with `CapitaFactor(1)`, cohort == Slice-1 behaviour (cap 4). With `CapitaFactor(30)` + a market whose `realized` and `candidates` both exceed the old cap, assert the attributed cohort grows to `min(realized/per_unit, 4*30, candidates)` (i.e. tracks candidates, not clamped at 4). Assert attribution still writes no money (the conservation property from Slice 1).

- [ ] **Step 3: Run + gate + commit**

```bash
git add backend/crates/sim-core/src/economy/attribution.rs
git commit -m "feat(economy): population-aware attribution cohort cap (visible density)"
```

---

### Task 2e: per-capita overflow stress test

Pin the overflow behaviour the existing 1M test does not cover (it leaves population a no-op).

**Files:**
- Modify: `economy/tests/wages.rs` (or a new `economy/tests/capita_overflow.rs`)

- [ ] **Step 1: Stress test**

Mirror `pay_wages_population_million_max_revenue_no_overflow` (the existing template in `economy/tests/wages.rs`). Drive revenue toward `i64::MAX/2` AND apply a large `capita_factor` to demand/supply across the relevant formulas (`target_spend` scaled autonomous, `spend_to_qty`, `affordable_qty`, supply `offered_qty` scale, `wage_for_revenue`), with a firm count of 10–100 across multiple ticks (to stress `wage_bill` and `traded_qty_last_tick` accumulators). Assert: each scaled multiply returns `EconomyError::Overflow` (fail-fast, never silent wrap) at the ceiling; and below the ceiling, conservation holds (audit byte-invariant). Confirm the realistic factor (~30 at 300 citizens) is comfortably below any ceiling.

- [ ] **Step 2: Run + gate + commit**

```bash
git add backend/crates/sim-core/src/economy/tests/
git commit -m "test(economy): per-capita overflow stress (fail-fast at ceiling, conservation below)"
```

---

### Task 2f: whole-system verification + ramp

**Files:** none new — integrated verification.

- [ ] **Step 1: Audit byte-invariance across the integrated slice**

`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core audit -- --nocapture`
`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core conservation -- --nocapture`
Expected green at default (identity) and at the tested ramped factors.

- [ ] **Step 2: Full Rust gate**

`scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check`
`scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings`
`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace`
All green. (Workspace covers sim-server, which seeds the economy beside mobility/population — confirms the cross-module `AgentIdIndex` read compiles + runs there.)

- [ ] **Step 3: Frontend gate**

`npm ci` (fresh worktree), then `npm run typecheck`, `npm test`, `npm run build` — all green (no frontend code changed; wire contract unchanged).

- [ ] **Step 4: Browser-smoke (mandatory — agent stream changes)**

The agent stream changes (more citizens economically routed once ramped). Run the e2e/render-smoke; the 300-pin holds (citizens keep `agent:walk:*` ids), more are economically active. (CI runs this in an isolated fresh DB — the safe environment; do not run against a shared dev DB.)

- [ ] **Step 5: Decide the shipped default factor**

Ship at **identity** (`capita_baseline = 1_000_000` → factor 1) so the merge is behaviourally inert and risk-free. Document how to ramp (lower `capita_baseline`) and the verified-safe target (~10–30× at ~300 citizens). Only ramp the *default* after 2a–2e are green AND, if seed `opening_cash` scaling was activated in 2b, after the one-time `DELETE FROM economy_snapshots`.

- [ ] **Step 6: Branch ready**

Branch `feat/per-capita-scaling` ready for finishing-a-development-branch (PR to `origin/main`). **Deploy note:** Slice 2 ships at identity → **no migration** by default. `mobility_snapshots`: no change. `economy_snapshots`: a one-time `DELETE` is needed ONLY if seed `opening_cash` scaling (2b Step 3) was activated and the factor is ramped — otherwise none.

---

## Self-Review (author checklist — completed)

**Spec coverage:** live-count factor (2c) · hybrid magnitude (2a demand + 2b supply, opening_cash conditional) · wages-as-labor-share unchanged (no task — verified untouched) · population-aware caps (2d) · overflow safety (2e) · audit byte-invariance (2a/2f) · monthly cadence (2c) · determinism (2a/2c tests) · persistence/migration (2b/2f deploy note) · browser-smoke + full gate (2f). The spec's "scale opening_cash + DELETE" is refined to *conditional* (2b Step 3) because at the realistic ~10–30× the loop is solvent on the fixed 1M — strictly safer (avoids a destructive migration); the capability is still implemented behind the factor.

**Placeholder scan:** No TBD/handle-errors. Two steps say "find the exact caller / month_of plumbing then thread the factor" (2a Step 4, 2c Step 1) — these name the exact function and the fallback (recompute per-tick), because the precise calling-system params and the `month_of`/`Tick` helper weren't captured verbatim and must not be invented.

**Type consistency:** `CapitaFactor(i64)` and `capita_factor(live_count: u64, capita_baseline: i64) -> i64` consistent across capita.rs, the `target_spend`/`generate_pool_orders_at_tick` params, the attribution `pop_cap`, and the refresh system. `capita_baseline: i64` on `EconomyConfig` consistent. Identity (factor 1) is the default everywhere → every existing test stays green without changes.
