# Economy LOD Warm-Tier Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Warm-anchored markets run a cheap aggregate trade at the frozen reference price (pro-rata, coarse interval, no order book) instead of freezing; Asleep stays frozen. Backend-only, deterministic, conservation-exact.

**Architecture:** Extend the LOD bridge to also classify `WarmMarkets`. A new `run_warm_market_flow_at_tick` (in `economy/warm_flow.rs`) trades `min(aggregate demand, aggregate supply)` per warm market-good at the reference price, allocating pro-rata via the slice-5 `prorata_distribute`, applied atomically on cloned books (mirroring `clear_market_good`). Wired into a new `EconomySet::WarmFlow` after `ClearMarkets`.

**Tech Stack:** Rust, `sim-core` economy. Cargo via `scripts/cargo-serial.sh`, `CARGO_TARGET_DIR=/tmp/abutown-lodwarm-target`.

**Confirmed grounding:**
- LOD bridge `refresh_dormant_markets_system` (`economy/systems.rs`) computes `DormantMarkets` from `MarketChunks` + active/hot chunk queries. `DormantMarkets`/`MarketChunks` live in `economy/market.rs`.
- `WarmChunk` marker at `crate::world::components::WarmChunk`.
- `prorata_distribute(weights: &[i64], total: i64) -> Vec<i64>` (pub, `auction.rs`; largest-remainder, Σ output == min(total, Σ weights)).
- `checked_order_value(price: Money, qty: Quantity) -> Result<Money, EconomyError>` = `(price·qty)/1000` floored. `ECONOMY_SCALE = 1000`.
- `affordable_qty(cash: Money, price: Money) -> Result<Quantity, EconomyError>` — **private** in `pools.rs`; make `pub(crate)` and reference as `crate::economy::pools::affordable_qty`.
- `AccountBook`: `account(actor) -> MoneyAccount` (`.available`/`.locked: Money`), `deposit(actor, Money)`, `lock_cash(actor, Money)` (available→locked, checks available≥amt), `debit_locked(actor, Money)` (locked-=amt). `InventoryBook`: `balance(actor, good) -> InventoryBalance` (`.available: Quantity`), `deposit(actor, good, Quantity)`, `consume(actor, good, Quantity)` (available-=qty, checks available≥qty). All `Result<(), EconomyError>`.
- `Money(i64)`, `Quantity(i64)`, scalar `.0`. `MarketGoodKey { market, good }`. `MarketGoods.0: BTreeMap<MarketGoodKey, MarketGoodState>`, `MarketGoodState.last_settlement_price: Money`.
- `DemandPool { actor, market, good, desired_qty_per_tick: Quantity, … }`, `SupplyPool { actor, market, good, offered_qty_per_tick: Quantity, … }`. `DemandPools.0`/`SupplyPools.0: BTreeMap<EconomicActorId, …>`.
- `EconomyConfig` (in `systems.rs`) has `Default` + fields incl. `trader_default_ref_price: Money`. A literal `EconomyConfig { … }` is constructed in `tests/systems.rs` — adding a field requires updating it.
- `u64::is_multiple_of` is used in the codebase (`tick.0.is_multiple_of(10)`).
- `EconomySet` chain (`systems.rs`): `RefreshLod → ExpireOrders → Production → Traders → GeneratePoolOrders → ClearMarkets → Telemetry`. `EconomyPlugin` (`economy/mod.rs`) inserts resources incl. `DormantMarkets::default()`.

---

### Task 1: `WarmMarkets` resource + config + event + bridge classification

**Files:** `economy/market.rs`, `economy/systems.rs` (bridge + config), `economy/ledger.rs`, `economy/pools.rs`, `economy/tests/warm_flow.rs` (new), `economy/tests/mod.rs`.

- [ ] **Step 1: Write the failing bridge test** — create `economy/tests/warm_flow.rs`:

