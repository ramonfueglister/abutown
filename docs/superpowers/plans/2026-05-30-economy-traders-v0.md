# Economy Trader Agents v0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. TDD, commit per task.

**Goal:** Aggregate traders that arbitrage a good between two markets via the auction, with abstract travel + transport cost, per `docs/superpowers/specs/2026-05-30-economy-traders-v0-design.md`. Money + goods conserved (transport cost transfers to a `TRANSPORT_OPERATOR` account).

**Branch/isolation:** worktree `/Users/ramonfuglister/Coding/abutown-traders` on `plan/economy-traders-v0` (from `origin/main` c1e9bce). `export CARGO_TARGET_DIR=/tmp/abutown-traders-target`. cargo via `scripts/cargo-serial.sh`, one at a time, `pgrep -f cargo` first. `fmt --check` each task.

## Grounding (verified)
- `EconomyError`: `Overflow, NegativeMoney, NegativeQuantity, ZeroPrice, InsufficientFunds, InsufficientGoods, InvalidOrder`.
- `AccountBook { accounts: BTreeMap<EconomicActorId, MoneyAccount> }`, `MoneyAccount { available, locked }`; methods `account`, `deposit`, `lock_cash`, `release_cash`, `debit_locked`, `total_money`.
- `MarketGoods(BTreeMap<MarketGoodKey, MarketGoodState>)`; `MarketGoodKey { market, good }`; `MarketGoodState.last_settlement_price: Money`.
- `create_bid(accounts, orders, ledger, dirty, next, current_tick, owner, market, good, qty, max_price, ttl_ticks) -> Result<OrderId, EconomyError>`; `create_ask(inventory, orders, ledger, dirty, next, current_tick, owner, market, good, qty, min_price, ttl_ticks) -> Result<OrderId>`.
- `OrderBook { bids: BTreeMap<OrderId, Bid>, asks: BTreeMap<OrderId, Ask> }`.
- `transport_cost(distance_tiles: i64, qty: Quantity, rate: Money) -> Result<Money, EconomyError>` (economy/transport.rs).
- `EconomyConfig` (systems.rs): `ewma_alpha_bps, default_order_ttl_ticks, transport_cost_per_tile_unit: Money` + `Default`. Normal `Res/ResMut` systems; `EconomySet { ExpireOrders, Production, GeneratePoolOrders, ClearMarkets, Telemetry }`, chained, `.before(crate::mobility::systems::tick_increment_system)`. `Tick = crate::mobility::resources::Tick`.

---

### Task 1: `AccountBook::transfer` + `TRANSPORT_OPERATOR`

**Files:** Modify `economy/accounts.rs`; Modify `economy/tests/locking.rs`.

- [ ] **Step 1: failing tests** (append to tests/locking.rs):
```rust
#[test]
fn transfer_moves_available_and_conserves_total() {
    let a = EconomicActorId(1);
    let b = EconomicActorId(2);
    let mut acc = AccountBook::default();
    acc.deposit(a, Money(1_000)).unwrap();
    let before = acc.total_money().unwrap();
    acc.transfer(a, b, Money(400)).unwrap();
    assert_eq!(acc.account(a).available, Money(600));
    assert_eq!(acc.account(b).available, Money(400));
    assert_eq!(acc.total_money().unwrap(), before);
}
#[test]
fn cannot_transfer_more_than_available() {
    let mut acc = AccountBook::default();
    acc.deposit(EconomicActorId(1), Money(100)).unwrap();
    assert_eq!(acc.transfer(EconomicActorId(1), EconomicActorId(2), Money(200)), Err(EconomyError::InsufficientFunds));
}
#[test]
fn cannot_transfer_negative() {
    let mut acc = AccountBook::default();
    assert_eq!(acc.transfer(EconomicActorId(1), EconomicActorId(2), Money(-1)), Err(EconomyError::NegativeMoney));
}
```
RUN `... -p sim-core transfer` → FAIL.

- [ ] **Step 2: implement** in `impl AccountBook`:
```rust
    pub fn transfer(&mut self, from: EconomicActorId, to: EconomicActorId, amount: Money) -> Result<(), EconomyError> {
        if amount.0 < 0 {
            return Err(EconomyError::NegativeMoney);
        }
        let mut f = self.account(from);
        if f.available < amount {
            return Err(EconomyError::InsufficientFunds);
        }
        f.available = f.available.checked_sub(amount)?;
        let mut t = self.account(to);
        t.available = t.available.checked_add(amount)?;
        self.accounts.insert(from, f);
        self.accounts.insert(to, t);
        Ok(())
    }
```
- [ ] **Step 3: RUN** → PASS; clippy/fmt clean. **Commit** `feat(economy): AccountBook::transfer (conserving cash move)`.

