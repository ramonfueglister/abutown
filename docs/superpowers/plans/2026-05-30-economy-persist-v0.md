# Economy Persistence v0 (slice 6a) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the economy state serde-serializable and round-trippable: an `EconomyPersistSnapshot` with `extract_from_world` / `apply_into_world`, plus an `EconomySnapshotProvider` implementing the existing `SnapshotProvider` trait (`kind = "economy"`). sim-core only; no migration, no sim-server, no wire.

**Architecture:** Add `Serialize, Deserialize` to the economy value types (and to `routing::NodeId`, referenced by `MarketSite`). Define `EconomyPersistSnapshot` with every map represented as a sorted `Vec<(K,V)>` (serde_json rejects non-string map keys; `BTreeMap` iteration gives byte-stable order). `extract` builds the Vecs from the live resources; `apply` rebuilds the resources into a freshly-installed `EconomyPlugin` world. The provider serializes the snapshot to JSON via `serde_json::to_vec`, mirroring `MobilitySnapshotProvider`.

**Tech Stack:** Rust, `sim-core` crate, `serde` + `serde_json` (already deps — mobility uses them). Cargo via `scripts/cargo-serial.sh`, `CARGO_TARGET_DIR=/tmp/abutown-persist-target`.

**Confirmed grounding (use directly):**
- Resources & inner shapes: `AccountBook { accounts: BTreeMap<EconomicActorId, MoneyAccount> }`; `InventoryBook { balances: BTreeMap<(EconomicActorId, GoodId), InventoryBalance> }`; `OrderBook { bids: BTreeMap<OrderId, Bid>, asks: BTreeMap<OrderId, Ask> }`; `NextOrderId(pub u64)`; `Markets(pub BTreeMap<MarketId, MarketSite>)`; `MarketGoods(pub BTreeMap<MarketGoodKey, MarketGoodState>)`; `DemandPools(pub BTreeMap<EconomicActorId, DemandPool>)`; `SupplyPools(...SupplyPool)`; `ProductionPools(...ProductionPool)`; `Traders(pub BTreeMap<EconomicActorId, Trader>)`; `MarketChunks(pub BTreeMap<MarketId, ChunkCoord>)`.
- `MoneyAccount { available: Money, locked: Money }` (Copy); `InventoryBalance { available: Quantity, locked: Quantity }` (Copy).
- `MarketSite { id: MarketId, node_id: crate::routing::NodeId, name: String }` — currently **no derives**.
- `MarketGoodState { key, last_settlement_price, ewma_reference_price, traded_qty_last_tick, unmet_demand_last_tick, unsold_supply_last_tick, dirty: bool, last_cleared_tick: u64 }` — currently **no derives**; `MarketGoodState::new(key)` exists.
- `Bid { id, owner, market, good, qty_remaining, max_price, cash_locked_remaining, created_tick, expires_tick }`; `Ask { …, min_price, goods_locked_remaining, … }`.
- `DemandPool { actor, market, good, desired_qty_per_tick, max_price, urgency_bps, elasticity_bps, interval_ticks, last_generated_tick: Option<u64> }`; `SupplyPool { actor, market, good, offered_qty_per_tick, min_price, interval_ticks, last_generated_tick }`.
- `Recipe { inputs: Vec<(GoodId, Quantity)>, outputs: Vec<(GoodId, Quantity)> }`; `ProductionPool { actor, recipe, interval_ticks, last_generated_tick }`.
- `Trader { actor, good, source, dest, distance_tiles: i64, batch_qty: Quantity, buy_premium_bps: i32, sell_discount_bps: i32, order_ttl_ticks: u64, state: TraderState }`; `TraderState::Buying { order: Option<OrderId> }` (+ ToDest/Selling/ToSource).
- `routing::NodeId(pub u32)` at `backend/crates/sim-core/src/routing/graph.rs:4-5`, derives `Component, Copy, Clone, Hash, Eq, PartialEq, Debug` (no serde).
- `ChunkCoord` already derives `Serialize, Deserialize`.
- `EconomyPlugin.install(&mut world, &mut schedule)` inserts all economy resources as defaults.
- `SnapshotProvider`/`SnapshotItem`/`SnapshotKey`/`MigrationError` at `crate::world::persistence`. `MobilitySnapshotProvider` (`backend/crates/sim-core/src/mobility/snapshot_provider.rs`) is the template.
- Goods consts: `GOOD_FOOD = GoodId(1)`, `GOOD_TOOLS = GoodId(4)`.

