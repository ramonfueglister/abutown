# Economy v0 — Deterministic Market Core Design

Date: 2026-05-30

## Status

Approved direction from architecture review. This is the first narrow slice of
the separate economy/game-mechanics roadmap called out by the million-agent
roadmap and the world-unification foundation. It intentionally builds only the
deterministic market core needed before production chains, traders, transport
costs, or economy LOD can be added safely.

## Goal

Add a small, isolated `sim-core::economy` subsystem that can create local
market orders from aggregate demand/supply pools, reserve cash and goods, clear
dirty local call auctions deterministically, emit a trade ledger, and prove by
tests that trades conserve money and goods.

## Spec-Conformance

- The million-agent roadmap lists economy, ledger, production, and combat as a
  separate game-mechanics roadmap. This spec starts that roadmap without
  changing mobility scale targets or frontend replication.
- The world-unification foundation establishes `SimPlugin` as the extension
  boundary. Economy v0 ships as `EconomyPlugin` in `sim-core`, installed beside
  `TimePlugin`, `RoutingPlugin`, `MobilityPlugin`, and `PopulationPlugin`.
- `sim-server` remains a runtime/API/persistence host. Economy rules, books,
  orders, clearing, and tests live in `backend/crates/sim-core/src/economy/`.
- No `bevy_app` dependency is introduced. The plugin implements the existing
  local `SimPlugin { name, install }` trait.
- No new Bevy/pathfinding/spatial plugins are introduced. Economy v0 does not
  need routing yet; later transport-cost slices will use the existing routing
  graph, flow fields, and spatial index.

## Non-Goals

Economy v0 does not include:

- moving trader agents;
- producers or input/output production chains;
- routing costs, risk costs, or physical delivery;
- LOD materialization/dematerialization;
- persistent economy snapshots;
- server API or wire protocol changes;
- pro-rata rationing;
- permanent order books;
- LLM/RL agents;
- a large goods catalog.

These are later additive slices after the market core has hard invariants.

## Module Layout

Create:

```text
backend/crates/sim-core/src/economy/
    mod.rs
    ids.rs
    money.rs
    goods.rs
    accounts.rs
    inventory.rs
    market.rs
    orders.rs
    pools.rs
    auction.rs
    ledger.rs
    systems.rs
    tests/
        mod.rs
        auction.rs
        conservation.rs
        determinism.rs
        expiry.rs
        locking.rs
        overflow.rs
```

Modify:

```text
backend/crates/sim-core/src/lib.rs
backend/crates/sim-server/src/runtime/mod.rs
```

`lib.rs` exports `pub mod economy;`. Runtime construction installs
`sim_core::economy::EconomyPlugin` after `MobilityPlugin` and before population
and persistence registration. Economy v0 does not read movement state, but it
uses the current `crate::mobility::resources::Tick` resource as the simulation
tick source until the repo has one unified world tick.

## Core Types

Stable IDs:

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct GoodId(pub u16);

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct MarketId(pub u32);

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct OrderId(pub u64);

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct EconomicActorId(pub u64);
```

Economy actors use stable `EconomicActorId`, not ECS `Entity`, because orders,
ledger rows, tests, and later persistence need IDs that survive schedule runs
and are easy to debug.

Money and quantities use fixed-point integer wrappers:

```rust
pub const ECONOMY_SCALE: i128 = 1_000;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Money(pub i64);

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Quantity(pub i64);
```

Scale convention:

- `Money(1_000)` is one currency unit.
- `Quantity(1_000)` is one physical unit.
- A price is `Money` per one `Quantity(1_000)` unit.
- Order value is always `price * quantity / ECONOMY_SCALE`.

No floats are used for cash, prices, goods quantities, or EWMA telemetry.
Critical arithmetic is checked and uses `i128` intermediates for
`Money * Quantity / ECONOMY_SCALE`. Settlement logic returns explicit errors on
overflow or negative values; it does not use saturating arithmetic.

```rust
pub enum EconomyError {
    Overflow,
    NegativeMoney,
    NegativeQuantity,
    ZeroPrice,
    InsufficientFunds,
    InsufficientGoods,
    InvalidOrder,
}
```

The initial goods catalog is intentionally tiny:

```rust
pub const GOOD_FOOD: GoodId = GoodId(1);
pub const GOOD_WOOD: GoodId = GoodId(2);
pub const GOOD_IRON: GoodId = GoodId(3);
pub const GOOD_TOOLS: GoodId = GoodId(4);
```

Stone can be added in a later production slice if it earns its place.

## Books

`AccountBook` is the only source of truth for cash:

```rust
pub struct MoneyAccount {
    pub available: Money,
    pub locked: Money,
}

