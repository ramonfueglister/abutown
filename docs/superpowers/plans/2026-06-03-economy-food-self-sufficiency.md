# FOOD Self-Sufficiency Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make GOOD_FOOD self-sustaining by adding two co-located `RAW→FOOD` extractors (one per FOOD supply market), mirroring the proven #75 TOOLS extractor, so the closed economy no longer winds down to a one-good system.

**Architecture:** Pure data + a mechanical rename. The generic `run_regen_at_tick` / `run_production_at_tick` / order systems already iterate ALL deposits/pools, so two new seed entries need NO new system, NO schedule change, NO new money code. FOOD's money circuit closes through the existing #75 seller→wage→100%-profit→household machinery. Reuses `GOOD_RAW` (per-actor faucets ⇒ no contention) and the already-persisted `raw_deposits`/`ProductionPools`/`SupplyPools` maps ⇒ no new schema field, no new mandatory DELETE beyond #75's.

**Tech Stack:** Rust (bevy_ecs 0.18), `sim-core` crate; fixed-point i64/i128 determinism; SFC double-entry money (`AccountBook::transfer`); TDD via `cargo test`.

**Spec:** `docs/superpowers/specs/2026-06-03-economy-food-self-sufficiency-design.md`

---

## Verified Facts (pinned against the real code — do not re-derive)

**Goods (`economy/goods.rs`):** `GOOD_FOOD = GoodId(1)`, `GOOD_TOOLS = GoodId(4)`, `GOOD_RAW = GoodId(5)` (non-tradable).

**Extractor (`economy/production.rs:72`):** `pub const EXTRACTOR: EconomicActorId = EconomicActorId(8_031);` — this is the TOOLS extractor; **renamed to `EXTRACTOR_TOOLS` in Task A1**. The new FOOD ids `8_032`/`8_033` are confirmed FREE.

**Struct shapes (`economy/production.rs`):**
```rust
pub struct Recipe { pub inputs: Vec<(GoodId, Quantity)>, pub outputs: Vec<(GoodId, Quantity)> }
pub struct ProductionPool { pub actor: EconomicActorId, pub recipe: Recipe, pub interval_ticks: u64, pub last_generated_tick: Option<u64> }
pub struct ProductionPools(pub BTreeMap<EconomicActorId, ProductionPool>);   // Resource
pub struct RawDeposit { pub good: GoodId, pub qty_per_interval: Quantity, pub interval_ticks: u64, pub last_regen_tick: Option<u64> }  // derive Copy
pub struct RawDeposits(pub BTreeMap<EconomicActorId, RawDeposit>);            // Resource
pub fn run_regen_at_tick(inventory: &mut InventoryBook, ledger: &mut TradeLedger, deposits: &mut RawDeposits, current_tick: u64) -> Result<(), EconomyError>
pub fn run_production_at_tick(inventory: &mut InventoryBook, ledger: &mut TradeLedger, production: &mut ProductionPools, current_tick: u64) -> Result<(), EconomyError>
```
`run_production_at_tick` is input-gated and atomic-per-pool: it consumes inputs only if ALL inputs are covered, emits `EconomyEvent::Consumed`/`Produced`, advances `last_generated_tick` every interval regardless. `run_regen_at_tick` deposits `qty_per_interval` of `good` unconditionally on the interval, emits `Regenerated`, stamps the cursor only when it fires.

**SupplyPool (`economy/pools.rs`):** fields `{ actor, market: MarketId, good: GoodId, offered_qty_per_tick: Quantity, min_price: Money, interval_ticks: u64, last_generated_tick: Option<u64> }`. `SupplyPools(pub BTreeMap<EconomicActorId, SupplyPool>)`, `DemandPools(pub BTreeMap<EconomicActorId, DemandPool>)` — **one pool per actor** (map key IS the actor).

**MarketDistances (`economy/market.rs:85`):** `pub struct MarketDistances(pub BTreeMap<(MarketId, MarketId), i64>);`

**Seed (`economy/seed.rs`):** `pub fn seed_demo_economy(world: &mut World)`, fresh-world-only (`if !world.resource::<Markets>().0.is_empty() { return; }`). Markets: `m_a=MarketId(9_001)`, `m_b=MarketId(9_002)`, `m_fa=MarketId(9_003)`, `m_fb=MarketId(9_004)`. Existing FOOD pairs: finite `food_supplier 8_011` sells FOOD @ `m_a` (10/tick, 1M endowment) → `food_consumer 8_012` demands @ `m_b`; finite `flow_supplier 8_021` sells FOOD @ `m_fa` (10/tick, 1M) → `flow_consumer 8_022` demands @ `m_fb`. The TOOLS extractor block is at `seed.rs:123-169`. The stale comment at **`seed.rs:130-131`** says FOOD is "intentionally left on the draining 1M endowment" — must be updated. `m_fa`/`m_fb` are defined at ~`seed.rs:256`; the FOOD-extractor block (Task B2) goes AFTER the `flow_consumer` DemandPool insert (~`seed.rs:328`) and BEFORE the `HouseholdSector` block (~`seed.rs:329`). Opening-price block (`seed.rs:360-381`) seeds only consumer markets (m_b TOOLS, m_b FOOD, m_fb FOOD) — **no change needed** (the FOOD extractors sell at supply markets m_a/m_fa which carry no consumer pool, and reuse the existing `(m_a,GOOD_FOOD)`/`(m_fa,GOOD_FOOD)` activity).

**EXTRACTOR rename blast radius:** 70 word-boundary occurrences across 10 files: `economy/{ledger,persist,goods,production,seed}.rs` and `economy/tests/{persist,systems,production,conservation,seed}.rs`.

**Test world builders (copy verbatim):**
- `tests/seed.rs::seed_world()` builds a 4-node spatial world with all economy resources, returns `World`.
- `tests/production.rs::regen_rate_covers_aggregate_tools_demand_at_seed` (lines 276-304) builds the same world inline and calls `seed_demo_economy`.
- `tests/conservation.rs::conservation_full_plugin_multi_tick` and `steady_state_multi_tick` install `CorePlugin` + `MobilityPlugin` + `EconomyPlugin`, seed ONE market `MarketId(1)` with the EXTRACTOR (supplier) AND the consumer **at the same market** (single-market auction — NOT macro_flow), then loop `schedule.run(&mut world); world.resource_mut::<Tick>().0 += 1;`.

