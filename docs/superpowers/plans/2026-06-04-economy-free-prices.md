# Free / Market-Clearing Prices Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make pool reservation prices scarcity-responsive — a damped tâtonnement nudge reads the (currently dead) `unmet−unsold` excess-demand telemetry and moves `DemandPool.max_price` + `SupplyPool.min_price` toward scarcity, bounded by an intensity clamp + a 1%/interval speed limit + slow cadence + absolute guardrails.

**Architecture:** One new pure function (`pricing.rs::run_adjust_reservation_prices_at_tick`) + one new schedule set (`EconomySet::AdjustReservationPrices`, between `Telemetry` and `UpdateConsumption`) + one cadence-gated system wrapper + four `EconomyConfig` knobs. Mutates only existing already-persisted i64 price fields ⇒ **no new snapshot field, no DELETE migration**. Conservation-trivial (prices are order parameters, not money). Stability via speed-limit + slow cadence (decoupled from the ~10-tick EWMA loop).

**Tech Stack:** Rust (bevy_ecs 0.18), `sim-core`; fixed-point i64/i128, floor, checked; TDD via `cargo test`.

**Spec:** `docs/superpowers/specs/2026-06-04-economy-free-prices-design.md`

---

## Verified Facts (pinned against the real code — do not re-derive)

**`EconomyConfig` (`economy/systems.rs:42-118`):** `#[derive(Resource, Debug, Clone, Copy, PartialEq)]`. NOT persisted (compiled-in defaults, frozen post-seed). Has `ewma_alpha_bps: u16 = 2_000`, `macro_flow_interval_ticks: u64 = 10`, `settlement_policy`, `labor_share_bps: u16`, `dividend_share_bps: u16 = 10_000`, etc. The validated-getter pattern (`systems.rs:80-95`):
```rust
pub fn validated_labor_share_bps(&self) -> Result<i128, crate::economy::EconomyError> {
    if self.labor_share_bps > 10_000 { return Err(crate::economy::EconomyError::InvalidOrder); }
    Ok(self.labor_share_bps as i128)
}
```
`Default for EconomyConfig` is a struct literal at `systems.rs:98-118` (append new fields there).

**`EconomyError` (`economy/money.rs:1-12`):** variants include `Overflow`, `ZeroPrice`, `InsufficientFunds`, `InvalidOrder`.

**`Money` / `Quantity`:** newtypes over `i64`; field `.0`. `Money(i64)`, `Quantity(i64)`.

**Pools (`economy/pools.rs`):** `DemandPool { actor, market: MarketId, good: GoodId, …, max_price: Money, … }` (line 12-…); `SupplyPool { actor, market: MarketId, good: GoodId, …, min_price: Money, … }` (line 42-…). `DemandPools(pub BTreeMap<EconomicActorId, DemandPool>)` (53); `SupplyPools(pub BTreeMap<EconomicActorId, SupplyPool>)` (56). **VERIFIED persisted:** both structs `#[derive(…, serde::Serialize, serde::Deserialize)]` (pools.rs:11/41) with NO `#[serde(skip)]` on any field; `EconomyPersistSnapshot { demand_pools: Vec<(EconomicActorId, DemandPool)>, supply_pools: Vec<(EconomicActorId, SupplyPool)> }` (persist.rs:44-45) extract/apply the WHOLE pool incl. `max_price`/`min_price` (persist.rs:100-101 extract, 135-136 apply). ⇒ mutating `max_price`/`min_price` needs NO new snapshot field, and nudged prices **survive restore**.

**Market state (`economy/market.rs`):** `MarketGoodKey { market: MarketId, good: GoodId }` (18-21); `MarketGoodState { key, …, unmet_demand_last_tick: Quantity (29), unsold_supply_last_tick: Quantity (30), … }`; `MarketGoods(pub BTreeMap<MarketGoodKey, MarketGoodState>)` (62). `unmet`/`unsold` are written fresh each tick (auction + macro_flow) and read by ZERO production code.

**Schedule (`economy/systems.rs:120-167`):** `install_systems` calls `schedule.configure_sets((EconomySet::ResetReceipts, …, Telemetry, UpdateConsumption).chain())` — **the `.chain()` enforces set order**. All systems registered in ONE `add_systems((…).before(crate::mobility::systems::tick_increment_system))`. The relevant tail: `EconomySet::Telemetry` ← `update_market_telemetry_system`; `EconomySet::UpdateConsumption` ← `run_consumption_update_system`. The `EconomySet` enum (around `systems.rs:20-40`) lists the variants.

**Cadence-gate precedent (`run_transport_rebate_system`, systems.rs:502-517):**
```rust
if config.macro_flow_interval_ticks == 0 || !tick.0.is_multiple_of(config.macro_flow_interval_ticks) { return; }
```

**Err-surfacing wrapper precedent (`run_macro_flow_system`, systems.rs:556-585):**
```rust
if let Err(reason) = run_macro_flow_at_tick(…) {
    ledger.0.push(EconomyEvent::MarketClearFailed { market: MarketId(0), good: GoodId(0), reason });
}
```
(NOTE: `update_market_telemetry_system` uses `let _ = …` — a PRE-EXISTING wart; do NOT copy that; use the macro_flow `if let Err` audit pattern.)

**Module wiring (`economy/mod.rs` + `economy/tests/mod.rs`):** production modules are declared `pub mod <name>;` (e.g. `pub mod pools;` at mod.rs:16) with `pub use <name>::*;` re-exports (e.g. `pub use pools::*;` at mod.rs:40); tests live in a SUBDIRECTORY gated by `#[cfg(test)] mod tests;` (mod.rs:93), and `economy/tests/mod.rs` lists each test file as `mod <name>;` (e.g. `mod pools;`). So: add `pub mod pricing;` + `pub use pricing::*;` to `mod.rs`, and add `mod pricing;` to `economy/tests/mod.rs` — NOT an inline `#[cfg(test)] mod tests { … }` body (which does not exist).