---

### Task 1: serde derives on economy value types + `NodeId`

**Files (derive-line edits only):**

- [ ] **Step 1: Add `Serialize, Deserialize`** (append to the existing derive list) on each type below. Add `use serde::{Deserialize, Serialize};` to each file that doesn't already import it.

  | File | Type | New derive line |
  |---|---|---|
  | `economy/money.rs` | `Money` | `#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]` |
  | `economy/money.rs` | `Quantity` | same shape + `Serialize, Deserialize` |
  | `economy/ids.rs` | `GoodId`, `MarketId`, `OrderId`, `EconomicActorId` | append `, Serialize, Deserialize` to each `#[derive(...)]` |
  | `economy/accounts.rs` | `MoneyAccount` | append `, Serialize, Deserialize` |
  | `economy/inventory.rs` | `InventoryBalance` | append `, Serialize, Deserialize` |
  | `economy/orders.rs` | `Bid`, `Ask` | append `, Serialize, Deserialize` |
  | `economy/market.rs` | `MarketGoodKey` | append `, Serialize, Deserialize` |
  | `economy/market.rs` | `MarketSite` | **add** `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]` (currently none) |
  | `economy/market.rs` | `MarketGoodState` | **add** `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]` (currently none) |
  | `economy/pools.rs` | `DemandPool`, `SupplyPool` | append `, Serialize, Deserialize` |
  | `economy/production.rs` | `Recipe`, `ProductionPool` | append `, Serialize, Deserialize` |
  | `economy/traders.rs` | `TraderState`, `Trader` | append `, Serialize, Deserialize` |
  | `routing/graph.rs` | `NodeId` | append `, Serialize, Deserialize` to its `#[derive(Component, Copy, Clone, Hash, Eq, PartialEq, Debug)]` (add `use serde::{Deserialize, Serialize};` if absent) |

  Note: `MarketSite`/`MarketGoodState` gaining `Eq` is sound (all fields are `Eq`: `String`, `NodeId`, `Money`, `Quantity`, `bool`, `u64`).

- [ ] **Step 2: Write a compile-proof test** — create `backend/crates/sim-core/src/economy/tests/persist.rs` with just:

```rust
use crate::economy::{GOOD_FOOD, MarketId, MarketSite, Money};

#[test]
fn value_types_are_serde_serializable() {
    let m = Money(1_234);
    let json = serde_json::to_string(&m).unwrap();
    assert_eq!(serde_json::from_str::<Money>(&json).unwrap(), m);

    let site = MarketSite {
        id: MarketId(1),
        node_id: crate::routing::NodeId(7),
        name: "M1".to_string(),
    };
    let j = serde_json::to_string(&site).unwrap();
    let back: MarketSite = serde_json::from_str(&j).unwrap();
    assert_eq!(back, site);
    let _ = GOOD_FOOD;
}
```

Add `mod persist;` to `backend/crates/sim-core/src/economy/tests/mod.rs` — insert between `mod pools;` and `mod production;`:

```rust
mod pools;
mod persist;
mod production;
```

- [ ] **Step 3: Run to verify it compiles + passes**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core value_types_are_serde`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add backend/crates/sim-core/src/economy/money.rs \
        backend/crates/sim-core/src/economy/ids.rs \
        backend/crates/sim-core/src/economy/accounts.rs \
        backend/crates/sim-core/src/economy/inventory.rs \
        backend/crates/sim-core/src/economy/orders.rs \
        backend/crates/sim-core/src/economy/market.rs \
        backend/crates/sim-core/src/economy/pools.rs \
        backend/crates/sim-core/src/economy/production.rs \
        backend/crates/sim-core/src/economy/traders.rs \
        backend/crates/sim-core/src/routing/graph.rs \
        backend/crates/sim-core/src/economy/tests/persist.rs \
        backend/crates/sim-core/src/economy/tests/mod.rs
git commit -m "feat(economy): serde derives for persisted value types"
```

