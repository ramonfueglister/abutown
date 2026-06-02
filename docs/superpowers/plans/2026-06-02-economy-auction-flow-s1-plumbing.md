# Economy Auction↔Flow Coupling — S1 Plumbing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Thread `&mut OrderBook` + `&mut NextOrderId` through `run_macro_flow_at_tick` and establish the full clone-validate-apply topology (top-level boundary clone + per-edge scratch fold) that S3's residual-drain will mutate — with **zero behavior change** (the flow still reads dormant pools only; no drain, no gate change).

**Architecture:** `run_macro_flow_at_tick` already does atomic clone-validate-apply: it clones `accounts`/`inventory`/`market_goods` at a top-level boundary (after the `flows.is_empty()` early return, preserving the no-clone-on-quiescent-interval property), settles each cross-edge into a per-edge scratch clone, folds the scratch back on `Ok` / drops it on `Err`, then commits the boundary clones to the live books. S1 extends this boundary to carry the `OrderBook` (`next_orders` boundary clone + `scratch_orders` per-edge, folded on `Ok`) and the `NextOrderId` counter (snapshot + commit-time write-back) — so in S3 the drain's lock-release and order-row mutation land **inside** the existing per-edge fault isolation, not outside it. S1 mutates neither; both clones round-trip unchanged, proven by a new guard test and the existing `macro_flow_replays_across_restart` (which runs the full schedule + persistence round-trip). A `EconomyConfig.drain_active_residual: bool` flag is added, **defaulted FALSE**, so S1+S2 land dark; S3 flips it.

**Tech Stack:** Rust, `bevy_ecs` (ECS resources/systems), `serde` (persistence). Tests are crate-internal unit tests under `backend/crates/sim-core/src/economy/tests/`. **Cargo MUST run via `scripts/cargo-serial.sh` with the isolated env** (`TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target`) to avoid the build-lock conflict with the user's parallel dev server. Never run a broad `--workspace --all-targets` during iteration; run the scoped `-p sim-core` command given. `cargo fmt` before every commit.

---

### Task 1: Add `EconomyConfig.drain_active_residual` flag (default FALSE)

**Files:**
- Modify: `backend/crates/sim-core/src/economy/systems.rs:33-47` (struct) and `:49-63` (Default impl)
- Test: `backend/crates/sim-core/src/economy/tests/macro_flow.rs` (new test)

- [ ] **Step 1: Write the failing test**

Add at the end of `backend/crates/sim-core/src/economy/tests/macro_flow.rs`:

```rust
#[test]
fn drain_active_residual_defaults_off() {
    // S1 lands the config surface dark: S1+S2 must not change behavior, so the
    // drain flag is FALSE by default; S3 flips it. This guards that safety property.
    assert!(!EconomyConfig::default().drain_active_residual);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core drain_active_residual_defaults_off`
Expected: FAIL — compile error `no field 'drain_active_residual' on type 'EconomyConfig'`.

- [ ] **Step 3: Add the field + default**

In `systems.rs`, add the field to the struct (after `shopper_radius_tiles: f32,` at `:46`):

```rust
    /// Radius (tiles) around a market to pick shopper origin nodes.
    pub shopper_radius_tiles: f32,
    /// When TRUE, the macro flow drains active/observed markets' post-auction
    /// residual orders into the inter-market flow (S3). FALSE keeps the flow
    /// dormant-only (S1/S2 land dark). Defaulted FALSE; S3 flips it.
    pub drain_active_residual: bool,
}
```

And in the `Default` impl (after `shopper_radius_tiles: 24.0,` at `:61`):

```rust
            shopper_radius_tiles: 24.0,
            drain_active_residual: false,
        }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core drain_active_residual_defaults_off`
Expected: PASS (1 passed).

- [ ] **Step 5: Format + commit**