**Tick double-increment in plugin tests:** the multi-tick conservation/steady-state tests loop `schedule.run(&mut world); world.resource_mut::<Tick>().0 += 1;`. `0.is_multiple_of(10) == true`, so the nudge fires at tick 0, 10, 20, … (same as the transport rebate).

**Cargo (MANDATORY — never bare cargo; isolated target + serial lock):**
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core <TESTNAME>
```
from `/Users/ramonfuglister/Coding/abutown-vtraders`. Never `--workspace --all-targets` during iteration. `mkdir -p /tmp/abutown-vtraders-tmp` once if missing. **fmt uses `--all`:** `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check`.

---

## Sub-Slice A — Config knobs + pure nudge core

### Task A1: `EconomyConfig` price-adjust knobs + validated getters

**Files:** Modify `backend/crates/sim-core/src/economy/systems.rs` (struct + Default + impl); Test `backend/crates/sim-core/src/economy/tests/systems.rs`.

- [ ] **Step 1: Write the failing test** (append to `tests/systems.rs`):

```rust
#[test]
fn price_adjust_config_defaults_and_validation() {
    use crate::economy::systems::EconomyConfig;
    use crate::economy::Money;
    let c = EconomyConfig::default();
    assert_eq!(c.price_adjust_k_bps, 500);
    assert_eq!(c.price_adjust_max_step_bps, 100);
    assert_eq!(c.price_floor, Money(1));
    assert_eq!(c.price_ceiling, Money(100_000));
    // Validated getters accept the defaults...
    assert_eq!(c.validated_price_adjust_k_bps().unwrap(), 500);
    assert_eq!(c.validated_price_adjust_max_step_bps().unwrap(), 100);
    assert_eq!(c.validated_price_band().unwrap(), (1, 100_000));
    // Inclusive boundary == 10_000 PASSES (mirrors validated_labor_share_bps).
    let edge_k = EconomyConfig { price_adjust_k_bps: 10_000, ..c };
    assert_eq!(edge_k.validated_price_adjust_k_bps().unwrap(), 10_000);
    // ...and reject out-of-band config (NO-FALLBACK: honest Err, no silent clamp).
    let bad_k = EconomyConfig { price_adjust_k_bps: 10_001, ..c };
    assert!(bad_k.validated_price_adjust_k_bps().is_err());
    let bad_step = EconomyConfig { price_adjust_max_step_bps: 10_001, ..c };
    assert!(bad_step.validated_price_adjust_max_step_bps().is_err());
    let bad_floor0 = EconomyConfig { price_floor: Money(0), ..c };
    assert!(bad_floor0.validated_price_band().is_err(), "floor must be > 0 (else ZeroPrice)");
    let bad_order = EconomyConfig { price_floor: Money(100_000), price_ceiling: Money(1), ..c };
    assert!(bad_order.validated_price_band().is_err(), "floor must be < ceiling");
}
```

- [ ] **Step 2: Run it — verify it FAILS** (fields/getters don't exist):

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core price_adjust_config_defaults_and_validation
```
Expected: FAIL — `no field price_adjust_k_bps` / `no method validated_price_band`.

- [ ] **Step 3: Add the four fields** to `struct EconomyConfig` (after `dividend_share_bps`, before the closing `}` at ~systems.rs:73):

```rust
    /// Tâtonnement gain (basis points) applied to the normalized excess-demand intensity
    /// when nudging reservation prices. Default 500 = 5%. VALIDATED `0..=10_000`.
    pub price_adjust_k_bps: u16,
    /// Hard per-interval speed limit on a reservation-price move (basis points of the
    /// current price). Default 100 = 1%/interval — the load-bearing anti-oscillation guard.
    /// VALIDATED `0..=10_000`.
    pub price_adjust_max_step_bps: u16,
    /// Absolute lower guardrail for any reservation price (MUST be > 0 so a price never
    /// reaches 0 and trips ZeroPrice). Default Money(1).
    pub price_floor: Money,
    /// Absolute upper guardrail for any reservation price. Default Money(100_000).
    pub price_ceiling: Money,
```

- [ ] **Step 4: Add the defaults** to the `Default` literal (after `dividend_share_bps: 10_000,` at ~systems.rs:115):

```rust
            price_adjust_k_bps: 500,
            price_adjust_max_step_bps: 100,
            price_floor: Money(1),
            price_ceiling: Money(100_000),
```

- [ ] **Step 5: Add the validated getters** to `impl EconomyConfig` (after `validated_dividend_share_bps`, ~systems.rs:95):

```rust
    /// `price_adjust_k_bps` as i128, refusing `> 10_000`. Boundary `== 10_000` allowed.
    pub fn validated_price_adjust_k_bps(&self) -> Result<i128, crate::economy::EconomyError> {
        if self.price_adjust_k_bps > 10_000 {
            return Err(crate::economy::EconomyError::InvalidOrder);
        }
        Ok(self.price_adjust_k_bps as i128)
    }
    /// `price_adjust_max_step_bps` as i128, refusing `> 10_000`.
    pub fn validated_price_adjust_max_step_bps(&self) -> Result<i128, crate::economy::EconomyError> {
        if self.price_adjust_max_step_bps > 10_000 {
            return Err(crate::economy::EconomyError::InvalidOrder);
        }
        Ok(self.price_adjust_max_step_bps as i128)
    }
    /// `(price_floor, price_ceiling)` as i64s, refusing `floor <= 0` or `floor >= ceiling`
    /// (a config bug that would allow a 0/negative price or an empty guardrail band).
    pub fn validated_price_band(&self) -> Result<(i64, i64), crate::economy::EconomyError> {
        if self.price_floor.0 <= 0 || self.price_floor.0 >= self.price_ceiling.0 {
            return Err(crate::economy::EconomyError::InvalidOrder);
        }
        Ok((self.price_floor.0, self.price_ceiling.0))
    }
```

