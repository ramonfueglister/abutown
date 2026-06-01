# Economy Slice 1 — Macro demand-driven flow, everywhere

Date: 2026-06-01

## Status

Approved in brainstorming (research-grounded, multi-agent design pass; all 7 critical / ~17 must-fix review items resolved below). Locked decisions:

- **One slice** (price-writeback + cross-market demand-driven flow + tier handoff). The riskiest piece — an explicit `k_bps` tâtonnement price-nudge — is **cut**; the band-clamped settlement price drives convergence with no tuning knob.
- **Demo/verification:** a dedicated multi-market **test world** hosts the convergence/direction tests, plus a **minimal live-seed extension** (a second good) so the live `economy_events` stream shows non-vacuous `MacroFlow`.
- **Band-derived dormant price:** accepted (see §3 STEP A). The alternative (price-inert until first auction) would mean an asleep market only converges *after* being observed — defeating the no-hollow-world guarantee.
- **`MacroFlow` event = summary** (`from,to,good,qty,price,transport`), mean-field; per-actor events are deferred to Slice 2.

This is the SOTA-aligned first step toward a mean-field macro/micro spatial economy (macro layer authoritative everywhere; visible traders a later sampled view). Implemented in its own worktree (`plan/economy-macro-demand-flow`) off `origin/main` (#68). Backend-only — **no render, WebSocket, or frontend wiring changes**; the CLAUDE.md browser-smoke mandate does **not** apply.

## 1. Goal

Make the *aggregate* market layer run for **every dormant market — warm AND asleep** — and flow goods **demand-driven across markets** (from cheap/surplus to dear/deficit, net of transport cost), replacing the warm-flow model shipped in #59 (`backend/crates/sim-core/src/economy/warm_flow.rs`).

Today #59 trades `min(total_demand, total_supply)` *intra-market* at a **frozen** local reference price, and only for markets anchored to a `WarmChunk`; markets anchored to an `AsleepChunk` get **zero** economic activity (`run_warm_market_flow_at_tick` filters on `warm_markets.contains(&pool.market)`, warm_flow.rs:45,57). That is the "hollow off-screen world": an asleep market neither trades nor moves its price, so when its chunk wakes its state is stale and incoherent with the observed economy.

Slice 1 closes that hole. The *macro* layer — a spatial network of per-(market,good) aggregates evolving toward **spatial price equilibrium** (Law of One Price net of transport, Samuelson/Takayama-Judge; approached by one deterministic relaxation step per coarse interval) — is the authoritative economic truth that must run *regardless of observation*. The slice is invisible but **real and observable** via the durable audit store (`economy_events`, PR #68): each accepted flow emits an `EconomyEvent` that drains to Postgres through the existing `LedgerAuditCursor` pipeline (audit.rs:19-57), so previously-frozen asleep markets finally produce auditable trade rows.

## 2. Architecture overview

Two cleanly partitioned tiers, disjoint per tick by `DormantMarkets`:

- **Active/Hot markets** keep the full order-book uniform-price auction (`clear_market_good_with_policy`, auction.rs:292). `generate_pool_orders_at_tick` and `run_traders_at_tick` skip dormant markets (pools.rs:79,118), so only active markets accumulate orders into `DirtyMarketGoods` and clear. **Untouched by this slice.**
- **Dormant markets** (warm + asleep) are governed by the new **macro flow** — a mean-field spatial-price-equilibrium layer. "Mean-field" is concrete: aggregate intent lives per-actor in `DemandPools`/`SupplyPools` (`BTreeMap<EconomicActorId, …>`, pools.rs:35-39) and is evolved *as numbers*; we never materialize an off-screen entity, FSM, or path. Scaling to "10000s of aggregate traders" means 10000s of **pool entries** scanned, not 10000s of ECS entities.

The macro flow is one deterministic excess-demand-driven step per coarse interval, per good: classify dormant markets as surplus (cheap) or deficit (dear) from their pools, route goods surplus→deficit when the price gap strictly exceeds transport, settle conservation-exactly, and **write the discovered price back** into `MarketGoodState.last_settlement_price` so prices drift toward equilibrium across intervals. Equilibrium is *approached across intervals*, never solved within a tick.

**Schedule placement.** The macro flow replaces the `WarmFlow` set's contents, renamed `EconomySet::MacroFlow`, in the same chained slot (systems.rs:55-82):

```
RefreshLod → ExpireOrders → Production → Traders → GeneratePoolOrders
           → ClearMarkets → MacroFlow → Materialize → Telemetry
```

It must stay **after `ClearMarkets`** (active markets publish fresh `last_settlement_price`) and **before `Telemetry`** (the EWMA fold picks up newly-written dormant prices). **New ordering constraint:** the economy chain is today anchored only `.before(tick_increment_system)` (systems.rs:81) with no tie to LOD reclassification (`CoreSet::LodReclassify` lives in `world/schedule.rs`, run in `world/plugin.rs:76`). Because the flow is now **stateful** (it writes prices), the set of markets it mutates must be a deterministic function of LOD classification. We therefore add `EconomySet::RefreshLod.after(CoreSet::LodReclassify)` (see §7).

## 3. The flow algorithm

`run_macro_flow_at_tick(accounts: &mut AccountBook, inventory: &mut InventoryBook, ledger: &mut TradeLedger, demand: &DemandPools, supply: &SupplyPools, market_goods: &mut MarketGoods, dirty: &DirtyMarketGoods, dormant: &BTreeSet<MarketId>, distances: &MarketDistances, config: &EconomyConfig, current_tick: u64) -> Result<(), EconomyError>`.

Interval gate (frozen-time-safe, keyed only on the live `Tick`):
```
if config.macro_flow_interval_ticks == 0
   || !current_tick.is_multiple_of(config.macro_flow_interval_ticks) { return Ok(()) }
```

**Conditional clone-validate-apply.** Compute candidate flows against the **live** books read-only first; only if the candidate set is non-empty enter the atomic boundary (`let mut next_accounts = accounts.clone(); let mut next_inventory = inventory.clone();`), mutate the clones with checked i128 ops, and commit `*accounts = next_accounts; *inventory = next_inventory; ledger.0.extend(events)` only on success. A truly-quiescent interval pays **no clone** (resolves the idle-cost contradiction — the inherited unconditional clone at warm_flow.rs:70-71 is `O(A)+O(A×G)` regardless of trade).

### STEP A — price signal & aggregate buckets

The pool model has **no** per-market clearing price to read: `DemandPool.max_price` / `SupplyPool.min_price` (pools.rs:17,30) exist, but the only settled price, `MarketGoodState.last_settlement_price`, is written **only** by the auction (auction.rs:396) and is `Money::ZERO` for any never-auctioned dormant market — which falls back to `trader_default_ref_price = Money(1000)` for both endpoints. A gap gate on that is identically zero across every never-auctioned pair → **moves nothing**.

**Resolution — derive a synthetic per-(market,good) price from the pools each interval.** Build buyers/sellers exactly as warm-flow does (group `DemandPools`/`SupplyPools` filtered to `dormant.contains(&pool.market)` — the only change from warm-flow's bucketing is swapping the membership set), then:

- **Effective demand** per buyer = `min(desired_qty_per_tick, affordable_qty(cash, bid_price))` read from the running `next_*` clones (warm_flow.rs:83-92). **Effective supply** per seller = `min(offered_qty_per_tick, on-hand available stock)` — **not** total accumulated inventory (resolves the production-burst risk: production runs globally for dormant actors, but the per-interval cap stays the deliberate LOD under-approximation).
- **Local synthetic price `p_m`** — the binding band, clamped with the auction's primitive. Let `bid_ceiling = max(buyer max_price)`, `ask_floor = min(seller min_price)`:
  - both demand & supply: `p_m = settlement_price_with_policy(prior, Money(bid_ceiling), Money(ask_floor), policy)` when `bid_ceiling >= ask_floor` (auction.rs:43-56; `prior = last_settlement_price if > 0 else trader_default_ref_price`).
  - supply-only: cheap → `p_m = ask_floor`. demand-only: dear → `p_m = bid_ceiling`.
- **Zero/negative-price guard:** replicate warm-flow's `if p_m.0 <= 0 { continue }` (warm_flow.rs:79-80) **before** any `checked_order_value`/`affordable_qty` call — a market with no usable band is skipped, never error-aborting the flow with `ZeroPrice`.

This makes supply-only asleep A (≈500) and demand-only asleep B (≈2000) yield a real `1500` gap on day one, with no prior auction.

**Convergence caveat (one-sided markets).** The supply-only and demand-only branches are **reservation-price-pinned** — they ignore `prior`, so a pure-source↔pure-sink pair flows *goods* every interval but its price gap does **not** narrow. This is correct economics: with no local price discovery (no competing opposite side) there is nothing to move a reservation price. **Price convergence (Law of One Price, §8 test 4) is therefore a property of price-discovering — both-sided — markets**, where `p_m = settlement_price_with_policy(prior, …)` drifts as the written-back `last_settlement_price` feeds the next interval. One-sided markets are *quantity-balanced* sources/sinks; driving their reservation prices by scarcity is a deferred enhancement (what the cut `k_bps` nudge would have done).

### STEP B — re-key by good
Group dormant buckets into `BTreeMap<GoodId, Vec<MarketGoodKey>>`. Each good is solved independently.

### STEP C — classify & budget
Per (market,good): `matched_m = min(total_demand_m, total_supply_m)` (the locally-clearable overlap); `surplus_capacity_m = total_supply_m − matched_m = max(0, supply − demand)` (exportable residual); `deficit_need_m = total_demand_m − matched_m = max(0, demand − supply)` (importable residual). At most one of surplus/deficit is non-zero. `matched` and the residual are **disjoint** quantities (overlap vs excess).

### STEP D — candidate directed edges
For each good, enumerate ordered pairs `(src, dst)`, `src ≠ dst`, both dormant, `src` with surplus capacity and `dst` with deficit need. **Distance** `dist = distances.get((src,dst))` from the precomputed `MarketDistances` table (§7) — the per-tick economy core stays graph-free. **Aggregate transport gate** (resolves the 1-unit-probe floor-to-zero bug): compute fill cap `q_cap = min(surplus_capacity_src, deficit_need_dst)` and test net gain on the **actual aggregate `q_cap`**:
```
net_gain = checked_order_value(p_dst, q_cap) − checked_order_value(p_src, q_cap)
           − transport_cost(dist, q_cap, rate)
keep candidate iff net_gain > 0   (strict — gap must exceed transport)
```
**Gate arithmetic — prune, don't fault.** `net_gain` uses checked i128 ops; if any term overflows (a pathological distance/qty), the edge is **pruned** — an uncomputable edge is not a trade opportunity, so gate-time arithmetic faults never become candidates and never emit events. (Settlement-time faults are different — see STEP H.) **Fixed-point transport floor.** `transport_cost(dist, q, rate) = (rate·q / ECONOMY_SCALE)·dist` floors to `0` whenever `rate·q < ECONOMY_SCALE` (e.g. the default `rate = 5` with `q < 200`). At demo-scale quantities transport is therefore ~free and the gate is correct-but-slack; the transport-gate tests (§8 test 7) set a config `transport_cost_per_tile_unit` (or quantities) large enough that transport is genuinely non-zero, so the Law-of-One-Price gate is actually exercised.

Also append a `from==to` **self-edge** for every market with `matched_m > 0` (`transport = 0`). Self-edges are **exempt from the `net_gain > 0` gate** (their `net_gain` is identically 0 since `p_src == p_dst` and transport is 0) — they clear the locally-matched `min(demand, supply)` whenever both sides exist, so intra-market clearing is one edge in the same single pass (no separate residual phase reading stale aggregates).

### STEP E — deterministic candidate sort
Sort all surviving candidates by a **total order over distinct keys**: `net_gain DESC, then good ASC, then src.market ASC, then dst.market ASC`. All ids are BTree-keyed → no surviving tie affects ordering.

### STEP F — single greedy pass with disjoint budgets
Per-market mutable counters initialized from STEP C: `remaining_matched[m]`, `remaining_surplus[m]`, `remaining_need[m]`. For each candidate in sorted order:
```
if src == dst {                              // self-edge: local clearing
    q = remaining_matched[src]
    if q <= 0 { continue }
    ... settle (STEP G, transport == 0) ...
    remaining_matched[src] -= q
} else {                                     // cross-edge: residual flow
    q = min(remaining_surplus[src], remaining_need[dst], q_cap)
    if q <= 0 { continue }
    ... settle (STEP G) ...
    remaining_surplus[src] -= q;  remaining_need[dst] -= q
}
```
The `matched` and `surplus`/`need` budgets are **disjoint** per market (overlap vs excess), so self-edges and cross-edges never contend for the same units — the self/cross ordering does not affect the outcome, and only the order **among cross-edges** (shared surplus/deficit) is load-bearing. Each budget is consumed **exactly once** → no double-spend; the result is a pure function of the sorted order. Single pass per interval, no inner fixpoint loop.

### STEP G — settle one accepted flow `(src, dst, good, q, p_src, p_dst, dist)`

**One pinned cash scheme — aggregate flooring only.** The per-line `checked_order_value` buyer-charging inherited from warm-flow is **FORBIDDEN** for this path (summing N per-line floors loses up to N−1 scale-units vs one aggregate floor, breaking reconciliation):

1. `src_revenue = checked_order_value(p_src, Quantity(q))` — one aggregate floor.
2. `transport_total = transport_cost(dist, Quantity(q), rate)` (= `Money(0)` for self-edges, transport.rs:29).
3. `dst_payment = src_revenue + transport_total` (transport **carved out of**, never added on top of, the buyer total).
4. **Sellers** at src: `seller_goods = prorata_distribute(seller_weights, q)`; consume from `next_inventory`. Seller cash = `prorata_distribute(seller_goods, src_revenue)` → `Σ == src_revenue`.
5. **Buyers** at dst: `buyer_goods = prorata_distribute(buyer_weights, q)`; deposit to `next_inventory`. Buyer charge = `prorata_distribute(buyer_goods, dst_payment)` → `Σ == dst_payment`; per buyer `lock_cash(charge)` then `debit_locked(charge)` (warm_flow.rs:130-131).
6. **Transport:** `next_accounts.deposit(TRANSPORT_OPERATOR, transport_total)`, `TRANSPORT_OPERATOR = EconomicActorId(u64::MAX)` (traders.rs:12) — never destroyed.
   - **Cash conservation:** `Σ buyer charges = dst_payment = src_revenue + transport_total = Σ seller cash + operator deposit`, zero rounding gap by construction.
7. **Market-state write-back** (the price discovery #59 lacked). On markets that traded this interval: set `last_settlement_price = p_dst` (dst) / `p_src` (src); `traded_qty_last_tick += q`; `last_cleared_tick = current_tick`; recompute `unmet_demand_last_tick`/`unsold_supply_last_tick` from post-flow residuals (mirrors auction.rs:395-402). **No `k_bps` nudge knob** — the written price IS the band-clamped settlement price of the realized flow; convergence emerges from repeated interval trading and does not fight the every-tick EWMA fold. Tests assert on `last_settlement_price`, not `ewma_reference_price`.

### STEP H — per-edge fault isolation
Settle each accepted flow against the clones; if a single edge errors **at settlement time** (a checked-arithmetic op overflows that the STEP-D gate did not already reject — e.g. a `traded_qty_last_tick` write-back accumulator at `i64::MAX`, or a price·qty product at pathological magnitudes), **skip that edge, emit `EconomyEvent::MarketClearFailed { market, good, reason }`, and continue**; one poisoning edge must not freeze the whole dormant economy. (Gate-time arithmetic faults are already pruned in STEP D, so this path handles **settlement** faults only.) **Note — cash over-charge is NOT a reachable settle fault here:** STEP A caps each buyer's effective demand to `affordable_qty(cash, p_m)` and the transport gate keeps each import's charge below that cap, so multi-source over-subscription cannot overdraw (verified during implementation by an exhaustive >1.5M-config parameter sweep — zero cash faults). The system wrapper **must stop swallowing the `Result`** (today `let _ = run_warm_market_flow_at_tick(...)`): on `Err` it emits an audit event. The atomic boundary is preserved per-edge — a faulted edge contributes nothing, healthy edges commit.

### STEP I — commit & emit
Commit the clones, extend the ledger. Emit one **`EconomyEvent::MacroFlow { from_market, to_market, good, qty, price, transport }`** per accepted flow (self-edges: `from==to`, `transport == Money(0)`). `event_type() == "macro_flow"`.

**Arithmetic invariants:** money/qty are i64 fixed-point (`ECONOMY_SCALE = 1000`); every product via i128 + `checked_order_value`/`transport_cost`/`affordable_qty` with `i64::try_from` → `Overflow`. `manhattan_tiles` rounds to integer tiles before subtraction (platform-stable, transport.rs:7-15).

## 4. Warm-flow reconciliation (clean replacement, no cruft)

**REMOVE (delete):**
- `warm_flow.rs` → renamed to `macro_flow.rs`, rewritten as above. Deletes `run_warm_market_flow_at_tick` and `warm_ref_price`.
- `WarmMarkets` resource (market.rs:75-79): sole consumer was warm-flow. Drop its registration, and drop the `mut warm: ResMut<WarmMarkets>` output + `warm_coords`/`warm.0` block from `refresh_dormant_markets_system` (systems.rs:102,104,107,114-119). That system now produces **only** `DormantMarkets`.
- `EconomyEvent::WarmMarketFlow` (ledger.rs:72-77) and its `event_type()` arm `"warm_market_flow"` (ledger.rs:98).

**RENAME / REUSE:**
- `EconomySet::WarmFlow` → `EconomySet::MacroFlow` (systems.rs:25,64,78). Same slot.
- `run_warm_market_flow_system` → `run_macro_flow_system`; profile label `"warm_flow"` → `"macro_flow"`.
- `EconomyConfig.warm_flow_interval_ticks` → `macro_flow_interval_ticks` (default 10).
- Reused verbatim: `prorata_distribute`, `checked_order_value`, `affordable_qty`, `settlement_price_with_policy`, `transport_cost`/`transport_cost_between` (this slice is the first production consumer of the currently-dead `transport_cost_between` path — satisfies no-dead-code), the clone-validate-apply skeleton, the `MarketGoods` persistence.

**Signature change:** the system drops `Res<WarmMarkets>`, adds `Res<DormantMarkets>` + `Res<DirtyMarketGoods>` + new `Res<MarketDistances>`, and takes **`ResMut<MarketGoods>`** (was read-only) so the flow can write discovered prices.

**Audit-row consequence (documented):** removing the serde-tagged `WarmMarketFlow` variant means historical `economy_events` rows tagged `"warm_market_flow"` no longer decode into the live enum. **Acceptable** — the store is append-only *observability*, never a recovery source (`EconomySnapshotStore` remains the recovery source of truth). Any future `/economy/events` read API must treat pre-slice rows as raw jsonb. Postgres maps every variant uniformly via `event_type()` + jsonb (postgres_economy_events.rs:63-71); no schema change for the new variant.

**Compile/test sweep (same slice):** delete `tests/warm_flow.rs`; **invert** `asleep_anchored_market_stays_frozen_end_to_end` (lod.rs:330) into `asleep_anchored_market_DOES_flow`; remove `WarmMarkets` registration + derivation; rename `warm_flow_interval_ticks`. The PR body must call out the inversion explicitly so it does not read as a regression.

## 5. LOD coupling & "everywhere" semantics

**Everywhere = all dormant markets flow.** `DormantMarkets = anchored markets whose chunk is not Active/Hot` (systems.rs:108-113), recomputed each tick (not persisted). With `WarmMarkets` deleted, the flow simply iterates `DormantMarkets` — warm and asleep treated identically. The chunk-LOD `Warm`/`Asleep` markers (`world/components.rs`) stay (they drive render/subscription LOD + the auction boundary); only the economy-flow distinction collapses.

**Slice-1 flow scope:** both flow endpoints AND price reads are restricted to `DormantMarkets`. Reading active markets as read-only price boundaries is **deferred** (it introduces an active→dormant continuity hazard not worth the first-slice risk). Demo consequence: if Demo-A is active and Demo-B dormant, B does not cross-market-flow until A also sleeps; convergence is an across-interval story.

**No double-counting — structural.** The dormant-skip partition already guarantees dormant markets generate zero orders (pools.rs:79,118) and never auction-clear. The flow gates **strictly on `DormantMarkets`, never on "not active"** — markets absent from `MarketChunks` are never dormant and must always auction (market.rs:62-67); gating on "not active" would sweep them in and double-trade.

**Tier-transition handoff (no heal-on-transition cruft).** On Active→dormant demotion, `generate_pool_orders`/`run_traders` stop immediately, but existing order-book bids/asks linger with **locked** cash/goods until TTL. `expire_orders_at_tick` stays **ungated** and re-dirties the key on expiry, so `clear_dirty_markets_system` runs one final auction clear. Two rules make the handoff lossless:
1. **Chain ordering** runs `ClearMarkets` before `MacroFlow`; the flow reads post-auction balances → no double-spend within a tick.
2. **`DirtyMarketGoods` skip-guard:** the flow takes `Res<DirtyMarketGoods>` and **skips any `(market,good)` key currently dirty** — a dormant market with a pending auction-clearable residual is still settling; the auction drains it, the flow takes over once the key is no longer dirty.

During demotion, locked cash/goods in stale orders are simply **unavailable** to the flow (it reads `available`, which excludes `locked`) until those orders expire/clear — conservation-safe (locked funds still count in `total_money`), explicitly **not** a "force-drain on demotion" heal hook. Dormant→active: pools resume one order (no backlog burst); the one-tick LOD lag is dwarfed by the 30-tick hysteresis.

**Per-tier cadence: NOT in Slice 1.** One `macro_flow_interval_ticks` (default 10) for all dormant markets. A warm-fast/asleep-slow split saves nothing at demo scale, adds a config-invariant footgun, and complicates the dirty lifecycle. Deferred.

## 6. Performance & scale

**Honest per-tick ceiling.** The macro **flow** is interval-amortized, but the economy chain *also* pays two **unconditional per-tick** scans that Slice 1 inherits and does not worsen:
- `update_market_telemetry` (systems.rs:211-227) loops over **all** `MarketGoods` every tick — `O(M_all × G)`/tick, no gate. Slice 1 makes this economically live for dormant rows but does not add the scan.
- `run_production_at_tick` has no dormant gate (production.rs:27) — `O(production_pools)`/tick.

LOD-gating the EWMA scan is named as the **next perf lever** — out of scope for Slice 1.

**Per macro-flow interval cost:** `O(P)` pool scan into buckets + `O(dormant_M² × G)` edge pairing/matching + `O(touched actors)` prorata, plus the conditional once-per-interval book clone. Demo (M=2, G=2) is trivial. Mean-field: **zero per-agent cost** (dormant traders FSM-gated off, traders.rs:92). `O(M²)` is structurally avoided from becoming a hidden quadratic only via later spatial pruning (deferred); the edge builder is the single swap point.

**Incremental `DirtyMarketGoods`: DEFERRED, with the honest reason.** Single-key dirty locality (which the auction relies on, systems.rs:180-181) **does not compose** with a cross-market flow: profitability is a per-*pair* property, and the price write-back re-dirties both endpoints, cascading. Slice 1 ships a **full per-interval scan of dormant buckets** — honest `O(dormant_M² × G)` per interval — and **does not claim O(0) idle from a dirty set**. The conditional-clone guard is what makes idle intervals cheap. (`DirtyMarketGoods` is still read — for the handoff skip-guard §5 — not for incremental recompute.)

**Spatial pruning: DEFERRED.** `NodeSpatialIndex.within_radius` returns `NodeId`; there is no `NodeId→MarketId` reverse index today. Because matching is edge-driven, the later swap (complete graph → within-radius neighbours) is localized and changes no conservation/price math.

**Bench guard.** No economy bench exists. Add a new `[[bench]]` at **two altitudes**:
- (a) **Isolated** `run_macro_flow_at_tick` on a flow-interval tick: `macro_flow_2m_2g` (demo parity) + `macro_flow_10k_pools_scale` (M≈200, G≈8, pools≈10000, all trading).
- (b) **Schedule-level** `economy_tick` running the full `EconomySet` chain over N ticks **including non-flow ticks**, parameterized by (M, G, A), with a large-`M×G`/small-`A` case to isolate the EWMA term.

**Budget:** schedule-level per-tick cost for the headroom case fits inside one mobility tick's frame budget; target `≤ 2 ms/interval` for the 10k-pool flow case on CI hardware; assert per-tick (non-flow) cost ~linear in `M×G` and flow cost ~linear in pools (compare 10k vs 20k — flag superlinear). **Gate:** run locally via `scripts/cargo-serial.sh`, record numbers in the PR; `>20%` regression on `macro_flow_2m_2g` or any superlinear trend blocks merge. (CI bench integration deferred — criterion noise on shared runners is its own task.)

## 7. Persistence, audit & determinism

**New persisted state — exactly one resource.** `MarketDistances(pub BTreeMap<(MarketId, MarketId), i64>)` (Manhattan tiles), computed once in `seed_demo_economy` from `Graph` + `manhattan_tiles(graph, a.node_id, b.node_id)` (mirrors how `Trader.distance_tiles` is baked at seed, seed.rs:51). **Persist it:** `apply_into_world` is graph-free (the economy core holds no Graph at hydrate, market.rs:62-67), so recompute-on-hydrate has no Graph available. Add `market_distances: Vec<((MarketId, MarketId), i64)>` to `EconomyPersistSnapshot` (14th field), serialized as a sorted Vec in BTreeMap order, stored **directed both-ways** (O(1) symmetric lookup). Extend `economy_snapshot_round_trips` / `economy_snapshot_is_byte_stable`. No serde-default shim.

**No flow cursor.** Cadence is purely `current_tick.is_multiple_of(macro_flow_interval_ticks)`; `Tick` is restored from the mobility snapshot, so cadence is implicitly persisted and frozen-time-correct. **No `last_macro_flow_tick` field** — adding one would invite offline catch-up (forbidden). `DormantMarkets` stays non-persisted.

**`EconomyConfig` is not persisted** (default-constructed by `EconomyPlugin::install` each boot); `macro_flow_interval_ticks` resets to `Default(10)` — static tuning, never evolving state.

**`MarketGoodState` already round-trips** (persist.rs:34), so the now-written dormant `last_settlement_price` + imbalance telemetry survive restart and resume from the saved Tick with zero migration.

**Audit (PR #68).** Each accepted flow emits `EconomyEvent::MacroFlow{…}` (and `MarketClearFailed` on a per-edge fault). These ride the existing `TradeLedger → pending_ledger_audit → EconomyEventStore::append → economy_events` pipeline (audit.rs:19-57, postgres_economy_events.rs) with **zero schema change** (jsonb payload + new `event_type` string). `LedgerAuditCursor` advances on success, re-inits to `ledger.len()` on hydrate. The `MacroFlow` event is **mean-field, summary-only** — explicitly **NOT** per-actor `CashLocked`/`Trade` events (that is the auction's behavior; per-agent events are Slice-2 territory and would reintroduce O(participants) cost).

**Conservation invariant (stated):** across any flow tick, `total_money` (Σ `available + locked`, accounts.rs:97-104) and `total_good(good)` (Σ `available + locked`, inventory.rs:109-118) are **exactly invariant**, transport a **transfer** to `TRANSPORT_OPERATOR` (not a sink). Goods conserve because `prorata_distribute` sums to `q`. Cash conserves because `Σ buyer charges = dst_payment = Σ seller cash + operator deposit` with zero rounding gap. Atomic clone-validate-apply; a per-edge error skips that edge leaving the books byte-identical for it.

**Determinism (stated):** no RNG/wall-clock/float (`DeterministicRng` is unused scaffolding; the thread_rng/rand ban is convention + tests + clippy). All iteration is BTreeMap/BTreeSet. Two explicit within-tick ordering dependencies: (1) candidate sort key `net_gain DESC, good ASC, src ASC, dst ASC`; (2) shared `remaining_surplus`/`remaining_need` budgets consumed once. `prorata_distribute` ties by ascending index. **New schedule ordering** `EconomySet::RefreshLod.after(CoreSet::LodReclassify)` makes the now-stateful flow's mutated-market set a deterministic function of LOD classification — without it the multi-threaded executor could classify before/after the marker swap and change saved state, breaking replay across save/restore.

## 8. Testing

A dedicated **multi-market test world** (a test-only seed building ≥3 markets / 2 goods with real supply/demand asymmetry on a fresh world) hosts the cross-market convergence/direction tests; the single-supplier/single-consumer demo cannot exercise cross-market flow alone. New `tests/macro_flow.rs` (replaces `tests/warm_flow.rs`), mirroring repo style (direct `run_macro_flow_at_tick(...)`, `total_money()`/`total_good()` before/after, `build() == build()` determinism, full-plugin install + manual `Tick` increment end-to-end).

1. **`macro_flow_conserves_money_and_goods`** — two dormant markets, surplus@A/deficit@B, finite distance. Assert `total_money` and `total_good` unchanged AND `TRANSPORT_OPERATOR.available` increased by exactly `transport_total`; no account `available < 0`.
2. **`macro_flow_conserves_with_N_buyers_per_line_floor`** — N≥2 buyers at a price/qty that floors per-line. Assert `total_money` invariant AND operator delta == `transport_total` exactly. (pins the aggregate-floor cash scheme; forbids per-line charging)
3. **`macro_flow_is_deterministic`** + **`macro_flow_tiebreak_is_stable`** — `build() == build()` on the ledger; two equidistant deficit markets competing for one surplus → assert the largest-remainder/ascending-MarketId split, byte-identical across runs.
4. **`prices_converge_to_within_transport_cost`** — uses **both-sided** markets (each has a small local demand *and* supply, so `p_m` is the `settlement_price_with_policy(prior, …)` clamp that drifts): A net-surplus & cheap, B net-deficit & dear, with a config transport rate large enough that `unit_transport_cost > 0`. Loop N intervals; assert on **`last_settlement_price`** that the gap is monotone non-increasing and converges to `≤ unit_transport_cost + Money(1)`. Companion **`one_sided_pair_flows_goods_but_price_is_pinned`** asserts a pure supply-only/demand-only pair moves goods every interval yet its price gap stays constant (documents the §3 STEP A caveat).
5. **`asleep_anchored_market_DOES_flow`** (inverts lod.rs:330) — anchor A and B to **AsleepChunks**, gap > transport, **zero subscribers, no Active chunk**. Run several intervals; assert `last_settlement_price`/`traded_qty_last_tick` changed AND goods moved A→B. (proves no-hollow-world)
6. **`goods_flow_from_cheap_surplus_to_dear_deficit`** — assert seller inventory@A decreased, buyer@B increased by the same `q`; `MacroFlow` event has `from==A, to==B`. Symmetric negative: swap, assert direction reverses.
7. **`no_flow_when_gap_below_transport_cost`** (and `==`); **`flow_starts_one_unit_above_transport_cost`** — pins strict-greater on the **aggregate** value.
8. **`macro_flow_only_fires_on_interval`** — tick 3 (not mult of 10): no flow; tick 0/10: flow.
9. **`active_to_dormant_handoff_conserves`** + inverse — demote a market with a live partially-locked order; run flow intervals through the TTL window. Assert every tick conserves; no `MacroFlow` trades the locked portion; a dirty key is skipped even on a flow-interval tick; released resources become flow-eligible after expiry.
10. **`poisoning_market_does_not_abort_others`** — construct a genuine **settle-time** checked-op fault (e.g. force a `traded_qty_last_tick` write-back accumulator to overflow at `i64::MAX`) on one edge alongside a healthy pair. Assert the healthy pair still flows + conserves, the faulted edge moves nothing (books byte-identical for it), and exactly one `MarketClearFailed` is emitted. (Cash over-charge is provably unreachable — see §3 STEP H — so it is not the fault trigger.)
11. **`macro_flow_emits_auditable_events`** + `event_type` extension — `MacroFlow{..}.event_type() == "macro_flow"`; drain via `pending_ledger_audit`/`commit_ledger_audit` into `InMemoryEconomyEventStore`; assert tick + type + both `from_market`/`to_market` round-trip through serde jsonb; cursor advanced.
12. **`macro_flow_replays_across_restart`** — run N intervals, `extract_from_world`→serialize→`apply_into_world` to a fresh world, run M more on both, assert identical `MarketGoods` + `AccountBook` + ledger tail. Extend `economy_snapshot_round_trips`/`_is_byte_stable` for `market_distances`.
13. **`dormant_producer_does_not_burst_dump`** — a dormant producer accumulates stock; assert per-interval flow bounded by `offered_qty_per_tick`, not total accumulated inventory.

**Edge cases (each its own `#[test]`):** `no_demand_no_flow`/`no_supply_no_flow`; `single_market_no_partner`; `zero_distance_markets` (transport 0 → full equalization, conserves); `overflow_edge_is_pruned_not_faulted` (a gate-overflow edge is dropped in STEP D — no candidate, no event, no panic, books unchanged); `tiny_qty_floors_to_zero` (a flow whose `rate·q/SCALE` floors transport to 0 still conserves); `zero_price_band_market_skipped` (the `p_m.0 <= 0` guard).

## 9. Scope & deferred

**Slice 1 IS:** (a) macro flow for **all** dormant markets — closes frozen-asleep; (b) **demand-driven cross-market** surplus→deficit net of transport, replacing #59's intra-market frozen-price `min(D,S)`; (c) it **writes** dormant prices + imbalance telemetry (band-clamped settlement, `ResMut<MarketGoods>`); (d) invisible but real and **audit-verifiable** via `MacroFlow` rows; (e) **clean replacement** of warm-flow (rename + delete, no dual path); (f) `MarketDistances` resource + persist field; (g) the `.after(CoreSet::LodReclassify)` ordering; (h) the conditional-clone perf guard + new bench; (i) a dedicated multi-market test world + a minimal live-seed extension (a second good) so the live `economy_events` stream is non-vacuous.

**Deferred:**
- **Slice 2:** visible/materialized walking traders as a conservation-exact **sampled read-side view** of the macro flow (render + wire); the macro flow stays the authority.
- **Slice 3:** spatial pruning via `NodeSpatialIndex.within_radius` + a `NodeId→MarketId` reverse index, cutting `O(M²)` → `O(M×k)`.
- Incremental per-good network-dirty macro-flow tracking (perf only).
- LOD-gating the unconditional per-tick EWMA telemetry scan (the next perf lever after the clone).
- An explicit `k_bps` tâtonnement price-nudge policy.
- Per-tier (warm vs asleep) cadence; reading active markets as price boundary conditions; using inert `urgency_bps`/`elasticity_bps`; per-actor cash events; true network/path-length transport cost; per-chunk economy snapshot partitioning; a `/economy/events` read API.

## 10. To resolve during planning (against real code)

1. Exact `MarketGoodState` field names for the write-back (confirm `last_settlement_price`, `traded_qty_last_tick`, `last_cleared_tick`, `unmet_demand_last_tick`, `unsold_supply_last_tick` against market.rs).
2. The precise `settlement_price_with_policy` / `Anchored` clamp signature reused for the both-sided band price (auction.rs:43-56).
3. The minimal live-seed extension shape (a second good with a supplier at one demo market + a consumer at the other) so the demo exhibits a real cross-market gap without enlarging the world.
4. Confirm `EconomySet` variant + `.chain()` wiring edits compile against the current `systems.rs` set ordering.
