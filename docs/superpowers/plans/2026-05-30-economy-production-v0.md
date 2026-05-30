# Economy Production v0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax. TDD, commit per task.

**Goal:** Add aggregate producers (consume inputs → produce outputs, with `Consumed`/`Produced` ledger events) to `sim_core::economy`, per `docs/superpowers/specs/2026-05-30-economy-production-v0-design.md`.

**Architecture:** New `economy/production.rs` (`Recipe`, `ProductionPool`, `ProductionPools`, `run_production_at_tick`); new `InventoryBook::consume`; two new `EconomyEvent` variants; a normal `Res`/`ResMut` `run_production_system` in a new `EconomySet::Production` slotted before `GeneratePoolOrders`.

**Tech Stack:** Rust, bevy_ecs 0.18, BTreeMap. No new deps.

**Branch/isolation:** worktree `/Users/ramonfuglister/Coding/abutown-production` on `plan/economy-production-v0` (from `origin/main` 25ddae0). `export CARGO_TARGET_DIR=/tmp/abutown-production-target`. Every cargo via `scripts/cargo-serial.sh`, one at a time, `pgrep -f cargo` first. `fmt --check` each task.

## Grounding (verified)
- `InventoryBook` (economy/inventory.rs): has `balance`, `deposit`, `lock_goods`, `release_goods`, `debit_locked_goods`, `total_good`; `InventoryBalance { available, locked }` (Quantity). NO consume-from-available yet.
- `EconomyError` (money.rs) has `Overflow`, `NegativeQuantity`, `InsufficientGoods`, `InsufficientFunds`.
- `EconomyEvent` (ledger.rs): no `Produced`/`Consumed` yet. `TradeLedger(pub Vec<EconomyEvent>)`.
- `interval_elapsed(last: Option<u64>, current_tick: u64, interval_ticks: u64) -> bool` is private `fn` in pools.rs:41 → promote to `pub(crate)`.
- `EconomyPlugin::install` (mod.rs:43-44) inserts `DemandPools::default()` / `SupplyPools::default()`, then `install_systems(schedule)` (line 47).
- systems.rs: `EconomySet { ExpireOrders, GeneratePoolOrders, ClearMarkets, Telemetry }`; `install_systems` does `configure_sets((...).chain())` + `add_systems((... .in_set(...)).before(crate::mobility::systems::tick_increment_system))`. `Tick` = `crate::mobility::resources::Tick`.
- Goods: `GOOD_WOOD=GoodId(2)`, `GOOD_IRON=GoodId(3)`, `GOOD_TOOLS=GoodId(4)`. Quantities are fixed-point ×1000.

---

### Task 1: `InventoryBook::consume`

**Files:** Modify `backend/crates/sim-core/src/economy/inventory.rs`; Modify `economy/tests/locking.rs`; (tests/mod.rs already declares `locking`).

