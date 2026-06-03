# Economy Consumption — Slice 1 (aggregate sink) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the aggregate consumption authority — consumers consume `min(held, desired_qty_per_tick)` of delivered goods per interval, emit a new `FinalConsumed` event, closing the production→consumption loop. Runs for ALL markets (viewport-independent). Spec: `docs/superpowers/specs/2026-06-02-economy-consumption-design.md`.

**Architecture:** Mirror `run_production_at_tick` (`production.rs:27-68`): a pure `run_consumption_at_tick` iterating `DemandPools` keys-first, interval-gated on a new `last_consumed_tick` cursor, clamping `qty = min(held available, desired)` so it can never fault, in-place (no scratch boundary). A new `EconomySet::Consume` runs it after `MacroFlow` / before `ShopperCapture`. Conservation shifts: `total_good` is no longer invariant → `Δtotal_good(g) == ΣProduced − Σ(Consumed+FinalConsumed)` (auditable from the ledger); money exactly invariant.

**Tech Stack:** Rust, `bevy_ecs`. **Cargo MUST run via `scripts/cargo-serial.sh` with `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target`.** Scoped `-p sim-core` during iteration; never RUN a bench. `cargo fmt --manifest-path backend/Cargo.toml --all` before every commit (NOT the bare `cargo-serial.sh fmt`).

---

### Task 1: `EconomyEvent::FinalConsumed` event variant

**Files:** Modify `backend/crates/sim-core/src/economy/ledger.rs` (enum + `event_type`); Test `backend/crates/sim-core/src/economy/tests/audit.rs`.

- [ ] **Step 1: Write the failing tag test**

Append to `tests/audit.rs`:

```rust
#[test]
fn final_consumed_event_tag() {
    let e = crate::economy::EconomyEvent::FinalConsumed {
        actor: crate::economy::EconomicActorId(1),
        good: crate::economy::GOOD_FOOD,
        qty: crate::economy::Quantity(3),
    };
    assert_eq!(e.event_type(), "final_consumed");
}
```

- [ ] **Step 2: Run — verify it fails**

Run: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core final_consumed_event_tag`
Expected: FAIL — `no variant ... FinalConsumed`.

- [ ] **Step 3: Add the variant + tag**

In `ledger.rs`, after the `Consumed { ... }` variant (`:52-56`):

```rust
    Consumed {
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    },
    /// Final consumption by an end-buyer (the demand-side sink), distinct from
    /// production's recipe-input `Consumed`. Both are goods-removals; splitting the
    /// variant keeps intermediate vs final consumption distinguishable in the audit log.
    FinalConsumed {
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    },
```

In `event_type()`, after the `Consumed` arm (`:96`):

```rust
            Self::Consumed { .. } => "consumed",
            Self::FinalConsumed { .. } => "final_consumed",
```

- [ ] **Step 4: Run — verify it passes**

Run: same as Step 2. Expected: PASS.

- [ ] **Step 5: Format + commit**

```bash
cargo fmt --manifest-path backend/Cargo.toml --all
git add backend/crates/sim-core/src/economy/ledger.rs backend/crates/sim-core/src/economy/tests/audit.rs
git commit -m "feat(economy): add EconomyEvent::FinalConsumed (demand-side sink event)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `DemandPool.last_consumed_tick` cursor field (compile-only, behavior-neutral)

**Files:** Modify `backend/crates/sim-core/src/economy/pools.rs` (struct) + EVERY `DemandPool { … }` literal across the crate.

- [ ] **Step 1: Add the field**

In `pools.rs`, in `DemandPool`, after `last_generated_tick: Option<u64>,`:

```rust
    pub last_generated_tick: Option<u64>,
    /// Cursor for the consumption sink (run_consumption_at_tick). MUST be separate from
    /// last_generated_tick (which gates bidding). Persists for free in demand_pools.
    pub last_consumed_tick: Option<u64>,
```

(`Option<u64>: Copy` → the `DemandPool` `Copy` derive survives; the `*v` persist extract keeps compiling.)

- [ ] **Step 2: Compile to enumerate the broken literals**

Run: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-core`
Expected: FAIL — `missing field last_consumed_tick` at each `DemandPool { … }` literal.

- [ ] **Step 3: Add `last_consumed_tick: None` to every literal**

Production code: `seed.rs:120, :160, :253`. Tests/helpers: `tests/flow_shipments.rs:61, :211`, `tests/systems.rs:29`, `tests/persist.rs:109`, `tests/macro_flow.rs:113, :1080, :1259, :1273, :1415, :2450`, `tests/pools.rs:24, :119`, `tests/lod.rs:56, :292, :480`. (Add it once inside each helper constructor — `lod.rs:55 seeded_demand_pool`, `macro_flow.rs:1414 dp(...)`, etc.) Use the compiler's error list as the authoritative set; add `last_consumed_tick: None,` after each `last_generated_tick: …,`.

- [ ] **Step 4: Compile clean + run sim-core lib**

Run: `… scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --lib`
Expected: PASS — every existing test byte-identical (the field is unused; no behavior change).

- [ ] **Step 5: Format + commit**

```bash
cargo fmt --manifest-path backend/Cargo.toml --all
git add -A
git commit -m "feat(economy): add DemandPool.last_consumed_tick cursor (behavior-neutral)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: `run_consumption_at_tick` pure fn + isolation test suite

