# Economy — Trader Agents v0 Design

Date: 2026-05-30

## Status

Next economy roadmap slice (deferred slice 3). Aggregate **traders** that move a
good between two markets, buying at the source and selling at the destination via
the existing auction, paying transport cost (slice 2). Backend-only,
deterministic. **Abstract travel** (a tick countdown) — NOT a materialized
mobility agent; visual materialization is roadmap slice 4 (Economy LOD: "active
chunks materialize traders"). This keeps the economic loop tractable and fully
conservation-preserving.

## Goal

A deterministic trader that cycles: buy `batch_qty` of a good at its source
market (bid into the auction) → travel to the destination (paying transport cost
over `travel_ticks`) → sell at the destination (ask into the auction) → return →
repeat. Spatial arbitrage with transport cost. Money stays conserved; goods stay
conserved (the trader only relocates what the auction moves to/from it).

## Architecture

### `AccountBook::transfer` (new, accounts.rs)
```rust
pub fn transfer(&mut self, from, to, amount: Money) -> Result<(), EconomyError> {
    if amount.0 < 0 { return Err(EconomyError::NegativeMoney); }
    let mut f = self.account(from);
    if f.available < amount { return Err(EconomyError::InsufficientFunds); }
    f.available = f.available.checked_sub(amount)?;
    let mut t = self.account(to);
    t.available = t.available.checked_add(amount)?;
    self.accounts.insert(from, f);
    self.accounts.insert(to, t);
    Ok(())
}
```
Conserves total money (debit one available, credit another). Used to pay
transport cost to the operator account so money is never destroyed.

### Transport operator
A reserved actor: `pub const TRANSPORT_OPERATOR: EconomicActorId = EconomicActorId(u64::MAX);`
receives all transport-cost payments. Conservation tests sum trader + operator +
counterparties.

### New module `economy/traders.rs`
```rust
pub enum TraderState {
    Buying { order: Option<OrderId> },   // at source market
    ToDest { remaining: u64 },
    Selling { order: Option<OrderId> },  // at destination market
    ToSource { remaining: u64 },
}
pub struct Trader {
    pub actor: EconomicActorId,
    pub good: GoodId,
    pub source: MarketId,
    pub dest: MarketId,
    pub distance_tiles: i64,            // precomputed source↔dest distance (no Graph dep at runtime)
    pub batch_qty: Quantity,
    pub buy_premium_bps: i32,           // max_price = source ref price * (1 + premium)
    pub sell_discount_bps: i32,         // min_price = dest ref price * (1 - discount)
    pub order_ttl_ticks: u64,
    pub state: TraderState,
}
```

`distance_tiles` is computed once when the trader is created (a seeder/caller may
use `transport::manhattan_tiles(graph, source_node, dest_node)`), so the per-tick
trader system needs **no `Res<Graph>`** — keeping it decoupled from routing
(economy unit tests run without a graph). Transport cost uses
`transport_cost(distance_tiles, batch_qty, rate)` directly.
pub struct Traders(pub BTreeMap<EconomicActorId, Trader>);  // Resource
```

### `run_traders_at_tick(accounts, inventory, orders, ledger, dirty, next, market_goods, graph, traders, config, current_tick)`
Per trader (BTreeMap order = sorted by actor), match on state:
- **Buying { order }**:
  - If the trader already holds `batch_qty` of the good (available ≥ batch_qty): the
    bid filled → pay transport cost (`transport_cost(distance_tiles, batch_qty,
    config.transport_cost_per_tile_unit)`; `accounts.transfer(actor, TRANSPORT_OPERATOR,
    cost)`; push `EconomyEvent::TransportPaid { actor, amount: cost }`),
    travel_ticks = `transport_ticks(distance_tiles, config)`, set `ToDest { remaining: travel_ticks }`.
  - else if `order` is `None` OR the stored order is no longer in `orders.bids`
    (filled-or-expired but goods not yet ≥ batch — partial/none): place a fresh bid
    `create_bid(... batch_qty, max_price = ref_price_with_premium, order_ttl_ticks)`,
    store its `OrderId`. (ref price = `market_goods` source last_settlement_price,
    fallback to a config default if the market never traded.)
  - else: waiting for the outstanding bid to fill — do nothing.
- **ToDest { remaining }**: `remaining -= 1`; at 0 → `Selling { order: None }`.
- **Selling { order }**: symmetric — if the trader no longer holds the good
  (available == 0, sold): set `ToSource { remaining: travel_ticks }`. else place/keep
  an ask (`create_ask(... batch_qty, min_price = dest ref * (1 - discount), ttl)`).
- **ToSource { remaining }**: `remaining -= 1`; at 0 → `Buying { order: None }`.

`transport_ticks(distance_tiles, config)` = `max(1, distance_tiles as u64 / config.trader_tiles_per_tick)` (a configured travel speed; ≥1 tick). Deterministic integer.

### Schedule (systems.rs)
New `EconomySet::Traders` between `Production` and `GeneratePoolOrders`
(so trader bids/asks are in the book before `ClearMarkets`):
`ExpireOrders -> Production -> Traders -> GeneratePoolOrders -> ClearMarkets -> Telemetry`.
`run_traders_system` is a normal `Res`/`ResMut` system (NO `Res<Graph>` — distance
is precomputed on each trader). `EconomyPlugin` inserts `Traders::default()`.

### Config (EconomyConfig)
Add `trader_tiles_per_tick: u64` (default e.g. 4) and `trader_default_ref_price: Money`
(used when a market has no settlement price yet, default e.g. `Money(1_000)`).

### Ledger
Add `EconomyEvent::TransportPaid { actor: EconomicActorId, amount: Money }`.

## Conservation / invariants
- **Money conserved** (incl. `TRANSPORT_OPERATOR`): auction trades conserve;
  transport cost is a `transfer` (not a sink). Test sums all accounts before/after.
- **Goods conserved**: the trader only gains/loses goods through auction fills.
- Deterministic: `BTreeMap`-keyed traders, integer travel ticks, reference prices
  from `MarketGoodState`, no rng/wall-clock/float.

## Testing
- `AccountBook::transfer` conserves total; rejects over-transfer / negative.
- **Trader cycle (integration, multi-tick schedule):** one trader (source M1, dest M2,
  good TOOLS), a source supply pool selling TOOLS cheap at M1, a dest demand pool
  buying TOOLS dear at M2, trader seeded with cash. Run the schedule N ticks; assert:
  the trader ends a cycle having bought at M1 + sold at M2; `TRANSPORT_OPERATOR`
  received > 0; total money (all accounts) conserved across the run; total TOOLS
  conserved.
- `trader_pays_transport_to_operator` (operator balance rises by the exact
  `transport_cost_between`).
- `trader_state_machine_advances` (Buying→ToDest→Selling→ToSource transitions on the
  right conditions; travel timer counts down).
- `traders_are_deterministic` (two identical worlds → identical ledger).
- End-to-end via `schedule.run`; wiring (`EconomyPlugin` installs `Traders`).
- Full gate; economy-v0/production/transport tests unaffected.

## What this is NOT
- No materialized mobility agent (abstract travel) — slice 4 materializes traders.
- No multi-good traders, no route optimization, no dynamic source/dest choice
  (fixed route per trader), no inventory location-tagging (the books are
  location-agnostic in v0). No partial-batch handling (bid is all-or-keep-waiting).

## Open questions (resolve in planning)
1. `EconomicActorId(u64::MAX)` as the operator sentinel — confirm no collision with
   seeded actor ids (they start at small ints; `u64::MAX` is safe).
2. Ref-price source: `MarketGoods` `last_settlement_price`; fallback
   `config.trader_default_ref_price` when the key is absent / price is `ZERO`.
3. `run_traders_system` params: `Res<Tick>`, `Res<EconomyConfig>`, `ResMut` of
   accounts/inventory/orders/ledger/dirty/next/market_goods/traders — ~10, under
   bevy's 16. `#[allow(clippy::too_many_arguments)]` on the `run_traders_at_tick` helper.
4. Confirm `EconomyError::NegativeMoney` exists (used by `transfer`); else use the
   present negative guard (`NegativeQuantity` is goods-only — add `NegativeMoney`
   if absent, or reuse `InvalidOrder`).