---

### Task 2: `EconomyPersistSnapshot` + extract/apply round-trip

**Files:**
- Create: `backend/crates/sim-core/src/economy/persist.rs`
- Modify: `backend/crates/sim-core/src/economy/mod.rs` (`pub mod persist; pub use persist::*;`)
- Modify: `backend/crates/sim-core/src/economy/tests/persist.rs` (round-trip tests)

- [ ] **Step 1: Write the failing tests** — append to `tests/persist.rs`:

```rust
use bevy_ecs::prelude::*;

use crate::economy::{
    AccountBook, Ask, Bid, DemandPool, DemandPools, EconomicActorId, EconomyPersistSnapshot,
    EconomyPlugin, GOOD_TOOLS, InventoryBook, MarketChunks, MarketGoodKey, MarketGoodState,
    MarketGoods, Markets, MoneyAccount, NextOrderId, OrderBook, OrderId, ProductionPool,
    ProductionPools, Quantity, Recipe, SupplyPool, SupplyPools, Trader, TraderState, Traders,
    apply_into_world, extract_from_world,
};
use crate::economy::{GOOD_FOOD, InventoryBalance};
use crate::ids::ChunkCoord;
use crate::world::schedule::SimPlugin;

fn install_economy() -> World {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    EconomyPlugin.install(&mut world, &mut schedule);
    world
}

fn seed(world: &mut World) {
    let a = EconomicActorId(1);
    let b = EconomicActorId(2);
    let m = crate::economy::MarketId(1);

    world.resource_mut::<AccountBook>().accounts.insert(
        a,
        MoneyAccount { available: crate::economy::Money(5_000), locked: crate::economy::Money(250) },
    );
    world
        .resource_mut::<InventoryBook>()
        .balances
        .insert((b, GOOD_FOOD), InventoryBalance { available: Quantity(40), locked: Quantity(5) });

    world.resource_mut::<OrderBook>().bids.insert(
        OrderId(1),
        Bid {
            id: OrderId(1), owner: a, market: m, good: GOOD_FOOD,
            qty_remaining: Quantity(10), max_price: crate::economy::Money(1_200),
            cash_locked_remaining: crate::economy::Money(12), created_tick: 1, expires_tick: 100,
        },
    );
    world.resource_mut::<OrderBook>().asks.insert(
        OrderId(2),
        Ask {
            id: OrderId(2), owner: b, market: m, good: GOOD_FOOD,
            qty_remaining: Quantity(10), min_price: crate::economy::Money(1_000),
            goods_locked_remaining: Quantity(10), created_tick: 1, expires_tick: 100,
        },
    );
    world.resource_mut::<NextOrderId>().0 = 3;

    world.resource_mut::<Markets>().0.insert(
        m,
        crate::economy::MarketSite { id: m, node_id: crate::routing::NodeId(9), name: "M1".to_string() },
    );
    let key = MarketGoodKey { market: m, good: GOOD_FOOD };
    let mut gs = MarketGoodState::new(key);
    gs.last_settlement_price = crate::economy::Money(1_100);
    gs.last_cleared_tick = 7;
    world.resource_mut::<MarketGoods>().0.insert(key, gs);

    world.resource_mut::<DemandPools>().0.insert(
        a,
        DemandPool {
            actor: a, market: m, good: GOOD_FOOD, desired_qty_per_tick: Quantity(5),
            max_price: crate::economy::Money(1_300), urgency_bps: 0, elasticity_bps: 0,
            interval_ticks: 1, last_generated_tick: Some(3),
        },
    );
    world.resource_mut::<SupplyPools>().0.insert(
        b,
        SupplyPool {
            actor: b, market: m, good: GOOD_FOOD, offered_qty_per_tick: Quantity(5),
            min_price: crate::economy::Money(900), interval_ticks: 1, last_generated_tick: Some(3),
        },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        b,
        ProductionPool {
            actor: b,
            recipe: Recipe {
                inputs: vec![(GOOD_FOOD, Quantity(2))],
                outputs: vec![(GOOD_TOOLS, Quantity(1))],
            },
            interval_ticks: 4, last_generated_tick: None,
        },
    );
    world.resource_mut::<Traders>().0.insert(
        a,
        Trader {
            actor: a, good: GOOD_TOOLS, source: m, dest: crate::economy::MarketId(2),
            distance_tiles: 4, batch_qty: Quantity(100), buy_premium_bps: 500,
            sell_discount_bps: 500, order_ttl_ticks: 10, state: TraderState::Buying { order: None },
        },
    );
    world
        .resource_mut::<MarketChunks>()
        .0
        .insert(m, ChunkCoord { x: 2, y: 3 });
}

#[test]
fn economy_snapshot_round_trips() {
    let mut world = install_economy();
    seed(&mut world);

    let snap = extract_from_world(&world);
    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();

    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &decoded);
    let snap2 = extract_from_world(&fresh);

    assert_eq!(snap, snap2, "extract->serialize->deserialize->apply->extract is identity");
}

#[test]
fn economy_snapshot_is_byte_stable() {
    let mut world = install_economy();
    seed(&mut world);
    let a = serde_json::to_vec(&extract_from_world(&world)).unwrap();
    let b = serde_json::to_vec(&extract_from_world(&world)).unwrap();
    assert_eq!(a, b);
}

#[test]
fn empty_economy_round_trips() {
    let world = install_economy();
    let snap = extract_from_world(&world);
    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &snap);
    assert_eq!(snap, extract_from_world(&fresh));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy_snapshot_round_trips`
