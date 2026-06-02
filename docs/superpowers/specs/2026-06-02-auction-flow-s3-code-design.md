<!-- S3 code-level design: resolved via a 5-agent design workflow (3 resolvers + adversarial conservation auditor + synthesis) on 2026-06-02. Binding refinement of the design spec §3-§5. -->

# S3 Final Code-Level Design — Auction↔Macro-Flow Coupling (ACTIVATING slice)

File-line anchors verified against `backend/crates/sim-core/src/economy/{macro_flow.rs,orders.rs,accounts.rs,auction.rs}` and `src/economy/tests/macro_flow.rs`. **Anchor correction:** the test helper `bucket(...)` lives at `src/economy/tests/macro_flow.rs:24-36` (an inline `mod tests`), NOT `tests/macro_flow.rs` — there is no top-level `tests/macro_flow.rs`. All test references below use the corrected path.

---

## 0. Two cross-area HOLES resolved before mechanism (from the adversarial audit)

The audit surfaced two genuine cross-area redesign decisions that the resolved areas left open. Both are resolved here as binding decisions, because every downstream signature depends on them.

### DECISION-1 (resolves audit A1/B1, the CRITICAL hole): the active bucket carries **one entry per residual order (per OrderId), NOT per owner**.

The resolved §3.1 said "group by owner." The audit proved this is unsound: released cash depends on per-row `max_price` drained in OrderId order, but a per-owner entry has only a scalar `max_price`, so the Stage-2 affordability predicate either over-admits (→ `lock_cash` `InsufficientFunds` at `macro_flow.rs:476` → stranded residual) or under-admits (lost trade). I confirmed multiple residual bids per owner per (market,good) ARE representable: the OrderBook (`orders.rs:50-53`) is keyed by `OrderId`, and the auction's `retain(qty_remaining > 0)` (`auction.rs:429-430`) preserves every partially-unfilled order independently.

**Resolution:** for active buckets, `MacroBucket.buyers`/`.sellers` carry **one (synthetic-actor, qty_remaining) entry per OrderId**, and the bucket carries a **parallel `Vec<OrderId>` and `Vec<Money>` (max_price)** aligned by index. This makes `max_prices[i]` unambiguous, makes the drain a 1:1 walk (no OrderId re-scan, no apportionment-within-owner), and is the only shape under which `released_i == checked_order_value(max_price_i, g_i)` holds exactly per the floor super-additivity proof. This does NOT break settle_flow's "per-entry apportion" contract — settle_flow (`macro_flow.rs:451-482`) apportions over whatever `(actor, weight)` slice it is given; it never assumes one-entry-per-distinct-actor. Two entries for the same `EconomicActorId` simply get two independent goods/charge shares, which is correct (each is a distinct order). **The dormant path is unchanged** — it remains one-entry-per-actor (a dormant pool is one (actor, market, good) row).