---

### Task 2: `TransportPaid` event + trader config

**Files:** Modify `economy/ledger.rs`, `economy/systems.rs`.

- [ ] **Step 1:** add to `EconomyEvent`: `TransportPaid { actor: EconomicActorId, amount: Money },`. Add to `EconomyConfig`: `pub trader_tiles_per_tick: u64,` and `pub trader_default_ref_price: Money,`; in `Default`: `trader_tiles_per_tick: 4, trader_default_ref_price: Money(1_000),`. Update any existing `EconomyConfig { .. }` literal in tests to include the new fields. RUN `cargo build -p sim-core`; clippy/fmt clean.
- [ ] **Step 2: commit** `feat(economy): TransportPaid event + trader config fields`.

---

### Task 3: `traders.rs` — trader state machine

**Files:** Create `economy/traders.rs`; Modify `economy/mod.rs` (`pub mod traders; pub use traders::*;`); Create `economy/tests/traders.rs`; Modify `economy/tests/mod.rs` (`mod traders;`).

- [ ] **Step 1: failing unit tests** — `tests/traders.rs`:
```rust
use crate::economy::{
    adjust_price, run_traders_at_tick, transport_ticks, AccountBook, DirtyMarketGoods,
    EconomicActorId, EconomyEvent, GOOD_TOOLS, InventoryBook, MarketGoods, MarketId, Money,
    NextOrderId, OrderBook, Quantity, Trader, TraderState, Traders, TradeLedger, TRANSPORT_OPERATOR,
};

fn trader(actor: EconomicActorId) -> Trader {
    Trader {
        actor, good: GOOD_TOOLS, source: MarketId(1), dest: MarketId(2),
        distance_tiles: 10, batch_qty: Quantity(1_000),
        buy_premium_bps: 0, sell_discount_bps: 0, order_ttl_ticks: 100,
        state: TraderState::Buying { order: None },
    }
}

#[test]
fn adjust_price_applies_bps() {
    assert_eq!(adjust_price(Money(1_000), 0).unwrap(), Money(1_000));
    assert_eq!(adjust_price(Money(1_000), 2_500).unwrap(), Money(1_250)); // +25%
    assert_eq!(adjust_price(Money(1_000), -2_000).unwrap(), Money(800));  // -20%
}

#[test]
fn transport_ticks_is_at_least_one() {
    let cfg = crate::economy::EconomyConfig { trader_tiles_per_tick: 4, ..Default::default() };
    assert_eq!(transport_ticks(10, &cfg), 2); // 10/4 = 2
    assert_eq!(transport_ticks(1, &cfg), 1);  // floor 0 -> max(1)
}

#[test]
fn buying_places_a_bid_when_short() {
    let actor = EconomicActorId(1);
    let mut accounts = AccountBook::default();
    accounts.deposit(actor, Money(100_000)).unwrap();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let goods = MarketGoods::default();
    let mut traders = Traders::default();
    traders.0.insert(actor, trader(actor));
    let cfg = crate::economy::EconomyConfig::default();

    run_traders_at_tick(&mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next, &goods, &mut traders, &cfg, 0).unwrap();

    assert_eq!(orders.bids.len(), 1); // placed a buy bid at source
    assert!(matches!(traders.0[&actor].state, TraderState::Buying { order: Some(_) }));
}

#[test]
fn acquired_goods_trigger_travel_and_transport_payment() {
    let actor = EconomicActorId(1);
    let mut accounts = AccountBook::default();
    accounts.deposit(actor, Money(100_000)).unwrap();
    let mut inventory = InventoryBook::default();
    inventory.deposit(actor, GOOD_TOOLS, Quantity(1_000)).unwrap(); // already holds a batch
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let goods = MarketGoods::default();
    let mut traders = Traders::default();
    let mut t = trader(actor);
    t.state = TraderState::Buying { order: Some(crate::economy::OrderId(1)) };
    traders.0.insert(actor, t);
    let cfg = crate::economy::EconomyConfig::default(); // transport_cost_per_tile_unit = Money(5)

    let before = accounts.total_money().unwrap();
    run_traders_at_tick(&mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next, &goods, &mut traders, &cfg, 0).unwrap();

    assert!(matches!(traders.0[&actor].state, TraderState::ToDest { .. }));
    // transport cost = transport_cost(10, Quantity(1000), Money(5)) = (5*1000/1000)*10 = 50
    assert_eq!(accounts.account(TRANSPORT_OPERATOR).available, Money(50));
    assert!(ledger.0.iter().any(|e| matches!(e, EconomyEvent::TransportPaid { amount, .. } if *amount == Money(50))));
    assert_eq!(accounts.total_money().unwrap(), before); // conserved (trader -> operator)
}

#[test]
fn travel_counts_down_then_sells() {
    let actor = EconomicActorId(1);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    inventory.deposit(actor, GOOD_TOOLS, Quantity(1_000)).unwrap();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let goods = MarketGoods::default();
    let mut traders = Traders::default();
    let mut t = trader(actor);
    t.state = TraderState::ToDest { remaining: 2 };
    traders.0.insert(actor, t);
    let cfg = crate::economy::EconomyConfig::default();
    let mut run = || run_traders_at_tick(&mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next, &goods, &mut traders, &cfg, 0).unwrap();
    run(); assert!(matches!(traders.0[&actor].state, TraderState::ToDest { remaining: 1 }));
    run(); assert!(matches!(traders.0[&actor].state, TraderState::Selling { .. }));
    run(); // Selling: holds goods -> places ask
    assert_eq!(orders.asks.len(), 1);
}

#[test]
fn traders_are_deterministic() {
    let build = || {
        let mut accounts = AccountBook::default();
        let mut inventory = InventoryBook::default();
        let mut orders = OrderBook::default();
        let mut ledger = TradeLedger::default();
        let mut dirty = DirtyMarketGoods::default();
        let mut next = NextOrderId::default();
        let goods = MarketGoods::default();
        let mut traders = Traders::default();
        for id in [2u64, 1u64] {
            let a = EconomicActorId(id);
            accounts.deposit(a, Money(100_000)).unwrap();
            traders.0.insert(a, trader(a));
        }
        let cfg = crate::economy::EconomyConfig::default();
        run_traders_at_tick(&mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next, &goods, &mut traders, &cfg, 0).unwrap();
        ledger.0
    };
    assert_eq!(build(), build());
}
```
RUN → FAIL (missing types/fns).