```bash
cargo fmt --manifest-path backend/Cargo.toml
git add backend/crates/sim-core/src/economy/systems.rs backend/crates/sim-core/src/economy/tests/macro_flow.rs
git commit -m "feat(economy): add EconomyConfig.drain_active_residual flag (default false)

S1 plumbing — lands the config surface for the auction<->flow coupling redesign
dark. FALSE keeps the macro flow dormant-only; S3 flips it. No behavior change.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Thread `&mut OrderBook` + `&mut NextOrderId` through the macro-flow atomic boundary

**Files:**
- Modify: `backend/crates/sim-core/src/economy/macro_flow.rs:560-694` (signature + topology)
- Modify: `backend/crates/sim-core/src/economy/systems.rs:335-364` (prod caller `run_macro_flow_system`)
- Modify: `backend/crates/sim-core/src/economy/tests/macro_flow.rs:1466-1482` (`run_flow` helper) and the 4 direct calls at `:1103, :1121, :1156, :1299`
- Modify: `backend/crates/sim-core/src/economy/tests/flow_shipments.rs:100, :246` (2 direct calls)
- Test: `backend/crates/sim-core/src/economy/tests/macro_flow.rs` (new guard test)

**Note on scope:** `DormantScenario` (struct at `:1364`, 7 literals) is **NOT** touched — `run_flow` passes a default `OrderBook`/`NextOrderId`, and the new guard test calls `run_macro_flow_at_tick` directly with a populated `OrderBook`. This keeps the blast radius to the 8 existing call sites + the prod caller.

- [ ] **Step 1: Write the failing guard test**

Add at the end of `backend/crates/sim-core/src/economy/tests/macro_flow.rs`:

```rust
#[test]
fn macro_flow_threads_orderbook_and_counter_unchanged() {
    // S1 behavior-neutral threading: a populated OrderBook + a non-zero NextOrderId
    // ride through the macro-flow atomic boundary UNTOUCHED, while the dormant flow
    // still executes its cross-edge. Guards the clone topology S3 will mutate.
    let mut s = surplus_deficit_scenario(1, 200, 500, 1, 200, 2000, 4, Money(50));

    // Arbitrary residual orders the dormant-flow path must ignore entirely in S1.
    let mut orders = crate::economy::OrderBook::default();
    orders.bids.insert(
        crate::economy::OrderId(7),
        crate::economy::Bid {
            id: crate::economy::OrderId(7),
            owner: EconomicActorId(99),
            market: MarketId(9_001),
            good: GOOD_FOOD,
            qty_remaining: Quantity(5),
            max_price: Money(1_500),
            cash_locked_remaining: Money(7_500),
            created_tick: 0,
            expires_tick: 100,
        },
    );
    orders.asks.insert(
        crate::economy::OrderId(8),
        crate::economy::Ask {
            id: crate::economy::OrderId(8),
            owner: EconomicActorId(98),
            market: MarketId(9_002),
            good: GOOD_FOOD,
            qty_remaining: Quantity(5),
            min_price: Money(400),
            goods_locked_remaining: Quantity(5),
            created_tick: 0,
            expires_tick: 100,
        },
    );
    let mut next_oid = crate::economy::NextOrderId(42);

    let orders_before = orders.clone();
    let oid_before = next_oid;

    run_macro_flow_at_tick(
        &mut s.accounts,
        &mut s.inventory,
        &mut s.ledger,
        &s.demand,
        &s.supply,
        &mut s.market_goods,
        &s.dirty,
        &s.dormant,
        &s.distances,
        &s.config,
        0,
        &mut crate::economy::FlowShipments::default(),
        &mut crate::economy::NextShipmentId::default(),
        &mut orders,
        &mut next_oid,
    )
    .unwrap();

    assert_eq!(orders, orders_before, "OrderBook must round-trip unchanged in S1");
    assert_eq!(next_oid, oid_before, "NextOrderId must round-trip unchanged in S1");
    assert!(
        s.ledger
            .0
            .iter()
            .any(|e| matches!(e, crate::economy::EconomyEvent::MacroFlow { .. })),
        "the dormant flow still executed its cross-edge while the OrderBook was carried through"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core macro_flow_threads_orderbook_and_counter_unchanged`
Expected: FAIL — compile error `E0061: this function takes 13 arguments but 15 arguments were supplied` (signature not yet extended).

- [ ] **Step 3: Extend the `run_macro_flow_at_tick` signature**

In `macro_flow.rs`, add two params after `next_shipment_id` (`:573`):

```rust
    shipments: &mut crate::economy::FlowShipments,
    next_shipment_id: &mut crate::economy::NextShipmentId,
    orders: &mut crate::economy::OrderBook,
    next_order_id: &mut crate::economy::NextOrderId,
) -> Result<(), EconomyError> {
```

- [ ] **Step 4: Add the boundary clone + counter snapshot**

In `macro_flow.rs`, in the boundary block (after `let mut next_goods = market_goods.clone();` at `:607`), add:

```rust
    let mut next_accounts = accounts.clone();
    let mut next_inventory = inventory.clone();
    let mut next_goods = market_goods.clone();
    // S1: carry the OrderBook + id counter through the SAME atomic boundary so S3's
    // residual-drain mutation lands inside the per-edge fault isolation below, not
    // outside it. S1 mutates neither — both round-trip unchanged (guarded by test).
    let mut next_orders = orders.clone();
    let next_oid = *next_order_id;
    let mut events: Vec<EconomyEvent> = Vec::new();
```

- [ ] **Step 5: Add the per-edge scratch clone + fold**

In `macro_flow.rs`, in the per-edge scratch block (after `let mut scratch_goods = next_goods.clone();` at `:641`), add `scratch_orders`:

```rust
        let mut scratch_accounts = next_accounts.clone();
        let mut scratch_inventory = next_inventory.clone();
        let mut scratch_goods = next_goods.clone();
        let mut scratch_orders = next_orders.clone(); // S1: threaded, not yet mutated
```

In the `Ok(event) =>` fold arm (after `next_goods = scratch_goods;` at `:659`), add:

```rust
            Ok(event) => {
                next_accounts = scratch_accounts;
                next_inventory = scratch_inventory;
                next_goods = scratch_goods;
                next_orders = scratch_orders; // S1: fold (round-trips unchanged)
```

(The `Err` arm drops `scratch_orders` by scope-exit, exactly like the other scratch clones — no change needed.)

- [ ] **Step 6: Commit the boundary clones to the live books**

In `macro_flow.rs`, in the commit block (`:689-692`), add the two new write-backs:

```rust
    *accounts = next_accounts;
    *inventory = next_inventory;
    *market_goods = next_goods;
    *orders = next_orders;
    *next_order_id = next_oid;
    ledger.0.extend(events);
    Ok(())
```

- [ ] **Step 7: Update the prod caller `run_macro_flow_system`**

In `systems.rs`, add two `ResMut` params after `next_shipment_id` (`:348`):

```rust
    mut shipments: ResMut<FlowShipments>,
    mut next_shipment_id: ResMut<NextShipmentId>,
    mut orders: ResMut<OrderBook>,
    mut next_order_id: ResMut<NextOrderId>,
) {
```

And pass them in the call (after `&mut next_shipment_id,` at `:363`):

```rust
        &mut shipments,
        &mut next_shipment_id,
        &mut orders,
        &mut next_order_id,
    ) {
```

(`OrderBook` and `NextOrderId` are already imported in `systems.rs:9`. The `EconomySet` chain already orders `MacroFlow` after the other `OrderBook` writers — `ExpireOrders`/`GeneratePoolOrders`/`ClearMarkets` — so the new `ResMut<OrderBook>` access is serialized; no system-ambiguity panic. The full-schedule `macro_flow_replays_across_restart` test verifies this.)

- [ ] **Step 8: Update the remaining direct call sites**

Append `&mut crate::economy::OrderBook::default(), &mut crate::economy::NextOrderId::default(),` (immediately after the `&mut crate::economy::NextShipmentId::default(),` line) at each of:
- `tests/macro_flow.rs:1480` (inside the `run_flow` helper)
- `tests/macro_flow.rs:1103, :1121, :1156, :1299` (4 direct calls — note line numbers shift after the helper edit; locate each `&mut crate::economy::NextShipmentId::default(),` followed by `)` and append)
- `tests/flow_shipments.rs:100, :246` (2 direct calls)

Each becomes, e.g.:

```rust
        &mut crate::economy::FlowShipments::default(),
        &mut crate::economy::NextShipmentId::default(),
        &mut crate::economy::OrderBook::default(),
        &mut crate::economy::NextOrderId::default(),
    )
```

- [ ] **Step 9: Run the full sim-core suite to verify green (guard + existing byte-identical)**

Run: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core`
Expected: PASS — the new `macro_flow_threads_orderbook_and_counter_unchanged` passes AND every existing test (incl. `macro_flow_replays_across_restart`, all `flow_shipments`/`macro_flow` tests) passes unchanged. If any existing test fails, the threading changed behavior — STOP and investigate (the clone must round-trip).

- [ ] **Step 10: Format + commit**

```bash
cargo fmt --manifest-path backend/Cargo.toml
git add backend/crates/sim-core/src/economy/macro_flow.rs backend/crates/sim-core/src/economy/systems.rs backend/crates/sim-core/src/economy/tests/macro_flow.rs backend/crates/sim-core/src/economy/tests/flow_shipments.rs
git commit -m "feat(economy): thread OrderBook + NextOrderId through macro-flow atomic boundary

S1 plumbing — establishes the full clone-validate-apply topology S3's residual
drain will mutate: a top-level next_orders boundary clone + a per-edge scratch_orders
fold (on Ok) + a NextOrderId snapshot/commit-writeback. S1 mutates neither; both
round-trip unchanged. NO drain logic, NO gate change — every existing test stays
byte-identical; macro_flow_replays_across_restart proves the clone round-trips across
save/restore. New guard test asserts a populated OrderBook + counter are untouched
while the dormant flow still fires.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Full Rust gate (fmt-check + clippy + workspace tests)

**Files:** none (verification only). S1 touches no frontend / wire / render — the Rust gate is sufficient (no typecheck/vitest/e2e needed).

- [ ] **Step 1: fmt-check**

Run: `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml -- --check`
Expected: no diff (exit 0). If it fails, run `cargo fmt --manifest-path backend/Cargo.toml`, re-stage, amend the relevant commit.

- [ ] **Step 2: clippy (workspace, deny warnings) — run in background, poll**

Run (background): `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings`
Expected: no warnings/errors. Common S1 risk: an "unused variable" on a threaded param — if seen, the param is not actually used in the boundary/commit (re-check Steps 4/6).

- [ ] **Step 3: sim-server tests (the slower crate the workspace gate needs — run in background, poll)**

Run (background): `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server`
Expected: PASS. `run_macro_flow_system` is exercised through the runtime; the added `ResMut<OrderBook>`/`ResMut<NextOrderId>` must not introduce a schedule-ambiguity panic. **Wait for the command to COMPLETE before declaring green** — do not judge on partial output.

- [ ] **Step 4: Confirm the whole gate is green**

All three steps green ⇒ S1 is complete and ready for the next slice (S2 — the pure drain primitive). Do NOT push/PR yet unless the user asks; S1–S3 are a merge train (S1+S2 land dark, S3 activates), so coordinate the PR strategy with the user.
