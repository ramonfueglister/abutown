# Economy Self-Sustaining Loop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the two leaks (goods + money) that unwind the economy loop, making it genuinely self-sustaining: a flow-capped raw-goods source feeding input-gated production, 100% of revenue (wage + profit) recycled to consuming households, and transport fees rebated — all conservation-exact and persistence-clean.

**Architecture:** Three sub-slices in one PR. (A) `GOOD_RAW` + a single `EXTRACTOR` actor that periodically deposits a flow-capped raw faucet, consumed by an existing-`run_production_at_tick` `RAW→GOOD_TOOLS` recipe (honest throttle on scarcity), with an explicit Sizing-Sim sub-step (§15.2) before the regen constant is fixed. (B) Full profit distribution from firms to the existing three labor households (fallible, audited — never panic/silent-skip) plus a transport-operator rebate, both via the same `pool_weights`/`apportion_cash` path as wages. (C) Persist the new `RawDeposits` resource (one `DELETE FROM economy_snapshots`, no serde-default), plus plugin-level conservation and a non-vacuous steady-state test with `EXTRACTOR` as the only supplier.

**Tech Stack:** Rust, `bevy_ecs` (Schedule/SystemSet), fixed-point `i64`/`i128` money/quantity, `BTreeMap` keys-first determinism, `serde_json` persistence. All cargo runs go through the isolated serial wrapper (CLAUDE.md: never run two cargo at once; never `--workspace --all-targets` during iteration; never kill cargo).

All test commands in this plan use:
```bash
export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core <filter>
```
Run from the repo root `/Users/ramonfuglister/Coding/abutown-vtraders`. Long runs: prefer `run_in_background` + poll, per CLAUDE.md.

---

### Verified facts (read against the real code before drafting; do NOT re-derive — confirm by inspection if you touch the relevant file)

- `goods.rs`: `GOOD_FOOD=GoodId(1)`, `GOOD_WOOD=GoodId(2)`, `GOOD_IRON=GoodId(3)`, `GOOD_TOOLS=GoodId(4)` → next free is `GoodId(5)`.
- `production.rs`: `Recipe { inputs: Vec<(GoodId, Quantity)>, outputs: Vec<(GoodId, Quantity)> }`, `ProductionPool { actor, recipe, interval_ticks: u64, last_generated_tick: Option<u64> }`, `ProductionPools(pub BTreeMap<EconomicActorId, ProductionPool>)`. `run_production_at_tick` uses `pools::interval_elapsed(last, current, interval)` and a keys-first `Vec` snapshot. Its top `use` block already imports `EconomicActorId, EconomyError, EconomyEvent, GoodId, InventoryBook, Quantity, TradeLedger, pools::interval_elapsed` — all `run_regen_at_tick` needs.
- `inventory.rs`: `balance(actor, good) -> InventoryBalance` (field `.available: Quantity`), `deposit(actor, good, qty) -> Result<(), EconomyError>`, `consume(...)`, `total_good(good) -> Result<Quantity, EconomyError>`.
- `accounts.rs`: `account(actor) -> MoneyAccount` (fields `.available`, `.locked`, both `Money`), `deposit(actor, amount) -> Result`, `transfer(from, to, amount) -> Result` (errors `NegativeMoney` if `amount<0`, `InsufficientFunds` if `from.available < amount`), `total_money() -> Result<Money, EconomyError>`.
- `auction.rs:106`: `pub fn apportion_cash(weights: &[i64], total: i64) -> Vec<i64>`.
- `wages.rs`: `HOUSEHOLD_SECTOR = EconomicActorId(u64::MAX - 1)`; `SellerReceipts(pub BTreeMap<(EconomicActorId, MarketId), Money>)`; `HouseholdSector { population: u64, pool_weights: BTreeMap<EconomicActorId, i64> }`; `pub(crate) fn wage_for_revenue(revenue: Money, labor_share_bps: i128) -> Result<Money, EconomyError>`; `run_pay_wages_at_tick` two-leg pattern (firm → HOUSEHOLD_SECTOR → pools via `apportion_cash` over `pool_weights`, resets `income_last_tick`, own net-zero `debug_assert`). The `wages.rs` top `use` block imports `AccountBook, DemandPools, EconomicActorId, EconomyConfig, EconomyError, EconomyEvent, MarketId, Money, TradeLedger, apportion_cash`.
- `transport.rs`: `TRANSPORT_OPERATOR = EconomicActorId(u64::MAX)`; top `use` is `use crate::economy::{EconomicActorId, EconomyError, Money, Quantity, checked_order_value};`. `manhattan_tiles`/`transport_cost*` use `Quantity` + `checked_order_value`.
- `macro_flow.rs:873-877`: the whole macro-flow step is gated `if config.macro_flow_interval_ticks == 0 || !current_tick.is_multiple_of(config.macro_flow_interval_ticks) { return Ok(()); }`. The `TRANSPORT_OPERATOR` credit (`accounts.deposit(TRANSPORT_OPERATOR, transport_total)?` at line 712) is INSIDE that gate, so the operator is credited ONLY on `Tick.0 % macro_flow_interval_ticks == 0` boundaries.
- `systems.rs`: `EconomyConfig` has `macro_flow_interval_ticks` (default `10`), `labor_share_bps` (default `6_000`) with `validated_labor_share_bps()`, `trader_default_ref_price = Money(1_000)`. `EconomySet` chain is `ResetReceipts, RefreshLod, ExpireOrders, Production, GeneratePoolOrders, ClearMarkets, MacroFlow, PayWages, Consume, ShopperCapture, CommuterCapture, Materialize, Telemetry, UpdateConsumption`. The parallel `add_systems((...)).before(crate::mobility::systems::tick_increment_system)` block holds `run_pay_wages_system` etc. `run_pay_wages_system` and `run_consumption_update_system` both `.expect(...)` (the spec-endorsed convention for invariant-break surfacing); `run_production_system` and `run_consumption_system` use `let _` (a pre-existing wart, NOT a license to add a new one).
- `ledger.rs`: `EconomyEvent` variants incl. `WagePaid`, `MarketClearFailed { market, good, reason }`, `Produced`, `Consumed`, `FinalConsumed`, `Trade`; `event_type()` is an exhaustive match. `TradeLedger(pub Vec<EconomyEvent>)`.
- `mod.rs`: `EconomyPlugin::install` hand-inserts every economy resource then calls `install_systems(schedule)`. `pub use goods::*; pub use production::*;` etc., so new consts/types are reachable as `crate::economy::*`.
- `persist.rs`: `EconomyPersistSnapshot` mirrors maps as sorted `Vec<(K,V)>`; `production_pools` field + extract (`production.0.iter().map(|(k,v)|(*k,v.clone())).collect()`) + apply (`ProductionPools(snap.production_pools.iter().cloned().collect())`). `household_sector` is the precedent for a no-serde-default new field. `EconomyError` derives serde so it survives JSON.
- `tests/seed.rs`: the unit-test world builder is **`seed_world()`** (NOT `seeded_world`). It hand-inserts ONLY `NodeSpatialIndex, Graph, Markets, MarketChunks, AccountBook, InventoryBook, SupplyPools, DemandPools, MarketDistances, MarketGoods` — it does **NOT** insert `ProductionPools` (and after A4 will not insert `RawDeposits`). Every existing seed test does `let mut world = seed_world(); seed_demo_economy(&mut world);` EXPLICITLY (the builder does NOT run the seeder). No seed test asserts a `SupplyPools.len()` (they use `.values().any(...)`), so no count needs bumping. `EconomicActorId` is already imported in `seed.rs:13`.
- `tests/wages.rs`: helpers `full_economy_world()`, `tick_world(world, schedule)` (runs schedule THEN `world.resource_mut::<Tick>().0 += 1`), `consumer_pool(actor, market)`. Already imports `ChunkCoord, AsleepChunk, ChunkCoordComp, MarketDistances, MarketChunks, MarketGoodKey, MarketGoodState, MarketGoods, HOUSEHOLD_SECTOR, HouseholdSector, SellerReceipts, EconomyEvent, GOOD_FOOD, GOOD_TOOLS`.
- `tests/systems.rs`: imports `CorePlugin`, `SimPlugin`, the recorder-into-set ordering pattern (`shopper_capture_set_runs_after_macro_flow_before_materialize`).
- `tests/persist.rs`: `install_economy()` calls the full `EconomyPlugin.install` (so any plugin-registered resource — incl. `RawDeposits` after A4 — is present); `extract_from_world`/`apply_into_world` round-trip helpers.

### Tick double-increment convention (load-bearing for C2/C3/B6 boundary reasoning)

`MobilityPlugin` unconditionally installs `tick_increment_system`, so a single `schedule.run` already advances `Tick.0` by 1. The house test helper `tick_world` (and every wired multi-tick test) then ALSO does `world.resource_mut::<Tick>().0 += 1` — a deliberate house convention, so over a run `Tick.0` advances by **2 per loop iteration** and the loop counter `t` is **NOT** equal to `Tick.0`. Consequence: any boundary reasoning (`is_multiple_of(macro_flow_interval_ticks)`, the operator-drain boundary) MUST be pinned against the **actual `world.resource::<Tick>().0` value read inside the loop**, never against the loop index. All tests below that assert on boundaries read `Tick.0` and gate their boundary asserts on `Tick.0 % interval == 0`.

---

## Sub-Slice A — Goods Source (continuous production)

### Task A1: `GOOD_RAW` constant (structurally non-tradable)

**Files:**
- Modify: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/goods.rs`
- Test: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/tests/production.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/production.rs`:

```rust
#[test]
fn good_raw_is_the_next_free_good_id_and_distinct() {
    use crate::economy::{GOOD_FOOD, GOOD_IRON, GOOD_RAW, GOOD_TOOLS, GOOD_WOOD, GoodId};
    // RAW takes the next free u16 after TOOLS(4); it must be distinct from all tradables.
    assert_eq!(GOOD_RAW, GoodId(5));
    for g in [GOOD_FOOD, GOOD_WOOD, GOOD_IRON, GOOD_TOOLS] {
        assert_ne!(g, GOOD_RAW, "GOOD_RAW must not collide with a tradable good");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core good_raw_is_the_next_free`
Expected: FAIL — `cannot find value GOOD_RAW in ... economy` (compile error).

- [ ] **Step 3: Add the constant**

Append to `goods.rs` (after `GOOD_TOOLS`):

```rust
/// A structurally non-tradable primary resource (the next free `GoodId` after
/// `GOOD_TOOLS`). RAW is NEVER constructed into a `SupplyPool`/`DemandPool`/market
/// seed, so there is no listing path: it can never reach an `OrderBook`/`MarketGoods`.
/// Non-tradability is ENFORCED by absence (no runtime guard). RAW exists only to be
/// deposited by the `EXTRACTOR` faucet (`run_regen_at_tick`) and consumed as a recipe
/// input by `run_production_at_tick`.
pub const GOOD_RAW: GoodId = GoodId(5);
```

`goods.rs` is glob-re-exported by `mod.rs` (`pub use goods::*;`), so `GOOD_RAW` is automatically reachable as `crate::economy::GOOD_RAW`.

- [ ] **Step 4: Run test to verify it passes**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core good_raw_is_the_next_free`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/goods.rs backend/crates/sim-core/src/economy/tests/production.rs
git commit -m "feat(economy): add structurally non-tradable GOOD_RAW

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task A2: `Regenerated` ledger event + `event_type` arm

**Files:**
- Modify: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/ledger.rs`
- Test: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/tests/production.rs`

This task adds ONLY the `Regenerated` variant (the `ProfitDistributed`/`TransportRebate` variants come in Sub-Slice B, Task B1, to keep each commit scoped to its sub-slice).

- [ ] **Step 1: Write the failing test**

Append to `tests/production.rs`:

```rust
#[test]
fn regenerated_event_type_tag_is_stable() {
    use crate::economy::{EconomicActorId, EconomyEvent, GOOD_RAW, Quantity};
    let e = EconomyEvent::Regenerated {
        actor: EconomicActorId(8_031),
        good: GOOD_RAW,
        qty: Quantity(100),
    };
    assert_eq!(e.event_type(), "regenerated");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core regenerated_event_type_tag_is_stable`
Expected: FAIL — `no variant ... Regenerated`.

- [ ] **Step 3: Add the variant + the `event_type` arm**

In `ledger.rs`, add the variant inside `enum EconomyEvent` (place it directly after the `WagePaid { ... }` variant, before the closing `}` of the enum):

```rust
    /// The `EXTRACTOR` faucet deposited `qty` of a raw good this interval (goods-only,
    /// no money). The sole source of new `GOOD_RAW`; pairs with the recipe `Consumed`
    /// events in the per-good conservation balance.
    Regenerated {
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    },
```

Add the matching arm inside `event_type()` (after the `Self::WagePaid { .. } => "wage_paid",` arm):

```rust
            Self::Regenerated { .. } => "regenerated",
```

- [ ] **Step 4: Run test to verify it passes**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core regenerated_event_type_tag_is_stable`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/ledger.rs backend/crates/sim-core/src/economy/tests/production.rs
git commit -m "feat(economy): add Regenerated ledger event

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task A3: `EXTRACTOR` + `RawDeposit`/`RawDeposits` + `run_regen_at_tick`

**Files:**
- Modify: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/production.rs`
- Test: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/tests/production.rs`

- [ ] **Step 1: Write the failing tests**

Append to `tests/production.rs`:

```rust
#[test]
fn regen_deposits_faucet_on_interval_and_stamps_cursor() {
    use crate::economy::production::{EXTRACTOR, RawDeposit, RawDeposits, run_regen_at_tick};
    use crate::economy::{EconomyEvent, GOOD_RAW, InventoryBook, Quantity, TradeLedger};
    use std::collections::BTreeMap;

    let mut inv = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    let mut deposits = RawDeposits(BTreeMap::new());
    deposits.0.insert(
        EXTRACTOR,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(100),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );

    run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, 5).unwrap();

    assert_eq!(inv.balance(EXTRACTOR, GOOD_RAW).available, Quantity(100));
    assert_eq!(deposits.0[&EXTRACTOR].last_regen_tick, Some(5));
    assert!(ledger.0.contains(&EconomyEvent::Regenerated {
        actor: EXTRACTOR,
        good: GOOD_RAW,
        qty: Quantity(100),
    }));
}

#[test]
fn regen_skips_within_interval_but_does_not_advance_cursor_on_skip() {
    use crate::economy::production::{EXTRACTOR, RawDeposit, RawDeposits, run_regen_at_tick};
    use crate::economy::{GOOD_RAW, InventoryBook, Quantity, TradeLedger};
    use std::collections::BTreeMap;

    let mut inv = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    let mut deposits = RawDeposits(BTreeMap::new());
    deposits.0.insert(
        EXTRACTOR,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(100),
            interval_ticks: 10,
            last_regen_tick: None,
        },
    );
    run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, 0).unwrap(); // fires (last=None)
    run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, 3).unwrap(); // interval not elapsed
    assert_eq!(
        inv.balance(EXTRACTOR, GOOD_RAW).available,
        Quantity(100),
        "only one deposit within the interval"
    );
    // On a skip the gate returns BEFORE stamping, so the cursor stays at the firing tick.
    assert_eq!(deposits.0[&EXTRACTOR].last_regen_tick, Some(0));
}

#[test]
fn regen_is_flow_capped_not_capacity_capped() {
    use crate::economy::production::{EXTRACTOR, RawDeposit, RawDeposits, run_regen_at_tick};
    use crate::economy::{GOOD_RAW, InventoryBook, Quantity, TradeLedger};
    use std::collections::BTreeMap;

    // No recipe consuming RAW here: deposits stack unboundedly per interval (faucet,
    // not a level-capped reservoir). The recipe is what bounds RAW in the live loop.
    let mut inv = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    let mut deposits = RawDeposits(BTreeMap::new());
    deposits.0.insert(
        EXTRACTOR,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(100),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );
    for t in 0..3 {
        run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, t).unwrap();
    }
    assert_eq!(inv.balance(EXTRACTOR, GOOD_RAW).available, Quantity(300));
}

#[test]
fn regen_is_deterministic_keys_first() {
    use crate::economy::production::{RawDeposit, RawDeposits, run_regen_at_tick};
    use crate::economy::{EconomicActorId, GOOD_RAW, InventoryBook, Quantity, TradeLedger};
    use std::collections::BTreeMap;

    let run = || {
        let mut inv = InventoryBook::default();
        let mut ledger = TradeLedger::default();
        let mut deposits = RawDeposits(BTreeMap::new());
        // Insert out of ascending order to prove keys-first iteration.
        for a in [EconomicActorId(9), EconomicActorId(2)] {
            deposits.0.insert(
                a,
                RawDeposit {
                    good: GOOD_RAW,
                    qty_per_interval: Quantity(50),
                    interval_ticks: 1,
                    last_regen_tick: None,
                },
            );
        }
        run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, 1).unwrap();
        ledger.0
    };
    assert_eq!(run(), run());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core regen_`
Expected: FAIL — `could not find ... EXTRACTOR / RawDeposit / RawDeposits / run_regen_at_tick`.

- [ ] **Step 3: Implement `EXTRACTOR`, `RawDeposit`, `RawDeposits`, `run_regen_at_tick`**

The existing top `use` block in `production.rs` already imports everything `run_regen_at_tick` needs (`EconomicActorId, EconomyError, EconomyEvent, GoodId, InventoryBook, Quantity, TradeLedger, pools::interval_elapsed`, and `std::collections::BTreeMap`). Append at the end of `production.rs`:

```rust
/// The single named primary-resource extractor. ONE faucet (not N scattered ones),
/// adjacent to the other seeded actor ids (8_001..8_022) but well clear of them.
pub const EXTRACTOR: EconomicActorId = EconomicActorId(8_031);

/// A standing raw-goods faucet for one actor. PERSISTED (mirrors `ProductionPool`).
/// `last_regen_tick` is the interval cursor (gates deposits, persists for free since
/// `Option<u64>: Copy` keeps `RawDeposit` `Copy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RawDeposit {
    pub good: GoodId,
    pub qty_per_interval: Quantity,
    pub interval_ticks: u64,
    pub last_regen_tick: Option<u64>,
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct RawDeposits(pub BTreeMap<EconomicActorId, RawDeposit>);

/// Flow-capped faucet: for each deposit whose `interval_ticks` have elapsed, deposit
/// `qty_per_interval` of `good` into the actor's inventory (goods-only — NEVER touches
/// money) and emit `Regenerated`. Deterministic, keys-first (ascending `EconomicActorId`).
/// Honest wording: this deposits unconditionally on the interval (it does NOT read the
/// raw stock), so RAW grows without a level cap here — the `RAW→good` recipe in
/// `run_production_at_tick` is what bounds it (RAW stays `<= 2*qty_per_interval` in the
/// live loop because the recipe drains it as soon as `>= recipe qty` is on hand). Stamps
/// `last_regen_tick` ONLY when the deposit fires (the gate returns before stamping on a
/// skip), so a within-interval skip is a true no-op (cursor included).
pub fn run_regen_at_tick(
    inventory: &mut InventoryBook,
    ledger: &mut TradeLedger,
    deposits: &mut RawDeposits,
    current_tick: u64,
) -> Result<(), EconomyError> {
    let actors: Vec<EconomicActorId> = deposits.0.keys().copied().collect();
    for actor in actors {
        let dep = deposits.0[&actor];
        if !interval_elapsed(dep.last_regen_tick, current_tick, dep.interval_ticks) {
            continue;
        }
        inventory.deposit(actor, dep.good, dep.qty_per_interval)?;
        ledger.0.push(EconomyEvent::Regenerated {
            actor,
            good: dep.good,
            qty: dep.qty_per_interval,
        });
        if let Some(d) = deposits.0.get_mut(&actor) {
            d.last_regen_tick = Some(current_tick);
        }
    }
    Ok(())
}
```

> Cursor-convention note: `run_regen_at_tick` stamps the cursor ONLY on fire (gate returns before stamping on a skip), which is the correct flow-faucet behavior and is exactly what the `regen_skips_within_interval_but_does_not_advance_cursor_on_skip` test pins. This differs from `run_production_at_tick` (which stamps every call); a faucet skip must be a complete no-op so multiple `interval_ticks` between fires are honored exactly.

- [ ] **Step 4: Run tests to verify they pass**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core regen_`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/production.rs backend/crates/sim-core/src/economy/tests/production.rs
git commit -m "feat(economy): add EXTRACTOR raw faucet (RawDeposits + run_regen_at_tick)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task A4: Register `RawDeposits` resource + `EconomySet::Regenerate` system between `ExpireOrders` and `Production`

**Files:**
- Modify: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/mod.rs`
- Modify: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/systems.rs`
- Test: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/tests/systems.rs`

- [ ] **Step 1: Write the failing tests**

Append to `tests/systems.rs`:

```rust
#[test]
fn regenerate_set_runs_after_expire_before_production() {
    // Pin EconomySet::Regenerate's position against the REAL install_systems chain:
    // recorder systems placed into ExpireOrders / Regenerate / Production must fire in
    // exactly that order (RAW deposited before the recipe can consume it same-tick).
    use crate::economy::{EconomyPlugin, systems::EconomySet};
    use bevy_ecs::prelude::*;

    #[derive(Resource, Default)]
    struct OrderLog(Vec<&'static str>);
    fn rec_expire(mut log: ResMut<OrderLog>) {
        log.0.push("expire");
    }
    fn rec_regen(mut log: ResMut<OrderLog>) {
        log.0.push("regen");
    }
    fn rec_production(mut log: ResMut<OrderLog>) {
        log.0.push("production");
    }

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    world.insert_resource(OrderLog::default());
    schedule.add_systems((
        rec_expire.in_set(EconomySet::ExpireOrders),
        rec_regen.in_set(EconomySet::Regenerate),
        rec_production.in_set(EconomySet::Production),
    ));
    schedule.run(&mut world);

    let log = &world.resource::<OrderLog>().0;
    let i_e = log.iter().position(|s| *s == "expire").unwrap();
    let i_r = log.iter().position(|s| *s == "regen").unwrap();
    let i_p = log.iter().position(|s| *s == "production").unwrap();
    assert!(i_e < i_r, "Regenerate must run AFTER ExpireOrders: {log:?}");
    assert!(i_r < i_p, "Regenerate must run BEFORE Production: {log:?}");
}

#[test]
fn raw_deposits_resource_is_installed_by_plugin() {
    use crate::economy::EconomyPlugin;
    use crate::economy::production::RawDeposits;
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    EconomyPlugin.install(&mut world, &mut schedule);
    assert!(world.get_resource::<RawDeposits>().is_some());
}

#[test]
fn regenerate_system_feeds_input_gated_production_same_tick() {
    // EXTRACTOR has a RAW faucet + a RAW->TOOLS recipe. After one schedule run, the RAW
    // deposited this tick is immediately consumed by the recipe → TOOLS appears; net RAW
    // on hand is bounded (deposit qty minus what the recipe took).
    use crate::economy::production::{
        EXTRACTOR, ProductionPool, ProductionPools, RawDeposit, RawDeposits, Recipe,
    };
    use crate::economy::{EconomyPlugin, GOOD_RAW, GOOD_TOOLS, InventoryBook, Quantity};

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    world.resource_mut::<RawDeposits>().0.insert(
        EXTRACTOR,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(100),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        EXTRACTOR,
        ProductionPool {
            actor: EXTRACTOR,
            recipe: Recipe {
                inputs: vec![(GOOD_RAW, Quantity(100))],
                outputs: vec![(GOOD_TOOLS, Quantity(100))],
            },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );

    schedule.run(&mut world);

    let inv = world.resource::<InventoryBook>();
    assert_eq!(
        inv.balance(EXTRACTOR, GOOD_TOOLS).available,
        Quantity(100),
        "Regenerate deposited RAW before Production consumed it (same-tick ordering)"
    );
    assert_eq!(
        inv.balance(EXTRACTOR, GOOD_RAW).available,
        Quantity(0),
        "the recipe drained the freshly-deposited RAW this tick"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core regenerate_ raw_deposits_resource`
Expected: FAIL — `no variant ... Regenerate` for the set, and `RawDeposits` not installed.

- [ ] **Step 3: Add the `Regenerate` set variant, wire the system, register the resource**

In `systems.rs`, extend the existing `crate::economy::{ ... }` import to add `run_regen_at_tick`. Change the tail of that import list:
```rust
    run_pay_wages_at_tick, run_production_at_tick,
};
```
to:
```rust
    run_pay_wages_at_tick, run_production_at_tick, run_regen_at_tick,
};
use crate::economy::production::RawDeposits;
```

Add `Regenerate` to the `EconomySet` enum, between `ExpireOrders` and `Production`:
```rust
    ExpireOrders,
    Regenerate,
    Production,
```

In `install_systems`, add `Regenerate` to the `.chain()` `configure_sets` tuple between `ExpireOrders` and `Production`:
```rust
            EconomySet::ExpireOrders,
            EconomySet::Regenerate,
            EconomySet::Production,
```

In the parallel `add_systems((...)).before(crate::mobility::systems::tick_increment_system)` block, add the regen system directly after `expire_orders_system`:
```rust
            expire_orders_system.in_set(EconomySet::ExpireOrders),
            run_regen_system.in_set(EconomySet::Regenerate),
            run_production_system.in_set(EconomySet::Production),
```

Add the system function (place directly before `run_production_system`):
```rust
/// The goods-only raw faucet. Surfaces an invariant break (an inventory.deposit overflow)
/// via `.expect` — matching the spec-endorsed `run_pay_wages_system`/`run_consumption_update_system`
/// convention, NOT the pre-existing `let _` wart in `run_production_system`. By construction
/// the faucet is flow-capped and cannot overflow a sane i64 balance, so the `.expect` is a
/// loud bug-surface, not a silent discard.
pub fn run_regen_system(
    tick: Res<Tick>,
    mut inventory: ResMut<InventoryBook>,
    mut ledger: ResMut<TradeLedger>,
    mut deposits: ResMut<RawDeposits>,
) {
    run_regen_at_tick(&mut inventory, &mut ledger, &mut deposits, tick.0)
        .expect("run_regen_at_tick is infallible by construction (flow-capped faucet deposit cannot overflow a sane i64 balance); an Err is a bug");
}
```

In `mod.rs`, register the resource inside `install` (add directly after the `ProductionPools::default()` insert):
```rust
        world.insert_resource(crate::economy::production::RawDeposits::default());
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core regenerate_ raw_deposits_resource`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/systems.rs backend/crates/sim-core/src/economy/mod.rs backend/crates/sim-core/src/economy/tests/systems.rs
git commit -m "feat(economy): schedule EconomySet::Regenerate before Production; register RawDeposits

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task A5a: Regen Sizing-Sim (§15.2) — fix the faucet rate against measured aggregate demand BEFORE seeding

**Files:**
- Test (analysis harness, kept as a permanent regression): `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/tests/production.rs`

§15.2 / §13-A REQUIRE a Sizing-Sim before the regen constant is fixed: the regen rate must cover the aggregate TOOLS demand at seed prices, because §8 is honest that static prices do NOT self-correct chronic scarcity — an undersized faucet would silently starve the steady-state test. This sub-task is a SHORT analysis test that measures aggregate per-tick TOOLS demand and asserts the chosen `REGEN_QTY` covers it; it both documents the sizing rationale and locks it against future drift. It runs BEFORE A5 fixes `REGEN_QTY`.

**Sizing analysis (recorded; the test below proves it):**
- The live seed has exactly ONE TOOLS consumer pool: `consumer = 8_002` at `m_b`, `desired_qty_per_tick = 10` (`seed.rs:119-136`). The two FOOD consumers (8_012, 8_022) demand FOOD, not TOOLS, so they do NOT draw on the EXTRACTOR's TOOLS output.
- Aggregate per-tick TOOLS demand at seed = `10`. The recipe is `RAW(REGEN_QTY) → TOOLS(REGEN_QTY)` with `interval_ticks=1`, and the `SupplyPool.offered_qty_per_tick = REGEN_QTY`. To cover demand without chronic scarcity we need `REGEN_QTY >= aggregate_tools_demand = 10`.
- Choose `REGEN_QTY = 10` (exactly covers; matches the existing finite TOOLS supplier's `offered_qty_per_tick=10`, so the EXTRACTOR is a drop-in standing replacement once the 1M endowment drains). RAW on hand stays `<= 2*10` because the same-tick recipe drains it (proved in A4's `regenerate_system_feeds_input_gated_production_same_tick`).
- **FOOD decision (§15.2, recorded):** FOOD is left on the finite endowment. The seeded FOOD suppliers (8_011 @ m_a, 8_021 @ m_fa) keep their 1M endowment and are intentionally NOT made self-sustaining by this slice — only RAW→TOOLS is closed. Rationale: this slice's goal is to prove ONE closed goods+money loop end-to-end (TOOLS); generalizing the faucet to FOOD (a second RAW→FOOD extractor) is a deferred follow-up. The steady-state test (C3) is EXTRACTOR/TOOLS-only precisely so the FOOD draining-endowment does not mask the closed loop. This decision is carried verbatim into the PR body (see the Deployment / design-decisions note at the end).

- [ ] **Step 1: Write the sizing analysis test (failing until `REGEN_QTY` is wired in A5; this test references the seed)**

Append to `tests/production.rs`. This test runs the live seeder, measures aggregate TOOLS demand, and asserts the seeded EXTRACTOR's faucet rate covers it. It will compile-fail until A5 seeds the EXTRACTOR; that is the intended TDD ordering — A5a writes the assertion, A5 makes it pass.

```rust
#[test]
fn regen_rate_covers_aggregate_tools_demand_at_seed() {
    // §15.2 Sizing-Sim: the EXTRACTOR's faucet (and its same-rate recipe + SupplyPool)
    // MUST cover aggregate per-tick TOOLS demand at seed prices, else static prices leave
    // chronic TOOLS scarcity (§8). Build the live demo world, measure aggregate TOOLS
    // demand, and assert the seeded faucet rate >= that demand.
    use crate::economy::production::{EXTRACTOR, RawDeposits};
    use crate::economy::{DemandPools, GOOD_TOOLS};

    // Reuse the seed-test world builder (added/extended in tests/seed.rs is a sibling
    // module; here we build a minimal spatial world the same way the seeder needs).
    // Build the world inline so this test does not depend on the seed test module.
    let mut world = bevy_ecs::world::World::new();
    {
        use crate::routing::{Graph, Node, NodeId, NodeKind, NodeSpatialIndex};
        let node = |id: u32, x: f32, y: f32| Node {
            id: NodeId(id),
            position: (x, y),
            kind: NodeKind::Intersection,
            legacy_id: None,
        };
        let nodes = vec![
            node(0, 2.0, 3.0),
            node(1, 13.0, 3.0),
            node(2, 16.0, 48.0),
            node(3, 208.0, 48.0),
        ];
        world.insert_resource(NodeSpatialIndex::from_nodes(&nodes));
        world.insert_resource(Graph::new(nodes, vec![]));
        world.insert_resource(crate::economy::Markets::default());
        world.insert_resource(crate::economy::MarketChunks::default());
        world.insert_resource(crate::economy::AccountBook::default());
        world.insert_resource(crate::economy::InventoryBook::default());
        world.insert_resource(crate::economy::SupplyPools::default());
        world.insert_resource(crate::economy::DemandPools::default());
        world.insert_resource(crate::economy::MarketDistances::default());
        world.insert_resource(crate::economy::MarketGoods::default());
        world.insert_resource(crate::economy::production::ProductionPools::default());
        world.insert_resource(crate::economy::production::RawDeposits::default());
    }
    crate::economy::seed::seed_demo_economy(&mut world);

    let aggregate_tools_demand: i64 = world
        .resource::<DemandPools>()
        .0
        .values()
        .filter(|p| p.good == GOOD_TOOLS)
        .map(|p| p.desired_qty_per_tick.0)
        .sum();
    assert_eq!(
        aggregate_tools_demand, 10,
        "seed has exactly one TOOLS consumer @ 10/tick (sizing baseline)"
    );

    let faucet = world.resource::<RawDeposits>().0[&EXTRACTOR];
    assert!(
        faucet.qty_per_interval.0 >= aggregate_tools_demand && faucet.interval_ticks == 1,
        "EXTRACTOR faucet rate ({} per {} tick(s)) must cover aggregate TOOLS demand ({}/tick) \
         at seed prices, else chronic scarcity (§8/§15.2)",
        faucet.qty_per_interval.0,
        faucet.interval_ticks,
        aggregate_tools_demand
    );
}
```

> This test depends on `seed_demo_economy` inserting `RawDeposits[EXTRACTOR]` (done in A5) and on `seed_world`-style resources being present including `ProductionPools` + `RawDeposits` (inserted inline above). It will be RED until A5 lands the EXTRACTOR seed.

- [ ] **Step 2: Run to verify it fails (no EXTRACTOR seeded yet)**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core regen_rate_covers_aggregate_tools_demand`
Expected: FAIL — panics on `world.resource::<RawDeposits>().0[&EXTRACTOR]` (key absent) because A5 has not seeded the EXTRACTOR yet. This RED test is the sizing contract A5 must satisfy.

- [ ] **Step 3: Commit the sizing contract (RED is expected; it goes green in A5)**

This sub-task deliberately commits a test that stays RED until A5. To keep the commit history green-per-commit, DO NOT commit A5a alone — implement A5a and A5 in the same working session and make a single combined commit at the end of A5 (Step 6) covering both files. Proceed directly to A5 without committing here.

---

### Task A5: Seed the `EXTRACTOR` (RAW inventory + RawDeposit + RAW→TOOLS recipe + TOOLS SupplyPool); add `ProductionPools`+`RawDeposits` to the seed-test world builder

**Files:**
- Modify: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/seed.rs`
- Modify: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/tests/seed.rs`

The `EXTRACTOR` is seeded ALONGSIDE the existing finite suppliers (lead decision): the endowments drain, the `EXTRACTOR` becomes the standing source. The `EXTRACTOR` sells `GOOD_TOOLS` at `m_a` (the existing TOOLS market) so its output flows through the existing auction/flow path. RAW is NEVER placed on a pool. The faucet rate `REGEN_QTY = 10` is fixed by the A5a Sizing-Sim.

> CRITICAL accuracy fix: the existing `seed_world()` builder in `tests/seed.rs` hand-inserts only `Markets, MarketChunks, AccountBook, InventoryBook, SupplyPools, DemandPools, MarketDistances, MarketGoods` (plus `Graph`/`NodeSpatialIndex`). It does **NOT** insert `ProductionPools` and does **NOT** insert `RawDeposits`. Once `seed_demo_economy` starts touching `ProductionPools`/`RawDeposits` (this task), the three existing seed tests (`seed_demo_economy_creates_four_markets`, `seed_adds_second_good_without_new_markets`, `seed_adds_flow_demo_markets_for_dormant_cross_flow`) — which each call `seed_demo_economy(&mut world)` on a `seed_world()` world — would panic with "Resource does not exist". This task therefore ALSO adds those two resource inserts to `seed_world()`. No seed test asserts a `SupplyPools.len()` (they use `.values().any(...)`), so no count needs bumping.

- [ ] **Step 1: Write the failing test**

Append to `tests/seed.rs`. It uses the existing `seed_world()` builder and EXPLICITLY calls `seed_demo_economy(&mut world)` (the builder does NOT run the seeder), exactly like the three sibling tests:

```rust
#[test]
fn seed_installs_extractor_with_raw_faucet_recipe_and_tools_supply_but_never_lists_raw() {
    use crate::economy::production::{EXTRACTOR, ProductionPools, RawDeposits};
    use crate::economy::{GOOD_RAW, GOOD_TOOLS, HouseholdSector, InventoryBook};

    let mut world = seed_world();
    seed_demo_economy(&mut world);

    // RawDeposit for EXTRACTOR exists and faucets GOOD_RAW at the sized rate.
    let dep = world.resource::<RawDeposits>().0[&EXTRACTOR];
    assert_eq!(dep.good, GOOD_RAW);
    assert_eq!(dep.qty_per_interval.0, 10, "fixed by the A5a Sizing-Sim");
    assert_eq!(dep.interval_ticks, 1);

    // EXTRACTOR has a RAW->TOOLS recipe.
    let pool = world.resource::<ProductionPools>().0[&EXTRACTOR].clone();
    assert_eq!(pool.recipe.inputs, vec![(GOOD_RAW, dep.qty_per_interval)]);
    assert_eq!(pool.recipe.outputs.len(), 1);
    assert_eq!(pool.recipe.outputs[0].0, GOOD_TOOLS);

    // EXTRACTOR sells TOOLS (the tradable), never RAW.
    let sp = world.resource::<SupplyPools>().0[&EXTRACTOR];
    assert_eq!(sp.good, GOOD_TOOLS);

    // GOOD_RAW is NEVER on any SupplyPool or DemandPool (structural non-tradability).
    assert!(
        world.resource::<SupplyPools>().0.values().all(|p| p.good != GOOD_RAW),
        "RAW must never be on a SupplyPool"
    );
    assert!(
        world.resource::<DemandPools>().0.values().all(|p| p.good != GOOD_RAW),
        "RAW must never be on a DemandPool"
    );

    // EXTRACTOR is NOT in pool_weights (it is a firm, not a consumer household).
    assert!(
        !world.resource::<HouseholdSector>().pool_weights.contains_key(&EXTRACTOR),
        "EXTRACTOR is a firm, not a labor household"
    );

    // EXTRACTOR holds an opening RAW endowment so production fires on tick 0.
    assert!(world.resource::<InventoryBook>().balance(EXTRACTOR, GOOD_RAW).available.0 > 0);
}
```

The `tests/seed.rs` import list (lines 4-7) must gain `HouseholdSector` and the test references `SupplyPools`/`DemandPools` (already imported). Add `HouseholdSector` to that `use crate::economy::{...}` block:
```rust
use crate::economy::{
    AccountBook, DemandPools, HouseholdSector, InventoryBook, MarketChunks, MarketGoods, MarketId,
    Markets, SupplyPools,
};
```

And extend `seed_world()` to insert the two resources the seeder now touches (add both lines just before `world` is returned, after the existing `MarketGoods` insert):
```rust
    world.insert_resource(crate::economy::production::ProductionPools::default());
    world.insert_resource(crate::economy::production::RawDeposits::default());
```

- [ ] **Step 2: Run test to verify it fails**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core seed_installs_extractor`
Expected: FAIL — `EXTRACTOR` not present in `RawDeposits`/`ProductionPools`/`SupplyPools`.

- [ ] **Step 3: Seed the `EXTRACTOR`**

In `seed.rs`, add the production imports (a SEPARATE `use` line so there is no risk of duplicating `EconomicActorId`, which is already in the main import block at line 13). Insert directly after the existing main `use crate::economy::{ ... };` block:
```rust
use crate::economy::production::{
    EXTRACTOR, ProductionPool, ProductionPools, RawDeposit, RawDeposits, Recipe,
};
```
Then add `GOOD_RAW` to the EXISTING `use crate::economy::{ ... }` block (it already imports `GOOD_FOOD, GOOD_TOOLS` and `EconomicActorId`; just insert `GOOD_RAW` in alpha order next to the other `GOOD_*`):
```rust
    AccountBook, DemandPool, DemandPools, EconomicActorId, GOOD_FOOD, GOOD_RAW, GOOD_TOOLS,
    HOUSEHOLD_SECTOR, HouseholdSector, InventoryBook, MarketChunks, MarketDistances, MarketGoodKey,
    MarketGoodState, MarketGoods, MarketId, MarketSite, Markets, Money, Quantity, SupplyPool,
    SupplyPools,
```

Insert the `EXTRACTOR` seed block AFTER the existing TOOLS supplier (`supplier = 8_001`) `SupplyPools` insert (which ends at line 118) and BEFORE the FOOD-supplier block (which starts at line 137). The `EXTRACTOR` sells TOOLS at the same market `m_a` as the finite TOOLS supplier:

```rust
    // ── Continuous goods source: the EXTRACTOR (Sub-Slice A) ──────────────────
    // A standing faucet of GOOD_RAW (non-tradable) + a RAW->TOOLS recipe + a TOOLS
    // SupplyPool at m_a. Seeded ALONGSIDE the finite supplier 8_001: the 1M TOOLS
    // endowment drains, the EXTRACTOR becomes the standing source. RAW is NEVER placed
    // on a pool/market (structurally non-tradable). REGEN_QTY=10 is fixed by the §15.2
    // Sizing-Sim (tests/production.rs::regen_rate_covers_aggregate_tools_demand_at_seed):
    // aggregate seed TOOLS demand is 10/tick (consumer 8_002), so 10/tick exactly covers
    // it and matches the finite supplier's offered rate. FOOD is intentionally left on
    // the draining 1M endowment (no RAW->FOOD extractor this slice — recorded decision).
    const REGEN_QTY: Quantity = Quantity(10);
    world
        .resource_mut::<InventoryBook>()
        .deposit(EXTRACTOR, GOOD_RAW, REGEN_QTY)
        .expect("seed: extractor opening raw stock");
    world.resource_mut::<RawDeposits>().0.insert(
        EXTRACTOR,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: REGEN_QTY,
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        EXTRACTOR,
        ProductionPool {
            actor: EXTRACTOR,
            recipe: Recipe {
                inputs: vec![(GOOD_RAW, REGEN_QTY)],
                outputs: vec![(GOOD_TOOLS, REGEN_QTY)],
            },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<SupplyPools>().0.insert(
        EXTRACTOR,
        SupplyPool {
            actor: EXTRACTOR,
            market: m_a,
            good: GOOD_TOOLS,
            offered_qty_per_tick: REGEN_QTY,
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
```

> The `EXTRACTOR` is a firm: it is NOT added to `pool_weights`. The `pool_weights` build at `seed.rs:282-301` iterates `DemandPools` keys; the EXTRACTOR has only a `SupplyPool` (no `DemandPool`), so it is correctly excluded — no change needed there.

- [ ] **Step 4: Run test to verify it passes (incl. the A5a sizing contract going green)**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core seed_installs_extractor regen_rate_covers_aggregate_tools_demand`
Expected: PASS (both — the seed test and the A5a sizing contract).

- [ ] **Step 5: Run the full seed + production + systems suites (regression — proves the three existing seed tests still pass after `seed_world()` gained the two resources)**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests::seed economy::tests::production economy::tests::systems`
Expected: PASS (all). The three pre-existing seed tests (`seed_demo_economy_creates_four_markets`, `seed_adds_second_good_without_new_markets`, `seed_adds_flow_demo_markets_for_dormant_cross_flow`) must STILL pass: they assert `Markets.len()==4` and use `.values().any(...)` on pools, none of which the EXTRACTOR changes; the only reason they could newly panic is the missing `ProductionPools`/`RawDeposits` resource in `seed_world()`, which Step 1 fixed.

- [ ] **Step 6: Commit (A5a sizing test + A5 seed together, single green commit)**

```bash
git add backend/crates/sim-core/src/economy/seed.rs backend/crates/sim-core/src/economy/tests/seed.rs backend/crates/sim-core/src/economy/tests/production.rs
git commit -m "feat(economy): seed EXTRACTOR (RAW faucet + RAW->TOOLS recipe + TOOLS supply) sized by §15.2

Sizing-Sim fixes REGEN_QTY=10 to cover aggregate seed TOOLS demand (10/tick).
FOOD left on the finite endowment (recorded decision; no RAW->FOOD extractor this slice).
seed_world() test builder gains ProductionPools+RawDeposits so the seeder no longer panics.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Sub-Slice B — Money Leaks (profit distribution + transport rebate)

### Task B1: `ProfitDistributed` + `TransportRebate` ledger events + `event_type` arms

**Files:**
- Modify: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/ledger.rs`
- Test: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/tests/wages.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/wages.rs`:

```rust
#[test]
fn profit_and_rebate_event_type_tags_are_stable() {
    use crate::economy::{EconomicActorId, EconomyEvent, MarketId, Money};
    let p = EconomyEvent::ProfitDistributed {
        firm: EconomicActorId(8_001),
        market: MarketId(9_001),
        amount: Money(400),
    };
    assert_eq!(p.event_type(), "profit_distributed");
    let r = EconomyEvent::TransportRebate {
        amount: Money(123),
    };
    assert_eq!(r.event_type(), "transport_rebate");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core profit_and_rebate_event_type`
Expected: FAIL — `no variant ... ProfitDistributed / TransportRebate`.

- [ ] **Step 3: Add the variants + arms**

In `ledger.rs`, add inside `enum EconomyEvent` (after the `Regenerated { ... }` variant from Task A2):

```rust
    /// One firm's profit (revenue − wage) distributed to the labor households this tick.
    /// Full-distribution v0: no owner/capitalist actor — profit flows to the existing
    /// consumer pools via the SAME pool_weights as wages. Emitted per (firm, market).
    ProfitDistributed {
        firm: EconomicActorId,
        market: MarketId,
        amount: Money,
    },
    /// The accumulated TRANSPORT_OPERATOR balance rebated to the labor households at a
    /// macro-flow interval boundary (the buyers paid the fee; it returns to them).
    TransportRebate {
        amount: Money,
    },
```

Add the arms inside `event_type()` (after the `Self::Regenerated { .. } => "regenerated",` arm):

```rust
            Self::ProfitDistributed { .. } => "profit_distributed",
            Self::TransportRebate { .. } => "transport_rebate",
```

- [ ] **Step 4: Run test to verify it passes**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core profit_and_rebate_event_type`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/ledger.rs backend/crates/sim-core/src/economy/tests/wages.rs
git commit -m "feat(economy): add ProfitDistributed + TransportRebate ledger events

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task B2: `dividend_share_bps` config field + `validated_dividend_share_bps()`

**Files:**
- Modify: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/systems.rs`
- Test: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/tests/wages.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/wages.rs`:

```rust
#[test]
fn dividend_share_default_is_full_and_validates_bounds() {
    use crate::economy::{EconomyConfig, EconomyError};
    let cfg = EconomyConfig::default();
    assert_eq!(cfg.dividend_share_bps, 10_000, "default is full distribution");
    assert_eq!(cfg.validated_dividend_share_bps().unwrap(), 10_000_i128);

    let bad = EconomyConfig {
        dividend_share_bps: 10_001,
        ..EconomyConfig::default()
    };
    assert_eq!(
        bad.validated_dividend_share_bps(),
        Err(EconomyError::InvalidOrder),
        "share > 10_000 is a config bug, not a default"
    );

    let zero = EconomyConfig {
        dividend_share_bps: 0,
        ..EconomyConfig::default()
    };
    assert_eq!(zero.validated_dividend_share_bps().unwrap(), 0_i128);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core dividend_share_default_is_full`
Expected: FAIL — `no field dividend_share_bps` / `no method validated_dividend_share_bps`.

- [ ] **Step 3: Add the field, default, and validator**

In `systems.rs`, add the field to `struct EconomyConfig` (after `max_commuters_per_market: usize,`):

```rust
    /// Share of firm PROFIT (revenue − wage) distributed to labor households (basis
    /// points, 0..=10_000). Default 10_000 = full distribution: firms net to zero each
    /// tick (no retained earnings, no capitalist class — lead decision). A value < 10_000
    /// would strand profit in firm accounts and the loop would NOT be self-sustaining.
    pub dividend_share_bps: u16,
```

Add to `impl Default for EconomyConfig` (after `max_commuters_per_market: 4,`):

```rust
            dividend_share_bps: 10_000,
```

Add the validator to `impl EconomyConfig` (after `validated_labor_share_bps`):

```rust
    /// `dividend_share_bps` as an i128, refusing `> 10_000` (a config bug that would
    /// over-distribute). Boundary `== 10_000` allowed (full distribution). Mirrors
    /// `validated_labor_share_bps`.
    pub fn validated_dividend_share_bps(&self) -> Result<i128, crate::economy::EconomyError> {
        if self.dividend_share_bps > 10_000 {
            return Err(crate::economy::EconomyError::InvalidOrder);
        }
        Ok(self.dividend_share_bps as i128)
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core dividend_share_default_is_full`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/systems.rs backend/crates/sim-core/src/economy/tests/wages.rs
git commit -m "feat(economy): add EconomyConfig.dividend_share_bps (default full) + validator

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task B3: `run_distribute_profit_at_tick` (fallible, audited)

**Files:**
- Modify: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/wages.rs`
- Test: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/tests/wages.rs`

This function distributes the profit `revenue − wage` from each `(firm, market)` to the labor households via the SAME `pool_weights`/`apportion_cash` path as wages, two legs (firm → `HOUSEHOLD_SECTOR` → households). It is FALLIBLE: a firm that bought via macro_flow after selling may hold `< profit` at distribution time. On `InsufficientFunds`, it books ONLY the covered amount and pushes an audited `MarketClearFailed`-style event — never `.expect`-panic, never silent skip. Its own `HOUSEHOLD_SECTOR` net-zero `debug_assert` is independent of the wage assert.

- [ ] **Step 1: Write the failing tests**

Append to `tests/wages.rs`:

```rust
#[test]
fn distribute_profit_conserves_money_and_drains_firm_to_zero() {
    use crate::economy::wages::run_distribute_profit_at_tick;
    use crate::economy::{EconomyConfig, EconomyError};
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let c2 = EconomicActorId(8_012);
    // Firm sold 1_000; PayWages already paid wage=600 and left 400 in the firm account.
    // Here we model the post-wage state: firm holds the 400 profit.
    let mut accounts = AccountBook::default();
    accounts.deposit(f1, Money(400)).unwrap();
    let mut receipts = SellerReceipts::default();
    receipts.0.insert((f1, MarketId(9_001)), Money(1_000)); // gross revenue captured this tick
    let mut demand = DemandPools::default();
    demand.0.insert(c1, consumer_pool(c1, MarketId(9_002)));
    demand.0.insert(c2, consumer_pool(c2, MarketId(9_002)));
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(c1, 1), (c2, 1)]),
    };
    let config = EconomyConfig::default(); // labor_share=6_000, dividend_share=10_000

    let before = accounts.total_money().unwrap();
    let mut ledger = TradeLedger::default();
    run_distribute_profit_at_tick(&mut accounts, &receipts, &mut demand, &household, &mut ledger, &config)
        .unwrap();

    // wage = floor(1000*0.6)=600; profit = 1000-600 = 400; dividend = floor(400*1.0)=400.
    assert_eq!(accounts.total_money().unwrap(), before, "byte-invariant total money");
    assert_eq!(accounts.account(f1).available, Money::ZERO, "full distribution drains the firm");
    assert_eq!(accounts.account(HOUSEHOLD_SECTOR).available, Money::ZERO, "sentinel nets to zero");
    assert_eq!(demand.0[&c1].income_last_tick, Money(200));
    assert_eq!(demand.0[&c2].income_last_tick, Money(200));
    assert!(ledger.0.contains(&EconomyEvent::ProfitDistributed {
        firm: f1,
        market: MarketId(9_001),
        amount: Money(400),
    }));
    let _ = EconomyError::InvalidOrder; // import sanity
}

#[test]
fn distribute_profit_underfunded_firm_books_only_covered_and_audits() {
    use crate::economy::wages::run_distribute_profit_at_tick;
    use crate::economy::EconomyConfig;
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    // Receipts say revenue=1_000 (profit target = 400), but the firm only HOLDS 150
    // (it spent the rest buying inputs via macro_flow this tick). We must book 150 and
    // audit the shortfall — never panic, never silently skip.
    let mut accounts = AccountBook::default();
    accounts.deposit(f1, Money(150)).unwrap();
    let mut receipts = SellerReceipts::default();
    receipts.0.insert((f1, MarketId(9_001)), Money(1_000));
    let mut demand = DemandPools::default();
    demand.0.insert(c1, consumer_pool(c1, MarketId(9_002)));
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(c1, 1)]),
    };
    let config = EconomyConfig::default();

    let before = accounts.total_money().unwrap();
    let mut ledger = TradeLedger::default();
    run_distribute_profit_at_tick(&mut accounts, &receipts, &mut demand, &household, &mut ledger, &config)
        .expect("the function itself returns Ok — the shortfall is surfaced via an audited event, not an Err");

    assert_eq!(accounts.total_money().unwrap(), before, "byte-invariant even under shortfall");
    assert_eq!(accounts.account(f1).available, Money::ZERO, "all the firm HELD was distributed");
    assert_eq!(accounts.account(HOUSEHOLD_SECTOR).available, Money::ZERO, "sentinel nets to zero");
    assert_eq!(demand.0[&c1].income_last_tick, Money(150), "only the covered amount reached the household");
    // Audited shortfall event (MarketClearFailed-style), NOT a panic, NOT a silent drop.
    let audited = ledger.0.iter().any(|e| matches!(
        e,
        EconomyEvent::MarketClearFailed { market, reason, .. }
            if *market == MarketId(9_001) && *reason == crate::economy::EconomyError::InsufficientFunds
    ));
    assert!(audited, "an underfunded profit distribution must surface an audited event");
    // The covered amount is still booked as ProfitDistributed.
    assert!(ledger.0.contains(&EconomyEvent::ProfitDistributed {
        firm: f1,
        market: MarketId(9_001),
        amount: Money(150),
    }));
}

#[test]
fn distribute_profit_zero_dividend_share_is_noop() {
    use crate::economy::wages::run_distribute_profit_at_tick;
    use crate::economy::EconomyConfig;
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let mut accounts = AccountBook::default();
    accounts.deposit(f1, Money(400)).unwrap();
    let mut receipts = SellerReceipts::default();
    receipts.0.insert((f1, MarketId(9_001)), Money(1_000));
    let mut demand = DemandPools::default();
    demand.0.insert(c1, consumer_pool(c1, MarketId(9_002)));
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(c1, 1)]),
    };
    let config = EconomyConfig { dividend_share_bps: 0, ..EconomyConfig::default() };
    let before = accounts.total_money().unwrap();
    let mut ledger = TradeLedger::default();
    run_distribute_profit_at_tick(&mut accounts, &receipts, &mut demand, &household, &mut ledger, &config)
        .unwrap();
    assert_eq!(accounts.total_money().unwrap(), before);
    assert_eq!(accounts.account(f1).available, Money(400), "0 share retains profit at the firm");
    assert_eq!(demand.0[&c1].income_last_tick, Money::ZERO);
}