```rust
use std::collections::BTreeSet;

use bevy_ecs::prelude::*;

use crate::economy::{
    DormantMarkets, MarketChunks, MarketId, WarmMarkets, refresh_dormant_markets_system,
};
use crate::ids::ChunkCoord;
use crate::world::components::{ActiveChunk, AsleepChunk, ChunkCoordComp, WarmChunk};

#[test]
fn bridge_classifies_warm_dormant_and_active() {
    let mut world = World::new();
    world.spawn((ChunkCoordComp(ChunkCoord { x: 0, y: 0 }), AsleepChunk));
    world.spawn((ChunkCoordComp(ChunkCoord { x: 1, y: 0 }), WarmChunk));
    world.spawn((ChunkCoordComp(ChunkCoord { x: 2, y: 0 }), ActiveChunk));

    let mut anchors = MarketChunks::default();
    anchors.0.insert(MarketId(10), ChunkCoord { x: 0, y: 0 }); // asleep
    anchors.0.insert(MarketId(11), ChunkCoord { x: 1, y: 0 }); // warm
    anchors.0.insert(MarketId(12), ChunkCoord { x: 2, y: 0 }); // active
    world.insert_resource(anchors);
    world.insert_resource(DormantMarkets::default());
    world.insert_resource(WarmMarkets::default());

    let mut schedule = bevy_ecs::schedule::Schedule::default();
    schedule.add_systems(refresh_dormant_markets_system);
    schedule.run(&mut world);

    let dormant = &world.resource::<DormantMarkets>().0;
    let warm = &world.resource::<WarmMarkets>().0;
    assert_eq!(*dormant, [MarketId(10), MarketId(11)].into_iter().collect::<BTreeSet<_>>());
    assert_eq!(*warm, [MarketId(11)].into_iter().collect::<BTreeSet<_>>());
}
```

Add `mod warm_flow;` to `economy/tests/mod.rs` (between `mod transport;`/`mod traders;` — alphabetical position is cosmetic; place after `mod transport;`).

- [ ] **Step 2: Run → fail**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core bridge_classifies_warm`
Expected: FAIL — `WarmMarkets` not found.

- [ ] **Step 3: Add `WarmMarkets`** to `economy/market.rs` (next to `DormantMarkets`):

```rust
/// Markets anchored (in `MarketChunks`) to a WARM chunk — they run the cheap
/// aggregate warm-flow update instead of the full auction. Subset of
/// `DormantMarkets`. Recomputed each tick by `refresh_dormant_markets_system`.
#[derive(Resource, Default)]
pub struct WarmMarkets(pub BTreeSet<MarketId>);
```

- [ ] **Step 4: Extend the bridge** in `economy/systems.rs` — add the warm query + param + classification. Add `WarmChunk` to the `crate::world::components` import and `WarmMarkets` to the `crate::economy` import:

```rust
pub fn refresh_dormant_markets_system(
    anchors: Res<MarketChunks>,
    active_chunks: Query<&ChunkCoordComp, Or<(With<ActiveChunk>, With<HotChunk>)>>,
    warm_chunks: Query<&ChunkCoordComp, With<WarmChunk>>,
    mut dormant: ResMut<DormantMarkets>,
    mut warm: ResMut<WarmMarkets>,
) {
    let active: BTreeSet<ChunkCoord> = active_chunks.iter().map(|c| c.0).collect();
    let warm_coords: BTreeSet<ChunkCoord> = warm_chunks.iter().map(|c| c.0).collect();
    dormant.0 = anchors
        .0
        .iter()
        .filter(|(_, coord)| !active.contains(coord))
        .map(|(market, _)| *market)
        .collect();
    warm.0 = anchors
        .0
        .iter()
        .filter(|(_, coord)| warm_coords.contains(coord))
        .map(|(market, _)| *market)
        .collect();
}
```

- [ ] **Step 5: Add the config field** in `economy/systems.rs` `EconomyConfig` — add `pub warm_flow_interval_ticks: u64,` and `warm_flow_interval_ticks: 10,` in `Default`.

- [ ] **Step 6: Add the ledger event** in `economy/ledger.rs` `EconomyEvent` (after `TransportPaid`):

```rust
    WarmMarketFlow {
        market: MarketId,
        good: GoodId,
        qty: Quantity,
        price: Money,
    },
