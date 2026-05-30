# Economy Rationing v0 Design — marginal-price-group pro-rata

Date: 2026-05-30

## Status

Economy roadmap **slice 5 (Rationing policy)**: *"add marginal-price-group
pro-rata rationing behind the same auction interface."* The current uniform-price
call auction (`build_clearing_plan`) matches with strict **price-time priority**:
at the marginal (clearing) price the earliest / lowest-id orders are filled in
full and later orders at the same price get nothing. This slice replaces the
time-priority allocation **at the marginal price tier only** with deterministic
integer **pro-rata** (largest-remainder / Hamilton apportionment), so when supply
is scarce at the margin every competing order is filled proportionally to its
size rather than first-come-first-served.

Entirely behind the existing interface: `build_clearing_plan`'s signature,
`ClearingPlan`/`Fill`, `clear_market_good`, the uniform settlement price, and all
conservation machinery are unchanged. Backend-only; nothing crosses the wire.

## Goal

Within a single market-good clearing: keep the clearing quantity `Q`, the
marginal bid/ask prices, and the uniform settlement price `p` **exactly as today**
(computed by the unchanged price-time greedy walk). Change only **how the
matched quantity is allocated** among orders:

- **Infra-marginal** orders (strictly better-priced than the marginal price on
  their side) are filled in full — price priority is preserved.
- **Marginal-tier** orders (priced exactly at the marginal price) split the
  remaining matched quantity **pro-rata** by their size, with leftover integer
  units distributed by largest fractional remainder, ties broken by the
  deterministic sort order (price, then `created_tick`, then `id`).
- Sub-marginal orders (worse than the marginal price) are not filled.

## Architecture

Three phases inside `build_clearing_plan(key, bids, asks, last_settlement_price)`
(same signature, returns the same `ClearingPlan`):

### Phase 1 — clearing quantity + margins (UNCHANGED price-time greedy)

The existing greedy walk (sort bids desc by `max_price` then `created_tick` then
`id`; asks asc by `min_price` then `created_tick` then `id`; walk while
`bid.max_price >= ask.min_price`) runs as today, but accumulates only the **total
matched quantity `total_q`** and the **marginal prices** (`marginal_bid` =
`max_price` of the last bid touched, `marginal_ask` = `min_price` of the last ask
touched). Its pairwise fills are *not* used for allocation. This guarantees `Q`,
the marginal prices, and therefore `settlement_price(last, marginal_bid,
marginal_ask)` are byte-identical to the current behavior.

