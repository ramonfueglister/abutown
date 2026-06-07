# One-Sided Price Convergence (Flow-Margin Feedback) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a complementarity-gated, margin-anchored flow-margin nudge that converges a flow-destination market's reservation price toward `source_price + (rate × distance)` (spatial Law of One Price), so demand-only sinks (abutopia 9002) stop ratcheting to the ceiling.

**Architecture:** A new pure nudge (`nudge_price_toward_target`) + a per-cadence `RealizedFlows` carrier populated by `MacroFlow` + a `run_flow_margin_feedback_at_tick` pass that, for each realized flow edge, anchors the sink's demand-pool `max_price` to `p_src + rate·dist` and the source's supply-pool `min_price` to `p_dst − rate·dist`. The existing local-unmet tâtonnement is **skipped for flow-coupled pools** (the margin governs them; local governs autarkic). Prices are order-parameters — no money moved.

**Tech Stack:** Rust (bevy_ecs), `sim-core`. Cargo via `scripts/cargo-serial.sh` only. Determinism: BTreeMap/sorted, i128/floor/clamp, no RNG/wall-clock.

**Worktree:** `/Users/ramonfuglister/Coding/abutown-live`, branch `feat/abutopia-live-visible` (off `origin/main` `53cd2e3`). Spec: `docs/superpowers/specs/2026-06-07-economy-onesided-price-convergence-design.md`.

---

## Background the implementer needs (verified)