This means **MacroBucket must carry per-entry order provenance for active buckets** (the resolved-design "drain re-scans by (owner,market,good)" recommendation is rejected — re-scan cannot reconstruct which subset of an owner's orders Stage-2 kept). See §1.1 for the field.

### DECISION-2 (resolves audit A2/B3/E2): the prune's `q'` and the **surviving-entry residual** thread through to the ask drain, seller prorata, write-back `eff_*`, AND the FlowShipment qty.

After Stage-2 drops buyers, `q → q'`. Every consumer of `q` and `eff_*` for that edge must use the post-prune values. Concretely the per-flow loop produces a `PrunedEdge { q_prime, surviving_buyers, surviving_buyer_orders, surviving_bid_max_prices, eff_demand_dst_post, eff_supply_dst_post }` and feeds `q_prime` to: the bid drain (drain exactly `g_i` per surviving bid), the ask drain (drain `q_prime` across asks in OrderId order), `settle_flow` via a cloned `PlannedFlow` with `.q = q_prime`, and the FlowShipment `qty` (`macro_flow.rs:680`). The write-back `eff_demand_dst` becomes `Σ surviving_buyers.qty` (not the pre-prune `bucket.total_demand()`), so `unmet_demand_last_tick = (eff_demand_dst_post - q_prime).max(0)` correctly reflects post-drain OrderBook residual. If `q_prime == 0`, skip the edge entirely (no clone-mutation, no settle, no shipment), mirroring `plan_flows`' `q <= 0` continue (`macro_flow.rs:366,388`).

---

## 1. PRECISE MECHANISM, END-TO-END

### 1.1 MacroBucket gains `intra_cleared` + active order provenance (`macro_flow.rs:52`)

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroBucket {
    pub price: Money,
    pub buyers: Vec<(EconomicActorId, i64)>,   // active: one entry PER residual bid (OrderId), qty = qty_remaining
    pub sellers: Vec<(EconomicActorId, i64)>,  // active: one entry PER residual ask (OrderId), qty = qty_remaining
    pub intra_cleared: bool,                    // NEW: true ⇒ active/residual-sourced; false ⇒ dormant/pool-sourced
    // NEW active-only provenance, index-aligned to buyers/sellers; empty for dormant buckets:
    pub buyer_orders: Vec<OrderId>,             // buyer_orders[i] is the bid backing buyers[i]
    pub buyer_max_prices: Vec<Money>,           // buyer_max_prices[i] = that bid's max_price (Stage-1/Stage-2 input)
    pub seller_orders: Vec<OrderId>,            // seller_orders[i] is the ask backing sellers[i]
}
```

- **NOT persisted** — `MacroBucket` has no `serde` derive (confirmed: derive line `macro_flow.rs:51` is `#[derive(Debug, Clone, PartialEq, Eq)]` only). No snapshot schema change.
- `total_demand()`/`total_supply()` (`macro_flow.rs:61/64`) **UNCHANGED** — they sum `buyers`/`sellers` qty. For active buckets each qty IS that order's `qty_remaining`, so `total_demand = Σ residual-bid qty_remaining`, `total_supply = Σ residual-ask qty_remaining` — exactly the effective supply/demand feeding `classify_bucket`.
- Dormant buckets set `intra_cleared: false` and leave the three provenance Vecs **empty** (`Vec::new()`); they are read only on the active path.
- **Test helper `bucket(...)` (`tests/macro_flow.rs:24-36`)** must add `intra_cleared: false, buyer_orders: Vec::new(), buyer_max_prices: Vec::new(), seller_orders: Vec::new()` to the literal (compile-break otherwise). Add a sibling helper `active_bucket(price, buyers_with_orders_and_prices, sellers_with_orders)` for the new tests.

### 1.2 Two-source `build_macro_buckets` (`macro_flow.rs:92-196`)

New signature (two params appended after `config`, preserving `#[allow(clippy::too_many_arguments)]` at `:92`):

```rust
pub fn build_macro_buckets(
    accounts: &AccountBook,
    inventory: &InventoryBook,
    demand: &DemandPools,
    supply: &SupplyPools,
    market_goods: &MarketGoods,
    dormant: &BTreeSet<MarketId>,
    config: &EconomyConfig,
    orders: &OrderBook,            // NEW — read-only residual scan
    drain_active_residual: bool,   // NEW — = config.drain_active_residual (systems.rs:50)
) -> Result<BTreeMap<MarketGoodKey, MacroBucket>, EconomyError>
```

**DORMANT CONTRIBUTOR (unchanged):** the Phase-1/Phase-2 code (`:102-194`) stays byte-identical EXCEPT the `MacroBucket` literal at `:186-193` now sets `intra_cleared: false` + the three empty provenance Vecs. Dormant gates `if !dormant.contains(&pool.market) { continue; }` at `:107` and `:122` STAY. The affordability cap `affordable_qty(cash, price)` at `:166` and the on-hand cap `inventory.balance(...).available.0` at `:176` STAY (dormant-only).

**ACTIVE CONTRIBUTOR (new, gated `if drain_active_residual { ... }`):**

```rust
if drain_active_residual {
    // Phase A: scan residual bids/asks of ACTIVE (non-dormant) markets, OrderId order.
    // BTreeMap<OrderId,_> iter is already OrderId-ascending → deterministic.
    // Group into per-(market,good) entry lists; ONE entry per OrderId (DECISION-1).
    use std::collections::BTreeMap;
    struct ActiveAccum {
        buyers: Vec<(EconomicActorId, i64)>,
        buyer_orders: Vec<OrderId>,
        buyer_max_prices: Vec<Money>,
        sellers: Vec<(EconomicActorId, i64)>,
        seller_orders: Vec<OrderId>,
    }
    let mut active: BTreeMap<MarketGoodKey, ActiveAccum> = BTreeMap::new();

    for (id, bid) in &orders.bids {                 // OrderId-ascending
        if bid.qty_remaining.0 <= 0 { continue; }
        if dormant.contains(&bid.market) { continue; } // active markets only
        let key = MarketGoodKey { market: bid.market, good: bid.good };
        // STAGE-1 affordability filter (Part-1, §1.4 below): the buyer must be
        // willing to pay at least the market's own discovered price.
        let price = prior_price(market_goods, key, config); // == auction last_settlement_price (>0, see §1.5)
        if bid.max_price.0 < price.0 { continue; }   // below-price bid: left for expire_orders (§2.3 partition)
        let e = active.entry(key).or_insert_with(ActiveAccum::default);
        e.buyers.push((bid.owner, bid.qty_remaining.0));
        e.buyer_orders.push(*id);
        e.buyer_max_prices.push(bid.max_price);
    }
    for (id, ask) in &orders.asks {                 // OrderId-ascending
        if ask.qty_remaining.0 <= 0 { continue; }
        if dormant.contains(&ask.market) { continue; }
        let key = MarketGoodKey { market: ask.market, good: ask.good };
        let e = active.entry(key).or_insert_with(ActiveAccum::default);
        e.sellers.push((ask.owner, ask.qty_remaining.0));
        e.seller_orders.push(*id);
        // NO ask-side max_price filter (audit B2: ask weights stay == full residual).
    }

    for (key, acc) in active {
        let price = prior_price(market_goods, key, config); // authoritative auction price
        // NO `if price.0 <= 0` guard here — prior_price is provably > 0 (§1.5, audit G1).
        if acc.buyers.is_empty() && acc.sellers.is_empty() { continue; } // mirror :183
        debug_assert!(!buckets.contains_key(&key), "dormant/active key collision"); // DECISION: assert, no merge branch
        buckets.insert(key, MacroBucket {
            price,
            buyers: acc.buyers,
            sellers: acc.sellers,
            intra_cleared: true,
            buyer_orders: acc.buyer_orders,
            buyer_max_prices: acc.buyer_max_prices,
            seller_orders: acc.seller_orders,
        });
    }
}
```

**CRITICAL — active weight = `qty_remaining`, NEVER `available` (audit A6, spec §3.1 CRITICAL #4):** do NOT read `accounts.account(actor).available` or `inventory.balance(actor,good).available` on the active path. Post-`GeneratePoolOrders` everything is LOCKED in residual orders; available ≈ 0. The released `q` must equal the order's actual `qty_remaining`. This is a separate code path — it does NOT call `affordable_qty` (the `:166` dormant cap). This is exactly why the drain's `remaining == 0` invariant (§1.6) holds: bucket weight ≡ `Σ qty_remaining` of the contributing rows.

**DISJOINTNESS (audit A4, stronger than spec argued):** `DormantMarkets(pub BTreeSet<MarketId>)` (`market.rs`) is keyed by **MarketId**, so a market is wholly dormant or wholly active — there is no per-good split. The dormant pass (`:107/:122` gate) and the active scan (`if dormant.contains` skip) therefore source **disjoint MarketGoodKeys**. No merge-resolution branch — the `debug_assert!` above plus the disjointness test (§3.1) enforce it. This is a no-cruft invariant assertion, not a heal-on-collision guard.

### 1.3 Self-edge suppression — BOTH `build_candidates` AND `plan_flows`

**WHY (audit A3/C2, spec §3.3):** an active non-crossing market holds residual bids at low `max_price` AND residual asks at high `min_price` simultaneously (the auction correctly left both unmatched on price). `classify_bucket` (`:80`) is price-blind: it computes `matched = min(D,S) > 0` from quantities. Without suppression, `build_candidates` (`:240`) would emit a self-edge clearing `matched` at the intra price — re-trading units the auction explicitly refused → double-clear.

**`build_candidates` (`:227-254`) — adopt option (a), carry `intra_cleared` in the `by_good` tuple:**
- `:227` widen the alias: `type MarketClassification = (i64, i64, i64, Money, bool);` (5th = `intra_cleared`).
- `:230-234` read `b.intra_cleared` and store as the 5th field.
- `:240` destructure the 5-tuple: `for (market, (matched, _surplus, _deficit, price, intra_cleared)) in markets {`.
- `:241` gate change: `if *matched > 0 && !*intra_cleared {`.
- `:256` and `:260` cross-edge loops destructure the 5-tuple too (use `_intra` for the unused field): `for (src, (_m_s, surplus, _d_s, p_src, _intra)) in markets` and `for (dst, (_m_d, _s_d, deficit, p_dst, _intra)) in markets`.
- **Cross-edges (`:255-309`) UNCHANGED:** active surplus markets export, active deficit markets import — exactly the point. Only the self-edge is suppressed.

**`plan_flows` (`:344-409`) — defense-in-depth (spec §3.3 "even if a future caller bypasses build_candidates"):**
- `:352-353` after `let (matched, surplus, deficit) = classify_bucket(...)`, force: `let matched = if b.intra_cleared { 0 } else { matched };` before `remaining_matched.insert(...)`. `b` is in scope (`for (key, b) in buckets` at `:351`). A stray self-edge Candidate would then find `remaining_matched == 0` (`:361-365`) and skip at the `q <= 0 continue` (`:366`). `surplus`/`deficit` budgets (`:354-355`) UNCHANGED so cross-edges still plan.

### 1.4 Affordability bound — Stage-1 (build) + Stage-2 (per-edge prune)

The fault chain: `settle_flow` charges each buyer `apportion_cash(buyer_goods, dst_payment)[idx]` from AVAILABLE via `lock_cash` (`:476`) + `debit_locked` (`:477`). For an active dst, that available came from `drain_residual_bid` releasing the field-difference (≈ `value(max_price_i, g_i)` per DECISION-1's one-entry-per-order shape). If `max_price_i < dst_payment/q` blended, `value(max_price_i, g_i) < charge_i` → `lock_cash` faults `InsufficientFunds` (`accounts.rs:38-39`) → whole edge Errs → stranded residual.

`transport` depends on `(q, dist)` not known at build time, so the bound is two-stage:

- **STAGE-1 (build, §1.2 active path):** admit a residual bid into the active bucket only if `bid.max_price.0 >= prior_price(...).0` (the local discovered price). Uses only build-time data. Below-price bids are dropped from the bucket → NOT drained → TTL-expire (spec §2.3 partition, audit "PARTITION"). **No ask-side filter.**

- **STAGE-2 (per-edge, new helper `prune_unaffordable_buyers`, called in the per-flow loop AFTER `plan_flows`, BEFORE drain+settle):**

```rust
fn prune_unaffordable_buyers(
    buyers: &[(EconomicActorId, i64)],
    buyer_orders: &[OrderId],
    max_prices: &[Money],
    q: i64,
    dist: i64,
    p_dst: Money,
    config: &EconomyConfig,
) -> Result<(Vec<(EconomicActorId, i64)>, Vec<OrderId>, Vec<Money>, i64), EconomyError>
// returns (surviving_buyers, surviving_orders, surviving_max_prices, q_prime)
```

Predicate (exact integer form, bit-identical to settle_flow's own arithmetic — no division approximation): a surviving buyer index `i` with contributed share `g_i = prorata_distribute(buyer_w, q)[i]` is affordable iff
`checked_order_value(max_prices[i], Quantity(g_i)) >= apportion_cash(buyer_goods, dst_payment.0)[i]`,
where `dst_payment = checked_order_value(p_dst, q) + transport_cost(dist, q, rate)` (recomputed with the same helpers settle_flow uses at `:443-446`). By floor super-additivity, the drain's released-for-`g_i` ≥ `value(max_prices[i], g_i)` ≥ the predicate LHS, so if the predicate holds, released ≥ charge and `lock_cash` cannot fault.

**Fixpoint drop policy (resolves audit D2 — PIN IT):** each pass, drop **every** buyer failing the predicate, recompute `q' = Σ surviving g_i`, recompute `buyer_goods`/`dst_payment` on the survivors, and repeat until a pass drops nobody. This "drop-all-failing-then-recompute" policy is order-independent and deterministic (a single-pass or drop-one policy is NOT — dropping a buyer raises survivors' per-unit share, which can newly disqualify a marginal survivor). Bounded by `buyers.len()` passes. Uses only `prorata_distribute`/`apportion_cash`/`checked_order_value` (largest-remainder, tie-by-ascending-index, i64/i128, no float). If `q' == 0`, skip the edge.

**REJECTED alternative:** pushing the bound into settle_flow as a per-buyer drop would force a second `apportion_cash` re-clamp the spec forbids (§5.1). Keeping it as a pre-settle prune leaves settle_flow's `Σ buyer_charge == dst_payment` invariant intact over the surviving set.

### 1.5 Active price = auction `last_settlement_price` (authoritative), via `prior_price` (`:69`)

`prior_price(market_goods, key, config)` returns `state.last_settlement_price` when `> 0` (`:71`), else `config.trader_default_ref_price` (`:72`). For active markets the auction discovered a real `last_settlement_price` this tick (`auction.rs:437`), so `prior_price` IS that authoritative value — NOT `synthetic_price` (no pool bands exist for active markets). **No `if price.0 <= 0 { continue; }` guard on the active path (resolves audit G1, no-cruft rule):** `prior_price` falls back to `config.trader_default_ref_price = Money(1_000) > 0` (`systems.rs:60`), so non-positive is unreachable here — adding the guard would be exactly the "defensive guard for a post-fix-unreachable state" the user's no-cruft rule forbids. (The dormant `:159` skip stays — `synthetic_price` CAN return ≤0 from a zero/crossed band, so it is reachable there.)

### 1.6 Drain wiring + apportionment inside the per-flow loop (`macro_flow.rs:625-696`)

Make `scratch_orders` mutable at `:649`: `let mut scratch_orders = next_orders.clone();`. The drain mutates it; it folds on Ok (`:668`) / drops on Err (`:688-694`) alongside the other three clones.

Per-flow sequence (replacing/extending `:644-663`), gated on `config.drain_active_residual`:

1. **Recover** `sellers`/`buyers` from buckets (`:626-640`, now also clone `buyer_orders`/`buyer_max_prices`/`seller_orders`). Derive `src_active`/`dst_active`:
   `let dst = buckets.get(&MarketGoodKey{market: flow.dst, good: flow.good}); let dst_active = dst.map(|b| b.intra_cleared).unwrap_or(false);` (and `src_active` for `flow.src`).

2. **Stage-2 prune (if dst_active):** `let (buyers, buyer_orders, _max, q_prime) = prune_unaffordable_buyers(&buyers, &buyer_orders, &buyer_max_prices, flow.q, flow.dist, flow.p_dst, config)?;` If `!dst_active`, `q_prime = flow.q` and buyers unchanged. If `q_prime == 0`, `continue` (skip edge — no clone, no settle, no shipment).

3. **Clone scratch** (`:646-649`, `scratch_orders` now `mut`).

4. **Build a pruned flow:** `let mut eflow = flow.clone(); eflow.q = q_prime;` — used for the drain, settle, and shipment so `q'` threads everywhere (DECISION-2, audit A2/B3).

5. **ASK-side drain (if src_active):** apportion `q_prime` across this (market,good)'s asks in OrderId order. Drive by `q_prime`, NOT pre-prune `flow.q` (audit B3):
```rust
let ask_ids: Vec<OrderId> = scratch_orders.asks.iter()
    .filter(|(_, a)| a.market == eflow.src && a.good == eflow.good && a.qty_remaining.0 > 0)
    .map(|(id, _)| *id).collect();              // BTreeMap iter is OrderId-ascending
let mut remaining = q_prime;
for id in ask_ids {
    if remaining <= 0 { break; }
    let take = remaining.min(scratch_orders.asks[&id].qty_remaining.0);
    drain_residual_ask(&mut scratch_orders, &mut scratch_inventory, id, Quantity(take))?;
    remaining -= take;
}
if remaining != 0 { return Err(EconomyError::...); } // hard-assert: under-drain ⇒ conservation breach (caught by Err arm)
```
Releases seller goods locked→available 1:1 (`orders.rs:232-234`, no `value()` floor ⇒ no orphan, audit A6). `remaining == 0` holds because seller weight ≡ `Σ qty_remaining` and `q_prime ≤ surplus ≤ Σ supply` (audit B2). Note: the ask drain currently apportions a per-flow `q_prime`; for the single-source-active seed it walks all the asks of the src market. (If a future multi-edge interval drained the same ask across two flows, the per-flow `q_prime ≤ remaining surplus` from `plan_flows` budgets still bounds total draw ≤ `Σ qty_remaining`.)

6. **BID-side drain (if dst_active):** per surviving buyer entry `i`, drain EXACTLY `g_i = prorata_distribute(buyer_w, q_prime)[i]` from `buyer_orders[i]` (one OrderId per entry, DECISION-1 — no within-owner apportionment, no re-scan):
```rust
let buyer_w: Vec<i64> = buyers.iter().map(|(_, w)| *w).collect();
let buyer_goods = prorata_distribute(&buyer_w, q_prime);
for (i, &oid) in buyer_orders.iter().enumerate() {
    let g_i = buyer_goods[i];
    if g_i > 0 { drain_residual_bid(&mut scratch_orders, &mut scratch_accounts, oid, Quantity(g_i))?; }
}
```
`drain_residual_bid` releases the field-difference `cash_locked_remaining - checked_order_value(max_price, new_qty)` (`orders.rs:265`); at `new_qty == 0` it releases the full lock and removes the row (`orders.rs:269-271`), disposing the floor remainder (audit A7). Because the drain `g_i` == the settle share `g_i`, released and charged reference the identical quantity — no drift (the "fused per-buyer" intent of §5.1, realized by computing `buyer_goods` once and using it for both drain and the settle that follows).

7. **settle_flow runs (`:650`)** on `&eflow` (q = q_prime), with the two new `preserve_price_src/dst` params (§1.7). It charges each buyer `apportion_cash(prorata_distribute(buyer_w, q_prime), dst_payment)[i]` from available. **The refund is IMPLICIT (resolves audit A5, Part-2):** the bid drain already deposited `released` (at max_price) into available; settle debits only `charge` (at blended p_dst); the surplus `released - charge` STAYS in available. NO separate deposit — adding one would mint `released - charge` from nothing and break `total_money`. This mirrors `auction.rs:397-399`'s refund-to-available via release-then-charge-less. The seller side consumes `prorata_distribute(seller_w, q_prime)` from the available the ask drain just populated.

8. **Ok arm (`:664-687`):** fold all four scratch clones (`next_orders = scratch_orders` already at `:668` — now the drained-shrunk OrderBook). FlowShipment (`:669-685`) uses `eflow.q == q_prime` (audit A2 — shipment reflects ACTUAL flowed qty).

9. **Err arm (`:688-694`):** all four scratch clones (incl. the half-drained `scratch_orders`) drop by scope; `next_*` keep prior values; `MarketClearFailed` pushed → OrderBook + books byte-identical to pre-edge (spec §5.2 CRITICAL #5).

**Mixed flows:** active-src→dormant-dst drains only the ask side; dormant-src→active-dst drains only the bid side. The non-active endpoint uses pool-derived available unchanged. `src_active`/`dst_active` are derived independently (audit F2).

### 1.7 Price-authority write-back (`write_back` `:535`, `settle_flow` `:428`) — user decision (a)

`write_back` currently unconditionally sets `state.last_settlement_price = price` (`:548`). For active endpoints this must NOT overwrite the auction's price.

```rust
fn write_back(market_goods, key, price, traded, unmet_demand, unsold_supply, current_tick, preserve_price: bool) {
    let state = market_goods.0.entry(key).or_insert_with(|| MarketGoodState::new(key));
    if !preserve_price { state.last_settlement_price = price; }     // GATED — only line 548 guarded
    state.traded_qty_last_tick = state.traded_qty_last_tick.checked_add(Quantity(traded))?; // :549 ALWAYS
    state.unmet_demand_last_tick = Quantity(unmet_demand);          // :550 ALWAYS (post-drain residual)
    state.unsold_supply_last_tick = Quantity(unsold_supply);        // :551 ALWAYS
    state.last_cleared_tick = current_tick;                         // :552 ALWAYS
    Ok(())
}
```

`settle_flow` (`:428`) gains `preserve_price_src: bool, preserve_price_dst: bool` (after `current_tick`), passed into the two `write_back` calls (`:491` src, `:504` dst). The per-flow loop derives `preserve_price_src = src_active`, `preserve_price_dst = dst_active` (the same `intra_cleared` lookups from §1.6 step 1).

**Entry precedence (audit E2):** for an active endpoint the state row already exists (`auction.rs:436` created/wrote it this tick), so `entry()` returns the auction's row; `preserve_price=true` leaves `last_settlement_price` as the auction set it; `traded_qty_last_tick` `checked_add` (`:549`) ACCUMULATES onto the auction's value (a market can both auction-clear and flow-import in one interval — additive, correct); `unmet/unsold` are OVERWRITTEN (`:550-551`) to the post-drain residual `(eff_demand_dst_post - q_prime).max(0)` — the post-flow reality #71's shopper projection reads (`shoppers.rs:92`).

**DECISION-2 threading into write-back `eff_*`:** for the active dst, the settle_flow `eff_demand_dst` arg must be `Σ surviving_buyers.qty` (post-prune), NOT the pre-prune `effective(flow.dst, flow.good)` closure value at `:618-623`. Compute `eff_demand_dst_post = surviving_buyers.iter().map(|(_,q)| q).sum()` in the loop and pass that. (`eff_supply_dst` for an active dst with no residual asks is 0; for a both-sided active dst it stays `Σ residual-ask qty_remaining`.) Dormant endpoints keep the pre-prune closure values (no prune runs).

### 1.8 Flag flip + call-site (`:586`, `systems.rs:66`)

- Call-site `:586`: `build_macro_buckets(accounts, inventory, demand, supply, market_goods, dormant, config, &*orders, config.drain_active_residual)` — `&*orders` is a read-only reborrow (the `&mut` is consumed later by the drain at `:649+`).
- `systems.rs:66`: flip `drain_active_residual: false` → `true`. (`EconomyConfig` is NOT persisted — re-defaulted each boot — so this is compile-time-ish behavior; mirrored in `macro_flow_replays_across_restart` per §5.5.)
- The post-bucket dirty-skip filter (`:595-598`) STAYS. Per §3.4, `clear_dirty_markets_system` empties `dirty` before MacroFlow in the chained schedule (confirmed `ClearMarkets` before `MacroFlow`), so just-cleared active markets survive. Add a comment noting the empty-at-MacroFlow invariant (audit G2). The `flows.is_empty()` early return (`:602`) preserves the no-clone-on-quiescent-interval property (audit E3/G4).

---

## 2. TASK DECOMPOSITION (ordered, each TDD-able, each leaves the crate compiling)

Run all cargo through `scripts/cargo-serial.sh`. After each task: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core` + `fmt --check` + `clippy`.

1. **`MacroBucket` fields + dormant literal + test helpers.** Add `intra_cleared`, `buyer_orders`, `buyer_max_prices`, `seller_orders` (`:52`). Set `intra_cleared: false` + empty Vecs in the dormant literal (`:186-193`). Update `bucket(...)` helper (`tests/macro_flow.rs:24`) + add `active_bucket(...)`. Compiles; all existing tests green (dormant behavior byte-identical). *No behavior change yet.*

2. **Self-edge suppression (both sites).** Widen `MarketClassification` to 5-tuple (`:227`), carry `intra_cleared` (`:230-234`), gate `:241`, destructure cross-edge loops (`:256/:260`). Force `matched=0` for active in `plan_flows` (`:352`). TDD: `build_candidates_suppresses_active_self_edge` + `plan_flows_active_self_and_cross_disjoint` using a **both-sided non-crossing** `active_bucket` (NOT single-sided seed). Compiles; dormant tests untouched.

3. **`prune_unaffordable_buyers` helper (pure, isolated).** New fn (§1.4) with the fixpoint drop-all-failing policy. TDD in isolation: single affordable, single unaffordable→dropped, 3-bid mixed-max_price cascade (one drop disqualifies a marginal survivor → second pass), `q'==0` empty return, determinism (same input → same output). No call-site yet; compiles.

4. **Two-source `build_macro_buckets` (active contributor).** Add `orders`/`drain_active_residual` params (`:93`), call-site `&*orders` + `config.drain_active_residual` (`:586`). Active scan with Stage-1 filter + one-entry-per-OrderId + disjointness `debug_assert!`. TDD: active bucket weights == `Σ qty_remaining` (NOT available); Stage-1 drops below-price bid; dormant↔active disjointness; flag=false ⇒ zero active buckets (existing tests byte-identical).

5. **`write_back` + `settle_flow` price-authority params.** Add `preserve_price` to `write_back` (`:535`), `preserve_price_src/dst` to `settle_flow` (`:428`), thread to the two `write_back` calls (`:491/:504`). Per-flow loop passes `false/false` for now (no active path yet) — pure superset, dormant behavior byte-identical. TDD: `write_back_preserve_price_skips_price_keeps_traded_unmet`.

6. **Drain wiring + Stage-2 + `q'` threading in the per-flow loop.** `mut scratch_orders` (`:649`). Derive `src_active`/`dst_active`; call `prune_unaffordable_buyers`; build `eflow` with `q_prime`; ask-drain (q_prime, OrderId order, `remaining==0` assert); bid-drain (per-entry `g_i`); pass `eflow` + `eff_*_post` + `preserve` flags into settle; shipment uses `eflow.q`. Still gated `if config.drain_active_residual` (flag still false). TDD with flag forced true in-test: no-double-count (both-sided non-crossing active), affordability mixed-max_price no-fault, fault-injection isolation.

7. **Flip flag + integration + replay.** `systems.rs:66` → `true`. TDD: full-tick conservation (ClearMarkets→Drain→MacroFlow byte-identical money+goods); §2.3 active-residual-no-profitable-edge NOT drained + DOES expire; rewritten `active_to_dormant_handoff_conserves`; `macro_flow_replays_across_restart` with the flag on. Run the full CI gate before push.

8. **Rewrite stale handoff/conservation tests.** Audit every test that constructs a `MacroBucket` literal or asserts pre-S3 dormant-only handoff; rewrite to the new shape and the active-source reality.

---

## 3. MUST-HAVE TESTS

1. **No-double-count, CONSTRUCTED both-sided non-crossing active market** (the discriminating fixture, spec §3.3): residual bids @ low `max_price` + residual asks @ high `min_price`, auction left both. Assert zero self-edges in `build_candidates` AND zero self-flows in `plan_flows`; assert surplus/deficit emit only as cross-edges. **Single-sided seed m_a/m_b would pass even with broken suppression — MUST be both-sided.**
2. **Full-tick conservation:** `total_money + total_good` (available+locked over all actors incl `TRANSPORT_OPERATOR`) byte-identical before==after the whole `ClearMarkets → drain_residual_* → MacroFlow` tick.
3. **Affordability / mixed-max_price no-fault:** an active dst bucket with one buyer below the blended landed price; assert Stage-2 drops it, `q'<q`, settle does NOT fault, the survivors trade, conservation holds. **3-bid cascade variant** (dropping one disqualifies a marginal survivor → fixpoint second pass).
4. **Fault injection:** force `settle_flow` Err mid-edge (after the drain mutated `scratch_orders`); assert `orders.bids`/`asks` + `total_money` + `total_good` unchanged (Err arm drops the half-drained scratch).
5. **Write-back price-authority:** after a full tick where an active market both auction-clears and flow-imports, assert `last_settlement_price == auction value` (NOT synthetic flow price) AND `traded_qty_last_tick == auction_traded + flow_traded` (accumulated).
6. **§2.3 partition:** an active residual ask (and a below-Stage-1 bid) with no profitable cross-edge is NOT drained this tick AND DOES TTL-expire normally on a later tick via `expire_orders_at_tick`.
7. **Rewritten `active_to_dormant_handoff_conserves`:** a market active this interval → dormant next; its standing residual orders are sourced by neither contributor (active scan skips now-dormant; dormant pool pass produces none) and TTL-expire without leak/double-count (audit G3 — do NOT "fix" this into the active path).
8. **Multi-bid-per-owner** (DECISION-1 regression guard): one owner holds two residual bids at different `max_price` on the same (market,good); assert two bucket entries (per OrderId), the drain releases per-row correctly, and `released_i >= charge_i` per entry (no fault, no stranded available).
9. **Determinism/replay:** `macro_flow_replays_across_restart` with `drain_active_residual=true` — identical OrderBook + books after restart-resume.
10. **Disjointness assert:** dormant pool + active orders never collide on a `MarketGoodKey` (per-MarketId dormancy).

---

## 4. RESIDUAL OPEN QUESTIONS (genuinely need the user / a follow-up decision)

1. **DECISION-1 changes the resolved §3.1 "group by owner" to "one entry per OrderId."** This is the correct, audit-mandated shape (it is the only one under which the affordability predicate and the drain agree). It adds three Vecs to `MacroBucket`. This is a binding design change to the resolved area, not a detail — flag for the user/area-owner to confirm they accept the MacroBucket shape carrying per-order provenance (vs. the rejected re-scan approach, which cannot reconstruct the Stage-2-surviving subset). **Recommend: accept DECISION-1.**

2. **Cadence interaction (spec §5.3, user-accepted (b)):** the 10-tick `macro_flow_interval_ticks` means active residual orders can sit up to 10 ticks before a flow drains them, and a demoted market's residual waits up to `default_order_ttl_ticks` (also 10) for expiry. The prune is stateless per interval tick, so no behavior change is needed — but confirm the user accepts that an active market's unmatched residual is only *flow-served every 10th tick* (it is auction-served every tick; only the cross-market flow is decimated). The audit found no conservation issue here; this is purely a latency/economic-liveness acceptance, already covered by user decision (b). **No code change; confirm acceptance only.**

3. **`eff_supply_dst` for a both-sided active dst under Stage-2:** Stage-2 prunes only buyers, never asks (no ask-side filter). So a both-sided active dst's `unsold_supply_last_tick` write-back should use the pre-prune `Σ residual-ask qty_remaining` (asks weren't pruned), while `unmet_demand_last_tick` uses the post-prune surviving-buyer sum. The design (§1.7) does exactly this (`eff_supply_dst` unchanged, `eff_demand_dst_post` = surviving buyers). Flag only so the implementer keeps the two `eff_*_post` derivations asymmetric (supply pre-prune, demand post-prune) and does not "symmetrize" them. **No user decision needed; implementer note.**

Everything else (self-edge dual-suppression, dormant/active disjointness via per-MarketId dormancy, ask-side no-orphan, TTL non-double-release, scratch-orders fault isolation, conditional-clone, price-authority write-back, dirty-skip empty-at-MacroFlow, demoted-market partition, determinism of the active source) is fully resolved with verified anchors above.