Expected: FAIL to compile — `EconomyPersistSnapshot`, `extract_from_world`, `apply_into_world` not found.

- [ ] **Step 3: Create `economy/persist.rs`**:

```rust
//! Persistable snapshot of the economy ECS resources. Mirrors the mobility
//! persist pattern: a serde struct plus `extract_from_world` / `apply_into_world`.
//! Every map is represented as a sorted `Vec<(K, V)>` because `serde_json` rejects
//! non-string map keys (`InventoryBook`'s tuple key, `MarketGoods`' struct key);
//! `BTreeMap` iteration yields byte-stable order.

use bevy_ecs::world::World;
use serde::{Deserialize, Serialize};

use crate::economy::{
    AccountBook, Ask, Bid, DemandPool, DemandPools, EconomicActorId, GoodId, InventoryBalance,
    InventoryBook, MarketChunks, MarketGoodKey, MarketGoodState, MarketGoods, MarketId, MarketSite,
    MoneyAccount, NextOrderId, OrderBook, OrderId, ProductionPool, ProductionPools, SupplyPool,
    SupplyPools, Trader, Traders,
};
use crate::ids::ChunkCoord;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomyPersistSnapshot {
    pub accounts: Vec<(EconomicActorId, MoneyAccount)>,
    pub inventory: Vec<((EconomicActorId, GoodId), InventoryBalance)>,
    pub bids: Vec<(OrderId, Bid)>,
    pub asks: Vec<(OrderId, Ask)>,
    pub next_order_id: u64,
    pub markets: Vec<(MarketId, MarketSite)>,
    pub market_goods: Vec<(MarketGoodKey, MarketGoodState)>,
    pub demand_pools: Vec<(EconomicActorId, DemandPool)>,
    pub supply_pools: Vec<(EconomicActorId, SupplyPool)>,
    pub production_pools: Vec<(EconomicActorId, ProductionPool)>,
    pub traders: Vec<(EconomicActorId, Trader)>,
    pub market_chunks: Vec<(MarketId, ChunkCoord)>,
}

/// Pull a snapshot out of a live economy `World`. `BTreeMap` iteration is sorted,
/// so the resulting `Vec`s — and the JSON they serialize to — are byte-stable.
pub fn extract_from_world(world: &World) -> EconomyPersistSnapshot {
    let accounts = world.resource::<AccountBook>();
    let inventory = world.resource::<InventoryBook>();
    let orders = world.resource::<OrderBook>();
    let next = world.resource::<NextOrderId>();
    let markets = world.resource::<Markets>();
    let market_goods = world.resource::<MarketGoods>();
    let demand = world.resource::<DemandPools>();
    let supply = world.resource::<SupplyPools>();
    let production = world.resource::<ProductionPools>();
    let traders = world.resource::<Traders>();
    let market_chunks = world.resource::<MarketChunks>();

    EconomyPersistSnapshot {
        accounts: accounts.accounts.iter().map(|(k, v)| (*k, *v)).collect(),
        inventory: inventory.balances.iter().map(|(k, v)| (*k, *v)).collect(),
        bids: orders.bids.iter().map(|(k, v)| (*k, v.clone())).collect(),
        asks: orders.asks.iter().map(|(k, v)| (*k, v.clone())).collect(),
        next_order_id: next.0,
        markets: markets.0.iter().map(|(k, v)| (*k, v.clone())).collect(),
        market_goods: market_goods.0.iter().map(|(k, v)| (*k, v.clone())).collect(),
        demand_pools: demand.0.iter().map(|(k, v)| (*k, *v)).collect(),
        supply_pools: supply.0.iter().map(|(k, v)| (*k, *v)).collect(),
        production_pools: production.0.iter().map(|(k, v)| (*k, v.clone())).collect(),
        traders: traders.0.iter().map(|(k, v)| (*k, v.clone())).collect(),
        market_chunks: market_chunks.0.iter().map(|(k, v)| (*k, *v)).collect(),
    }
}

/// Rebuild economy resources in a freshly-installed `EconomyPlugin` world from a
/// snapshot. Overwrites the default resources. `DormantMarkets` is left at its
/// default — it is recomputed by the LOD bridge on the next tick.
pub fn apply_into_world(world: &mut World, snap: &EconomyPersistSnapshot) {
    world.insert_resource(AccountBook {
        accounts: snap.accounts.iter().cloned().collect(),
    });
    world.insert_resource(InventoryBook {
        balances: snap.inventory.iter().cloned().collect(),
    });
    world.insert_resource(OrderBook {
        bids: snap.bids.iter().cloned().collect(),
        asks: snap.asks.iter().cloned().collect(),
    });
    world.insert_resource(NextOrderId(snap.next_order_id));
    world.insert_resource(Markets(snap.markets.iter().cloned().collect()));
    world.insert_resource(MarketGoods(snap.market_goods.iter().cloned().collect()));
    world.insert_resource(DemandPools(snap.demand_pools.iter().cloned().collect()));
    world.insert_resource(SupplyPools(snap.supply_pools.iter().cloned().collect()));
    world.insert_resource(ProductionPools(
        snap.production_pools.iter().cloned().collect(),
    ));
    world.insert_resource(Traders(snap.traders.iter().cloned().collect()));
    world.insert_resource(MarketChunks(snap.market_chunks.iter().cloned().collect()));
}
```

