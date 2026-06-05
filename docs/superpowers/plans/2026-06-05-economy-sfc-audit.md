# Release-Grade SFC Conservation Audit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enforce + observe `total_money` byte-invariance at runtime (release too): a per-tick audit that fail-fasts on any conservation drift, emits a queryable `TickAudit` event, and upgrades the `HOUSEHOLD_SECTOR` net-zero sentinels from `debug_assert` to release-grade.

**Architecture:** One pure fn (`audit.rs::run_tick_audit_at_tick`) + an ephemeral `LastTickMoney` baseline resource + a new `EconomySet::TickAudit` system that runs LAST (after `UpdateConsumption`, before `tick_increment`) and `.expect`-panics on drift. Plus two `debug_assert`→release-grade sentinel upgrades. Mutates only the ledger + the ephemeral baseline — moves no money, changes no economic behavior. NO new persisted field, NO DELETE migration.

**Tech Stack:** Rust (bevy_ecs 0.18), `sim-core`; fixed-point i64/i128; TDD via `cargo test`.

**Spec:** `docs/superpowers/specs/2026-06-05-economy-sfc-audit-design.md`

---

## Verified Facts (pinned against the real code — do not re-derive)

**`EconomyError` (`economy/money.rs:4-12`):** `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] pub enum EconomyError { Overflow, NegativeMoney, NegativeQuantity, ZeroPrice, InsufficientFunds, InsufficientGoods, InvalidOrder }`. (NOT persisted as a standalone snapshot field; safe to extend.)

**`total_money` (`economy/accounts.rs:97`):** `pub fn total_money(&self) -> Result<Money, EconomyError>` — sums `available + locked` per account via `checked_add` (so the locked leg is counted → `lock`/`release`/`debit_locked` are conservation-neutral). Returns `Err(Overflow)` on sum overflow.

**`EconomyEvent` (`economy/ledger.rs:6-7`):** `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub enum EconomyEvent { … TransportRebate { amount: Money } }`. The `event_type(&self) -> &'static str` match (ledger.rs:120-141) is exhaustive (adding a variant forces a new arm). `TradeLedger(pub Vec<EconomyEvent>)` is the Resource (ledger.rs:145). `Money` + `u64` are both `Serialize`/`Copy`.

