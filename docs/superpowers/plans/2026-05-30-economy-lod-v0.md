# Economy LOD v0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Gate economy compute by chunk LOD — markets anchored to Active/Hot chunks run full-fidelity auctions; markets anchored to Warm/Asleep chunks go dormant (pools + traders skipped), conserving money and goods, with clean single-order resume on wake.

**Architecture:** Two new resources — `MarketChunks` (MarketId→ChunkCoord anchoring, populated by the spatial seeder) and `DormantMarkets` (derived per-tick). A bridge system `refresh_dormant_markets_system` reads the world's chunk LOD marker components and rewrites `DormantMarkets`. The two expensive pure helpers (`generate_pool_orders_at_tick`, `run_traders_at_tick`) gain a `dormant: &BTreeSet<MarketId>` parameter and `continue` past dormant markets. Backwards-compatible: un-anchored markets are never dormant, so all existing tests (which never set up a spatial world) keep running at full fidelity. Backend-only; nothing crosses the wire.

**Tech Stack:** Rust, bevy_ecs 0.18, `sim-core` crate. Cargo via `scripts/cargo-serial.sh`, `CARGO_TARGET_DIR=/tmp/abutown-lod-target`.

**Confirmed grounding (do not re-verify, just use):**
- `MarketId(pub u32)`, `GoodId(pub u16)`, `EconomicActorId(pub u64)` in `economy/ids.rs`.
- `ChunkCoord { x: i32, y: i32 }` derives `Ord` (`crate::ids::ChunkCoord`).
- World chunk entities carry `crate::world::components::ChunkCoordComp(pub ChunkCoord)` plus exactly one of the marker components `AsleepChunk` / `WarmChunk` / `ActiveChunk` / `HotChunk` (all `pub`, in `crate::world::components`).
- `economy/market.rs` already has `use std::collections::{BTreeMap, BTreeSet};`.
- `generate_pool_orders_at_tick(accounts, inventory, orders, ledger, dirty, next, demand, supply, current_tick, ttl_ticks)` — direct callers: `economy/systems.rs:98` (the system) and `tests/pools.rs` lines 35, 82, 128.
- `run_traders_at_tick(accounts, inventory, orders, ledger, dirty, next, market_goods, traders, config, current_tick)` — direct callers: `economy/systems.rs:183` (the system) and `tests/traders.rs` lines 54, 98, 152, 218.
- Economy unit tests build `World::new()` + `Schedule::default()` then `EconomyPlugin.install(...)`; they never populate `MarketChunks` and spawn no chunk entities.
- `plugin.rs` test installs `CorePlugin` + `MobilityPlugin` + `EconomyPlugin` and uses `world.contains_resource::<T>()` (presence checks, not exact-set), then `schedule.run`.

**Parameter convention:** append the new `dormant: &BTreeSet<MarketId>` as the **last** parameter of each helper. At call sites that should not gate (existing tests), pass `&BTreeSet::new()` — the element type infers to `MarketId` from the signature.

---

### Task 1: Anchoring + dormancy resources + bridge system

**Files:**
- Modify: `backend/crates/sim-core/src/economy/market.rs` (add two resources)
- Modify: `backend/crates/sim-core/src/economy/systems.rs` (add bridge system + imports)
- Create: `backend/crates/sim-core/src/economy/tests/lod.rs`
- Modify: `backend/crates/sim-core/src/economy/tests/mod.rs` (add `mod lod;`)

- [ ] **Step 1: Write the failing test** — create `tests/lod.rs` with the bridge test:

```rust
use std::collections::BTreeSet;

use bevy_ecs::prelude::*;

use crate::economy::{DormantMarkets, MarketChunks, MarketId, refresh_dormant_markets_system};
use crate::ids::ChunkCoord;
use crate::world::components::{ActiveChunk, AsleepChunk, ChunkCoordComp, HotChunk, WarmChunk};

#[test]
fn refresh_dormant_markets_marks_only_anchored_inactive() {
    let mut world = World::new();
    // Four chunks, one per LOD level.
    world.spawn((ChunkCoordComp(ChunkCoord { x: 0, y: 0 }), AsleepChunk));
    world.spawn((ChunkCoordComp(ChunkCoord { x: 1, y: 0 }), WarmChunk));
    world.spawn((ChunkCoordComp(ChunkCoord { x: 2, y: 0 }), ActiveChunk));
    world.spawn((ChunkCoordComp(ChunkCoord { x: 3, y: 0 }), HotChunk));

    let mut anchors = MarketChunks::default();
    anchors.0.insert(MarketId(10), ChunkCoord { x: 0, y: 0 }); // asleep -> dormant
    anchors.0.insert(MarketId(11), ChunkCoord { x: 1, y: 0 }); // warm   -> dormant
    anchors.0.insert(MarketId(12), ChunkCoord { x: 2, y: 0 }); // active -> awake
    anchors.0.insert(MarketId(13), ChunkCoord { x: 3, y: 0 }); // hot    -> awake
    world.insert_resource(anchors);
    world.insert_resource(DormantMarkets::default());

    let mut schedule = bevy_ecs::schedule::Schedule::default();
    schedule.add_systems(refresh_dormant_markets_system);
    schedule.run(&mut world);

    let dormant = world.resource::<DormantMarkets>();
    let expected: BTreeSet<MarketId> = [MarketId(10), MarketId(11)].into_iter().collect();
    assert_eq!(dormant.0, expected);
}

#[test]
fn unanchored_market_is_never_dormant() {
    let mut world = World::new();
    // No active chunks at all, and the market is not anchored.
    world.insert_resource(MarketChunks::default());
    world.insert_resource(DormantMarkets::default());

    let mut schedule = bevy_ecs::schedule::Schedule::default();
    schedule.add_systems(refresh_dormant_markets_system);
    schedule.run(&mut world);

    assert!(world.resource::<DormantMarkets>().0.is_empty());
}
```

- [ ] **Step 2: Add `mod lod;`** to `tests/mod.rs` (alphabetical — between `mod locking;` and `mod overflow;`):

```rust
mod locking;
mod lod;
mod overflow;
```

- [ ] **Step 3: Run test to verify it fails**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core refresh_dormant_markets`
Expected: FAIL to compile — `MarketChunks`, `DormantMarkets`, `refresh_dormant_markets_system` not found.

- [ ] **Step 4: Add the two resources** to the end of `economy/market.rs`. It already has `use std::collections::{BTreeMap, BTreeSet};`; add `use crate::ids::ChunkCoord;` to the top imports.

```rust
/// MarketId -> the chunk that contains its market node. Populated by the spatial
/// seeder (which owns the routing `Graph`) so the economy core needs no per-tick
/// `Graph` dependency. Markets absent from this map are un-anchored and ALWAYS
/// simulated — this is what keeps pure-economy tests at full fidelity.
#[derive(Resource, Default)]
pub struct MarketChunks(pub BTreeMap<MarketId, ChunkCoord>);

/// The set of currently DORMANT markets: anchored (present in `MarketChunks`) to
/// a chunk that is NOT Active/Hot. Recomputed every tick by
/// `refresh_dormant_markets_system`. Anything not in this set runs full fidelity.
#[derive(Resource, Default)]
pub struct DormantMarkets(pub BTreeSet<MarketId>);
```

- [ ] **Step 5: Add the bridge system** to `economy/systems.rs`. Add these imports near the top (after the existing `use crate::mobility::resources::Tick;`):

```rust
use std::collections::BTreeSet;

use bevy_ecs::query::Or;

use crate::economy::{DormantMarkets, MarketChunks, MarketId};
use crate::ids::ChunkCoord;
use crate::world::components::{ActiveChunk, ChunkCoordComp, HotChunk};
```

(Note: `Res`, `ResMut`, `Query`, `With` come from the existing `use bevy_ecs::prelude::*;`. If `With` is not in scope, add it to the `bevy_ecs::query` import: `use bevy_ecs::query::{Or, With};`.)

Then add the system function (place it just above `pub fn expire_orders_system`):

```rust
/// Bridge: derive `DormantMarkets` from chunk LOD. A market anchored (in
/// `MarketChunks`) to a chunk that is not Active/Hot is dormant; everything else
/// runs at full fidelity. Cheap: one pass over active chunk coords + one over the
/// anchor map. Deterministic (BTree iteration, set membership).
pub fn refresh_dormant_markets_system(
    anchors: Res<MarketChunks>,
    active_chunks: Query<&ChunkCoordComp, Or<(With<ActiveChunk>, With<HotChunk>)>>,
    mut dormant: ResMut<DormantMarkets>,
) {
    let active: BTreeSet<ChunkCoord> = active_chunks.iter().map(|c| c.0).collect();
    dormant.0 = anchors
        .0
        .iter()
        .filter(|(_, coord)| !active.contains(coord))
        .map(|(market, _)| *market)
        .collect();
}
```

(`MarketId` is referenced via the `crate::economy::{... MarketId}` import only if needed by later code in this file; the system itself infers it. Keep the import — it is used by the gated systems in Tasks 2–3. If clippy flags it unused at this step, remove it now and re-add in Task 2.)

- [ ] **Step 6: Run test to verify it passes**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core lod`
Expected: PASS — `refresh_dormant_markets_marks_only_anchored_inactive`, `unanchored_market_is_never_dormant`.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-core/src/economy/market.rs \
        backend/crates/sim-core/src/economy/systems.rs \
        backend/crates/sim-core/src/economy/tests/lod.rs \
        backend/crates/sim-core/src/economy/tests/mod.rs