#[test]
fn distribute_profit_does_not_reset_income() {
    // Profit distribution ADDS to income_last_tick (wages credited it first); it must not
    // zero it. Seed a non-zero income and assert it accumulates.
    use crate::economy::wages::run_distribute_profit_at_tick;
    use crate::economy::EconomyConfig;
    let f1 = EconomicActorId(8_001);
    let c1 = EconomicActorId(8_002);
    let mut accounts = AccountBook::default();
    accounts.deposit(f1, Money(400)).unwrap();
    let mut receipts = SellerReceipts::default();
    receipts.0.insert((f1, MarketId(9_001)), Money(1_000));
    let mut demand = DemandPools::default();
    let mut pool = consumer_pool(c1, MarketId(9_002));
    pool.income_last_tick = Money(600); // wages already credited this tick
    demand.0.insert(c1, pool);
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(c1, 1)]),
    };
    let config = EconomyConfig::default();
    let mut ledger = TradeLedger::default();
    run_distribute_profit_at_tick(&mut accounts, &receipts, &mut demand, &household, &mut ledger, &config)
        .unwrap();
    assert_eq!(demand.0[&c1].income_last_tick, Money(1_000), "wage 600 + dividend 400, accumulated");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core distribute_profit_`
Expected: FAIL — `cannot find function run_distribute_profit_at_tick`.

- [ ] **Step 3: Implement `run_distribute_profit_at_tick`**

The existing `wages.rs` top `use` block already imports `AccountBook, DemandPools, EconomicActorId, EconomyConfig, EconomyError, EconomyEvent, MarketId, Money, TradeLedger, apportion_cash`. `HouseholdSector`, `SellerReceipts`, and `wage_for_revenue` are all declared in this module. The only missing item is `GoodId` (used for the audit-event sentinel) — reference it fully-qualified as `crate::economy::GoodId` to avoid touching the import block. Append the function at the end of `wages.rs`:

```rust
/// Full profit distribution to the labor households (no owner/capitalist class, v0).
/// For each `(firm, market)` in `receipts` (keys-first → ascending): recompute the wage
/// the SAME way `run_pay_wages_at_tick` did (`wage_for_revenue(revenue, labor_share)`,
/// identical flooring), so `profit = revenue − wage`; the dividend to distribute is
/// `floor(profit * dividend_share_bps / 10_000)`. With the default `dividend_share_bps =
/// 10_000` the dividend == profit and the firm nets to zero (wages + profit == revenue).
///
/// FALLIBLE / AUDITED (no-fallback discipline): a firm that BOTH sold and bought (via
/// macro_flow) this tick can hold `< dividend` at distribution time. We do NOT `.expect`
/// (latent process panic) and do NOT silently skip (stranded profit, broken loop).
/// Instead we book ONLY the amount the firm actually HOLDS (`min(dividend, available)`)
/// and, when that is short of the intended dividend, push an audited
/// `MarketClearFailed { market, good=GoodId(0), reason: InsufficientFunds }` event. The
/// covered amount flows firm → HOUSEHOLD_SECTOR → households via `apportion_cash` over the
/// SAME `pool_weights` as wages, crediting `income_last_tick` (ADD, not reset — wages
/// credited it first). Conservation: only `transfer`s ⇒ `total_money` byte-invariant.
/// Own independent `HOUSEHOLD_SECTOR` net-zero `debug_assert`.
pub fn run_distribute_profit_at_tick(
    accounts: &mut AccountBook,
    receipts: &SellerReceipts,
    demand: &mut DemandPools,
    household: &HouseholdSector,
    ledger: &mut TradeLedger,
    config: &EconomyConfig,
) -> Result<(), EconomyError> {
    let labor_share = config.validated_labor_share_bps()?;
    let dividend_share = config.validated_dividend_share_bps()?;

    let payees: Vec<EconomicActorId> = household.pool_weights.keys().copied().collect();
    let weights: Vec<i64> = household.pool_weights.values().copied().collect();
    let weight_sum: i128 = weights.iter().map(|w| *w as i128).sum();

    for (&(firm, market), &revenue) in receipts.0.iter() {
        let wage = wage_for_revenue(revenue, labor_share)?;
        let profit = revenue.checked_sub(wage)?; // wage <= revenue ⇒ profit >= 0
        let dividend_raw = i64::try_from((profit.0 as i128) * dividend_share / 10_000)
            .map_err(|_| EconomyError::Overflow)?;
        let intended = Money(dividend_raw);
        if intended.0 <= 0 || weight_sum <= 0 {
            continue; // nothing to distribute, or no payout target
        }
        // Book only what the firm actually holds; surface any shortfall loudly.
        let held = accounts.account(firm).available;
        let covered = Money(intended.0.min(held.0));
        if covered.0 < intended.0 {
            ledger.0.push(EconomyEvent::MarketClearFailed {
                market,
                good: crate::economy::GoodId(0),
                reason: EconomyError::InsufficientFunds,
            });
        }
        if covered.0 <= 0 {
            continue;
        }
        // LEG 1: firm → HOUSEHOLD_SECTOR. `covered <= held` ⇒ cannot fault.
        accounts.transfer(firm, HOUSEHOLD_SECTOR, covered)?;
        // LEG 2: HOUSEHOLD_SECTOR → households (largest-remainder, sum-preserving).
        let splits = apportion_cash(&weights, covered.0);
        for (idx, actor) in payees.iter().enumerate() {
            let share = Money(splits[idx]);
            if share.0 <= 0 {
                continue;
            }
            let pool = demand
                .0
                .get_mut(actor)
                .expect("pool_weights key must reference a seeded demand pool");
            accounts.transfer(HOUSEHOLD_SECTOR, *actor, share)?;
            pool.income_last_tick = pool.income_last_tick.checked_add(share)?;
        }
        ledger.0.push(EconomyEvent::ProfitDistributed {
            firm,
            market,
            amount: covered,
        });
    }

    debug_assert_eq!(
        accounts.account(HOUSEHOLD_SECTOR).available,
        Money::ZERO,
        "HOUSEHOLD_SECTOR must net to zero after profit distribution (sentinel-stranded cash)"
    );
    Ok(())
}
```

> The `expect` on `demand.0.get_mut(actor)` mirrors the EXACT same line in `run_pay_wages_at_tick` (a seed-consistency invariant: every `pool_weights` key references a seeded pool). That is a structural invariant break, not a money-fallback, so `.expect` is the spec-endorsed surfacing — identical to the established wage path. The MONEY-shortfall path (the only genuinely-fallible case) is the audited `MarketClearFailed` push, never an `.expect`. The `good: GoodId(0)` sentinel matches `run_macro_flow_system`'s convention for a money fault not attributable to one `(market, good)`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core distribute_profit_`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/wages.rs backend/crates/sim-core/src/economy/tests/wages.rs
git commit -m "feat(economy): add fallible, audited run_distribute_profit_at_tick

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task B4: `run_transport_rebate_at_tick`