- [ ] **Step 6: Run it — verify it PASSES**

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core price_adjust_config_defaults_and_validation
```
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-core/src/economy/systems.rs backend/crates/sim-core/src/economy/tests/systems.rs
git commit -m "feat(economy): add price-adjust EconomyConfig knobs + validated getters"
```

### Task A2: pure `run_adjust_reservation_prices_at_tick` core (`pricing.rs`)

**Files:** Create `backend/crates/sim-core/src/economy/pricing.rs`; Modify `backend/crates/sim-core/src/economy/mod.rs` (declare + re-export module, add test module); Create `backend/crates/sim-core/src/economy/tests/pricing.rs`.

- [ ] **Step 1: Create `pricing.rs`** with the pure core:

```rust
//! Free / market-clearing prices: a damped tâtonnement nudge that moves pool reservation
//! prices toward scarcity. Pure over its refs (no `World`). Conservation-trivial — it writes
//! only i64 price fields and reads telemetry; it NEVER touches money/inventory. Deterministic:
//! i128 intermediates, floor division, keys-first BTreeMap iteration.

use crate::economy::{
    DemandPools, EconomyConfig, EconomyError, MarketGoodKey, MarketGoodState, MarketGoods, Money,
    SupplyPools,
};

/// Normalized excess-demand intensity for one market-good, in basis points ∈ [-10_000, +10_000].
/// `net = unmet − unsold`; `scale = max(1, unmet + unsold)`; `x = net*10_000/scale`. Since
/// `|net| <= unmet+unsold = scale`, `|x| <= 10_000`. i128, floor (truncates toward zero).
fn intensity_bps(state: &MarketGoodState) -> i128 {
    let unmet = state.unmet_demand_last_tick.0 as i128;
    let unsold = state.unsold_supply_last_tick.0 as i128;
    let net = unmet - unsold;
    let scale = (unmet + unsold).max(1);
    (net * 10_000) / scale
}

/// Nudge one reservation `price` by the market's scarcity intensity, speed-limited and clamped.
/// `step_bps = clamp(k_bps * x_bps / 10_000, ±max_step_bps)`; `new = price + price*step/10_000`,
/// then clamped into `[floor, ceiling]`. Shortage (x>0) raises, glut (x<0) lowers. Checked i128.
fn nudge_price(
    price: Money,
    state: &MarketGoodState,
    k_bps: i128,
    max_step_bps: i128,
    floor: i64,
    ceiling: i64,
) -> Result<Money, EconomyError> {
    let x_bps = intensity_bps(state);
    let step_bps = ((k_bps * x_bps) / 10_000).clamp(-max_step_bps, max_step_bps);
    let delta = ((price.0 as i128) * step_bps) / 10_000;
    let raw = (price.0 as i128) + delta;
    let clamped = raw.clamp(floor as i128, ceiling as i128);
    Ok(Money(i64::try_from(clamped).map_err(|_| EconomyError::Overflow)?))
}

/// For every demand pool, nudge `max_price`; for every supply pool, nudge `min_price` — each by
/// the excess-demand signal of ITS OWN `(market, good)` state (shortage→up, glut→down: both walls
/// translate the same direction). A pool whose `(market, good)` has no `MarketGoodState` yet (a
/// market that has never cleared) has NO scarcity signal this interval, so its price correctly
/// stays put — this is "no data, no action", NOT a defaulted price. Keys-first (BTreeMap) → deterministic.
pub fn run_adjust_reservation_prices_at_tick(
    demand: &mut DemandPools,
    supply: &mut SupplyPools,
    market_goods: &MarketGoods,
    config: &EconomyConfig,
) -> Result<(), EconomyError> {
    let k_bps = config.validated_price_adjust_k_bps()?;
    let max_step_bps = config.validated_price_adjust_max_step_bps()?;
    let (floor, ceiling) = config.validated_price_band()?;

    for pool in demand.0.values_mut() {
        let key = MarketGoodKey { market: pool.market, good: pool.good };
        if let Some(state) = market_goods.0.get(&key) {
            pool.max_price = nudge_price(pool.max_price, state, k_bps, max_step_bps, floor, ceiling)?;
        }
    }
    for pool in supply.0.values_mut() {
        let key = MarketGoodKey { market: pool.market, good: pool.good };
        if let Some(state) = market_goods.0.get(&key) {
            pool.min_price = nudge_price(pool.min_price, state, k_bps, max_step_bps, floor, ceiling)?;
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Wire the module.** In `economy/mod.rs`: add `pub mod pricing;` next to the other `pub mod <name>;` declarations (e.g. after `pub mod pools;` at mod.rs:16) and `pub use pricing::*;` in the re-export block (after `pub use pools::*;` at mod.rs:40). In `economy/tests/mod.rs`: add `mod pricing;` to the test-file list (alongside `mod pools;`) — this is the real test-wiring pattern (a `tests/` subdirectory gated by `#[cfg(test)] mod tests;` at mod.rs:93), NOT an inline `#[cfg(test)] mod tests { … }` body.

- [ ] **Step 3: Create `tests/pricing.rs`** with the unit tests:

```rust
use crate::economy::pricing::run_adjust_reservation_prices_at_tick;
use crate::economy::systems::EconomyConfig;
use crate::economy::{
    DemandPool, DemandPools, EconomicActorId, GOOD_TOOLS, MarketGoodKey, MarketGoodState,
    MarketGoods, MarketId, Money, Quantity, SupplyPool, SupplyPools,
};
use std::collections::BTreeMap;

fn state(market: MarketId, unmet: i64, unsold: i64) -> MarketGoodState {
    let key = MarketGoodKey { market, good: GOOD_TOOLS };
    let mut s = MarketGoodState::new(key);
    s.unmet_demand_last_tick = Quantity(unmet);
    s.unsold_supply_last_tick = Quantity(unsold);
    s
}
fn demand_pool(actor: u64, market: MarketId, max_price: i64) -> DemandPool {
    DemandPool {
        actor: EconomicActorId(actor), market, good: GOOD_TOOLS,
        desired_qty_per_tick: Quantity(10), max_price: Money(max_price),
        urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1,
        last_generated_tick: None, last_consumed_tick: None,
        income_last_tick: Money::ZERO, mpc_bps: 8_000, autonomous: Money(5_000),
    }
}
fn supply_pool(actor: u64, market: MarketId, min_price: i64) -> SupplyPool {
    SupplyPool {
        actor: EconomicActorId(actor), market, good: GOOD_TOOLS,
        offered_qty_per_tick: Quantity(10), min_price: Money(min_price),
        interval_ticks: 1, last_generated_tick: None,
    }
}

#[test]
fn shortage_raises_glut_lowers_balanced_unchanged_both_walls_translate() {
    let m = MarketId(1);
    let cfg = EconomyConfig::default(); // k=500, max_step=100, floor=1, ceiling=100_000
    let run = |unmet: i64, unsold: i64| {
        let mut d = DemandPools(BTreeMap::from([(EconomicActorId(1), demand_pool(1, m, 2_000))]));
        let mut s = SupplyPools(BTreeMap::from([(EconomicActorId(2), supply_pool(2, m, 500))]));
        let mut g = MarketGoods::default();
        g.0.insert(MarketGoodKey { market: m, good: GOOD_TOOLS }, state(m, unmet, unsold));
        run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
        (d.0[&EconomicActorId(1)].max_price.0, s.0[&EconomicActorId(2)].min_price.0)
    };
    // Shortage (unmet>unsold): BOTH walls up.
    let (max_up, min_up) = run(100, 0);
    assert!(max_up > 2_000 && min_up > 500, "shortage raises both walls; got max={max_up} min={min_up}");
    // Glut (unsold>unmet): BOTH walls down.
    let (max_dn, min_dn) = run(0, 100);
    assert!(max_dn < 2_000 && min_dn < 500, "glut lowers both walls; got max={max_dn} min={min_dn}");
    // Balanced (net=0): unchanged (system quiescent at equilibrium).
    let (max_eq, min_eq) = run(50, 50);
    assert_eq!((max_eq, min_eq), (2_000, 500), "no net imbalance → no nudge");
    // Band stays clearable in every case (min < max).
    for (mx, mn) in [(max_up, min_up), (max_dn, min_dn)] { assert!(mn < mx, "min<max preserved"); }
}

#[test]
fn step_is_speed_limited_regardless_of_signal_magnitude() {
    let m = MarketId(1);
    let cfg = EconomyConfig::default(); // max_step=100 bps = 1%
    let mut d = DemandPools(BTreeMap::from([(EconomicActorId(1), demand_pool(1, m, 10_000))]));
    let mut s = SupplyPools::default();
    let mut g = MarketGoods::default();
    g.0.insert(MarketGoodKey { market: m, good: GOOD_TOOLS }, state(m, 1_000_000, 0)); // huge shortage
    run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
    // x_bps saturates at +10_000; k*x/10_000 = 500 bps; clamped to max_step 100 bps = 1% of 10_000 = 100.
    assert_eq!(d.0[&EconomicActorId(1)].max_price.0, 10_100, "1%/interval cap binds for any huge imbalance");
}

#[test]
fn guardrails_clamp_and_never_zero() {
    let m = MarketId(1);
    // Tight ceiling: a price near the ceiling cannot exceed it under shortage.
    let cfg = EconomyConfig { price_ceiling: Money(2_010), ..EconomyConfig::default() };
    let mut d = DemandPools(BTreeMap::from([(EconomicActorId(1), demand_pool(1, m, 2_000))]));
    let mut s = SupplyPools(BTreeMap::from([(EconomicActorId(2), supply_pool(2, m, 500))]));
    let mut g = MarketGoods::default();
    g.0.insert(MarketGoodKey { market: m, good: GOOD_TOOLS }, state(m, 1_000, 0));
    run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
    assert!(d.0[&EconomicActorId(1)].max_price.0 <= 2_010, "clamped to ceiling");
    assert!(s.0[&EconomicActorId(2)].min_price.0 >= 1, "never below floor (>0)");
}

#[test]
fn no_state_means_no_nudge_not_a_default() {
    let m = MarketId(1);
    let cfg = EconomyConfig::default();
    let mut d = DemandPools(BTreeMap::from([(EconomicActorId(1), demand_pool(1, m, 2_000))]));
    let mut s = SupplyPools::default();
    let g = MarketGoods::default(); // no state for (m, TOOLS)
    run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
    assert_eq!(d.0[&EconomicActorId(1)].max_price.0, 2_000, "no signal → price unchanged (not defaulted)");
}

#[test]
fn invalid_config_is_honest_err_no_silent_default() {
    let m = MarketId(1);
    let cfg = EconomyConfig { price_floor: Money(0), ..EconomyConfig::default() };
    let mut d = DemandPools(BTreeMap::from([(EconomicActorId(1), demand_pool(1, m, 2_000))]));
    let mut s = SupplyPools::default();
    let mut g = MarketGoods::default();
    g.0.insert(MarketGoodKey { market: m, good: GOOD_TOOLS }, state(m, 100, 0));
    assert!(run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).is_err(), "floor<=0 → Err");
    assert_eq!(d.0[&EconomicActorId(1)].max_price.0, 2_000, "no partial mutation on config Err");
}

#[test]
fn nudge_is_deterministic() {
    let m = MarketId(1);
    let cfg = EconomyConfig::default();
    let run = || {
        let mut d = DemandPools(BTreeMap::from([
            (EconomicActorId(9), demand_pool(9, m, 2_000)),
            (EconomicActorId(2), demand_pool(2, m, 2_000)),
        ]));
        let mut s = SupplyPools(BTreeMap::from([(EconomicActorId(5), supply_pool(5, m, 500))]));
        let mut g = MarketGoods::default();
        g.0.insert(MarketGoodKey { market: m, good: GOOD_TOOLS }, state(m, 70, 10));
        run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
        (d.0[&EconomicActorId(9)].max_price.0, d.0[&EconomicActorId(2)].max_price.0, s.0[&EconomicActorId(5)].min_price.0)
    };
    assert_eq!(run(), run());
}
```

