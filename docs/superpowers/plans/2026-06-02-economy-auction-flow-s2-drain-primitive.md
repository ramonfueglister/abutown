# Economy Auction↔Flow Coupling — S2 Drain Primitive Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the two **pure** drain primitives `drain_residual_ask` and `drain_residual_bid` that release a residual order's lock (locked→available), shrink the order, and remove it at `qty_remaining == 0` — operating **only on passed-in clones**, conservation-tested in isolation. NOT wired into `run_macro_flow_at_tick` (that is S3, where a same-atomic-step consumer exists). No schedule interaction, no flag.

**Architecture:** A residual `Ask` holds `goods_locked_remaining` in the owner's *locked* inventory; a residual `Bid` holds `cash_locked_remaining` in *locked* cash. `settle_flow` consumes from *available*, so the flow can only move a residual after the lock is released. These primitives perform exactly that release + the order-row bookkeeping, mirroring `expire_orders_at_tick`'s release discipline (`orders.rs`) but **partially** (`release_cash`/`release_goods` check only `locked >= amount`, so they are partial-capable). The ask side is a 1:1 quantity release (`goods_locked_remaining == qty_remaining` preserved exactly). The bid side releases the **field-difference** — `released = cash_locked_remaining − checked_order_value(max_price, new_qty)` — never a recomputed per-`q` product; at `new_qty == 0` this releases the full locked field (`checked_order_value(_, 0) == 0`), disposing any floor-drift remainder so no cash is orphaned in `locked` (spec §5.1, resolves CRITICAL #1). Both return `bool` (was the order fully drained + removed) and reject invalid input (`q <= 0`, `q > qty_remaining`, missing id) with `EconomyError::InvalidOrder`. They never use `debit_locked`/`debit_locked_goods` (the auction *fill* path — would conflate matched-exit with residual-exit and let a later `expire_orders` double-release; spec §2.4).

**Tech Stack:** Rust, crate-internal unit tests under `backend/crates/sim-core/src/economy/tests/`. **Cargo MUST run via `scripts/cargo-serial.sh` with the isolated env** (`TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target`). Run the scoped `-p sim-core` command; never a broad `--workspace --all-targets` during iteration; never RUN a bench. `cargo fmt --manifest-path backend/Cargo.toml --all` before every commit (the bare `cargo-serial.sh fmt` form printed help in S1 — use the explicit `cargo fmt ... --all` form).

---

### Task 1: `drain_residual_ask` (1:1 goods release) + its tests

**Files:**
- Modify: `backend/crates/sim-core/src/economy/orders.rs` (add the fn after `expire_orders_at_tick`)
- Create: `backend/crates/sim-core/src/economy/tests/drain.rs`
- Modify: `backend/crates/sim-core/src/economy/tests/mod.rs` (register the module)

- [ ] **Step 1: Register the new test module + write the failing ask tests**

In `tests/mod.rs`, add `mod drain;` immediately after `mod determinism;`:

```rust
mod determinism;
mod drain;
mod expiry;
```

Create `backend/crates/sim-core/src/economy/tests/drain.rs`:

```rust
//! S2 drain primitives: pure release+shrink+remove on passed-in clones. Conservation
//! is asserted in isolation (total_money/total_good byte-equal; the field-difference
//! bid drain leaves zero orphaned cash in `locked`).

use crate::economy::orders::{drain_residual_ask, drain_residual_bid};
use crate::economy::{
    Ask, Bid, EconomicActorId, EconomyError, GoodId, InventoryBook, MarketId, Money, OrderBook,
    OrderId, Quantity, checked_order_value,
};
use crate::economy::AccountBook;

const GOOD: GoodId = GoodId(1);

fn ask_book(owner: EconomicActorId, qty: i64, locked: i64) -> OrderBook {
    let mut orders = OrderBook::default();
    orders.asks.insert(
        OrderId(1),
        Ask {
            id: OrderId(1),
            owner,
            market: MarketId(10),
            good: GOOD,
            qty_remaining: Quantity(qty),
            min_price: Money(500),
            goods_locked_remaining: Quantity(locked),
            created_tick: 0,
            expires_tick: 100,
        },
    );
    orders
}

#[test]
fn drain_residual_ask_partial_releases_and_shrinks() {
    let owner = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(owner, GOOD, Quantity(100)).unwrap();
    inv.lock_goods(owner, GOOD, Quantity(5)).unwrap(); // available 95, locked 5
    let mut orders = ask_book(owner, 5, 5);
    let total_before = inv.total_good(GOOD).unwrap();

    let removed = drain_residual_ask(&mut orders, &mut inv, OrderId(1), Quantity(2)).unwrap();

    assert!(!removed, "partial drain keeps the order");
    let ask = &orders.asks[&OrderId(1)];
    assert_eq!(ask.qty_remaining, Quantity(3));
    assert_eq!(ask.goods_locked_remaining, Quantity(3)); // 1:1 preserved
    let bal = inv.balance(owner, GOOD);
    assert_eq!(bal.locked, Quantity(3)); // 5 - 2
    assert_eq!(bal.available, Quantity(97)); // 95 + 2
    assert_eq!(inv.total_good(GOOD).unwrap(), total_before, "goods conserved");
}

#[test]
fn drain_residual_ask_full_removes_row_no_orphan() {
    let owner = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(owner, GOOD, Quantity(100)).unwrap();
    inv.lock_goods(owner, GOOD, Quantity(5)).unwrap();
    let mut orders = ask_book(owner, 5, 5);
    let total_before = inv.total_good(GOOD).unwrap();

    let removed = drain_residual_ask(&mut orders, &mut inv, OrderId(1), Quantity(5)).unwrap();

    assert!(removed, "full drain removes the order");
    assert!(orders.asks.is_empty(), "row removed at zero");
    let bal = inv.balance(owner, GOOD);
    assert_eq!(bal.locked, Quantity(0), "no orphaned lock");
    assert_eq!(bal.available, Quantity(100));
    assert_eq!(inv.total_good(GOOD).unwrap(), total_before);
}

#[test]
fn drain_residual_ask_rejects_invalid_q() {
    let owner = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(owner, GOOD, Quantity(100)).unwrap();
    inv.lock_goods(owner, GOOD, Quantity(5)).unwrap();
    let mut orders = ask_book(owner, 5, 5);

    assert_eq!(
        drain_residual_ask(&mut orders, &mut inv, OrderId(1), Quantity(6)),
        Err(EconomyError::InvalidOrder),
        "q greater than qty_remaining"
    );
    assert_eq!(
        drain_residual_ask(&mut orders, &mut inv, OrderId(1), Quantity(0)),
        Err(EconomyError::InvalidOrder),
        "non-positive q"
    );
    assert_eq!(
        drain_residual_ask(&mut orders, &mut inv, OrderId(999), Quantity(1)),
        Err(EconomyError::InvalidOrder),
        "missing order id"
    );
    // The failed calls must not have mutated anything.
    assert_eq!(orders.asks[&OrderId(1)].qty_remaining, Quantity(5));
    assert_eq!(inv.balance(owner, GOOD).locked, Quantity(5));
}
```

- [ ] **Step 2: Run the ask tests to verify they fail**

Run: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core drain_residual_ask`
Expected: FAIL — compile error `cannot find function 'drain_residual_ask' in module 'crate::economy::orders'`.

- [ ] **Step 3: Implement `drain_residual_ask`**

In `orders.rs`, add immediately after `expire_orders_at_tick` (it uses the already-imported `OrderBook`, `InventoryBook`, `OrderId`, `Quantity`, `EconomyError`):

```rust
/// S2 drain primitive (ask side): release `q` units of a residual ask's goods lock
/// (locked→available, 1:1), shrink the order, and remove it at `qty_remaining == 0`.
/// PURE — mutates only the passed-in (scratch) clones; performs NO settle and is NOT
/// wired into any schedule (that is S3). Returns `true` if the order was fully drained
/// and removed. Rejects `q <= 0`, `q > qty_remaining`, or a missing id with
/// `InvalidOrder`. Never uses `debit_locked_goods` (the auction fill path) — that would
/// conflate matched-exit with residual-exit and let `expire_orders` double-release.
pub fn drain_residual_ask(
    orders: &mut OrderBook,
    inventory: &mut InventoryBook,
    ask_id: OrderId,
    q: Quantity,
) -> Result<bool, EconomyError> {
    let ask = orders.asks.get_mut(&ask_id).ok_or(EconomyError::InvalidOrder)?;
    if q.0 <= 0 || q > ask.qty_remaining {
        return Err(EconomyError::InvalidOrder);
    }
    inventory.release_goods(ask.owner, ask.good, q)?;
    ask.qty_remaining = ask.qty_remaining.checked_sub(q)?;
    ask.goods_locked_remaining = ask.goods_locked_remaining.checked_sub(q)?;
    let removed = ask.qty_remaining.0 == 0;
    if removed {
        orders.asks.remove(&ask_id);
    }
    Ok(removed)
}
```

- [ ] **Step 4: Run the ask tests to verify they pass**

Run: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core drain_residual_ask`
Expected: PASS (3 passed).

- [ ] **Step 5: Format + commit**

```bash
cargo fmt --manifest-path backend/Cargo.toml --all
git add backend/crates/sim-core/src/economy/orders.rs backend/crates/sim-core/src/economy/tests/drain.rs backend/crates/sim-core/src/economy/tests/mod.rs
git commit -m "feat(economy): add drain_residual_ask pure primitive (S2)

Releases q units of a residual ask's goods lock (locked->available, 1:1), shrinks
the order, removes it at zero. Pure — operates on passed-in clones, no settle, no
schedule wiring (that is S3). Conservation-tested in isolation; InvalidOrder guards.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `drain_residual_bid` (field-difference cash release) + its tests

**Files:**
- Modify: `backend/crates/sim-core/src/economy/orders.rs` (add the fn after `drain_residual_ask`)
- Modify: `backend/crates/sim-core/src/economy/tests/drain.rs` (add bid tests)

- [ ] **Step 1: Write the failing bid tests**

Append to `tests/drain.rs`. **Note:** the fixtures use a `cash_locked_remaining` that EQUALS `checked_order_value(max_price, qty)` exactly (value-consistent — required so the field-difference math is exercised honestly; do NOT use an arbitrary lock):

```rust
fn bid_book(owner: EconomicActorId, qty: i64, max_price: Money, locked: Money) -> OrderBook {
    let mut orders = OrderBook::default();
    orders.bids.insert(
        OrderId(1),
        Bid {
            id: OrderId(1),
            owner,
            market: MarketId(10),
            good: GOOD,
            qty_remaining: Quantity(qty),
            max_price,
            cash_locked_remaining: locked,
            created_tick: 0,
            expires_tick: 100,
        },
    );
    orders
}

#[test]
fn drain_residual_bid_partial_releases_field_difference() {
    // max_price 1000, qty 5 -> lock = 1000*5/1000 = Money(5).
    let owner = EconomicActorId(1);
    let max_price = Money(1_000);
    let lock = checked_order_value(max_price, Quantity(5)).unwrap();
    assert_eq!(lock, Money(5));
    let mut acc = AccountBook::default();
    acc.deposit(owner, Money(100)).unwrap();
    acc.lock_cash(owner, lock).unwrap(); // available 95, locked 5
    let mut orders = bid_book(owner, 5, max_price, lock);
    let total_before = acc.total_money().unwrap();

    let removed = drain_residual_bid(&mut orders, &mut acc, OrderId(1), Quantity(2)).unwrap();

    assert!(!removed);
    let bid = &orders.bids[&OrderId(1)];
    assert_eq!(bid.qty_remaining, Quantity(3));
    // new lock = checked_order_value(1000, 3) = Money(3); released = 5 - 3 = 2.
    assert_eq!(bid.cash_locked_remaining, Money(3));
    let a = acc.account(owner);
    assert_eq!(a.locked, Money(3));
    assert_eq!(a.available, Money(97)); // 95 + 2
    assert_eq!(acc.total_money().unwrap(), total_before, "money conserved");
}

#[test]
fn drain_residual_bid_fractional_full_drain_zero_orphan() {
    // FRACTIONAL price: max_price 1500, qty 3 -> lock = 1500*3/1000 = floor(4.5) = Money(4).
    // A per-q recompute (value(1500,1)*3 = 1*3 = 3) would strand Money(1) in `locked`.
    // The field-difference rule (release the FULL field at new_qty==0) leaves zero orphan.
    let owner = EconomicActorId(1);
    let max_price = Money(1_500);
    let lock = checked_order_value(max_price, Quantity(3)).unwrap();
    assert_eq!(lock, Money(4));
    let mut acc = AccountBook::default();
    acc.deposit(owner, Money(100)).unwrap();
    acc.lock_cash(owner, lock).unwrap(); // available 96, locked 4
    let mut orders = bid_book(owner, 3, max_price, lock);
    let total_before = acc.total_money().unwrap();

    let removed = drain_residual_bid(&mut orders, &mut acc, OrderId(1), Quantity(3)).unwrap();

    assert!(removed, "full drain removes the order");
    assert!(orders.bids.is_empty(), "row removed at zero");
    let a = acc.account(owner);
    assert_eq!(a.locked, Money(0), "ZERO orphan — full field released, no floor-drift residue");
    assert_eq!(a.available, Money(100), "all locked cash returned to available");
    assert_eq!(acc.total_money().unwrap(), total_before, "money conserved");
}

#[test]
fn drain_residual_bid_rejects_invalid_q() {
    let owner = EconomicActorId(1);
    let max_price = Money(1_000);
    let lock = checked_order_value(max_price, Quantity(5)).unwrap();
    let mut acc = AccountBook::default();
    acc.deposit(owner, Money(100)).unwrap();
    acc.lock_cash(owner, lock).unwrap();
    let mut orders = bid_book(owner, 5, max_price, lock);

    assert_eq!(
        drain_residual_bid(&mut orders, &mut acc, OrderId(1), Quantity(6)),
        Err(EconomyError::InvalidOrder)
    );
    assert_eq!(
        drain_residual_bid(&mut orders, &mut acc, OrderId(1), Quantity(0)),
        Err(EconomyError::InvalidOrder)
    );
    assert_eq!(
        drain_residual_bid(&mut orders, &mut acc, OrderId(999), Quantity(1)),
        Err(EconomyError::InvalidOrder)
    );
    assert_eq!(orders.bids[&OrderId(1)].qty_remaining, Quantity(5));
    assert_eq!(acc.account(owner).locked, lock);
}
```

- [ ] **Step 2: Run the bid tests to verify they fail**

Run: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core drain_residual_bid`
Expected: FAIL — `cannot find function 'drain_residual_bid'`.

- [ ] **Step 3: Implement `drain_residual_bid`**

In `orders.rs`, add immediately after `drain_residual_ask` (uses the already-imported `AccountBook`, `checked_order_value`):

```rust
/// S2 drain primitive (bid side): release a residual bid's cash lock by the
/// FIELD-DIFFERENCE — never a recomputed per-`q` product — shrink the order, and
/// remove it at `qty_remaining == 0`. `released = cash_locked_remaining −
/// checked_order_value(max_price, new_qty)`; at `new_qty == 0` this is the full locked
/// field (`checked_order_value(_, 0) == 0`), disposing any floor-drift remainder so no
/// cash is orphaned in `locked` (spec §5.1, resolves CRITICAL #1). PURE — mutates only
/// the passed-in clones; NO settle, NOT wired (S3 adds the settle+refund wrapper).
/// Returns `true` if fully drained + removed. Rejects bad input with `InvalidOrder`.
pub fn drain_residual_bid(
    orders: &mut OrderBook,
    accounts: &mut AccountBook,
    bid_id: OrderId,
    q: Quantity,
) -> Result<bool, EconomyError> {
    let bid = orders.bids.get_mut(&bid_id).ok_or(EconomyError::InvalidOrder)?;
    if q.0 <= 0 || q > bid.qty_remaining {
        return Err(EconomyError::InvalidOrder);
    }
    let new_qty = bid.qty_remaining.checked_sub(q)?;
    let target_lock = checked_order_value(bid.max_price, new_qty)?;
    let released = bid.cash_locked_remaining.checked_sub(target_lock)?;
    accounts.release_cash(bid.owner, released)?;
    bid.cash_locked_remaining = target_lock;
    bid.qty_remaining = new_qty;
    let removed = new_qty.0 == 0;
    if removed {
        orders.bids.remove(&bid_id);
    }
    Ok(removed)
}
```

- [ ] **Step 4: Run the bid tests to verify they pass**

Run: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core drain_residual_bid`
Expected: PASS (3 passed).

- [ ] **Step 5: Run the whole drain module + confirm no existing test regressed**

Run: `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core`
Expected: PASS — the 6 new drain tests plus all pre-existing tests (S2 adds pure functions, wires nothing, so every existing test is byte-identical). **Wait for the command to COMPLETE before judging green.**

- [ ] **Step 6: Format + commit**

```bash
cargo fmt --manifest-path backend/Cargo.toml --all
git add backend/crates/sim-core/src/economy/orders.rs backend/crates/sim-core/src/economy/tests/drain.rs
git commit -m "feat(economy): add drain_residual_bid pure primitive (S2)

Releases a residual bid's cash lock by the FIELD-DIFFERENCE (released =
cash_locked_remaining - checked_order_value(max_price, new_qty)); at new_qty==0
releases the full field so no floor-drift cash is orphaned in locked (spec §5.1).
Pure — operates on passed-in clones, no settle, no wiring (S3 adds the settle+refund
wrapper). Fractional-price zero-orphan test + conservation + InvalidOrder guards.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Full Rust gate (fmt-check + clippy + workspace tests)

**Files:** none (verification only). S2 adds pure functions, touches no frontend / wire / render / schedule.

- [ ] **Step 1: fmt-check**

Run: `cargo fmt --manifest-path backend/Cargo.toml --all -- --check`
Expected: no diff (exit 0). (The bare `cargo-serial.sh fmt` form prints help — use this explicit form.)

- [ ] **Step 2: clippy (workspace, deny warnings) — background, poll, wait for completion**

Run (background): `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings`
Expected: clean. `--all-targets` compiles (does NOT run) the criterion benches too — S2 adds no call sites there, so no bench breakage expected, but the flag is what catches it if wrong.

- [ ] **Step 3: sim-server tests — background, poll, wait for completion**

Run (background): `TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server`
Expected: PASS (unchanged — S2 adds no system, no runtime path). Wait for COMPLETE before judging.

- [ ] **Step 4: Confirm the whole gate is green**

All three green ⇒ S2 is complete. S1+S2 now both land dark (pure primitives + threaded topology, nothing activated). **S3 is next: the activating slice** — it re-grains the dormant gate into the two-source bucket builder, wires these drains into the per-edge scratch boundary with the settle+refund wrapper, flips `drain_active_residual` to TRUE, and resolves the price-authority + #71 interactions. Coordinate the PR strategy with the user (S1+S2+S3 likely ship together since S3 activates them).