```

- [ ] **Step 7: Make `affordable_qty` reusable** — change `fn affordable_qty` to `pub(crate) fn affordable_qty` in `economy/pools.rs`.

- [ ] **Step 8: Update the `EconomyConfig` literal** in `economy/tests/systems.rs` — add `warm_flow_interval_ticks: 10,` (or whatever the test needs) to the struct literal so it compiles.

- [ ] **Step 9: Run → pass**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core bridge_classifies_warm`
Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add backend/crates/sim-core/src/economy/market.rs \
        backend/crates/sim-core/src/economy/systems.rs \
        backend/crates/sim-core/src/economy/ledger.rs \
        backend/crates/sim-core/src/economy/pools.rs \
        backend/crates/sim-core/src/economy/tests/warm_flow.rs \
        backend/crates/sim-core/src/economy/tests/mod.rs
git commit -m "feat(economy): classify WarmMarkets + warm-flow config/event"
```

---

### Task 2: the aggregate warm-flow function

**Files:** `economy/warm_flow.rs` (new), `economy/mod.rs`, `economy/tests/warm_flow.rs`.

- [ ] **Step 1: Write failing unit tests** — append to `economy/tests/warm_flow.rs`:

```rust
use crate::economy::{
    AccountBook, DemandPool, DemandPools, EconomicActorId, EconomyConfig, GOOD_FOOD, InventoryBook,
    MarketGoodKey, MarketGoodState, MarketGoods, Money, Quantity, SupplyPool, SupplyPools,
    TradeLedger, run_warm_market_flow_at_tick,
};

fn dp(actor: u64, market: MarketId, qty: i64) -> DemandPool {
    DemandPool {
        actor: EconomicActorId(actor), market, good: GOOD_FOOD,
        desired_qty_per_tick: Quantity(qty), max_price: Money(10_000),
        urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1, last_generated_tick: None,
    }
}
fn sp(actor: u64, market: MarketId, qty: i64) -> SupplyPool {
    SupplyPool {
        actor: EconomicActorId(actor), market, good: GOOD_FOOD,
        offered_qty_per_tick: Quantity(qty), min_price: Money(1),
        interval_ticks: 1, last_generated_tick: None,
    }
}
fn with_ref_price(market: MarketId, price: Money) -> MarketGoods {
    let key = MarketGoodKey { market, good: GOOD_FOOD };
    let mut mg = MarketGoods::default();
    let mut st = MarketGoodState::new(key);
    st.last_settlement_price = price;
    mg.0.insert(key, st);
    mg
}

#[test]
fn warm_flow_trades_min_at_reference_price() {
    let market = MarketId(1);
    let buyer = EconomicActorId(1);
    let seller = EconomicActorId(2);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    accounts.deposit(buyer, Money(1_000_000)).unwrap();
    inventory.deposit(seller, GOOD_FOOD, Quantity(1_000)).unwrap();
    let mut demand = DemandPools::default();
    demand.0.insert(buyer, dp(1, market, 100));
    let mut supply = SupplyPools::default();
    supply.0.insert(seller, sp(2, market, 60));
    let mg = with_ref_price(market, Money(1_000));
    let warm: BTreeSet<MarketId> = [market].into_iter().collect();
    let cfg = EconomyConfig::default();

    let money_before = accounts.total_money().unwrap();
    let goods_before = inventory.total_good(GOOD_FOOD).unwrap();

    // tick 0 is a multiple of interval (10).
    run_warm_market_flow_at_tick(
        &mut accounts, &mut inventory, &mut ledger,
        &demand, &supply, &mg, &warm, &cfg, 0,
    ).unwrap();

    assert_eq!(inventory.balance(buyer, GOOD_FOOD).available, Quantity(60));
    assert_eq!(inventory.balance(seller, GOOD_FOOD).available, Quantity(940));
    // ref price 1000, scale 1000 -> 1 money per unit -> 60 moved.
    assert_eq!(accounts.account(seller).available, Money(60));
    assert_eq!(accounts.total_money().unwrap(), money_before);
    assert_eq!(inventory.total_good(GOOD_FOOD).unwrap(), goods_before);
}

