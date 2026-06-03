# Economy Consumption — the Demand-Side Sink (design)

**Status:** approved in brainstorming (2026-06-02). The long-anticipated **consumption slice** (economy-v0 §:485-486/649: "Later production and consumption slices will add explicit `Produced` and `Consumed` events" — production shipped, consumption never did). Resolved via a 6-agent ground→design→adversarial→synthesize workflow; every file:line below was read against `origin/main` (`0ac8da7`), not assumed.

## 1. Why + the economic model

Today the economy loop is **open**: production creates goods → supply/demand pools generate orders → the auction (intra) + macro flow (inter) deliver goods into buyer `available` inventories → **nothing consumes them**. Buyers accumulate forever (the bid is **cash-capped**, not goods-held-capped — `pools.rs:85-87` vs the supply-side held-cap `:124`), so the world fills up and trade freezes once cash drains. The visible shoppers (`shoppers.rs`) are a pure render projection of `unmet_demand_last_tick` with **no economic effect**.

**The fix is a consumption sink:** consumers consume the goods delivered to them, at the demand rate, so the loop reaches a flowing steady-state (`production ≈ delivery ≈ consumption`). This is the SOTA demand-side completion that mirrors production (the source).

**Architecture (mirrors macro flow #69 = aggregate authority + flow-traders #70 = render projection):** consumption is an **aggregate authority that runs for ALL markets, observed or not** — NOT driven by the observed shopper, or the economy would depend on the viewport (the anti-pattern #69 fixed). The shopper (Slice 2) is the visible **projection** of consumption.

### 1.1 The new conservation invariant (exact)

Goods are **no longer invariant**. Per good `g`, across any interval `(t0, t1]`:

```
total_good(g)_t1  ==  total_good(g)_t0  +  Σ Produced(g)  −  Σ (Consumed(g) + FinalConsumed(g))
```

`total_good(g)` = `InventoryBook::total_good(g)` (`inventory.rs:109-118`, sums `available + locked`); the sums range over ledger events in `(t0, t1]`. **Conservation is auditable from the ledger, not violated.** **Money is exactly invariant** — the sink calls only `inventory.consume` (mutates `available`, never `AccountBook`); the consumer already paid on delivery via its bid.

### 1.2 Decisions (baked)

- **Rate = full `desired_qty_per_tick`** (`min(held, desired)`): sink-rate == demand-rate → steady-state, no permanent inventory residue. (vs a fraction, which re-freezes mildly.)
- **Cadence = per pool `interval_ticks`** (gate on a new cursor): symmetric to production/order-gen, deterministic, never outruns interval-gated supply.
- **Split a NEW `EconomyEvent::FinalConsumed` variant** for end-consumption, distinct from production's recipe-input `Consumed`: economically correct (intermediate vs final consumption is a real macro distinction), clean for the audit query (#68), and no-cruft-compliant (a new variant, not a serde-default field). Forward-compatible; covered by the required snapshot DELETE (§4).

## 2. Slice 1 — the aggregate consumption authority

### 2.1 Resource shape — reuse `DemandPool` + a separate cursor

Add **one field** to `DemandPool` (`pools.rs:12-22`), after `last_generated_tick`:

```rust
pub last_consumed_tick: Option<u64>,
```

Reuse `DemandPool` (not a parallel `ConsumptionPool`): consumer/market/good/`desired_qty_per_tick` already live there; want and consume stay parametrically coupled (same rate → steady state); it already round-trips in `EconomyPersistSnapshot.demand_pools` (`persist.rs:35/81/107`) → the cursor persists for **free**. `DemandPool` is `Copy` and `Option<u64>: Copy`, so the derive (`pools.rs:11`) and the `*v` extract (`persist.rs:81`) keep compiling.

**The cursor MUST be a separate `last_consumed_tick`** — never share `last_generated_tick` (written every interval by `generate_pool_orders_at_tick` `pools.rs:111` and gating bids `:82`; sharing it is a cadence-corruption bug).

### 2.2 The consume core — `run_consumption_at_tick` (in `pools.rs`)

Pure fn mirroring `run_production_at_tick` (`production.rs:27-68`):

```rust
pub fn run_consumption_at_tick(
    inventory: &mut InventoryBook,
    ledger: &mut TradeLedger,
    demand: &mut DemandPools,
    current_tick: u64,
) -> Result<(), EconomyError> {
    let actors: Vec<EconomicActorId> = demand.0.keys().copied().collect();
    for actor in actors {
        let pool = demand.0[&actor]; // DemandPool is Copy
        if !interval_elapsed(pool.last_consumed_tick, current_tick, pool.interval_ticks) {
            continue;
        }
        let available = inventory.balance(actor, pool.good).available;
        let qty = Quantity(pool.desired_qty_per_tick.0.min(available.0)); // clamp BEFORE consume
        if qty.0 > 0 {
            inventory.consume(actor, pool.good, qty)?; // qty <= available ⇒ never faults
            ledger.0.push(EconomyEvent::FinalConsumed { actor, good: pool.good, qty });
        }
        if let Some(p) = demand.0.get_mut(&actor) {
            p.last_consumed_tick = Some(current_tick);
        }
    }
    Ok(())
}
```

Properties (each tied to code):
- **`qty = min(held, desired)`** clamps so `qty ≤ available` (`inventory.rs:101` guard never fires) and `qty ≥ 0` (`:97` never fires) → the `?` is dead-but-typed-for-symmetry; the sink **cannot fault**, no scratch/rollback boundary needed (production's in-place pattern, not macro_flow's clone-validate-apply).
- **Consume from `available` only** — consumers hold zero `locked` goods (the bid locks *cash*; deliveries land in `available` at `auction.rs:402` / `macro_flow.rs:665`).
- **`if qty.0 > 0` skip** — no zero-qty events.
- **Interval gate on the new cursor** via `interval_elapsed` (`pools.rs:44`, `saturating_sub`) → exactly elapsed-whole-intervals on resume (frozen-time, no offline catch-up).
- **Determinism** — keys-first `Vec` collect then iterate (`production.rs:33`); `Copy` read (no clone); BTreeMap key order + integer `min`, no RNG/float → byte-identical event sequence.
- **NOT gated on `DormantMarkets`** (deliberate divergence from `generate_pool_orders` `:79`): the sink runs for **all** consumers regardless of observation. A dormant market's bids paused → its inventory stops topping up → the sink drains on-hand to ~0 then no-ops (clamp to 0). Viewport-independent by construction.

### 2.3 `FinalConsumed` event

Add to the `EconomyEvent` enum (`ledger.rs`, beside `Consumed { actor, good, qty }` at `:52-56`):

```rust
FinalConsumed { actor: EconomicActorId, good: GoodId, qty: Quantity },
```

`event_type()` (`ledger.rs:96`) returns `"final_consumed"`. Flows to the audit store (#68) like every variant via the existing `ledger.0.extend`/drain path. **Conservation equation §1.1 counts both `Consumed` and `FinalConsumed` as goods-removals.**

### 2.4 System wrapper + schedule placement

New `EconomySet::Consume` between `MacroFlow` and `ShopperCapture` (`systems.rs:17-28` enum + the `.chain()` tuple `:70-83`):

```rust
RefreshLod, ExpireOrders, Production, GeneratePoolOrders,
ClearMarkets, MacroFlow, Consume, ShopperCapture, Materialize, Telemetry,
```

The slot is uniquely correct: **after** both delivery paths (`ClearMarkets` deposit `auction.rs:402` + `MacroFlow` deposit `macro_flow.rs:665`) so the interval's goods land before the sink drains; **before** next tick's `GeneratePoolOrders` (automatic — the chain is `.before(tick_increment_system)` `:102`).

Plain parallel `ResMut` system (NOT exclusive — touches no `World`/Graph), mirroring `run_production_system` (`systems.rs:308-315`), in the same parallel `add_systems` tuple (`:92-103`):

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

Add `run_consumption_at_tick` to the `use crate::economy::{…}` import at `systems.rs:6-12`. **Do NOT add a `pub use`** — `mod.rs:37` is `pub use pools::*` (glob), so the fn is already re-exported. The chain serializes `InventoryBook`/`TradeLedger`/`DemandPools` access (macro_flow takes `DemandPools` as `Res`, order-gen as `ResMut`, both chain-ordered) → no race, no parallelism loss.

### 2.5 Loop stability + the macro-flow cadence note

Symmetric seed (want=offer, interval=1): deposit → drain `min(want,held)` → `available` oscillates in `[0, want]`; re-bids next interval (cash permitting). **Bounded steady-state flux; no runaway/freeze/negative stock** (clamp guarantees `qty ≤ available`). `want > offer` → supply-limited (held → ~0); `want < offer` → supply backs up at the seller (ask caps by on-hand `:125`), price falls via EWMA. All bounded.

**Macro-flow cadence mismatch (correct behaviour, not a bug):** seed pools use `interval_ticks: 1` but `macro_flow_interval_ticks` defaults to **10** (`systems.rs:59`). A flow-fed consumer drains every tick while the inter-market flow delivers every 10 → **sawtooth inventory** (drain to ~0 over 9 ticks, then a delivery burst). Bounded/deterministic; Slice-2 foot-traffic for flow markets is correspondingly bursty.

## 3. Slice 2 — re-point the shopper projection (follow-on)

Today `capture_shopper_visits` reads `unmet_demand_last_tick` (`shoppers.rs:92`, UNSERVED). For economically-real shoppers projecting **consumption** (goods being used), flip the visible meaning to consumed-qty. The shopper stays a pure projection; consumption is the authority.

- **Add `consumed_qty_last_tick: Quantity` to `MarketGoodState`** (`market.rs:23-33` struct + `new()` `:39-50`, init `Quantity::ZERO`). `MarketGoodState: PartialEq` (`:23`), so this field becomes load-bearing for `macro_flow_replays_across_restart`'s `MarketGoods.0 ==` assert (the §5 determinism guard).
- **Attribute consumption to the market in the sink:** Slice 2 gives `run_consumption_at_tick` a `&mut MarketGoods` (the system gains `ResMut<MarketGoods>`); `DemandPool.market` (`pools.rs:14`) is the key. After consuming: `state.consumed_qty_last_tick = state.consumed_qty_last_tick.checked_add(qty)?`.
- **RESET semantics (critical):** zero `consumed_qty_last_tick` for **every present `MarketGoodState`** at the top of the pass, then `+=` per pool — the sink is the field's **sole writer** (a per-touched-key reset leaks phantom consumption). Rides the already-persisted `market_goods` (no new persist wiring).
- **Re-point** `capture_shopper_visits` to read `consumed_qty_last_tick` instead of `unmet_demand_last_tick`; the browser smoke (`smoke-shoppers.mjs`) updates to the new visible meaning.

## 4. Persistence & deploy

- `last_consumed_tick` rides `demand_pools` (`persist.rs:35/81/107`); `consumed_qty_last_tick` rides `market_goods` (`:34`) — **zero new persist wiring**. The cursor MUST persist (else a restart replays the catch-up gap — the `LastProcessedMonth` bug class); persisted cursor + `saturating_sub` → exactly elapsed-whole-intervals on resume.
- Both new fields + the `FinalConsumed` variant are non-defaultable additions to persisted structs/enums (`persist.rs` has **no `#[serde(default)]`**; no-cruft → no serde shim). **Required one-time deploy step: `DELETE FROM economy_snapshots` once before deploying** (matching #69's `market_distances` protocol; re-seeds). CI/e2e use a fresh DB.

## 5. Test plan

**Confirmed CI-reds (convert, do not file under "unaffected"):**
1. `tests/lod.rs::active_to_dormant_handoff_conserves` (`:431`) — asserts raw `total_good` invariant every tick (`:526-533/552-553`) with a seeded `DemandPool`; once the sink lands the consumer's FOOD drains. **Fix:** assert the ledger-derived invariant `Δtotal_good(g) == ΣProduced − Σ(Consumed+FinalConsumed)` (keep the money-conservation assert).
2. `tests/systems.rs::economy_clears_a_trade_end_to_end` (`:56`) — tick-0 deposit then same-tick consume (cursor `None` → elapsed) drains to 0, but `:97-103` asserts `== Quantity(1_000)`. **Fix:** assert pre-consume inventory or the post-consume value. Audit `dirty_market_keys_are_processed_in_stable_order` (`:115`, same seed).

**Compile-break fan-out** (every `DemandPool { … }` literal needs `last_consumed_tick: None`): production code `seed.rs:120/160/253`; tests `flow_shipments.rs:61/211`, `systems.rs:29`, `persist.rs:109`, `macro_flow.rs:113/1080/1259/1273/1415/2450`, `pools.rs:24/119`, `lod.rs:56/292/480` (fix helper constructors once). Slice 2: every `MarketGoodState { … }` literal needs `consumed_qty_last_tick: Quantity::ZERO` (`conservation.rs:8`, `systems.rs:161`).

**Determinism guard:** `tests/macro_flow.rs::macro_flow_replays_across_restart` (`:2413`) — Slice 1 must stay green (sink mutates only `InventoryBook` + pushes events; cursor persists → identical both continuations; the ledger tail is now `FinalConsumed`-dominated — re-run, don't assume). Slice 2: `consumed_qty_last_tick` enters the `MarketGoods.0 ==` compare (`:2525`) → the guard for the §3 reset semantics.

**New consume-invariant suite** (mirror `tests/production.rs`: money-invariance + per-actor deltas + event + determinism, NOT global total_good): (a) money unchanged; (b) `total_good(g)_after == before − consumed`; (c) a clamped `FinalConsumed{actor,good,qty}` is pushed; (d) `min(held, want)` clamp with `held < want` → consumed `== held`, no fault; (e) determinism (two runs → identical ledger); (f) cursor persistence round-trip (no double-consume after resume); (g) ledger-derived global invariant; (h) off-screen/dormant market still consumes. `audit.rs` (~`:28`): `FinalConsumed → "final_consumed"` tag assertion.

## 6. Scope & deferred

**In scope:** Slice 1 (the aggregate sink — independently shippable) + Slice 2 (shopper re-pointing).

**Deferred (flagged, NOT this slice):**
- **Income/wage source** (the next economic-realism gap): consumers hold finite seeded cash (`Money(1_000_000)`); the sink un-freezes the loop, but cash drain to sellers becomes the long-run floor (`InsufficientFunds`). A truly closed loop needs income feeding consumer accounts. The sink itself never causes runaway/freeze.
- **Bid-side held-cap** (`cap want by max(0, target_stock − held)`): not required (the sink bounds the stockpile); evaluate as a follow-on.
- **Per-good consumption rates / utility tracking** beyond the flat `desired_qty` rate (YAGNI).