**`audit.rs`:** holds `LedgerAuditCursor(pub usize)` (#68) + flush helpers; imports `EconomyEvent, PERSISTED_LEDGER_TAIL, TradeLedger` + `crate::mobility::resources::Tick`. The natural home for `run_tick_audit_at_tick` + `LastTickMoney`.

**Sentinels (`economy/wages.rs`):** `run_pay_wages_at_tick` (fn at wages.rs:77) ends with `debug_assert_eq!(accounts.account(HOUSEHOLD_SECTOR).available, Money::ZERO, "HOUSEHOLD_SECTOR must net to zero after PayWages (sentinel-stranded cash)")` at **wages.rs:147-151**, then `Ok(())`. `run_distribute_profit_at_tick` (fn at wages.rs:172) ends with the analogous `debug_assert_eq!` at **wages.rs:232-236**, then `Ok(())`. Both fns return `Result<(), EconomyError>`.

**Wrappers (`economy/systems.rs`):**
- `run_pay_wages_system` ends with `.expect("run_pay_wages_at_tick is infallible …; an Err is a bug")` (systems.rs:508) → a sentinel `Err` here **already** fail-fasts (panics). NO wrapper change needed for the wage path.
- `run_distribute_profit_system` (systems.rs:516-538) deliberately does NOT `.expect` — on `Err` it pushes `EconomyEvent::MarketClearFailed { market: MarketId(0), good: GoodId(0), reason }` and continues (the profit shortfall is genuinely fallible per #75). This MUST be changed to fail-fast SPECIFICALLY on `ConservationViolation` while keeping the soft path for other reasons.

**Schedule (`economy/systems.rs`):** the `EconomySet` enum tail is `… Telemetry, AdjustReservationPrices, UpdateConsumption` (#77 added `AdjustReservationPrices`). `install_systems` `configure_sets((…, EconomySet::Telemetry, EconomySet::AdjustReservationPrices, EconomySet::UpdateConsumption).chain())` — **the `.chain()` enforces set order** (so adding `TickAudit` as the new LAST chain element is sufficient for ordering). There are **FIVE** `add_systems` calls: the MAIN chained tuple (whose tail registrations are `update_market_telemetry_system.in_set(Telemetry)`, `run_adjust_reservation_prices_system.in_set(AdjustReservationPrices)`, `run_consumption_update_system.in_set(UpdateConsumption)`), plus four standalone ones (`run_distribute_profit_system.in_set(PayWages)`, shopper, commuter, materialize). The audit system goes in the **MAIN chained tuple** (after `run_consumption_update_system`). The set-chain guarantees `TickAudit` (chain pos 18) runs after ALL money moves — incl. `run_distribute_profit_system` which lives in `EconomySet::PayWages` (chain pos 9). No money moves in Telemetry/AdjustReservationPrices/UpdateConsumption, so a system after `UpdateConsumption` sees the tick's final money.

**Resource registration (`economy/mod.rs:60-91`):** `EconomyPlugin::install` does `world.insert_resource(...)` for each resource (incl. `crate::economy::audit::LedgerAuditCursor::default()` at mod.rs:78), then `install_systems(schedule)` at mod.rs:91.

**No runtime minting (verified by the spec review):** `total_money` is constant after seed — every runtime money path uses `transfer`/`lock_cash`/`release_cash`/`debit_locked`+netting-`deposit`; only `seed.rs` mints (once, before tick 0). So the tick-over-tick equality check cannot panic spuriously.

**Cargo (MANDATORY — isolated target + serial lock; the /tmp target was cleaned, so the FIRST run rebuilds from scratch — expect a few minutes):**
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core <TESTNAME>
```
from `/Users/ramonfuglister/Coding/abutown-vtraders`. Never `--workspace --all-targets` during iteration. `mkdir -p /tmp/abutown-vtraders-tmp` if missing. **fmt:** `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check`.

---

## Sub-Slice A — Error variant + TickAudit event + LastTickMoney resource

### Task A1: `EconomyError::ConservationViolation` + `EconomyEvent::TickAudit` + `LastTickMoney`

**Files:** Modify `backend/crates/sim-core/src/economy/money.rs`, `ledger.rs`, `audit.rs`, `mod.rs`; Test `backend/crates/sim-core/src/economy/tests/audit.rs`.

- [ ] **Step 1: Write the failing test** (append to `tests/audit.rs`):

```rust
#[test]
fn sfc_audit_primitives_exist() {
    use crate::economy::audit::LastTickMoney;
    use crate::economy::{EconomicActorId as _, EconomyError, EconomyEvent, Money};
    // The new honest error variant.
    let _ = EconomyError::ConservationViolation;
    // The TickAudit event + its stable tag.
    let e = EconomyEvent::TickAudit { tick: 7, total_money: Money(12_345) };
    assert_eq!(e.event_type(), "tick_audit");
    // The ephemeral baseline defaults to None.
    assert_eq!(LastTickMoney::default().0, None);
}
```

- [ ] **Step 2: Run it — verify it FAILS** (variant/event/resource don't exist):

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core sfc_audit_primitives_exist
```
Expected: FAIL — `no variant ConservationViolation` / `no variant TickAudit` / `no LastTickMoney`.

- [ ] **Step 3: Add `ConservationViolation`** to `EconomyError` (money.rs, after `InvalidOrder,`):

```rust
    /// A runtime SFC conservation invariant was violated (money minted/destroyed, or a
    /// net-zero sentinel held stranded cash). UNRECOVERABLE — surfaced fail-fast.
    ConservationViolation,
```

- [ ] **Step 4: Add the `TickAudit` variant** to `EconomyEvent` (ledger.rs, after the `TransportRebate { amount: Money }` variant, before the enum's closing `}`):

```rust
    /// Per-tick SFC conservation heartbeat: the total money in circulation at the end of
    /// this tick. Emitted every tick by the audit system; the queryable conservation trace.
    TickAudit { tick: u64, total_money: Money },
```

- [ ] **Step 5: Add the `event_type` arm** (ledger.rs, in the match, after the `TransportRebate` arm):

```rust
            Self::TickAudit { .. } => "tick_audit",
```

- [ ] **Step 6: Add the `LastTickMoney` resource** to `audit.rs` (after the `LedgerAuditCursor` definition). First extend the `use crate::economy::{…}` line to also import `Money`:

```rust
/// The previous tick's `total_money`, for the per-tick byte-invariance check. EPHEMERAL —
/// NOT persisted (re-initialized from the restored, conserved `total_money` on the first audit
/// tick after a hydrate, so it stays consistent across restarts without a snapshot field).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LastTickMoney(pub Option<crate::economy::Money>);
```

- [ ] **Step 7: Register the resource** in `mod.rs` `EconomyPlugin::install` (after the `LedgerAuditCursor` insert at mod.rs:78):

```rust
        world.insert_resource(crate::economy::audit::LastTickMoney::default());
```
(`mod.rs` already has `pub use audit::*`, so `LastTickMoney` is reachable at both `crate::economy::audit::LastTickMoney` and `crate::economy::LastTickMoney` — no extra re-export needed.)

- [ ] **Step 8: Run it — verify it PASSES**

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core sfc_audit_primitives_exist
```
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add backend/crates/sim-core/src/economy/money.rs backend/crates/sim-core/src/economy/ledger.rs backend/crates/sim-core/src/economy/audit.rs backend/crates/sim-core/src/economy/mod.rs backend/crates/sim-core/src/economy/tests/audit.rs
git commit -m "feat(economy): ConservationViolation error + TickAudit event + LastTickMoney resource"
```

---

## Sub-Slice B — Audit system + schedule + drift fail-fast

### Task B1: `run_tick_audit_at_tick` pure core + tests

**Files:** Modify `backend/crates/sim-core/src/economy/audit.rs`; Test `backend/crates/sim-core/src/economy/tests/audit.rs`.

- [ ] **Step 1: Write the failing tests** (append to `tests/audit.rs`):

```rust
#[test]
fn tick_audit_emits_event_and_tracks_baseline_when_conserved() {
    use crate::economy::audit::{run_tick_audit_at_tick, LastTickMoney};
    use crate::economy::{AccountBook, EconomicActorId, EconomyEvent, Money, TradeLedger};
    let mut accounts = AccountBook::default();
    accounts.deposit(EconomicActorId(1), Money(1_000)).unwrap();
    let mut ledger = TradeLedger::default();
    let mut last = LastTickMoney::default();

    // First tick: no prior baseline → initialize, emit event, no check.
    run_tick_audit_at_tick(&accounts, &mut ledger, &mut last, 0).unwrap();
    assert_eq!(last.0, Some(Money(1_000)));
    assert_eq!(ledger.0, vec![EconomyEvent::TickAudit { tick: 0, total_money: Money(1_000) }]);

    // Second tick, unchanged total → Ok, second event, baseline unchanged.
    run_tick_audit_at_tick(&accounts, &mut ledger, &mut last, 1).unwrap();
    assert_eq!(last.0, Some(Money(1_000)));
    assert_eq!(ledger.0.len(), 2);
}

#[test]
fn tick_audit_returns_err_on_money_drift() {
    use crate::economy::audit::{run_tick_audit_at_tick, LastTickMoney};
    use crate::economy::{AccountBook, EconomicActorId, EconomyError, Money, TradeLedger};
    let mut accounts = AccountBook::default();
    accounts.deposit(EconomicActorId(1), Money(1_000)).unwrap();
    let mut ledger = TradeLedger::default();
    let mut last = LastTickMoney::default();
    run_tick_audit_at_tick(&accounts, &mut ledger, &mut last, 0).unwrap(); // baseline = 1_000

    // Mint money (a conservation violation) → next audit detects drift.
    accounts.deposit(EconomicActorId(2), Money(500)).unwrap();
    let r = run_tick_audit_at_tick(&accounts, &mut ledger, &mut last, 1);
    assert_eq!(r, Err(EconomyError::ConservationViolation));
}
```

- [ ] **Step 2: Run them — verify they FAIL** (fn doesn't exist):

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core tick_audit
```
Expected: FAIL — `cannot find function run_tick_audit_at_tick`.

- [ ] **Step 3: Add the pure core** to `audit.rs`. Extend the `use crate::economy::{…}` import to add `AccountBook, EconomyError, Money` (keep the existing `EconomyEvent, PERSISTED_LEDGER_TAIL, TradeLedger`), then:

```rust
/// Per-tick SFC conservation audit (pure over its refs). Reads `total_money` (= Σ available+locked,
/// which is byte-CONSTANT after seed since every runtime money move is a conservative transfer),
/// asserts it equals the prior tick's value (drift ⇒ `Err(ConservationViolation)`), emits a
/// `TickAudit` heartbeat event, and updates the ephemeral baseline. Moves NO money. Deterministic.
pub fn run_tick_audit_at_tick(
    accounts: &AccountBook,
    ledger: &mut TradeLedger,
    last: &mut LastTickMoney,
    current_tick: u64,
) -> Result<(), EconomyError> {
    let total = accounts.total_money()?;
    if let Some(prev) = last.0 {
        if total != prev {
            return Err(EconomyError::ConservationViolation);
        }
    }
    ledger.0.push(EconomyEvent::TickAudit { tick: current_tick, total_money: total });
    last.0 = Some(total);
    Ok(())
}
```

- [ ] **Step 4: Run them — verify they PASS**

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core tick_audit
```
Expected: PASS (both).

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/audit.rs backend/crates/sim-core/src/economy/tests/audit.rs
git commit -m "feat(economy): run_tick_audit_at_tick (total_money byte-invariance + TickAudit heartbeat)"
```

### Task B2: Wire `EconomySet::TickAudit` (fail-fast wrapper) + full-plugin tests

**Files:** Modify `backend/crates/sim-core/src/economy/systems.rs`; Test `backend/crates/sim-core/src/economy/tests/audit.rs`.

- [ ] **Step 1: Write the failing tests** (append to `tests/audit.rs`). The first proves the audit fires each tick under the full plugin; the second proves the wrapper fail-fasts on an injected drift.

```rust
#[test]
fn tick_audit_fires_every_tick_under_full_plugin() {
    use crate::economy::systems::{run_tick_audit_system, EconomySet};
    use crate::economy::{EconomyEvent, EconomyPlugin, TradeLedger};
    use crate::mobility::resources::Tick;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    use bevy_ecs::prelude::*;

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);
    world.insert_resource(Tick(0));

    for _ in 0..3 {
        schedule.run(&mut world);
        world.resource_mut::<Tick>().0 += 1;
    }
    let n_audit = world.resource::<TradeLedger>().0.iter()
        .filter(|e| matches!(e, EconomyEvent::TickAudit { .. })).count();
    assert!(n_audit >= 3, "a TickAudit event per tick: {n_audit}");
    let _ = (run_tick_audit_system, EconomySet::TickAudit);
}

#[test]
#[should_panic(expected = "CONSERVATION VIOLATION")]
fn injected_money_drift_panics_the_audit() {
    use crate::economy::{AccountBook, EconomicActorId, EconomyPlugin, Money};
    use crate::mobility::resources::Tick;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    use bevy_ecs::prelude::*;

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);
    world.insert_resource(Tick(0));

    schedule.run(&mut world); // tick 0: audit sets the baseline
    world.resource_mut::<Tick>().0 += 1;
    // MINT money (a conservation violation) between ticks.
    world.resource_mut::<AccountBook>().deposit(EconomicActorId(42), Money(1_000)).unwrap();
    schedule.run(&mut world); // tick 1: audit sees total changed → .expect panics
}
```

- [ ] **Step 2: Run them — verify they FAIL** (set/system don't exist):

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core tick_audit_fires injected_money_drift
```
Expected: FAIL — `no variant TickAudit` on `EconomySet` / `cannot find function run_tick_audit_system`.

- [ ] **Step 3: Add the `EconomySet::TickAudit` variant** — in the `EconomySet` enum, add `TickAudit` AFTER `UpdateConsumption`:

```rust
    AdjustReservationPrices,
    UpdateConsumption,
    TickAudit,
```

- [ ] **Step 4: Add it to the `configure_sets` chain** — append `EconomySet::TickAudit` after `EconomySet::UpdateConsumption` in the `.chain()` tuple:

```rust
            EconomySet::AdjustReservationPrices,
            EconomySet::UpdateConsumption,
            EconomySet::TickAudit,
```

- [ ] **Step 5: Add the system wrapper** in `systems.rs` (near the other wrappers; ensure `crate::economy::audit::{run_tick_audit_at_tick, LastTickMoney}` are reachable — they are re-exported via `pub use audit::*`, so `LastTickMoney`/`run_tick_audit_at_tick` resolve at `crate::economy::*`; the wrapper can call `crate::economy::run_tick_audit_at_tick`):

```rust
/// End-of-tick SFC conservation audit. Runs LAST (after UpdateConsumption, so all of this tick's
/// money moves are settled). Fail-fast: a drift is an unrecoverable invariant break, so a returned
/// Err panics — exactly like the codebase's other "this is impossible" .expect points. Emits a
/// TickAudit heartbeat every tick (the queryable conservation trace).
pub fn run_tick_audit_system(
    tick: Res<Tick>,
    accounts: Res<AccountBook>,
    mut ledger: ResMut<TradeLedger>,
    mut last: ResMut<crate::economy::audit::LastTickMoney>,
) {
    crate::economy::audit::run_tick_audit_at_tick(&accounts, &mut ledger, &mut last, tick.0)
        .expect("CONSERVATION VIOLATION: total_money changed between ticks (money minted/destroyed) — the SFC byte-invariant is broken; halting the tick. This must never happen.");
}
```
(Confirm `Tick`, `AccountBook`, `TradeLedger` are imported at the top of `systems.rs` — they are used by neighboring systems.)

- [ ] **Step 6: Register the system** — in the **MAIN chained `add_systems` tuple** (the one ending `.before(crate::mobility::systems::tick_increment_system)`, NOT one of the four standalone calls), after `run_consumption_update_system.in_set(EconomySet::UpdateConsumption),`:

```rust
            run_tick_audit_system.in_set(EconomySet::TickAudit),
```

- [ ] **Step 7: Run the two tests — verify they PASS**

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core tick_audit_fires injected_money_drift
```
Expected: PASS (the first sees ≥3 TickAudit events; the second panics with "CONSERVATION VIOLATION", which `#[should_panic]` catches).

- [ ] **Step 8: Run the FULL economy suite** — the audit is now active in the default schedule; confirm no existing test trips it (no spurious drift):

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::
```
Expected: PASS — ALL economy tests green (esp. `conservation_full_plugin_multi_tick` + `steady_state_multi_tick`, which now also emit TickAudit events but whose money-invariant holds, so the audit never panics). **If a multi-tick test now panics with "CONSERVATION VIOLATION", STOP and report BLOCKED** — that means a real money-drift exists in that path (a genuine finding); do NOT silence the audit.

- [ ] **Step 9: Commit**

```bash
git add backend/crates/sim-core/src/economy/systems.rs backend/crates/sim-core/src/economy/tests/audit.rs
git commit -m "feat(economy): wire EconomySet::TickAudit (fail-fast on money drift, end-of-tick)"
```

---

## Sub-Slice C — Sentinel upgrade + non-destabilization + gate

### Task C1: Release-grade `HOUSEHOLD_SECTOR` net-zero sentinels

**Files:** Modify `backend/crates/sim-core/src/economy/wages.rs` (two sentinels) + `systems.rs` (profit wrapper fail-fast); Test `backend/crates/sim-core/src/economy/tests/wages.rs`.

- [ ] **Step 1: Write the failing test** (append to `tests/wages.rs`) — proves a non-zero `HOUSEHOLD_SECTOR` sentinel is now a release-grade `Err` (not a debug-only assert). Inject stranded cash into the sentinel before calling the pure core:

```rust
#[test]
fn nonzero_household_sentinel_is_release_grade_err() {
    use crate::economy::wages::{run_pay_wages_at_tick, HouseholdSector, SellerReceipts, WageTelemetry, HOUSEHOLD_SECTOR};
    use crate::economy::{AccountBook, DemandPools, EconomyError, Money, TradeLedger};
    use crate::economy::systems::EconomyConfig;
    use std::collections::BTreeMap;
    // No receipts → no wage transfers → the second leg never runs; but we pre-strand cash in the
    // sentinel so the net-zero check at the end is violated.
    let mut accounts = AccountBook::default();
    accounts.deposit(HOUSEHOLD_SECTOR, Money(123)).unwrap(); // stranded sentinel cash
    let receipts = SellerReceipts::default();
    let mut demand = DemandPools::default();
    let household = HouseholdSector { population: 1, pool_weights: BTreeMap::new() };
    let mut wt = WageTelemetry::default();
    let mut ledger = TradeLedger::default();
    let cfg = EconomyConfig::default();
    let r = run_pay_wages_at_tick(&mut accounts, &receipts, &mut demand, &household, &mut wt, &mut ledger, &cfg);
    assert_eq!(r, Err(EconomyError::ConservationViolation), "non-zero sentinel → release-grade Err, not a debug_assert");
}
```
(If the exact `run_pay_wages_at_tick` argument list differs, read wages.rs:77 and match it — the verified signature is `(accounts, receipts, demand, household, wage_telemetry, ledger, config)`.)

- [ ] **Step 2: Run it — verify it FAILS** (today the sentinel is `debug_assert`; in a release-mode test build it would not fire, and in debug it would PANIC not return Err — either way the `assert_eq!(r, Err(...))` fails):

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core nonzero_household_sentinel_is_release_grade
```
Expected: FAIL (panics on the `debug_assert_eq!` in the debug test build, OR returns `Ok` — not `Err`).

- [ ] **Step 3: Upgrade sentinel #1** (wages.rs:147-151) — replace the `debug_assert_eq!` block in `run_pay_wages_at_tick` with a release-grade check:

```rust
    if accounts.account(HOUSEHOLD_SECTOR).available != Money::ZERO {
        // HOUSEHOLD_SECTOR must net to zero after PayWages — stranded sentinel cash is a
        // conservation violation. Release-grade (the wrapper .expect surfaces it fail-fast).
        return Err(EconomyError::ConservationViolation);
    }
```

- [ ] **Step 4: Upgrade sentinel #2** (wages.rs:232-236) — replace the `debug_assert_eq!` block in `run_distribute_profit_at_tick` with the same:

```rust
    if accounts.account(HOUSEHOLD_SECTOR).available != Money::ZERO {
        // HOUSEHOLD_SECTOR must net to zero after profit distribution — stranded sentinel cash
        // is a conservation violation. Surfaced fail-fast by the wrapper (see C1 Step 5).
        return Err(EconomyError::ConservationViolation);
    }
```

- [ ] **Step 5: Make the profit wrapper fail-fast on `ConservationViolation`** (systems.rs `run_distribute_profit_system`, the `if let Err(reason) = … { ledger.0.push(MarketClearFailed …) }` block). The profit wrapper deliberately soft-degrades genuine faults (`InsufficientFunds` shortfall) to a `MarketClearFailed` event, but a `ConservationViolation` is unrecoverable and must fail-fast. Replace the body of the `if let Err(reason)` with:

```rust
    if let Err(reason) = run_distribute_profit_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut ledger,
        &config,
    ) {
        assert_ne!(
            reason,
            EconomyError::ConservationViolation,
            "CONSERVATION VIOLATION: HOUSEHOLD_SECTOR sentinel non-zero after profit distribution (sentinel-stranded cash) — the SFC invariant is broken; halting. This must never happen."
        );
        ledger.0.push(EconomyEvent::MarketClearFailed {
            market: MarketId(0),
            good: GoodId(0),
            reason,
        });
    }
```
(`EconomyError` is `Copy + PartialEq + Debug`; confirm it's imported in `systems.rs` — it is, used by other systems.)

- [ ] **Step 6: Run the sentinel test — verify it PASSES**

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core nonzero_household_sentinel_is_release_grade
```
Expected: PASS (returns `Err(ConservationViolation)`).

- [ ] **Step 7: Run all wage/profit + conservation tests** — confirm the normal (sentinel-zero) path is unaffected:

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests::wages economy::tests::conservation
```
Expected: PASS — every existing wage/profit/conservation test stays green (in all of them the sentinel nets to zero, so the new check passes silently).

- [ ] **Step 8: Commit**

```bash
git add backend/crates/sim-core/src/economy/wages.rs backend/crates/sim-core/src/economy/systems.rs backend/crates/sim-core/src/economy/tests/wages.rs
git commit -m "feat(economy): release-grade HOUSEHOLD_SECTOR net-zero sentinels (fail-fast)"
```

### Task C2: Persist round-trip (TickAudit in the tail) + full local gate

**Files:** Test `backend/crates/sim-core/src/economy/tests/persist.rs`; then verification only.

- [ ] **Step 1: Write the persist round-trip test** (append to `tests/persist.rs`) — a `TickAudit` event in the bounded `ledger_tail` round-trips losslessly (proves the new variant is serde-backward-compatible, no schema change):

```rust
#[test]
fn tick_audit_event_round_trips_in_ledger_tail() {
    use crate::economy::{EconomyEvent, EconomyPersistSnapshot, Money, TradeLedger};
    let mut world = install_economy();
    world.resource_mut::<TradeLedger>().0.push(EconomyEvent::TickAudit { tick: 5, total_money: Money(99_999) });
    let snap = extract_from_world(&world);
    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &decoded);
    assert!(
        fresh.resource::<TradeLedger>().0.iter().any(|e|
            matches!(e, EconomyEvent::TickAudit { tick: 5, total_money: Money(99_999) })),
        "TickAudit survives the ledger_tail round-trip"
    );
    assert_eq!(snap, extract_from_world(&fresh), "full snapshot identity with a TickAudit event");
}
```

- [ ] **Step 2: Run it — verify it PASSES** (persist is generic over `EconomyEvent`; this is a regression guard):

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core tick_audit_event_round_trips
```
Expected: PASS. (If it fails to deserialize, the new variant broke serde — STOP and investigate; that would contradict the no-DELETE claim.)

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-core/src/economy/tests/persist.rs
git commit -m "test(economy): TickAudit event round-trips in the persisted ledger tail"
```

- [ ] **Step 4: Full local gate** — run each; all must be clean:

```bash
cd /Users/ramonfuglister/Coding/abutown-vtraders
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace
```
Expected: fmt clean (run `cargo fmt --all` to fix if dirty, then re-check + commit); clippy exit 0; ALL workspace tests pass. (`--workspace` here at the END is the final gate, not iteration.)

- [ ] **Step 5: Frontend gate + e2e** (backend-only change, but mandatory before push):

```bash
cd /Users/ramonfuglister/Coding/abutown-vtraders
npm run typecheck && npx vitest run && node scripts/build.mjs
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target npm run test:e2e
```
Expected: typecheck clean, 220 vitest pass, build OK; e2e render-smoke 2/2 (the audit is economy-only — it doesn't touch mobility/agent counts; and a healthy economy never drifts, so the e2e server never panics).

- [ ] **Step 6: Commit any gate fix** (only if Steps 4-5 required a change):

```bash
git add -A && git commit -m "fix(economy): <describe the gate fix>"
```

---

## PR-body notes (for `finishing-a-development-branch`)

1. **No `DELETE FROM economy_snapshots`.** `LastTickMoney` is ephemeral (not persisted); the new `TickAudit` event variant is serde-backward-compatible (old `ledger_tail` data simply lacks it). No schema change.
2. **Conservation-observing only** — the audit moves no money and changes no economic behavior; it enforces (fail-fast) + observes the `total_money` byte-invariance that was previously only test-proven + `debug_assert` (release-elided).
3. **Fail-fast on drift** (lead decision): a conservation violation is unrecoverable → `.expect`-panic, halting loudly. Verified sound: `total_money` is constant after seed (no runtime minting), so the audit cannot panic spuriously. The profit-distribution wrapper fail-fasts *specifically* on `ConservationViolation` while keeping #75's soft path for the recoverable shortfall.
4. **The safety canary for the coming multi-stage slice** (firms-as-buyers makes the latent profit-leak live; this audit + the release-grade sentinels catch any resulting drift at runtime).
5. **Deferred:** per-good runtime ledger reconciliation; profit-leak recovery (multi-stage slice); `/economy/events` read-API; a cadence-gate on the TickAudit event if per-tick volume becomes a concern.

---

## Self-Review (run after writing — done)

**Spec coverage:** §3 mechanism → B1 (pure core) + B2 (wrapper/schedule, fail-fast). §3 sentinel upgrade + the profit-wrapper asymmetry → C1. §4 components → A1 (error/event/resource) + B/C. §5 conservation/determinism/no-fallback/persistence → A1/B/C2 + PR notes. §6 tests 1-6 → A1 (event/type), B1 (pure conserved/drift), B2 (full-plugin event + should_panic drift), C1 (sentinel Err + normal path green), C2 (persist round-trip + non-destabilization via the full economy suite). §7 sub-slices A/B/C → matched.

**Placeholder scan:** none — exact code + commands throughout.

**Type consistency:** `run_tick_audit_at_tick(&AccountBook, &mut TradeLedger, &mut LastTickMoney, u64) -> Result<(), EconomyError>` consistent A1↔B1↔B2; `EconomyError::ConservationViolation` + `EconomyEvent::TickAudit { tick: u64, total_money: Money }` + `event_type "tick_audit"` consistent across A1/B/C; `EconomySet::TickAudit` enum↔chain↔registration (B2); the wages pure-core signature `(accounts, receipts, demand, household, wage_telemetry, ledger, config)` matches the C1 test + the verified wages.rs:77 signature; the profit wrapper `assert_ne!` uses the real `MarketClearFailed { market, good, reason }` shape.