#[test]
fn warm_flow_conserves_with_two_sided_pro_rata() {
    let market = MarketId(1);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    accounts.deposit(EconomicActorId(1), Money(1_000_000)).unwrap();
    accounts.deposit(EconomicActorId(2), Money(1_000_000)).unwrap();
    inventory.deposit(EconomicActorId(3), GOOD_FOOD, Quantity(1_000)).unwrap();
    inventory.deposit(EconomicActorId(4), GOOD_FOOD, Quantity(1_000)).unwrap();
    let mut demand = DemandPools::default();
    demand.0.insert(EconomicActorId(1), dp(1, market, 100));
    demand.0.insert(EconomicActorId(2), dp(2, market, 100));
    let mut supply = SupplyPools::default();
    supply.0.insert(EconomicActorId(3), sp(3, market, 50));
    supply.0.insert(EconomicActorId(4), sp(4, market, 50));
    let mg = with_ref_price(market, Money(1_000));
    let warm: BTreeSet<MarketId> = [market].into_iter().collect();
    let cfg = EconomyConfig::default();
    let m0 = accounts.total_money().unwrap();
    let g0 = inventory.total_good(GOOD_FOOD).unwrap();

    run_warm_market_flow_at_tick(
        &mut accounts, &mut inventory, &mut ledger, &demand, &supply, &mg, &warm, &cfg, 0,
    ).unwrap();

    // demand 200, supply 100 -> Q=100; buyers split 50/50, sellers 50/50.
    assert_eq!(inventory.balance(EconomicActorId(1), GOOD_FOOD).available, Quantity(50));
    assert_eq!(inventory.balance(EconomicActorId(2), GOOD_FOOD).available, Quantity(50));
    assert_eq!(accounts.total_money().unwrap(), m0);
    assert_eq!(inventory.total_good(GOOD_FOOD).unwrap(), g0);
}

#[test]
fn warm_flow_only_fires_on_interval() {
    let market = MarketId(1);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    accounts.deposit(EconomicActorId(1), Money(1_000_000)).unwrap();
    inventory.deposit(EconomicActorId(2), GOOD_FOOD, Quantity(1_000)).unwrap();
    let mut demand = DemandPools::default();
    demand.0.insert(EconomicActorId(1), dp(1, market, 100));
    let mut supply = SupplyPools::default();
    supply.0.insert(EconomicActorId(2), sp(2, market, 60));
    let mg = with_ref_price(market, Money(1_000));
    let warm: BTreeSet<MarketId> = [market].into_iter().collect();
    let cfg = EconomyConfig::default(); // interval 10

    run_warm_market_flow_at_tick(
        &mut accounts, &mut inventory, &mut ledger, &demand, &supply, &mg, &warm, &cfg, 3,
    ).unwrap();
    assert_eq!(inventory.balance(EconomicActorId(1), GOOD_FOOD).available, Quantity(0));
}

#[test]
fn non_warm_market_does_not_flow() {
    let market = MarketId(1);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    accounts.deposit(EconomicActorId(1), Money(1_000_000)).unwrap();
    inventory.deposit(EconomicActorId(2), GOOD_FOOD, Quantity(1_000)).unwrap();
    let mut demand = DemandPools::default();
    demand.0.insert(EconomicActorId(1), dp(1, market, 100));
    let mut supply = SupplyPools::default();
    supply.0.insert(EconomicActorId(2), sp(2, market, 60));
    let mg = with_ref_price(market, Money(1_000));
    let warm: BTreeSet<MarketId> = BTreeSet::new(); // market NOT warm (e.g. asleep)
    let cfg = EconomyConfig::default();

    run_warm_market_flow_at_tick(
        &mut accounts, &mut inventory, &mut ledger, &demand, &supply, &mg, &warm, &cfg, 0,
    ).unwrap();
    assert_eq!(inventory.balance(EconomicActorId(1), GOOD_FOOD).available, Quantity(0));
}
```

- [ ] **Step 2: Run → fail**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core warm_flow_trades_min`
Expected: FAIL — `run_warm_market_flow_at_tick` not found.