(`MoneyAccount`/`InventoryBalance`/`DemandPool`/`SupplyPool`/ids/`ChunkCoord` are `Copy`, so `(*k, *v)` works; `Bid`/`Ask`/`MarketSite`/`MarketGoodState`/`ProductionPool`/`Trader` use `v.clone()`. If clippy flags `map(|(k,v)| (*k, *v)).collect()` as `clone_on_copy` or suggests `.clone()` consistency, follow its suggestion.)

- [ ] **Step 4: Wire the module** in `economy/mod.rs` — add alongside the other `pub mod` / `pub use` lines:

```rust
pub mod persist;
```
```rust
pub use persist::*;
```

- [ ] **Step 5: Run to verify it passes**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core -- economy_snapshot_round_trips economy_snapshot_is_byte_stable empty_economy_round_trips`
Expected: PASS — all three.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/economy/persist.rs \
        backend/crates/sim-core/src/economy/mod.rs \
        backend/crates/sim-core/src/economy/tests/persist.rs
git commit -m "feat(economy): EconomyPersistSnapshot extract/apply round-trip"
```

---

### Task 3: `EconomySnapshotProvider`

**Files:**
- Modify: `backend/crates/sim-core/src/economy/persist.rs` (add the provider)
- Modify: `backend/crates/sim-core/src/economy/tests/persist.rs` (provider test)

