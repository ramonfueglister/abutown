# Economy LOD Warm-Tier Design — aggregate flow for warm markets

Date: 2026-05-31

## Status

Follow-on to the merged **Economy LOD** slice (chunk-LOD gating). That slice
shipped only the gating foundation: markets anchored to Active/Hot chunks run the
full auction; markets anchored to Warm **or** Asleep chunks go fully dormant
(pools + traders skipped, frozen). This follow-on adds the **warm tier**: a
**Warm**-anchored market runs a cheap *aggregate* economic update instead of
freezing, so the unobserved-but-recently-active world keeps evolving — exactly
mirroring mobility's `warm_chunk_flow_system` (active = full per-agent, warm =
cheap aggregate flow, asleep = frozen/lazy). **Asleep stays frozen** (lazy
catch-up on wake, unchanged). Backend-only; deterministic; conservation-exact.

## Goal

Per coarse interval, each Warm market-good trades `min(aggregate demand,
aggregate supply)` at its **frozen reference price** (last settlement price; no
price discovery while unobserved), moving cash and goods between the participating
pool actors pro-rata. No order book, no per-order auction — O(pools), not the full
matching path. Money and goods are conserved exactly; the result is explainable
from prior state + elapsed time (the v2 wake invariant): prices didn't move
because nobody was watching, but stock and cash flowed at the last known price.

## Architecture

### Classification (extend the LOD bridge)

`refresh_dormant_markets_system` already computes `DormantMarkets` (markets
anchored to a NOT-Active/Hot chunk). Extend it to also populate a new
`WarmMarkets(BTreeSet<MarketId>)` resource = markets anchored to a **Warm** chunk:

```rust
pub fn refresh_dormant_markets_system(
    anchors: Res<MarketChunks>,
    active_chunks: Query<&ChunkCoordComp, Or<(With<ActiveChunk>, With<HotChunk>)>>,
    warm_chunks: Query<&ChunkCoordComp, With<WarmChunk>>,
    mut dormant: ResMut<DormantMarkets>,
    mut warm: ResMut<WarmMarkets>,
) { … }
```

Both queries read `&ChunkCoordComp` with disjoint filters (no conflict).
`DormantMarkets` is unchanged (warm + asleep both skip the full auction — the
full-fidelity systems gate on it as before). `WarmMarkets ⊆ DormantMarkets`.
Asleep markets are in `DormantMarkets` but **not** `WarmMarkets` → frozen.

### Aggregate flow (`run_warm_market_flow_at_tick`)