**Files:**
- Modify: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/transport.rs`
- Test: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/tests/transport.rs`

Drains the ENTIRE `TRANSPORT_OPERATOR` balance → `HOUSEHOLD_SECTOR` → households via `apportion_cash` over `pool_weights`; credits `income_last_tick`; emits `TransportRebate`; own net-zero `debug_assert`. (The system-level macro_flow-modulo gating is done in Task B5; the function itself drains unconditionally.) This is conservative-by-construction (it transfers exactly the held operator balance), so the SYSTEM wrapper may `.expect` it — that is acceptable per spec (genuinely infallible).

- [ ] **Step 1: Write the failing tests**

Append to `tests/transport.rs`:

```rust
#[test]
fn transport_rebate_drains_operator_to_zero_and_conserves() {
    use crate::economy::transport::run_transport_rebate_at_tick;
    use crate::economy::{
        AccountBook, DemandPool, DemandPools, EconomicActorId, EconomyEvent, GOOD_TOOLS,
        HOUSEHOLD_SECTOR, HouseholdSector, MarketId, Money, Quantity, TRANSPORT_OPERATOR,
        TradeLedger,
    };
    use std::collections::BTreeMap;

    fn pool(actor: EconomicActorId) -> DemandPool {
        DemandPool {
            actor,
            market: MarketId(9_002),
            good: GOOD_TOOLS,
            desired_qty_per_tick: Quantity(10),
            max_price: Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
            last_consumed_tick: None,
            income_last_tick: Money::ZERO,
            mpc_bps: 8_000,
            autonomous: Money(5_000),
        }
    }

    let c1 = EconomicActorId(8_002);
    let c2 = EconomicActorId(8_012);
    let c3 = EconomicActorId(8_022);
    let mut accounts = AccountBook::default();
    accounts.deposit(TRANSPORT_OPERATOR, Money(301)).unwrap();
    let mut demand = DemandPools::default();
    for c in [c1, c2, c3] {
        demand.0.insert(c, pool(c));
    }
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(c1, 1), (c2, 1), (c3, 1)]),
    };

    let before = accounts.total_money().unwrap();
    let mut ledger = TradeLedger::default();
    run_transport_rebate_at_tick(&mut accounts, &mut demand, &household, &mut ledger).unwrap();

    assert_eq!(accounts.total_money().unwrap(), before, "byte-invariant total money");
    assert_eq!(accounts.account(TRANSPORT_OPERATOR).available, Money::ZERO, "operator fully drained");
    assert_eq!(accounts.account(HOUSEHOLD_SECTOR).available, Money::ZERO, "sentinel nets to zero");
    // 301 across 3 equal weights, largest-remainder ⇒ 101/100/100 (lowest index wins extra).
    assert_eq!(demand.0[&c1].income_last_tick, Money(101));
    assert_eq!(demand.0[&c2].income_last_tick, Money(100));
    assert_eq!(demand.0[&c3].income_last_tick, Money(100));
    let total: i64 = demand.0.values().map(|p| p.income_last_tick.0).sum();
    assert_eq!(total, 301, "Σ income == rebated amount");
    assert!(ledger.0.contains(&EconomyEvent::TransportRebate { amount: Money(301) }));
}

#[test]
fn transport_rebate_zero_balance_is_noop() {
    use crate::economy::transport::run_transport_rebate_at_tick;
    use crate::economy::{
        AccountBook, DemandPool, DemandPools, EconomicActorId, GOOD_TOOLS, HouseholdSector,
        MarketId, Money, Quantity, TradeLedger,
    };
    use std::collections::BTreeMap;
    let c1 = EconomicActorId(8_002);
    let mut accounts = AccountBook::default();
    let mut demand = DemandPools::default();
    demand.0.insert(
        c1,
        DemandPool {
            actor: c1,
            market: MarketId(9_002),
            good: GOOD_TOOLS,
            desired_qty_per_tick: Quantity(10),
            max_price: Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
            last_consumed_tick: None,
            income_last_tick: Money::ZERO,
            mpc_bps: 8_000,
            autonomous: Money(5_000),
        },
    );
    let household = HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(c1, 1)]),
    };
    let before = accounts.total_money().unwrap();
    let mut ledger = TradeLedger::default();
    run_transport_rebate_at_tick(&mut accounts, &mut demand, &household, &mut ledger).unwrap();
    assert_eq!(accounts.total_money().unwrap(), before);
    assert_eq!(demand.0[&c1].income_last_tick, Money::ZERO);
    assert!(ledger.0.is_empty(), "no rebate event when nothing to drain");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core transport_rebate_`
Expected: FAIL — `cannot find function run_transport_rebate_at_tick`.

- [ ] **Step 3: Implement `run_transport_rebate_at_tick`**

In `transport.rs`, change the top import. Current first line:
```rust
use crate::economy::{EconomicActorId, EconomyError, Money, Quantity, checked_order_value};
```
to:
```rust
use crate::economy::{
    AccountBook, DemandPools, EconomicActorId, EconomyError, EconomyEvent, HouseholdSector, Money,
    Quantity, TradeLedger, apportion_cash, checked_order_value,
};
```

Append at the end of `transport.rs`:

```rust
/// Recycle the accumulated `TRANSPORT_OPERATOR` balance back to the labor households (the
/// buyers paid the transport fee, so it returns to them). Drains the ENTIRE operator
/// balance → `HOUSEHOLD_SECTOR`, then `apportion_cash` over the SAME `pool_weights` as
/// wages → households, crediting `income_last_tick` (ADD). Emits `TransportRebate` with
/// the drained amount. Conservation: only `transfer`s ⇒ `total_money` byte-invariant; own
/// independent `HOUSEHOLD_SECTOR` net-zero `debug_assert`. The macro-flow-interval gating
/// (phase-locked to the operator CREDIT in macro_flow, no persisted cursor) is applied in
/// the system wrapper, NOT here — this function always drains whatever is present.
pub fn run_transport_rebate_at_tick(
    accounts: &mut AccountBook,
    demand: &mut DemandPools,
    household: &HouseholdSector,
    ledger: &mut TradeLedger,
) -> Result<(), EconomyError> {
    let amount = accounts.account(TRANSPORT_OPERATOR).available;
    if amount.0 <= 0 {
        return Ok(());
    }
    let payees: Vec<EconomicActorId> = household.pool_weights.keys().copied().collect();
    let weights: Vec<i64> = household.pool_weights.values().copied().collect();
    let weight_sum: i128 = weights.iter().map(|w| *w as i128).sum();
    if weight_sum <= 0 {
        return Ok(()); // no payout target: leave the fee in the operator (never strand it elsewhere)
    }

    // LEG 1: operator → HOUSEHOLD_SECTOR (the operator holds exactly `amount`).
    accounts.transfer(TRANSPORT_OPERATOR, HOUSEHOLD_SECTOR, amount)?;
    // LEG 2: HOUSEHOLD_SECTOR → households (sum-preserving).
    let splits = apportion_cash(&weights, amount.0);
    for (idx, actor) in payees.iter().enumerate() {
        let share = Money(splits[idx]);
        if share.0 <= 0 {
            continue;
        }
        let pool = demand
            .0
            .get_mut(actor)
            .expect("pool_weights key must reference a seeded demand pool");
        accounts.transfer(HOUSEHOLD_SECTOR, *actor, share)?;
        pool.income_last_tick = pool.income_last_tick.checked_add(share)?;
    }
    ledger.0.push(EconomyEvent::TransportRebate { amount });

    debug_assert_eq!(
        accounts.account(HOUSEHOLD_SECTOR).available,
        Money::ZERO,
        "HOUSEHOLD_SECTOR must net to zero after transport rebate (sentinel-stranded cash)"
    );
    Ok(())
}
```

> `Quantity` and `checked_order_value` remain imported because the existing `transport_cost`/`transport_cost_between` use them. `TRANSPORT_OPERATOR` is the module-local `pub const` defined at the top of this file.