- [ ] **Step 2: implement** `traders.rs`:
```rust
use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::economy::{
    create_ask, create_bid, transport_cost, AccountBook, DirtyMarketGoods, EconomicActorId,
    EconomyConfig, EconomyError, EconomyEvent, GoodId, InventoryBook, MarketGoodKey, MarketGoods,
    MarketId, Money, NextOrderId, OrderBook, OrderId, Quantity, TradeLedger,
};

/// Reserved account that receives transport-cost payments (keeps money conserved).
pub const TRANSPORT_OPERATOR: EconomicActorId = EconomicActorId(u64::MAX);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraderState {
    Buying { order: Option<OrderId> },
    ToDest { remaining: u64 },
    Selling { order: Option<OrderId> },
    ToSource { remaining: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trader {
    pub actor: EconomicActorId,
    pub good: GoodId,
    pub source: MarketId,
    pub dest: MarketId,
    pub distance_tiles: i64,
    pub batch_qty: Quantity,
    pub buy_premium_bps: i32,
    pub sell_discount_bps: i32,
    pub order_ttl_ticks: u64,
    pub state: TraderState,
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct Traders(pub BTreeMap<EconomicActorId, Trader>);

/// price * (10000 + bps) / 10000, checked i128. Result must stay positive.
pub fn adjust_price(price: Money, bps: i32) -> Result<Money, EconomyError> {
    let factor = 10_000_i128 + bps as i128;
    if factor <= 0 {
        return Err(EconomyError::ZeroPrice);
    }
    let raw = (price.0 as i128).checked_mul(factor).ok_or(EconomyError::Overflow)? / 10_000;
    let out = i64::try_from(raw).map_err(|_| EconomyError::Overflow)?;
    if out <= 0 {
        return Err(EconomyError::ZeroPrice);
    }
    Ok(Money(out))
}

pub fn transport_ticks(distance_tiles: i64, config: &EconomyConfig) -> u64 {
    let per = config.trader_tiles_per_tick.max(1);
    ((distance_tiles.max(0) as u64) / per).max(1)
}

fn ref_price(market_goods: &MarketGoods, market: MarketId, good: GoodId, config: &EconomyConfig) -> Money {
    market_goods
        .0
        .get(&MarketGoodKey { market, good })
        .map(|s| s.last_settlement_price)
        .filter(|p| p.0 > 0)
        .unwrap_or(config.trader_default_ref_price)
}

#[allow(clippy::too_many_arguments)]
pub fn run_traders_at_tick(
    accounts: &mut AccountBook,
    inventory: &mut InventoryBook,
    orders: &mut OrderBook,
    ledger: &mut TradeLedger,
    dirty: &mut DirtyMarketGoods,
    next: &mut NextOrderId,
    market_goods: &MarketGoods,
    traders: &mut Traders,
    config: &EconomyConfig,
    current_tick: u64,
) -> Result<(), EconomyError> {
    let actors: Vec<EconomicActorId> = traders.0.keys().copied().collect();
    for actor in actors {
        let mut trader = traders.0[&actor].clone();
        match trader.state {
            TraderState::Buying { order } => {
                let held = inventory.balance(actor, trader.good).available;
                if held >= trader.batch_qty {
                    let cost = transport_cost(trader.distance_tiles, trader.batch_qty, config.transport_cost_per_tile_unit)?;
                    if cost.0 > 0 {
                        accounts.transfer(actor, TRANSPORT_OPERATOR, cost)?;
                        ledger.0.push(EconomyEvent::TransportPaid { actor, amount: cost });
                    }
                    trader.state = TraderState::ToDest { remaining: transport_ticks(trader.distance_tiles, config) };
                } else {
                    let need = match order {
                        None => true,
                        Some(id) => !orders.bids.contains_key(&id),
                    };
                    if need {
                        let price = adjust_price(ref_price(market_goods, trader.source, trader.good, config), trader.buy_premium_bps)?;
                        let want = Quantity(trader.batch_qty.0 - held.0);
                        let id = create_bid(accounts, orders, ledger, dirty, next, current_tick, actor, trader.source, trader.good, want, price, trader.order_ttl_ticks)?;
                        trader.state = TraderState::Buying { order: Some(id) };
                    }
                }
            }
            TraderState::ToDest { remaining } => {
                trader.state = if remaining <= 1 {
                    TraderState::Selling { order: None }
                } else {
                    TraderState::ToDest { remaining: remaining - 1 }
                };
            }
            TraderState::Selling { order } => {
                let held = inventory.balance(actor, trader.good).available;
                if held.0 == 0 {
                    trader.state = TraderState::ToSource { remaining: transport_ticks(trader.distance_tiles, config) };
                } else {
                    let need = match order {
                        None => true,
                        Some(id) => !orders.asks.contains_key(&id),
                    };
                    if need {
                        let price = adjust_price(ref_price(market_goods, trader.dest, trader.good, config), -trader.sell_discount_bps)?;
                        let id = create_ask(inventory, orders, ledger, dirty, next, current_tick, actor, trader.dest, trader.good, held, price, trader.order_ttl_ticks)?;
                        trader.state = TraderState::Selling { order: Some(id) };
                    }
                }
            }
            TraderState::ToSource { remaining } => {
                trader.state = if remaining <= 1 {
                    TraderState::Buying { order: None }
                } else {
                    TraderState::ToSource { remaining: remaining - 1 }
                };
            }
        }
        traders.0.insert(actor, trader);
    }
    Ok(())
}
```
Add `pub mod traders; pub use traders::*;` to mod.rs; `mod traders;` to tests/mod.rs. (Note: `held` in Selling — `create_ask` locks `held` goods; if a prior ask is still live it isn't re-placed. When the ask fully fills, held→0 → ToSource.)

- [ ] **Step 3: RUN** `cargo test -p sim-core traders` → PASS; clippy/fmt clean. **Commit** `feat(economy): trader state machine (buy@source -> travel -> sell@dest)`.

---

### Task 4: schedule wiring + end-to-end cycle test

**Files:** Modify `economy/systems.rs` (`EconomySet::Traders`, `run_traders_system`, chain), `economy/mod.rs` (insert `Traders::default()`), Modify `economy/tests/systems.rs`.

- [ ] **Step 1: failing e2e test** (append to tests/systems.rs):
```rust
#[test]
fn trader_arbitrages_between_markets_end_to_end() {
    use crate::economy::{
        DemandPool, DemandPools, SupplyPool, SupplyPools, Trader, TraderState, Traders,
        TRANSPORT_OPERATOR,
    };
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    let trader = EconomicActorId(1);
    let supplier = EconomicActorId(2); // sells cheap at source M1
    let consumer = EconomicActorId(3); // buys dear at dest M2
    let src = MarketId(1);
    let dst = MarketId(2);

    world.resource_mut::<AccountBook>().deposit(trader, Money(1_000_000)).unwrap();
    world.resource_mut::<AccountBook>().deposit(consumer, Money(1_000_000)).unwrap();
    world.resource_mut::<InventoryBook>().deposit(supplier, GOOD_TOOLS, Quantity(100_000)).unwrap();
    // supplier offers TOOLS cheap at M1
    world.resource_mut::<SupplyPools>().0.insert(supplier, SupplyPool {
        actor: supplier, market: src, good: GOOD_TOOLS, offered_qty_per_tick: Quantity(1_000),
        min_price: Money(800), interval_ticks: 1, last_generated_tick: None,
    });
    // consumer demands TOOLS dear at M2
    world.resource_mut::<DemandPools>().0.insert(consumer, DemandPool {
        actor: consumer, market: dst, good: GOOD_TOOLS, desired_qty_per_tick: Quantity(1_000),
        max_price: Money(2_000), urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1, last_generated_tick: None,
    });
    world.resource_mut::<Traders>().0.insert(trader, Trader {
        actor: trader, good: GOOD_TOOLS, source: src, dest: dst, distance_tiles: 4,
        batch_qty: Quantity(1_000), buy_premium_bps: 5_000, sell_discount_bps: 5_000,
        order_ttl_ticks: 50, state: TraderState::Buying { order: None },
    });

    let total_money_before = {
        let a = world.resource::<AccountBook>();
        a.total_money().unwrap()
    };
    for _ in 0..40 {
        schedule.run(&mut world);
    }

    // The trader paid transport at least once (completed a buy->travel leg).
    assert!(world.resource::<AccountBook>().account(TRANSPORT_OPERATOR).available.0 > 0);
    // A trade happened at the destination (trader sold to consumer): consumer holds TOOLS.
    assert!(world.resource::<InventoryBook>().balance(consumer, GOOD_TOOLS).available.0 > 0);
    // Money conserved across all accounts (incl. operator).
    assert_eq!(world.resource::<AccountBook>().total_money().unwrap(), total_money_before);
}
```
(Add imports as needed: `AccountBook`, `InventoryBook`, `GOOD_TOOLS`, `Money`, `Quantity`, `MarketId`, `EconomicActorId`.) RUN → FAIL (no `Traders` resource / system).

- [ ] **Step 2: implement.** In systems.rs: add `Traders` to `EconomySet` between `Production` and `GeneratePoolOrders`; add:
```rust
#[allow(clippy::too_many_arguments)]
pub fn run_traders_system(
    tick: Res<Tick>,
    config: Res<EconomyConfig>,
    mut accounts: ResMut<AccountBook>,
    mut inventory: ResMut<InventoryBook>,
    mut orders: ResMut<OrderBook>,
    mut ledger: ResMut<TradeLedger>,
    mut dirty: ResMut<DirtyMarketGoods>,
    mut next: ResMut<NextOrderId>,
    market_goods: Res<MarketGoods>,
    mut traders: ResMut<Traders>,
) {
    let _ = run_traders_at_tick(&mut accounts, &mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next, &market_goods, &mut traders, &config, tick.0);
}
```
(import `run_traders_at_tick`, `Traders`, `EconomyConfig`, the books/order types as needed.) Add to `configure_sets((... EconomySet::Traders ...).chain())` (between Production and GeneratePoolOrders) and `add_systems((... run_traders_system.in_set(EconomySet::Traders) ...))`. In mod.rs `EconomyPlugin::install`: `world.insert_resource(Traders::default());`.

- [ ] **Step 3: RUN** `cargo test -p sim-core trader_arbitrages_between_markets_end_to_end` → PASS, then full `cargo test -p sim-core economy` green. clippy/fmt clean. **Commit** `feat(economy): run traders in the economy schedule`.

---

### Task 5: Final gate
- [ ] fmt --check · clippy --workspace --all-targets -D warnings · test --workspace · build -p sim-server (all green)
- [ ] (orchestrator) PR → CI green → merge → cleanup.

## Deferred
Materialize traders as mobility agents (slice 4 LOD); dynamic routing/recipe choice; multi-good; partial-batch optimization; transport cost as a real good/fuel.