git commit -m "feat(economy): market chunk-anchoring + dormant-markets bridge"
```

---

### Task 2: Gate pool order generation

**Files:**
- Modify: `backend/crates/sim-core/src/economy/pools.rs` (param + guards)
- Modify: `backend/crates/sim-core/src/economy/systems.rs` (`generate_pool_orders_system`)
- Modify: `backend/crates/sim-core/src/economy/tests/pools.rs` (3 call sites + import)
- Modify: `backend/crates/sim-core/src/economy/tests/lod.rs` (new gating tests)

- [ ] **Step 1: Write the failing tests** — append to `tests/lod.rs`:

```rust
use crate::economy::{
    AccountBook, DemandPool, DemandPools, DirtyMarketGoods, EconomicActorId, GOOD_FOOD,
    InventoryBook, Money, NextOrderId, OrderBook, Quantity, SupplyPool, SupplyPools, TradeLedger,
    generate_pool_orders_at_tick,
};

fn seeded_demand_pool(actor: EconomicActorId, market: MarketId) -> DemandPool {
    DemandPool {
        actor,
        market,
        good: GOOD_FOOD,
        desired_qty_per_tick: Quantity(5),
        max_price: Money(1_000),
        urgency_bps: 0,
        elasticity_bps: 0,
        interval_ticks: 1,
        last_generated_tick: None,
    }
}

#[test]
fn dormant_market_generates_no_orders() {
    let actor = EconomicActorId(1);
    let market = MarketId(7);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();
    accounts.deposit(actor, Money(1_000_000)).unwrap();
    demand.0.insert(actor, seeded_demand_pool(actor, market));
    let before = accounts.total_money();

    let dormant: BTreeSet<MarketId> = [market].into_iter().collect();
    generate_pool_orders_at_tick(
        &mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next,
        &mut demand, &mut supply, 0, 5, &dormant,
    )
    .unwrap();

    assert!(orders.bids.is_empty(), "dormant market must not place bids");
    assert!(dirty.0.is_empty(), "dormant market must not dirty any market-good");
    assert_eq!(accounts.total_money(), before, "no cash locked while dormant");
}

#[test]
fn awake_market_still_generates_orders() {
    let actor = EconomicActorId(1);
    let market = MarketId(7);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();
    accounts.deposit(actor, Money(1_000_000)).unwrap();
    demand.0.insert(actor, seeded_demand_pool(actor, market));

    let dormant: BTreeSet<MarketId> = BTreeSet::new();
    generate_pool_orders_at_tick(
        &mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next,
        &mut demand, &mut supply, 0, 5, &dormant,
    )
    .unwrap();

    assert_eq!(orders.bids.len(), 1, "awake market places its bid");
}