- **The canonical condition** (spec §"Theoretical grounding"): on an active route, `p_dst = p_src + transport_per_unit` (spatial Law of One Price; Samuelson 1952; Takayama & Judge 1971). Transport per unit **in price units = `rate × dist`** — verified from the macro-flow margin `net_gain = (q/SCALE)·(p_dst − p_src − rate·dist)` (`macro_flow.rs` `build_candidates`). So `transport_per_unit_price = config.transport_cost_per_tile_unit.0 × dist`. Do **NOT** use `transport_cost(dist, Quantity(1), rate)` — that returns a *value* (`rate·1·dist/ECONOMY_SCALE`) which floors to 0.
- **`PlannedFlow`** (`macro_flow.rs`): `{ good: GoodId, src: MarketId, dst: MarketId, q: i64, p_src: Money, p_dst: Money, dist: i64 }`. A realized flow (`q > 0`) carries everything the nudge needs (`src`, `dst`, `good`, `p_src`, `p_dst`, `dist`) — no separate `MarketDistances` lookup required.
- **Existing local nudge** (`economy/pricing.rs`, reuse its discipline): `nudge_price(price, state, k_bps, max_step_bps, floor, ceiling)` and `run_adjust_reservation_prices_at_tick(demand, supply, market_goods, config)` iterate `demand.0.values_mut()` (nudge `max_price`) and `supply.0.values_mut()` (nudge `min_price`) by each pool's own `(market,good)` excess-demand signal. Config getters: `validated_price_adjust_k_bps()`, `validated_price_adjust_max_step_bps()`, `validated_price_band() -> (floor, ceiling)`.
- **Schedule wrapper** (`systems.rs` `run_adjust_reservation_prices_system`): cadence-gated on `tick % macro_flow_interval_ticks == 0`, in `EconomySet::AdjustReservationPrices` (after `Telemetry`, before `UpdateConsumption`); surfaces `Err` as a `MarketClearFailed` ledger event. `MacroFlow` runs earlier same cadence via `run_macro_flow_system` → `run_macro_flow_at_tick(...)` (the flow settle loop is the `match settle_flow_with_receipts(...)` around `macro_flow.rs:1012`).
- **`MarketGoodState.last_settlement_price: Money`** is the post-flow recorded price at each market.
- Conservation: the new code writes only i64 `max_price`/`min_price`, reads `last_settlement_price` + the realized-flow carrier. Moves no money → `total_money` byte-invariant (#78 unaffected).

## File Structure

- **Modify** `backend/crates/sim-core/src/economy/pricing.rs` — add `nudge_price_toward_target` + `run_flow_margin_feedback_at_tick` + the coexistence skip-set; the local pass gains a `skip` set.
- **Modify** `backend/crates/sim-core/src/economy/macro_flow.rs` — populate a `RealizedFlows` carrier in `run_macro_flow_at_tick`.
- **Create/Modify** `backend/crates/sim-core/src/economy/` resource for `RealizedFlows` (place beside the other flow resources, e.g. in `market.rs` or `mod.rs` where `FlowShipments` lives) + register in `EconomyPlugin` (`economy/mod.rs`).
- **Modify** `backend/crates/sim-core/src/economy/systems.rs` — thread `RealizedFlows` into `run_macro_flow_system` (write) and `run_adjust_reservation_prices_system` (read).
- **Modify** `backend/crates/sim-core/src/economy/tests/abutopia_price_stability.rs` — the convergence regression (remove the temporary failing-as-evidence assertions; assert convergence).
- **Modify** `backend/crates/sim-core/src/economy/tests/pricing.rs` (or the pricing test module) — pure unit tests + complementarity + conservation.

---

### Task 1: `nudge_price_toward_target` (pure) + unit tests

**Files:** Modify `backend/crates/sim-core/src/economy/pricing.rs`; tests in `economy/tests/pricing.rs` (match how existing pricing tests are organized — grep `mod pricing` / existing `nudge` tests).

- [ ] **Step 1: Write the failing unit test**

In the pricing test module add:

```rust
#[test]
fn nudge_toward_target_pulls_down_when_above_and_is_speed_limited() {
    use crate::economy::pricing::nudge_price_toward_target;
    use crate::economy::Money;
    // price far ABOVE target → step is negative, capped at -max_step_bps (1%).
    let out = nudge_price_toward_target(Money(10_000), Money(1_360), 500, 100, 1, 100_000).unwrap();
    assert!(out.0 < 10_000, "above target → nudged down");
    assert!(out.0 >= 10_000 - 10_000 / 100, "down-step capped at 1% (max_step_bps=100)");
}

#[test]
fn nudge_toward_target_pulls_up_when_below_and_clamps_floor_ceiling() {
    use crate::economy::pricing::nudge_price_toward_target;
    use crate::economy::Money;
    let up = nudge_price_toward_target(Money(500), Money(1_360), 500, 100, 1, 100_000).unwrap();
    assert!(up.0 > 500, "below target → nudged up");
    // never leaves the band
    let c = nudge_price_toward_target(Money(99_999), Money(1_000_000), 500, 100, 1, 100_000).unwrap();
    assert!(c.0 <= 100_000, "clamped to ceiling");
}

#[test]
fn nudge_toward_target_at_target_is_noop() {
    use crate::economy::pricing::nudge_price_toward_target;
    use crate::economy::Money;
    let out = nudge_price_toward_target(Money(1_360), Money(1_360), 500, 100, 1, 100_000).unwrap();
    assert_eq!(out.0, 1_360, "at target → no move (gap 0)");
}
```

- [ ] **Step 2: Run → FAIL** (`nudge_price_toward_target` not defined).
Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core pricing -- --nocapture`

- [ ] **Step 3: Implement the pure function** in `pricing.rs` (mirror `nudge_price`'s discipline; signed normalized gap → speed-limited proportional step → clamp):

```rust
/// Nudge `price` toward a spatial-equilibrium `target` (Law of One Price: target =
/// source_price + rate·dist). Signed, speed-limited, clamped — the inter-market
/// arbitrage term that anchors a one-sided market's price (Samuelson, 1952; Takayama
/// & Judge, 1971). `step_bps = clamp(k_bps · gap_bps / 10_000, ±max_step_bps)` where
/// `gap_bps = (target − price)·10_000 / max(1, price)` ∈ signed bps; `new = price +
/// price·step/10_000`, clamped into `[floor, ceiling]`. Above target → pulls down
/// (the recovery force a pure-sink's local-unmet term lacks); below → pulls up.
/// Conservation-trivial (writes no money). Checked i128, floor.
pub fn nudge_price_toward_target(
    price: Money,
    target: Money,
    k_bps: i128,
    max_step_bps: i128,
    floor: i64,
    ceiling: i64,
) -> Result<Money, EconomyError> {
    let p = price.0 as i128;
    let denom = p.max(1);
    let gap_bps = ((target.0 as i128 - p) * 10_000) / denom;
    let step_bps = ((k_bps * gap_bps) / 10_000).clamp(-max_step_bps, max_step_bps);
    let delta = (p * step_bps) / 10_000;
    let raw = p + delta;
    let clamped = raw.clamp(floor as i128, ceiling as i128);
    Ok(Money(
        i64::try_from(clamped).map_err(|_| EconomyError::Overflow)?,
    ))
}
```

- [ ] **Step 4: Run → PASS.** `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core pricing -- --nocapture` (the 3 new tests pass).
- [ ] **Step 5: fmt + clippy** (scoped), then **commit**:
```bash
git add backend/crates/sim-core/src/economy/pricing.rs backend/crates/sim-core/src/economy/tests/pricing.rs
git commit -m "feat(economy): nudge_price_toward_target — spatial-LoOP arbitrage nudge (pure)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `RealizedFlows` carrier populated by MacroFlow

**Files:** add the resource (beside `FlowShipments`), register in `EconomyPlugin` (`economy/mod.rs`), populate in `macro_flow.rs::run_macro_flow_at_tick`, thread through `systems.rs::run_macro_flow_system`.

- [ ] **Step 1: Define the resource.** Where `FlowShipments`/`NextShipmentId` are defined, add:

```rust
/// Per-cadence record of realized macro-flows (q > 0). Cleared and repopulated each
/// MacroFlow run; NOT persisted (transient signal for the flow-margin price nudge).
#[derive(bevy_ecs::prelude::Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct RealizedFlows(pub Vec<RealizedFlow>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealizedFlow {
    pub src: MarketId,
    pub dst: MarketId,
    pub good: GoodId,
    pub p_src: Money,
    pub p_dst: Money,
    pub dist: i64,
}
```
Export both from `economy/mod.rs` (match how `FlowShipments` is re-exported).

- [ ] **Step 2: Register in `EconomyPlugin`** (`economy/mod.rs`, next to `world.insert_resource(FlowShipments::default())`):
```rust
world.insert_resource(RealizedFlows::default());
```

- [ ] **Step 3: Populate in `run_macro_flow_at_tick`.** Add a `realized: &mut RealizedFlows` parameter; at the TOP of the function clear it (`realized.0.clear();`); in the flow settle loop (the `match settle_flow_with_receipts(...)` around `macro_flow.rs:1012`), after a SUCCESSFUL settle of a flow with `flow.q > 0`, push:
```rust
realized.0.push(crate::economy::RealizedFlow {
    src: flow.src, dst: flow.dst, good: flow.good,
    p_src: flow.p_src, p_dst: flow.p_dst, dist: flow.dist,
});
```
(`flow` is the `PlannedFlow` being settled; confirm the binding name in that loop.)

- [ ] **Step 4: Thread through `run_macro_flow_system`** (`systems.rs`): add `mut realized: ResMut<RealizedFlows>` param and pass `&mut realized` to `run_macro_flow_at_tick(...)`. Fix all other callers of `run_macro_flow_at_tick` (tests) to pass a `&mut RealizedFlows::default()`.

- [ ] **Step 5: Build green + commit.**
Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core macro_flow -- --nocapture` (existing macro_flow tests still pass; fix signatures).
```bash
git add -A && git commit -m "feat(economy): RealizedFlows carrier populated by MacroFlow (per-cadence, transient)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Flow-margin feedback pass + coexistence + wiring

**Files:** `economy/pricing.rs` (new pass + skip-set), `systems.rs` (wire into `run_adjust_reservation_prices_system`), tests.

- [ ] **Step 1: Write the complementarity + conservation tests** (pricing test module):

```rust
#[test]
fn flow_margin_skips_edges_with_no_realized_flow() {
    // With an EMPTY RealizedFlows, the flow-margin pass must make NO price change.
    use crate::economy::pricing::run_flow_margin_feedback_at_tick;
    use crate::economy::{DemandPools, SupplyPools, MarketGoods, RealizedFlows, EconomyConfig};
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();
    // (seed one demand pool + its MarketGoodState; see seed_two_extractor_economy pattern)
    // ... build a single demand pool at market 9002 good 4 with max_price 5000 ...
    let before = /* that pool's max_price */;
    let goods = MarketGoods::default();
    let realized = RealizedFlows::default(); // no active edges
    run_flow_margin_feedback_at_tick(&mut demand, &mut supply, &goods, &realized, &EconomyConfig::default()).unwrap();
    assert_eq!(/* pool max_price */, before, "no realized flow → no nudge (complementarity)");
}
```
(Fill the seed using the existing `economy/tests` pool-construction pattern — grep `DemandPool {` in tests for a literal.) Add a conservation test that runs the full schedule N ticks with the new pass active and asserts `total_money` byte-invariant (extend `conservation_full_plugin_multi_tick`).

- [ ] **Step 2: Run → FAIL** (function not defined).

- [ ] **Step 3: Implement the flow-margin pass + coexistence** in `pricing.rs`:

```rust
use crate::economy::RealizedFlows;
use std::collections::BTreeSet;

/// Anchor flow-coupled (one-sided) markets to the spatial Law of One Price. For each
/// realized flow S→D (good g) this cadence: target_D = p_src + rate·dist (the landed
/// cost), target_S = p_dst − rate·dist. Nudge D's demand pools' max_price toward
/// target_D and S's supply pools' min_price toward target_S, damped/speed-limited/
/// clamped. The fixpoint is p_D − p_S = rate·dist (Samuelson, 1952). Complementarity:
/// only edges with realized flow (q>0) are touched — dormant routes are NOT forced to
/// equality. Conservation-trivial (no money moved). Keys-first/deterministic.
pub fn run_flow_margin_feedback_at_tick(
    demand: &mut DemandPools,
    supply: &mut SupplyPools,
    _market_goods: &MarketGoods,
    realized: &RealizedFlows,
    config: &EconomyConfig,
) -> Result<(), EconomyError> {
    let k_bps = config.validated_price_adjust_k_bps()?;
    let max_step_bps = config.validated_price_adjust_max_step_bps()?;
    let (floor, ceiling) = config.validated_price_band()?;
    let rate = config.transport_cost_per_tile_unit.0 as i128;

    for f in realized.0.iter() {
        let t = rate * f.dist as i128; // per-unit transport IN PRICE UNITS (rate·dist)
        let target_d = Money(i64::try_from((f.p_src.0 as i128 + t).clamp(floor as i128, ceiling as i128)).map_err(|_| EconomyError::Overflow)?);
        let target_s = Money(i64::try_from((f.p_dst.0 as i128 - t).clamp(floor as i128, ceiling as i128)).map_err(|_| EconomyError::Overflow)?);
        for pool in demand.0.values_mut() {
            if pool.market == f.dst && pool.good == f.good {
                pool.max_price = nudge_price_toward_target(pool.max_price, target_d, k_bps, max_step_bps, floor, ceiling)?;
            }
        }
        for pool in supply.0.values_mut() {
            if pool.market == f.src && pool.good == f.good {
                pool.min_price = nudge_price_toward_target(pool.min_price, target_s, k_bps, max_step_bps, floor, ceiling)?;
            }
        }
    }
    Ok(())
}

/// The set of (market, good) pairs governed by the flow-margin term this cadence
/// (so the local-unmet tâtonnement skips them — margin anchors, local doesn't fight).
pub fn flow_coupled_keys(realized: &RealizedFlows) -> (BTreeSet<(u32, u32)>, BTreeSet<(u32, u32)>) {
    let mut demand_keys = BTreeSet::new();
    let mut supply_keys = BTreeSet::new();
    for f in realized.0.iter() {
        demand_keys.insert((f.dst.0, f.good.0));
        supply_keys.insert((f.src.0, f.good.0));
    }
    (demand_keys, supply_keys)
}
```

Then add a `skip` parameter to `run_adjust_reservation_prices_at_tick` so the local pass skips flow-coupled pools:

```rust
pub fn run_adjust_reservation_prices_at_tick(
    demand: &mut DemandPools,
    supply: &mut SupplyPools,
    market_goods: &MarketGoods,
    config: &EconomyConfig,
    skip_demand: &BTreeSet<(u32, u32)>,
    skip_supply: &BTreeSet<(u32, u32)>,
) -> Result<(), EconomyError> {
    // ... unchanged, but: `if skip_demand.contains(&(pool.market.0, pool.good.0)) { continue; }`
    //                     `if skip_supply.contains(&(pool.market.0, pool.good.0)) { continue; }`
    //     before each nudge_price call.
}
```
(Update its existing unit tests to pass empty `&BTreeSet::new()` for both, preserving current behavior.)

- [ ] **Step 4: Wire the system** (`systems.rs::run_adjust_reservation_prices_system`): add `realized: Res<RealizedFlows>`; compute `let (sd, ss) = pricing::flow_coupled_keys(&realized);`, call `run_adjust_reservation_prices_at_tick(&mut demand, &mut supply, &market_goods, &config, &sd, &ss)`, THEN `run_flow_margin_feedback_at_tick(&mut demand, &mut supply, &market_goods, &realized, &config)` — surface either `Err` as the existing `MarketClearFailed` audit event.

- [ ] **Step 5: Run → PASS** (complementarity + conservation tests). fmt + clippy. **Commit.**

---

### Task 4: Abutopia convergence regression

**Files:** `economy/tests/abutopia_price_stability.rs`.

- [ ] **Step 1: Rewrite the assertions** for convergence (the test that previously FAILED at the ceiling now asserts convergence to LoOP). Replace the `peak_price_9002 < ceiling/10` body with: run ~2000 ticks, then assert (a) `total_money` byte-invariant each tick (keep); (b) every price in-band (keep); (c) 9002's final `ewma_reference_price` converges near `p_9001 + rate·dist` and is **well below the ceiling** (e.g. `< 10_000`); (d) consumption sustained in the last quarter (`> 0`). Print `final_price_9002`, `final_price_9001`, and `rate·dist` so the convergence is legible. Tune the band from the printed actuals (must stay a real "not-ceiling" assertion).

- [ ] **Step 2: Run → PASS.** `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core abutopia_price_stability -- --nocapture`. Expected: 9002 converges to ≈ `p_9001 + 860`, in-band, consuming. If it does NOT converge, the flow-margin pass isn't governing 9002 — debug the RealizedFlows population + the coexistence skip (is (9002,good) in the demand skip-set? is the edge in RealizedFlows?). Do not weaken the assertion to force green.

- [ ] **Step 3: Commit.**

---

### Task 5: Whole-system gate + PR

- [ ] **Step 1: Rust workspace gate** (one at a time, via cargo-serial): `fmt --all -- --check` (0); `clippy --workspace --all-targets -D warnings` (clean); `test --workspace` (all pass incl. the new + the abutopia convergence test, sim-server unaffected).
- [ ] **Step 2: Frontend gate** (symlink `node_modules` from main worktree if absent): `npm run typecheck`, `npm test`, `npm run build` — all clean (no frontend change).
- [ ] **Step 3: e2e** `CORS_ALLOWED_ORIGINS="http://127.0.0.1:5173" npm run test:e2e` → render-smoke 2/2 (no wire change).
- [ ] **Step 4: finishing-a-development-branch** → push + PR against `main`. Body: cite the spec (literature-grounded, APA7); state it implements the free-prices spec's deferred one-sided LoOP convergence; no migration; the abutopia convergence test is the proof. Wait for CI green, squash-merge, clean up.

---

## Self-Review

**1. Spec coverage:** `nudge_price_toward_target` → Task 1 (spec "Mechanism"). RealizedFlows + active-flow gate → Task 2 (spec "active-flow signal"). Flow-margin pass + complementarity gate + margin-anchor coexistence → Task 3 (spec "Mechanism" + "Coexistence rule"). Conservation/determinism → Tasks 1+3 (i128/clamp/keys-first, no money). Convergence regression → Task 4 (spec "Testing"). Gate/PR → Task 5. All spec sections covered.

**2. Placeholder scan:** Full code for the two pure functions + the coexistence skip; precise step-by-step for the resource/MacroFlow/schedule wiring (the implementer confirms the settle-loop binding name + the pool-construction literal in tests — these are verify-points, not placeholders). The transport-per-unit is pinned to `rate·dist` (not the floored `transport_cost(dist,1,rate)`).

**3. Type consistency:** `RealizedFlow{src,dst,good,p_src,p_dst,dist}` used consistently across Tasks 2–3; `nudge_price_toward_target` signature identical in Task 1 def and Task 3 calls; `(u32,u32)` skip-keys (`market.0`,`good.0`) consistent between `flow_coupled_keys` and `run_adjust_reservation_prices_at_tick`. Transport term `rate·dist` consistent with the spec.