**Files:** Modify `backend/crates/sim-core/src/economy/pools.rs` (the fn); Test `backend/crates/sim-core/src/economy/tests/pools.rs` (or a new section).

- [ ] **Step 1: Write the failing tests**

Append to `tests/pools.rs` (uses the already-imported helpers; add imports as the compiler requires — `InventoryBook`, `TradeLedger`, `DemandPools`, `run_consumption_at_tick`):

```rust
fn consume_pool(actor: u64, market: u64, want: i64) -> DemandPool {
    DemandPool {
        actor: EconomicActorId(actor),
        market: MarketId(market),
        good: GOOD_FOOD,
        desired_qty_per_tick: Quantity(want),
        max_price: Money(1_000),
        urgency_bps: 0,
        elasticity_bps: 0,
        interval_ticks: 1,
        last_generated_tick: None,
        last_consumed_tick: None,
    }
}

#[test]
fn consumption_removes_min_held_want_and_emits_finalconsumed() {
    let owner = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(owner, GOOD_FOOD, Quantity(4)).unwrap(); // held 4 < want 10
    let mut ledger = TradeLedger::default();
    let mut demand = DemandPools::default();
    demand.0.insert(owner, consume_pool(1, 10, 10));
    let good_before = inv.total_good(GOOD_FOOD).unwrap();

    run_consumption_at_tick(&mut inv, &mut ledger, &mut demand, 0).unwrap();

    // consumed exactly min(held 4, want 10) = 4, no fault.
    assert_eq!(inv.balance(owner, GOOD_FOOD).available, Quantity(0));
    assert_eq!(inv.total_good(GOOD_FOOD).unwrap().0, good_before.0 - 4, "goods removed by exactly consumed");
    assert!(
        ledger.0.iter().any(|e| matches!(e,
            crate::economy::EconomyEvent::FinalConsumed { actor, qty, .. }
            if *actor == owner && *qty == Quantity(4))),
        "a clamped FinalConsumed(4) event is pushed"
    );
    // cursor advanced.
    assert_eq!(demand.0[&owner].last_consumed_tick, Some(0));
}

#[test]
fn consumption_conserves_money_and_is_deterministic() {
    let build = || {
        let mut inv = InventoryBook::default();
        for a in [1_u64, 2] {
            inv.deposit(EconomicActorId(a), GOOD_FOOD, Quantity(10)).unwrap();
        }
        let mut acc = AccountBook::default();
        acc.deposit(EconomicActorId(1), Money(500)).unwrap();
        let mut ledger = TradeLedger::default();
        let mut demand = DemandPools::default();
        demand.0.insert(EconomicActorId(1), consume_pool(1, 10, 3));
        demand.0.insert(EconomicActorId(2), consume_pool(2, 11, 5));
        let m0 = acc.total_money().unwrap();
        run_consumption_at_tick(&mut inv, &mut ledger, &mut demand, 0).unwrap();
        assert_eq!(acc.total_money().unwrap(), m0, "money invariant across consume");
        ledger.0
    };
    assert_eq!(build(), build(), "consumption is deterministic");
}

#[test]
fn consumption_respects_interval_cursor() {
    let owner = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(owner, GOOD_FOOD, Quantity(100)).unwrap();
    let mut ledger = TradeLedger::default();
    let mut demand = DemandPools::default();
    let mut p = consume_pool(1, 10, 4);
    p.interval_ticks = 5;
    demand.0.insert(owner, p);

    run_consumption_at_tick(&mut inv, &mut ledger, &mut demand, 0).unwrap(); // tick 0: consumes (cursor None)
    run_consumption_at_tick(&mut inv, &mut ledger, &mut demand, 2).unwrap(); // tick 2: < interval 5 -> skip
    assert_eq!(inv.balance(owner, GOOD_FOOD).available, Quantity(96), "only one interval consumed (4)");
    run_consumption_at_tick(&mut inv, &mut ledger, &mut demand, 5).unwrap(); // tick 5: elapsed -> consumes
    assert_eq!(inv.balance(owner, GOOD_FOOD).available, Quantity(92));
}
```

- [ ] **Step 2: Run — verify they fail**