- [ ] **Step 3: Create `economy/warm_flow.rs`**:

```rust
//! Warm-tier aggregate economy flow (Economy LOD). A market anchored to a WARM
//! chunk trades min(aggregate demand, aggregate supply) at its frozen reference
//! price, pro-rata, on a coarse interval — no order book, no price discovery.
//! Conservation-exact (atomic clone-validate-apply) and deterministic.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::economy::pools::affordable_qty;
use crate::economy::{
    AccountBook, DemandPools, EconomicActorId, EconomyConfig, EconomyError, EconomyEvent,
    InventoryBook, MarketGoodKey, MarketGoods, MarketId, Money, Quantity, SupplyPools, TradeLedger,
    checked_order_value, prorata_distribute,
};

fn warm_ref_price(market_goods: &MarketGoods, key: MarketGoodKey, config: &EconomyConfig) -> Money {
    match market_goods.0.get(&key) {
        Some(state) if state.last_settlement_price.0 > 0 => state.last_settlement_price,
        _ => config.trader_default_ref_price,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_warm_market_flow_at_tick(
    accounts: &mut AccountBook,
    inventory: &mut InventoryBook,
    ledger: &mut TradeLedger,
    demand: &DemandPools,
    supply: &SupplyPools,
    market_goods: &MarketGoods,
    warm_markets: &BTreeSet<MarketId>,
    config: &EconomyConfig,
    current_tick: u64,
) -> Result<(), EconomyError> {
    if config.warm_flow_interval_ticks == 0
        || !current_tick.is_multiple_of(config.warm_flow_interval_ticks)
    {
        return Ok(());
    }

    // Group warm demand/supply by market-good (deterministic BTreeMap order).
    type Side = Vec<(EconomicActorId, i64)>;
    let mut buckets: BTreeMap<MarketGoodKey, (Side, Side)> = BTreeMap::new();
    for pool in demand.0.values() {
        if warm_markets.contains(&pool.market) {
            buckets
                .entry(MarketGoodKey { market: pool.market, good: pool.good })
                .or_default()
                .0
                .push((pool.actor, pool.desired_qty_per_tick.0));
        }
    }
    for pool in supply.0.values() {
        if warm_markets.contains(&pool.market) {
            buckets
                .entry(MarketGoodKey { market: pool.market, good: pool.good })
                .or_default()
                .1
                .push((pool.actor, pool.offered_qty_per_tick.0));
        }
    }

    // Atomic clone-validate-apply (mirrors clear_market_good).
    let mut next_accounts = accounts.clone();
    let mut next_inventory = inventory.clone();
    let mut events: Vec<EconomyEvent> = Vec::new();

    for (key, (demands, supplies)) in &buckets {
        if demands.is_empty() || supplies.is_empty() {
            continue;
        }
        let price = warm_ref_price(market_goods, *key, config);
        if price.0 <= 0 {
            continue;
        }

        // Effective demand capped by affordability; supply capped by stock.
        let mut buyers: Side = Vec::new();
        for (actor, want) in demands {
            let cash = next_accounts.account(*actor).available;
            let afford = affordable_qty(cash, price)?.0;
            let eff = (*want).min(afford);
            if eff > 0 {
                buyers.push((*actor, eff));
            }
        }
        let mut sellers: Side = Vec::new();
        for (actor, offer) in supplies {
            let have = next_inventory.balance(*actor, key.good).available.0;
            let eff = (*offer).min(have);
            if eff > 0 {
                sellers.push((*actor, eff));
            }
        }
        if buyers.is_empty() || sellers.is_empty() {
            continue;
        }

        let total_demand: i64 = buyers.iter().map(|(_, q)| *q).sum();
        let total_supply: i64 = sellers.iter().map(|(_, q)| *q).sum();
        let traded = total_demand.min(total_supply);
        if traded <= 0 {
            continue;
        }

        let buyer_w: Vec<i64> = buyers.iter().map(|(_, q)| *q).collect();
        let seller_w: Vec<i64> = sellers.iter().map(|(_, q)| *q).collect();
        let buyer_goods = prorata_distribute(&buyer_w, traded);
        let seller_goods = prorata_distribute(&seller_w, traded);

        // Per-buyer floored cost; the exact sum is distributed to sellers so
        // both sides move identical cash (money conserved despite rounding).
        let mut costs: Vec<i64> = Vec::with_capacity(buyers.len());
        for goods in &buyer_goods {
            costs.push(checked_order_value(price, Quantity(*goods))?.0);
        }
        let buyers_total: i64 = costs.iter().sum();
        let seller_cash = prorata_distribute(&seller_goods, buyers_total);

        for (idx, (actor, _)) in buyers.iter().enumerate() {
            let goods = buyer_goods[idx];
            let cost = Money(costs[idx]);
            if cost.0 > 0 {
                next_accounts.lock_cash(*actor, cost)?;
                next_accounts.debit_locked(*actor, cost)?;
            }
            if goods > 0 {
                next_inventory.deposit(*actor, key.good, Quantity(goods))?;
            }
        }
        for (idx, (actor, _)) in sellers.iter().enumerate() {
            let goods = seller_goods[idx];
            if goods > 0 {
                next_inventory.consume(*actor, key.good, Quantity(goods))?;
            }
            let receipt = Money(seller_cash[idx]);
            if receipt.0 > 0 {
                next_accounts.deposit(*actor, receipt)?;
            }
        }

        events.push(EconomyEvent::WarmMarketFlow {
            market: key.market,
            good: key.good,
            qty: Quantity(traded),
            price,
        });
    }

    *accounts = next_accounts;
    *inventory = next_inventory;
    ledger.0.extend(events);
    Ok(())
}
```

