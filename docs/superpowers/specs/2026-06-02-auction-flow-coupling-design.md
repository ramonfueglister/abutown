# Auction ↔ Macro-Flow Coupling Redesign — Letting Active/Hot Markets Participate in Inter-Market Flow

**Status:** Design + sub-slice decomposition. Branch: design/research only (read code from `abutown-vtraders`, do not edit).
**Date:** 2026-06-02
**Predecessors:** #69 (macro demand-flow over dormant markets), #70 (visible flow-traders), #71 (visible shoppers).
**Author role:** lead architect, synthesizing the four per-dimension designs and resolving every critique issue.

---

## 1. Status & Goal

### 1.1 SOTA end-state

Today the economy partitions market-goods by a single coarse gate (`DormantMarkets`):

- **Dormant** (anchored, not observed): no orders generated (`pools.rs:79,118`), no auction; the macro flow is the *only* mover, clearing matched intra-market via a self-edge and moving surplus→deficit inter-market via cross-edges.
- **Active** (observed/Hot): pools generate live orders, the auction (`clear_market_good_with_policy`, `auction.rs:333`) clears the locally-matched portion every tick; the only inter-market hauling is the fixed-route demo `Trader` (`run_traders_at_tick`, `traders.rs:76`).

The committed end-state is a **fine-grained split that holds for ALL markets**:

> **AUCTION clears INTRA-market** (the locally-matched portion, for active markets, every tick).
> **MACRO FLOW moves INTER-market residual** (surplus→deficit net-of-transport) for **all** markets — active *and* dormant.

This is the textbook **Takayama–Judge / Samuelson spatial price equilibrium** the #69 spec (`docs/superpowers/specs/2026-06-01-economy-macro-demand-flow-design.md:31`) names as the model: locally-matchable demand clears locally; the genuinely-unmatched residual is precisely the spatial mispricing signal that should flow to where it is wanted, net of transport. This redesign is the explicit **un-deferral of #69 line 151** ("reading active markets as price boundaries is deferred … active→dormant continuity hazard") and the agreed next step in #70 line 106 ("unifying the observed-market economic Trader into the flow-derived model").

Concretely, once shipped:

1. The dormant-only gate at `build_macro_buckets` (`macro_flow.rs:107,122`) is *re-grained*, not deleted: `DormantMarkets` membership becomes a **source selector** (dormant → pool intent; active → post-auction residual orders), never an include/exclude switch.
2. Flow-traders (#70 `FlowShipment`) arrive/depart at the **observed** market the player watches, not just transit it.
3. The fixed-route demo `Trader` is **retired** (clean delete, no shim).

### 1.2 What changes

- `run_macro_flow_at_tick` gains `&mut OrderBook` + `&mut NextOrderId` and clones `OrderBook` into its atomic boundary (the single biggest structural change).
- A new **drain** step (release locked residual → available, shrink/remove the order) inside the flow's existing clone-validate-apply boundary.
- A **two-source** `build_macro_buckets`: dormant = pool intent (unchanged); active = post-auction residual *orders* (new).
- A per-bucket `intra_cleared` provenance flag that suppresses the self-edge for active markets.
- Demo `Trader` deletion across ~8 sites including the persisted `traders` field.

### 1.3 What stays invariant

- **Wire / render unchanged.** Zero protobuf / WS / frontend change. `FlowShipment` + `materialize` keep projecting MacroFlow edges through the existing `trader:` sprite path (`materialize.rs:150-194` is a generic `RenderActor` machine). The render layers (#70/#71) consume active-market edges with **no code change** — the only new visible behavior (spawn/despawn at the observed endpoint) emerges for free from the gate-lift. **Caveat:** the gate-lift *is* a frontend↔backend behavior change, so CLAUDE.md's browser-smoke mandate applies to the render-verification slice (S4) — unit tests are explicitly insufficient.
- **Conservation-exact.** `total_money()` (Σ available+locked over all accounts incl. `TRANSPORT_OPERATOR`, `accounts.rs:97-104`) and `total_good(g)` (`inventory.rs:109-118`) byte-identical before==after every tick.
- **No double-counting.** A (market,good) unit is auction-matched XOR flow-residual, never both.
- **Determinism / replay-safe.** BTreeMap/BTreeSet iteration; candidate sort `net_gain DESC, good/src/dst ASC` (`macro_flow.rs:318-326`); no RNG, no float in the economic path; no new persisted cursor.
- **Macro flow remains the sole economic authority.** #70/#71 are read-side projections; the auction owns intra-market matched, the flow owns all inter-market residual.

---

## 2. The Residual-Handoff Mechanism

**Adopted: mechanism (a) — the flow DRAINS post-auction residual orders inside its own atomic boundary.** This is *forced*, not preferred (see §2.4).

### 2.1 The grounded crux

After `clear_market_good_with_policy`, unmatched orders survive (`retain(qty_remaining>0)`, `auction.rs:429-430`) with their locks intact: a residual `Bid` holds `cash_locked_remaining` in the owner's `locked` cash bucket; a residual `Ask` holds `goods_locked_remaining` in `locked` inventory. `unmet_demand_last_tick` / `unsold_supply_last_tick` are **aggregate telemetry, not order handles** (`auction.rs:235-236,439-440`). `settle_flow` consumes from **available** (`inventory.consume`/`accounts.lock_cash` require available, `macro_flow.rs:457,476`). So the residual is *not freely flowable*: it must be **released (locked→available) first**, inside the same atomic clone that moves it inter-market.

### 2.2 The atomic sequence (per planned cross-edge, on scratch clones only)

The drain lives **inside** `run_macro_flow_at_tick`'s existing clone-validate-apply boundary, sharing **one** set of clones — `next_accounts / next_inventory / next_goods / next_orders` (the 4th is new) — committed together at `macro_flow.rs:689-692`. It is **NOT** a separate Bevy system with its own commit (see §5.2 for why this is load-bearing). For each planned cross-edge, on the per-edge **scratch** triple+orders:

**Ask side (unsold supply — the flowable good):** for drained qty `q` apportioned to a specific residual `Ask` (in `OrderId` order):
1. `inventory.release_goods(ask.owner, good, q)` — locked→available (1:1 quantity lock, no division).
2. `ask.qty_remaining -= q; ask.goods_locked_remaining -= q` (preserves `goods_locked_remaining == qty_remaining` exactly, for any `q`).
3. Remove the row when `qty_remaining == 0` (same `retain` discipline as the auction).
4. `settle_flow` consumes the now-available goods from the seller.

**Bid side (unmet demand — the deficit to satisfy):** for drained qty `q` apportioned to a specific residual `Bid`:
1. **Release the field-difference, never a recomputed per-`q` product** (resolves CRITICAL #1, see §5.1). Compute `new_qty = bid.qty_remaining - q`, `target_lock = checked_order_value(bid.max_price, new_qty)`, `released = bid.cash_locked_remaining - target_lock`. Then `accounts.release_cash(bid.owner, released)`, set `bid.cash_locked_remaining = target_lock`, `bid.qty_remaining = new_qty`. When `new_qty == 0`: `released = bid.cash_locked_remaining` (the *full* field, mirroring `expire_orders_at_tick:166`), and **remove the row** — this disposes the floor-drift remainder cleanly so no cash is orphaned in `locked`.
2. The released cash (priced at `max_price`) funds the buyer's `dst_payment` in `settle_flow` at flow price `p_dst`; the surplus `released − buyer_charge` is **deposited back to the buyer's available**, mirroring the auction's own refund (`auction.rs:394-399`). This is a **release+settle+refund wrapper, not `settle_flow` unchanged** (resolves critique MUST-FIX; see §5.1).

On `Ok`: fold `next_accounts/next_inventory/next_goods/next_orders = scratch_*` (all four). On `Err`: drop all four scratch clones, push `MarketClearFailed`, leave books byte-identical.

### 2.3 Drain scope — the order space partition (resolves order-lifecycle MAJOR)

The drain consumes **only** active-market residual orders for which the flow plans a **profitable** cross-edge (`net_gain > 0`, `macro_flow.rs:290-297`). Everything else stays for `expire_orders_at_tick`: dormant orders (none exist — dormant markets generate none), demoted-mid-interval orders, and active orders with no profitable cross-edge. **`expire_orders` is NOT made redundant** — drain and expire *partition* the order space, they do not race on it. A test must prove an active residual ask with no profitable cross-edge is NOT drained and DOES TTL-expire normally.

### 2.4 Why mechanism (a) is forced

`build_macro_buckets` caps demand by `affordable_qty(available_cash)` and supply by `inventory.balance().available` (`macro_flow.rs:166,176`); `GeneratePoolOrders` runs *before* `MacroFlow` and locks active markets' available into auction orders. So **merely lifting the gate while reading available makes the flow see ~zero for active markets** (everything is locked) — the "ordering trap." The residual is reachable *only* by releasing locks. `release_cash`/`release_goods` are confirmed partial-capable (check only `locked >= amount`, `accounts.rs:53` / `inventory.rs:67`). `debit_locked` is the auction *fill* path and must **never** be used by the drain (it consumes locked into the counterparty without returning to available; using it would conflate matched-exit and residual-exit and let a later `expire_orders` double-release a stale lock).

**Rejected:** (b) feed raw pool intent — double-counts (the same pool already became a locked order). (c) pre-split each pool into auction-estimate + flow-reserve — requires predicting the clearing outcome before clearing (non-deterministic; the auction is the authority on matched-vs-residual). (d) `debit_locked` the residual directly — wrong primitive, breaks `settle_flow`'s available-balance contract.

---

## 3. Lifting the Dormant Gate

**Adopted: a TWO-SOURCE `build_macro_buckets` keyed on `DormantMarkets` membership, with a hard structural split between matched-source and residual-source, plus explicit "no self-edge for active markets."**

### 3.1 Two contributors merging into one `BTreeMap<MarketGoodKey, MacroBucket>`

- **DORMANT contributor (unchanged):** pools whose market IS in `dormant` → buyers/sellers from pool intent (`desired/offered_qty_per_tick`) capped by affordable cash / on-hand available, exactly as `macro_flow.rs:106-182`. Carries matched + residual; **self-edge ON**; price = synthetic band price.
- **ACTIVE contributor (new):** markets NOT in `dormant` → **do NOT read `DemandPools`/`SupplyPools`**. Scan post-auction `orders.bids`/`orders.asks` filtered to (market,good) with `qty_remaining > 0`, grouped by owner: a residual `Bid` → `(owner, qty_remaining)` in `buyers`; a residual `Ask` → `(owner, qty_remaining)` in `sellers`. **Weight = `qty_remaining`, price = the auction's real `last_settlement_price`** (active markets already discovered a price — no synthetic price). This is a **separate code path**, not the reused `available`-cap path (resolves CRITICAL #4: reading `available` would read ~0 post-lock and decouple released `q` from the order's actual lock).

This requires threading `&OrderBook` into `build_macro_buckets` / `run_macro_flow_at_tick` (read-only at bucket-build time; `&mut` for the drain arrives in the same boundary).

### 3.2 No double-count — structural

Dormant markets generate **zero** orders, so their pool intent is the flow's sole consumer. Active markets feed the flow from **residual orders** = exactly the units the auction did NOT match. Therefore a unit is **auction-matched XOR flow-residual**, never both. The danger #69 line 153 names (lifting the gate while still reading pool intent) is avoided because active markets are sourced from `OrderBook` survivors, not from pools.

### 3.3 Self-edge reconciliation (resolves CRITICAL #4 / MAJOR)

Add `intra_cleared: bool` to `MacroBucket` (in-memory derived type — **not persisted**, no snapshot schema change): `true` for active (residual-sourced), `false` for dormant. The self-edge is the intra-market clearing primitive:

- **Dormant:** flow's self-edge IS the intra-clear → keep it (`build_candidates:240-254`, transport 0, gate-exempt).
- **Active:** the auction already did the intra-clear at `ClearMarkets` the same tick → the flow MUST emit **no self-edge** and force `matched = 0`.

Enforce in **both** places: `build_candidates` gates self-edge emission on `!bucket.intra_cleared`; `plan_flows` sets `remaining_matched = 0` for active buckets so no self-edge can ever be *planned* even if a future caller bypasses `build_candidates`.

**This is REQUIRED, not an optimization.** A non-crossing active market can simultaneously hold residual bids at low `max_price` AND residual asks at high `min_price` (the auction correctly left both unmatched). `classify_bucket` would then compute `matched = min(D,S) > 0` and emit a self-edge that executes a trade the auction *explicitly refused on price* — a double-clear and economic-meaning break. The test must use exactly this both-sided non-crossing construction; **the seed's single-sided m_a/m_b markets have `matched=0` by construction and would pass even with broken suppression** (resolves decomposition MAJOR). Mirror `plan_flows_self_and_cross_are_disjoint` (`tests/macro_flow.rs:463`; throughout this spec `tests/` = `backend/crates/sim-core/src/economy/tests/`) with an active both-sided market asserting zero self-edges and surplus/deficit appearing only as cross-edges. This preserves v0's named invariant that **settlement stays inside executable bid/ask bounds** (v0 §:224-226,636): the self-edge suppression + the §5.1 affordability bound are what keep the flow from clearing across a price gap the auction refused.

### 3.4 The `dormant` param's role changes (no half-gate cruft)

`DormantMarkets` is **not deleted** (markets absent from `MarketChunks` are never dormant and always auction, `market.rs:62-67` — deleting it would sweep un-anchored full-fidelity markets into the flow). Its role changes from *include/exclude* to *select source path*. This is the explicit, surfaced consequence the no-legacy-cruft rule demands. The active-residual source is **not** accidentally suppressed by #69's `DirtyMarketGoods` skip-guard: `clear_dirty_markets_system` empties `dirty.0` before MacroFlow in the same tick (the `GeneratePoolOrders→ClearMarkets→MacroFlow` chain), so the dirty set is empty at MacroFlow time and just-cleared active markets are not skipped. Correct, but state it so it is not left implicit.

---

## 4. Economic Trader Unification / Retirement

**RETIRE cleanly, as the LAST slice (S5), strictly gated on S3 (flow serves the m_a→m_b residual edge) + S4 (render parity browser-smoke verified).**

### 4.1 Why retire, why last

The demo `Trader` exists for exactly one thing the dormant-only gate forbade: visible inter-market hauling of `GOOD_TOOLS` between the **observed** m_a(9001)→m_b(9002) pair (it hibernates when its source is dormant, `traders.rs:92`). The redesign makes that gap disappear: the seed places supply@m_a and demand@m_b as separate single-sided markets, so once the gate is lifted the flow serves that exact surplus→deficit cross-edge directly.

**Keeping it would VIOLATE no-double-count:** the Trader posts real bids/asks into the *same* auction (`traders.rs:125-138,168-181`) on the *same* observed market the flow now drains residual from — the same haul attempted by two mechanisms, breaking disjointness/conservation. It also violates the no-legacy-cruft rule (a coexisting/disabled FSM is exactly the forbidden cruft).

**Retiring early** leaves the observed world with zero inter-market hauling mid-migration. Retirement MUST follow render-parity verification.

### 4.2 Render parity is automatic

The frontend cannot tell a `FlowShipment` from a demo `Trader` (shared `trader:` prefix, shared `kind:'trader'`). `FlowShipment` rendering (`materialize.rs:479-495`) and the generic `plan_mutations` machine stay. The player keeps seeing walking traders — now arriving/departing at the observed market itself, strictly an improvement.

### 4.3 Clean delete list (no `#[deprecated]` shim, no FSM left disabled)

Delete: `Trader`/`TraderState`/`Traders` + `run_traders_at_tick` + `adjust_price`/`ref_price` (`traders.rs`); `transport_ticks` (`traders.rs:56`) and `trader_travel` (`trader_render.rs:46-47`) — **these are demo-only** (resolves order-lifecycle MUST-FIX: `settle_flow` uses `transport_cost` + the independent `shipment_travel_ticks`, never `transport_ticks`); `run_traders_system` + `EconomySet::Traders` + chain wiring (`systems.rs:23,95`); `Traders` resource insert (`mod.rs:69`); the seed `Trader` insert (`seed.rs:176-189`) **keeping** m_a/m_b markets + their `MarketChunks`/`MarketDistances` + TOOLS pools (they become flow-served); the demo-trader render branch (`materialize.rs:460-474`) + the `endpoints()`/`is_outbound`/`leg_progress` helpers it solely feeds; the `traders` field of `EconomyPersistSnapshot` + extract/apply; tests `trader_arbitrages_between_markets_end_to_end` (`tests/systems.rs:238-344`), `tests/traders.rs` (incl. `transport_ticks_is_at_least_one`), `tests/lod.rs` `dormant_trader_is_frozen_and_conserves` (`lod.rs:206`).

**KEEP:** `TRANSPORT_OPERATOR` (`traders.rs:12` — `settle_flow` still uses it), `transport_cost`, `trader_tiles_per_tick` config (used by `shipment_travel_ticks`), `route_polyline` + `leg_polyline` (shared by `FlowShipment` at `materialize.rs:485`), `shipment_travel_ticks`. **Verify `leg_polyline` still compiles** after `endpoints`/`is_outbound`/`leg_progress` removal (it does not depend on `Trader`).

**Re-home conservation coverage before deleting:** migrate `trader_arbitrages`' `TRANSPORT_OPERATOR` money-conservation assertion onto a flow cross-edge test (total_money incl. `TRANSPORT_OPERATOR` + total_good byte-equal across the m_a→m_b residual flow) — coverage migrated, not dropped.

### 4.4 Persist field-drop is SILENT — make it loud (resolves decomposition CRITICAL)

**Verified:** `EconomyPersistSnapshot` derives plain `Deserialize` with **no `#[serde(deny_unknown_fields)]`**, `schema_version()` returns `1`, `migrate()` is identity (`persist.rs:26,134,150`). So a new binary reading an old save that contains a `traders` JSON key **silently ignores it** — it does **NOT** "fail to deserialize" (the decomposition plan's claim is FALSE). The demo Trader's in-flight cash/goods live in the separately-persisted `OrderBook` (8003's open bids/asks), so those survive; only the ephemeral FSM position is lost.

Per the no-defensive-guards / surface-consequences rule, the drop should be **visible** — but **verify the mechanism actually runs.** The provider's `migrate()`/`schema_version()` hook (`persist.rs:150`) is **NOT** on the economy hydration path: `PostgresEconomySnapshotStore::read` (`postgres_economy.rs:120`) deserializes with a strict `serde_json::from_value::<EconomyPersistSnapshot>` and never invokes `migrate()` (the read also keys on `base_world_schema_version`, not the provider value). So bumping the provider `schema_version`→2 + stripping `traders` in `migrate()` would be a **dead no-op** — it would *look* like a surfaced migration while doing nothing. Two correct options:

- **(a) Rely on the silent ignore + document it.** Plain `Deserialize` (no `deny_unknown_fields`) drops the legacy `traders` key on load, old saves resume cleanly, and 8003's orphaned orders self-heal (below). Simply call the drop out in the PR. Lowest-cruft.
- **(b) If a loud, code-visible drop is required,** add a real payload migration in `PostgresEconomySnapshotStore::read` mirroring `postgres_mobility.rs`'s `migrate_legacy_agent_birth_ticks` (`postgres_mobility.rs:70,129` — a `serde_json::Value` rewrite on the read path), or bump `base_world_schema_version`. **Do NOT use the unused provider `migrate()` hook.**

**Verify** — do not assume — that resuming a pre-retirement save leaves no orphaned actor-8003 orders that no system owns: `expire_orders_at_tick` is owner-agnostic (iterates all surviving rows by `expires_tick`), so 8003's orders TTL-expire and release normally on resume. State this in the PR; do not leave it implicit.

---

## 5. Conservation + Determinism

### 5.1 The invariant and the bid-side math (resolves CRITICAL #1, #2, minor #7)

**Invariant:** `total_money()` and `total_good(g)` (each Σ available+locked over all actors incl. `TRANSPORT_OPERATOR`) byte-identical before==after the full `ClearMarkets → Drain → MacroFlow` tick. Locked counts toward the total, so a release without an equal-and-opposite move breaks the fold (already enforced for the auction alone, `tests/conservation.rs:24-295`).

**The floor-drift orphan (verified critical).** `checked_order_value` floors via integer division by `ECONOMY_SCALE=1000` (`money.rs:68-71`) and is therefore **sub-additive**: `value(p, a) - value(p, b)` can be strictly greater than `value(p, a-b)`. Worked example: `max_price=Money(1500), qty=3` → `value(1500,3)=floor(4500/1000)=4`. Draining `q=1` three times, each releasing `value(1500,1)=1`, leaves `cash_locked_remaining=1` *orphaned in `locked`* with no order backing it — `total_money()` still balances (it counts locked) so the **assertion passes green while 1 unit leaks into unrecoverable locked-limbo.** This is the exact no-defensive-guards failure class the user flags.

**Fix (mandated):** the bid drain releases the **field-difference**, never a recomputed per-`q` product (the §2.2 rule): `released = cash_locked_remaining - checked_order_value(max_price, new_qty)`; at `new_qty==0`, `released = full cash_locked_remaining` and the row is removed (mirroring `expire_orders_at_tick:166`, which releases the full field, never a recomputed value). This disposes the drift remainder cleanly.

**The bid-refund is new cross-actor arithmetic (verified critical).** `settle_flow` charges buyers `apportion_cash(buyer_goods, dst_payment)` — a largest-remainder share of an aggregate, NOT each bid's own `value(max_price, q)`. So "release per-order then charge per-actor-aggregate" can diverge, and the refund can go negative (overcharge) for a buyer whose `max_price < blended dst price`, faulting `settle_flow`'s `lock_cash` (`macro_flow.rs:476`) → stranded residual. **Fix:** bound the drain — only flow a bid into a cross-edge whose per-unit landed price (`p_dst + transport/q`) does not exceed the bid's `max_price` (mirroring the dormant path's `affordable_qty` cap, `macro_flow.rs:166`); drop below-price bids (genuinely unwilling importers) for `expire_orders`. The `net_gain > 0` candidate gate should already imply this but it **must be asserted, not assumed**. Compute the refund as an exact per-buyer subtraction (`released_for_buyer − buyer_charge[idx]`) using the `apportion_cash` result directly — **never** a second `prorata_distribute` pass (which would clamp and drop sub-units).

**Ask side is safe (minor, stated explicitly for bound-the-audit):** `goods_locked_remaining == qty_remaining` is a 1:1 quantity lock with no division, preserved exactly for any `q`. `prorata_distribute` (clamped to Σ=q) is correct for goods. Keep bid and ask drain as **separate helpers** so a future refactor never applies the floored `value()` pattern to the quantity lock.

**Money/goods split rule:** `apportion_cash` (unclamped) for all money, `prorata_distribute` (clamped to Σ=q) only for goods — wrong choice mints/destroys transport money (tested `tests/macro_flow.rs:747-829`).

### 5.2 The atomic boundary — one shared boundary, per-edge scratch incl. orders (resolves CRITICAL #5, decomposition MAJOR)

The drain lives inside `run_macro_flow_at_tick`'s **single** clone-validate-apply boundary — **not** a separate system with its own commit. A separate drain system would expose a state where the order is shrunk (lock gone) but goods haven't moved; a per-edge fault then strands the residual in `available` with no order to re-bid it, and a crash between commits loses it — `total_money/total_good` would *still balance* at each commit, so conservation tests pass while the economy silently leaks residual into limbo. The single shared boundary makes release+shrink+move atomic.

**The per-edge fault isolation MUST be extended to `scratch_orders`.** Today the per-edge scratch is only accounts/inventory/goods (`macro_flow.rs:639-641`). The drain's lock-release **and** order-row shrink/remove must mutate `scratch_orders` and fold-on-`Ok` / drop-on-`Err` **together** with the other three, or a settle fault commits a half-drained order whose stale lock gets double-released by next tick's `expire_orders`. A fault-injection test must force `settle_flow` `Err` mid-edge and assert `OrderBook` bids/asks + `total_money/total_good` byte-identical to pre-flow.

### 5.3 Cadence / TTL — correct framing (resolves order-lifecycle MAJOR)

The "`macro_flow_interval==ttl==10` so the cadence is aligned / the residual was going to TTL-expire and re-bid anyway" rationale is **true only for dormant/demoted markets and materially wrong for the active case this redesign serves.** An active market's pool orders are **re-created every tick with a fresh `expires_tick`** (`pools.rs:63-154`; handoff test `interval_ticks:1`), so they **never TTL-expire while active.** There is a fresh, never-aging residual every tick; the flow fires only every 10th.

**Decision (S3 must make explicit):** the drain runs on every flow-interval tick over **whatever residual exists that tick** (fresh, never TTL-aged). S3 must decide whether the 10-tick gap (a watched surplus/deficit persisting ~9 ticks before inter-market relief) is acceptable latency, or whether `macro_flow_interval` should be reduced for the active path. Add a test that an active market with persistent residual sees inter-market flow **ON the interval tick** (not after a TTL window) — distinct from the demoted-market handoff test. **Do NOT carry the "10==10 so it just works" framing into the spec.**

**TTL/drain double-release:** because the drain runs *after* `ClearMarkets` in the same tick and `expire_orders` runs at the *top of the next* tick on the already-shrunk/removed row, a same-order double-release is impossible **provided the drain removes the row at `qty_remaining==0`** (make this a hard invariant in S2, the `retain(qty_remaining>0)` discipline). A drained-to-zero-but-not-removed ghost would round-trip the snapshot and be hit by `expire` with a zero lock — safe only by luck (`release_cash(0)` is a no-op); make it safe by contract.

### 5.4 Price write-back authority (resolves order-lifecycle MAJOR / the deferred #69 hazard)

Draining a standing active residual is **NOT** discovery-neutral: an unmatched ask sitting at a high `min_price` is exactly the supply pressure feeding next tick's marginal-band clearing (`auction.rs:183-202`); removing it changes the auction's discovered `last_settlement_price` trajectory. And the flow's `write_back` (`macro_flow.rs:548`) would **overwrite** the auction's `last_settlement_price` the same tick with a synthetic price. **Drop the "discovery is unweakened" claim.**

**Decision (S3 must make explicit):** for an active market the **auction price is authoritative** — the flow's `write_back` for active endpoints must **NOT** overwrite `last_settlement_price`; it only **accumulates `traded_qty_last_tick`** (the existing `checked_add`, `macro_flow.rs:549`) and recomputes the **post-drain** residual into `unmet_demand_last_tick`/`unsold_supply_last_tick` (read by the shopper projection #71, `shoppers.rs:92` — telemetry drifts if wrong). Verify the `entry`/`get_mut` interplay so the flow's accumulate does not clobber the auction's assigned `traded_qty` (the state entry exists because the auction created it). Add a write-back-precedence test. This is the deferred #69 line-151 hazard confronted head-on, not waved away.

**Behavioral interaction with #71 — name it, it is real (audit P1/P2).** Beyond telemetry-correctness, zeroing the post-drain residual has a *visible* consequence for the shopper projection. Once the m_a→m_b FOOD surplus→deficit edge is flow-served, on each `macro_flow_interval_ticks=10` tick the post-drain recompute sets m_b's `unmet_demand_last_tick`→0, so `ShopperCapture` (same tick, after MacroFlow) spawns **0** FOOD shoppers that tick — changing #71's demo cadence from a steady trickle to a blip-to-zero every 10th tick. This is **benign**: the other 9 ticks the auction re-dirties (m_b, FOOD), unmet returns, #71 reconciles, and `expire_arrived_shoppers` despawns nothing on a zero-spawn tick. But it **invalidates #71's load-bearing demo claim** ("`unmet_demand_last_tick` ≈ 10 persistently every tick", #71 §7:87–89) and can flake `smoke-shoppers.mjs`'s `count > 0` if it samples exactly on a flow-interval tick. **When S3 lands:** (1) update #71 §7 to state FOOD-at-m_b unmet is now flow-served on the interval (blip, not steady); (2) make `smoke-shoppers.mjs` sample across more than one tick / off a flow-interval boundary — or, cheaper, key the shopper demo on a good with NO m_a supply so unmet stays standing. S5's demo section must call this out so it does not read as a regression.

### 5.5 Determinism / persistence

Drain iterates `orders.bids`/`orders.asks` (`BTreeMap<OrderId,_>`) in `OrderId` order, filtered to (market,good) `qty_remaining>0`; apportions drained `q` back to specific orders in `OrderId` order; money splits use `apportion_cash` largest-remainder tie-by-ascending-index. No RNG, no float. Candidate sort and interval-gating unchanged. The drained set is a deterministic function of the post-auction `OrderBook`, itself a deterministic function of LOD (`RefreshLod.after(CoreSet::LodReclassify)`, `systems.rs:87-89`).

**`OrderBook` + `NextOrderId` ARE persisted** (`persist.rs:30-32,74-76`) and round-trip; the drain mutates that persisted state in-place inside the committed boundary, so snapshot→restart→continue reproduces identical state. **No new persisted cursor** — the residual lives in the already-persisted `OrderBook`, the drain is stateless per tick (avoids the LastProcessedMonth-class reload-reset bug). `EconomyConfig` is NOT persisted (default-constructed each boot); any drain knob must be re-applied on restart and mirrored in `macro_flow_replays_across_restart` (the test already does this for the transport rate, `tests/macro_flow.rs:2462-2469`). `FlowShipments`/`NextShipmentId` stay ephemeral. **Conditional-clone preserved:** the `next_orders` clone is added *after* the `flows.is_empty()` early return (`macro_flow.rs:600-601`), so the no-clone-on-quiescent-interval property holds; the read-only `OrderBook` scan at bucket-build runs every interval tick (acceptable — it already scans pools every interval tick), threaded as `&OrderBook`.

### 5.6 How a test asserts it

- **Full-tick conservation** (extend `tests/conservation.rs:226-295`): bind `before = total_money()/total_good(g)` over all actors incl. `TRANSPORT_OPERATOR`; run `ClearMarkets` then `MacroFlow` (with drain); assert unchanged AND assert the drained order's `qty_remaining`/locked shrank by exactly the flowed `q` (no orphaned lock).
- **Fractional-price zero-orphan** (resolves CRITICAL #1/#2): `max_price=Money(1500), qty=3`; fully drain; assert the owner's `locked` returns to its **exact pre-order value** (zero orphan) — not merely that `total_money` is unchanged (the existing `partial_fill_conserves` uses even `qty=500` with clean division and cannot catch this).
- **Mixed-max_price multi-bid** (resolves CRITICAL #2): a bucket with two residual bids at different `max_prices` (one below blended dst); assert no negative refund, no `InsufficientFunds`, `total_money` byte-identical, the below-price bid NOT flowed.
- **Fault injection** (resolves CRITICAL #5): force `settle_flow` `Err` mid-edge; assert `OrderBook` + books byte-identical.
- **No-double-count / no-self-edge** (resolves §3.3): both-sided non-crossing active market; assert `matched=0`, zero self-edges, surplus/deficit only as cross-edges.
- **Write-back precedence** (resolves §5.4): assert active-market `last_settlement_price` after the full tick equals the auction's value (not the synthetic flow price).
- **Rewrite `active_to_dormant_handoff_conserves`** (`tests/lod.rs:500-625`): keep the **per-tick conservation assertion** (`609-610` — the real atomicity regression guard); the **timing assertion** (`621-624`, consumer_goods>0 only after TTL) WILL and SHOULD change (consumer gets goods on the flow interval the drain fires). The PR must call out this inversion so it does not read as a regression.
- **Active no-TTL-expiry-while-active** + **expire-still-has-work** (resolves §2.3/§5.3): an active residual with no profitable cross-edge is NOT drained and DOES TTL-expire.
- **Replay** (extend `macro_flow_replays_across_restart`): run past a drain tick, snapshot, restart, continue; assert identical `OrderBook` and continuation; confirm no zero-qty ghost order survives.

---

## 6. Sub-Slice Decomposition

Five slices in dependency order; each conservation-green and merge-able on its own. The refinement of the candidate 3-slice (A/B/C) shape is justified: threading `&mut OrderBook` is a genuinely behavior-neutral standalone change; the drain primitive is genuinely separable; retirement is correctly gated last.

### S1 — PLUMBING (smallest valuable first slice; behavior-neutral) ← **SPEC THIS NEXT**

Thread `&mut OrderBook` + `&mut NextOrderId` into `run_macro_flow_at_tick` (`macro_flow.rs:560`) and `run_macro_flow_system` (`systems.rs:335-348`). Establish the **full clone topology S3 needs**: add `next_orders` at the top-level boundary (after the `flows.is_empty()` early return, preserving conditional-clone) **AND** `scratch_orders` inside the per-edge fault-isolation loop (clone from `next_orders`, fold on `Ok`, drop on `Err`) — even though S1 mutates neither (resolves decomposition MAJOR: otherwise S3's order mutation escapes per-edge isolation invisibly). Add `EconomyConfig.drain_active_residual: bool` defaulted **FALSE**. NO drain logic, NO gate change — flow still reads dormant pools only, every existing test stays byte-identical; `macro_flow_replays_across_restart` proves the extra clone round-trips. **Value:** lands the biggest structural risk green with zero behavior risk. **Deferred:** any residual reading.

### S2 — DRAIN PRIMITIVE (pure function, conservation-tested in isolation)

Add `drain_residual_bid(...)` / `drain_residual_ask(...)` (separate per-side helpers, §5.1) operating **only on passed-in clones**: scan in `OrderId` order, release via the field-difference rule (bids) / 1:1 (asks), **remove** the row at `qty_remaining==0`. **Keep it a PURE function with a standalone unit test over hand-built clones — do NOT wire it into `run_macro_flow_at_tick`, not even as a flag-gated no-op** (resolves decomposition MAJOR: the "no-op-by-default first phase" framing invites the release-without-consume / TTL-double-release limbo; the in-flow wiring belongs in S3 where a same-atomic-step consumer exists). Tests: `total_money`/`total_good` byte-equal; the fractional-price zero-orphan test; ask-side no-orphan symmetry test; assert the row is removed at zero. **Build the drain-test order fixtures with a `cash_locked_remaining` that EQUALS `checked_order_value(max_price, qty)` exactly** — do NOT reuse S1's `macro_flow_threads_orderbook_and_counter_unchanged` fixture (its `Money(7_500)` lock is deliberately arbitrary because S1 never reads it; an inconsistent lock here would mask the floor-drift orphan §5.1 exists to catch). **Deferred:** feeding drained output into buckets. **The `drain_active_residual` flag lives in S3, not S2 — S2 has zero schedule interaction.**

### S3 — ACTIVE RESIDUAL → BUCKETS (the economic heart; single activating commit)

Re-grain the gate at `macro_flow.rs:107/122` into the two-source builder (§3.1): active buckets from drained residual orders (price = `last_settlement_price`), `intra_cleared=true`, `matched=0`, no self-edge in both `build_candidates` and `plan_flows` (§3.3). Wire the S2 drain into the per-edge scratch boundary (§2.2, §5.2) with the bid-refund wrapper + affordability bound (§5.1). Resolve the write-back authority (§5.4: auction price authoritative for active; accumulate `traded_qty`; recompute post-drain residual telemetry). Decide cadence latency (§5.3). Flip `drain_active_residual` default to TRUE here; once shipped the FALSE path is removed (not legacy cruft — it only decoupled the merge train). **S2-drain and S3-consume are ONE atomic economic change** (decoupled for *merge* via the flag, atomic in *behavior*) — this is the **non-isolatable piece**, correctly flagged: do not ship S3's gate-lift without the drain active. Tests: no-double-count (both-sided non-crossing active market — NOT seed-shaped); full-tick conservation; fault injection; write-back precedence; mixed-max_price refund; rewritten `active_to_dormant_handoff_conserves`; expire-still-has-work.

### S4 — RENDER PARITY + BROWSER SMOKE (frontend boundary; CLAUDE.md-mandated)

Confirm `FlowShipment` now spawns/arrives **AT** the observed m_a/m_b endpoints, not just transit. Adapt **`scripts/smoke-flow-traders.mjs`** (the closer template, not `smoke-7a.mjs`) and add the **new assertion**: subscribe to the chunk containing m_a or m_b, run past a `macro_flow_interval`, assert a `trader:` agent's **first** observed sample is AT the endpoint chunk (spawn) or **last** is AT it (arrival/despawn) — not a mid-route transit sample (resolves decomposition minor: the existing smoke asserts transit only and would be green on pre-existing behavior). Unit tests are explicitly insufficient (Phase-7a lesson). **Deferred:** nothing economic — pure render verification gate.

### S5 — RETIRE THE DEMO TRADER (clean delete; LAST, gated on S3+S4)

The §4.3 delete list. Drop the `traders` field — it loads silently on old saves (§4.4: the provider `migrate()` hook is NOT on the economy read path, so do not use it; rely on silent-ignore + a PR note, or add a real `postgres_economy::read` payload migration only if a loud drop is wanted); verify 8003's orphaned orders self-heal via owner-agnostic `expire_orders`. Re-home `trader_arbitrages`' transport-conservation coverage onto a flow test *before* deleting it. No `#[deprecated]` shim. Update `macro_flow_replays_across_restart` to not seed/expect `Traders`.

### Non-isolatable piece (flagged)

**S2-drain + S3-consume cannot both be behaviorally active across separate merges** without double-counting (re-read pool intent) or seeing-nothing (everything locked). They are decoupled for *merge* via the `drain_active_residual` flag (S1+S2 land dark, S3 activates), but the *economic* change is atomic. Do not ship S3's gate-lift without the drain in the same activating state.

---

## 7. Risks + Open Questions for the User

1. **Active-market inter-market latency (§5.3).** With `macro_flow_interval_ticks=10`, an observed market's surplus/deficit persists ~9 ticks before inter-market relief (active residual never TTL-expires, so it just waits for the flow tick). Is the 10-tick visible lag acceptable, or should the active drain path run more frequently (e.g. a shorter interval, or every-tick for active markets only)? This is a tuning/UX decision, not a correctness one.

2. **Price authority for active markets (§5.4).** Recommended: the auction's `last_settlement_price` is authoritative for active markets; the flow only accumulates `traded_qty` and recomputes post-drain residual telemetry, never overwriting price. Confirm this is the desired economic semantics (the alternative — flow price wins at the dst it imported to — would let inter-market transport costs visibly move the observed local price). This is the deferred #69 continuity hazard; the choice is a modeling commitment.

3. **Save-compatibility policy for S5 (§4.4).** Retirement is a schema break: pre-retirement saves carry an 8003 `Trader` blob and in-flight 8003 orders. **Note (audit P0):** the provider `migrate()` hook does NOT run on the economy read path (`postgres_economy.rs:120` does a strict `from_value`), so the field-drop is **silent by default** — old saves load and 8003's orphaned orders self-heal via owner-agnostic `expire_orders`. Recommended: accept the silent drop + document it in the PR; only add a real `postgres_economy::read` payload migration if a loud, code-visible drop is required. Confirm cross-version save compatibility is not required (project policy appears to allow breaks) — if it *is* required, S5 needs a richer migration that also cancels/releases 8003's open orders explicitly.

4. **Mean-field fidelity loss (§4.1).** The flow does not reproduce the demo Trader's real-bid price discovery against local counterparties or its per-trip knobs (`batch_qty`, `buy_premium_bps`, `sell_discount_bps`) — these have no flow analog and are silently lost on retirement. This is consistent with the SOTA mean-field direction (#69 already moved dormant markets to mean-field), but confirm no downstream telemetry/tuning depends on those knobs before S5 deletes them.

---

## 8. S3 code-level resolution (BINDING, 2026-06-02)

Before implementing S3 (the activating slice), a 5-agent design workflow (3 resolvers → adversarial conservation auditor → synthesis) resolved the code-level subtleties §3/§5 left open. Full resolution + verified file:line anchors + the 8-task decomposition + 10 must-have tests: **`docs/superpowers/specs/2026-06-02-auction-flow-s3-code-design.md`**. Binding decisions (refine the earlier sections):

- **DECISION-1 (supersedes §3.1 "group by owner"):** active buckets carry **one entry per residual `OrderId`**, not per owner. `MacroBucket` gains `intra_cleared: bool` + index-aligned active-only provenance `buyer_orders: Vec<OrderId>`, `buyer_max_prices: Vec<Money>`, `seller_orders: Vec<OrderId>` (empty for dormant). This is the *only* shape under which the per-row `max_price` affordability predicate and the per-`OrderId` drain agree (a scalar per-owner `max_price` would over/under-admit → `lock_cash` fault / lost trade). `MacroBucket` stays unpersisted (no serde) → no snapshot change.
- **DECISION-2:** the Stage-2 affordability prune's `q'` (post-drop traded qty) threads through the ask-drain, seller prorata, write-back `eff_*`, and the `FlowShipment` qty. `q'==0` ⇒ skip the edge.
- **Affordability is two-stage:** Stage-1 (build) admits an active bid only if `max_price >= prior_price` (local discovered price); Stage-2 (per-edge, `prune_unaffordable_buyers`) is a deterministic fixpoint "drop-all-failing-then-recompute" against the exact `value(p_dst,q)+transport` charge, so `settle_flow.lock_cash` can never fault. The bid refund is **implicit** (drain releases at `max_price`, settle charges less, surplus stays available — no re-deposit).
- **Price-authority (user decision a):** `write_back` gains `preserve_price: bool`; for active endpoints it does NOT overwrite `last_settlement_price` (auction authoritative), but still accumulates `traded_qty` and overwrites `unmet/unsold` with the post-drain residual.
- **Self-edge suppression** lives in BOTH `build_candidates` (carry `intra_cleared` in the `by_good` tuple, gate self-edge on `!intra_cleared`) and `plan_flows` (force `matched=0` for active buckets).
- **Cadence (user decision b):** the 10-tick interval is accepted (active residual is auction-served every tick, flow-served every 10th); no code change.