If no pair matched (`marginal_bid`/`marginal_ask` are `None`): return an empty
`ClearingPlan` with `settlement_price = None`, `unmet_demand = total_bid_qty`,
`unsold_supply = total_ask_qty` (same as today's no-trade path).

### Phase 2 — per-side allocation (`allocate_side`)

A pure helper computes a per-order allocation vector for each side:

```rust
fn allocate_side(prices: &[i64], qtys: &[i64], marginal: i64, total_q: i64,
                 better_is_higher: bool) -> Vec<i64>
```

For each order: if it is strictly better-priced than `marginal`
(`> marginal` for bids, `< marginal` for asks) it is **infra-marginal** and gets
its full `qty`. Orders priced `== marginal` form the **marginal tier**. The tier
shares `to_ration = total_q − infra_sum` via `prorata_distribute`. Sub-marginal
orders get 0. (Because the greedy walk fills all strictly-better orders before
reaching the marginal price, `infra_sum ≤ total_q` and `to_ration ≤
marginal_tier_qty_sum`, so every allocation is `≤` its order's quantity.)

### Phase 3 — pair allocations into fills (`pair_fills`, north-west-corner)

Given per-bid and per-ask allocation vectors that both sum to `total_q`, a
deterministic north-west-corner walk over the sorted orders emits `Fill { bid,
ask, qty }` records: advance past any order whose remaining allocation is 0, else
fill `min(bid_rem, ask_rem)`. This yields a valid bipartite set whose per-bid and
per-ask sums equal each order's allocation — exactly what `clear_market_good`
needs to settle (it tolerates multiple fills per order; locked cash/goods are
decremented per fill).

`unmet_demand = total_bid_qty − total_q`, `unsold_supply = total_ask_qty −
total_q` (identical semantics to today: total minus matched).

### Integer apportionment helper (`prorata_distribute`)

```rust
/// Largest-remainder (Hamilton) integer apportionment. Distributes `total` units
/// across `weights` proportionally; leftover units (from flooring) go one-by-one
/// to the largest fractional remainders, ties broken by ascending index (callers
/// pass weights in deterministic order). Output length == weights; each output ≤
/// its weight when `total ≤ sum(weights)`. All inputs non-negative.
pub fn prorata_distribute(weights: &[i64], total: i64) -> Vec<i64>
```

Uses `i128` for the `total * weight` products to avoid overflow, integer floor +
remainder, then a stable sort of `(remainder desc, index asc)` to assign the
leftover. Deterministic; no float, no rng.

## Conservation / determinism

- **Quantity balanced:** both allocation vectors sum to `total_q`, so total goods
  bought == total sold; `pair_fills` preserves this. `clear_market_good`'s
  existing checked arithmetic then conserves money and goods exactly as before.
- **Per-order safety:** each allocation ≤ the order's `qty_remaining`, and the sum
  of an order's fills ≤ its `cash_locked_remaining` / `goods_locked_remaining`
  (locked at order price × full qty), so no debit underflows.
- **Deterministic:** sort order is total (price, `created_tick`, `id`); pro-rata
  leftover assignment is a stable largest-remainder rule; integer-only.
- **Settlement unchanged:** `settlement_price` and the chosen `Q`/margins are
  produced by the unchanged Phase-1 walk, so prices are identical to pre-rationing.

## Testing

Pure-function tests on `prorata_distribute`:
- exact division (`[10,10]`, total 10 → `[5,5]`),
- leftover by largest remainder (`[1,1,1]`, total 2 → `[1,1,0]` — first two by
  index tiebreak),
- proportional (`[30,10]`, total 20 → `[15,5]`),
- total ≥ sum returns each weight; total 0 returns zeros; never exceeds a weight.

`build_clearing_plan` tests:
- **Non-contested unchanged:** existing `no_trade_without_price_overlap`,
  `trade_happens_with_price_overlap`, partial-fill, and conservation tests stay
  green (single bid/ask ⇒ pro-rata == greedy: one fill).
- **Contested margin pro-rata:** two equal-price bids (sizes A, B) vs one smaller
  ask at the same price ⇒ each bid filled proportionally (e.g. `1000` ask split
  `500/500` across two `1000` bids; `1500/500` across `1500`+`500` bids), instead
  of the first bid taking everything.
- **Infra-marginal priority preserved:** a higher-priced bid fills fully before
  the marginal tier is rationed.
- **Integer remainder determinism:** an odd contested quantity (e.g. ask `1001`
  across two equal bids) splits `501/500` deterministically by the tiebreak; two
  identical inputs produce identical plans.
- **Conservation with rationing:** a contested clearing run through
  `clear_market_good` conserves total money and total goods.

Full gate: existing economy/auction/conservation/pools/traders/production/
transport/LOD suites unaffected; fmt + clippy `-D warnings` + `test --workspace
--all-targets` green.

## What this is NOT

- No change to the settlement-price rule (midpoint settlement remains a separate
  deferred policy variant).
- No change to order eligibility, expiry, locking, or the `clear_market_good`
  settlement path.
- No pro-rata across *different* price levels (price priority across tiers is
  strictly preserved; pro-rata applies only within the single marginal tier).
- No fee/priority-fee tie-breaking; size-proportional with a deterministic
  remainder rule only.

## Open questions (resolved)

1. Strictly-between settlement price vs the rationed tier → partition by the
   **marginal price** (`marginal_bid`/`marginal_ask` from the greedy walk), not by
   the settlement price `p`; infra = strictly-better, tier = equal. Resolved.
2. Integer leftover → largest-remainder (Hamilton), stable tiebreak by sort index.
   Resolved.
3. Pairing per-side allocations into `Fill`s → north-west-corner walk; tolerated
   by `clear_market_good` (multiple fills per order). Resolved.
4. Interface stability → `build_clearing_plan` signature + `ClearingPlan` + all
   callers unchanged. Resolved.