- [ ] **Step 4: Wire the module** in `economy/mod.rs` — add `pub mod warm_flow;` and `pub use warm_flow::*;` alongside the others.

- [ ] **Step 5: Run → pass** (and add the remaining tests from Step 1 if not all present)

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core warm_flow`
Expected: PASS — trade-min, two-sided-conserve, interval-gate, non-warm tests.

- [ ] **Step 6: Add affordability + determinism tests** — append to `economy/tests/warm_flow.rs`:

```rust
#[test]
fn warm_flow_caps_by_affordability_and_availability() {
    let market = MarketId(1);
    let buyer = EconomicActorId(1);
    let seller = EconomicActorId(2);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    accounts.deposit(buyer, Money(30)).unwrap(); // affords 30 units at price 1000 (1/unit)
    inventory.deposit(seller, GOOD_FOOD, Quantity(20)).unwrap(); // only 20 in stock
    let mut demand = DemandPools::default();
    demand.0.insert(buyer, dp(1, market, 100));
    let mut supply = SupplyPools::default();
    supply.0.insert(seller, sp(2, market, 100));
    let mg = with_ref_price(market, Money(1_000));
    let warm: BTreeSet<MarketId> = [market].into_iter().collect();
    let cfg = EconomyConfig::default();
    let m0 = accounts.total_money().unwrap();
    let g0 = inventory.total_good(GOOD_FOOD).unwrap();

    run_warm_market_flow_at_tick(
        &mut accounts, &mut inventory, &mut ledger, &demand, &supply, &mg, &warm, &cfg, 0,
    ).unwrap();

    // min(affordable 30, stock 20) = 20 traded; conserved; no overdraw.
    assert_eq!(inventory.balance(buyer, GOOD_FOOD).available, Quantity(20));
    assert_eq!(accounts.total_money().unwrap(), m0);
    assert_eq!(inventory.total_good(GOOD_FOOD).unwrap(), g0);
    assert!(accounts.account(buyer).available.0 >= 0);
}

