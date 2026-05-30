# Economy Rationing v0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace strict time-priority allocation at the marginal price tier of the call auction with deterministic integer pro-rata (largest-remainder), keeping clearing quantity, marginal prices, uniform settlement price, and conservation exactly as today — entirely inside `build_clearing_plan`, behind the unchanged `ClearingPlan` / `clear_market_good` interface.

**Architecture:** `build_clearing_plan` keeps its unchanged price-time greedy walk (Phase 1) to compute total matched quantity `total_q` + marginal bid/ask prices + settlement. New Phase 2 (`allocate_side`) fills infra-marginal orders fully and pro-rata-rations the marginal tier via `prorata_distribute`. New Phase 3 (`pair_fills`) pairs the per-side allocations into `Fill`s with a north-west-corner walk.

**Tech Stack:** Rust, `sim-core` crate, `economy/auction.rs`. Cargo via `scripts/cargo-serial.sh`, `CARGO_TARGET_DIR=/tmp/abutown-rationing-target`.

**Confirmed grounding (use directly):**
- `Money(pub i64)`, `Quantity(pub i64)`, `Quantity::ZERO`, `OrderId(pub u64)`, `MarketId(pub u32)`, `GoodId`, `EconomicActorId(pub u64)`.
- `Bid { id, owner, market, good, qty_remaining, max_price, cash_locked_remaining, created_tick, expires_tick }`; `Ask { id, owner, market, good, qty_remaining, min_price, goods_locked_remaining, created_tick, expires_tick }`.
- `Fill { bid: OrderId, ask: OrderId, qty: Quantity }`; `ClearingPlan { key, fills: Vec<Fill>, settlement_price: Option<Money>, unmet_demand: Quantity, unsold_supply: Quantity }`.
- `settlement_price(last, marginal_bid, marginal_ask) -> Money` — KEEP unchanged.
- `build_clearing_plan(key: MarketGoodKey, bids: &[Bid], asks: &[Ask], last_settlement_price: Money) -> Result<ClearingPlan, EconomyError>` — signature UNCHANGED; only the body changes.
- `EconomyError::{Overflow, InvalidOrder}` exist.
- Existing test helpers in `tests/auction.rs`: `bid(id, max_price, qty, created_tick)`, `ask(id, min_price, qty, created_tick)` (owners `10+id` / `20+id`, market `MarketId(1)`, good `GOOD_FOOD`, `expires_tick: 100`).
- All existing auction + conservation tests are single-bid/single-ask (non-contested) ⇒ pro-rata reduces to one fill ⇒ they MUST stay green.

---

### Task 1: `prorata_distribute` integer apportionment helper

**Files:**
- Modify: `backend/crates/sim-core/src/economy/auction.rs` (add the helper)
- Create: `backend/crates/sim-core/src/economy/tests/rationing.rs`
- Modify: `backend/crates/sim-core/src/economy/tests/mod.rs` (add `mod rationing;`)

- [ ] **Step 1: Write the failing tests** — create `tests/rationing.rs`:

```rust
use crate::economy::prorata_distribute;

#[test]
fn prorata_exact_division() {
    assert_eq!(prorata_distribute(&[10, 10], 10), vec![5, 5]);
}

#[test]
fn prorata_proportional() {
    assert_eq!(prorata_distribute(&[30, 10], 20), vec![15, 5]);
}

#[test]
fn prorata_leftover_to_largest_remainder_then_index() {
    // total 2 across three equal weights: floors are [0,0,0], 2 leftover units go
    // to the two largest remainders; all remainders equal -> lowest indices win.
    assert_eq!(prorata_distribute(&[1, 1, 1], 2), vec![1, 1, 0]);
}

#[test]
fn prorata_odd_split_is_deterministic() {
    // 1001 across two equal weights -> 501 / 500 (extra unit to index 0).
    assert_eq!(prorata_distribute(&[1000, 1000], 1001), vec![501, 500]);
}

#[test]
fn prorata_total_at_or_above_sum_returns_weights() {
    assert_eq!(prorata_distribute(&[3, 7], 10), vec![3, 7]);
    assert_eq!(prorata_distribute(&[3, 7], 100), vec![3, 7]);
}

#[test]
fn prorata_zero_total_is_zeros() {
    assert_eq!(prorata_distribute(&[5, 5], 0), vec![0, 0]);
}

#[test]
fn prorata_never_exceeds_a_weight() {
    let weights = [2, 2, 2];
    for total in 0..=6 {
        let out = prorata_distribute(&weights, total);
        assert_eq!(out.iter().sum::<i64>(), total.min(6));
        for (o, w) in out.iter().zip(weights.iter()) {
            assert!(*o <= *w, "alloc {o} exceeded weight {w} at total {total}");
        }
    }
}
```