- [ ] **Step 4: Run the pricing tests — verify they PASS**

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests::pricing
```
Expected: PASS (6 tests). If `shortage_raises…` math is off, re-derive: shortage(100,0)→x=10000→step=clamp(500,±100)=100→max 2000·1.01=2020, min 500·1.01=505. Glut(0,100)→x=−10000→step=−100→max 1980, min 495.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/pricing.rs backend/crates/sim-core/src/economy/mod.rs backend/crates/sim-core/src/economy/tests/pricing.rs
git commit -m "feat(economy): pure run_adjust_reservation_prices_at_tick (tatonnement nudge core)"
```

---

## Sub-Slice B — Schedule wiring

### Task B1: `EconomySet::AdjustReservationPrices` + cadence-gated system

**Files:** Modify `backend/crates/sim-core/src/economy/systems.rs` (enum + chain + wrapper + registration); Test `backend/crates/sim-core/src/economy/tests/systems.rs`.

- [ ] **Step 1: Write the failing ordering+cadence test** (append to `tests/systems.rs`).

**CRITICAL SETUP NOTE (plan-review blocker fix):** the full economy schedule registers every system `.before(crate::mobility::systems::tick_increment_system)` and many take `Res<Tick>` — both HARD dependencies provided by **MobilityPlugin**, not EconomyPlugin. So a `schedule.run` with EconomyPlugin alone PANICS on missing `Tick`. Every schedule-running economy test installs **CorePlugin + MobilityPlugin + EconomyPlugin** (see `conservation.rs::conservation_full_plugin_multi_tick`); mirror that exactly. Use a genuine demand>supply imbalance at a single co-located market so the auction produces a real `unmet_demand_last_tick` the nudge can read (manually-set `unmet` would be overwritten by `ClearMarkets` before the nudge runs).

```rust
#[test]
fn adjust_reservation_prices_fires_on_cadence_boundary_only() {
    use crate::economy::systems::{run_adjust_reservation_prices_system, EconomySet};
    use crate::economy::{
        AccountBook, DemandPool, DemandPools, EconomicActorId, GOOD_TOOLS, HouseholdSector,
        InventoryBook, MarketGoodKey, MarketGoodState, MarketGoods, MarketId, MarketSite, Markets,
        Money, Quantity, SupplyPool, SupplyPools,
    };
    use crate::economy::EconomyPlugin;
    use crate::mobility::resources::Tick;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    use bevy_ecs::prelude::*;
    use std::collections::BTreeMap;

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let m = MarketId(1);
    let consumer = EconomicActorId(8_002);
    let supplier = EconomicActorId(8_001);
    // Demand 10 > supply 5 at the SAME market → after ClearMarkets, unmet_demand_last_tick = 5.
    world.resource_mut::<DemandPools>().0.insert(
        consumer,
        DemandPool {
            actor: consumer, market: m, good: GOOD_TOOLS,
            desired_qty_per_tick: Quantity(10), max_price: Money(2_000),
            urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1,
            last_generated_tick: None, last_consumed_tick: None,
            income_last_tick: Money::ZERO, mpc_bps: 8_000, autonomous: Money(5_000),
        },
    );
    world.resource_mut::<SupplyPools>().0.insert(
        supplier,
        SupplyPool {
            actor: supplier, market: m, good: GOOD_TOOLS,
            offered_qty_per_tick: Quantity(5), min_price: Money(500),
            interval_ticks: 1, last_generated_tick: None,
        },
    );
    world.resource_mut::<AccountBook>().deposit(consumer, Money(10_000_000)).unwrap();
    world.resource_mut::<InventoryBook>().deposit(supplier, GOOD_TOOLS, Quantity(1_000_000)).unwrap();
    world.resource_mut::<Markets>().0.insert(
        m, MarketSite { id: m, node_id: crate::routing::NodeId(0), name: "M1".to_string() },
    );
    world.insert_resource(HouseholdSector { population: 1_000_000, pool_weights: BTreeMap::from([(consumer, 1_i64)]) });
    {
        let key = MarketGoodKey { market: m, good: GOOD_TOOLS };
        let mut g = world.resource_mut::<MarketGoods>();
        let st = g.0.entry(key).or_insert_with(|| MarketGoodState::new(key));
        st.ewma_reference_price = Money(1_000);
        st.last_settlement_price = Money(1_000);
    }

    // macro_flow_interval_ticks default = 10. Tick 0 is a multiple of 10 → nudge fires.
    // The nudge runs AFTER ClearMarkets, so it reads the real post-clear unmet (=5) → raises max_price.
    world.insert_resource(Tick(0));
    schedule.run(&mut world);
    let after_fire = world.resource::<DemandPools>().0[&consumer].max_price.0;
    assert!(after_fire > 2_000, "nudge fired on cadence boundary (tick 0), read post-clear unmet: {after_fire}");

    // Non-boundary tick (3): the cadence gate skips the nudge. max_price (only the nudge writes it)
    // must be unchanged from the post-tick-0 value.
    world.insert_resource(Tick(3));
    let before_noop = world.resource::<DemandPools>().0[&consumer].max_price.0;
    schedule.run(&mut world);
    let after_noop = world.resource::<DemandPools>().0[&consumer].max_price.0;
    assert_eq!(after_noop, before_noop, "no nudge off the cadence boundary (tick 3)");

    let _ = (run_adjust_reservation_prices_system, EconomySet::AdjustReservationPrices);
}
```
(Ordering — that the nudge runs *after* Telemetry/ClearMarkets — is guaranteed structurally by the `configure_sets(...).chain()` slot (Step 4) and is *exercised* here: the nudge reading the real post-clear `unmet` is only possible if it runs after `ClearMarkets`. No recorder needed.)