- [ ] **Step 1: failing tests** (append to `tests/locking.rs`):
```rust
#[test]
fn consume_debits_available() {
    let actor = EconomicActorId(3);
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_WOOD, Quantity(5_000)).unwrap();
    inv.consume(actor, GOOD_WOOD, Quantity(2_000)).unwrap();
    assert_eq!(inv.balance(actor, GOOD_WOOD).available, Quantity(3_000));
    assert_eq!(inv.balance(actor, GOOD_WOOD).locked, Quantity(0));
}
#[test]
fn cannot_consume_more_than_available() {
    let actor = EconomicActorId(3);
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_WOOD, Quantity(1_000)).unwrap();
    assert_eq!(inv.consume(actor, GOOD_WOOD, Quantity(2_000)), Err(EconomyError::InsufficientGoods));
}
#[test]
fn cannot_consume_negative() {
    let mut inv = InventoryBook::default();
    assert_eq!(inv.consume(EconomicActorId(3), GOOD_WOOD, Quantity(-1)), Err(EconomyError::NegativeQuantity));
}
```
(Ensure `GOOD_WOOD` is imported in locking.rs's `use crate::economy::{...}`.) RUN: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core consume` → FAIL (no `consume`).

- [ ] **Step 2: implement** `consume` in `impl InventoryBook` (inventory.rs):
```rust
    pub fn consume(
        &mut self,
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    ) -> Result<(), EconomyError> {
        if qty.0 < 0 {
            return Err(EconomyError::NegativeQuantity);
        }
        let mut balance = self.balance(actor, good);
        if balance.available < qty {
            return Err(EconomyError::InsufficientGoods);
        }
        balance.available = balance.available.checked_sub(qty)?;
        self.balances.insert((actor, good), balance);
        Ok(())
    }
```
- [ ] **Step 3: RUN** → PASS. clippy `-p sim-core --all-targets -D warnings`, fmt --check clean.
- [ ] **Step 4: commit** `feat(economy): InventoryBook::consume debits available` (+ Co-Authored-By trailer).

---

### Task 2: `Produced`/`Consumed` ledger events

**Files:** Modify `economy/ledger.rs`.

- [ ] **Step 1:** add to `EconomyEvent` (after `GoodsReleased` / before `OrderRejected`, matching field style):
```rust
    Produced {
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    },
    Consumed {
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    },
```
(`EconomyEvent` derives `Debug, Clone, PartialEq, Eq` — these variants fit.) No standalone test (exercised by Task 3). RUN `cargo build -p sim-core` to confirm it compiles. clippy/fmt clean.
- [ ] **Step 2: commit** `feat(economy): add Produced/Consumed ledger events`.

---

### Task 3: `production.rs` — pools + `run_production_at_tick`

**Files:** Create `economy/production.rs`; Modify `economy/mod.rs` (`pub mod production; pub use production::*;`); Create `economy/tests/production.rs`; Modify `economy/tests/mod.rs` (`mod production;`); promote `interval_elapsed` to `pub(crate)` in `pools.rs`.

- [ ] **Step 1: failing tests** — `tests/production.rs`:
```rust
use crate::economy::{
    run_production_at_tick, EconomicActorId, EconomyEvent, GOOD_IRON, GOOD_TOOLS, GOOD_WOOD,
    InventoryBook, ProductionPool, ProductionPools, Quantity, Recipe, TradeLedger,
};

fn tools_recipe() -> Recipe {
    Recipe {
        inputs: vec![(GOOD_WOOD, Quantity(2_000)), (GOOD_IRON, Quantity(1_000))],
        outputs: vec![(GOOD_TOOLS, Quantity(1_000))],
    }
}
fn seed(actor: EconomicActorId, interval: u64) -> ProductionPools {
    let mut p = ProductionPools::default();
    p.0.insert(actor, ProductionPool { actor, recipe: tools_recipe(), interval_ticks: interval, last_generated_tick: None });
    p
}

#[test]
fn production_consumes_inputs_and_produces_outputs() {
    let actor = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_WOOD, Quantity(2_000)).unwrap();
    inv.deposit(actor, GOOD_IRON, Quantity(1_000)).unwrap();
    let mut ledger = TradeLedger::default();
    let mut prod = seed(actor, 1);
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 5).unwrap();
    assert_eq!(inv.balance(actor, GOOD_WOOD).available, Quantity(0));
    assert_eq!(inv.balance(actor, GOOD_IRON).available, Quantity(0));
    assert_eq!(inv.balance(actor, GOOD_TOOLS).available, Quantity(1_000));
    assert!(ledger.0.contains(&EconomyEvent::Consumed { actor, good: GOOD_WOOD, qty: Quantity(2_000) }));
    assert!(ledger.0.contains(&EconomyEvent::Consumed { actor, good: GOOD_IRON, qty: Quantity(1_000) }));
    assert!(ledger.0.contains(&EconomyEvent::Produced { actor, good: GOOD_TOOLS, qty: Quantity(1_000) }));
}

#[test]
fn production_skips_when_inputs_insufficient() {
    let actor = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_WOOD, Quantity(2_000)).unwrap(); // no IRON
    let mut ledger = TradeLedger::default();
    let mut prod = seed(actor, 1);
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 5).unwrap();
    assert_eq!(inv.balance(actor, GOOD_WOOD).available, Quantity(2_000)); // unchanged
    assert_eq!(inv.balance(actor, GOOD_TOOLS).available, Quantity(0));
    assert!(ledger.0.is_empty());
    assert_eq!(prod.0[&actor].last_generated_tick, Some(5)); // cadence still advances
}

#[test]
fn production_respects_interval() {
    let actor = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_WOOD, Quantity(4_000)).unwrap();
    inv.deposit(actor, GOOD_IRON, Quantity(2_000)).unwrap();
    let mut ledger = TradeLedger::default();
    let mut prod = seed(actor, 10);
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 0).unwrap(); // produces (last=None)
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 3).unwrap(); // interval not elapsed → skip
    assert_eq!(inv.balance(actor, GOOD_TOOLS).available, Quantity(1_000)); // only one batch
}

#[test]
fn production_conserves_money() {
    use crate::economy::{AccountBook, Money};
    let actor = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_WOOD, Quantity(2_000)).unwrap();
    inv.deposit(actor, GOOD_IRON, Quantity(1_000)).unwrap();
    let mut accounts = AccountBook::default();
    accounts.deposit(actor, Money(5_000)).unwrap();
    let before = accounts.total_money().unwrap();
    let mut ledger = TradeLedger::default();
    let mut prod = seed(actor, 1);
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 1).unwrap();
    assert_eq!(accounts.total_money().unwrap(), before); // production never touches money
}

#[test]
fn production_is_deterministic() {
    let run = || {
        let a1 = EconomicActorId(2);
        let a2 = EconomicActorId(1);
        let mut inv = InventoryBook::default();
        for a in [a1, a2] {
            inv.deposit(a, GOOD_WOOD, Quantity(2_000)).unwrap();
            inv.deposit(a, GOOD_IRON, Quantity(1_000)).unwrap();
        }
        let mut ledger = TradeLedger::default();
        let mut prod = ProductionPools::default();
        for a in [a1, a2] {
            prod.0.insert(a, ProductionPool { actor: a, recipe: tools_recipe(), interval_ticks: 1, last_generated_tick: None });
        }
        run_production_at_tick(&mut inv, &mut ledger, &mut prod, 1).unwrap();
        ledger.0
    };
    assert_eq!(run(), run());
}
```
RUN: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core production` → FAIL (missing types/fn).

- [ ] **Step 2: implement** `production.rs`:
```rust
use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::economy::{
    pools::interval_elapsed, EconomicActorId, EconomyError, EconomyEvent, GoodId, InventoryBook,
    Quantity, TradeLedger,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    pub inputs: Vec<(GoodId, Quantity)>,
    pub outputs: Vec<(GoodId, Quantity)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductionPool {
    pub actor: EconomicActorId,
    pub recipe: Recipe,
    pub interval_ticks: u64,
    pub last_generated_tick: Option<u64>,
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ProductionPools(pub BTreeMap<EconomicActorId, ProductionPool>);

pub fn run_production_at_tick(
    inventory: &mut InventoryBook,
    ledger: &mut TradeLedger,
    production: &mut ProductionPools,
    current_tick: u64,
) -> Result<(), EconomyError> {
    let actors: Vec<EconomicActorId> = production.0.keys().copied().collect();
    for actor in actors {
        let pool = production.0[&actor].clone();
        if !interval_elapsed(pool.last_generated_tick, current_tick, pool.interval_ticks) {
            continue;
        }
        // All inputs must be covered before consuming any (atomic per pool).
        let can_produce = pool
            .recipe
            .inputs
            .iter()
            .all(|(good, qty)| inventory.balance(actor, *good).available >= *qty);
        if can_produce {
            for (good, qty) in &pool.recipe.inputs {
                inventory.consume(actor, *good, *qty)?;
                ledger.0.push(EconomyEvent::Consumed { actor, good: *good, qty: *qty });
            }
            for (good, qty) in &pool.recipe.outputs {
                inventory.deposit(actor, *good, *qty)?;
                ledger.0.push(EconomyEvent::Produced { actor, good: *good, qty: *qty });
            }
        }
        if let Some(p) = production.0.get_mut(&actor) {
            p.last_generated_tick = Some(current_tick);
        }
    }
    Ok(())
}
```
In `pools.rs`, change `fn interval_elapsed` → `pub(crate) fn interval_elapsed`. In `mod.rs` add `pub mod production;` + `pub use production::*;` (alongside the other module re-exports).

- [ ] **Step 3: RUN** `cargo test -p sim-core production` → PASS (5 tests). clippy/fmt clean. (Note: `production_ledger_accounts_for_goods_delta` is covered by `production_consumes_inputs_and_produces_outputs` asserting both the inventory result AND the ledger events — the delta equals produced−consumed by construction.)
- [ ] **Step 4: commit** `feat(economy): aggregate production pools (consume inputs -> produce outputs)`.

---

### Task 4: schedule wiring + end-to-end test

**Files:** Modify `economy/systems.rs` (`EconomySet::Production`, `run_production_system`, install in chain), `economy/mod.rs` (insert `ProductionPools::default()`), Modify `economy/tests/systems.rs` (e2e test). 

- [ ] **Step 1: failing e2e test** (append to `tests/systems.rs`):
```rust
#[test]
fn production_runs_through_schedule() {
    use crate::economy::{ProductionPool, ProductionPools, Recipe};
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let actor = EconomicActorId(7);
    world.resource_mut::<InventoryBook>().deposit(actor, GOOD_WOOD, Quantity(2_000)).unwrap();
    world.resource_mut::<InventoryBook>().deposit(actor, GOOD_IRON, Quantity(1_000)).unwrap();
    world.resource_mut::<ProductionPools>().0.insert(actor, ProductionPool {
        actor,
        recipe: Recipe { inputs: vec![(GOOD_WOOD, Quantity(2_000)), (GOOD_IRON, Quantity(1_000))], outputs: vec![(GOOD_TOOLS, Quantity(1_000))] },
        interval_ticks: 1,
        last_generated_tick: None,
    });

    schedule.run(&mut world);

    assert_eq!(world.resource::<InventoryBook>().balance(actor, GOOD_TOOLS).available, Quantity(1_000));
    assert_eq!(world.resource::<InventoryBook>().balance(actor, GOOD_WOOD).available, Quantity(0));
}
```
(Add `GOOD_WOOD, GOOD_IRON, GOOD_TOOLS, InventoryBook` to the test file's imports as needed.) RUN → FAIL (no `ProductionPools` resource / no system).

- [ ] **Step 2: implement.** In `systems.rs`:
  - add `Production` to `EconomySet` (between `ExpireOrders` and `GeneratePoolOrders`).
  - add `run_production_system`:
```rust
pub fn run_production_system(
    tick: Res<Tick>,
    mut inventory: ResMut<InventoryBook>,
    mut ledger: ResMut<TradeLedger>,
    mut production: ResMut<ProductionPools>,
) {
    let _ = run_production_at_tick(&mut inventory, &mut ledger, &mut production, tick.0);
}
```
(import `run_production_at_tick`, `ProductionPools` from `crate::economy`.)
  - in `install_systems`: add `EconomySet::Production` to the `configure_sets((...).chain())` (place it right after `ExpireOrders`), and add `run_production_system.in_set(EconomySet::Production),` to the `add_systems((...))` tuple. New chain: `ExpireOrders -> Production -> GeneratePoolOrders -> ClearMarkets -> Telemetry`.

  In `mod.rs` `EconomyPlugin::install`, after the `SupplyPools::default()` insert: `world.insert_resource(ProductionPools::default());`.

- [ ] **Step 3: RUN** `cargo test -p sim-core production_runs_through_schedule` → PASS. Then full `cargo test -p sim-core economy` green. clippy/fmt clean.
- [ ] **Step 4: commit** `feat(economy): run production in the economy schedule`.

---

### Task 5: Final gate
- [ ] `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check`
- [ ] `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings`
- [ ] `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace`
- [ ] `scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server`
- [ ] (orchestrator) PR → CI green via `gh run watch --exit-status` → merge → cleanup.

## Deferred (later)
Production pricing/recipes-from-market, multi-step graphs, labor coupling, trader agents, transport costs, LOD, persistence.