**Persist (`economy/persist.rs` + `tests/persist.rs`):** `EconomyPersistSnapshot.raw_deposits: Vec<(EconomicActorId, RawDeposit)>` already exists (#75), extracted in sorted order; `extract_from_world` / `apply_into_world` are general over the map; `economy_snapshot_round_trips` proves identity. No `serde(default)` (the `snapshot_without_raw_deposits_field_fails_to_deserialize` test enforces this).

**Cargo (MANDATORY — never run bare cargo; route through the serial lock with the isolated target so it never collides with the user's parallel dev server):**
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core <TESTNAME>
```
Run from the worktree root `/Users/ramonfuglister/Coding/abutown-vtraders`. Never `--workspace --all-targets` during iteration. `mkdir -p /tmp/abutown-vtraders-tmp` once if missing.

---

## Sub-Slice A — Constants + Rename

### Task A1: Rename `EXTRACTOR` → `EXTRACTOR_TOOLS`

**Files:** Modify (word-boundary replace `EXTRACTOR` → `EXTRACTOR_TOOLS`):
`backend/crates/sim-core/src/economy/{ledger,persist,goods,production,seed}.rs` and
`backend/crates/sim-core/src/economy/tests/{persist,systems,production,conservation,seed}.rs`.

- [ ] **Step 1: Mechanical rename across all 10 files** (macOS sed; word-boundary so it can't double-apply and won't touch future `EXTRACTOR_FOOD_*`):

```bash
cd /Users/ramonfuglister/Coding/abutown-vtraders
sed -i '' -E 's/\bEXTRACTOR\b/EXTRACTOR_TOOLS/g' \
  backend/crates/sim-core/src/economy/ledger.rs \
  backend/crates/sim-core/src/economy/persist.rs \
  backend/crates/sim-core/src/economy/goods.rs \
  backend/crates/sim-core/src/economy/production.rs \
  backend/crates/sim-core/src/economy/seed.rs \
  backend/crates/sim-core/src/economy/tests/persist.rs \
  backend/crates/sim-core/src/economy/tests/systems.rs \
  backend/crates/sim-core/src/economy/tests/production.rs \
  backend/crates/sim-core/src/economy/tests/conservation.rs \
  backend/crates/sim-core/src/economy/tests/seed.rs
```

- [ ] **Step 2: Verify no bare `EXTRACTOR` remains**

```bash
grep -rwn "EXTRACTOR" backend/crates/sim-core/src/economy && echo "FAIL: bare EXTRACTOR remains" || echo "OK: no bare EXTRACTOR"
grep -rn "EXTRACTOR_TOOLS" backend/crates/sim-core/src/economy | wc -l   # expect 70
```
Expected: `OK: no bare EXTRACTOR`; the count line prints `70`.

- [ ] **Step 3: Compile-check the crate (rename must not break anything)**

Run:
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::
```
Expected: PASS (every existing economy test still green; this is a pure rename).

- [ ] **Step 4: Commit**

```bash
git add backend/crates/sim-core/src/economy
git commit -m "refactor(economy): rename EXTRACTOR -> EXTRACTOR_TOOLS (FOOD extractors incoming)"
```

### Task A2: Add the two FOOD extractor id constants

**Files:** Modify `backend/crates/sim-core/src/economy/production.rs:72` (just below the renamed const); Test `backend/crates/sim-core/src/economy/tests/production.rs`.

- [ ] **Step 1: Write the failing test** (append to `tests/production.rs`):

```rust
#[test]
fn food_extractor_ids_are_free_and_distinct() {
    use crate::economy::production::{EXTRACTOR_FOOD_A, EXTRACTOR_FOOD_FA, EXTRACTOR_TOOLS};
    use crate::economy::EconomicActorId;
    // Distinct from the TOOLS extractor and from each other.
    assert_ne!(EXTRACTOR_FOOD_A, EXTRACTOR_TOOLS);
    assert_ne!(EXTRACTOR_FOOD_FA, EXTRACTOR_TOOLS);
    assert_ne!(EXTRACTOR_FOOD_A, EXTRACTOR_FOOD_FA);
    // Distinct from every seeded actor id (8_001..8_022, 8_031).
    for seeded in [8_001u64, 8_002, 8_011, 8_012, 8_021, 8_022, 8_031] {
        assert_ne!(EXTRACTOR_FOOD_A, EconomicActorId(seeded));
        assert_ne!(EXTRACTOR_FOOD_FA, EconomicActorId(seeded));
    }
}
```

- [ ] **Step 2: Run it — verify it FAILS** (constants don't exist):

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core food_extractor_ids_are_free_and_distinct
```
Expected: FAIL — `cannot find value EXTRACTOR_FOOD_A in module`.

- [ ] **Step 3: Add the constants** in `production.rs`, immediately after the `EXTRACTOR_TOOLS` const:

```rust
/// FOOD self-sufficiency: one continuous RAW->FOOD extractor co-located at each FOOD
/// supply market. `_A` sits at m_a (backs finite supplier 8_011), `_FA` at m_fa (backs
/// finite flow supplier 8_021). Adjacent to EXTRACTOR_TOOLS (8_031), clear of 8_001..8_022.
pub const EXTRACTOR_FOOD_A: EconomicActorId = EconomicActorId(8_032);
pub const EXTRACTOR_FOOD_FA: EconomicActorId = EconomicActorId(8_033);
```

- [ ] **Step 4: Run it — verify it PASSES**

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core food_extractor_ids_are_free_and_distinct
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/production.rs backend/crates/sim-core/src/economy/tests/production.rs
git commit -m "feat(economy): add EXTRACTOR_FOOD_A/FA actor ids"
```

---

## Sub-Slice B — Seed + Mechanism Tests

### Task B1: Multi-extractor regen + production + per-actor RAW balance

Proves the generic mechanism handles MULTIPLE extractors making DIFFERENT goods from the SAME `GOOD_RAW`, each balancing its own RAW (the non-redundant FOOD-specific behavior; the single-extractor RAW→TOOLS path is already covered).

**Files:** Test `backend/crates/sim-core/src/economy/tests/production.rs`.

- [ ] **Step 1: Write the failing test** (append to `tests/production.rs`):

```rust
#[test]
fn two_extractors_make_distinct_goods_and_each_balances_its_own_raw() {
    use crate::economy::production::{
        EXTRACTOR_FOOD_A, EXTRACTOR_TOOLS, ProductionPool, ProductionPools, RawDeposit,
        RawDeposits, Recipe, run_production_at_tick, run_regen_at_tick,
    };
    use crate::economy::{
        EconomyEvent, GOOD_FOOD, GOOD_RAW, GOOD_TOOLS, InventoryBook, Quantity, TradeLedger,
    };
    use std::collections::BTreeMap;

    let mut inv = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    let mut deposits = RawDeposits(BTreeMap::new());
    let mut prod = ProductionPools::default();
    for (actor, out) in [(EXTRACTOR_TOOLS, GOOD_TOOLS), (EXTRACTOR_FOOD_A, GOOD_FOOD)] {
        deposits.0.insert(
            actor,
            RawDeposit { good: GOOD_RAW, qty_per_interval: Quantity(10), interval_ticks: 1, last_regen_tick: None },
        );
        prod.0.insert(
            actor,
            ProductionPool {
                actor,
                recipe: Recipe { inputs: vec![(GOOD_RAW, Quantity(10))], outputs: vec![(out, Quantity(10))] },
                interval_ticks: 1,
                last_generated_tick: None,
            },
        );
    }

    // One tick: regen (deposits RAW) then production (consumes RAW, emits goods).
    run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, 0).unwrap();
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 0).unwrap();

    // Each extractor produced its OWN good...
    assert_eq!(inv.balance(EXTRACTOR_TOOLS, GOOD_TOOLS).available, Quantity(10));
    assert_eq!(inv.balance(EXTRACTOR_FOOD_A, GOOD_FOOD).available, Quantity(10));
    // ...and the FOOD extractor made NO tools, the TOOLS extractor made NO food.
    assert_eq!(inv.balance(EXTRACTOR_FOOD_A, GOOD_TOOLS).available, Quantity(0));
    assert_eq!(inv.balance(EXTRACTOR_TOOLS, GOOD_FOOD).available, Quantity(0));

    // Per-actor RAW balance: each regenerated 10 and consumed 10 -> net 0 on hand.
    for actor in [EXTRACTOR_TOOLS, EXTRACTOR_FOOD_A] {
        let regen: i64 = ledger.0.iter().filter_map(|e| match e {
            EconomyEvent::Regenerated { actor: a, good: GOOD_RAW, qty } if *a == actor => Some(qty.0),
            _ => None,
        }).sum();
        let consumed: i64 = ledger.0.iter().filter_map(|e| match e {
            EconomyEvent::Consumed { actor: a, good: GOOD_RAW, qty } if *a == actor => Some(qty.0),
            _ => None,
        }).sum();
        assert_eq!(regen, consumed, "actor {actor:?} RAW regenerated == consumed");
        assert_eq!(inv.balance(actor, GOOD_RAW).available, Quantity(0), "actor {actor:?} RAW on-hand 0");
    }

    // Throttle: with RAW exhausted (consumed this tick) and no regen on a within-interval
    // call, no further FOOD/TOOLS is produced.
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 0).unwrap(); // same tick, interval not elapsed
    assert_eq!(inv.balance(EXTRACTOR_FOOD_A, GOOD_FOOD).available, Quantity(10), "no double-produce");
}
```

Note: `GOOD_RAW` is `GoodId(5)`; the `Regenerated { good: GOOD_RAW, .. }` and `Consumed { good: GOOD_RAW, .. }` pattern matches use the constant directly (it is a `const`, usable in a match guard via `if`-binding as written).

- [ ] **Step 2: Run it — verify it FAILS** (constants/behavior path):

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core two_extractors_make_distinct_goods
```
Expected: PASS immediately (the mechanism is already general; this test is a guard that the multi-extractor shared-RAW case behaves correctly). If it does NOT pass, STOP — that is a real mechanism bug, not a missing impl.

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-core/src/economy/tests/production.rs
git commit -m "test(economy): multi-extractor distinct goods + per-actor RAW balance"
```

### Task B2: Seed the two FOOD extractors

**Files:** Modify `backend/crates/sim-core/src/economy/seed.rs` (imports + the FOOD-extractor block + the stale comment); Test `backend/crates/sim-core/src/economy/tests/seed.rs`.

- [ ] **Step 1: Write the failing test** — in `tests/seed.rs`, replace the **ENTIRE** existing function `seed_installs_extractor_with_raw_faucet_recipe_and_tools_supply_but_never_lists_raw` (delete the old `#[test] fn …` definition completely — do NOT leave it alongside; the new one supersedes it) with the renamed, three-extractor version below:

```rust
#[test]
fn seed_installs_three_extractors_tools_and_two_food_but_never_lists_raw() {
    use crate::economy::production::{
        EXTRACTOR_FOOD_A, EXTRACTOR_FOOD_FA, EXTRACTOR_TOOLS, ProductionPools, RawDeposits,
    };
    use crate::economy::{GOOD_FOOD, GOOD_RAW, GOOD_TOOLS, HouseholdSector, InventoryBook, MarketId};

    let mut world = seed_world();
    seed_demo_economy(&mut world);

    // (market, output-good) expected for each extractor.
    let expected = [
        (EXTRACTOR_TOOLS, MarketId(9_001), GOOD_TOOLS),   // m_a
        (EXTRACTOR_FOOD_A, MarketId(9_001), GOOD_FOOD),   // m_a
        (EXTRACTOR_FOOD_FA, MarketId(9_003), GOOD_FOOD),  // m_fa
    ];
    for (actor, market, out_good) in expected {
        let dep = world.resource::<RawDeposits>().0[&actor];
        assert_eq!(dep.good, GOOD_RAW, "{actor:?} faucets RAW");
        assert_eq!(dep.qty_per_interval.0, 10, "{actor:?} faucet rate 10");
        assert_eq!(dep.interval_ticks, 1);

        let pool = world.resource::<ProductionPools>().0[&actor].clone();
        assert_eq!(pool.recipe.inputs, vec![(GOOD_RAW, dep.qty_per_interval)], "{actor:?} consumes RAW");
        assert_eq!(pool.recipe.outputs.len(), 1);
        assert_eq!(pool.recipe.outputs[0].0, out_good, "{actor:?} outputs the right good");

        let sp = world.resource::<SupplyPools>().0[&actor];
        assert_eq!(sp.good, out_good, "{actor:?} sells its output good");
        assert_eq!(sp.market, market, "{actor:?} sells at the right market");
        assert_eq!(sp.offered_qty_per_tick.0, 10);

        assert!(world.resource::<InventoryBook>().balance(actor, GOOD_RAW).available.0 > 0,
            "{actor:?} holds opening RAW so production fires on tick 0");
        assert!(!world.resource::<HouseholdSector>().pool_weights.contains_key(&actor),
            "{actor:?} is a firm, not a labor household");
    }

    // GOOD_RAW is NEVER on any SupplyPool or DemandPool (structural non-tradability).
    assert!(world.resource::<SupplyPools>().0.values().all(|p| p.good != GOOD_RAW), "RAW never on a SupplyPool");
    assert!(world.resource::<DemandPools>().0.values().all(|p| p.good != GOOD_RAW), "RAW never on a DemandPool");
}
```

- [ ] **Step 2: Run it — verify it FAILS** (FOOD extractors not seeded):

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core seed_installs_three_extractors
```
Expected: FAIL — panics indexing `RawDeposits.0[&EXTRACTOR_FOOD_A]` (key missing).

- [ ] **Step 3: Add the FOOD imports** to the `use crate::economy::production::{...}` line at the top of `seed.rs` — add `EXTRACTOR_FOOD_A, EXTRACTOR_FOOD_FA` (and keep `EXTRACTOR_TOOLS`):

```rust
use crate::economy::production::{
    EXTRACTOR_FOOD_A, EXTRACTOR_FOOD_FA, EXTRACTOR_TOOLS, ProductionPool, ProductionPools,
    RawDeposit, RawDeposits, Recipe,
};
```

- [ ] **Step 4: Update the stale comment** at `seed.rs:130-131`. Find the line ending:
```
    // it and matches the finite supplier's offered rate. FOOD is intentionally left on
    // the draining 1M endowment (no RAW->FOOD extractor this slice — recorded decision).
```
Replace those two comment lines with:
```
    // it and matches the finite supplier's offered rate. FOOD gets its OWN two co-located
    // extractors below (one per FOOD supply market m_a/m_fa) — see the FOOD-extractor block.
```

- [ ] **Step 5: Insert the FOOD-extractor block** immediately AFTER the `flow_consumer` `DemandPools` insert (the block ending at ~`seed.rs:328`, the `world.resource_mut::<DemandPools>().0.insert(flow_consumer, …);`) and BEFORE the `// ── SFC household sector` comment (~`seed.rs:329`):

```rust
    // ── Continuous FOOD source: two co-located extractors (FOOD self-sufficiency) ──────
    // Mirror of EXTRACTOR_TOOLS, one per FOOD supply market (m_a, m_fa), seeded ALONGSIDE
    // the finite food suppliers 8_011/8_021 (drain-then-handover). Reuses GOOD_RAW (per-actor
    // faucet — no contention; each extractor regens+consumes its own RAW the same tick). RAW
    // is NEVER placed on a pool/market. REGEN_QTY_FOOD=10 covers the 10/tick demand routed to
    // each supply market via MarketDistances (8_012@m_b sourced from m_a; 8_022@m_fb from m_fa).
    // Supply and demand are CROSS-market: m_a feeds m_b, m_fa feeds m_fb (no m_a<->m_fb distance).
    // These extractors are FIRMS (sellers): their FOOD-sale revenue flows through the existing
    // #75 money circuit (run_pay_wages_at_tick + run_distribute_profit_at_tick -> households),
    // so FOOD's money loop closes like TOOLS. They are NOT in pool_weights (firms, not households).
    const REGEN_QTY_FOOD: Quantity = Quantity(10);
    for (extractor, supply_market) in [(EXTRACTOR_FOOD_A, m_a), (EXTRACTOR_FOOD_FA, m_fa)] {
        world
            .resource_mut::<InventoryBook>()
            .deposit(extractor, GOOD_RAW, REGEN_QTY_FOOD)
            .expect("seed: food extractor opening raw stock");
        world.resource_mut::<RawDeposits>().0.insert(
            extractor,
            RawDeposit {
                good: GOOD_RAW,
                qty_per_interval: REGEN_QTY_FOOD,
                interval_ticks: 1,
                last_regen_tick: None,
            },
        );
        world.resource_mut::<ProductionPools>().0.insert(
            extractor,
            ProductionPool {
                actor: extractor,
                recipe: Recipe {
                    inputs: vec![(GOOD_RAW, REGEN_QTY_FOOD)],
                    outputs: vec![(GOOD_FOOD, REGEN_QTY_FOOD)],
                },
                interval_ticks: 1,
                last_generated_tick: None,
            },
        );
        world.resource_mut::<SupplyPools>().0.insert(
            extractor,
            SupplyPool {
                actor: extractor,
                market: supply_market,
                good: GOOD_FOOD,
                offered_qty_per_tick: REGEN_QTY_FOOD,
                min_price: Money(500),
                interval_ticks: 1,
                last_generated_tick: None,
            },
        );
    }
```

- [ ] **Step 6: Run the seed test — verify it PASSES**

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core seed_installs_three_extractors
```
Expected: PASS.

- [ ] **Step 7: Run all seed + economy tests (no regression in the other seed tests)**

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests::seed
```
Expected: PASS (the other four seed tests still green — extractors add no markets, FOOD `.any()` assertions still hold).

- [ ] **Step 8: Commit**

```bash
git add backend/crates/sim-core/src/economy/seed.rs backend/crates/sim-core/src/economy/tests/seed.rs
git commit -m "feat(economy): seed two co-located RAW->FOOD extractors (m_a, m_fa)"
```

### Task B3: Routing-aware faucet-sizing invariant (+ negative control)

**Files:** Test `backend/crates/sim-core/src/economy/tests/production.rs`.

The invariant (spec §6): for each consumer `DemandPool` (good g, demand-market d, qty q), the sum of continuous faucet rates for g at all supply markets that REACH d via `MarketDistances` (plus same-market supply) must be ≥ q. Non-vacuous because it keys on real demand pools against reachable supply, not on the (demand-free) supply market.

- [ ] **Step 1: Write the failing test** (append to `tests/production.rs`):

```rust
#[test]
fn faucet_rate_covers_routed_demand_per_consumer_pool_at_seed() {
    use crate::economy::production::{ProductionPools, RawDeposits};
    use crate::economy::{
        DemandPools, GoodId, MarketDistances, MarketId, SupplyPools,
    };
    use std::collections::BTreeMap;

    // Helper: for a given world's pools/deposits, for every consumer DemandPool compute the
    // continuous faucet supply of its good reachable from its demand-market and assert >= demand.
    // "Reachable" = same market, or a supply market s with a MarketDistances entry (s, d).
    // Returns the list of (consumer_market, good, demand, reachable_faucet) for inspection.
    fn check(
        demand: &DemandPools,
        supply: &SupplyPools,
        deposits: &RawDeposits,
        production: &ProductionPools,
        distances: &MarketDistances,
    ) -> Vec<(MarketId, GoodId, i64, i64)> {
        // Map extractor-actor -> (supply_market, output_good, faucet_rate). An "extractor" is an
        // actor that has BOTH a RawDeposit and a SupplyPool, producing its output via a recipe.
        let mut rows = Vec::new();
        for (&actor, dp) in demand.0.iter() {
            let d_market = dp.market;
            let g = dp.good;
            let need = dp.desired_qty_per_tick.0;
            let mut reachable_faucet: i64 = 0;
            for (&s_actor, sp) in supply.0.iter() {
                if sp.good != g {
                    continue;
                }
                // Only count CONTINUOUS supply (an extractor faucet), not finite endowment.
                let Some(dep) = deposits.0.get(&s_actor) else { continue };
                // The faucet feeds a recipe whose output is this good at this supply pool.
                let produces_g = production
                    .0
                    .get(&s_actor)
                    .map(|p| p.recipe.outputs.iter().any(|(og, _)| *og == g))
                    .unwrap_or(false);
                if !produces_g {
                    continue;
                }
                let s_market = sp.market;
                let reaches = s_market == d_market
                    || distances.0.contains_key(&(s_market, d_market));
                if reaches {
                    // faucet rate normalized to per-tick (interval_ticks == 1 in seed).
                    reachable_faucet += dep.qty_per_interval.0 / dep.interval_ticks.max(1) as i64;
                }
            }
            let _ = actor;
            rows.push((d_market, g, need, reachable_faucet));
        }
        rows
    }

    // Build the live seed world (same builder as the sizing test above).
    let mut world = bevy_ecs::world::World::new();
    {
        use crate::routing::{Graph, Node, NodeId, NodeKind, NodeSpatialIndex};
        let node = |id: u32, x: f32, y: f32| Node { id: NodeId(id), position: (x, y), kind: NodeKind::Intersection, legacy_id: None };
        let nodes = vec![node(0, 2.0, 3.0), node(1, 13.0, 3.0), node(2, 16.0, 48.0), node(3, 208.0, 48.0)];
        world.insert_resource(NodeSpatialIndex::from_nodes(&nodes));
        world.insert_resource(Graph::new(nodes, vec![]));
        world.insert_resource(crate::economy::Markets::default());
        world.insert_resource(crate::economy::MarketChunks::default());
        world.insert_resource(crate::economy::AccountBook::default());
        world.insert_resource(crate::economy::InventoryBook::default());
        world.insert_resource(SupplyPools::default());
        world.insert_resource(DemandPools::default());
        world.insert_resource(MarketDistances::default());
        world.insert_resource(crate::economy::MarketGoods::default());
        world.insert_resource(ProductionPools::default());
        world.insert_resource(RawDeposits::default());
    }
    crate::economy::seed::seed_demo_economy(&mut world);

    let rows = check(
        world.resource::<DemandPools>(),
        world.resource::<SupplyPools>(),
        world.resource::<RawDeposits>(),
        world.resource::<ProductionPools>(),
        world.resource::<MarketDistances>(),
    );
    assert!(!rows.is_empty(), "non-vacuous: there are consumer pools to check");
    for (market, good, need, faucet) in &rows {
        assert!(
            faucet >= need,
            "consumer @ {market:?} for {good:?} demands {need}/tick but reachable continuous faucet is {faucet}/tick",
        );
    }

    // NEGATIVE CONTROL — prove the invariant is not vacuous: move BOTH FOOD faucets to m_a.
    // Then 8_022's demand @ m_fb (reachable only from m_fa) loses its faucet and the check must
    // report a shortfall (reachable_faucet == 0 for that FOOD pool).
    {
        use crate::economy::production::EXTRACTOR_FOOD_FA;
        let mut sp = world.resource_mut::<SupplyPools>();
        sp.0.get_mut(&EXTRACTOR_FOOD_FA).unwrap().market = MarketId(9_001); // move m_fa -> m_a
    }
    let broken = check(
        world.resource::<DemandPools>(),
        world.resource::<SupplyPools>(),
        world.resource::<RawDeposits>(),
        world.resource::<ProductionPools>(),
        world.resource::<MarketDistances>(),
    );
    use crate::economy::GOOD_FOOD;
    let fb_food = broken
        .iter()
        .find(|(m, g, _, _)| *m == MarketId(9_004) && *g == GOOD_FOOD)
        .expect("there is a FOOD consumer @ m_fb");
    assert!(
        fb_food.2 > fb_food.3,
        "negative control: FOOD @ m_fb demand {} must now EXCEED reachable faucet {} (proves the check binds on routing)",
        fb_food.2,
        fb_food.3,
    );
}
```

- [ ] **Step 2: Run it — verify it PASSES (positive) and the negative control holds**

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core faucet_rate_covers_routed_demand
```
Expected: PASS. If the positive part fails, the seed is mis-sized (a real bug — STOP and fix the seed, not the test). If the negative control fails, the check isn't routing-aware (fix the `check` helper).

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-core/src/economy/tests/production.rs
git commit -m "test(economy): routing-aware faucet-sizing invariant + negative control"
```

---

## Sub-Slice C — Conservation + Stability + Persist

### Task C1: Extend `conservation_full_plugin_multi_tick` to multi-good (FOOD)

Add a FOOD extractor + FOOD consumer at the SAME single market (auction path), track `GOOD_FOOD` alongside `GOOD_RAW`/`GOOD_TOOLS`, and assert per-actor RAW balance for both extractors.

**Files:** Modify `backend/crates/sim-core/src/economy/tests/conservation.rs::conservation_full_plugin_multi_tick`.

- [ ] **Step 1: Add the FOOD extractor + FOOD consumer to the world setup.** After the existing TOOLS `consumer` `DemandPools` insert (the block ending `…autonomous: Money(5_000), },);` for `consumer`), and after the imports include `EXTRACTOR_FOOD_A` and `GOOD_FOOD`, insert:

```rust
    // FOOD self-sufficiency: a parallel FOOD extractor + FOOD consumer at the SAME market
    // (single-market auction), proving FOOD conserves and flows exactly like TOOLS.
    let food_consumer = EconomicActorId(8_012);
    world.resource_mut::<RawDeposits>().0.insert(
        EXTRACTOR_FOOD_A,
        RawDeposit { good: GOOD_RAW, qty_per_interval: Quantity(10), interval_ticks: 1, last_regen_tick: None },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        EXTRACTOR_FOOD_A,
        ProductionPool {
            actor: EXTRACTOR_FOOD_A,
            recipe: Recipe { inputs: vec![(GOOD_RAW, Quantity(10))], outputs: vec![(GOOD_FOOD, Quantity(10))] },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<SupplyPools>().0.insert(
        EXTRACTOR_FOOD_A,
        SupplyPool {
            actor: EXTRACTOR_FOOD_A, market, good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(10), min_price: Money(500),
            interval_ticks: 1, last_generated_tick: None,
        },
    );
    world.resource_mut::<AccountBook>().deposit(food_consumer, Money(10_000_000)).unwrap();
    world.resource_mut::<DemandPools>().0.insert(
        food_consumer,
        DemandPool {
            actor: food_consumer, market, good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(10), max_price: Money(2_000),
            urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1,
            last_generated_tick: None, last_consumed_tick: None,
            income_last_tick: Money::ZERO, mpc_bps: 8_000, autonomous: Money(5_000),
        },
    );
```

- [ ] **Step 2: Add `food_consumer` to `pool_weights`** — change the `HouseholdSector` insert from `BTreeMap::from([(consumer, 1_i64)])` to:

```rust
    world.insert_resource(HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(consumer, 1_i64), (food_consumer, 1_i64)]),
    });
```

- [ ] **Step 3: Seed the FOOD opening price** — in the `MarketGoods` seeding block, after the TOOLS `(market, GOOD_TOOLS)` price seed, add a `(market, GOOD_FOOD)` price seed:

```rust
    {
        let key = MarketGoodKey { market, good: GOOD_FOOD };
        let mut goods = world.resource_mut::<MarketGoods>();
        let st = goods.0.entry(key).or_insert_with(|| MarketGoodState::new(key));
        st.ewma_reference_price = Money(1_000);
        st.last_settlement_price = Money(1_000);
    }
```

- [ ] **Step 4: Track GOOD_FOOD + per-actor RAW.** Change `let goods_to_track = [GOOD_RAW, GOOD_TOOLS];` to:

```rust
    let goods_to_track = [GOOD_RAW, GOOD_TOOLS, GOOD_FOOD];
```
And after the existing per-good balance assertion loop, add a per-actor RAW balance check (the loop already imports `EconomyEvent`):

```rust
    // Per-actor RAW balance: with two extractors sharing GOOD_RAW, each must have regenerated
    // exactly as much RAW as it consumed (no shared-RAW double-spend / cross-actor leak).
    for actor in [EXTRACTOR_TOOLS, EXTRACTOR_FOOD_A] {
        let regen: i64 = world.resource::<TradeLedger>().0.iter().filter_map(|e| match e {
            EconomyEvent::Regenerated { actor: a, good, qty } if *a == actor && *good == GOOD_RAW => Some(qty.0),
            _ => None,
        }).sum();
        let consumed: i64 = world.resource::<TradeLedger>().0.iter().filter_map(|e| match e {
            EconomyEvent::Consumed { actor: a, good, qty } if *a == actor && *good == GOOD_RAW => Some(qty.0),
            _ => None,
        }).sum();
        assert_eq!(regen, consumed, "actor {actor:?}: RAW regenerated == consumed over the run");
    }
    // FOOD flowed (non-vacuity for the new good).
    assert!(
        world.resource::<TradeLedger>().0.iter()
            .any(|e| matches!(e, EconomyEvent::Produced { good, .. } if *good == GOOD_FOOD)),
        "FOOD was produced"
    );
```

- [ ] **Step 5: Update imports** in the test — add `EXTRACTOR_FOOD_A` to the `production::{…}` import and `GOOD_FOOD` to the `economy::{…}` import within `conservation_full_plugin_multi_tick`.

- [ ] **Step 6: Run it — verify it PASSES**

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core conservation_full_plugin_multi_tick
```
Expected: PASS — money byte-invariant every tick; per-good delta == ledger for RAW, TOOLS, AND FOOD; per-actor RAW balanced.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-core/src/economy/tests/conservation.rs
git commit -m "test(economy): conservation_full_plugin_multi_tick covers FOOD + per-actor RAW"
```

### Task C2: Extend `steady_state_multi_tick` to prove FOOD also lives

Add a FOOD extractor (sole FOOD supplier — no finite endowment) + FOOD consumer at the same market, and assert FOOD lives in a bounded band exactly like TOOLS.

**Files:** Modify `backend/crates/sim-core/src/economy/tests/conservation.rs::steady_state_multi_tick`.

- [ ] **Step 1: Add the FOOD extractor + FOOD consumer** (same shape as C1 Step 1, but the FOOD consumer is bootstrapped from autonomous like the TOOLS one — `desired_qty_per_tick: Quantity(0)`). Insert after the TOOLS `consumer` DemandPool block:

```rust
    let food_consumer = EconomicActorId(8_012);
    world.resource_mut::<RawDeposits>().0.insert(
        EXTRACTOR_FOOD_A,
        RawDeposit { good: GOOD_RAW, qty_per_interval: Quantity(10), interval_ticks: 1, last_regen_tick: None },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        EXTRACTOR_FOOD_A,
        ProductionPool {
            actor: EXTRACTOR_FOOD_A,
            recipe: Recipe { inputs: vec![(GOOD_RAW, Quantity(10))], outputs: vec![(GOOD_FOOD, Quantity(10))] },
            interval_ticks: 1, last_generated_tick: None,
        },
    );
    world.resource_mut::<SupplyPools>().0.insert(
        EXTRACTOR_FOOD_A,
        SupplyPool {
            actor: EXTRACTOR_FOOD_A, market, good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(10), min_price: Money(500),
            interval_ticks: 1, last_generated_tick: None,
        },
    );
    world.resource_mut::<AccountBook>().deposit(food_consumer, Money(1_000_000)).unwrap();
    world.resource_mut::<DemandPools>().0.insert(
        food_consumer,
        DemandPool {
            actor: food_consumer, market, good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(0), max_price: Money(2_000),
            urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1,
            last_generated_tick: None, last_consumed_tick: None,
            income_last_tick: Money::ZERO, mpc_bps: 8_000, autonomous: Money(5_000),
        },
    );
```

- [ ] **Step 2: Add `food_consumer` to `pool_weights`** and seed the FOOD opening price (identical to C1 Steps 2-3): `pool_weights: BTreeMap::from([(consumer, 1_i64), (food_consumer, 1_i64)])`, and the `(market, GOOD_FOOD)` price-seed block.

- [ ] **Step 3: Capture a FOOD tail band.** Add a `food_traded_tail: Vec<i64>` alongside `traded_tail`, and inside `if i >= n - k { … }` push the FOOD market's traded qty:

```rust
            let food_key = MarketGoodKey { market, good: GOOD_FOOD };
            let food_traded = world.resource::<MarketGoods>().0.get(&food_key)
                .map(|s| s.traded_qty_last_tick.0).unwrap_or(0);
            food_traded_tail.push(food_traded);
```
(Declare `let mut food_traded_tail: Vec<i64> = Vec::new();` next to `traded_tail`.)

- [ ] **Step 4: Assert FOOD lives** — after the TOOLS `traded_qty` band asserts, add:

```rust
    // FOOD market also lives in a bounded band (lo > 0) — FOOD self-sustains identically.
    // PRECONDITION (spec §8): TOOLS and FOOD share identical opening prices (Money(1_000))
    // and equal pool_weights, so the two goods stay symmetric and these bands mirror TOOLS'.
    // If a band fails, a REAL asymmetry exists (e.g. divergent ewma_reference_price) — STOP
    // and investigate; do NOT loosen the band to force green.
    let f_lo = min(&food_traded_tail);
    let f_hi = max(&food_traded_tail);
    assert!(f_lo > 0, "FOOD market traded every tick in steady state (lo={f_lo})");
    assert!((f_hi as i128) < (f_lo as i128) * 5, "FOOD traded_qty band hi/lo < 5 (hi={f_hi}, lo={f_lo})");
```
And add a FOOD non-vacuity assertion next to the existing ones:

```rust
    assert!(
        ev.iter().any(|e| matches!(e, EconomyEvent::Produced { good, .. } if *good == GOOD_FOOD)),
        "food produced"
    );
```

- [ ] **Step 5: Update imports** — add `EXTRACTOR_FOOD_A` to `production::{…}` and `GOOD_FOOD` to `economy::{…}` within `steady_state_multi_tick`.

- [ ] **Step 6: Run it — verify it PASSES**

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core steady_state_multi_tick
```
Expected: PASS — money constant every tick; consumer + FOOD bands `lo > 0` and bounded. If a band fails, FOOD is NOT behaving symmetrically to TOOLS — STOP and investigate (do not loosen the band to force green).

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-core/src/economy/tests/conservation.rs
git commit -m "test(economy): steady_state_multi_tick proves FOOD also lives (bounded band)"
```

### Task C3: Three-extractor persist round-trip

**Files:** Test `backend/crates/sim-core/src/economy/tests/persist.rs`.

- [ ] **Step 1: Write the failing test** (append to `tests/persist.rs`):

```rust
#[test]
fn three_extractor_raw_deposits_round_trip() {
    use crate::economy::production::{EXTRACTOR_FOOD_A, EXTRACTOR_FOOD_FA, EXTRACTOR_TOOLS, RawDeposit, RawDeposits};
    use crate::economy::GOOD_RAW;

    let mut world = install_economy();
    for (actor, cursor) in [(EXTRACTOR_TOOLS, Some(7u64)), (EXTRACTOR_FOOD_A, Some(11)), (EXTRACTOR_FOOD_FA, None)] {
        world.resource_mut::<RawDeposits>().0.insert(
            actor,
            RawDeposit { good: GOOD_RAW, qty_per_interval: Quantity(10), interval_ticks: 1, last_regen_tick: cursor },
        );
    }

    let snap = extract_from_world(&world);
    assert_eq!(snap.raw_deposits.len(), 3, "all three extractor deposits persist");
    // Sorted by EconomicActorId (8_031, 8_032, 8_033).
    assert_eq!(snap.raw_deposits[0].0, EXTRACTOR_TOOLS);
    assert_eq!(snap.raw_deposits[1].0, EXTRACTOR_FOOD_A);
    assert_eq!(snap.raw_deposits[2].0, EXTRACTOR_FOOD_FA);

    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &decoded);
    assert_eq!(fresh.resource::<RawDeposits>().0.len(), 3);
    assert_eq!(fresh.resource::<RawDeposits>().0[&EXTRACTOR_FOOD_A].last_regen_tick, Some(11));
    assert_eq!(snap, extract_from_world(&fresh), "full snapshot identity with three raw_deposits");
}
```

- [ ] **Step 2: Run it — verify it PASSES** (persist is already general; this is a regression guard):

```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core three_extractor_raw_deposits_round_trip
```
Expected: PASS. If it fails, the persist layer is NOT general over the map — STOP and investigate.

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-core/src/economy/tests/persist.rs
git commit -m "test(economy): three-extractor raw_deposits persist round-trip"
```

### Task C4: Full local gate

**Files:** none (verification only); commit only if a gate fix was required.

- [ ] **Step 1: Rust gate (fmt-check, clippy, full sim-core test, then sim-server)** — run each, all must be clean:

```bash
cd /Users/ramonfuglister/Coding/abutown-vtraders
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml -- --check
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace
```
Expected: fmt clean; clippy exit 0; all workspace tests pass. (The `--workspace` test run here at the END is allowed — it is the final gate, not iteration.)

- [ ] **Step 2: Frontend gate** (typecheck src+tests+scripts, vitest, build) — the slice is backend-only, but the gate is mandatory before push:

```bash
cd /Users/ramonfuglister/Coding/abutown-vtraders
npm run typecheck && npx vitest run && node scripts/build.mjs
```
Expected: typecheck clean, all vitest pass, build succeeds.

- [ ] **Step 3: e2e render-smoke** (the seed changed — two new FOOD supply pools at m_a/m_fa; confirm the live world still renders and the smoke passes):

```bash
cd /Users/ramonfuglister/Coding/abutown-vtraders
npm run test:e2e
```
Expected: render-smoke passes. **If it fails because pinned agent counts changed** (the extra continuous FOOD supply may alter flow-shipment/visible-agent counts), inspect the diff: if the new counts are a correct consequence of the FOOD extractors (more sustained FOOD flow), UPDATE the pinned expectations with a one-line justification in the test; if counts are unexpected (e.g. zero, or wildly off), STOP — that is a real regression. Do NOT blindly bump the numbers.

- [ ] **Step 4: Commit any gate fix** (only if Steps 1-3 required a code change):

```bash
git add -A && git commit -m "fix(economy): <describe the gate fix>"
```

---

## PR-body notes (for `finishing-a-development-branch`)

1. **No new mandatory DELETE beyond #75's.** This slice adds NO new snapshot field (reuses #75's `raw_deposits`/`ProductionPools`/`SupplyPools` maps). Old snapshots still deserialize. But because seeding is fresh-world-only (no heal-on-restore), the FOOD extractors appear only on fresh worlds — a world persisted under #75 needs the SAME one-time `DELETE FROM economy_snapshots` (already required by #75's `raw_deposits` field) to re-seed and gain them. Net: ONE DELETE total when #75 + this slice deploy together.
2. **FOOD's money circuit now closes like TOOLS.** The FOOD extractors are firms; their FOOD-sale revenue flows through the existing #75 seller→wage→100%-profit→household machinery, so perpetual FOOD sales fund perpetual household income (replacing the finite, draining endowment income). No new money code; `total_money` byte-invariant.
3. **`EXTRACTOR` renamed to `EXTRACTOR_TOOLS`** (mechanical, 70 refs / 10 files) so the three extractors are explicitly named.
4. **Stacked on #75 (PR #75 unmerged).** This branch (`plan/economy-food-self-sufficiency`) contains #75's commits; its diff narrows to just the FOOD slice once #75 merges to main. The FOOD-on-finite-endowment limitation #75 recorded is now CLOSED.
5. **Deferred (unchanged):** dedicated `GOOD_RAW_FOOD` + shared-input contention; multi-stage chains; free/market-clearing prices (the runner-up SOTA realism slice); profit-leak recovery + release-grade SFC audit; per-capita consumption scaling; explicit labor market. Faucet sizing remains static-hand-sized (now guarded by the routing-aware invariant, not yet runtime-self-correcting — that waits on free prices).

---

## Self-Review (run after writing — done)

**Spec coverage:** §1 problem → Tasks B2/C2 (FOOD now sustains). §2 topology + §3 two co-located extractors → B2 (seed at m_a/m_fa). §4 reuse GOOD_RAW → B1/B2 (per-actor faucet). §5 conservation / money circuit → C1 (money byte-invariant + per-actor RAW) + PR note 2. §6 routing-aware sizing → B3 (+ negative control). §7 rename → A1 (+ grep verify). §8 schedule unchanged → no task needed (generic systems). §9 persistence → C3 + PR note 1. §10 scaling → O(3) extractors, inherent. §11 touched files → all covered. §12 tests 1-7 → B1(1,6 mechanism+per-actor RAW), B2(3 seed), B3(4 sizing), C1(per-good FOOD + per-actor RAW), C2(5 steady-state FOOD), C3(7 persist), and item 2 (RAW never traded) → B2 Step 1 final asserts. §13 sub-slices A/B/C → matched.

**Placeholder scan:** none — every step has exact code + exact commands.

**Type consistency:** `RawDeposit`/`ProductionPool`/`SupplyPool`/`DemandPool`/`Recipe` field names match the pinned Verified Facts; `EXTRACTOR_FOOD_A/FA` used consistently; `MarketDistances.0: BTreeMap<(MarketId,MarketId), i64>` matches the B3 `.contains_key(&(s_market, d_market))` usage; `GOOD_FOOD=GoodId(1)`/`GOOD_RAW=GoodId(5)` consistent.