- [ ] **Step 4: Run tests to verify they pass**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core transport_rebate_`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/transport.rs backend/crates/sim-core/src/economy/tests/transport.rs
git commit -m "feat(economy): add run_transport_rebate_at_tick (operator -> households)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task B5: Wire profit-distribution into PayWages (`.after`) + `EconomySet::TransportRebate` system + scoped intra-set ordering check

**Files:**
- Modify: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/systems.rs`
- Test: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/tests/systems.rs`

Profit distribution runs in the EXISTING `PayWages` set with an explicit `.after(run_pay_wages_system)` edge (the wage net-zero assert fires before profit credits). The transport rebate is a NEW set `EconomySet::TransportRebate`, after `PayWages` and before `Consume`, gated in its SYSTEM on `tick.0.is_multiple_of(config.macro_flow_interval_ticks)` (phase-locked to the operator credit at `macro_flow.rs:712`, which is itself inside the same modulo gate; no persisted cursor). The profit-distribution system surfaces any whole-call `Err` (a config-validation `Err`, which must NOT be `let _`'d) as an audited `MarketClearFailed`.

> Granularity fix: the PRIMARY ordering proof is the recorder-into-set test (deterministic, scoped). The plan does NOT gate a commit on a whole-schedule `ambiguity_detection: LogLevel::Error` run (which would panic on any pre-existing ambiguity anywhere in the CorePlugin+MobilityPlugin+EconomyPlugin schedule — an unknown global property). Determinism between the three writers is instead guaranteed by the explicit `.after` edges + set-chain membership, and verified by the recorder ordering test. An optional, scoped ambiguity probe (Step 6) checks ONLY the three new writers against each other and is informational, not a commit gate.

- [ ] **Step 1: Write the failing test (recorder-into-set ordering — the primary, deterministic gate)**

Append to `tests/systems.rs`:

```rust
#[test]
fn pay_wages_then_profit_then_rebate_order_within_schedule() {
    // Pin the intra/inter-set ordering: wage-credit (PayWages) → profit-credit (PayWages,
    // .after wage) → rebate-credit (TransportRebate set, after PayWages, before Consume).
    // Recorders are anchored to the REAL systems via .after edges so the recorded order
    // reflects the real .after edges, not just set membership.
    use crate::economy::systems::{
        run_distribute_profit_system, run_pay_wages_system, run_transport_rebate_system, EconomySet,
    };
    use crate::economy::EconomyPlugin;
    use bevy_ecs::prelude::*;

    #[derive(Resource, Default)]
    struct OrderLog(Vec<&'static str>);
    fn rec_wages(mut log: ResMut<OrderLog>) { log.0.push("wages"); }
    fn rec_profit(mut log: ResMut<OrderLog>) { log.0.push("profit"); }
    fn rec_rebate(mut log: ResMut<OrderLog>) { log.0.push("rebate"); }
    fn rec_consume(mut log: ResMut<OrderLog>) { log.0.push("consume"); }

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    // The rebate system is macro-flow-modulo gated; force tick 0 so the gate is open
    // (0 is a multiple of macro_flow_interval_ticks) and the rebate recorder still has a
    // deterministic position even though the recorder itself is unconditional.
    world.insert_resource(crate::mobility::resources::Tick(0));

    world.insert_resource(OrderLog::default());
    schedule.add_systems((
        rec_wages.in_set(EconomySet::PayWages).after(run_pay_wages_system),
        rec_profit.in_set(EconomySet::PayWages).after(run_distribute_profit_system),
        rec_rebate.in_set(EconomySet::TransportRebate).after(run_transport_rebate_system),
        rec_consume.in_set(EconomySet::Consume),
    ));
    schedule.run(&mut world);

    let log = &world.resource::<OrderLog>().0;
    let i_w = log.iter().position(|s| *s == "wages").unwrap();
    let i_p = log.iter().position(|s| *s == "profit").unwrap();
    let i_r = log.iter().position(|s| *s == "rebate").unwrap();
    let i_c = log.iter().position(|s| *s == "consume").unwrap();
    assert!(i_w < i_p, "profit credit AFTER wage credit: {log:?}");
    assert!(i_p < i_r, "rebate AFTER profit: {log:?}");
    assert!(i_r < i_c, "rebate BEFORE consume: {log:?}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core pay_wages_then_profit_then_rebate`
Expected: FAIL — `no variant ... TransportRebate`, `cannot find ... run_distribute_profit_system / run_transport_rebate_system`.

- [ ] **Step 3: Add the set, the two systems, and the schedule edges**

In `systems.rs` imports, extend the existing `crate::economy::{ ... }` import. The tail currently reads (after A4):
```rust
    run_pay_wages_at_tick, run_production_at_tick, run_regen_at_tick,
};
use crate::economy::production::RawDeposits;
```
Change the function-import tail to add the two new core functions, and add `TRANSPORT_OPERATOR` + `SellerReceipts` (SellerReceipts is already imported via the `wages` re-export list earlier in the same block — confirm by inspection; if already present do NOT re-list it). Final tail:
```rust
    run_distribute_profit_at_tick, run_pay_wages_at_tick, run_production_at_tick,
    run_regen_at_tick, run_transport_rebate_at_tick,
};
use crate::economy::production::RawDeposits;
```
`HouseholdSector` and `SellerReceipts` are already in the existing `systems.rs` import block (the file uses them in `run_pay_wages_system`). `TRANSPORT_OPERATOR` is referenced only inside the `transport.rs` core function, not in `systems.rs`, so it is NOT imported here (avoids an unused-import clippy warning).

Add `TransportRebate` to the `EconomySet` enum, between `PayWages` and `Consume`:
```rust
    PayWages,
    TransportRebate,
    Consume,
```

Add `TransportRebate` to the `.chain()` `configure_sets` tuple in `install_systems`, between `PayWages` and `Consume`:
```rust
            EconomySet::PayWages,
            EconomySet::TransportRebate,
            EconomySet::Consume,
```

In the parallel `add_systems((...)).before(crate::mobility::systems::tick_increment_system)` block, replace the single `run_pay_wages_system` line with three lines (wage; profit `.after` wage, both in `PayWages`; rebate in its own set):
```rust
            run_pay_wages_system.in_set(EconomySet::PayWages),
            run_distribute_profit_system
                .in_set(EconomySet::PayWages)
                .after(run_pay_wages_system),
            run_transport_rebate_system.in_set(EconomySet::TransportRebate),
```

Add the two system functions (place directly after `run_pay_wages_system`):

```rust
/// Profit distribution: runs in the PayWages set with an explicit `.after(run_pay_wages_system)`
/// edge so the wage net-zero assert fires first and income accumulates wage→profit in a
/// deterministic order. Fallible/audited at the per-firm level inside the core; the wrapper
/// surfaces a whole-call Err (only a config-validation failure can produce one) as an audited
/// MarketClearFailed event — never `let _` (which would swallow a config bug), never `.expect`
/// (the call is genuinely fallible).
pub fn run_distribute_profit_system(
    config: Res<EconomyConfig>,
    receipts: Res<SellerReceipts>,
    household: Res<HouseholdSector>,
    mut accounts: ResMut<AccountBook>,
    mut demand: ResMut<DemandPools>,
    mut ledger: ResMut<TradeLedger>,
) {
    if let Err(reason) = run_distribute_profit_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut ledger,
        &config,
    ) {
        ledger.0.push(EconomyEvent::MarketClearFailed {
            market: MarketId(0),
            good: GoodId(0),
            reason,
        });
    }
}

/// Transport rebate: gated on the SAME `tick.0.is_multiple_of(macro_flow_interval_ticks)`
/// modulo as the operator CREDIT in macro_flow (which is itself inside that gate at
/// macro_flow.rs:712), so credit and rebate are phase-locked — stateless, NO persisted
/// cursor. Mid-interval the operator balance may be `> 0`; at every interval boundary it
/// drains to zero. `run_transport_rebate_at_tick` is conservative-by-construction (it
/// drains exactly the held operator balance via `transfer`), so `.expect` here is genuinely
/// infallible — mirroring the spec-endorsed `run_pay_wages_system` wrapper convention.
pub fn run_transport_rebate_system(
    tick: Res<Tick>,
    config: Res<EconomyConfig>,
    household: Res<HouseholdSector>,
    mut accounts: ResMut<AccountBook>,
    mut demand: ResMut<DemandPools>,
    mut ledger: ResMut<TradeLedger>,
) {
    if config.macro_flow_interval_ticks == 0
        || !tick.0.is_multiple_of(config.macro_flow_interval_ticks)
    {
        return;
    }
    run_transport_rebate_at_tick(&mut accounts, &mut demand, &household, &mut ledger)
        .expect("run_transport_rebate_at_tick is infallible by construction (drains exactly the held operator balance via transfer); an Err is a bug");
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core pay_wages_then_profit_then_rebate`
Expected: PASS.

- [ ] **Step 5: Run the full systems + wages suites (regression)**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests::systems economy::tests::wages`
Expected: PASS (all). The existing `full_tick_wage_loop_conserves_total_money_auction_path` and `full_tick_macro_flow_feeds_pay_wages_and_conserves` still pass: profit/rebate are additional `transfer`s, so `total_money` is still byte-invariant and the per-tick wage sentinel net-zero assert holds independently (the profit/rebate functions carry their OWN net-zero asserts).

- [ ] **Step 6 (optional, informational — NOT a commit gate): scoped ambiguity probe of just the three writers**

If you want extra confidence that the three writers (`run_pay_wages_system`, `run_distribute_profit_system`, `run_transport_rebate_system`) have no UNORDERED conflict with EACH OTHER, build a MINIMAL schedule containing ONLY those three systems (no CorePlugin/MobilityPlugin) plus their resources, set `ambiguity_detection: LogLevel::Error`, and `schedule.run`. Because the set is scoped to the three writers, a panic implicates exactly them — not an unrelated pre-existing global ambiguity. This is exploratory; do NOT gate the commit on it, and do NOT add a whole-schedule `LogLevel::Error` test. If it surfaces a real wage↔profit↔rebate ambiguity, add the missing `.after`/`.before` edge and re-run Step 4; otherwise discard the probe.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-core/src/economy/systems.rs backend/crates/sim-core/src/economy/tests/systems.rs
git commit -m "feat(economy): wire profit distribution (.after wages) + transport rebate set

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task B6: Plugin-level three-leg `HOUSEHOLD_SECTOR` net-zero + non-vacuity (boundary pinned against real `Tick.0`)

**Files:**
- Test: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/tests/wages.rs`

The three core functions each carry their own `debug_assert` (existing wages, B3 profit, B4 rebate). This task adds a plugin-level test that exercises all three legs in one world, asserts the sentinel nets to zero after EVERY tick (proving the three asserts are independent, not one "over three legs"), and — fixing the boundary-reasoning bug — pins the operator-drained assert against the ACTUAL `world.resource::<Tick>().0` value (NOT the loop counter, which differs because `tick_world` double-increments). The demo macro-flow pair uses ids in the 8_0xx labor-household band for consistency with the rest of the economy.

- [ ] **Step 1: Write the failing test**

Append to `tests/wages.rs`:

```rust
#[test]
fn full_tick_wage_profit_rebate_all_net_household_sector_to_zero() {
    // One world where a firm sells (auction path → revenue → wage + profit) AND transport
    // accrues (a dormant macro-flow pair → operator fee). Run past a macro-flow interval so
    // wage, profit, AND rebate all fire. Assert: total_money byte-invariant every tick AND
    // HOUSEHOLD_SECTOR == 0 after every tick (each of the three legs nets it independently).
    let (mut world, mut schedule) = full_economy_world();

    // Auction-path firm: supplier sells TOOLS at active market m1, consumer buys there.
    let supplier = EconomicActorId(8_001);
    let consumer = EconomicActorId(8_002);
    let m1 = MarketId(1);
    world
        .resource_mut::<InventoryBook>()
        .deposit(supplier, GOOD_TOOLS, Quantity(1_000_000))
        .unwrap();
    world
        .resource_mut::<AccountBook>()
        .deposit(consumer, Money(10_000_000))
        .unwrap();
    world.resource_mut::<SupplyPools>().0.insert(
        supplier,
        SupplyPool {
            actor: supplier,
            market: m1,
            good: GOOD_TOOLS,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world
        .resource_mut::<DemandPools>()
        .0
        .insert(consumer, consumer_pool(consumer, m1));
    world.resource_mut::<crate::economy::Markets>().0.insert(
        m1,
        crate::economy::MarketSite {
            id: m1,
            node_id: crate::routing::NodeId(0),
            name: "M1".to_string(),
        },
    );

    // Dormant macro-flow pair (GOOD_FOOD) → accrues a TRANSPORT_OPERATOR fee each interval.
    // Use ids in the 8_0xx labor band for consistency with the rest of the economy.
    let f_supplier = EconomicActorId(8_041);
    let f_consumer = EconomicActorId(8_042);
    let m_src = MarketId(9_401);
    let m_dst = MarketId(9_402);
    let chunk_src = ChunkCoord { x: 5, y: 5 };
    let chunk_dst = ChunkCoord { x: 9, y: 5 };
    world
        .resource_mut::<InventoryBook>()
        .deposit(f_supplier, GOOD_FOOD, Quantity(1_000_000))
        .unwrap();
    world
        .resource_mut::<AccountBook>()
        .deposit(f_consumer, Money(1_000_000_000))
        .unwrap();
    world.resource_mut::<SupplyPools>().0.insert(
        f_supplier,
        SupplyPool {
            actor: f_supplier,
            market: m_src,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(200),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<DemandPools>().0.insert(
        f_consumer,
        DemandPool {
            actor: f_consumer,
            market: m_dst,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(200),
            max_price: Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
            last_consumed_tick: None,
            income_last_tick: Money::ZERO,
            mpc_bps: 8_000,
            autonomous: Money(5_000),
        },
    );
    world.resource_mut::<MarketChunks>().0.insert(m_src, chunk_src);
    world.resource_mut::<MarketChunks>().0.insert(m_dst, chunk_dst);
    let mut dist = MarketDistances(BTreeMap::new());
    dist.0.insert((m_src, m_dst), 4);
    dist.0.insert((m_dst, m_src), 4);
    world.insert_resource(dist);
    world.resource_mut::<EconomyConfig>().transport_cost_per_tile_unit = Money(50);
    world.spawn((ChunkCoordComp(chunk_src), AsleepChunk));
    world.spawn((ChunkCoordComp(chunk_dst), AsleepChunk));

    // Household pays BOTH consumers (the labor households).
    world.insert_resource(HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(consumer, 1), (f_consumer, 1)]),
    });

    // Seed opening prices for both consumer (market, good)s.
    for (mk, good) in [(m1, GOOD_TOOLS), (m_dst, GOOD_FOOD)] {
        let key = MarketGoodKey { market: mk, good };
        let mut goods = world.resource_mut::<MarketGoods>();
        let state = goods.0.entry(key).or_insert_with(|| MarketGoodState::new(key));
        state.ewma_reference_price = Money(1_000);
        state.last_settlement_price = Money(1_000);
    }

    let before = world.resource::<AccountBook>().total_money().unwrap();
    let interval = world.resource::<EconomyConfig>().macro_flow_interval_ticks;

    // Run enough loop iterations that at least one rebate-boundary tick is reached. Because
    // tick_world DOUBLE-increments Tick (schedule's own tick_increment + the helper's +=1),
    // Tick.0 advances by 2 per iteration, so the loop counter is NOT Tick.0. We therefore
    // pin every boundary assertion against the REAL Tick.0 read inside the loop, never the
    // loop index. Run generously past one interval.
    let mut saw_operator_drained_on_boundary = false;
    for _ in 0..(interval as usize + 4) {
        tick_world(&mut world, &mut schedule);
        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            before,
            "total_money byte-invariant across wage+profit+rebate legs"
        );
        assert_eq!(
            world.resource::<AccountBook>().account(HOUSEHOLD_SECTOR).available,
            Money::ZERO,
            "HOUSEHOLD_SECTOR nets to zero after the full tick (all three legs)"
        );
        // The rebate system gates on Tick.0 % interval == 0 (phase-locked to the operator
        // credit). On any such boundary tick the operator must read zero AFTER the tick.
        let now = world.resource::<crate::mobility::resources::Tick>().0;
        if interval != 0 && now % interval == 0 {
            assert_eq!(
                world.resource::<AccountBook>().account(crate::economy::TRANSPORT_OPERATOR).available,
                Money::ZERO,
                "operator drained at the interval boundary Tick.0={now}"
            );
            saw_operator_drained_on_boundary = true;
        }
    }
    assert!(
        saw_operator_drained_on_boundary,
        "the run must cross at least one rebate boundary (Tick.0 multiple of {interval})"
    );

    // Non-vacuity: all three event kinds must have fired at least once.
    let ev = &world.resource::<TradeLedger>().0;
    assert!(ev.iter().any(|e| matches!(e, EconomyEvent::WagePaid { .. })), "wages fired");
    assert!(ev.iter().any(|e| matches!(e, EconomyEvent::ProfitDistributed { .. })), "profit fired");
    assert!(ev.iter().any(|e| matches!(e, EconomyEvent::TransportRebate { .. })), "rebate fired");
}
```

> The `tick_world`/`full_economy_world` helpers exist at the bottom of `tests/wages.rs`. `ChunkCoord`, `ChunkCoordComp`, `AsleepChunk`, `MarketDistances`, `MarketChunks` are already imported there. `crate::mobility::resources::Tick` is imported at the top of `tests/wages.rs` (line 17).

- [ ] **Step 2: Run test to verify it passes**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core full_tick_wage_profit_rebate_all_net`
Expected: PASS (the implementation from B3/B4/B5 already satisfies it). This is a regression-locking test. If it FAILS, the most likely cause is the operator-credit / rebate phase-lock not aligning — debug via `superpowers:systematic-debugging` before changing behavior. The boundary asserts read `Tick.0` directly, so they are robust to the double-increment; the `saw_operator_drained_on_boundary` guard guarantees non-vacuity (the run actually crosses a boundary). Adjust the test (e.g. extend the loop) if it never crosses a boundary; never change production code to fit it.

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-core/src/economy/tests/wages.rs
git commit -m "test(economy): plugin-level three-leg HOUSEHOLD_SECTOR net-zero + non-vacuity (boundary pinned to Tick.0)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Sub-Slice C — Persistence + Conservation + Stability

### Task C1: Persist `raw_deposits` in `EconomyPersistSnapshot` (no serde-default, one DELETE)

**Files:**
- Modify: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/persist.rs`
- Test: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/tests/persist.rs`

Mirror `production_pools`: a `Vec<(EconomicActorId, RawDeposit)>` field, extracted/applied keys-first. NO `#[serde(default)]` — old rows fail to deserialize, requiring one `DELETE FROM economy_snapshots` before deploy (#69/#73/#74 discipline). `install_economy()` in the persist test uses the full `EconomyPlugin.install`, so `RawDeposits` is already registered (A4) and `extract_from_world` can read it.

- [ ] **Step 1: Write the failing test**

Append to `tests/persist.rs`:

```rust
#[test]
fn raw_deposits_round_trip() {
    use crate::economy::production::{EXTRACTOR, RawDeposit, RawDeposits};
    use crate::economy::GOOD_RAW;

    let mut world = install_economy();
    world.resource_mut::<RawDeposits>().0.insert(
        EXTRACTOR,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(10),
            interval_ticks: 1,
            last_regen_tick: Some(42),
        },
    );

    let snap = extract_from_world(&world);
    assert_eq!(
        snap.raw_deposits,
        vec![(EXTRACTOR, RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(10),
            interval_ticks: 1,
            last_regen_tick: Some(42),
        })],
        "raw_deposits extracted in sorted order"
    );

    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &decoded);

    let restored = fresh.resource::<RawDeposits>().0[&EXTRACTOR];
    assert_eq!(restored.last_regen_tick, Some(42));
    assert_eq!(restored.qty_per_interval, Quantity(10));
    assert_eq!(snap, extract_from_world(&fresh), "full snapshot identity with raw_deposits");
}

#[test]
fn snapshot_without_raw_deposits_field_fails_to_deserialize() {
    // No serde-default: a JSON object missing `raw_deposits` MUST fail (forces the one-time
    // DELETE FROM economy_snapshots before deploy; no silent legacy shim).
    let json = r#"{"accounts":[],"inventory":[],"bids":[],"asks":[],"next_order_id":0,
        "markets":[],"market_goods":[],"demand_pools":[],"supply_pools":[],
        "production_pools":[],"market_chunks":[],"ledger_tail":[],"market_distances":[],
        "household_sector":{"population":0,"pool_weights":[]}}"#;
    let res: Result<EconomyPersistSnapshot, _> = serde_json::from_str(json);
    assert!(res.is_err(), "missing raw_deposits must fail (no serde-default)");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core raw_deposits_round_trip snapshot_without_raw_deposits`
Expected: FAIL — `no field raw_deposits` on `EconomyPersistSnapshot`.

- [ ] **Step 3: Add the field + extract + apply**

In `persist.rs`, extend the existing `crate::economy::{ ... }` import to add `RawDeposit`/`RawDeposits` after `ProductionPool, ProductionPools,`:
```rust
    OrderBook, OrderId, ProductionPool, ProductionPools, RawDeposit, RawDeposits, SupplyPool,
    SupplyPools, TradeLedger,
```

Add the field to `struct EconomyPersistSnapshot` (after `production_pools`):
```rust
    /// The raw-goods faucets (EXTRACTOR + future extractors). Mirrors `production_pools`;
    /// persisted so the `last_regen_tick` cursor survives restart (frozen-time model). New
    /// non-default snapshot field; old rows fail to deserialize (one-time
    /// `DELETE FROM economy_snapshots` before deploy). NO serde-default.
    pub raw_deposits: Vec<(EconomicActorId, RawDeposit)>,
```

In `extract_from_world`, add the resource read (after `let production = world.resource::<ProductionPools>();`):
```rust
    let raw_deposits = world.resource::<RawDeposits>();
```
And add the field to the returned struct literal (after `production_pools: ...`):
```rust
        raw_deposits: raw_deposits.0.iter().map(|(k, v)| (*k, *v)).collect(),
```

In `apply_into_world`, add the resource insert (after the `ProductionPools` insert):
```rust
    world.insert_resource(RawDeposits(snap.raw_deposits.iter().cloned().collect()));
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core raw_deposits_round_trip snapshot_without_raw_deposits`
Expected: PASS (2 tests).

- [ ] **Step 5: Run the full persist suite (regression — covers the existing identity round-trips)**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests::persist`
Expected: PASS (all — the existing identity round-trips still hold; the new field defaults to an empty `Vec` for the empty-economy world and round-trips). Note: the existing persist tests build their snapshot via `extract_from_world(&install_economy())`, so they pick up the new field automatically; any hand-built `EconomyPersistSnapshot { .. }` literal in the test file would need the new field — if a test fails to compile for that reason, add `raw_deposits: vec![]` to that literal.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/economy/persist.rs backend/crates/sim-core/src/economy/tests/persist.rs
git commit -m "feat(economy): persist raw_deposits in EconomyPersistSnapshot (one DELETE, no serde-default)

Requires a one-time DELETE FROM economy_snapshots before deploy.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task C2: `conservation_full_plugin_multi_tick` (money byte-invariant + per-good ledger balance)

**Files:**
- Test: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/tests/conservation.rs`

A plugin-level multi-tick test asserting (a) `total_money()` is byte-invariant every tick, and (b) for each good `g`, the on-hand delta over the run equals `Σ(Regenerated_g + Produced_g) − Σ(Consumed_g + FinalConsumed_g)`, accumulating ledger events tick-by-tick into a `BTreeMap<GoodId, i64>`. Trade-internal moves net to zero (they neither produce nor consume), so they do not appear in the balance. The loop advances Tick by 2 per iteration (`schedule.run` increments via `tick_increment_system`, then the test does `+= 1`), matching the house convention; per-good and money invariants do not depend on the boundary, so they hold every iteration.

- [ ] **Step 1: Write the test**

Append to `tests/conservation.rs` (imports declared at the test-fn level so the existing file header stays untouched):

```rust
#[test]
fn conservation_full_plugin_multi_tick() {
    use bevy_ecs::prelude::*;
    use std::collections::BTreeMap;
    use crate::economy::production::{
        EXTRACTOR, ProductionPool, ProductionPools, RawDeposit, RawDeposits, Recipe,
    };
    use crate::economy::{
        AccountBook, DemandPool, DemandPools, EconomicActorId, EconomyEvent, EconomyPlugin,
        GOOD_RAW, GOOD_TOOLS, GoodId, HouseholdSector, InventoryBook, MarketGoodKey,
        MarketGoodState, MarketGoods, MarketId, MarketSite, Markets, Money, Quantity, SupplyPool,
        SupplyPools, TradeLedger,
    };
    use crate::mobility::resources::Tick;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let consumer = EconomicActorId(8_002);
    let market = MarketId(1);

    world.resource_mut::<RawDeposits>().0.insert(
        EXTRACTOR,
        RawDeposit { good: GOOD_RAW, qty_per_interval: Quantity(10), interval_ticks: 1, last_regen_tick: None },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        EXTRACTOR,
        ProductionPool {
            actor: EXTRACTOR,
            recipe: Recipe { inputs: vec![(GOOD_RAW, Quantity(10))], outputs: vec![(GOOD_TOOLS, Quantity(10))] },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<SupplyPools>().0.insert(
        EXTRACTOR,
        SupplyPool { actor: EXTRACTOR, market, good: GOOD_TOOLS, offered_qty_per_tick: Quantity(10), min_price: Money(500), interval_ticks: 1, last_generated_tick: None },
    );
    world.resource_mut::<AccountBook>().deposit(consumer, Money(10_000_000)).unwrap();
    world.resource_mut::<DemandPools>().0.insert(
        consumer,
        DemandPool {
            actor: consumer, market, good: GOOD_TOOLS,
            desired_qty_per_tick: Quantity(10), max_price: Money(2_000),
            urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1,
            last_generated_tick: None, last_consumed_tick: None,
            income_last_tick: Money::ZERO, mpc_bps: 8_000, autonomous: Money(5_000),
        },
    );
    world.resource_mut::<Markets>().0.insert(
        market,
        MarketSite { id: market, node_id: crate::routing::NodeId(0), name: "M1".to_string() },
    );
    world.insert_resource(HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(consumer, 1_i64)]),
    });
    {
        let key = MarketGoodKey { market, good: GOOD_TOOLS };
        let mut goods = world.resource_mut::<MarketGoods>();
        let st = goods.0.entry(key).or_insert_with(|| MarketGoodState::new(key));
        st.ewma_reference_price = Money(1_000);
        st.last_settlement_price = Money(1_000);
    }

    let goods_to_track = [GOOD_RAW, GOOD_TOOLS];
    let money_before = world.resource::<AccountBook>().total_money().unwrap();
    let mut good_before: BTreeMap<GoodId, i64> = BTreeMap::new();
    for g in goods_to_track {
        good_before.insert(g, world.resource::<InventoryBook>().total_good(g).unwrap().0);
    }

    let mut net_ledger: BTreeMap<GoodId, i64> = BTreeMap::new();
    let mut last_seen = 0usize;
    let n = 60u64;
    for _ in 0..n {
        schedule.run(&mut world);
        world.resource_mut::<Tick>().0 += 1;
        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            money_before,
            "total_money byte-invariant every tick"
        );
        let ledger = world.resource::<TradeLedger>();
        for e in &ledger.0[last_seen..] {
            match e {
                EconomyEvent::Regenerated { good, qty, .. } => *net_ledger.entry(*good).or_insert(0) += qty.0,
                EconomyEvent::Produced { good, qty, .. } => *net_ledger.entry(*good).or_insert(0) += qty.0,
                EconomyEvent::Consumed { good, qty, .. } => *net_ledger.entry(*good).or_insert(0) -= qty.0,
                EconomyEvent::FinalConsumed { good, qty, .. } => *net_ledger.entry(*good).or_insert(0) -= qty.0,
                _ => {}
            }
        }
        last_seen = ledger.0.len();
    }

    for g in goods_to_track {
        let after = world.resource::<InventoryBook>().total_good(g).unwrap().0;
        let delta = after - good_before[&g];
        let from_events = *net_ledger.get(&g).unwrap_or(&0);
        assert_eq!(
            delta, from_events,
            "per-good balance for {g:?}: on-hand delta == Σ(Regen+Produced) − Σ(Consumed+FinalConsumed)"
        );
    }

    assert!(
        *net_ledger.get(&GOOD_RAW).unwrap_or(&0) != 0 || *net_ledger.get(&GOOD_TOOLS).unwrap_or(&0) != 0,
        "the conservation test must be non-vacuous (goods flowed)"
    );
    assert!(
        world.resource::<TradeLedger>().0.iter().any(|e| matches!(e, EconomyEvent::Regenerated { .. })),
        "EXTRACTOR regenerated RAW"
    );
    assert!(
        world.resource::<TradeLedger>().0.iter().any(|e| matches!(e, EconomyEvent::Produced { good, .. } if *good == GOOD_TOOLS)),
        "the recipe produced TOOLS"
    );
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core conservation_full_plugin_multi_tick`
Expected: PASS. If a per-good balance assert fails, that is a genuine conservation bug — debug with `superpowers:systematic-debugging`, do NOT loosen the assert. If the `total_money` invariant fails, a money leg is minting/burning (re-audit B3/B4).

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-core/src/economy/tests/conservation.rs
git commit -m "test(economy): full-plugin multi-tick money + per-good conservation

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task C3: `steady_state_multi_tick` (EXTRACTOR-only, non-vacuous, bands; caveats in prose)

**Files:**
- Test: `/Users/ramonfuglister/Coding/abutown-vtraders/backend/crates/sim-core/src/economy/tests/conservation.rs`

The actual proof of self-sustainment (§8). `EXTRACTOR` is the ONLY supplier in this test world (no finite 1M endowment to mask the steady state). `N ≥ 200` loop iterations. Hard asserts: (a) `total_money` constant every tick; (b) the EXTRACTOR firm balance bounded over the last K iterations (`max−min < ε` — catches unbounded retained earnings; with full distribution it nets ~0); (c) a representative consumer ACCOUNT balance AND a market `traded_qty_last_tick` each within a committed `[lo, hi]` band with `lo > 0` (living, not frozen); (d) `total_good(GOOD_TOOLS)` bounded (not monotonically growing/collapsing).

> §8 caveats: the capped-price-regulator caveat (static seed prices do NOT self-correct chronic scarcity) and the autonomous-floor caveat (consumer demand has a non-zero autonomous floor that keeps the loop from freezing) are stated BOTH as code comments here AND, per §8, in the PR body / design-decisions note (see the end of this plan). The bands below are pinned a priori to the compiled-in defaults; the first honest run confirms or recalibrates them within the non-vacuity guardrails (`lo > 0` and money-constant are inviolate).

- [ ] **Step 1: Write the test**

Append to `tests/conservation.rs`:

```rust
#[test]
fn steady_state_multi_tick() {
    use bevy_ecs::prelude::*;
    use std::collections::BTreeMap;
    use crate::economy::production::{
        EXTRACTOR, ProductionPool, ProductionPools, RawDeposit, RawDeposits, Recipe,
    };
    use crate::economy::{
        AccountBook, DemandPool, DemandPools, EconomicActorId, EconomyEvent, EconomyPlugin,
        GOOD_RAW, GOOD_TOOLS, HouseholdSector, InventoryBook, MarketGoodKey, MarketGoodState,
        MarketGoods, MarketId, MarketSite, Markets, Money, Quantity, SupplyPool, SupplyPools,
        TradeLedger,
    };
    use crate::mobility::resources::Tick;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;

    // §8 CAVEATS (also carried into the PR body):
    //  - Capped-price regulator: seed prices are static opening values; the auction EWMA
    //    smooths but does NOT self-correct chronic scarcity. The §15.2 Sizing-Sim (A5a)
    //    sized the faucet to cover aggregate demand precisely so scarcity does not arise.
    //  - Autonomous floor: each consumer pool has a non-zero `autonomous` demand term, so
    //    consumption never collapses to zero even at a transient income dip — this is what
    //    keeps the steady state "living" (lo > 0) rather than freezing.

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let consumer = EconomicActorId(8_002);
    let market = MarketId(1);

    // EXTRACTOR is the ONLY supplier (no finite 1M endowment to mask steady state).
    world.resource_mut::<RawDeposits>().0.insert(
        EXTRACTOR,
        RawDeposit { good: GOOD_RAW, qty_per_interval: Quantity(10), interval_ticks: 1, last_regen_tick: None },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        EXTRACTOR,
        ProductionPool {
            actor: EXTRACTOR,
            recipe: Recipe { inputs: vec![(GOOD_RAW, Quantity(10))], outputs: vec![(GOOD_TOOLS, Quantity(10))] },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<SupplyPools>().0.insert(
        EXTRACTOR,
        SupplyPool { actor: EXTRACTOR, market, good: GOOD_TOOLS, offered_qty_per_tick: Quantity(10), min_price: Money(500), interval_ticks: 1, last_generated_tick: None },
    );
    // Consumer cash: ample but FINITE — the loop must recycle, not just drain a hoard.
    world.resource_mut::<AccountBook>().deposit(consumer, Money(1_000_000)).unwrap();
    world.resource_mut::<DemandPools>().0.insert(
        consumer,
        DemandPool {
            actor: consumer, market, good: GOOD_TOOLS,
            desired_qty_per_tick: Quantity(0), // bootstrapped from autonomous at tick 0
            max_price: Money(2_000),
            urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1,
            last_generated_tick: None, last_consumed_tick: None,
            income_last_tick: Money::ZERO, mpc_bps: 8_000, autonomous: Money(5_000),
        },
    );
    world.resource_mut::<Markets>().0.insert(
        market,
        MarketSite { id: market, node_id: crate::routing::NodeId(0), name: "M1".to_string() },
    );
    world.insert_resource(HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(consumer, 1_i64)]),
    });
    {
        let key = MarketGoodKey { market, good: GOOD_TOOLS };
        let mut goods = world.resource_mut::<MarketGoods>();
        let st = goods.0.entry(key).or_insert_with(|| MarketGoodState::new(key));
        st.ewma_reference_price = Money(1_000);
        st.last_settlement_price = Money(1_000);
    }

    let money_before = world.resource::<AccountBook>().total_money().unwrap();

    let n: usize = 240;
    let k: usize = 50; // tail window (iterations)
    let mut consumer_bal_tail: Vec<i64> = Vec::new();
    let mut ext_bal_tail: Vec<i64> = Vec::new();
    let mut traded_tail: Vec<i64> = Vec::new();
    let mut tools_total_tail: Vec<i64> = Vec::new();

    for i in 0..n {
        schedule.run(&mut world);
        world.resource_mut::<Tick>().0 += 1;
        // (a) money constant EVERY tick.
        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            money_before,
            "total_money constant in steady state (iter {i})"
        );
        if i >= n - k {
            let accounts = world.resource::<AccountBook>();
            consumer_bal_tail.push(accounts.account(consumer).available.0);
            // EXTRACTOR is the sole firm seller. With full distribution it nets ~0 each tick;
            // the tail spread bounds "no unbounded retained earnings".
            ext_bal_tail.push(accounts.account(EXTRACTOR).available.0);

            let key = MarketGoodKey { market, good: GOOD_TOOLS };
            let traded = world.resource::<MarketGoods>().0.get(&key).map(|s| s.traded_qty_last_tick.0).unwrap_or(0);
            traded_tail.push(traded);

            tools_total_tail.push(world.resource::<InventoryBook>().total_good(GOOD_TOOLS).unwrap().0);
        }
    }

    let min = |v: &[i64]| *v.iter().min().unwrap();
    let max = |v: &[i64]| *v.iter().max().unwrap();

    // (b) EXTRACTOR balance bounded over the tail (no unbounded retained earnings).
    // Full distribution ⇒ the firm nets to ~zero each tick; allow a small epsilon for
    // intra-tick rounding remainders left by floor wage/dividend (never minted).
    let seller_eps: i64 = 1_000;
    assert!(
        max(&ext_bal_tail) - min(&ext_bal_tail) < seller_eps,
        "EXTRACTOR balance bounded over tail (max-min={} < {seller_eps}); tail={:?}",
        max(&ext_bal_tail) - min(&ext_bal_tail),
        ext_bal_tail
    );

    // (c1) consumer ACCOUNT balance lives in a committed band [lo,hi], lo>0, hi/lo < r.
    let cons_lo = min(&consumer_bal_tail);
    let cons_hi = max(&consumer_bal_tail);
    assert!(cons_lo > 0, "consumer never drains to zero (living loop); lo={cons_lo}");
    assert!(
        (cons_hi as i128) < (cons_lo as i128) * 4,
        "consumer balance band ratio hi/lo < 4 (hi={cons_hi}, lo={cons_lo})"
    );

    // (c2) market traded_qty lives in a band [lo,hi], lo>0.
    let tr_lo = min(&traded_tail);
    let tr_hi = max(&traded_tail);
    assert!(tr_lo > 0, "market traded every tick in steady state (lo={tr_lo})");
    assert!(
        (tr_hi as i128) < (tr_lo as i128) * 5,
        "traded_qty band ratio hi/lo < 5 (hi={tr_hi}, lo={tr_lo})"
    );

    // (d) total_good(GOOD_TOOLS) bounded (not monotonic growth/collapse). With regen=10 and
    // consumption ~10/tick the on-hand TOOLS stays small and bounded.
    let tools_lo = min(&tools_total_tail);
    let tools_hi = max(&tools_total_tail);
    assert!(
        tools_hi - tools_lo < 10_000,
        "TOOLS on-hand bounded over tail (hi={tools_hi}, lo={tools_lo})"
    );

    // Non-vacuity: regen + production + trades all occurred.
    let ev = &world.resource::<TradeLedger>().0;
    assert!(ev.iter().any(|e| matches!(e, EconomyEvent::Regenerated { .. })), "regen fired");
    assert!(ev.iter().any(|e| matches!(e, EconomyEvent::Produced { good, .. } if *good == GOOD_TOOLS)), "tools produced");
    assert!(ev.iter().any(|e| matches!(e, EconomyEvent::Trade { .. })), "trades cleared");
}
```

> The numeric band thresholds (`seller_eps`, the `hi/lo` ratios, the TOOLS bound) are PINNED to the compiled-in defaults (`labor_share_bps=6_000`, `dividend_share_bps=10_000`, `mpc_bps=8_000`, `autonomous=5_000`, `min_price=500`, `max_price=2_000`, regen `10/tick`, opening price `1_000`). The TDD loop for the bands is honestly observe-then-pin: if the FIRST run shows the actual tail values fall just outside these bands while the loop is genuinely stable (money constant, `lo > 0`, bounded), tighten/relax ONLY the epsilon/ratio constants to the observed-but-non-vacuous values — never weaken `lo > 0` or the money-constant assert, and never change production code to fit a band. Capture the observed tail (the asserts print the values) and commit the pinned constants with a comment citing the observed range. If `lo == 0` (consumer drains, or no trade in some tail iteration) that is a REAL stability failure: debug the loop closure (re-audit B3/B4/B5 wiring and the A5a regen sizing), do not patch the test.

- [ ] **Step 2: Run the test (background + poll)**

Run in background: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core steady_state_multi_tick`
Expected: PASS. 240 iterations of the full schedule is fast, but route through the serial wrapper and poll per CLAUDE.md. If it fails on a band, follow the Step 1 calibration note (observe the printed values, pin the constants, keep `lo > 0` and money-constant inviolate).

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-core/src/economy/tests/conservation.rs
git commit -m "test(economy): non-vacuous steady-state (EXTRACTOR-only, living bands, §8 caveats in prose)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task C4: Full-gate regression + format

**Files:** (no source changes unless the gate finds an issue)

- [ ] **Step 1: Format check**

Run: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml -- --check`
Expected: clean. If it reports diffs, run `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml` and re-check. (Per MEMORY: local rust toolchain must be at least CI's `@stable`; if only fmt is skewed, `rustup update stable` first, then reformat and re-gate.)

- [ ] **Step 2: Clippy (scoped, background + poll)**

Run in background: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml -p sim-core --all-targets -- -D warnings`
Expected: clean. Fix any new warnings and re-run.

- [ ] **Step 3: Full sim-core test suite (background + poll)**

Run in background: `export TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target; scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core`
Expected: PASS (all). This is the only broad run; it is scoped to `-p sim-core` (NOT `--workspace --all-targets`), serialized via the wrapper, and run once at the end of iteration per CLAUDE.md.

- [ ] **Step 4: Commit any fixes from the gate**

```bash
git add -A
git commit -m "chore(economy): fmt + clippy clean for self-sustaining-loop slice

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Deployment + design-decisions note (carry verbatim into the PR body)

1. **One DELETE before deploy.** This slice adds exactly ONE new persisted snapshot field (`raw_deposits`) with NO serde-default. Run a one-time `DELETE FROM economy_snapshots` BEFORE deploying so the server hydrates a fresh world (which re-seeds the `EXTRACTOR`); without the DELETE, old rows missing `raw_deposits` will fail to deserialize. This mirrors the #69/#73/#74 discipline.

2. **FOOD stays on the finite endowment (§15.2 decision, recorded).** Only RAW→TOOLS is made self-sustaining. The seeded FOOD suppliers (8_011 @ m_a, 8_021 @ m_fa) keep their 1M endowment and are intentionally NOT converted to a RAW→FOOD extractor in this slice. The steady-state test is EXTRACTOR/TOOLS-only precisely so the draining FOOD endowment cannot mask the closed loop. A second RAW→FOOD extractor is a deferred follow-up.

3. **§8 steady-state caveats (in prose AND in the C3 test).** (a) Capped-price regulator: seed prices are static opening values; the auction EWMA smooths but does NOT self-correct chronic scarcity, which is why the §15.2 Sizing-Sim (Task A5a) sized the faucet to cover aggregate TOOLS demand exactly. (b) Autonomous floor: each consumer pool carries a non-zero `autonomous` demand term, so consumption never collapses to zero — this is what keeps the steady state living (`lo > 0`) rather than freezing.

4. **Full profit distribution, no capitalist class.** `dividend_share_bps` defaults to 10_000: 100% of profit (revenue − wage) flows to the existing three labor households via the same `pool_weights`/`apportion_cash` path as wages. Firms net to zero each tick (no retained earnings). The distribution is FALLIBLE and AUDITED: an underfunded firm books only the covered amount and surfaces a `MarketClearFailed`-style event — never `.expect`-panic, never a silent skip. The transport rebate likewise returns the operator fee to the labor households, phase-locked to the macro-flow interval boundary (no persisted cursor).

## Self-review (performed against the spec + the issue list)

- **§4 (goods source):** A1–A5 cover `GOOD_RAW(5)`, `EXTRACTOR(8_031)`, `RawDeposit`/`RawDeposits`, `run_regen_at_tick` (keys-first, interval gate, flow-capped, stamp-on-fire), `Regenerated` event, `EconomySet::Regenerate` between `ExpireOrders` and `Production`, the EXTRACTOR seed, and the `mod.rs` resource registration. Input-gated throttle + RAW-never-listed asserted (A4 step 1, A5 step 1).
- **§15.2 Sizing-Sim (was missing):** A5a is a dedicated sub-task that measures aggregate seed TOOLS demand (10/tick) and asserts `REGEN_QTY=10` covers it BEFORE A5 fixes the constant. The FOOD-on-endowment decision is recorded in A5a, in the seed comment, and in the PR body.
- **No-fallback (was violated in A4):** `run_regen_system` now uses `.expect(...)` (matching the spec-endorsed wages/consumption-update convention), NOT `let _`. Profit distribution is fallible+audited (B3) and its wrapper surfaces a whole-call Err as `MarketClearFailed`, never `let _`/`.expect`. Transport rebate is conservative-by-construction so its wrapper `.expect` is acceptable.
- **Consistency (was wrong):** A5 uses the REAL builder `seed_world()` and EXPLICITLY calls `seed_demo_economy(&mut world)`; it ALSO adds `ProductionPools`+`RawDeposits` to `seed_world()` so the three existing seed tests don't panic. The duplicate-import risk is avoided (separate `use crate::economy::production::{...}` line; `GOOD_RAW` inserted into the existing `GOOD_*` import; `EconomicActorId` not re-listed).
- **Tick double-increment (was unreconciled):** B6 and C2/C3 reason against the actual `world.resource::<Tick>().0` value, never the loop counter. B6's operator-drained assert is gated on `Tick.0 % interval == 0` and guarded for non-vacuity (`saw_operator_drained_on_boundary`). The B6 macro-flow demo pair moved to the 8_0xx band (8_041/8_042) for id consistency.
- **Granularity (was too broad):** B5's primary ordering gate is the scoped recorder-into-set test; the whole-schedule `LogLevel::Error` ambiguity gate is removed and demoted to an optional, scoped, informational probe (Step 6) that is NOT a commit gate.
- **§5–§9 (money + ordering):** B2 (`dividend_share_bps`+validator), B3 (audited profit dist + `ProfitDistributed`), B4 (rebate + `TransportRebate`), B5 (`EconomySet::TransportRebate`, profit `.after` wages, macro-flow-modulo gate phase-locked to the operator credit, no cursor). Ordering `wage→profit→rebate→consume` pinned by the recorder test. Three independent net-zero `debug_assert`s confirmed by B6.
- **§7/§11 (conservation/persistence):** C1 (`raw_deposits`, one DELETE, no serde-default, missing-field-fails test), C2 (money byte-invariant + per-good ledger balance), C3 (EXTRACTOR-only, N=240, money-constant, firm-bounded, consumer-account + traded_qty living bands with `lo>0`, TOOLS bounded, non-vacuous, §8 caveats in prose + test).
- **Verified canonical signatures** against the real code: `apportion_cash(weights:&[i64], total:i64)->Vec<i64>`; `wage_for_revenue` is `pub(crate)` reachable from the new fn (same module); `HOUSEHOLD_SECTOR=u64::MAX-1`, `TRANSPORT_OPERATOR=u64::MAX`; `EconomyError::{Overflow, InsufficientFunds, InvalidOrder}` (and serde-derived); `Recipe`/`ProductionPool` shapes; `macro_flow` operator credit at line 712 is inside the `is_multiple_of(macro_flow_interval_ticks)` gate (873-877); `transfer` net-zero conservation; `interval_elapsed`; `InventoryBook::{deposit, balance, total_good}`; `AccountBook::{account, deposit, transfer, total_money}`; `MarketGoodState::new`. `GOOD_RAW=GoodId(5)` is the next free id.