- [ ] **Step 2: Add `mod rationing;`** to `tests/mod.rs` — insert the line between `mod production;` and `mod systems;`:

```rust
mod production;
mod rationing;
mod systems;
```

- [ ] **Step 3: Run to verify it fails**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core prorata`
Expected: FAIL to compile — `prorata_distribute` not found.

- [ ] **Step 4: Implement** — add to `auction.rs` (top-level, after the `settlement_price` fn):

```rust
/// Largest-remainder (Hamilton) integer apportionment. Distributes `total` units
/// across `weights` proportionally to each weight; leftover units from flooring
/// are assigned one-by-one to the largest fractional remainders, ties broken by
/// ascending index (callers pass weights in a deterministic order). Returns a Vec
/// the same length as `weights`. When `total <= sum(weights)` each output is
/// `<= its weight`; when `total >= sum(weights)` each output equals its weight.
/// All inputs are treated as non-negative.
pub fn prorata_distribute(weights: &[i64], total: i64) -> Vec<i64> {
    let n = weights.len();
    let sum: i128 = weights.iter().map(|w| (*w).max(0) as i128).sum();
    if sum <= 0 || total <= 0 {
        return vec![0; n];
    }
    let total = (total as i128).min(sum);
    let mut alloc = vec![0i64; n];
    let mut remainders: Vec<(i128, usize)> = Vec::with_capacity(n);
    let mut distributed: i128 = 0;
    for (idx, &w) in weights.iter().enumerate() {
        let w = w.max(0) as i128;
        let num = total * w;
        let base = num / sum;
        alloc[idx] = base as i64;
        distributed += base;
        remainders.push((num % sum, idx));
    }
    let mut leftover = (total - distributed) as usize;
    // Largest remainder first; ties by ascending index for determinism.
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for &(_, idx) in &remainders {
        if leftover == 0 {
            break;
        }
        alloc[idx] += 1;
        leftover -= 1;
    }
    alloc
}
```

- [ ] **Step 5: Run to verify it passes**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core prorata`
Expected: PASS — all 7 prorata tests.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/economy/auction.rs \
        backend/crates/sim-core/src/economy/tests/rationing.rs \
        backend/crates/sim-core/src/economy/tests/mod.rs
git commit -m "feat(economy): largest-remainder integer apportionment helper"
```

---

### Task 2: rewrite `build_clearing_plan` to ration the marginal tier pro-rata

**Files:**
- Modify: `backend/crates/sim-core/src/economy/auction.rs` (rewrite `build_clearing_plan`, add `allocate_side` + `pair_fills`)
- Modify: `backend/crates/sim-core/src/economy/tests/rationing.rs` (clearing tests)

- [ ] **Step 1: Write the failing tests** — append to `tests/rationing.rs`. (Reuse the `bid`/`ask` helpers from `tests/auction.rs` by re-declaring local copies, since they are module-private — copy the two helper fns to the top of `rationing.rs`.)

```rust
use crate::economy::{
    Ask, Bid, EconomicActorId, GOOD_FOOD, MarketGoodKey, MarketId, Money, OrderId, Quantity,
    build_clearing_plan,
};

fn rbid(id: u64, max_price: Money, qty: Quantity, created_tick: u64) -> Bid {
    Bid {
        id: OrderId(id),
        owner: EconomicActorId(10 + id),
        market: MarketId(1),
        good: GOOD_FOOD,
        qty_remaining: qty,
        max_price,
        cash_locked_remaining: Money(max_price.0 * qty.0 / 1_000),
        created_tick,
        expires_tick: 100,
    }
}

fn rask(id: u64, min_price: Money, qty: Quantity, created_tick: u64) -> Ask {
    Ask {
        id: OrderId(id),
        owner: EconomicActorId(20 + id),
        market: MarketId(1),
        good: GOOD_FOOD,
        qty_remaining: qty,
        min_price,
        goods_locked_remaining: qty,
        created_tick,
        expires_tick: 100,
    }
}

fn key() -> MarketGoodKey {
    MarketGoodKey { market: MarketId(1), good: GOOD_FOOD }
}