Run: `… test --manifest-path backend/Cargo.toml -p sim-core consumption_`
Expected: FAIL — `cannot find function run_consumption_at_tick`.

- [ ] **Step 3: Implement `run_consumption_at_tick`**

In `pools.rs`, after `run_production_at_tick`'s home module (place beside the other pool fns; `EconomyEvent`, `interval_elapsed`, `InventoryBook`, `TradeLedger`, `Quantity` are in scope):

```rust
/// The demand-side SINK (mirror of run_production_at_tick): each consumer consumes
/// `min(held available, desired_qty_per_tick)` of its good per interval, emitting a
/// `FinalConsumed` event. Pure, deterministic (keys-first, no clone — DemandPool is Copy),
/// in-place. The `min` clamp guarantees `qty <= available` so `consume` can never fault.
/// NOT gated on DormantMarkets — the sink is an aggregate authority that runs for ALL
/// markets (viewport-independent). Conservation: total_good drops by exactly Σ qty;
/// money untouched.
pub fn run_consumption_at_tick(
    inventory: &mut InventoryBook,
    ledger: &mut TradeLedger,
    demand: &mut DemandPools,
    current_tick: u64,
) -> Result<(), EconomyError> {
    let actors: Vec<EconomicActorId> = demand.0.keys().copied().collect();
    for actor in actors {
        let pool = demand.0[&actor];
        if !interval_elapsed(pool.last_consumed_tick, current_tick, pool.interval_ticks) {
            continue;
        }
        let available = inventory.balance(actor, pool.good).available;
        let qty = Quantity(pool.desired_qty_per_tick.0.min(available.0));
        if qty.0 > 0 {
            inventory.consume(actor, pool.good, qty)?;
            ledger.0.push(EconomyEvent::FinalConsumed {
                actor,
                good: pool.good,
                qty,
            });
        }
        if let Some(p) = demand.0.get_mut(&actor) {
            p.last_consumed_tick = Some(current_tick);
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Run — verify pass**

Run: `… test --manifest-path backend/Cargo.toml -p sim-core consumption_`
Expected: PASS (3 passed).

- [ ] **Step 5: Format + commit**

```bash
cargo fmt --manifest-path backend/Cargo.toml --all
git add backend/crates/sim-core/src/economy/pools.rs backend/crates/sim-core/src/economy/tests/pools.rs
git commit -m "feat(economy): run_consumption_at_tick — the pure demand-side sink

min(held, desired) per interval, emits FinalConsumed; pure/deterministic/in-place,
cannot fault (clamp); not yet wired. Isolation tests: clamp+event, money-invariance+
determinism, interval cursor.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Wire `EconomySet::Consume` (activate) + convert the integration tests

**Files:** Modify `backend/crates/sim-core/src/economy/systems.rs` (enum + chain + system + import); convert `tests/lod.rs` (`active_to_dormant_handoff_conserves`) and `tests/systems.rs` (`economy_clears_a_trade_end_to_end`, audit `dirty_market_keys_are_processed_in_stable_order`).

- [ ] **Step 1: Add the set, the system, the import, the schedule slot**

In `systems.rs`: add `Consume` to the `EconomySet` enum (`:17-28`) between `MacroFlow` and `ShopperCapture`; the SAME insertion in the `.chain()` tuple (`:70-83`). Add `run_consumption_at_tick` to the `use crate::economy::{…}` import (`:6-12`). Add the wrapper + registration:

```rust
pub fn run_consumption_system(
    tick: Res<Tick>,
    mut inventory: ResMut<InventoryBook>,
    mut ledger: ResMut<TradeLedger>,
    mut demand: ResMut<DemandPools>,
) {
    let _ = run_consumption_at_tick(&mut inventory, &mut ledger, &mut demand, tick.0);
}
```

In the parallel `add_systems` tuple (`:92-103`), add: `run_consumption_system.in_set(EconomySet::Consume),`. (Do NOT add a `pub use` — `mod.rs:37` glob already re-exports the fn.)

- [ ] **Step 2: Run the full schedule tests to see the expected reds**

Run: `… test --manifest-path backend/Cargo.toml -p sim-core`
Expected: FAIL at `active_to_dormant_handoff_conserves` (lod.rs — raw total_good invariant) and `economy_clears_a_trade_end_to_end` (systems.rs — post-consume inventory == 0 ≠ 1000). Both are the spec's anticipated CI-reds.

- [ ] **Step 3: Convert `tests/systems.rs::economy_clears_a_trade_end_to_end` (`:97-103`)**

The trade still clears (auction unaffected); the consumer's delivered goods are then drained by the same-tick sink. Assert the trade happened via the ledger / pre-consume state rather than post-tick inventory. Replace the `inventory.balance(buyer, GOOD_FOOD).available == Quantity(1_000)` assertion with an assertion that the `Trade` cleared (a `Trade`/`MacroFlow` event in the ledger, or `total_money` conserved) and that `total_good` delta equals `−FinalConsumed` for that tick:

```rust
    // The trade cleared (1000 delivered) and was then consumed by the same-tick sink.
    let final_consumed: i64 = world
        .resource::<TradeLedger>()
        .0
        .iter()
        .filter_map(|e| match e {
            EconomyEvent::FinalConsumed { good, qty, .. } if *good == GOOD_FOOD => Some(qty.0),
            _ => None,
        })
        .sum();
    assert_eq!(final_consumed, 1_000, "the delivered 1000 FOOD was consumed by the buyer");
    assert_eq!(
        world.resource::<InventoryBook>().balance(buyer, GOOD_FOOD).available,
        Quantity(0),
        "post-consume buyer inventory is drained (sink == delivery)"
    );
```

(Adjust to the test's exact variable names — it builds a world + runs the schedule; read it and adapt. Also re-check `dirty_market_keys_are_processed_in_stable_order` `:115` — if it asserts buyer inventory, apply the same fix; if it only asserts dirty-key order, it stays green.)

- [ ] **Step 4: Convert `tests/lod.rs::active_to_dormant_handoff_conserves` (`:526-533`, `:552-553`)**

Keep the money-conservation assertion. Replace the per-tick raw `total_good(GOOD_FOOD) == good_total` assertion with the ledger-derived invariant: track cumulative `Produced − (Consumed + FinalConsumed)` for FOOD and assert `total_good(FOOD) == good_total + that_delta`:

```rust
    // Goods are no longer invariant (the sink consumes). Assert the ledger-derived
    // conservation: Δtotal_good(FOOD) == Σ Produced − Σ (Consumed + FinalConsumed).
    let net: i64 = world
        .resource::<TradeLedger>()
        .0
        .iter()
        .filter_map(|e| match e {
            EconomyEvent::Produced { good, qty, .. } if *good == GOOD_FOOD => Some(qty.0),
            EconomyEvent::Consumed { good, qty, .. } if *good == GOOD_FOOD => Some(-qty.0),
            EconomyEvent::FinalConsumed { good, qty, .. } if *good == GOOD_FOOD => Some(-qty.0),
            _ => None,
        })
        .sum();
    assert_eq!(
        world.resource::<InventoryBook>().total_good(GOOD_FOOD).unwrap().0,
        good_total.0 + net,
        "goods conserved against the ledger (Produced − Consumed − FinalConsumed)"
    );
```

Apply at BOTH assertion sites (`:526-533` tick-0 and `:552-553` the loop) — note `net` is cumulative over the whole ledger, so compute it fresh at each check against the original `good_total`. Keep `total_money == money_total`.

- [ ] **Step 5: Run the full sim-core suite — verify green**

Run: `… test --manifest-path backend/Cargo.toml -p sim-core`
Expected: PASS — incl. `macro_flow_replays_across_restart` (the sink mutates only InventoryBook + pushes events; the cursor persists → both continuations consume identically; the ledger tail is now `FinalConsumed`-dominated but identical). If it fails, the cursor isn't persisting deterministically — investigate.

- [ ] **Step 6: Format + commit**

```bash
cargo fmt --manifest-path backend/Cargo.toml --all
git add -A
git commit -m "feat(economy): wire EconomySet::Consume — activate the demand-side sink

run_consumption_system runs after MacroFlow / before ShopperCapture for ALL markets.
The production->consumption loop now closes (economy flows instead of freezing).
Converted the two anticipated integration reds to the ledger-derived conservation
invariant (Δtotal_good == ΣProduced − Σ(Consumed+FinalConsumed); money still invariant).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Full Rust gate

**Files:** none (verification only). Slice 1 is backend-only (no frontend/wire/render).

- [ ] **Step 1: fmt-check** — `cargo fmt --manifest-path backend/Cargo.toml --all -- --check` → clean.
- [ ] **Step 2: clippy (background, poll)** — `… scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings` → clean. (Watch for an unused-import or a `dead_code` on the new fn if the wiring somehow didn't reference it.)
- [ ] **Step 3: sim-server tests (background, poll)** — `… scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server`. The runtime seeds DemandPools; the sink now runs in the live schedule — confirm no runtime assertion (e.g. an inventory expectation) broke; if one does, it asserted pre-sink inventory and must convert to the ledger invariant. **Wait for COMPLETE before judging.**
- [ ] **Step 4: Confirm green** ⇒ Slice 1 complete. Note: the demo now needs a one-time `DELETE FROM economy_snapshots` before any DB deploy (non-defaultable `last_consumed_tick`). Slice 2 (shopper projects `consumed_qty`) is next; coordinate PR strategy with the user.