- [ ] **Step 1: Write the failing test** — append to `tests/persist.rs`:

```rust
use crate::economy::EconomySnapshotProvider;
use crate::world::persistence::SnapshotProvider;

#[test]
fn provider_collects_single_economy_item() {
    let mut world = install_economy();
    seed(&mut world);

    let provider = EconomySnapshotProvider { world_id: "w1".to_string() };
    assert_eq!(provider.name(), "economy");
    assert_eq!(provider.schema_version(), 1);

    let items = provider.collect(&world);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].key.kind, "economy");
    assert_eq!(items[0].key.identifier, "full");
    assert_eq!(items[0].key.world_id, "w1");
    assert_eq!(items[0].schema_version, 1);
    assert!(!items[0].payload.is_empty());

    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&items[0].payload).unwrap();
    assert_eq!(decoded, extract_from_world(&world));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core provider_collects_single_economy_item`
Expected: FAIL to compile — `EconomySnapshotProvider` not found.

- [ ] **Step 3: Implement** — append to `economy/persist.rs`:

```rust
use crate::world::persistence::{MigrationError, SnapshotItem, SnapshotKey, SnapshotProvider};

/// A `SnapshotProvider` emitting the full economy state as one JSON item. The
/// persist loop (slice 6b) dispatches by `key.kind == "economy"` to the economy
/// store. Mirrors `MobilitySnapshotProvider`.
pub struct EconomySnapshotProvider {
    pub world_id: String,
}

impl SnapshotProvider for EconomySnapshotProvider {
    fn name(&self) -> &'static str {
        "economy"
    }
    fn schema_version(&self) -> u32 {
        1
    }
    fn collect(&self, world: &World) -> Vec<SnapshotItem> {
        let snapshot = extract_from_world(world);
        let payload = serde_json::to_vec(&snapshot).expect("serde encodes EconomyPersistSnapshot");
        vec![SnapshotItem {
            key: SnapshotKey {
                world_id: self.world_id.clone(),
                kind: "economy",
                identifier: "full".to_string(),
            },
            schema_version: 1,
            payload,
        }]
    }
    fn migrate(&self, raw: SnapshotItem, _from: u32) -> Result<SnapshotItem, MigrationError> {
        Ok(raw)
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core persist`
Expected: PASS — all persist tests (value-serde, round-trip, byte-stable, empty, provider).

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/persist.rs \
        backend/crates/sim-core/src/economy/tests/persist.rs
git commit -m "feat(economy): EconomySnapshotProvider (kind=economy)"
```

---

### Final gate (orchestrator runs; implementer reports readiness)

```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace --all-targets
```

All green. Implementer does NOT push or open a PR — report per-task RED→GREEN + commit SHAs, the `-p sim-core persist` summary, and clippy/fmt status.

## Self-review notes

- **Spec coverage:** serde derives (incl. `NodeId`) ✓, `EconomyPersistSnapshot` with Vec-of-pairs maps ✓, `extract`/`apply` ✓, `EconomySnapshotProvider` ✓, round-trip/byte-stable/empty/provider tests ✓.
- **Map-key safety:** every map is `Vec<(K,V)>` — no non-string JSON keys. Byte-stable from `BTreeMap` order.
- **Round-trip assertion** uses `EconomyPersistSnapshot: PartialEq` (so only the value types need `PartialEq` — `MarketSite`/`MarketGoodState` get it; the resource wrappers need no derive change).
- **Additive only:** no system/schedule/matching change; existing suites unaffected.
- **Scope:** sim-core only; no migration, no sim-server, no wire (6b).