// Sum of fill qty attributed to a given bid / ask id.
fn filled_for_bid(plan: &crate::economy::ClearingPlan, id: u64) -> i64 {
    plan.fills.iter().filter(|f| f.bid == OrderId(id)).map(|f| f.qty.0).sum()
}
fn filled_for_ask(plan: &crate::economy::ClearingPlan, id: u64) -> i64 {
    plan.fills.iter().filter(|f| f.ask == OrderId(id)).map(|f| f.qty.0).sum()
}

#[test]
fn marginal_tier_is_rationed_pro_rata() {
    // Two equal-price bids (1000 each @ 1000) compete for one ask of 1000 @ 1000.
    // Time-priority would give bid 1 all 1000; pro-rata gives each 500.
    let plan = build_clearing_plan(
        key(),
        &[
            rbid(1, Money(1_000), Quantity(1_000), 1),
            rbid(2, Money(1_000), Quantity(1_000), 2),
        ],
        &[rask(3, Money(1_000), Quantity(1_000), 1)],
        Money(1_000),
    )
    .unwrap();
    assert_eq!(filled_for_bid(&plan, 1), 500);
    assert_eq!(filled_for_bid(&plan, 2), 500);
    assert_eq!(filled_for_ask(&plan, 3), 1_000);
    assert_eq!(plan.settlement_price, Some(Money(1_000)));
}

#[test]
fn marginal_tier_pro_rata_is_size_weighted() {
    // Bids 1500 and 500 (both @ 1000) vs one ask of 1000 @ 1000 -> 750 / 250.
    let plan = build_clearing_plan(
        key(),
        &[
            rbid(1, Money(1_000), Quantity(1_500), 1),
            rbid(2, Money(1_000), Quantity(500), 2),
        ],
        &[rask(3, Money(1_000), Quantity(1_000), 1)],
        Money(1_000),
    )
    .unwrap();
    assert_eq!(filled_for_bid(&plan, 1), 750);
    assert_eq!(filled_for_bid(&plan, 2), 250);
}

#[test]
fn infra_marginal_bids_keep_price_priority() {
    // A higher bid (1200) fills fully before the marginal tier (1000) is rationed.
    // ask supply 1000 total: bid1@1200 x600 fills fully (600), remaining 400 split
    // pro-rata across the two 1000-priced bids (500 each -> 200 each).
    let plan = build_clearing_plan(
        key(),
        &[
            rbid(1, Money(1_200), Quantity(600), 1),
            rbid(2, Money(1_000), Quantity(500), 2),
            rbid(3, Money(1_000), Quantity(500), 3),
        ],
        &[rask(4, Money(1_000), Quantity(1_000), 1)],
        Money(1_100),
    )
    .unwrap();
    assert_eq!(filled_for_bid(&plan, 1), 600, "infra-marginal bid fully filled");
    assert_eq!(filled_for_bid(&plan, 2), 200);
    assert_eq!(filled_for_bid(&plan, 3), 200);
    assert_eq!(filled_for_ask(&plan, 4), 1_000);
}

#[test]
fn odd_contested_quantity_splits_deterministically() {
    // ask 1001 across two equal 1000-bids -> 501 / 500 (extra to lower index after
    // sort: bid 1, created_tick 1).
    let plan = build_clearing_plan(
        key(),
        &[
            rbid(1, Money(1_000), Quantity(1_000), 1),
            rbid(2, Money(1_000), Quantity(1_000), 2),
        ],
        &[rask(3, Money(1_000), Quantity(1_001), 1)],
        Money(1_000),
    )
    .unwrap();
    assert_eq!(filled_for_bid(&plan, 1) + filled_for_bid(&plan, 2), 1_001);
    assert_eq!(filled_for_bid(&plan, 1), 501);
    assert_eq!(filled_for_bid(&plan, 2), 500);
}

#[test]
fn clearing_plan_is_deterministic() {
    let mk = || {
        build_clearing_plan(
            key(),
            &[
                rbid(1, Money(1_000), Quantity(700), 1),
                rbid(2, Money(1_000), Quantity(300), 2),
            ],
            &[rask(3, Money(1_000), Quantity(900), 1)],
            Money(1_000),
        )
        .unwrap()
    };
    assert_eq!(mk(), mk());
}