#[test]
fn warm_flow_is_deterministic() {
    let build = || {
        let market = MarketId(1);
        let mut accounts = AccountBook::default();
        let mut inventory = InventoryBook::default();
        let mut ledger = TradeLedger::default();
        accounts.deposit(EconomicActorId(1), Money(1_000_000)).unwrap();
        accounts.deposit(EconomicActorId(2), Money(1_000_000)).unwrap();
        inventory.deposit(EconomicActorId(3), GOOD_FOOD, Quantity(1_000)).unwrap();
        let mut demand = DemandPools::default();
        demand.0.insert(EconomicActorId(1), dp(1, market, 70));
        demand.0.insert(EconomicActorId(2), dp(2, market, 30));
        let mut supply = SupplyPools::default();
        supply.0.insert(EconomicActorId(3), sp(3, market, 90));
        let mg = with_ref_price(market, Money(1_000));
        let warm: BTreeSet<MarketId> = [market].into_iter().collect();
        run_warm_market_flow_at_tick(
            &mut accounts, &mut inventory, &mut ledger,
            &demand, &supply, &mg, &warm, &EconomyConfig::default(), 0,
        ).unwrap();
        ledger.0
    };
    assert_eq!(build(), build());
}
```

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core warm_flow`
Expected: PASS — all warm-flow unit tests.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-core/src/economy/warm_flow.rs \
        backend/crates/sim-core/src/economy/mod.rs \
        backend/crates/sim-core/src/economy/tests/warm_flow.rs
git commit -m "feat(economy): warm-tier aggregate market flow (conservation-exact)"
```

---

### Task 3: schedule wiring + plugin + e2e

**Files:** `economy/systems.rs` (set + system), `economy/mod.rs` (plugin resource), `economy/tests/warm_flow.rs` (e2e).

- [ ] **Step 1: Write the failing e2e** — append to `economy/tests/warm_flow.rs`:

```rust
use crate::economy::EconomyPlugin;
use crate::mobility::resources::Tick;
use crate::world::schedule::SimPlugin;

#[test]
fn warm_market_flows_through_the_schedule_and_conserves() {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    EconomyPlugin.install(&mut world, &mut schedule);

    let market = MarketId(1);
    let buyer = EconomicActorId(1);
    let seller = EconomicActorId(2);
    let coord = ChunkCoord { x: 9, y: 9 };
    world.spawn((ChunkCoordComp(coord), WarmChunk));
    {
        let mut acc = world.resource_mut::<AccountBook>();
        acc.deposit(buyer, Money(1_000_000)).unwrap();
    }
    {
        let mut inv = world.resource_mut::<InventoryBook>();
        inv.deposit(seller, GOOD_FOOD, Quantity(1_000)).unwrap();
    }
    {
        let mut mg = world.resource_mut::<MarketGoods>();
        let key = MarketGoodKey { market, good: GOOD_FOOD };
        let mut st = MarketGoodState::new(key);
        st.last_settlement_price = Money(1_000);
        mg.0.insert(key, st);
    }
    {
        let mut d = world.resource_mut::<DemandPools>();
        d.0.insert(buyer, dp(1, market, 50));
    }
    {
        let mut s = world.resource_mut::<SupplyPools>();
        s.0.insert(seller, sp(2, market, 50));
    }
    world.resource_mut::<MarketChunks>().0.insert(market, coord);
    world.insert_resource(Tick(0));

    let m0 = world.resource::<AccountBook>().total_money().unwrap();
    let g0 = world.resource::<InventoryBook>().total_good(GOOD_FOOD).unwrap();

    // tick 0 fires the warm flow (multiple of 10).
    schedule.run(&mut world);

    assert_eq!(world.resource::<InventoryBook>().balance(buyer, GOOD_FOOD).available, Quantity(50));
    assert_eq!(world.resource::<AccountBook>().total_money().unwrap(), m0);
    assert_eq!(world.resource::<InventoryBook>().total_good(GOOD_FOOD).unwrap(), g0);
    assert!(world.contains_resource::<crate::economy::WarmMarkets>());
}
```

- [ ] **Step 2: Run → fail**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core warm_market_flows_through_the_schedule`
Expected: FAIL — `WarmMarkets` not installed by the plugin / warm flow not wired, so no trade.