pub struct AccountBook {
    pub accounts: BTreeMap<EconomicActorId, MoneyAccount>,
}
```

`InventoryBook` is the only source of truth for goods:

```rust
pub struct InventoryBalance {
    pub available: Quantity,
    pub locked: Quantity,
}

pub struct InventoryBook {
    pub balances: BTreeMap<(EconomicActorId, GoodId), InventoryBalance>,
}
```

Demand and supply pools describe intent only. They do not own independent budget
or stock fields. All available and locked balances live in the books so
conservation tests can sum one authoritative state.

## Markets

Markets are local hubs attached to the existing routing graph:

```rust
pub struct MarketSite {
    pub id: MarketId,
    pub node_id: crate::routing::NodeId,
    pub name: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct MarketGoodKey {
    pub market: MarketId,
    pub good: GoodId,
}

pub struct MarketGoodState {
    pub key: MarketGoodKey,
    pub last_settlement_price: Money,
    pub ewma_reference_price: Money,
    pub traded_qty_last_tick: Quantity,
    pub unmet_demand_last_tick: Quantity,
    pub unsold_supply_last_tick: Quantity,
    pub dirty: bool,
    pub last_cleared_tick: u64,
}
```

`last_settlement_price` is the last actual trade price. `ewma_reference_price`
is telemetry/belief only and must never be used as the settlement price for a
trade. Actual settlement must always stay inside the executable bid/ask bounds.

Every `MarketGoodState` is created with an explicit positive
`last_settlement_price` and `ewma_reference_price` from scenario seed data or a
test fixture. There is no implicit zero-price default. `MarketSite.node_id` is
stored for later transport-cost work; v0 systems do not dereference it or
require a non-empty routing graph.

## Orders

Orders are stored in `OrderBook`, keyed and iterated deterministically:

```rust
pub struct Bid {
    pub id: OrderId,
    pub owner: EconomicActorId,
    pub market: MarketId,
    pub good: GoodId,
    pub qty_remaining: Quantity,
    pub max_price: Money,
    pub cash_locked_remaining: Money,
    pub created_tick: u64,
    pub expires_tick: u64,
}

pub struct Ask {
    pub id: OrderId,
    pub owner: EconomicActorId,
    pub market: MarketId,
    pub good: GoodId,
    pub qty_remaining: Quantity,
    pub min_price: Money,
    pub goods_locked_remaining: Quantity,
    pub created_tick: u64,
    pub expires_tick: u64,
}
```

Order IDs come from a `NextOrderId` resource. No UUIDs, wall-clock time, or
randomness are used for order identity.

`created_tick`, `expires_tick`, and the current tick used by systems come from
the existing simulation tick resource, not from wall-clock time.

Bid creation:

1. Computes `max_price * qty / ECONOMY_SCALE` with checked arithmetic.
2. Moves that cash from buyer `available` to `locked`.
3. Inserts the bid.
4. Marks `MarketGoodKey` dirty.
5. Emits `OrderCreated` and `CashLocked`.

Ask creation:

1. Moves `qty` from seller inventory `available` to `locked`.
2. Inserts the ask.
3. Marks `MarketGoodKey` dirty.
4. Emits `OrderCreated` and `GoodsLocked`.

Order expiry releases only the remaining locked cash/goods, removes the order,
marks the market-good dirty, and emits release events.

## Pools

Pools generate aggregate orders in v0:

```rust
pub struct DemandPool {
    pub actor: EconomicActorId,
    pub market: MarketId,
    pub good: GoodId,
    pub desired_qty_per_tick: Quantity,
    pub max_price: Money,
    pub urgency_bps: i32,
    pub elasticity_bps: i32,
    pub interval_ticks: u64,
    pub last_generated_tick: Option<u64>,
}

pub struct SupplyPool {
    pub actor: EconomicActorId,
    pub market: MarketId,
    pub good: GoodId,
    pub offered_qty_per_tick: Quantity,
    pub min_price: Money,
    pub interval_ticks: u64,
    pub last_generated_tick: Option<u64>,
}
```

`interval_ticks` and `last_generated_tick` prevent pools from creating new
orders every schedule run. The account and inventory books decide how much a
pool can afford or stock at generation time.

Demand generation computes the affordable quantity as:

```text
available_cash * ECONOMY_SCALE / max_price
```

and creates an order for `min(desired_qty_per_tick, affordable_qty)`. If the
result is zero, no order is created and the ledger records `OrderRejected`.
Supply generation creates an order for
`min(offered_qty_per_tick, available_inventory)`. If the result is zero, no
order is created and the ledger records `OrderRejected`.

For v0, urgency and elasticity are stored but not read by the generator. They
reserve stable fields for later demand adjustment without changing the public
pool type immediately.

## Auction And Settlement

Only dirty `MarketGoodKey`s are cleared.

Dirty keys are gathered into a `Vec`, sorted by `(market, good)`, then processed.
The implementation must not rely on `HashMap` iteration order for auction
semantics.

Bids sort by:

1. `max_price` descending;
2. `created_tick` ascending;
3. `OrderId` ascending.

Asks sort by:

1. `min_price` ascending;
2. `created_tick` ascending;
3. `OrderId` ascending.

Clearing is a two-phase batch for one `MarketGoodKey`:

1. Match sorted bids and asks into deterministic fills while the best bid
   overlaps the best ask.
2. Record the final matched `marginal_bid` and `marginal_ask`.
3. Compute one uniform settlement price for the whole batch.
4. Pre-validate all money/quantity arithmetic for the batch.
5. Apply all fills at the uniform settlement price.

The match loop advances to the next bid or ask whenever the current order's
`qty_remaining` reaches zero. `last_settlement_price` is not updated during the
loop; it is read once before clearing and written once after the batch succeeds.

Matching shape:

```rust
while best_bid.max_price >= best_ask.min_price {
    let qty = min(best_bid.qty_remaining, best_ask.qty_remaining);
    fills.push(Fill { bid: best_bid.id, ask: best_ask.id, qty });
    marginal_bid = Some(best_bid.max_price);
    marginal_ask = Some(best_ask.min_price);
    advance_exhausted_orders();
}
```

v0 settlement policy:

```rust
fn settlement_price(last: Money, marginal_bid: Money, marginal_ask: Money) -> Money {
    debug_assert!(marginal_bid.0 >= marginal_ask.0);
    if last.0 < marginal_ask.0 {
        marginal_ask
    } else if last.0 > marginal_bid.0 {
        marginal_bid
    } else {
        last
    }
}
```

This keeps prices stable when possible while guaranteeing the uniform trade
price is valid for every matched order because all matched asks are at or below
the marginal ask and all matched bids are at or above the marginal bid. Midpoint
settlement and pro-rata rationing are deferred policy variants.

If no fills match, no trade is executed, `last_settlement_price` stays unchanged,
and telemetry records unmet demand / unsold supply from remaining orders.

## Trade Execution

For each filled quantity `q`:

1. Compute `locked_for_q = bid.max_price * q / ECONOMY_SCALE`.
2. Compute `actual_cost = settlement_price * q / ECONOMY_SCALE`.
3. Compute `refund = locked_for_q - actual_cost`.
4. Buyer cash: `locked -= locked_for_q`, `available += refund`.
5. Seller cash: `available += actual_cost`.
6. Seller inventory: `locked_goods -= q`.
7. Buyer inventory: `available_goods += q`.
8. Reduce `qty_remaining` on both orders.
9. Reduce `cash_locked_remaining` by `locked_for_q`.
10. Reduce `goods_locked_remaining` by `q`.
11. Remove filled orders after all fills are applied; keep partially filled
    orders with their remaining quantities and locks.
12. Emit `Trade` and `CashReleased` for refund when nonzero.

This rule is deliberately simple for partial fills: each filled subquantity
releases the surplus between the bid's max-price lock and the actual settlement
cost for that subquantity.

## Ledger

`TradeLedger` records economy events for tests and debugging:

```rust
pub enum EconomyEvent {
    OrderCreated {
        order: OrderId,
        actor: EconomicActorId,
        market: MarketId,
        good: GoodId,
    },
    OrderExpired {
        order: OrderId,
        actor: EconomicActorId,
        market: MarketId,
        good: GoodId,
    },
    Trade {
        market: MarketId,
        good: GoodId,
        buyer: EconomicActorId,
        seller: EconomicActorId,
        qty: Quantity,
        price: Money,
        total: Money,
    },
    CashLocked {
        actor: EconomicActorId,
        amount: Money,
    },
    CashReleased {
        actor: EconomicActorId,
        amount: Money,
    },
    GoodsLocked {
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    },
    GoodsReleased {
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    },
    OrderRejected {
        actor: EconomicActorId,
        market: MarketId,
        good: GoodId,
        reason: EconomyError,
    },
    MarketClearFailed {
        market: MarketId,
        good: GoodId,
        reason: EconomyError,
    },
}
```

Later production and consumption slices will add explicit `Produced` and
`Consumed` events. In v0, trades must conserve both money and goods.

System-level errors do not panic in normal runtime. A failed pool order leaves
books and orders unchanged, emits `OrderRejected`, and the system continues. A
failed market clear pre-validation leaves that market-good's books and orders
unchanged, emits `MarketClearFailed`, clears the dirty flag for that run, and
continues to the next sorted dirty key. Tests cover the error path directly.

## Systems And Schedule

`EconomyPlugin` registers its own system set chain:

```rust
#[derive(SystemSet, Hash, Eq, PartialEq, Debug, Clone)]
pub enum EconomySet {
    ExpireOrders,
    GeneratePoolOrders,
    ClearMarkets,
    Telemetry,
}
```

Order:

```text
ExpireOrders -> GeneratePoolOrders -> ClearMarkets -> Telemetry
```

The plugin uses the same Bevy schedule APIs already used by `CorePlugin`:
`schedule.configure_sets((...).chain())` for the internal economy order and
`.before(crate::mobility::systems::tick_increment_system)` so every economy
system reads the pre-increment `Tick` for the current schedule run. Expiry is
inclusive: an order expires when `current_tick >= expires_tick`.

Systems:

- `expire_orders_system`: releases stale locks, removes expired orders, marks
  market-good keys dirty.
- `generate_pool_orders_system`: creates bids/asks from pools only when the
  pool interval has elapsed and books have available balances.
- `clear_dirty_markets_system`: clears sorted dirty keys, applies trades, frees
  bid surplus, removes filled orders, and updates `MarketGoodState`.
- `update_market_telemetry_system`: computes EWMA reference price and debug
  counters from the last clearing result. It does not mutate balances or
  orders.

Economy v0 does not attach to `MobilitySet`; it has no movement dependency.

`update_market_telemetry_system` computes EWMA with integer basis points and
`i128` intermediates:

```text
ewma = (old * (10_000 - alpha_bps) + settlement * alpha_bps) / 10_000
```

`alpha_bps` is a deterministic config value, not a float.

## Runtime Seeding

`EconomyPlugin` installs empty books, order books, ledgers, market collections,
pool collections, dirty-key state, `NextOrderId`, and config resources. The
default production runtime is allowed to start with no markets, actors, pools,
or balances; in that state economy systems are no-ops by design. Scenario seed
data and tests are responsible for inserting the initial three-market/four-good
fixture.

## Determinism Requirements

- `BTreeMap`, dense vectors, or explicitly sorted key vectors are used whenever
  iteration order affects outcomes.
- `NextOrderId` is deterministic.
- No `thread_rng`, `rand::random`, UUID generation, wall-clock timestamps, or
  hash-map iteration order are used in order creation or clearing.
- Same inputs and same tick produce byte-identical trade event sequences.

## Tests

Required unit/integration tests:

Conservation:

- `auction_conserves_total_money`
- `auction_conserves_total_goods`
- `partial_fill_conserves_money_and_goods`
- `expired_orders_release_locks_without_changing_totals`

Locking:

- `bid_locks_cash`
- `ask_locks_goods`
- `cannot_bid_without_available_cash`
- `cannot_ask_without_available_goods`
- `cannot_double_lock_cash`
- `cannot_double_lock_goods`

Settlement:

- `no_trade_without_price_overlap`
- `trade_happens_with_price_overlap`
- `settlement_price_is_within_bid_ask_bounds`
- `successful_bid_refunds_locked_surplus`
- `partial_bid_refunds_surplus_for_filled_quantity`

Expiry:

- `expired_bid_releases_remaining_cash`
- `expired_ask_releases_remaining_goods`
- `expired_order_marks_market_good_dirty`

Arithmetic:

- `price_quantity_scale_computes_order_value`
- `max_price_times_qty_overflow_returns_error`
- `money_add_overflow_returns_error`
- `quantity_add_overflow_returns_error`
- `negative_quantity_is_rejected`
- `integer_ewma_uses_basis_points_without_float`

Pools and errors:

- `demand_pool_caps_order_to_affordable_quantity`
- `supply_pool_caps_order_to_available_inventory`
- `rejected_pool_order_leaves_books_unchanged`
- `failed_market_clear_leaves_books_and_orders_unchanged`

Determinism:

- `same_inputs_same_trades`
- `tie_break_uses_created_tick_then_order_id`
- `dirty_market_keys_are_processed_in_stable_order`

Plugin wiring:

- `economy_plugin_installs_books_orderbook_ledger_and_sets`
- `runtime_installs_economy_plugin`

## Acceptance Criteria

The first PR is complete when:

- `sim_core::economy` exists and is exported.
- `EconomyPlugin` implements the existing `SimPlugin` trait.
- `AccountBook`, `InventoryBook`, `OrderBook`, `TradeLedger`,
  `MarketGoodState`, demand pools, and supply pools are resources or
  resource-owned collections.
- Demand pools and supply pools generate deterministic orders at configured
  intervals.
- Bids reserve cash; asks reserve goods.
- Expired orders release remaining reserves.
- Dirty market-good keys are processed in stable order.
- Settlement prices never leave executable bid/ask bounds.
- Successful bids release locked surplus cash.
- Filled orders are removed; partially filled orders retain correct remaining
  quantity and locks.
- Pool order generation caps order quantity to available cash or goods.
- System-level errors leave books/orders unchanged and emit ledger failures.
- Trades conserve total money and total goods.
- Critical arithmetic uses checked operations and `i128` intermediates where
  needed.
- The required tests pass through the repo's cargo wrapper.

## Deferred Slices

1. **Production v0:** aggregate producers consume inputs and emit explicit
   `Produced`/`Consumed` ledger events.
2. **Transport costs:** market comparisons and pool prices incorporate existing
   routing/flow-field route costs without creating moving traders yet.
3. **Trader agents:** visible/materialized traders reserve, travel, deliver, and
   sell using mobility state.
4. **Economy LOD:** active chunks materialize traders/inventories; warm and
   asleep chunks use aggregate flows and pools.
5. **Rationing policy:** add marginal-price-group pro-rata rationing behind the
   same auction interface.
6. **Persistence/API:** snapshot economy books and expose debugging views only
   after v0 invariants are stable.