Runs only when `current_tick % config.warm_flow_interval_ticks == 0`
(default 10, like mobility's warm cadence). For each `(market, good)` with
`market ∈ WarmMarkets`, gathered from the demand/supply pools:

1. **Reference price** `p` = `MarketGoodState.last_settlement_price` if `> 0`,
   else `config.trader_default_ref_price` (same rule as the trader `ref_price`).
2. **Effective demand** per demand pool at this market-good: `d_i =
   min(desired_qty_per_tick, affordable_qty(buyer.available_cash, p))`. `D = Σ d_i`.
3. **Effective supply** per supply pool: `s_j = min(offered_qty_per_tick,
   seller.available_goods)`. `S = Σ s_j`.
4. **Traded** `Q = min(D, S)`. If `Q == 0`, skip.
5. **Allocate** `Q` to buyers pro-rata by `d_i` and to sellers pro-rata by `s_j`
   (`prorata_distribute`, the slice-5 helper). `buyer_goods`, `seller_goods`
   (each sums to `Q`).
6. **Cash:** `total_cash = checked_order_value(p, Q)`. Each buyer pays
   `checked_order_value(p, buyer_goods[i])` (≤ their available by the
   affordability cap). `total_cash` is distributed to sellers pro-rata by
   `seller_goods[j]` via `prorata_distribute` on cash, so `Σ seller receipts ==
   total_cash == Σ buyer payments` — money conserved exactly despite integer
   rounding.
7. **Apply** on cloned `AccountBook`/`InventoryBook` (atomic clone-validate-apply,
   mirroring `clear_market_good`): buyers `lock_cash` + `debit_locked` their
   payment and `deposit` their goods; sellers `consume` their goods and `deposit`
   their receipt. Commit the clones only if every step succeeds. Push one
   `EconomyEvent::WarmMarketFlow { market, good, qty: Q, price: p }`.

The warm path is fully **independent of the order book and of the pools'
`last_generated_tick`** (the full path's bookkeeping) — it neither reads nor
writes those, so there is no dual-path interference. Trading the per-tick desired
quantity once per (longer) warm interval is a deliberate LOD under-approximation
(warm regions trade less, cheaply); documented, not a bug.

### Schedule / config / ledger

- New `EconomySet::WarmFlow` in the chain after `ClearMarkets`, before
  `Telemetry`: `… ClearMarkets → WarmFlow → Telemetry`. `run_warm_market_flow_system`
  (normal `Res`/`ResMut`) runs in it. Active markets clear first, then warm
  markets flow (disjoint market sets; the chain satisfies bevy's mut-resource
  ordering).
- `EconomyConfig` gains `warm_flow_interval_ticks: u64` (default 10).
- `EconomyEvent::WarmMarketFlow { market: MarketId, good: GoodId, qty: Quantity, price: Money }`.
- `EconomyPlugin` inserts `WarmMarkets::default()`.
- `affordable_qty` in `pools.rs` becomes `pub(crate)` for reuse (no duplication).

## Conservation / determinism

- **Money conserved:** `Σ buyer payments == total_cash == Σ seller receipts`;
  applied atomically on clones. **Goods conserved:** `Σ buyer goods == Q == Σ
  seller goods`. No overdraw (buyers capped by affordability, sellers by
  availability; allocations ≤ those caps).
- **Deterministic:** pools iterated in `BTreeMap` order; `prorata_distribute` is
  largest-remainder with a stable tiebreak; integer-only; ref price from
  `MarketGoodState`. No rng/wall-clock/float.
- **Explainable on wake:** a warm market's prices are unchanged (no discovery);
  stock/cash flowed at the frozen price. On promotion to Active the full auction
  (with discovery) resumes; on demotion to Asleep it freezes.

## Testing

1. Bridge: chunks at each LOD + anchored markets → `WarmMarkets` holds exactly
   the Warm-anchored ones; `DormantMarkets` holds warm + asleep; active/hot in
   neither.
2. `warm_flow_trades_min_at_reference_price`: 1 demand (wants 100) + 1 supply
   (offers 60) at a warm market-good, ref price set → trades 60; buyer gains 60
   goods, seller loses 60, cash moved = `60·p/SCALE`.
3. `warm_flow_conserves_money_and_goods`: 2 buyers + 2 sellers, contested → total
   money + total goods invariant across the call; pro-rata split.
4. `warm_flow_respects_affordability_and_availability`: a cash-poor buyer / a
   stock-poor seller cap the trade; no overdraw.
5. `warm_flow_only_fires_on_interval`: no trade on non-multiple ticks.
6. `asleep_market_does_not_flow`: a market anchored to an asleep chunk (in
   `DormantMarkets`, not `WarmMarkets`) trades nothing.
7. `warm_flow_is_deterministic`: two identical worlds → identical ledgers/books.
8. E2E via the full schedule (Core+Mobility+Economy): an active market trades via
   the auction while a warm market flows aggregately and an asleep market freezes;
   total money conserved across the run; `EconomyPlugin` installs `WarmMarkets`.

Full gate: existing economy/auction/conservation/pools/traders/production/LOD/
rationing/persist suites unaffected (additive; warm flow only touches markets in
the new `WarmMarkets` set, empty in all existing tests); fmt + clippy `-D
warnings` + `test --workspace --all-targets` green.

## What this is NOT

- No price discovery in warm markets (prices frozen at last settlement — that is
  the point of the LOD tier).
- No change to the Active/Hot full-auction path or to the Asleep freeze.
- No order-book usage in the warm path; no touching pool `last_generated_tick`.
- No visible materialization of traders/agents (separate deferred slice; that one
  crosses the render boundary).

## Open questions (resolved)

1. Per-actor integer rounding breaking money conservation → distribute
   `total_cash` to sellers via `prorata_distribute` on cash so both sides sum to
   `total_cash`. Resolved.
2. Spend-from-available primitive → `lock_cash` + `debit_locked` (net available
   decrease, conserving) on cloned books; sellers `consume` goods + `deposit`
   cash. Atomic clone-commit. Resolved.
3. Warm vs asleep distinction → new `WarmMarkets` set from `WarmChunk`-anchored
   markets; asleep = dormant-but-not-warm. Resolved.
4. Interference with the full path → warm path ignores the order book and pool
   bookkeeping; runs on its own interval over a disjoint market set. Resolved.