- [ ] **Step 3: Add the set + system** in `economy/systems.rs`. Add `WarmFlow` to `EconomySet` (after `ClearMarkets`, before `Telemetry`), to the `configure_sets` chain, and register the system. Add `WarmMarkets`, `run_warm_market_flow_at_tick` to the `crate::economy` import:

```rust
pub enum EconomySet {
    RefreshLod,
    ExpireOrders,
    Production,
    Traders,
    GeneratePoolOrders,
    ClearMarkets,
    WarmFlow,
    Telemetry,
}
```
In `configure_sets(( … ).chain())` insert `EconomySet::WarmFlow,` between `ClearMarkets` and `Telemetry`. In `add_systems((...))` add:
```rust
            run_warm_market_flow_system.in_set(EconomySet::WarmFlow),
```
And the system fn:
```rust
#[allow(clippy::too_many_arguments)]
pub fn run_warm_market_flow_system(
    tick: Res<Tick>,
    config: Res<EconomyConfig>,
    warm: Res<WarmMarkets>,
    mut accounts: ResMut<AccountBook>,
    mut inventory: ResMut<InventoryBook>,
    mut ledger: ResMut<TradeLedger>,
    demand: Res<DemandPools>,
    supply: Res<SupplyPools>,
    market_goods: Res<MarketGoods>,
) {
    let _ = run_warm_market_flow_at_tick(
        &mut accounts,
        &mut inventory,
        &mut ledger,
        &demand,
        &supply,
        &market_goods,
        &warm.0,
        &config,
        tick.0,
    );
}
```

- [ ] **Step 4: Insert the resource** in `economy/mod.rs` `EconomyPlugin::install` — add `world.insert_resource(WarmMarkets::default());` alongside `DormantMarkets::default()`.

- [ ] **Step 5: Run → pass**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core warm_market_flows_through_the_schedule`
Expected: PASS.

Then the full economy suite:
Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy`
Expected: PASS — all prior tests unaffected (warm flow only touches markets in `WarmMarkets`, empty everywhere else).

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/economy/systems.rs \
        backend/crates/sim-core/src/economy/mod.rs \
        backend/crates/sim-core/src/economy/tests/warm_flow.rs
git commit -m "feat(economy): run warm-market flow in the economy schedule"
```

---

### Final gate (orchestrator runs)

```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace --all-targets
```

All green. Implementer does NOT push/PR. Report per-task RED→GREEN + SHAs, `-p sim-core warm_flow` + `economy` summaries, clippy/fmt.

## Self-review notes

- **Conservation:** `Σ buyer payments == buyers_total == Σ seller receipts` (the exact buyer-paid sum is what's distributed to sellers); `Σ buyer goods == traded == Σ seller goods`. Atomic clone-commit. Affordability/availability caps prevent overdraw.
- **Determinism:** `BTreeMap` buckets + `BTreeMap` pool iteration + `prorata_distribute` stable + integer-only.
- **No interference:** warm path ignores the order book + pool `last_generated_tick`; runs on its own interval over `WarmMarkets` (disjoint from the active set the full auction handles). `DormantMarkets` gating (warm+asleep skip the full auction) is unchanged.
- **Additive:** existing suites unaffected (`WarmMarkets` empty everywhere they don't set it up). Only the `EconomyConfig` literal in `tests/systems.rs` needs the new field.
- **Type consistency:** `run_warm_market_flow_at_tick` and `_system` arg order match; `WarmMarketFlow` event fields match; `affordable_qty` `pub(crate)`.