- [ ] **Step 2: Run it — verify it FAILS** (set/system don't exist):

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core adjust_reservation_prices_fires_on_cadence_boundary_only
```
Expected: FAIL — `no variant AdjustReservationPrices` / `cannot find function run_adjust_reservation_prices_system`.

- [ ] **Step 3: Add the `EconomySet` variant** — in the `EconomySet` enum add `AdjustReservationPrices` **between** `Telemetry` and `UpdateConsumption`:

```rust
    Telemetry,
    AdjustReservationPrices,
    UpdateConsumption,
```

- [ ] **Step 4: Add it to the `configure_sets` chain** — in `install_systems`, in the `.chain()` tuple, insert `EconomySet::AdjustReservationPrices` between `EconomySet::Telemetry` and `EconomySet::UpdateConsumption`:

```rust
            EconomySet::Telemetry,
            EconomySet::AdjustReservationPrices,
            EconomySet::UpdateConsumption,
```

- [ ] **Step 5: Add the system wrapper** in `systems.rs` (near the other wrappers, e.g. after `run_consumption_update_system`):

```rust
/// Cadence-gated reservation-price nudge. Runs every `macro_flow_interval_ticks` (same slow
/// timescale as macro_flow, so the fast EWMA quantity loop settles between nudges). Surfaces a
/// genuine Err (config-validation / overflow) as an audited `MarketClearFailed` — never `let _`,
/// never a silent default.
pub fn run_adjust_reservation_prices_system(
    tick: Res<Tick>,
    config: Res<EconomyConfig>,
    market_goods: Res<MarketGoods>,
    mut demand: ResMut<DemandPools>,
    mut supply: ResMut<SupplyPools>,
    mut ledger: ResMut<TradeLedger>,
) {
    if config.macro_flow_interval_ticks == 0
        || !tick.0.is_multiple_of(config.macro_flow_interval_ticks)
    {
        return;
    }
    if let Err(reason) = crate::economy::pricing::run_adjust_reservation_prices_at_tick(
        &mut demand,
        &mut supply,
        &market_goods,
        &config,
    ) {
        ledger.0.push(EconomyEvent::MarketClearFailed {
            market: MarketId(0),
            good: GoodId(0),
            reason,
        });
    }
}
```
(Confirm the needed imports — `Tick`, `MarketGoods`, `DemandPools`, `SupplyPools`, `TradeLedger`, `EconomyEvent`, `MarketId`, `GoodId` — are already in scope at the top of `systems.rs`; they are used by the neighboring systems. Add any missing.)

- [ ] **Step 6: Register the system** — in the `add_systems((…))` tuple, add (next to `update_market_telemetry_system`):

```rust
            run_adjust_reservation_prices_system.in_set(EconomySet::AdjustReservationPrices),
```

- [ ] **Step 7: Run the ordering+cadence test — verify it PASSES**

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core adjust_reservation_prices_fires_on_cadence_boundary_only
```
Expected: PASS.

- [ ] **Step 8: Run the FULL economy suite — the nudge is now active in the default schedule, so confirm it does NOT perturb the existing multi-tick tests**

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::
```
Expected: PASS. In particular `conservation_full_plugin_multi_tick` and `steady_state_multi_tick` must stay green (the nudge is quiescent at equilibrium — `net≈0 → x≈0 → no move`). **If a multi-tick test now fails**, do NOT loosen it: investigate whether the nudge destabilized the steady state (a real finding — report it; the speed-limit/cadence may need tuning, or the test's transient window needs the equilibrium to be reached). A single-tick or direct-function test should be unaffected (the nudge only changes NEXT tick's reservation prices).

- [ ] **Step 9: Commit**

```bash
git add backend/crates/sim-core/src/economy/systems.rs backend/crates/sim-core/src/economy/tests/systems.rs
git commit -m "feat(economy): wire AdjustReservationPrices set (cadence-gated, after Telemetry)"
```

---

## Sub-Slice C — Behavior + stability + gate

### Task C1: Scarcity-response behavior (sustained signal → monotone bounded price move)

**Files:** Test `backend/crates/sim-core/src/economy/tests/pricing.rs`.

- [ ] **Step 1: Write the test** (append to `tests/pricing.rs`) — proves the dead telemetry now drives the price over multiple cadence boundaries, monotone and bounded:

```rust
#[test]
fn sustained_shortage_raises_price_monotonically_and_bounded_over_intervals() {
    use crate::economy::{EconomicActorId, GOOD_TOOLS, MarketGoodKey, MarketGoodState, MarketGoods, MarketId, Money, Quantity, DemandPools, SupplyPools, DemandPool};
    let m = MarketId(1);
    let cfg = EconomyConfig::default();
    let mut d = DemandPools(std::collections::BTreeMap::from([(EconomicActorId(1), {
        let mut p = demand_pool(1, m, 2_000); p
    })]));
    let mut s = SupplyPools::default();
    let mut g = MarketGoods::default();
    let key = MarketGoodKey { market: m, good: GOOD_TOOLS };
    g.0.insert(key, state(m, 100, 0)); // sustained shortage every interval

    let mut prices = Vec::new();
    for _ in 0..8 {
        run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
        prices.push(d.0[&EconomicActorId(1)].max_price.0);
    }
    // Monotone non-decreasing (sustained shortage → price keeps rising)...
    for w in prices.windows(2) { assert!(w[1] >= w[0], "monotone rise under sustained shortage: {prices:?}"); }
    assert!(prices[0] > 2_000, "rose on the first interval");
    // ...and bounded by the per-interval speed limit (<=1% per step → <= ~1.01^8 of 2000).
    assert!(*prices.last().unwrap() <= 2_000 + 2_000 * 9 / 100, "rise is speed-limited (<=~1%/interval): {prices:?}");
    assert!(*prices.last().unwrap() <= cfg.price_ceiling.0, "never exceeds ceiling");

    // Glut variant: sustained unsold lowers the price monotonically, never below floor.
    let mut d2 = DemandPools(std::collections::BTreeMap::from([(EconomicActorId(1), demand_pool(1, m, 2_000))]));
    let mut g2 = MarketGoods::default();
    g2.0.insert(key, state(m, 0, 100));
    let mut down = Vec::new();
    for _ in 0..8 {
        run_adjust_reservation_prices_at_tick(&mut d2, &mut SupplyPools::default(), &g2, &cfg).unwrap();
        down.push(d2.0[&EconomicActorId(1)].max_price.0);
    }
    for w in down.windows(2) { assert!(w[1] <= w[0], "monotone fall under sustained glut: {down:?}"); }
    assert!(*down.last().unwrap() >= cfg.price_floor.0, "never below floor");
}
```

- [ ] **Step 2: Add the cross-market gap MEASUREMENT test** (spec §9 Test 10 deliverable — the one-sided source↔sink gap is LOGGED, never asserted-to-converge). Append to `tests/pricing.rs` — pure-core (deterministic, no plugin):

```rust
#[test]
fn cross_market_source_sink_gap_is_logged_and_stays_bounded() {
    // Pure-core model of the cross-market topology: a pure SOURCE market m_a (supply only →
    // post-flow glut → unsold>0) and a pure SINK market m_b (demand only → import shortfall →
    // unmet>0). Under the LOCAL-signal nudge: source min_price falls (glut), sink max_price
    // rises (shortage) — so the spatial gap WIDENS, NOT converges. This is the honest, spec-
    // disclosed limitation (§2): we LOG the gap and assert ONLY boundedness, never convergence.
    use crate::economy::{
        DemandPools, EconomicActorId, GOOD_TOOLS, MarketGoodKey, MarketGoods, MarketId, SupplyPools,
    };
    let cfg = EconomyConfig::default();
    let m_a = MarketId(1); // source
    let m_b = MarketId(2); // sink
    let mut d = DemandPools(std::collections::BTreeMap::from([(EconomicActorId(1), demand_pool(1, m_b, 2_000))]));
    let mut s = SupplyPools(std::collections::BTreeMap::from([(EconomicActorId(2), supply_pool(2, m_a, 500))]));
    let mut g = MarketGoods::default();
    g.0.insert(MarketGoodKey { market: m_b, good: GOOD_TOOLS }, state(m_b, 100, 0));  // sink: unmet
    g.0.insert(MarketGoodKey { market: m_a, good: GOOD_TOOLS }, state(m_a, 0, 100));  // source: unsold

    for i in 0..6 {
        run_adjust_reservation_prices_at_tick(&mut d, &mut s, &g, &cfg).unwrap();
        let sink = d.0[&EconomicActorId(1)].max_price.0;
        let src = s.0[&EconomicActorId(2)].min_price.0;
        // LOG (never assert) the cross-market gap — observability per spec Test 10.
        println!("interval {i}: sink_max={sink} src_min={src} gap={}", sink - src);
        // Assert ONLY boundedness/clearability: every reservation price stays in [floor, ceiling].
        assert!(src >= cfg.price_floor.0 && src <= cfg.price_ceiling.0, "src in band");
        assert!(sink >= cfg.price_floor.0 && sink <= cfg.price_ceiling.0, "sink in band");
    }
    // Honest documented outcome: gap widened (sink up, source down) — local scarcity-response is
    // correct; full one-sided LoOP convergence is NOT claimed (would need flow-margin feedback).
    assert!(d.0[&EconomicActorId(1)].max_price.0 > 2_000, "sink price rose under shortage");
    assert!(s.0[&EconomicActorId(2)].min_price.0 < 500, "source price fell under glut");
}
```

- [ ] **Step 3: Run both behavior tests — verify PASS**

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core -- --nocapture economy::tests::pricing::sustained_shortage_raises_price economy::tests::pricing::cross_market_source_sink_gap
```
Expected: PASS (the cross-market test PRINTS the widening gap via `--nocapture`; it asserts only boundedness + correct local direction, NEVER convergence).

- [ ] **Step 4: Commit**

```bash
git add backend/crates/sim-core/src/economy/tests/pricing.rs
git commit -m "test(economy): sustained scarcity drives price + cross-market gap logged (bounded, no LoOP claim)"
```

### Task C2: Steady-state non-destabilization + conservation (nudge active)

**Files:** Modify `backend/crates/sim-core/src/economy/tests/conservation.rs::steady_state_multi_tick`.

Note: `conservation_full_plugin_multi_tick` already asserts `total_money` byte-invariant every tick AND now runs with the nudge active (after B1) — that IS spec Test 8 (no change needed; B1 Step 8 confirmed it stays green). This task adds a reservation-price-stability band to the steady-state proof.

- [ ] **Step 1: Capture a reservation-price tail in `steady_state_multi_tick`.** Next to the existing tail vectors, add `let mut max_price_tail: Vec<i64> = Vec::new();`. Inside the `if i >= n - k { … }` tail block, push the consumer's live reservation price:

```rust
            max_price_tail.push(world.resource::<DemandPools>().0[&consumer].max_price.0);
```
(Confirm `DemandPools` is imported in the test fn; it is used by the setup.)

- [ ] **Step 2: Assert the reservation price is BOUNDED over the tail** (the nudge doesn't run away or oscillate unboundedly). After the existing band asserts, add:

```rust
    // Free-prices nudge is active in the default schedule. In the converged steady state the
    // excess-demand signal is ~0, so the reservation price must settle into a bounded band
    // (it neither runs to the ceiling nor oscillates wildly). This is the stability guard.
    let mp_lo = *max_price_tail.iter().min().unwrap();
    let mp_hi = *max_price_tail.iter().max().unwrap();
    assert!(mp_lo > 0, "reservation price stays positive (no ZeroPrice)");
    assert!(mp_hi - mp_lo < 2_000, "reservation-price tail bounded (no runaway/oscillation): lo={mp_lo} hi={mp_hi}");
```

- [ ] **Step 3: Run it — verify PASS**

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core steady_state_multi_tick
```
Expected: PASS — the steady state still lives, money constant, AND the reservation price is bounded with the nudge active. **If the price band assertion fails** (price ran away or oscillated), that is a REAL stability finding — STOP and report; do NOT widen the band to force green (the cadence/speed-limit design is on trial here).

- [ ] **Step 4: Commit**

```bash
git add backend/crates/sim-core/src/economy/tests/conservation.rs
git commit -m "test(economy): steady_state proves reservation price stays bounded with nudge active"
```

### Task C3: Full local gate

**Files:** none (verification only); commit only if a gate fix was required.

- [ ] **Step 1: Rust gate**

```bash
cd /Users/ramonfuglister/Coding/abutown-vtraders
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace
```
Expected: fmt clean; clippy exit 0; ALL workspace tests pass (the `--workspace` run here at the END is the final gate, not iteration).

- [ ] **Step 2: Frontend gate**

```bash
cd /Users/ramonfuglister/Coding/abutown-vtraders
npm run typecheck && npx vitest run && node scripts/build.mjs
```
Expected: typecheck clean, vitest pass, build OK.

- [ ] **Step 3: e2e render-smoke** (backend-only change, but mandatory before push; export the isolated target so the e2e_server build doesn't collide with the dev server):

```bash
cd /Users/ramonfuglister/Coding/abutown-vtraders
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target npm run test:e2e
```
Expected: render-smoke 2/2 (the nudge changes prices, not mobility/agent counts, so the pinned 300-pedestrian smoke is unaffected). If it fails on agent counts, that is unexpected for a pricing-only change — STOP and investigate (do not bump counts).

- [ ] **Step 4: Commit any gate fix** (only if Steps 1-3 required a change):

```bash
git add -A && git commit -m "fix(economy): <describe the gate fix>"
```

---

## PR-body notes (for `finishing-a-development-branch`)

1. **First economy slice with NO `DELETE FROM economy_snapshots`.** Mutates the already-persisted `min_price`/`max_price` fields, uses a stateless `macro_flow_interval_ticks` cadence gate (no cursor), and the new `EconomyConfig` knobs are not persisted (frozen post-seed). No schema change → old snapshots load unchanged.
2. **Conservation is trivial** — the nudge writes only i64 price *parameters* and reads telemetry; it never moves money/goods. `total_money` byte-invariance follows from settlement atomicity (unchanged), proven by `conservation_full_plugin_multi_tick` now running with the nudge active.
3. **Honest scope:** scarcity-response is *guaranteed* (sustained imbalance → bounded monotone price move, proven); cross-market Law-of-One-Price convergence is *not asserted* — the steady-state test only guards boundedness/clearability (no over-claim).
4. **Implements the spec-named deferred enhancement** (#69 cut `k_bps` nudge, lines 9/68/228; #75 "freie Preissetzung … späterer Bogen", line 128).
5. **Stacked on #76 → #75.** Merge order #75 → #76 → this. PR based on the #76 branch for a FOOD-then-prices clean diff; GitHub auto-retargets to main as the stack merges.
6. **Deferred (unchanged):** elasticity-shaped demand (`elasticity_bps`/`urgency_bps`); inventory/coverage pricing; full one-sided LoOP convergence via flow-margin feedback; profit-leak recovery + release-grade SFC audit; multi-stage chains; explicit labor market; per-capita consumption.

---

## Self-Review (run after writing — done)

**Spec coverage:** §3 mechanism → A2 (intensity/step/translate/clamp). §4 insertion point/config/error-model → A1 (config) + B1 (set after Telemetry before UpdateConsumption, cadence gate, Err→MarketClearFailed wrapper). §5 conservation/determinism/no-fallback/band/coexistence → A2 (pure, no money) + tests. §7 files → `pricing.rs` (A2), `systems.rs` (A1/B1), `mod.rs` (A2), `tests/{pricing,systems,conservation}.rs`. §8 schedule → B1 (chain slot). §9 tests 1-6 → A2 pure tests; test 7 → B1 ordering+cadence; test 8 → existing `conservation_full_plugin_multi_tick` (nudge active, B1 Step 8); test 9 → C1; test 10 → C2. §10 sub-slices A/B/C → matched. §11 no-DELETE/honest-scope → PR notes 1/3.

**Placeholder scan:** none — every step has exact code + commands.

**Type consistency:** `run_adjust_reservation_prices_at_tick(&mut DemandPools, &mut SupplyPools, &MarketGoods, &EconomyConfig) -> Result<(), EconomyError>` used identically in A2/B1/C1; `intensity_bps`/`nudge_price` helpers consistent; config getters `validated_price_adjust_k_bps`/`validated_price_adjust_max_step_bps`/`validated_price_band` match A1↔A2; `EconomySet::AdjustReservationPrices` + `run_adjust_reservation_prices_system` match B1 enum↔registration↔test; `MarketGoodState::new(key)` + `unmet_demand_last_tick`/`unsold_supply_last_tick` (Quantity) match market.rs; `MarketClearFailed { market, good, reason }` matches the macro_flow precedent.