#[test]
fn market_resumes_with_single_order_no_burst() {
    let actor = EconomicActorId(1);
    let market = MarketId(7);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();
    accounts.deposit(actor, Money(1_000_000)).unwrap();
    demand.0.insert(actor, seeded_demand_pool(actor, market));

    let dormant: BTreeSet<MarketId> = [market].into_iter().collect();
    // Dormant for 100 ticks: no orders accrue.
    for tick in 0..100 {
        generate_pool_orders_at_tick(
            &mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next,
            &mut demand, &mut supply, tick, 5, &dormant,
        )
        .unwrap();
    }
    assert!(orders.bids.is_empty());

    // Wake on tick 100: exactly ONE order, not a 100-order backlog burst.
    let awake: BTreeSet<MarketId> = BTreeSet::new();
    generate_pool_orders_at_tick(
        &mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next,
        &mut demand, &mut supply, 100, 5, &awake,
    )
    .unwrap();
    assert_eq!(orders.bids.len(), 1, "wake emits exactly one order");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core dormant_market_generates_no_orders`
Expected: FAIL to compile — `generate_pool_orders_at_tick` takes 10 args, not 11.

- [ ] **Step 3: Add the parameter + guards** in `pools.rs`. Add `BTreeSet` to the imports (`use std::collections::{BTreeMap, BTreeSet};`). Change the signature's tail and add a guard at the top of each loop body:

Signature — append the parameter:
```rust
pub fn generate_pool_orders_at_tick(
    accounts: &mut AccountBook,
    inventory: &mut InventoryBook,
    orders: &mut OrderBook,
    ledger: &mut TradeLedger,
    dirty: &mut DirtyMarketGoods,
    next: &mut NextOrderId,
    demand: &mut DemandPools,
    supply: &mut SupplyPools,
    current_tick: u64,
    ttl_ticks: u64,
    dormant: &BTreeSet<MarketId>,
) -> Result<(), EconomyError> {
```

Demand loop — add the guard as the FIRST statement after `let mut pool = demand.0[&actor];`:
```rust
    for actor in demand_ids {
        let mut pool = demand.0[&actor];
        if dormant.contains(&pool.market) {
            continue; // dormant market: no orders, last_generated_tick untouched
        }
        if !interval_elapsed(pool.last_generated_tick, current_tick, pool.interval_ticks) {
            continue;
        }
```

Supply loop — likewise, FIRST statement after `let mut pool = supply.0[&actor];`:
```rust
    for actor in supply_ids {
        let mut pool = supply.0[&actor];
        if dormant.contains(&pool.market) {
            continue; // dormant market: no orders, last_generated_tick untouched
        }
        if !interval_elapsed(pool.last_generated_tick, current_tick, pool.interval_ticks) {
            continue;
        }
```

(`MarketId` is already imported in `pools.rs`.)

- [ ] **Step 4: Update the system** `generate_pool_orders_system` in `systems.rs`. Add `dormant: Res<DormantMarkets>` to its params and forward `&dormant.0` as the final arg:

```rust
#[allow(clippy::too_many_arguments)]
pub fn generate_pool_orders_system(
    tick: Res<Tick>,
    config: Res<EconomyConfig>,
    dormant: Res<DormantMarkets>,
    mut accounts: ResMut<AccountBook>,
    mut inventory: ResMut<InventoryBook>,
    mut orders: ResMut<OrderBook>,
    mut ledger: ResMut<TradeLedger>,
    mut dirty: ResMut<DirtyMarketGoods>,
    mut next: ResMut<NextOrderId>,
    mut demand: ResMut<DemandPools>,
    mut supply: ResMut<SupplyPools>,
) {
    let _ = generate_pool_orders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut demand,
        &mut supply,
        tick.0,
        config.default_order_ttl_ticks,
        &dormant.0,
    );
}
```

- [ ] **Step 5: Update the 3 existing call sites** in `tests/pools.rs`. Add `use std::collections::BTreeSet;` at the top of the file. At each of the three `generate_pool_orders_at_tick(...)` calls (after the `current_tick, ttl_ticks` pair — i.e. after the `5,` line), append a final argument `&BTreeSet::new(),`:

```rust
        &mut demand,
        &mut supply,
        10,
        5,
        &BTreeSet::new(),
    )
    .unwrap();
```

(The third call uses `1, 5,` — append `&BTreeSet::new(),` the same way.)

- [ ] **Step 6: Run to verify**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core pools lod`
Expected: PASS — existing pool tests + the three new gating tests all green.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-core/src/economy/pools.rs \
        backend/crates/sim-core/src/economy/systems.rs \
        backend/crates/sim-core/src/economy/tests/pools.rs \
        backend/crates/sim-core/src/economy/tests/lod.rs
git commit -m "feat(economy): gate pool order generation by market dormancy"
```

---

### Task 3: Gate traders

**Files:**
- Modify: `backend/crates/sim-core/src/economy/traders.rs` (param + guard)
- Modify: `backend/crates/sim-core/src/economy/systems.rs` (`run_traders_system`)
- Modify: `backend/crates/sim-core/src/economy/tests/traders.rs` (4 call sites + import)
- Modify: `backend/crates/sim-core/src/economy/tests/lod.rs` (frozen-trader test)

- [ ] **Step 1: Write the failing test** — append to `tests/lod.rs`. (This mirrors the trader e2e setup but with the trader's `source` market dormant; assert the trader does nothing and money/goods are conserved.)

```rust
use crate::economy::{
    GOOD_TOOLS, Trader, TraderState, Traders, run_traders_at_tick, EconomyConfig, MarketGoods,
};

#[test]
fn dormant_trader_is_frozen_and_conserves() {
    let trader_actor = EconomicActorId(1);
    let source = MarketId(1);
    let dest = MarketId(2);

    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let goods = MarketGoods::default();
    let cfg = EconomyConfig::default();

    accounts.deposit(trader_actor, Money(1_000_000)).unwrap();
    let mut traders = Traders::default();
    traders.0.insert(
        trader_actor,
        Trader {
            actor: trader_actor,
            good: GOOD_TOOLS,
            source,
            dest,
            distance_tiles: 4,
            batch_qty: Quantity(100),
            buy_premium_bps: 500,
            sell_discount_bps: 500,
            order_ttl_ticks: 10,
            state: TraderState::Buying { order: None },
        },
    );

    let money_before = accounts.total_money();
    let trader_before = traders.0[&trader_actor].clone();

    // source market dormant -> trader frozen for many ticks
    let dormant: BTreeSet<MarketId> = [source].into_iter().collect();
    for tick in 0..20 {
        run_traders_at_tick(
            &mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next,
            &goods, &mut traders, &cfg, tick, &dormant,
        )
        .unwrap();
    }

    assert!(orders.bids.is_empty(), "frozen trader places no bids");
    assert_eq!(accounts.total_money(), money_before, "money conserved while frozen");
    assert_eq!(
        traders.0[&trader_actor].state,
        trader_before.state,
        "frozen trader keeps its state",
    );
}
```

(If `Trader`/`TraderState` do not derive `Clone`/`PartialEq` for the asserts, the implementer may compare via `matches!(traders.0[&trader_actor].state, TraderState::Buying { .. })` instead — do NOT add derives unless they already exist.)

- [ ] **Step 2: Run to verify it fails**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core dormant_trader_is_frozen`
Expected: FAIL to compile — `run_traders_at_tick` takes 10 args, not 11.

- [ ] **Step 3: Add the parameter + guard** in `traders.rs`. Add `use std::collections::BTreeSet;` (the file currently imports `BTreeMap`). Append the parameter to the signature and add the guard as the first statement in the loop body:

Signature tail:
```rust
    config: &EconomyConfig,
    current_tick: u64,
    dormant: &BTreeSet<MarketId>,
) -> Result<(), EconomyError> {
    let actors: Vec<EconomicActorId> = traders.0.keys().copied().collect();
    for actor in actors {
        let mut trader = traders.0[&actor].clone();
        if dormant.contains(&trader.source) {
            continue; // source region unobserved: hibernate, state frozen
        }
        match trader.state {
```

(`MarketId` is already imported in `traders.rs`.)

- [ ] **Step 4: Update the system** `run_traders_system` in `systems.rs`. Add `dormant: Res<DormantMarkets>` to its params and forward `&dormant.0` as the final arg:

```rust
#[allow(clippy::too_many_arguments)]
pub fn run_traders_system(
    tick: Res<Tick>,
    config: Res<EconomyConfig>,
    dormant: Res<DormantMarkets>,
    mut accounts: ResMut<AccountBook>,
    mut inventory: ResMut<InventoryBook>,
    mut orders: ResMut<OrderBook>,
    mut ledger: ResMut<TradeLedger>,
    mut dirty: ResMut<DirtyMarketGoods>,
    mut next: ResMut<NextOrderId>,
    market_goods: Res<MarketGoods>,
    mut traders: ResMut<Traders>,
) {
    let _ = run_traders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &market_goods,
        &mut traders,
        &config,
        tick.0,
        &dormant.0,
    );
}
```

- [ ] **Step 5: Update the 4 existing call sites** in `tests/traders.rs`. Add `use std::collections::BTreeSet;` at the top. Append `&BTreeSet::new()` as the final argument:
  - Lines ~54 and ~218 (multi-line calls ending `&cfg, 0,`): add `&BTreeSet::new(),` after the `0,`.
  - Line ~98 (multi-line call ending `&cfg, 0,`): same.
  - Line ~152 (single-line closure call `run_traders_at_tick(accounts, inventory, orders, ledger, dirty, next, goods, traders, cfg, 0,)`): change to `... cfg, 0, &BTreeSet::new(),`.

- [ ] **Step 6: Run to verify**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core traders lod`
Expected: PASS — existing trader tests + `dormant_trader_is_frozen_and_conserves`.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-core/src/economy/traders.rs \
        backend/crates/sim-core/src/economy/systems.rs \
        backend/crates/sim-core/src/economy/tests/traders.rs \
        backend/crates/sim-core/src/economy/tests/lod.rs
git commit -m "feat(economy): gate traders by source-market dormancy"
```

---

### Task 4: Schedule wiring + plugin resources + end-to-end & determinism

**Files:**
- Modify: `backend/crates/sim-core/src/economy/systems.rs` (`EconomySet::RefreshLod` + install)
- Modify: `backend/crates/sim-core/src/economy/mod.rs` (insert two resources)
- Modify: `backend/crates/sim-core/src/economy/tests/plugin.rs` (presence asserts)
- Modify: `backend/crates/sim-core/src/economy/tests/lod.rs` (e2e + determinism via full schedule)

- [ ] **Step 1: Write the failing e2e + determinism tests** — append to `tests/lod.rs`:

```rust
use crate::economy::{EconomyPlugin, SupplyPool, SupplyPools};
use crate::world::plugin::CorePlugin;
use crate::world::schedule::SimPlugin;
use crate::mobility::resources::Tick;

// Build a world with Core + Mobility + Economy, one supply pool selling FOOD at
// `market`, the trader/markets un-touched. Anchor `market` to `coord`, and spawn a
// chunk entity at `coord` with the given marker. Returns the assembled world+schedule.
fn lod_world(market: MarketId, coord: ChunkCoord, asleep: bool) -> (World, bevy_ecs::schedule::Schedule) {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let supplier = EconomicActorId(50);
    {
        let mut inv = world.resource_mut::<InventoryBook>();
        inv.deposit(supplier, GOOD_FOOD, Quantity(1_000_000)).unwrap();
    }
    {
        let mut supply = world.resource_mut::<SupplyPools>();
        supply.0.insert(
            supplier,
            SupplyPool {
                actor: supplier,
                market,
                good: GOOD_FOOD,
                offered_qty_per_tick: Quantity(10),
                min_price: Money(1_000),
                interval_ticks: 1,
                last_generated_tick: None,
            },
        );
    }
    {
        let mut anchors = world.resource_mut::<MarketChunks>();
        anchors.0.insert(market, coord);
    }
    if asleep {
        world.spawn((ChunkCoordComp(coord), AsleepChunk));
    } else {
        world.spawn((ChunkCoordComp(coord), ActiveChunk));
    }
    world.insert_resource(Tick(0));
    (world, schedule)
}

#[test]
fn asleep_anchored_market_stays_frozen_end_to_end() {
    let market = MarketId(99);
    let coord = ChunkCoord { x: 5, y: 5 };
    let (mut world, mut schedule) = lod_world(market, coord, /*asleep=*/ true);

    for _ in 0..10 {
        schedule.run(&mut world);
        let mut t = world.resource_mut::<Tick>();
        t.0 += 1;
    }
    // No asks were ever placed because the supplier's market is dormant.
    assert!(world.resource::<OrderBook>().asks.is_empty());
    // Plugin installed the two new resources.
    assert!(world.contains_resource::<MarketChunks>());
    assert!(world.contains_resource::<DormantMarkets>());
}

#[test]
fn active_anchored_market_trades_end_to_end() {
    let market = MarketId(99);
    let coord = ChunkCoord { x: 5, y: 5 };
    let (mut world, mut schedule) = lod_world(market, coord, /*asleep=*/ false);

    let mut saw_ask = false;
    for _ in 0..10 {
        schedule.run(&mut world);
        if !world.resource::<OrderBook>().asks.is_empty() {
            saw_ask = true;
        }
        let mut t = world.resource_mut::<Tick>();
        t.0 += 1;
    }
    assert!(saw_ask, "active market must place asks");
}
```

(Note on the supplier inventory total: if `InventoryBook::deposit` has a different signature, the implementer adapts to the real one — confirmed earlier as `inventory.deposit(actor, good, qty)`. If `OrderBook::asks` is not a public field, assert on the `TradeLedger` events instead.)

- [ ] **Step 2: Run to verify it fails**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core asleep_anchored_market_stays_frozen`
Expected: FAIL — `DormantMarkets`/`MarketChunks` not installed by the plugin, and `RefreshLod` system not wired, so dormancy is never computed (the asleep market would still trade) and/or `contains_resource` asserts fail.

- [ ] **Step 3: Wire the set + system** in `systems.rs`. Add `RefreshLod` as the FIRST variant of `EconomySet` and first in the chain, and register the bridge system in it:

```rust
#[derive(SystemSet, Hash, Eq, PartialEq, Debug, Clone)]
pub enum EconomySet {
    RefreshLod,
    ExpireOrders,
    Production,
    Traders,
    GeneratePoolOrders,
    ClearMarkets,
    Telemetry,
}
```

```rust
    schedule.configure_sets(
        (
            EconomySet::RefreshLod,
            EconomySet::ExpireOrders,
            EconomySet::Production,
            EconomySet::Traders,
            EconomySet::GeneratePoolOrders,
            EconomySet::ClearMarkets,
            EconomySet::Telemetry,
        )
            .chain(),
    );
    schedule.add_systems(
        (
            refresh_dormant_markets_system.in_set(EconomySet::RefreshLod),
            expire_orders_system.in_set(EconomySet::ExpireOrders),
            run_production_system.in_set(EconomySet::Production),
            run_traders_system.in_set(EconomySet::Traders),
            generate_pool_orders_system.in_set(EconomySet::GeneratePoolOrders),
            clear_dirty_markets_system.in_set(EconomySet::ClearMarkets),
            update_market_telemetry_system.in_set(EconomySet::Telemetry),
        )
            .before(crate::mobility::systems::tick_increment_system),
    );
```

- [ ] **Step 4: Insert the two resources** in `economy/mod.rs` `EconomyPlugin::install`, alongside the others (before `install_systems(schedule);`):

```rust
        world.insert_resource(MarketChunks::default());
        world.insert_resource(DormantMarkets::default());
```

- [ ] **Step 5: Add presence asserts** in `tests/plugin.rs`. Extend the import to include the two new resources and add two asserts after the existing block:

```rust
    assert!(world.contains_resource::<crate::economy::MarketChunks>());
    assert!(world.contains_resource::<crate::economy::DormantMarkets>());
```

- [ ] **Step 6: Run the e2e + full economy suite**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core lod`
Expected: PASS — `asleep_anchored_market_stays_frozen_end_to_end`, `active_anchored_market_trades_end_to_end`.

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy`
Expected: PASS — all prior economy/production/transport/trader tests unaffected.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-core/src/economy/systems.rs \
        backend/crates/sim-core/src/economy/mod.rs \
        backend/crates/sim-core/src/economy/tests/plugin.rs \
        backend/crates/sim-core/src/economy/tests/lod.rs
git commit -m "feat(economy): wire chunk-LOD gating into the economy schedule"
```

---

### Final gate (orchestrator runs; implementer reports readiness)

```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace --all-targets
```

All three must be green. The implementer does NOT push or open a PR — report back the per-task RED→GREEN + commit SHAs, the `-p sim-core lod` and `-p sim-core economy` summaries, and clippy/fmt status.

## Self-review notes

- **Spec coverage:** anchoring (`MarketChunks`) ✓, derived dormancy (`DormantMarkets`) ✓, bridge system ✓, pool gating ✓, trader gating ✓, clearing auto-gated via dirty ✓, schedule set ✓, plugin resources ✓, conservation/determinism/no-burst/frozen-trader/e2e tests ✓.
- **Backwards compatibility:** every existing call site passes `&BTreeSet::new()` (no gating); tests never populate `MarketChunks`, so `DormantMarkets` is empty there → identical behavior.
- **Type consistency:** `dormant: &BTreeSet<MarketId>` is the last param of both helpers; both systems read `Res<DormantMarkets>` and forward `&dormant.0`; `MarketChunks`/`DormantMarkets` are `Resource` + `Default`.
- **No second engine:** dormancy is `continue`, not an alternate settlement path — conservation is automatic.