#[test]
fn fills_balance_quantity_per_side() {
    let plan = build_clearing_plan(
        key(),
        &[
            rbid(1, Money(1_000), Quantity(1_000), 1),
            rbid(2, Money(1_000), Quantity(1_000), 2),
        ],
        &[
            rask(3, Money(1_000), Quantity(700), 1),
            rask(4, Money(1_000), Quantity(800), 2),
        ],
        Money(1_000),
    )
    .unwrap();
    let total: i64 = plan.fills.iter().map(|f| f.qty.0).sum();
    assert_eq!(total, 1_500, "matched quantity = min(2000 demand, 1500 supply)");
    assert_eq!(filled_for_ask(&plan, 3) + filled_for_ask(&plan, 4), 1_500);
    assert_eq!(filled_for_bid(&plan, 1) + filled_for_bid(&plan, 2), 1_500);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core rationing`
Expected: FAIL — the current greedy `build_clearing_plan` gives bid 1 the full 1000 (time priority), so `marginal_tier_is_rationed_pro_rata` and friends fail.

- [ ] **Step 3: Rewrite `build_clearing_plan`** in `auction.rs`. Replace the entire existing `build_clearing_plan` function body (keep `settlement_price` above it untouched) with:

```rust
pub fn build_clearing_plan(
    key: MarketGoodKey,
    bids: &[Bid],
    asks: &[Ask],
    last_settlement_price: Money,
) -> Result<ClearingPlan, EconomyError> {
    let mut sorted_bids = bids.to_vec();
    sorted_bids.sort_by(|a, b| {
        b.max_price
            .cmp(&a.max_price)
            .then(a.created_tick.cmp(&b.created_tick))
            .then(a.id.cmp(&b.id))
    });
    let mut sorted_asks = asks.to_vec();
    sorted_asks.sort_by(|a, b| {
        a.min_price
            .cmp(&b.min_price)
            .then(a.created_tick.cmp(&b.created_tick))
            .then(a.id.cmp(&b.id))
    });

    // Phase 1: clearing quantity + marginal prices (unchanged price-time greedy).
    let mut i = 0;
    let mut j = 0;
    let mut total_q: i64 = 0;
    let mut marginal_bid: Option<Money> = None;
    let mut marginal_ask: Option<Money> = None;
    {
        let mut bid_rem: Vec<i64> = sorted_bids.iter().map(|b| b.qty_remaining.0).collect();
        let mut ask_rem: Vec<i64> = sorted_asks.iter().map(|a| a.qty_remaining.0).collect();
        while i < sorted_bids.len() && j < sorted_asks.len() {
            if sorted_bids[i].max_price < sorted_asks[j].min_price {
                break;
            }
            let q = bid_rem[i].min(ask_rem[j]);
            if q <= 0 {
                return Err(EconomyError::InvalidOrder);
            }
            total_q = total_q.checked_add(q).ok_or(EconomyError::Overflow)?;
            marginal_bid = Some(sorted_bids[i].max_price);
            marginal_ask = Some(sorted_asks[j].min_price);
            bid_rem[i] -= q;
            ask_rem[j] -= q;
            if bid_rem[i] == 0 {
                i += 1;
            }
            if ask_rem[j] == 0 {
                j += 1;
            }
        }
    }

    let total_bid_qty: i64 = sorted_bids.iter().map(|b| b.qty_remaining.0).sum();
    let total_ask_qty: i64 = sorted_asks.iter().map(|a| a.qty_remaining.0).sum();

    let (Some(m_bid), Some(m_ask)) = (marginal_bid, marginal_ask) else {
        return Ok(ClearingPlan {
            key,
            fills: Vec::new(),
            settlement_price: None,
            unmet_demand: Quantity(total_bid_qty),
            unsold_supply: Quantity(total_ask_qty),
        });
    };
    let settlement = settlement_price(last_settlement_price, m_bid, m_ask);

    // Phase 2: per-side allocation (infra-marginal full; marginal tier pro-rata).
    let bid_prices: Vec<i64> = sorted_bids.iter().map(|b| b.max_price.0).collect();
    let bid_qtys: Vec<i64> = sorted_bids.iter().map(|b| b.qty_remaining.0).collect();
    let bid_alloc = allocate_side(&bid_prices, &bid_qtys, m_bid.0, total_q, true);

    let ask_prices: Vec<i64> = sorted_asks.iter().map(|a| a.min_price.0).collect();
    let ask_qtys: Vec<i64> = sorted_asks.iter().map(|a| a.qty_remaining.0).collect();
    let ask_alloc = allocate_side(&ask_prices, &ask_qtys, m_ask.0, total_q, false);

    // Phase 3: pair allocations into fills (north-west corner).
    let fills = pair_fills(&sorted_bids, &bid_alloc, &sorted_asks, &ask_alloc);

    Ok(ClearingPlan {
        key,
        fills,
        settlement_price: Some(settlement),
        unmet_demand: Quantity(total_bid_qty - total_q),
        unsold_supply: Quantity(total_ask_qty - total_q),
    })
}

/// Allocate `total_q` units across one side. Orders strictly better than
/// `marginal` (higher for bids when `better_is_higher`, lower for asks) are filled
/// in full; orders priced exactly at `marginal` share the remainder pro-rata;
/// worse-priced orders get 0. Indexed parallel to `prices`/`qtys`.
fn allocate_side(
    prices: &[i64],
    qtys: &[i64],
    marginal: i64,
    total_q: i64,
    better_is_higher: bool,
) -> Vec<i64> {
    let n = prices.len();
    let mut alloc = vec![0i64; n];
    let mut infra_sum: i64 = 0;
    let mut marginal_idx: Vec<usize> = Vec::new();
    for idx in 0..n {
        let is_infra = if better_is_higher {
            prices[idx] > marginal
        } else {
            prices[idx] < marginal
        };
        if is_infra {
            alloc[idx] = qtys[idx];
            infra_sum += qtys[idx];
        } else if prices[idx] == marginal {
            marginal_idx.push(idx);
        }
    }
    let to_ration = (total_q - infra_sum).max(0);
    let weights: Vec<i64> = marginal_idx.iter().map(|&k| qtys[k]).collect();
    let shares = prorata_distribute(&weights, to_ration);
    for (s, &k) in shares.iter().zip(marginal_idx.iter()) {
        alloc[k] = *s;
    }
    alloc
}

/// Pair per-bid and per-ask allocations (both summing to the same total) into
/// fills via a north-west-corner walk over the already-sorted orders.
fn pair_fills(bids: &[Bid], bid_alloc: &[i64], asks: &[Ask], ask_alloc: &[i64]) -> Vec<Fill> {
    let mut fills = Vec::new();
    let mut brem = bid_alloc.to_vec();
    let mut arem = ask_alloc.to_vec();
    let mut bi = 0;
    let mut aj = 0;
    while bi < bids.len() && aj < asks.len() {
        if brem[bi] == 0 {
            bi += 1;
            continue;
        }
        if arem[aj] == 0 {
            aj += 1;
            continue;
        }
        let q = brem[bi].min(arem[aj]);
        fills.push(Fill {
            bid: bids[bi].id,
            ask: asks[aj].id,
            qty: Quantity(q),
        });
        brem[bi] -= q;
        arem[aj] -= q;
    }
    fills
}
```

(Note: the `use crate::economy::{...}` imports already present in `auction.rs` cover `Ask, Bid, EconomyError, MarketGoodKey, Money, OrderId, Quantity` and the `Fill`/`ClearingPlan` types are defined in this file. No new imports needed. If clippy flags `needless_range_loop` on `for idx in 0..n`, rewrite as `for (idx, &p) in prices.iter().enumerate()` and index `qtys[idx]`.)

- [ ] **Step 4: Run rationing + existing auction tests**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core auction rationing`
Expected: PASS — new rationing tests AND the existing `no_trade_without_price_overlap`, `trade_happens_with_price_overlap`, `settlement_price_is_within_bid_ask_bounds` (non-contested ⇒ single fill, unchanged).

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/auction.rs \
        backend/crates/sim-core/src/economy/tests/rationing.rs
git commit -m "feat(economy): pro-rata rationing at the marginal price tier"
```

---

### Task 3: conservation with rationing (integration via `clear_market_good`)

**Files:**
- Modify: `backend/crates/sim-core/src/economy/tests/rationing.rs` (conservation integration test)

- [ ] **Step 1: Write the failing/【green】 test** — append to `tests/rationing.rs`. This drives a contested clearing through `clear_market_good` and asserts conservation + that both marginal bids got a proportional share. (Mirror the conservation.rs setup: deposit cash to two buyers, goods to one seller, create two equal bids + one smaller ask, clear, assert totals conserved and each buyer received goods.)

```rust
use crate::economy::{
    AccountBook, DirtyMarketGoods, GOOD_FOOD as FOOD, InventoryBook, MarketGoodState, MarketGoods,
    NextOrderId, OrderBook, TradeLedger, clear_market_good, create_ask, create_bid,
};

#[test]
fn contested_clearing_conserves_and_rations() {
    let buyer_a = EconomicActorId(1);
    let buyer_b = EconomicActorId(2);
    let seller = EconomicActorId(3);
    let market = MarketId(1);
    let k = MarketGoodKey { market, good: FOOD };

    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut goods = MarketGoods::default();
    let mut state = MarketGoodState::new(k);
    state.last_settlement_price = Money(1_000);
    state.dirty = true;
    goods.0.insert(k, state);

    accounts.deposit(buyer_a, Money(10_000)).unwrap();
    accounts.deposit(buyer_b, Money(10_000)).unwrap();
    inventory.deposit(seller, FOOD, Quantity(1_000)).unwrap();

    // Two equal bids @1000 x1000 each, one ask @1000 x1000 -> 500/500 rationed.
    create_bid(&mut accounts, &mut orders, &mut ledger, &mut dirty, &mut next, 1,
        buyer_a, market, FOOD, Quantity(1_000), Money(1_000), 10).unwrap();
    create_bid(&mut accounts, &mut orders, &mut ledger, &mut dirty, &mut next, 1,
        buyer_b, market, FOOD, Quantity(1_000), Money(1_000), 10).unwrap();
    create_ask(&mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next, 1,
        seller, market, FOOD, Quantity(1_000), Money(1_000), 10).unwrap();

    let money_before = accounts.total_money().unwrap();
    let goods_before = inventory.total_good(FOOD).unwrap();

    clear_market_good(&mut accounts, &mut inventory, &mut orders, &mut ledger,
        &mut goods, k, 2).unwrap();

    assert_eq!(accounts.total_money().unwrap(), money_before, "money conserved");
    assert_eq!(inventory.total_good(FOOD).unwrap(), goods_before, "goods conserved");
    // Each buyer received a proportional 500 of FOOD.
    assert_eq!(inventory.balance(buyer_a, FOOD).available, Quantity(500));
    assert_eq!(inventory.balance(buyer_b, FOOD).available, Quantity(500));
    assert_eq!(inventory.balance(seller, FOOD).available, Quantity(0));
}
```

(If `create_bid`/`create_ask`/`MarketGoodState::new` argument orders differ from the conservation.rs usage, match the real signatures — they were confirmed identical to `tests/conservation.rs`. `inventory.balance(actor, good).available` returns a `Quantity`.)

- [ ] **Step 2: Run to verify**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core rationing conservation`
Expected: PASS — `contested_clearing_conserves_and_rations` plus the existing conservation tests (still green — they are non-contested).

- [ ] **Step 3: Run the full economy suite** (regression sweep)

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy`
Expected: PASS — all prior auction/conservation/pools/traders/production/transport/LOD tests unaffected.

- [ ] **Step 4: Commit**

```bash
git add backend/crates/sim-core/src/economy/tests/rationing.rs
git commit -m "test(economy): conservation under contested pro-rata clearing"
```

---

### Final gate (orchestrator runs; implementer reports readiness)

```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace --all-targets
```

All green. Implementer does NOT push or open a PR — report per-task RED→GREEN + commit SHAs, the `-p sim-core rationing` / `auction` / `economy` summaries, and clippy/fmt status.

## Self-review notes

- **Spec coverage:** `prorata_distribute` (largest-remainder) ✓, Phase-1 unchanged greedy for Q/margins/settlement ✓, `allocate_side` infra-full + marginal pro-rata ✓, `pair_fills` NWC ✓, unmet/unsold = total − Q ✓, conservation integration ✓, determinism ✓, non-contested unchanged ✓.
- **Interface stability:** `build_clearing_plan` signature, `ClearingPlan`, `Fill`, `settlement_price`, `clear_market_good`, and all callers unchanged.
- **Regression safety:** existing auction/conservation tests are single-order ⇒ one bid at == marginal_bid, one ask at == marginal_ask, pro-rata of the whole quantity to a single order ⇒ identical single fill + identical settlement.
- **Type consistency:** helpers operate on `i64` price/qty extracted from `Money.0`/`Quantity.0`; `prorata_distribute(&[i64], i64) -> Vec<i64>`; `Fill.qty` rewrapped as `Quantity`.
- **No second interface:** rationing lives entirely inside `build_clearing_plan`; settlement and conservation paths untouched (no junk / no parallel engine).
