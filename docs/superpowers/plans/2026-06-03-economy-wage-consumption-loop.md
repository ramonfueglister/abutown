# Implementation Plan: SFC Wage / Consumption Loop — close the money circuit

## Goal

Close the demand-side money circuit in the economy. Today money is minted exactly once (the `Money(1_000_000)` consumer-pool seed); the three consumer actors (8_002, 8_012, 8_022) only ever *buy*, so consumer cash falls monotonically to `InsufficientFunds` and trade freezes. This slice makes firms pay a labor share of revenue to a household sector (income), and makes households consume out of current income (a Keynesian consumption function `C = a + b·Y`). Money stays **byte-invariant** (every move is `AccountBook::transfer`, never mint/burn), the authority is **O(sectors)**, and visible commuters project the wage flow.

## Architecture

- **Income side (Part A):** a per-tick ephemeral `SellerReceipts` accumulator captures gross revenue per `(firm, market)` at the two settle points (auction + macro flow). `run_pay_wages_at_tick` reads it, computes `wage = revenue · labor_share_bps / 10_000` per firm, runs a conservative two-leg transfer `firm → HOUSEHOLD_SECTOR → consumer pools` (largest-remainder split, nets the sentinel to zero), and records each pool's `income_last_tick`.
- **Consumption side (Part B):** `run_consumption_update_system` reads each pool's `income_last_tick` + smoothed `ewma_reference_price`, computes `target_spend(autonomous, mpc_bps, income)`, maps it to `desired_qty_per_tick` via `spend_to_qty`. The 1-tick lag (income booked in tick T drives bids in tick T+1) is the Lengnick period structure and avoids same-tick circularity.
- **Projection:** `commuters.rs` mirrors `shoppers.rs` — a pure `capture_commuter_trips` reads ephemeral `WageTelemetry` (wage paid per market) and spawns render-only commuter agents; never persisted, regenerated on restart.
- **Conservation discipline:** fixed-point i64/i128, floor division, keys-first BTreeMap iteration, no float, no RNG, ties-by-ascending-index. Every money field touched only via the existing conservation-proven `transfer`/`lock`/`debit` paths.

## Tech Stack

- Rust, `bevy_ecs` (resources + `SystemSet` `.chain()` scheduling), `serde` (snapshot Vec<(K,V)>).
- Crate: `backend/crates/sim-core`, module `src/economy/`.
- Money/Quantity are `i64` newtypes (`ECONOMY_SCALE: i128 = 1_000`); intermediates in `i128` with `i64::try_from(...) → EconomyError::Overflow`.
- Tests run via the serial wrapper: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core <filter>`.

**Verified facts about the real code (the implementer must hold these as ground truth):**

- `EconomicActorId(pub u64)`, `MarketId(pub u32)`, `GoodId(pub u16)` (so `GoodId(0)` is valid), `Money(pub i64)`, `Quantity(pub i64)`. `Money::ZERO`, `Quantity::ZERO`, `Money::checked_add/checked_sub` exist.
- `AccountBook::{account, deposit, lock_cash, debit_locked, transfer, total_money}` — `transfer(from, to, amount)` is the conservation-proven two-leg move; `account(actor)` returns a copied `MoneyAccount{available, locked}` (default-zero if absent).
- `apportion_cash(weights: &[i64], total: i64) -> Vec<i64>` lives in `auction.rs`, is `pub`, re-exported as `crate::economy::apportion_cash` (sum-preserving largest-remainder, ties by ascending index).
- `TRANSPORT_OPERATOR = EconomicActorId(u64::MAX)` (transport.rs); `SHIPMENT_ACTOR_OFFSET = 1 << 32` (flow_shipments.rs); `SHOPPER_ACTOR_OFFSET = 2 << 32` (shoppers.rs). New: `HOUSEHOLD_SECTOR = EconomicActorId(u64::MAX - 1)`; `COMMUTER_ACTOR_OFFSET = 3 << 32`.
- `EconomyConfig` derives `#[derive(Resource, Debug, Clone, Copy, PartialEq)]` (NO `Eq`); adding `u16`/`i64`/`usize` fields preserves `Copy`. No test asserts config equality.
- `EconomyError` variants: `Overflow, NegativeMoney, NegativeQuantity, ZeroPrice, InsufficientFunds, InsufficientGoods, InvalidOrder`.
- `MarketGoodState` fields: `key, last_settlement_price, ewma_reference_price, traded_qty_last_tick, unmet_demand_last_tick, unsold_supply_last_tick, consumed_qty_last_tick, dirty, last_cleared_tick`; `MarketGoodState::new(key)` zeroes prices.
- **`EconomyPlugin::install` does NOT call `seed_demo_economy`** — every wired-schedule test installs `CorePlugin::default()` then `crate::mobility::MobilityPlugin` then `EconomyPlugin`, and seeds state manually. `Tick(pub u64)` lives in `crate::mobility::resources` and is inserted by `MobilityPlugin`; `tick_increment_system` is `crate::mobility::systems::tick_increment_system` (referenced by `.before(...)` in `install_systems`). **There is NO EconomyPlugin-only schedule.run in the repo, and `install_systems` references `crate::world::schedule::CoreSet::LodReclassify` (`.after`) and `tick_increment_system` (`.before`). Any full-tick test in this slice MUST install CorePlugin + MobilityPlugin + EconomyPlugin (NOT EconomyPlugin alone) and use the MobilityPlugin-provided `Tick`.**
- `settle_flow` signature order: `(accounts, inventory, market_goods, flow, sellers, buyers, eff_demand_src, eff_supply_src, eff_demand_dst, eff_supply_dst, config, current_tick, preserve_price_src, preserve_price_dst)`.
- `run_macro_flow_at_tick` commit block (macro_flow.rs:1018-1023): `*accounts; *inventory; *market_goods; *orders; *next_order_id; ledger.0.extend(events)`. The per-edge Ok-fold (lines 984-1006) assigns `next_accounts = scratch_accounts; next_inventory = scratch_inventory; next_goods = scratch_goods; next_orders = scratch_orders;` then pushes the shipment + event.
- `clear_market_good_with_policy` body is auction.rs:332-445; the deposit site is `next_accounts.deposit(ask.owner, actual_cost)?;` (line 400); commit block is lines 432-443.
- materialize.rs `resource_scope` closure (lines 407-453) binds, in scope: `world` (param), `cache: Mut<FlowFieldCache>` (param), `graph`, `hpa`, `markets`, and `out`. `leg_polyline(graph, hpa, &mut cache, from, to)`. `id_prefix` handles only shopper/trader. `rendering_shopper_ids` currently does an UNBOUNDED `checked_sub(SHOPPER_ACTOR_OFFSET)`.
- tests/pools.rs already imports (line 3-8): `AccountBook, DemandPool, DemandPools, DirtyMarketGoods, EconomicActorId, EconomyEvent, GOOD_FOOD, InventoryBook, MarketGoodKey, MarketGoodState, MarketGoods, MarketId, Money, NextOrderId, OrderBook, Quantity, SupplyPool, SupplyPools, TradeLedger, generate_pool_orders_at_tick, run_consumption_at_tick`. It does NOT import `GOOD_TOOLS`, `EconomyConfig`, `EconomyError`.
- `DemandPool { ... }` literals across the crate (exact set to update for new fields): `seed.rs` (3), `tests/flow_shipments.rs` (2), `tests/persist.rs` (1 inside `seed`), `tests/macro_flow.rs` (7), `tests/systems.rs` (1, in `seed_trading_pair`), `tests/pools.rs` (4, in 3 inline + `consume_pool` helper), `tests/lod.rs` (4). The `consume_pool` helper (tests/pools.rs) and `seed_trading_pair` (tests/systems.rs) are functions — update them once each.
- Frontend gate commands (from `package.json`): `npm run typecheck` (= `tsc -p tsconfig.typecheck.json`), `npm run test` (= `vitest run`), `npm run build` (= proto-gen + `scripts/build.mjs`), `npm run test:e2e` (= build + playwright). Shopper smoke template: `scripts/smoke-shoppers.mjs`.

**Reference reading (already done by the planner; the implementer should skim before starting):** `economy/pools.rs`, `economy/accounts.rs`, `economy/auction.rs`, `economy/macro_flow.rs`, `economy/systems.rs`, `economy/market.rs`, `economy/persist.rs`, `economy/seed.rs`, `economy/ledger.rs`, `economy/shoppers.rs`, `economy/materialize.rs`, `economy/tests/{pools,systems,lod,plugin,persist,macro_flow}.rs`.

**Migration (whole slice in ONE PR):** three new non-default `DemandPool` fields + one new top-level snapshot field (`household_sector`) ⇒ old `economy_snapshots` rows fail to deserialize (the #69 `market_distances` precedent). **Run `DELETE FROM economy_snapshots` once before deploy.** No serde-default shims.

**Canonical-interface vs spec §13 note:** the spec §13 table lists `WageTelemetry` under `commuters.rs`, but the canonical interface (which this plan follows) puts `WageTelemetry` AND `HouseholdSector` in `wages.rs`. This is intentional: tests and `commuters.rs` import `WageTelemetry` from `crate::economy` (re-exported via `pub use wages::*`), so the physical module is invisible to callers. Follow the canonical interface; the §13 row is the inconsistency.

**Commuter origin radius note:** `capture_commuter_trips`'s origin provider reuses `config.shopper_radius_tiles` (the spec adds no commuter-specific radius, and the canonical interface adds only `commuters_per_wage_unit` + `max_commuters_per_market`). This is an intentional, documented reuse, not a missing knob.

---

## Task 1 — `SellerReceipts` + `ResetReceipts` + both write sites

Introduce the ephemeral per-tick revenue accumulator and wire it into the auction and macro-flow settle paths plus a tick-start reset. Nothing reads it yet; this task is verified purely by conservation-neutrality and population of the map.

### Files
- **Create** `backend/crates/sim-core/src/economy/wages.rs` — `HOUSEHOLD_SECTOR`, `SellerReceipts` (only these two for now).
- **Modify** `backend/crates/sim-core/src/economy/mod.rs` — `pub mod wages;`, `pub use wages::*;`, insert `SellerReceipts` in `EconomyPlugin::install`.
- **Modify** `backend/crates/sim-core/src/economy/auction.rs` — add `clear_market_good_with_receipts`; make `clear_market_good_with_policy` delegate.
- **Modify** `backend/crates/sim-core/src/economy/macro_flow.rs` — add `settle_flow_with_receipts`; make `settle_flow` delegate; thread receipts through `run_macro_flow_at_tick`.
- **Modify** `backend/crates/sim-core/src/economy/systems.rs` — `EconomySet::ResetReceipts`; reset system; pass `SellerReceipts` into `clear_dirty_markets_system` + `run_macro_flow_system`.
- **Modify** `backend/crates/sim-core/src/economy/tests/mod.rs` — `mod wages;`.
- **Create** `backend/crates/sim-core/src/economy/tests/wages.rs` — receipts-capture + conservation tests.

### Test
`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core wages::`

### Steps

- [ ] **Step 1.1 — Write the failing test for the auction write site.**

  Create `backend/crates/sim-core/src/economy/tests/wages.rs` with a SINGLE consolidated top-of-file import block (every later step ADDS to this block via `Edit`, never a second `use crate::economy::{...}`):

  ```rust
  use std::collections::BTreeMap;

  use crate::economy::auction::SettlementPolicy;
  use crate::economy::{
      AccountBook, DirtyMarketGoods, EconomicActorId, GOOD_FOOD, InventoryBook, MarketGoodKey,
      MarketGoodState, MarketGoods, MarketId, Money, NextOrderId, OrderBook, Quantity,
      SellerReceipts, TradeLedger, clear_market_good_with_receipts, create_ask, create_bid,
  };

  fn seeded_state(market: MarketId) -> MarketGoodState {
      MarketGoodState {
          key: MarketGoodKey { market, good: GOOD_FOOD },
          last_settlement_price: Money(1_100),
          ewma_reference_price: Money(1_100),
          traded_qty_last_tick: Quantity(0),
          unmet_demand_last_tick: Quantity(0),
          unsold_supply_last_tick: Quantity(0),
          consumed_qty_last_tick: Quantity::ZERO,
          dirty: true,
          last_cleared_tick: 0,
      }
  }

  #[test]
  fn auction_captures_seller_revenue_into_receipts() {
      let buyer = EconomicActorId(1);
      let seller = EconomicActorId(2);
      let market = MarketId(1);
      let key = MarketGoodKey { market, good: GOOD_FOOD };
      let mut accounts = AccountBook::default();
      let mut inventory = InventoryBook::default();
      let mut orders = OrderBook::default();
      let mut ledger = TradeLedger::default();
      let mut dirty = DirtyMarketGoods::default();
      let mut next = NextOrderId::default();
      let mut goods = MarketGoods::default();
      goods.0.insert(key, seeded_state(market));
      accounts.deposit(buyer, Money(10_000)).unwrap();
      inventory.deposit(seller, GOOD_FOOD, Quantity(2_000)).unwrap();
      create_bid(&mut accounts, &mut orders, &mut ledger, &mut dirty, &mut next, 1, buyer, market, GOOD_FOOD, Quantity(1_000), Money(1_500), 10).unwrap();
      create_ask(&mut inventory, &mut orders, &mut ledger, &mut dirty, &mut next, 1, seller, market, GOOD_FOOD, Quantity(1_000), Money(1_000), 10).unwrap();

      let before = accounts.total_money().unwrap();
      let mut receipts = SellerReceipts::default();
      clear_market_good_with_receipts(
          &mut accounts, &mut inventory, &mut orders, &mut ledger, &mut goods, key, 2,
          SettlementPolicy::Anchored, &mut receipts.0,
      )
      .unwrap();

      // Settles at last=1100 (within band) → 1000 units * 1.100 = 1_100 money to the seller.
      assert_eq!(accounts.total_money().unwrap(), before, "money conserved");
      assert_eq!(receipts.0.get(&(seller, market)).copied(), Some(Money(1_100)));
      assert_eq!(receipts.0.get(&(buyer, market)).copied(), None, "buyers are not credited");
  }

  #[test]
  fn auction_receipts_discarded_on_fault_are_coherent() {
      let market = MarketId(7);
      let key = MarketGoodKey { market, good: GOOD_FOOD };
      let mut accounts = AccountBook::default();
      let mut inventory = InventoryBook::default();
      let mut orders = OrderBook::default();
      let mut ledger = TradeLedger::default();
      let mut goods = MarketGoods::default();
      goods.0.insert(key, seeded_state(market));
      let mut receipts = SellerReceipts::default();
      clear_market_good_with_receipts(
          &mut accounts, &mut inventory, &mut orders, &mut ledger, &mut goods, key, 1,
          SettlementPolicy::Anchored, &mut receipts.0,
      )
      .unwrap();
      assert!(receipts.0.is_empty(), "no fills → no receipts");
  }
  ```

  Run it; expect a **compile failure** (`SellerReceipts`, `clear_market_good_with_receipts` do not exist).

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core wages::auction` → **FAIL (does not compile)**.

- [ ] **Step 1.2 — Create `wages.rs` with the sentinel + resource.**

  Create `backend/crates/sim-core/src/economy/wages.rs`:

  ```rust
  //! The SFC wage / income side of the economy: per-tick seller revenue capture
  //! (`SellerReceipts`), the household clearing sentinel (`HOUSEHOLD_SECTOR`), and
  //! (added later) the conservative two-leg wage transfer. Money is byte-invariant:
  //! every move is an `AccountBook::transfer`; the wage sentinel nets to zero each tick.

  use std::collections::BTreeMap;

  use bevy_ecs::prelude::*;

  use crate::economy::{EconomicActorId, MarketId, Money};

  /// Reserved clearing-account sentinel for the household sector, adjacent to
  /// `TRANSPORT_OPERATOR = EconomicActorId(u64::MAX)`. Firms pay wages INTO this
  /// account; it is fully apportioned out to consumer pools in the same tick, so
  /// it nets to ZERO every PayWages (asserted in debug). Distinct from every
  /// seeded id (8_001..8_022) and the actor-offset bands (`n << 32`).
  pub const HOUSEHOLD_SECTOR: EconomicActorId = EconomicActorId(u64::MAX - 1);

  /// Gross sales revenue credited to each `(firm, market)` THIS tick. A non-monetary
  /// running statistic (NOT a money store), zeroed at the very start of every tick
  /// (`EconomySet::ResetReceipts`) and NEVER persisted. The `(actor, market)` key
  /// carries the market dimension for commuter attribution. Captured at the settle
  /// points where seller id + market + amount are all in scope (auction + macro flow),
  /// so it is coherent with the money move: a fault that discards the settle clone
  /// discards its receipts too.
  #[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
  pub struct SellerReceipts(pub BTreeMap<(EconomicActorId, MarketId), Money>);
  ```

- [ ] **Step 1.3 — Export from `mod.rs` and insert the resource in the plugin.**

  In `backend/crates/sim-core/src/economy/mod.rs`, add to the `pub mod` list (after `pub mod transport;`):

  ```rust
  pub mod wages;
  ```

  Add to the re-export block (after `pub use transport::*;`):

  ```rust
  pub use wages::*;
  ```

  In `EconomyPlugin::install`, after `world.insert_resource(crate::economy::shoppers::NextShopperId::default());`, add:

  ```rust
  world.insert_resource(crate::economy::wages::SellerReceipts::default());
  ```

- [ ] **Step 1.4 — Add the receipts-capturing auction overload + delegate.**

  In `backend/crates/sim-core/src/economy/auction.rs`, replace the existing `clear_market_good_with_policy` definition (lines 332–445) with a thin delegating wrapper plus the new `clear_market_good_with_receipts`:

  ```rust
  #[allow(clippy::too_many_arguments)]
  pub fn clear_market_good_with_policy(
      accounts: &mut AccountBook,
      inventory: &mut InventoryBook,
      orders: &mut OrderBook,
      ledger: &mut TradeLedger,
      market_goods: &mut MarketGoods,
      key: MarketGoodKey,
      current_tick: u64,
      policy: SettlementPolicy,
  ) -> Result<(), EconomyError> {
      let mut discard = std::collections::BTreeMap::new();
      clear_market_good_with_receipts(
          accounts, inventory, orders, ledger, market_goods, key, current_tick, policy, &mut discard,
      )
  }

  /// As [`clear_market_good_with_policy`], but accumulates each fill's gross sale
  /// value into `receipts[(ask.owner, key.market)]`. The accumulation lives on a
  /// SCRATCH map mutated alongside the `next_accounts` clone and committed in the
  /// SAME `*accounts = next_accounts` block — so a fault that drops the clone drops
  /// the receipts too (coherent with the money). Reset is the caller's job
  /// (`EconomySet::ResetReceipts`); this only ADDS.
  #[allow(clippy::too_many_arguments)]
  pub fn clear_market_good_with_receipts(
      accounts: &mut AccountBook,
      inventory: &mut InventoryBook,
      orders: &mut OrderBook,
      ledger: &mut TradeLedger,
      market_goods: &mut MarketGoods,
      key: MarketGoodKey,
      current_tick: u64,
      policy: SettlementPolicy,
      receipts: &mut std::collections::BTreeMap<
          (crate::economy::EconomicActorId, crate::economy::MarketId),
          Money,
      >,
  ) -> Result<(), EconomyError> {
      let last_settlement_price = market_goods
          .0
          .entry(key)
          .or_insert_with(|| MarketGoodState::new(key))
          .last_settlement_price;
      let bids: Vec<_> = orders
          .bids
          .values()
          .filter(|bid| bid.market == key.market && bid.good == key.good)
          .cloned()
          .collect();
      let asks: Vec<_> = orders
          .asks
          .values()
          .filter(|ask| ask.market == key.market && ask.good == key.good)
          .cloned()
          .collect();
      let plan = build_clearing_plan_with_policy(key, &bids, &asks, last_settlement_price, policy)?;
      let Some(price) = plan.settlement_price else {
          if let Some(state) = market_goods.0.get_mut(&key) {
              state.traded_qty_last_tick = Quantity::ZERO;
              state.unmet_demand_last_tick = plan.unmet_demand;
              state.unsold_supply_last_tick = plan.unsold_supply;
              state.last_cleared_tick = current_tick;
              state.dirty = false;
          }
          return Ok(());
      };

      let mut next_accounts = accounts.clone();
      let mut next_inventory = inventory.clone();
      let mut next_orders = orders.clone();
      let mut trade_events = Vec::new();
      let mut traded_qty = Quantity::ZERO;
      // Scratch receipts: committed in the SAME block as next_accounts so a fault
      // (which returns ? without committing) discards them with the money clone.
      let mut scratch_receipts: std::collections::BTreeMap<
          (crate::economy::EconomicActorId, crate::economy::MarketId),
          Money,
      > = std::collections::BTreeMap::new();

      for fill in &plan.fills {
          let bid = next_orders
              .bids
              .get_mut(&fill.bid)
              .ok_or(EconomyError::InvalidOrder)?
              .clone();
          let ask = next_orders
              .asks
              .get_mut(&fill.ask)
              .ok_or(EconomyError::InvalidOrder)?
              .clone();
          let locked_for_q = checked_order_value(bid.max_price, fill.qty)?;
          let actual_cost = checked_order_value(price, fill.qty)?;
          let refund = locked_for_q.checked_sub(actual_cost)?;

          next_accounts.debit_locked(bid.owner, locked_for_q)?;
          if refund.0 > 0 {
              next_accounts.deposit(bid.owner, refund)?;
          }
          next_accounts.deposit(ask.owner, actual_cost)?;
          // SellerReceipts: gross sale value to the seller at this market.
          let slot = scratch_receipts.entry((ask.owner, key.market)).or_insert(Money::ZERO);
          *slot = slot.checked_add(actual_cost)?;
          next_inventory.debit_locked_goods(ask.owner, ask.good, fill.qty)?;
          next_inventory.deposit(bid.owner, bid.good, fill.qty)?;

          let bid_mut = next_orders.bids.get_mut(&fill.bid).unwrap();
          bid_mut.qty_remaining = bid_mut.qty_remaining.checked_sub(fill.qty)?;
          bid_mut.cash_locked_remaining = bid_mut.cash_locked_remaining.checked_sub(locked_for_q)?;
          let ask_mut = next_orders.asks.get_mut(&fill.ask).unwrap();
          ask_mut.qty_remaining = ask_mut.qty_remaining.checked_sub(fill.qty)?;
          ask_mut.goods_locked_remaining = ask_mut.goods_locked_remaining.checked_sub(fill.qty)?;

          trade_events.push(EconomyEvent::Trade {
              market: key.market,
              good: key.good,
              buyer: bid.owner,
              seller: ask.owner,
              qty: fill.qty,
              price,
              total: actual_cost,
          });
          if refund.0 > 0 {
              trade_events.push(EconomyEvent::CashReleased { actor: bid.owner, amount: refund });
          }
          traded_qty = traded_qty.checked_add(fill.qty)?;
      }

      next_orders.bids.retain(|_, bid| bid.qty_remaining.0 > 0);
      next_orders.asks.retain(|_, ask| ask.qty_remaining.0 > 0);

      *accounts = next_accounts;
      *inventory = next_inventory;
      *orders = next_orders;
      for (k, v) in scratch_receipts {
          let slot = receipts.entry(k).or_insert(Money::ZERO);
          *slot = slot.checked_add(v)?;
      }
      ledger.0.extend(trade_events);
      if let Some(state) = market_goods.0.get_mut(&key) {
          state.last_settlement_price = price;
          state.traded_qty_last_tick = traded_qty;
          state.unmet_demand_last_tick = plan.unmet_demand;
          state.unsold_supply_last_tick = plan.unsold_supply;
          state.last_cleared_tick = current_tick;
          state.dirty = false;
      }
      Ok(())
  }
  ```

- [ ] **Step 1.5 — Register the test module and run.**

  In `backend/crates/sim-core/src/economy/tests/mod.rs`, add `mod wages;` (alphabetical: insert after `mod transport;` — it is the last entry; add a new line `mod wages;`).

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core wages::auction` → **PASS** (both auction tests green).

- [ ] **Step 1.6 — Add `ResetReceipts` set + reset system, and thread receipts through `clear_dirty_markets_system`.**

  In `backend/crates/sim-core/src/economy/systems.rs`:

  Extend the top `use crate::economy::{...}` block by adding `SellerReceipts` and `clear_market_good_with_receipts` (do NOT add a second `use`; merge into the existing one).

  Add the variant at the head of `EconomySet`:

  ```rust
  #[derive(SystemSet, Hash, Eq, PartialEq, Debug, Clone)]
  pub enum EconomySet {
      ResetReceipts,
      RefreshLod,
      ExpireOrders,
      Production,
      GeneratePoolOrders,
      ClearMarkets,
      MacroFlow,
      Consume,
      ShopperCapture,
      Materialize,
      Telemetry,
  }
  ```

  In `install_systems`, prepend `ResetReceipts` to the `.chain()` tuple so it leads:

  ```rust
  schedule.configure_sets(
      (
          EconomySet::ResetReceipts,
          EconomySet::RefreshLod,
          EconomySet::ExpireOrders,
          EconomySet::Production,
          EconomySet::GeneratePoolOrders,
          EconomySet::ClearMarkets,
          EconomySet::MacroFlow,
          EconomySet::Consume,
          EconomySet::ShopperCapture,
          EconomySet::Materialize,
          EconomySet::Telemetry,
      )
          .chain(),
  );
  ```

  Add `reset_seller_receipts_system` to the parallel `add_systems(...)` tuple (the one ending `.before(crate::mobility::systems::tick_increment_system)`):

  ```rust
  reset_seller_receipts_system.in_set(EconomySet::ResetReceipts),
  ```

  Define the system (near `expire_orders_system`):

  ```rust
  /// Tick-start: clear `SellerReceipts` so the settle points accumulate exactly one
  /// tick of revenue (mirrors `run_consumption_at_tick`'s reset-all-then-accumulate).
  pub fn reset_seller_receipts_system(mut receipts: ResMut<SellerReceipts>) {
      receipts.0.clear();
  }
  ```

  Replace `clear_dirty_markets_system`'s signature/body to take `SellerReceipts` and call the receipts overload:

  ```rust
  #[allow(clippy::too_many_arguments)]
  pub fn clear_dirty_markets_system(
      tick: Res<Tick>,
      config: Res<EconomyConfig>,
      mut accounts: ResMut<AccountBook>,
      mut inventory: ResMut<InventoryBook>,
      mut orders: ResMut<OrderBook>,
      mut ledger: ResMut<TradeLedger>,
      mut goods: ResMut<MarketGoods>,
      mut dirty: ResMut<DirtyMarketGoods>,
      mut receipts: ResMut<SellerReceipts>,
  ) {
      let keys: Vec<_> = dirty.0.iter().copied().collect();
      dirty.0.clear();
      for key in keys {
          if let Err(reason) = clear_market_good_with_receipts(
              &mut accounts,
              &mut inventory,
              &mut orders,
              &mut ledger,
              &mut goods,
              key,
              tick.0,
              config.settlement_policy,
              &mut receipts.0,
          ) {
              ledger.0.push(EconomyEvent::MarketClearFailed {
                  market: key.market,
                  good: key.good,
                  reason,
              });
          }
      }
  }
  ```

- [ ] **Step 1.7 — Write the failing macro-flow capture test.**

  Extend the top import block of `tests/wages.rs` (via `Edit` on the existing `use crate::economy::{...}` list) to add `EconomyConfig` and `PlannedFlow` and `settle_flow_with_receipts`. Then append the test (note: positional args `10, 10, 10, 10` are `eff_demand_src, eff_supply_src, eff_demand_dst, eff_supply_dst`, then `&config, 1, false, false` then `&mut receipts.0` — matches `settle_flow_with_receipts`'s arg order):

  ```rust
  #[test]
  fn settle_flow_captures_seller_revenue_into_receipts() {
      // Single seller at src, single buyer at dst, q=10 @ p_src=1_000 → src_revenue=10.
      let seller = EconomicActorId(2);
      let buyer = EconomicActorId(1);
      let src = MarketId(10);
      let dst = MarketId(11);
      let mut accounts = AccountBook::default();
      let mut inventory = InventoryBook::default();
      let mut goods = MarketGoods::default();
      accounts.deposit(buyer, Money(1_000_000)).unwrap();
      inventory.deposit(seller, GOOD_FOOD, Quantity(1_000)).unwrap();
      let flow = PlannedFlow {
          good: GOOD_FOOD,
          src,
          dst,
          q: 10,
          p_src: Money(1_000),
          p_dst: Money(1_200),
          dist: 0,
      };
      let config = EconomyConfig::default();
      let before = accounts.total_money().unwrap();
      let mut receipts = SellerReceipts::default();
      crate::economy::settle_flow_with_receipts(
          &mut accounts, &mut inventory, &mut goods, &flow,
          &[(seller, 10)], &[(buyer, 10)],
          10, 10, 10, 10, &config, 1, false, false, &mut receipts.0,
      )
      .unwrap();
      assert_eq!(accounts.total_money().unwrap(), before, "money conserved");
      // src_revenue = value(1_000, 10) = 1_000*10/1_000 = 10.
      assert_eq!(receipts.0.get(&(seller, src)).copied(), Some(Money(10)));
  }
  ```

  (Confirm `PlannedFlow`'s field set against macro_flow.rs before writing; the literal above matches `{ good, src, dst, q, p_src, p_dst, dist }`. No dead bindings — `MarketGoodKey` is NOT imported into this test since it is unused by this test alone but IS used by `seeded_state`; it is already in the top block.)

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core wages::settle_flow` → **FAIL (does not compile)**.

- [ ] **Step 1.8 — Add the receipts-capturing macro-flow overload + delegate.**

  In `backend/crates/sim-core/src/economy/macro_flow.rs`, replace the `settle_flow` definition (lines 610–713) with a thin delegating wrapper plus the new `settle_flow_with_receipts`. The seller-credit loop accumulates `receipts[(actor, flow.src)] += receipt` exactly at the existing `accounts.deposit(*actor, receipt)?` site. Confirm `MarketId` and `BTreeMap` are already imported at the top of `macro_flow.rs` (both are; no import change needed):

  ```rust
  #[allow(clippy::too_many_arguments)]
  pub fn settle_flow(
      accounts: &mut AccountBook,
      inventory: &mut InventoryBook,
      market_goods: &mut MarketGoods,
      flow: &PlannedFlow,
      sellers: &[(EconomicActorId, i64)],
      buyers: &[(EconomicActorId, i64)],
      eff_demand_src: i64,
      eff_supply_src: i64,
      eff_demand_dst: i64,
      eff_supply_dst: i64,
      config: &EconomyConfig,
      current_tick: u64,
      preserve_price_src: bool,
      preserve_price_dst: bool,
  ) -> Result<EconomyEvent, EconomyError> {
      let mut discard = BTreeMap::new();
      settle_flow_with_receipts(
          accounts, inventory, market_goods, flow, sellers, buyers, eff_demand_src,
          eff_supply_src, eff_demand_dst, eff_supply_dst, config, current_tick,
          preserve_price_src, preserve_price_dst, &mut discard,
      )
  }

  /// As [`settle_flow`], but accumulates each seller's `receipt` into
  /// `receipts[(seller, flow.src)]`. Mutates the SAME `accounts` ref `settle_flow`
  /// does, so a caller that runs this against per-edge SCRATCH clones and folds only
  /// on the Ok branch (see `run_macro_flow_at_tick`) discards the receipts on a
  /// fault. The caller folds the receipts map in the SAME Ok branch.
  #[allow(clippy::too_many_arguments)]
  pub fn settle_flow_with_receipts(
      accounts: &mut AccountBook,
      inventory: &mut InventoryBook,
      market_goods: &mut MarketGoods,
      flow: &PlannedFlow,
      sellers: &[(EconomicActorId, i64)],
      buyers: &[(EconomicActorId, i64)],
      eff_demand_src: i64,
      eff_supply_src: i64,
      eff_demand_dst: i64,
      eff_supply_dst: i64,
      config: &EconomyConfig,
      current_tick: u64,
      preserve_price_src: bool,
      preserve_price_dst: bool,
      receipts: &mut BTreeMap<(EconomicActorId, MarketId), Money>,
  ) -> Result<EconomyEvent, EconomyError> {
      let q = flow.q;
      let src_revenue = checked_order_value(flow.p_src, Quantity(q))?;
      let transport_total =
          transport_cost(flow.dist, Quantity(q), config.transport_cost_per_tile_unit)?;
      let dst_payment = src_revenue.checked_add(transport_total)?;

      let seller_w: Vec<i64> = sellers.iter().map(|(_, w)| *w).collect();
      let seller_goods = prorata_distribute(&seller_w, q);
      let seller_cash = apportion_cash(&seller_goods, src_revenue.0);
      for (idx, (actor, _)) in sellers.iter().enumerate() {
          let goods = seller_goods[idx];
          if goods > 0 {
              inventory.consume(*actor, flow.good, Quantity(goods))?;
          }
          let receipt = Money(seller_cash[idx]);
          if receipt.0 > 0 {
              accounts.deposit(*actor, receipt)?;
              let slot = receipts.entry((*actor, flow.src)).or_insert(Money::ZERO);
              *slot = slot.checked_add(receipt)?;
          }
      }

      let buyer_w: Vec<i64> = buyers.iter().map(|(_, w)| *w).collect();
      let buyer_goods = prorata_distribute(&buyer_w, q);
      let buyer_charge = apportion_cash(&buyer_goods, dst_payment.0);
      for (idx, (actor, _)) in buyers.iter().enumerate() {
          let goods = buyer_goods[idx];
          let charge = Money(buyer_charge[idx]);
          if charge.0 > 0 {
              accounts.lock_cash(*actor, charge)?;
              accounts.debit_locked(*actor, charge)?;
          }
          if goods > 0 {
              inventory.deposit(*actor, flow.good, Quantity(goods))?;
          }
      }

      if transport_total.0 > 0 {
          accounts.deposit(TRANSPORT_OPERATOR, transport_total)?;
      }

      write_back(
          market_goods,
          MarketGoodKey { market: flow.src, good: flow.good },
          flow.p_src, q, (eff_demand_src - q).max(0), (eff_supply_src - q).max(0),
          current_tick, preserve_price_src,
      )?;
      if flow.dst != flow.src {
          write_back(
              market_goods,
              MarketGoodKey { market: flow.dst, good: flow.good },
              flow.p_dst, q, (eff_demand_dst - q).max(0), (eff_supply_dst - q).max(0),
              current_tick, preserve_price_dst,
          )?;
      }

      Ok(EconomyEvent::MacroFlow {
          from_market: flow.src,
          to_market: flow.dst,
          good: flow.good,
          qty: Quantity(q),
          price: flow.p_dst,
          transport: transport_total,
      })
  }
  ```

  Run only to confirm the macro-flow test now compiles+passes:
  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core wages::settle_flow` → **PASS**.

- [ ] **Step 1.9a — Thread a receipts parameter through `run_macro_flow_at_tick`'s signature + pre-loop clone.**

  In `macro_flow.rs`, `run_macro_flow_at_tick` (line 814): add a trailing parameter after `next_order_id`:

  ```rust
      next_order_id: &mut crate::economy::NextOrderId,
      receipts: &mut BTreeMap<(EconomicActorId, MarketId), Money>,
  ) -> Result<(), EconomyError> {
  ```

  Immediately after the existing pre-loop clones (`let next_oid = *next_order_id;`, line 870), add the receipts fold scratch:

  ```rust
  let mut next_receipts = receipts.clone();
  ```

  Build to confirm the wrapper still compiles (the caller in systems.rs will be updated in 1.9c; expect a transient arity error there only):
  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --no-run` → expect ONE error (the systems.rs call site arity) — that is fixed in 1.9c.

- [ ] **Step 1.9b — Switch the per-edge settle call to the receipts overload + fold on Ok.**

  In the per-edge loop, replace the `match settle_flow( ... )` (lines 968-983) with a per-edge scratch-receipts clone fed to `settle_flow_with_receipts`, and add `next_receipts = scratch_receipts;` to the existing Ok branch (alongside `next_accounts = scratch_accounts;` etc., line 985-988):

  ```rust
          let mut scratch_receipts = next_receipts.clone();
          match settle_flow_with_receipts(
              &mut scratch_accounts,
              &mut scratch_inventory,
              &mut scratch_goods,
              &eflow,
              &sellers,
              &buyers,
              eff_demand_src,
              eff_supply_src,
              eff_demand_dst,
              eff_supply_dst,
              config,
              current_tick,
              src_active,
              dst_active,
              &mut scratch_receipts,
          ) {
              Ok(event) => {
                  next_accounts = scratch_accounts;
                  next_inventory = scratch_inventory;
                  next_goods = scratch_goods;
                  next_orders = scratch_orders;
                  next_receipts = scratch_receipts;
                  // ... existing shipment-insert + events.push(event) UNCHANGED ...
  ```

  In the commit block (lines 1018-1023), add the receipts commit before `ledger.0.extend(events);`:

  ```rust
  *next_order_id = next_oid;
  *receipts = next_receipts;
  ledger.0.extend(events);
  ```

- [ ] **Step 1.9c — Update `run_macro_flow_system` to pass receipts.**

  In `systems.rs`, `run_macro_flow_system`: add `mut receipts: ResMut<SellerReceipts>` to the params and pass `&mut receipts.0` as the new trailing arg to `run_macro_flow_at_tick`.

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --no-run` → **PASS (compiles)**.

- [ ] **Step 1.10 — Run the full wages file + the existing suites.**

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core wages::` → **PASS** (all 3).
  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests` → **PASS** (no regressions in conservation/systems/lod/macro_flow/persist; the signature changes are covered by delegating wrappers).

- [ ] **Step 1.11 — Commit.**

  ```
  git add -A
  git commit -m "feat(economy): SellerReceipts capture at both settle points + ResetReceipts

  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
  ```

---

## Task 2 — `income_last_tick` field + labor-share config + `run_pay_wages_at_tick` pure core + `HouseholdSector` + `WageTelemetry` + conservation tests

Build the wage transfer engine as a pure function over the books, with the full conservation contract under test. `income_last_tick` is read/written here, so it is added to `DemandPool` in this task.

### Files
- **Modify** `backend/crates/sim-core/src/economy/pools.rs` — add `income_last_tick` field to `DemandPool`.
- **Modify** every `DemandPool { ... }` literal/helper in the crate (one explicit edit per file, see Steps 2.1a–2.1g).
- **Modify** `backend/crates/sim-core/src/economy/ledger.rs` — `EconomyEvent::WagePaid` + `event_type` arm.
- **Modify** `backend/crates/sim-core/src/economy/systems.rs` — `labor_share_bps` config + validator.
- **Modify** `backend/crates/sim-core/src/economy/wages.rs` — `WageTelemetry`, `HouseholdSector`, `wage_for_revenue`, `run_pay_wages_at_tick`.
- **Modify** `backend/crates/sim-core/src/economy/tests/wages.rs` — conservation + pathological + property tests.

### Test
`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core wages::pay_wages`

### Steps

- [ ] **Step 2.1 — Add `income_last_tick` to `DemandPool`.**

  In `backend/crates/sim-core/src/economy/pools.rs`, extend `DemandPool` (after `last_consumed_tick`):

  ```rust
  pub last_consumed_tick: Option<u64>,
      /// Wage Money this household pool received in the PREVIOUS tick (a period FLOW,
      /// not a balance). Zeroed every tick before the wage split accumulates. Drives the
      /// consumption function (Part B). `Money: Copy` keeps `DemandPool` `Copy`; persists
      /// for free in `demand_pools`. Conservation contract: credited ONLY from the `to`
      /// side of a COMPLETED `transfer(HOUSEHOLD_SECTOR, consumer, share)` — never minted.
      pub income_last_tick: Money,
  }
  ```

  Confirm it compiles-as-error (every literal now lacks the field): `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --no-run 2>&1 | grep -c "missing field"` → expect a positive count. The next seven steps fix each literal explicitly (no open-ended sweep).

- [ ] **Step 2.1a — Fix `seed.rs` (3 literals).** Add `income_last_tick: Money::ZERO,` after the `last_consumed_tick: None,` line in each of the three `DemandPool { ... }` literals (consumer 8_002, food_consumer 8_012, flow_consumer 8_022). `Money` is already imported in `seed.rs`.

- [ ] **Step 2.1b — Fix `tests/systems.rs` (`seed_trading_pair` helper, 1 literal).** Add `income_last_tick: Money::ZERO,` after `last_consumed_tick: ...,`. `Money` is already imported there.

- [ ] **Step 2.1c — Fix `tests/lod.rs` (4 literals).** Add `income_last_tick: Money::ZERO,` after each `last_consumed_tick`. Confirm `Money` is imported (it is used throughout lod.rs).

- [ ] **Step 2.1d — Fix `tests/persist.rs` (1 literal in `seed`).** Add `income_last_tick: crate::economy::Money::ZERO,` after `last_consumed_tick: None,`.

- [ ] **Step 2.1e — Fix `tests/pools.rs` (3 inline literals + the `consume_pool` helper).** Add `income_last_tick: Money::ZERO,` after each `last_consumed_tick: None,`. `Money` is already imported.

- [ ] **Step 2.1f — Fix `tests/macro_flow.rs` (7 literals).** Add `income_last_tick: Money::ZERO,` after each `last_consumed_tick`. Confirm `Money` import.

- [ ] **Step 2.1g — Fix `tests/flow_shipments.rs` (2 literals).** Add `income_last_tick: Money::ZERO,` after each `last_consumed_tick`. Confirm `Money` import (add `Money` to its import block if absent).

  After 2.1a–2.1g: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --no-run` → **PASS (compiles)**.

- [ ] **Step 2.2 — Add `EconomyEvent::WagePaid` + `event_type` arm.**

  In `backend/crates/sim-core/src/economy/ledger.rs`, add to `EconomyEvent` (after `MacroFlow`):

  ```rust
  /// One firm's wage payment to the household sector this tick (labor share of
  /// revenue). Emitted in ascending firm-id order for determinism.
  WagePaid {
      firm: EconomicActorId,
      market: MarketId,
      amount: Money,
  },
  ```

  Add the match arm in `event_type`:

  ```rust
  Self::WagePaid { .. } => "wage_paid",
  ```

- [ ] **Step 2.3 — Add `labor_share_bps` to `EconomyConfig` with validation.**

  In `backend/crates/sim-core/src/economy/systems.rs`, add the field to `EconomyConfig` (the derive set `Resource, Debug, Clone, Copy, PartialEq` is unchanged; `u16` preserves `Copy`):

  ```rust
  /// Labor share of value added (basis points, 0..=10_000). Default 6_000 = 0.60
  /// (Kaldor stylized fact). VALIDATED `0..=10_000` so `wage <= revenue` ⇒ no overdraft.
  pub labor_share_bps: u16,
  ```

  In `Default::default`, add `labor_share_bps: 6_000,`.

  Add a validator near the config:

  ```rust
  impl EconomyConfig {
      /// `labor_share_bps` as an i128, refusing `> 10_000` (a config bug that would
      /// over-pay). Exposed for the pure `run_pay_wages_at_tick` core. Boundary
      /// `== 10_000` is allowed (full labor share).
      pub fn validated_labor_share_bps(&self) -> Result<i128, crate::economy::EconomyError> {
          if self.labor_share_bps > 10_000 {
              return Err(crate::economy::EconomyError::InvalidOrder);
          }
          Ok(self.labor_share_bps as i128)
      }
  }
  ```

- [ ] **Step 2.4 — Add `WageTelemetry`, `HouseholdSector`, and `wage_for_revenue` to `wages.rs`.**

  In `backend/crates/sim-core/src/economy/wages.rs`, add a SECOND import line (the file currently imports only `{EconomicActorId, MarketId, Money}`; merge the new names into that ONE existing `use crate::economy::{...}` so there is no duplicate `use`):

  ```rust
  use crate::economy::{
      AccountBook, DemandPools, EconomicActorId, EconomyConfig, EconomyError, EconomyEvent, GoodId,
      MarketId, Money, TradeLedger, apportion_cash,
  };
  ```

  Then add:

  ```rust
  /// Wage Money paid per MARKET this tick (the commuter-projection driver). Ephemeral,
  /// NOT persisted, reset-all-then-accumulate by `run_pay_wages_at_tick`. NOT on
  /// `MarketGoodState` (avoids the constructor fan-out + an extra DELETE).
  #[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
  pub struct WageTelemetry(pub BTreeMap<MarketId, Money>);

  /// The mean-field household sector. PERSISTED. `population` parametrizes the sector
  /// budget arithmetically (never materializes per-person accounts — the loop is
  /// O(firms + pools)). `pool_weights` is the largest-remainder split of the wage bill
  /// across consumer pools (equal weights v0). At least one weight MUST be positive
  /// (seed assert), else the wage bill would strand in HOUSEHOLD_SECTOR.
  #[derive(Resource, Debug, Clone, PartialEq, Eq)]
  pub struct HouseholdSector {
      pub population: u64,
      pub pool_weights: BTreeMap<EconomicActorId, i64>,
  }

  /// `wage = floor(revenue * labor_share_bps / 10_000)`. `labor_share_bps <= 10_000`
  /// (validated by the caller) ⇒ `wage <= revenue` ⇒ no overdraft. Floor leaves the
  /// rounding remainder at the firm (never minted). i128 intermediate, `try_from` → Overflow.
  pub(crate) fn wage_for_revenue(revenue: Money, labor_share_bps: i128) -> Result<Money, EconomyError> {
      let raw = (revenue.0 as i128) * labor_share_bps / 10_000;
      Ok(Money(i64::try_from(raw).map_err(|_| EconomyError::Overflow)?))
  }
  ```

  (`std::collections::BTreeMap` is already imported at the top of `wages.rs` from Task 1.)

- [ ] **Step 2.5 — Write the failing conservation tests for `run_pay_wages_at_tick`.**

  Extend the top import block of `tests/wages.rs` (via `Edit` on the SINGLE existing `use crate::economy::{...}`) to add: `DemandPool, GOOD_TOOLS, HouseholdSector, HOUSEHOLD_SECTOR, WageTelemetry, run_pay_wages_at_tick`. Do NOT add a second `use std::collections::BTreeMap;` — it is already at the top of the file from Step 1.1. Then append the helper + suite:

  ```rust
  fn consumer_pool(actor: EconomicActorId, market: MarketId) -> DemandPool {
      DemandPool {
          actor,
          market,
          good: GOOD_TOOLS,
          desired_qty_per_tick: Quantity(10),
          max_price: Money(2_000),
          urgency_bps: 0,
          elasticity_bps: 0,
          interval_ticks: 1,
          last_generated_tick: None,
          last_consumed_tick: None,
          income_last_tick: Money::ZERO,
          mpc_bps: 8_000,
          autonomous: Money(5_000),
      }
  }

  fn fixture(
      firm_revenues: &[(EconomicActorId, MarketId, Money)],
      consumers: &[EconomicActorId],
  ) -> (AccountBook, SellerReceipts, DemandPools, HouseholdSector, EconomyConfig) {
      let mut accounts = AccountBook::default();
      let mut receipts = SellerReceipts::default();
      for (firm, market, rev) in firm_revenues {
          accounts.deposit(*firm, *rev).unwrap();
          let slot = receipts.0.entry((*firm, *market)).or_insert(Money::ZERO);
          *slot = slot.checked_add(*rev).unwrap();
      }
      let mut demand = DemandPools::default();
      let mut weights = BTreeMap::new();
      for c in consumers {
          demand.0.insert(*c, consumer_pool(*c, MarketId(9_002)));
          weights.insert(*c, 1);
      }
      let household = HouseholdSector { population: 1_000_000, pool_weights: weights };
      (accounts, receipts, demand, household, EconomyConfig::default())
  }

  #[test]
  fn pay_wages_conserves_money_and_nets_sentinel_to_zero() {
      let f1 = EconomicActorId(8_001);
      let c1 = EconomicActorId(8_002);
      let c2 = EconomicActorId(8_012);
      let (mut accounts, receipts, mut demand, household, config) =
          fixture(&[(f1, MarketId(9_001), Money(1_000))], &[c1, c2]);
      let before = accounts.total_money().unwrap();
      let mut wage_tel = WageTelemetry::default();
      let mut ledger = TradeLedger::default();

      run_pay_wages_at_tick(&mut accounts, &receipts, &mut demand, &household, &mut wage_tel, &mut ledger, &config).unwrap();

      assert_eq!(accounts.total_money().unwrap(), before, "byte-invariant total money");
      assert_eq!(accounts.account(HOUSEHOLD_SECTOR).available, Money::ZERO, "sentinel nets to zero");
      assert_eq!(accounts.account(HOUSEHOLD_SECTOR).locked, Money::ZERO);
      // wage = 1000 * 6000/10000 = 600. firm keeps 400. consumers split 600 (300/300).
      assert_eq!(accounts.account(f1).available, Money(400));
      let inc: i64 = demand.0.values().map(|p| p.income_last_tick.0).sum();
      assert_eq!(inc, 600, "Σ income == wage bill (== Σ firm→household transfers)");
      assert_eq!(demand.0[&c1].income_last_tick, Money(300));
      assert_eq!(demand.0[&c2].income_last_tick, Money(300));
      assert_eq!(wage_tel.0.get(&MarketId(9_001)).copied(), Some(Money(600)));
  }

  #[test]
  fn pay_wages_no_overdraft_and_income_equals_transfers() {
      let f1 = EconomicActorId(8_001);
      let f2 = EconomicActorId(8_011);
      let c1 = EconomicActorId(8_002);
      let (mut accounts, receipts, mut demand, household, config) = fixture(
          &[(f1, MarketId(9_001), Money(1_000)), (f2, MarketId(9_003), Money(500))],
          &[c1],
      );
      let mut wage_tel = WageTelemetry::default();
      let mut ledger = TradeLedger::default();
      run_pay_wages_at_tick(&mut accounts, &receipts, &mut demand, &household, &mut wage_tel, &mut ledger, &config).unwrap();
      assert!(accounts.account(f1).available.0 >= 0);
      assert!(accounts.account(f2).available.0 >= 0);
      // wage1=600, wage2=300 → Σ=900 all to the single consumer.
      assert_eq!(demand.0[&c1].income_last_tick, Money(900));
      let firms: Vec<EconomicActorId> = ledger.0.iter().filter_map(|e| match e {
          EconomyEvent::WagePaid { firm, .. } => Some(*firm),
          _ => None,
      }).collect();
      assert_eq!(firms, vec![f1, f2], "WagePaid emitted in ascending firm id");
  }

  #[test]
  fn pay_wages_zero_receipts_is_noop() {
      let c1 = EconomicActorId(8_002);
      let (mut accounts, _r, mut demand, household, config) = fixture(&[], &[c1]);
      let receipts = SellerReceipts::default();
      let before = accounts.total_money().unwrap();
      let mut wage_tel = WageTelemetry::default();
      let mut ledger = TradeLedger::default();
      run_pay_wages_at_tick(&mut accounts, &receipts, &mut demand, &household, &mut wage_tel, &mut ledger, &config).unwrap();
      assert_eq!(accounts.total_money().unwrap(), before);
      assert_eq!(demand.0[&c1].income_last_tick, Money::ZERO);
      assert!(wage_tel.0.is_empty());
  }

  #[test]
  fn pay_wages_wage_bill_smaller_than_pools_floors_some_to_zero() {
      // wage_bill = floor(2 * 0.6) = 1, split across 3 equal pools → 1/0/0 (largest-remainder).
      let f1 = EconomicActorId(8_001);
      let (c1, c2, c3) = (EconomicActorId(8_002), EconomicActorId(8_012), EconomicActorId(8_022));
      let (mut accounts, receipts, mut demand, household, config) =
          fixture(&[(f1, MarketId(9_001), Money(2))], &[c1, c2, c3]);
      let before = accounts.total_money().unwrap();
      let mut wage_tel = WageTelemetry::default();
      let mut ledger = TradeLedger::default();
      run_pay_wages_at_tick(&mut accounts, &receipts, &mut demand, &household, &mut wage_tel, &mut ledger, &config).unwrap();
      assert_eq!(accounts.total_money().unwrap(), before);
      assert_eq!(accounts.account(HOUSEHOLD_SECTOR).available, Money::ZERO);
      let total_income: i64 = demand.0.values().map(|p| p.income_last_tick.0).sum();
      assert_eq!(total_income, 1, "Σ income == wage bill even when some pools floor to 0");
      assert_eq!(demand.0[&c1].income_last_tick, Money(1), "lowest index wins the single unit");
      assert_eq!(demand.0[&c2].income_last_tick, Money::ZERO);
  }

  #[test]
  fn pay_wages_full_labor_share_pays_all_revenue() {
      let f1 = EconomicActorId(8_001);
      let c1 = EconomicActorId(8_002);
      let (mut accounts, receipts, mut demand, household, mut config) =
          fixture(&[(f1, MarketId(9_001), Money(1_000))], &[c1]);
      config.labor_share_bps = 10_000;
      let before = accounts.total_money().unwrap();
      let mut wage_tel = WageTelemetry::default();
      let mut ledger = TradeLedger::default();
      run_pay_wages_at_tick(&mut accounts, &receipts, &mut demand, &household, &mut wage_tel, &mut ledger, &config).unwrap();
      assert_eq!(accounts.total_money().unwrap(), before);
      assert_eq!(accounts.account(f1).available, Money::ZERO, "labor_share=1.0 → firm pays all");
      assert_eq!(demand.0[&c1].income_last_tick, Money(1_000));
  }

  #[test]
  fn pay_wages_all_zero_weights_skips_first_leg() {
      // Σ weights == 0 ⇒ wage bill must NOT strand in the sentinel; first leg skipped.
      let f1 = EconomicActorId(8_001);
      let c1 = EconomicActorId(8_002);
      let (mut accounts, receipts, mut demand, mut household, config) =
          fixture(&[(f1, MarketId(9_001), Money(1_000))], &[c1]);
      household.pool_weights.insert(c1, 0);
      let before = accounts.total_money().unwrap();
      let mut wage_tel = WageTelemetry::default();
      let mut ledger = TradeLedger::default();
      run_pay_wages_at_tick(&mut accounts, &receipts, &mut demand, &household, &mut wage_tel, &mut ledger, &config).unwrap();
      assert_eq!(accounts.total_money().unwrap(), before);
      assert_eq!(accounts.account(HOUSEHOLD_SECTOR).available, Money::ZERO, "no strand");
      assert_eq!(accounts.account(f1).available, Money(1_000), "firm keeps all (no payout target)");
      assert_eq!(demand.0[&c1].income_last_tick, Money::ZERO);
  }

  #[test]
  fn pay_wages_population_million_max_revenue_no_overflow() {
      let f1 = EconomicActorId(8_001);
      let c1 = EconomicActorId(8_002);
      let big = Money(i64::MAX / 2);
      let mut accounts = AccountBook::default();
      accounts.deposit(f1, big).unwrap();
      let mut receipts = SellerReceipts::default();
      receipts.0.insert((f1, MarketId(9_001)), big);
      let mut demand = DemandPools::default();
      demand.0.insert(c1, consumer_pool(c1, MarketId(9_002)));
      let mut weights = BTreeMap::new();
      weights.insert(c1, 1);
      let household = HouseholdSector { population: 1_000_000, pool_weights: weights };
      let config = EconomyConfig::default();
      let before = accounts.total_money().unwrap();
      let mut wage_tel = WageTelemetry::default();
      let mut ledger = TradeLedger::default();
      run_pay_wages_at_tick(&mut accounts, &receipts, &mut demand, &household, &mut wage_tel, &mut ledger, &config).unwrap();
      assert_eq!(accounts.total_money().unwrap(), before, "no mint under huge revenue");
      assert_eq!(accounts.account(HOUSEHOLD_SECTOR).available, Money::ZERO);
  }
  ```

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core wages::pay_wages` → **FAIL (does not compile: `run_pay_wages_at_tick`, `mpc_bps`/`autonomous` missing)**. (The `consumer_pool` helper references `mpc_bps`/`autonomous`, added to `DemandPool` in Task 4; until then this step's failure includes those field errors — that is expected and is resolved once Task 4.1 lands. To keep this step self-contained, temporarily omit `mpc_bps`/`autonomous` from `consumer_pool` here and add them in Step 4.1; OR land Task 4.1's field addition together with Step 2.5's helper. **Decision: include `mpc_bps: 8_000, autonomous: Money(5_000)` in `consumer_pool` now and add the two `DemandPool` fields in Step 2.6a so this file compiles.**)

- [ ] **Step 2.6a — Add `mpc_bps` + `autonomous` to `DemandPool` (pulled earlier so `tests/wages.rs` compiles) and re-fix every literal.**

  In `pools.rs`, extend `DemandPool` (after `income_last_tick`):

  ```rust
      pub income_last_tick: Money,
      /// Marginal propensity to consume (basis points, validated `0..=10_000`). Keynesian
      /// `C = autonomous + mpc_bps*income/10_000`. Default 8_000 (0.8). Persisted per-pool.
      pub mpc_bps: i32,
      /// Autonomous (subsistence) consumption spend per tick, financed from wealth. `> 0`
      /// breaks the zero-trap (income=0 ⇒ C=autonomous ⇒ a floor bid keeps the loop alive).
      /// Persisted per-pool.
      pub autonomous: Money,
  }
  ```

  Re-fix every literal/helper edited in 2.1a–2.1g by appending `mpc_bps: 8_000,` and `autonomous: Money(5_000),` after `income_last_tick: Money::ZERO,` (same seven files, same per-file explicit edits). The `consumer_pool` helper in `tests/wages.rs` already carries both (Step 2.5).

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --no-run` → **PASS (compiles)** — except `run_pay_wages_at_tick` is still missing, so `wages::pay_wages` still fails to link until 2.6b. Run `wages::pay_wages` to confirm the only remaining error is the missing fn.

- [ ] **Step 2.6b — Implement `run_pay_wages_at_tick`.**

  In `backend/crates/sim-core/src/economy/wages.rs`, add:

  ```rust
  /// The SFC wage step. Pure over its refs (no `World`). For each `(firm, market)` in
  /// `receipts` (keys-first → ascending), pays `wage = floor(revenue * labor_share / 10_000)`
  /// from the firm into `HOUSEHOLD_SECTOR` via `transfer` (two-leg, conservative); the
  /// wage bill is summed ONLY from transfers that actually succeeded. Then largest-remainder
  /// splits the wage bill across consumer pools (`pool_weights`, ties-by-ascending-index)
  /// and transfers each share `HOUSEHOLD_SECTOR → consumer`, crediting `income_last_tick`
  /// from the COMPLETED `to`-side. Resets `income_last_tick` (keys-first) and `WageTelemetry`
  /// first. Conservation: `total_money` byte-invariant (only transfers); `HOUSEHOLD_SECTOR`
  /// nets to zero (`apportion_cash` is exactly sum-preserving when Σweights>0). When Σweights==0
  /// BOTH legs are skipped so nothing strands.
  #[allow(clippy::too_many_arguments)]
  pub fn run_pay_wages_at_tick(
      accounts: &mut AccountBook,
      receipts: &SellerReceipts,
      demand: &mut DemandPools,
      household: &HouseholdSector,
      wage_telemetry: &mut WageTelemetry,
      ledger: &mut TradeLedger,
      config: &EconomyConfig,
  ) -> Result<(), EconomyError> {
      for pool in demand.0.values_mut() {
          pool.income_last_tick = Money::ZERO;
      }
      wage_telemetry.0.clear();

      let labor_share = config.validated_labor_share_bps()?;

      let pool_ids: Vec<EconomicActorId> = demand.0.keys().copied().collect();
      let weights: Vec<i64> = pool_ids
          .iter()
          .map(|a| household.pool_weights.get(a).copied().unwrap_or(0).max(0))
          .collect();
      let weight_sum: i128 = weights.iter().map(|w| *w as i128).sum();

      // FIRST LEG: firms → HOUSEHOLD_SECTOR. Skipped entirely when there is no payout
      // target (Σweights==0), so the bill never strands in the sentinel.
      let mut wage_bill: i64 = 0;
      if weight_sum > 0 {
          for (&(firm, market), &revenue) in receipts.0.iter() {
              let wage = wage_for_revenue(revenue, labor_share)?;
              if wage.0 <= 0 {
                  continue;
              }
              // Affordability holds by invariant (wage <= revenue just received). A
              // failure is AUDITED (the spec §12 honest InsufficientFunds halt), not
              // swallowed, and its share is NOT added to wage_bill.
              if accounts.account(firm).available < wage {
                  ledger.0.push(EconomyEvent::MarketClearFailed {
                      market,
                      good: GoodId(0),
                      reason: EconomyError::InsufficientFunds,
                  });
                  continue;
              }
              accounts.transfer(firm, HOUSEHOLD_SECTOR, wage)?;
              wage_bill = wage_bill.checked_add(wage.0).ok_or(EconomyError::Overflow)?;
              let slot = wage_telemetry.0.entry(market).or_insert(Money::ZERO);
              *slot = slot.checked_add(wage)?;
              ledger.0.push(EconomyEvent::WagePaid { firm, market, amount: wage });
          }
      }

      // SECOND LEG: HOUSEHOLD_SECTOR → consumer pools (largest-remainder, sum-preserving).
      if wage_bill > 0 && weight_sum > 0 {
          let splits = apportion_cash(&weights, wage_bill);
          for (idx, actor) in pool_ids.iter().enumerate() {
              let share = Money(splits[idx]);
              if share.0 <= 0 {
                  continue;
              }
              accounts.transfer(HOUSEHOLD_SECTOR, *actor, share)?;
              if let Some(pool) = demand.0.get_mut(actor) {
                  pool.income_last_tick = pool.income_last_tick.checked_add(share)?;
              }
          }
      }

      debug_assert_eq!(
          accounts.account(HOUSEHOLD_SECTOR).available,
          Money::ZERO,
          "HOUSEHOLD_SECTOR must net to zero after PayWages (sentinel-stranded cash)"
      );
      Ok(())
  }
  ```

- [ ] **Step 2.7 — Run the conservation suite.**

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core wages::pay_wages` → **PASS** (all 7).

- [ ] **Step 2.8 — Add the audited-halt test (spec §12) + the property test.**

  Append to `tests/wages.rs`. First, the explicit spec-§12 "clean audited InsufficientFunds halt" unit test (the only place that asserts the audited-halt branch at the pure-core level):

  ```rust
  #[test]
  fn pay_wages_firm_short_of_wage_emits_audited_halt_and_skips_its_bill() {
      // A firm whose CASH is below the computed wage (an impossible-by-invariant state
      // forced here) must AUDIT (MarketClearFailed/InsufficientFunds) and contribute
      // nothing to the wage bill — a clean halt, not a panic or a mint.
      let f1 = EconomicActorId(8_001);
      let c1 = EconomicActorId(8_002);
      let market = MarketId(9_001);
      let mut accounts = AccountBook::default();
      accounts.deposit(f1, Money(100)).unwrap(); // cash 100, but receipts claim 1_000
      let mut receipts = SellerReceipts::default();
      receipts.0.insert((f1, market), Money(1_000)); // wage would be 600 > 100
      let mut demand = DemandPools::default();
      demand.0.insert(c1, consumer_pool(c1, MarketId(9_002)));
      let household = HouseholdSector { population: 1, pool_weights: BTreeMap::from([(c1, 1)]) };
      let before = accounts.total_money().unwrap();
      let mut wage_tel = WageTelemetry::default();
      let mut ledger = TradeLedger::default();
      run_pay_wages_at_tick(&mut accounts, &receipts, &mut demand, &household, &mut wage_tel, &mut ledger, &EconomyConfig::default()).unwrap();
      assert_eq!(accounts.total_money().unwrap(), before, "no mint on the halt path");
      assert_eq!(accounts.account(f1).available, Money(100), "firm untouched");
      assert_eq!(demand.0[&c1].income_last_tick, Money::ZERO, "no income from a halted firm");
      assert!(
          ledger.0.iter().any(|e| matches!(e,
              EconomyEvent::MarketClearFailed { reason: EconomyError::InsufficientFunds, .. })),
          "the halt is audited"
      );
      assert!(wage_tel.0.is_empty());
  }

  #[test]
  fn pay_wages_property_conserves_over_random_inputs() {
      let mut state: u64 = 0x1234_5678_9abc_def0;
      let mut next = || {
          state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
          (state >> 33) as i64
      };
      for _ in 0..400 {
          let labor = (next().rem_euclid(10_001)) as u16;
          let n_firms = 1 + next().rem_euclid(4);
          let n_pools = 1 + next().rem_euclid(3);
          let mut accounts = AccountBook::default();
          let mut receipts = SellerReceipts::default();
          let mut total_wage_expected: i64 = 0;
          for k in 0..n_firms {
              let firm = EconomicActorId(8_001 + k as u64 * 10);
              let rev = Money(next().rem_euclid(1_000_000));
              if rev.0 > 0 {
                  accounts.deposit(firm, rev).unwrap();
                  let slot = receipts.0.entry((firm, MarketId(9_000 + k as u32))).or_insert(Money::ZERO);
                  *slot = slot.checked_add(rev).unwrap();
                  total_wage_expected += (rev.0 as i128 * labor as i128 / 10_000) as i64;
              }
          }
          let mut demand = DemandPools::default();
          let mut weights = BTreeMap::new();
          for p in 0..n_pools {
              let c = EconomicActorId(8_002 + p as u64 * 10);
              demand.0.insert(c, consumer_pool(c, MarketId(9_002)));
              weights.insert(c, 1);
          }
          let household = HouseholdSector { population: 1_000_000, pool_weights: weights };
          let mut config = EconomyConfig::default();
          config.labor_share_bps = labor;
          let before = accounts.total_money().unwrap();
          let mut wage_tel = WageTelemetry::default();
          let mut ledger = TradeLedger::default();
          run_pay_wages_at_tick(&mut accounts, &receipts, &mut demand, &household, &mut wage_tel, &mut ledger, &config).unwrap();
          assert_eq!(accounts.total_money().unwrap(), before, "money conserved (labor={labor})");
          assert_eq!(accounts.account(HOUSEHOLD_SECTOR).available, Money::ZERO);
          let inc: i64 = demand.0.values().map(|p| p.income_last_tick.0).sum();
          assert_eq!(inc, total_wage_expected, "Σ income == Σ wages (labor={labor})");
          for acc in accounts.accounts.values() {
              assert!(acc.available.0 >= 0 && acc.locked.0 >= 0, "no negative balance");
          }
      }
  }
  ```

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core wages::pay_wages` → **PASS** (now 9 conservation/pathological/property tests).

- [ ] **Step 2.9 — Commit.**

  ```
  git add -A
  git commit -m "feat(economy): run_pay_wages_at_tick pure core + HouseholdSector + WageTelemetry + DemandPool wage/mpc fields + conservation suite

  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
  ```

---

## Task 3 — seed `HouseholdSector`, persist it, wire the `PayWages` system, assert the full-plugin-tick conservation (both settle paths)

Make the wage core run inside the live plugin tick: seed the household sector + equal pool weights, persist it (no serde-default), insert the resources in the plugin, and add the `PayWages` system at the pinned schedule position. Verify with a full-plugin-tick conservation test that exercises BOTH settle paths feeding PayWages.

### Files
- **Modify** `backend/crates/sim-core/src/economy/seed.rs` — build `HouseholdSector` (equal weights), assert ≥1 positive weight.
- **Modify** `backend/crates/sim-core/src/economy/persist.rs` — `household_sector` field + extract/apply (no serde-default).
- **Modify** `backend/crates/sim-core/src/economy/mod.rs` — insert `HouseholdSector` + `WageTelemetry` in `EconomyPlugin::install`.
- **Modify** `backend/crates/sim-core/src/economy/systems.rs` — `EconomySet::PayWages`; `run_pay_wages_system`.
- **Modify** `backend/crates/sim-core/src/economy/tests/wages.rs` — full-plugin-tick conservation tests (auction path + macro-flow path).
- **Modify** `backend/crates/sim-core/src/economy/tests/persist.rs` — household-sector round-trip.

### Test
`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core wages::full_tick`

### Steps

- [ ] **Step 3.1 — Insert `HouseholdSector` + `WageTelemetry` in the plugin.**

  In `backend/crates/sim-core/src/economy/mod.rs`, in `EconomyPlugin::install` (after the `SellerReceipts` insert from Task 1):

  ```rust
  world.insert_resource(crate::economy::wages::WageTelemetry::default());
  world.insert_resource(crate::economy::wages::HouseholdSector {
      population: 0,
      pool_weights: std::collections::BTreeMap::new(),
  });
  ```

  (A fresh world starts with an empty sector; `seed_demo_economy` populates it, and `apply_into_world` overwrites it on hydrate.)

- [ ] **Step 3.2 — Seed `HouseholdSector` with equal weights over the consumer pools.**

  In `backend/crates/sim-core/src/economy/seed.rs`, extend the EXISTING `use crate::economy::{...}` block (currently lines 11–15: `AccountBook, DemandPool, DemandPools, EconomicActorId, GOOD_FOOD, GOOD_TOOLS, InventoryBook, MarketChunks, MarketDistances, MarketId, MarketSite, Markets, Money, Quantity, SupplyPool, SupplyPools`) by adding `HouseholdSector` and `HOUSEHOLD_SECTOR` to it (single merged block — `Money`, `EconomicActorId`, `DemandPools` are already present):

  ```rust
  use crate::economy::{
      AccountBook, DemandPool, DemandPools, EconomicActorId, GOOD_FOOD, GOOD_TOOLS, HOUSEHOLD_SECTOR,
      HouseholdSector, InventoryBook, MarketChunks, MarketDistances, MarketId, MarketSite, Markets,
      Money, Quantity, SupplyPool, SupplyPools,
  };
  ```

  At the END of `seed_demo_economy` (after the flow-demo pools are inserted, just before the closing brace), add:

  ```rust
  // ── SFC household sector: equal-weight payout across the seeded consumer pools ──
  // population is the mean-field 1M; weights are uniform (v0). Assert ≥1 positive
  // weight so the wage bill is never stranded in HOUSEHOLD_SECTOR.
  {
      let consumer_ids: Vec<EconomicActorId> =
          world.resource::<DemandPools>().0.keys().copied().collect();
      let mut weights = std::collections::BTreeMap::new();
      for id in &consumer_ids {
          weights.insert(*id, 1_i64);
      }
      assert!(
          weights.values().any(|w| *w > 0),
          "seed: HouseholdSector must have at least one positive pool weight"
      );
      assert!(
          HOUSEHOLD_SECTOR.0 == u64::MAX - 1 && !consumer_ids.contains(&HOUSEHOLD_SECTOR),
          "HOUSEHOLD_SECTOR must not collide with a seeded actor id"
      );
      world.insert_resource(HouseholdSector {
          population: 1_000_000,
          pool_weights: weights,
      });
  }
  ```

- [ ] **Step 3.3 — Persist `household_sector` (no serde-default).**

  In `backend/crates/sim-core/src/economy/persist.rs`, extend the EXISTING `use crate::economy::{...}` block (lines 10–15) by adding `HouseholdSector` (single merged block — do NOT add a second `use`):

  ```rust
  use crate::economy::{
      AccountBook, Ask, Bid, DemandPool, DemandPools, EconomicActorId, EconomyEvent, GoodId,
      HouseholdSector, InventoryBalance, InventoryBook, MarketChunks, MarketDistances, MarketGoodKey,
      MarketGoodState, MarketGoods, MarketId, MarketSite, Markets, MoneyAccount, NextOrderId,
      OrderBook, OrderId, ProductionPool, ProductionPools, SupplyPool, SupplyPools, TradeLedger,
  };
  ```

  Add the field to `EconomyPersistSnapshot` (after `market_distances`):

  ```rust
      /// The mean-field household sector (population + per-pool wage weights). New
      /// non-default snapshot field; old rows fail to deserialize (one-time
      /// `DELETE FROM economy_snapshots` before deploy). No serde-default shim.
      pub household_sector: HouseholdSectorSnapshot,
  ```

  Add the snapshot struct (the map key `EconomicActorId` is rejected by `serde_json` as a map key, so use `Vec<(K,V)>`):

  ```rust
  /// Wire form of `HouseholdSector` (the `pool_weights` map → sorted `Vec<(K,V)>`).
  #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
  pub struct HouseholdSectorSnapshot {
      pub population: u64,
      pub pool_weights: Vec<(EconomicActorId, i64)>,
  }
  ```

  In `extract_from_world`, after `let market_distances = world.resource::<MarketDistances>();`:

  ```rust
  let household = world.resource::<HouseholdSector>();
  ```

  and in the returned struct literal (after `market_distances: ...`):

  ```rust
      household_sector: HouseholdSectorSnapshot {
          population: household.population,
          pool_weights: household.pool_weights.iter().map(|(k, v)| (*k, *v)).collect(),
      },
  ```

  In `apply_into_world`, after `world.insert_resource(MarketDistances(...));`:

  ```rust
  world.insert_resource(HouseholdSector {
      population: snap.household_sector.population,
      pool_weights: snap.household_sector.pool_weights.iter().cloned().collect(),
  });
  ```

  Note: `apply_into_world` runs against a freshly-installed `EconomyPlugin` world (which inserted the empty `HouseholdSector` in 3.1), so this OVERWRITES it. The ephemeral `SellerReceipts`/`WageTelemetry` (and Task 6's `CommuterTrips`/`NextCommuterId`) are NOT in the snapshot and stay at the plugin defaults — intentional.

- [ ] **Step 3.4 — Add the household round-trip test.**

  In `backend/crates/sim-core/src/economy/tests/persist.rs`, append to the end of the `seed` helper body (where `a` and `b` are already defined):

  ```rust
  world.insert_resource(crate::economy::HouseholdSector {
      population: 1_000_000,
      pool_weights: std::collections::BTreeMap::from([(a, 3_i64), (b, 1_i64)]),
  });
  ```

  Then add a focused test:

  ```rust
  #[test]
  fn household_sector_round_trips() {
      let mut world = install_economy();
      seed(&mut world);
      let snap = extract_from_world(&world);
      let bytes = serde_json::to_vec(&snap).unwrap();
      let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
      let mut fresh = install_economy();
      apply_into_world(&mut fresh, &decoded);
      assert_eq!(
          world.resource::<crate::economy::HouseholdSector>(),
          fresh.resource::<crate::economy::HouseholdSector>(),
          "household sector survives extract->serialize->apply"
      );
      assert_eq!(snap, extract_from_world(&fresh));
  }
  ```

  (`install_economy`, `seed`, `extract_from_world`, `apply_into_world`, `EconomyPersistSnapshot` are all already in scope in this test file.)

- [ ] **Step 3.5 — Add `PayWages` set + system at the pinned position.**

  In `backend/crates/sim-core/src/economy/systems.rs`, extend the top `use crate::economy::{...}` block by adding `HouseholdSector`, `WageTelemetry`, `run_pay_wages_at_tick` (merge into the single block).

  Add `PayWages` to `EconomySet` (between `MacroFlow` and `Consume`):

  ```rust
      MacroFlow,
      PayWages,
      Consume,
  ```

  In the `.chain()` tuple in `install_systems`, insert `EconomySet::PayWages,` between `EconomySet::MacroFlow,` and `EconomySet::Consume,`. Because the chain is `... ClearMarkets, MacroFlow, PayWages, Consume ...`, PayWages is transitively `.after(ClearMarkets).after(MacroFlow).before(Consume)` — exactly the canonical ordering.

  Add `run_pay_wages_system` to the parallel `add_systems(...)` tuple:

  ```rust
  run_pay_wages_system.in_set(EconomySet::PayWages),
  ```

  Define the system (near `run_consumption_system`):

  ```rust
  /// The SFC wage step: firms pay a labor share of this tick's revenue into the
  /// household sector, which is apportioned to consumer pools (income). Runs after
  /// BOTH settle paths (`ClearMarkets`, `MacroFlow`) so all receipts are booked, and
  /// before `Consume` so the goods sink drains after wages land.
  pub fn run_pay_wages_system(
      config: Res<EconomyConfig>,
      receipts: Res<SellerReceipts>,
      household: Res<HouseholdSector>,
      mut accounts: ResMut<AccountBook>,
      mut demand: ResMut<DemandPools>,
      mut wage_telemetry: ResMut<WageTelemetry>,
      mut ledger: ResMut<TradeLedger>,
  ) {
      let _ = run_pay_wages_at_tick(
          &mut accounts,
          &receipts,
          &mut demand,
          &household,
          &mut wage_telemetry,
          &mut ledger,
          &config,
      );
  }
  ```

- [ ] **Step 3.6 — Add the full-tick test harness helper + the auction-path conservation test.**

  Extend the top import block of `tests/wages.rs` to add: `EconomyPlugin, SupplyPool, SupplyPools, GOOD_TOOLS` (GOOD_TOOLS already added in 2.5), plus `use crate::world::plugin::CorePlugin;`, `use crate::world::schedule::SimPlugin;`, and `use bevy_ecs::prelude::*;` (add these THREE `use` lines once near the top, not duplicated). Append the harness + test.

  **Critical:** the harness installs `CorePlugin::default()` + `MobilityPlugin` + `EconomyPlugin` (NOT EconomyPlugin alone) — matching every existing wired-schedule test (tests/systems.rs:61-65, tests/lod.rs, tests/plugin.rs). `Tick` is provided by `MobilityPlugin`; there is NO manual `Tick` insert. The schedule runs graph-free (materialize + shopper-capture early-return without `Graph`; PayWages is graph-free).

  ```rust
  fn full_economy_world() -> (World, bevy_ecs::schedule::Schedule) {
      let mut world = World::new();
      let mut schedule = bevy_ecs::schedule::Schedule::default();
      CorePlugin::default().install(&mut world, &mut schedule);
      crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
      EconomyPlugin.install(&mut world, &mut schedule);
      (world, schedule)
  }

  #[test]
  fn full_tick_wage_loop_conserves_total_money_auction_path() {
      // Un-anchored (always-active) market: supplier@m sells TOOLS, a cash-rich
      // consumer@m buys, the auction settles → SellerReceipts → PayWages.
      let (mut world, mut schedule) = full_economy_world();
      let supplier = EconomicActorId(8_001);
      let consumer = EconomicActorId(8_002);
      let m = MarketId(1);
      world.resource_mut::<InventoryBook>().deposit(supplier, GOOD_TOOLS, Quantity(1_000_000)).unwrap();
      world.resource_mut::<AccountBook>().deposit(consumer, Money(1_000_000)).unwrap();
      world.resource_mut::<SupplyPools>().0.insert(supplier, SupplyPool {
          actor: supplier, market: m, good: GOOD_TOOLS,
          offered_qty_per_tick: Quantity(10), min_price: Money(500),
          interval_ticks: 1, last_generated_tick: None,
      });
      world.resource_mut::<DemandPools>().0.insert(consumer, consumer_pool(consumer, m));
      world.insert_resource(HouseholdSector {
          population: 1_000_000,
          pool_weights: BTreeMap::from([(consumer, 1_i64)]),
      });

      let before = world.resource::<AccountBook>().total_money().unwrap();
      // CorePlugin/MobilityPlugin own the Tick; just run the schedule (it increments).
      for _ in 0..6 {
          schedule.run(&mut world);
      }
      let after = world.resource::<AccountBook>().total_money().unwrap();
      assert_eq!(after, before, "total money byte-invariant across 6 full ticks");
      assert_eq!(
          world.resource::<AccountBook>().account(HOUSEHOLD_SECTOR).available,
          Money::ZERO,
          "sentinel nets to zero after every tick"
      );
      let earned = world.resource::<DemandPools>().0[&consumer].income_last_tick.0
          + world.resource::<AccountBook>().account(consumer).available.0;
      assert!(earned > 0, "wage loop produced income");
  }
  ```

  Note: extend the import block to also include `InventoryBook` (already present from Step 1.1) and `Tick` is NOT imported (we never touch it manually).

- [ ] **Step 3.7 — Add the macro-flow-path full-tick test (spec §8: both settle paths feed PayWages).**

  Append to `tests/wages.rs` a test that exercises the MacroFlow→SellerReceipts→PayWages path end-to-end through the schedule. A DORMANT market pair (anchored to an unobserved chunk, so the auction never clears it but the macro flow does on the interval) drives `settle_flow_with_receipts`, which must feed `PayWages`. Reuse the seed pattern from `tests/macro_flow.rs` for a dormant pair (read that file's dormant-pair setup and mirror it). Assert, on a tick that is a multiple of `macro_flow_interval_ticks` (default 10):

  ```rust
  #[test]
  fn full_tick_macro_flow_feeds_pay_wages_and_conserves() {
      // A dormant supply@src / demand@dst pair for GOOD_FOOD: the auction never clears
      // it (dormant), but run_macro_flow_at_tick settles it every interval, crediting
      // SellerReceipts, which PayWages (running .after(MacroFlow)) turns into income.
      // Spec §8: (3) no firm negative, (4) Σ income == Σ firm→household transfers, with
      // the MacroFlow settle path active.
      let (mut world, mut schedule) = full_economy_world();
      // -- mirror the dormant-pair construction from tests/macro_flow.rs here --
      //   * two markets src/dst in MarketChunks anchored to a chunk that is NOT spawned
      //     Active/Hot (so refresh_dormant_markets_system marks them dormant),
      //   * MarketDistances for (src,dst)/(dst,src),
      //   * a supplier with GOOD_FOOD inventory + SupplyPool@src,
      //   * a consumer with cash + DemandPool@dst,
      //   * a HouseholdSector paying the supplier's wages to the consumer.
      // (See tests/macro_flow.rs for the exact dormant-anchor + run-to-interval pattern;
      //  copy it verbatim, then add the HouseholdSector + the assertions below.)
      world.insert_resource(HouseholdSector {
          population: 1_000_000,
          pool_weights: BTreeMap::from([(EconomicActorId(8_022), 1_i64)]),
      });

      let before = world.resource::<AccountBook>().total_money().unwrap();
      // Run through at least one macro_flow interval boundary (default 10).
      let interval = world.resource::<EconomyConfig>().macro_flow_interval_ticks;
      for _ in 0..(interval + 2) {
          schedule.run(&mut world);
      }
      // Conservation + sentinel + (3) no firm negative.
      assert_eq!(world.resource::<AccountBook>().total_money().unwrap(), before);
      assert_eq!(world.resource::<AccountBook>().account(HOUSEHOLD_SECTOR).available, Money::ZERO);
      for acc in world.resource::<AccountBook>().accounts.values() {
          assert!(acc.available.0 >= 0 && acc.locked.0 >= 0, "(3) no firm negative");
      }
      // (4) Σ WagePaid amounts on the interval tick == Σ income credited that tick.
      // The MacroFlow path fired at least once → at least one WagePaid event exists.
      let wage_events: i64 = world.resource::<TradeLedger>().0.iter().filter_map(|e| match e {
          EconomyEvent::WagePaid { amount, .. } => Some(amount.0),
          _ => None,
      }).sum();
      assert!(wage_events > 0, "MacroFlow settle path produced wages through PayWages");
  }
  ```

  Implementation note for the implementer: the precise dormant-pair seeding (chunk anchoring + which ticks spawn chunks) is non-trivial — read `tests/macro_flow.rs` for a working dormant-only macro-flow fixture and adapt it; do NOT invent the anchoring from scratch. The load-bearing assertion is that `WagePaid` is emitted via the MacroFlow path while total money stays invariant.

- [ ] **Step 3.8 — Run.**

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core wages::full_tick` → **PASS**.
  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core persist::household` → **PASS**.
  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests` → **PASS** (no regressions; `economy_snapshot_round_trips`/`market_distances_round_trips` cover the new field via identity round-trip).

- [ ] **Step 3.9 — Commit.**

  ```
  git add -A
  git commit -m "feat(economy): seed + persist HouseholdSector, wire PayWages, assert both-settle-path full-tick conservation

  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
  ```

---

## Task 4 — `target_spend` + `spend_to_qty` pure consumption math + unit tests

Add the consumption-function math (the `DemandPool` `mpc_bps`/`autonomous` fields already landed in Step 2.6a). Pure-function unit tests only; nothing is wired into the schedule yet.

### Files
- **Modify** `backend/crates/sim-core/src/economy/pools.rs` — `target_spend`, `spend_to_qty`.
- **Modify** `backend/crates/sim-core/src/economy/tests/pools.rs` — unit tests for the new pure fns.

### Test
`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core pools::`

### Steps

- [ ] **Step 4.1 — Write failing unit tests for `target_spend` + `spend_to_qty`.**

  In `backend/crates/sim-core/src/economy/tests/pools.rs`, extend the EXISTING top `use crate::economy::{...}` block (lines 3–8) by adding `EconomyError` and `GOOD_TOOLS` (do NOT re-import `Money`/`Quantity`/`MarketGoodKey`/`MarketGoodState`/`MarketGoods`/`MarketId`/`EconomicActorId` — all already present; that would be E0252). Then add ONE new `use` line for the `pub(crate)` fns (they live in `crate::economy::pools`):

  ```rust
  use crate::economy::pools::{spend_to_qty, target_spend};
  ```

  Append the tests:

  ```rust
  #[test]
  fn target_spend_is_autonomous_plus_mpc_times_income() {
      assert_eq!(target_spend(Money(5_000), 8_000, Money(10_000)).unwrap(), Money(13_000));
  }

  #[test]
  fn target_spend_at_zero_income_is_autonomous() {
      assert_eq!(target_spend(Money(5_000), 8_000, Money::ZERO).unwrap(), Money(5_000));
  }

  #[test]
  fn target_spend_floors_the_induced_term() {
      // 0.8 * 12_345 = 9_876.0 floors to 9_876; + 1 autonomous = 9_877.
      assert_eq!(target_spend(Money(1), 8_000, Money(12_345)).unwrap(), Money(9_877));
  }

  #[test]
  fn target_spend_rejects_out_of_band_mpc() {
      assert_eq!(target_spend(Money(1), -1, Money(1)), Err(EconomyError::InvalidOrder));
      assert_eq!(target_spend(Money(1), 10_001, Money(1)), Err(EconomyError::InvalidOrder));
  }

  #[test]
  fn target_spend_full_mpc_passes_all_income() {
      assert_eq!(target_spend(Money(0), 10_000, Money(7_000)).unwrap(), Money(7_000));
  }

  #[test]
  fn spend_to_qty_inverts_affordable_qty_scale() {
      // qty = spend * SCALE / p_ref = 10_000 * 1_000 / 1_000 = 10_000.
      assert_eq!(spend_to_qty(Money(10_000), Money(1_000)).unwrap(), Quantity(10_000));
  }

  #[test]
  fn spend_to_qty_floors() {
      // 1_000 * 1_000 / 3 = 333_333.33 → floor 333_333.
      assert_eq!(spend_to_qty(Money(1_000), Money(3)).unwrap(), Quantity(333_333));
  }

  #[test]
  fn spend_to_qty_rejects_zero_price() {
      assert_eq!(spend_to_qty(Money(1), Money(0)), Err(EconomyError::ZeroPrice));
      assert_eq!(spend_to_qty(Money(1), Money(-5)), Err(EconomyError::ZeroPrice));
  }
  ```

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core pools::target_spend` → **FAIL (does not compile)**.

- [ ] **Step 4.2 — Implement `target_spend` + `spend_to_qty`.**

  In `backend/crates/sim-core/src/economy/pools.rs`, add (near `affordable_qty`). `ECONOMY_SCALE` (an `i128`) and `EconomyError`/`Money`/`Quantity` are already imported at the top of `pools.rs`:

  ```rust
  /// Keynesian consumption target (Money): `C = autonomous + floor(mpc_bps * income / 10_000)`.
  /// `mpc_bps` validated `0..=10_000`. i128 intermediate, floor, `try_from` → Overflow.
  pub(crate) fn target_spend(
      autonomous: Money,
      mpc_bps: i32,
      income_last_tick: Money,
  ) -> Result<Money, EconomyError> {
      if !(0..=10_000).contains(&mpc_bps) {
          return Err(EconomyError::InvalidOrder);
      }
      let induced =
          i64::try_from((income_last_tick.0 as i128) * (mpc_bps as i128) / 10_000)
              .map_err(|_| EconomyError::Overflow)?;
      autonomous.checked_add(Money(induced))
  }

  /// Map a target SPEND (Money) to a desired Quantity at a reference price, inverting
  /// `affordable_qty`'s SCALE math: `qty = floor(spend * ECONOMY_SCALE / p_ref)`.
  pub(crate) fn spend_to_qty(spend: Money, p_ref: Money) -> Result<Quantity, EconomyError> {
      if p_ref.0 <= 0 {
          return Err(EconomyError::ZeroPrice);
      }
      let raw = (spend.0 as i128) * ECONOMY_SCALE / p_ref.0 as i128;
      Ok(Quantity(i64::try_from(raw).map_err(|_| EconomyError::Overflow)?))
  }
  ```

- [ ] **Step 4.3 — Confirm seed values.**

  The three seed consumer pools (8_002, 8_012, 8_022) carry `mpc_bps: 8_000, autonomous: Money(5_000)` from Step 2.6a's re-fix of `seed.rs`. No additional code; just confirm by reading `seed.rs`.

- [ ] **Step 4.4 — Run.**

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core pools::` → **PASS** (new unit tests + existing pool tests).

- [ ] **Step 4.5 — Commit.**

  ```
  git add -A
  git commit -m "feat(economy): target_spend + spend_to_qty consumption math

  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
  ```

---

## Task 5 — `run_consumption_update_system` + `UpdateConsumption` + closed-loop integration test

Wire the consumption function into the schedule: read each pool's `income_last_tick` + smoothed `ewma_reference_price`, write `desired_qty_per_tick`. The 1-tick lag means the new desired quantity becomes a bid in tick T+1.

### Files
- **Modify** `backend/crates/sim-core/src/economy/pools.rs` — `run_consumption_update_at_tick` pure core.
- **Modify** `backend/crates/sim-core/src/economy/systems.rs` — `EconomySet::UpdateConsumption`; `run_consumption_update_system`.
- **Modify** `backend/crates/sim-core/src/economy/tests/pools.rs` — unit test for the update core + helper.
- **Modify** `backend/crates/sim-core/src/economy/tests/wages.rs` — closed-loop bootstrap/lag integration test.

### Test
`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core wages::closed_loop`

### Steps

- [ ] **Step 5.1 — Write the failing unit test for the update core + the local helper.**

  In `backend/crates/sim-core/src/economy/tests/pools.rs`, extend the EXISTING top `use crate::economy::{...}` block to add `EconomyConfig` and `run_consumption_update_at_tick` (everything else used by these tests — `DemandPools, GOOD_TOOLS, MarketGoodKey, MarketGoodState, MarketGoods, MarketId, EconomicActorId, Money, Quantity` — is ALREADY in scope from prior steps; do NOT re-import any of them, that is E0252). Then add the tests + helper. The helper is defined in THIS module (`tests::pools`) and called WITHOUT a path qualifier:

  ```rust
  fn pools_test_pool(actor: EconomicActorId, market: MarketId) -> DemandPool {
      DemandPool {
          actor, market, good: GOOD_TOOLS,
          desired_qty_per_tick: Quantity(0),
          max_price: Money(2_000),
          urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1,
          last_generated_tick: None, last_consumed_tick: None,
          income_last_tick: Money::ZERO, mpc_bps: 8_000, autonomous: Money(5_000),
      }
  }

  #[test]
  fn consumption_update_sets_desired_qty_from_income_and_ref_price() {
      let actor = EconomicActorId(8_002);
      let market = MarketId(9_002);
      let mut demand = DemandPools::default();
      let mut pool = pools_test_pool(actor, market);
      pool.income_last_tick = Money(10_000);
      pool.mpc_bps = 8_000;
      pool.autonomous = Money(5_000);
      demand.0.insert(actor, pool);

      let mut goods = MarketGoods::default();
      let key = MarketGoodKey { market, good: GOOD_TOOLS };
      let mut state = MarketGoodState::new(key);
      state.ewma_reference_price = Money(1_000);
      goods.0.insert(key, state);

      run_consumption_update_at_tick(&mut demand, &goods, &EconomyConfig::default()).unwrap();

      // C = 5_000 + 0.8*10_000 = 13_000; qty = 13_000 * 1_000 / 1_000 = 13_000.
      assert_eq!(demand.0[&actor].desired_qty_per_tick, Quantity(13_000));
  }

  #[test]
  fn consumption_update_falls_back_to_default_ref_price_when_ewma_zero() {
      let actor = EconomicActorId(8_002);
      let market = MarketId(9_002);
      let mut demand = DemandPools::default();
      let mut pool = pools_test_pool(actor, market);
      pool.income_last_tick = Money::ZERO; // C = autonomous = 5_000
      demand.0.insert(actor, pool);
      // No MarketGoodState present → ewma 0 → fallback to trader_default_ref_price (1_000).
      let goods = MarketGoods::default();
      run_consumption_update_at_tick(&mut demand, &goods, &EconomyConfig::default()).unwrap();
      // qty = 5_000 * 1_000 / 1_000 = 5_000.
      assert_eq!(demand.0[&actor].desired_qty_per_tick, Quantity(5_000));
  }
  ```

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core pools::consumption_update` → **FAIL (does not compile)**.

- [ ] **Step 5.2 — Implement `run_consumption_update_at_tick`.**

  In `backend/crates/sim-core/src/economy/pools.rs`, the existing top import (lines 5–9) already has `MarketGoodKey, MarketGoodState, MarketGoods`. It does NOT have `EconomyConfig`. Add ONLY `EconomyConfig` to that existing `use crate::economy::{...}` block (do NOT add a second `use`, and do NOT re-import `MarketGoodKey`/`MarketGoods` — that is E0252). Then add:

  ```rust
  /// Part B: rewrite each consumer pool's `desired_qty_per_tick` from its
  /// `income_last_tick` (booked by PayWages THIS tick) and the SMOOTHED reference price.
  /// `p_ref = ewma_reference_price` if > 0 else `config.trader_default_ref_price`. Writes
  /// a Quantity ONLY; touches no money field. Pure, deterministic, keys-first. A per-pool
  /// fault is skipped (the pool keeps its prior desired_qty), never aborting the others.
  pub fn run_consumption_update_at_tick(
      demand: &mut DemandPools,
      market_goods: &MarketGoods,
      config: &EconomyConfig,
  ) -> Result<(), EconomyError> {
      let actors: Vec<EconomicActorId> = demand.0.keys().copied().collect();
      for actor in actors {
          let pool = demand.0[&actor];
          let key = MarketGoodKey { market: pool.market, good: pool.good };
          let p_ref = match market_goods.0.get(&key) {
              Some(s) if s.ewma_reference_price.0 > 0 => s.ewma_reference_price,
              _ => config.trader_default_ref_price,
          };
          let spend = match target_spend(pool.autonomous, pool.mpc_bps, pool.income_last_tick) {
              Ok(s) => s,
              Err(_) => continue,
          };
          let qty = match spend_to_qty(spend, p_ref) {
              Ok(q) => q,
              Err(_) => continue,
          };
          if let Some(p) = demand.0.get_mut(&actor) {
              p.desired_qty_per_tick = qty;
          }
      }
      Ok(())
  }
  ```

- [ ] **Step 5.3 — Add `UpdateConsumption` set + system.**

  In `backend/crates/sim-core/src/economy/systems.rs`, extend the top import block to add `run_consumption_update_at_tick` (merge into the single `use crate::economy::{...}`).

  Add to `EconomySet` (after `Telemetry`, since `UpdateConsumption` runs `.after(PayWages).after(Telemetry)`):

  ```rust
      Telemetry,
      UpdateConsumption,
  ```

  In `install_systems`, append `EconomySet::UpdateConsumption,` to the END of the `.chain()` tuple (so it is after `Telemetry`; the chain gives `.after(PayWages)` transitively). Add to the parallel `add_systems(...)` tuple:

  ```rust
  run_consumption_update_system.in_set(EconomySet::UpdateConsumption),
  ```

  Define the system:

  ```rust
  /// Part B: rewrite each consumer pool's desired quantity from its current income +
  /// the FINAL smoothed reference price. Runs after PayWages (income) and after
  /// Telemetry (the ewma write). The new desired_qty becomes a bid in NEXT tick's
  /// GeneratePoolOrders — the explicit 1-tick income→consumption lag.
  pub fn run_consumption_update_system(
      config: Res<EconomyConfig>,
      mut demand: ResMut<DemandPools>,
      goods: Res<MarketGoods>,
  ) {
      let _ = run_consumption_update_at_tick(&mut demand, &goods, &config);
  }
  ```

- [ ] **Step 5.4 — Write the closed-loop bootstrap + lag integration test.**

  Append to `tests/wages.rs` (uses the `full_economy_world` harness + `consumer_pool` from Task 3; CorePlugin+MobilityPlugin own the Tick):

  ```rust
  #[test]
  fn closed_loop_bootstraps_from_autonomous_and_lags_one_tick() {
      // Tick 0: income=0 ⇒ desired_qty driven by autonomous only (set by UpdateConsumption
      // at end of tick 0) ⇒ subsequent ticks bid the autonomous floor ⇒ trade ⇒ wage ⇒
      // income>0. Asserts the loop is self-starting and conservative.
      let (mut world, mut schedule) = full_economy_world();
      let supplier = EconomicActorId(8_001);
      let consumer = EconomicActorId(8_002);
      let m = MarketId(1);
      world.resource_mut::<InventoryBook>().deposit(supplier, GOOD_TOOLS, Quantity(1_000_000)).unwrap();
      world.resource_mut::<AccountBook>().deposit(consumer, Money(1_000_000)).unwrap();
      world.resource_mut::<SupplyPools>().0.insert(supplier, SupplyPool {
          actor: supplier, market: m, good: GOOD_TOOLS,
          offered_qty_per_tick: Quantity(1_000), min_price: Money(500),
          interval_ticks: 1, last_generated_tick: None,
      });
      let mut pool = consumer_pool(consumer, m);
      pool.desired_qty_per_tick = Quantity(0);
      pool.good = GOOD_TOOLS;
      world.resource_mut::<DemandPools>().0.insert(consumer, pool);
      world.insert_resource(HouseholdSector {
          population: 1_000_000,
          pool_weights: BTreeMap::from([(consumer, 1_i64)]),
      });

      let before = world.resource::<AccountBook>().total_money().unwrap();

      // Tick 0: UpdateConsumption sets desired_qty from autonomous (income still 0).
      schedule.run(&mut world);
      let dq0 = world.resource::<DemandPools>().0[&consumer].desired_qty_per_tick.0;
      assert!(dq0 > 0, "autonomous term sets a positive desired_qty at tick 0 (bootstrap)");

      let mut saw_income = false;
      for _ in 0..7 {
          schedule.run(&mut world);
          if world.resource::<DemandPools>().0[&consumer].income_last_tick.0 > 0 {
              saw_income = true;
          }
          assert_eq!(world.resource::<AccountBook>().total_money().unwrap(), before, "money conserved");
          assert_eq!(world.resource::<AccountBook>().account(HOUSEHOLD_SECTOR).available, Money::ZERO);
      }
      assert!(saw_income, "the wage→income→consumption loop closed (income became positive)");
  }
  ```

- [ ] **Step 5.5 — Run.**

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core pools::consumption_update` → **PASS**.
  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core wages::closed_loop` → **PASS**.
  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests` → **PASS**.

- [ ] **Step 5.6 — Commit.**

  ```
  git add -A
  git commit -m "feat(economy): run_consumption_update_system closes the wage→income→consumption loop (1-tick lag)

  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
  ```

---

## Task 6 — `commuters.rs` projection + capture + materialize reading `WageTelemetry`

Add the visible-commuter projection: a pure `capture_commuter_trips` (twin of `capture_shopper_visits`) reading `WageTelemetry`, the ephemeral `CommuterTrips`/`NextCommuterId` resources, the `CommuterCapture` exclusive system, and the materialize render bridge. Add the commuter config knobs.

### Files
- **Create** `backend/crates/sim-core/src/economy/commuters.rs`.
- **Modify** `backend/crates/sim-core/src/economy/mod.rs` — `pub mod commuters; pub use commuters::*;`, insert resources in the plugin.
- **Modify** `backend/crates/sim-core/src/economy/systems.rs` — `commuters_per_wage_unit` + `max_commuters_per_market` config; `EconomySet::CommuterCapture`; `run_commuter_capture_system`.
- **Modify** `backend/crates/sim-core/src/economy/materialize.rs` — render commuter trips + bound `rendering_shopper_ids` + `id_prefix`.
- **Modify** `backend/crates/sim-core/src/economy/tests/mod.rs` — `mod commuters;`.
- **Create** `backend/crates/sim-core/src/economy/tests/commuters.rs`.

### Test
`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core commuters::`

### Steps

- [ ] **Step 6.1 — Add the commuter config knobs.**

  In `backend/crates/sim-core/src/economy/systems.rs`, add to `EconomyConfig` (both preserve `Copy`):

  ```rust
  /// How many wage-Money units one visible commuter represents.
  pub commuters_per_wage_unit: i64,
  /// Absolute cap on simultaneous commuters rendered per market (viewport-bounded;
  /// NEVER derived from the wage magnitude, else the 1M population would leak in).
  pub max_commuters_per_market: usize,
  ```

  In `Default`: `commuters_per_wage_unit: 100,` and `max_commuters_per_market: 4,`.

- [ ] **Step 6.2 — Write the failing pure-capture test.**

  Create `backend/crates/sim-core/src/economy/tests/commuters.rs`:

  ```rust
  use std::collections::{BTreeMap, BTreeSet};

  use crate::economy::{
      CommuterTrip, CommuterTrips, EconomyConfig, MarketId, MarketSite, Markets, Money,
      NextCommuterId, WageTelemetry, capture_commuter_trips,
  };
  use crate::routing::NodeId;

  fn markets() -> Markets {
      let mut m = Markets::default();
      m.0.insert(MarketId(9_001), MarketSite { id: MarketId(9_001), node_id: NodeId(1), name: "A".into() });
      m
  }

  #[test]
  fn capture_spawns_commuters_proportional_to_wage_capped() {
      let mut wage = WageTelemetry::default();
      wage.0.insert(MarketId(9_001), Money(1_000)); // 1_000 / 100 = 10, capped at 4
      let observed: BTreeSet<MarketId> = BTreeSet::from([MarketId(9_001)]);
      let markets = markets();
      let config = EconomyConfig::default();
      let origins = |_n: NodeId| -> Vec<(NodeId, i64)> {
          (10..16).map(|i| (NodeId(i), i as i64)).collect()
      };
      let mut trips = CommuterTrips::default();
      let mut next = NextCommuterId::default();
      capture_commuter_trips(&wage, &observed, &markets, origins, &config, 1, &mut trips, &mut next);
      assert_eq!(trips.0.len(), 4, "target = min(10, cap=4)");
      let origin_nodes: Vec<u64> = trips.0.values().map(|t| t.origin_node.0).collect();
      let mut sorted = origin_nodes.clone();
      sorted.sort_unstable();
      assert_eq!(origin_nodes, sorted, "trips take the first-N sorted origins");
      assert!(trips.0.values().all(|t| t.market == MarketId(9_001)));
  }

  #[test]
  fn capture_ignores_unobserved_and_zero_wage_markets() {
      let mut wage = WageTelemetry::default();
      wage.0.insert(MarketId(9_001), Money(0)); // zero wage → no commuters
      let observed: BTreeSet<MarketId> = BTreeSet::new();
      let markets = markets();
      let config = EconomyConfig::default();
      let origins = |_n: NodeId| -> Vec<(NodeId, i64)> { vec![(NodeId(10), 1)] };
      let mut trips = CommuterTrips::default();
      let mut next = NextCommuterId::default();
      capture_commuter_trips(&wage, &observed, &markets, origins, &config, 1, &mut trips, &mut next);
      assert!(trips.0.is_empty());
  }

  #[test]
  fn capture_tops_up_only_the_shortfall() {
      let mut wage = WageTelemetry::default();
      wage.0.insert(MarketId(9_001), Money(1_000)); // target 4
      let observed: BTreeSet<MarketId> = BTreeSet::from([MarketId(9_001)]);
      let markets = markets();
      let config = EconomyConfig::default();
      let origins = |_n: NodeId| -> Vec<(NodeId, i64)> {
          (10..20).map(|i| (NodeId(i), i as i64)).collect()
      };
      let mut trips = CommuterTrips::default();
      let mut next = NextCommuterId::default();
      for _ in 0..3 {
          let id = next.next();
          trips.0.insert(id, CommuterTrip {
              id, market: MarketId(9_001), origin_node: NodeId(99),
              start_tick: 0, travel_ticks: 5,
          });
      }
      capture_commuter_trips(&wage, &observed, &markets, origins, &config, 1, &mut trips, &mut next);
      assert_eq!(trips.0.len(), 4, "topped up 3 → 4 (shortfall 1)");
  }
  ```

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core commuters::` → **FAIL (does not compile)**.

- [ ] **Step 6.3 — Create `commuters.rs`.**

  Create `backend/crates/sim-core/src/economy/commuters.rs`:

  ```rust
  //! Render-only projection of the WAGE flow (twin of shoppers.rs / flow_shipments.rs):
  //! an observed market that PAID wages last tick spawns commuter trips that materialize
  //! as pedestrians walking home-node → market-node along the footway graph. PURE VIEW —
  //! no economic state, NOT persisted, regenerated on restart. Reads `WageTelemetry` only.

  use std::collections::{BTreeMap, BTreeSet};

  use bevy_ecs::prelude::*;

  use crate::economy::{EconomyConfig, MarketId, Markets, Money, WageTelemetry};
  use crate::routing::NodeId;

  /// Reserved actor-id offset for commuter agents; distinct from shoppers (2<<32) and
  /// flow-traders (1<<32).
  pub const COMMUTER_ACTOR_OFFSET: u64 = 3 << 32;

  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct CommuterTrip {
      pub id: u64,
      pub market: MarketId,
      pub origin_node: NodeId,
      pub start_tick: u64,
      pub travel_ticks: u64,
  }

  impl CommuterTrip {
      pub fn progress(&self, tick: u64) -> f32 {
          let elapsed = tick.saturating_sub(self.start_tick);
          (elapsed as f32 / self.travel_ticks.max(1) as f32).clamp(0.0, 1.0)
      }
      pub fn arrived(&self, tick: u64) -> bool {
          tick.saturating_sub(self.start_tick) >= self.travel_ticks
      }
  }

  /// Active commuter trips, keyed by id. Ephemeral, NOT persisted.
  #[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
  pub struct CommuterTrips(pub BTreeMap<u64, CommuterTrip>);

  /// Monotone id counter. EPHEMERAL — NOT persisted (resets to 0 on restore).
  #[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
  pub struct NextCommuterId(pub u64);

  impl NextCommuterId {
      #[allow(clippy::should_implement_trait)]
      pub fn next(&mut self) -> u64 {
          let id = self.0;
          self.0 += 1;
          id
      }
  }

  /// Visible walk time for a commuter over `dist` tiles, at the trader/shopper speed.
  /// Always `>= 1`.
  pub fn commuter_travel_ticks(dist: i64, config: &EconomyConfig) -> u64 {
      let speed = config.trader_tiles_per_tick.max(1);
      (dist.max(0) as u64).div_ceil(speed).max(1)
  }

  /// Pure capture core (no `World`). For each OBSERVED market with `wage_paid_last_tick
  /// > 0`, compute `target = min(wage / commuters_per_wage_unit, max_commuters_per_market)`,
  /// count this market's in-flight trips, and top up the shortfall with new trips taking
  /// the first-N candidates from `origins` (already SORTED by NodeId, market node excluded,
  /// Walk-routable, paired with Manhattan distance). Deterministic: BTree iteration + sorted
  /// candidates + monotone id. No RNG, no float for the count. The cap is ABSOLUTE (never
  /// from the wage magnitude) so the viewport bound holds independent of the 1M population.
  #[allow(clippy::too_many_arguments)]
  pub fn capture_commuter_trips(
      wage_telemetry: &WageTelemetry,
      observed: &BTreeSet<MarketId>,
      markets: &Markets,
      origins: impl Fn(NodeId) -> Vec<(NodeId, i64)>,
      config: &EconomyConfig,
      tick: u64,
      trips: &mut CommuterTrips,
      next: &mut NextCommuterId,
  ) {
      let per_unit = config.commuters_per_wage_unit.max(1);
      for (market, wage) in wage_telemetry.0.iter() {
          if !observed.contains(market) {
              continue;
          }
          if wage.0 <= 0 {
              continue;
          }
          let Some(site) = markets.0.get(market) else {
              continue;
          };
          let _ = Money::ZERO; // (Money imported for the doc/intent; no dead binding.)
          let target =
              (wage.0 / per_unit).clamp(0, config.max_commuters_per_market as i64) as usize;
          let current = trips.0.values().filter(|t| t.market == *market).count();
          if current >= target {
              continue;
          }
          let shortfall = target - current;
          for (origin_node, dist) in origins(site.node_id).into_iter().take(shortfall) {
              let id = next.next();
              trips.0.insert(
                  id,
                  CommuterTrip {
                      id,
                      market: *market,
                      origin_node,
                      start_tick: tick,
                      travel_ticks: commuter_travel_ticks(dist, config),
                  },
              );
          }
      }
  }

  /// Drop commuter trips that have arrived by `tick` AND whose agent is no longer
  /// rendered (ghost-free leave→despawn first). `rendering` is the set of commuter ids
  /// still materialized.
  pub fn expire_arrived_commuters(trips: &mut CommuterTrips, tick: u64, rendering: &BTreeSet<u64>) {
      trips
          .0
          .retain(|id, t| !t.arrived(tick) || rendering.contains(id));
  }
  ```

  (Remove the `let _ = Money::ZERO;` line and the `Money` import if clippy flags `Money` as unused — keep the import ONLY if `Money` is referenced; otherwise drop both. The implementer should let clippy decide and not ship a dead binding.)

- [ ] **Step 6.4 — Export + insert resources.**

  In `backend/crates/sim-core/src/economy/mod.rs`, add `pub mod commuters;` (after `pub mod auction;`/in the `pub mod` block) and `pub use commuters::*;` (in the re-export block). In `EconomyPlugin::install`:

  ```rust
  world.insert_resource(crate::economy::commuters::CommuterTrips::default());
  world.insert_resource(crate::economy::commuters::NextCommuterId::default());
  ```

  Register `mod commuters;` in `backend/crates/sim-core/src/economy/tests/mod.rs`.

- [ ] **Step 6.5 — Run the pure-capture tests.**

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core commuters::` → **PASS** (all 3).

- [ ] **Step 6.6 — Add the `CommuterCapture` exclusive system.**

  In `backend/crates/sim-core/src/economy/systems.rs`, add to `EconomySet` (after `UpdateConsumption`):

  ```rust
      UpdateConsumption,
      CommuterCapture,
  ```

  Configure `CommuterCapture` after `PayWages` (it reads the fresh `WageTelemetry`). After the `.chain()` block in `install_systems`, add:

  ```rust
  schedule.configure_sets(EconomySet::CommuterCapture.after(EconomySet::PayWages));
  ```

  Register the exclusive system separately (mirror `run_shopper_capture_system`):

  ```rust
  schedule.add_systems(
      run_commuter_capture_system
          .in_set(EconomySet::CommuterCapture)
          .before(crate::mobility::systems::tick_increment_system),
  );
  ```

  Define `run_commuter_capture_system` (clone of `run_shopper_capture_system`, reading `WageTelemetry`; reuses `config.shopper_radius_tiles` for the origin radius — see the documented reuse note in the Tech Stack section):

  ```rust
  /// Exclusive system: fill `CommuterTrips` from observed markets' wage payments
  /// (`WageTelemetry`). Mirrors `run_shopper_capture_system`: derives observed markets
  /// from chunk LOD, builds a deterministic NodeId-sorted origin provider via
  /// `NodeSpatialIndex::within_radius` (reusing `config.shopper_radius_tiles`), and
  /// delegates to the pure `capture_commuter_trips`. No-op when the spatial world is
  /// absent (pure-economy schedule).
  pub fn run_commuter_capture_system(world: &mut World) {
      use crate::economy::WageTelemetry;
      use crate::economy::commuters::{CommuterTrips, NextCommuterId, capture_commuter_trips};
      use crate::economy::transport::manhattan_tiles;
      use crate::routing::{Graph, NodeId, NodeSpatialIndex};

      if world.get_resource::<Graph>().is_none() || world.get_resource::<NodeSpatialIndex>().is_none()
      {
          return;
      }
      let tick = world.get_resource::<Tick>().map(|t| t.0).unwrap_or(0);
      let observed_chunks: BTreeSet<ChunkCoord> = {
          let mut q =
              world.query_filtered::<&ChunkCoordComp, Or<(With<ActiveChunk>, With<HotChunk>)>>();
          q.iter(world).map(|c| c.0).collect()
      };
      let captured = {
          let graph = world.resource::<Graph>();
          let markets = world.resource::<crate::economy::Markets>();
          let observed_markets: BTreeSet<MarketId> = markets
              .0
              .iter()
              .filter(|(_, site)| {
                  let pos = graph.node(site.node_id).position;
                  observed_chunks.contains(&crate::mobility::chunk_of(pos.0, pos.1, 32))
              })
              .map(|(id, _)| *id)
              .collect();
          if observed_markets.is_empty() {
              return;
          }
          let spatial = world.resource::<NodeSpatialIndex>();
          let config = *world.resource::<EconomyConfig>();
          let wage = world.resource::<WageTelemetry>().clone();
          let mut trips = world.resource::<CommuterTrips>().clone();
          let mut next = *world.resource::<NextCommuterId>();
          let origins = |market_node: NodeId| -> Vec<(NodeId, i64)> {
              let pos = graph.node(market_node).position;
              let mut cands = spatial.within_radius((pos.0, pos.1), config.shopper_radius_tiles);
              cands.sort_unstable_by_key(|n| n.0);
              cands
                  .into_iter()
                  .filter(|n| *n != market_node)
                  .map(|n| (n, manhattan_tiles(graph, n, market_node)))
                  .collect()
          };
          capture_commuter_trips(&wage, &observed_markets, markets, origins, &config, tick, &mut trips, &mut next);
          (trips, next)
      };
      *world.resource_mut::<CommuterTrips>() = captured.0;
      *world.resource_mut::<NextCommuterId>() = captured.1;
  }
  ```

- [ ] **Step 6.7 — Render commuter trips in materialize.**

  In `backend/crates/sim-core/src/economy/materialize.rs`:

  Update `id_prefix` to route commuter actors to their own namespace (commuter offset 3<<32 > shopper 2<<32, so check it FIRST):

  ```rust
  pub(crate) fn id_prefix(actor: EconomicActorId) -> &'static str {
      if actor.0 >= crate::economy::commuters::COMMUTER_ACTOR_OFFSET {
          "commuter:"
      } else if actor.0 >= crate::economy::shoppers::SHOPPER_ACTOR_OFFSET {
          "shopper:"
      } else {
          "trader:"
      }
  }
  ```

  Bound `rendering_shopper_ids` so it does NOT mis-collect commuter ids (its current unbounded `checked_sub(SHOPPER_ACTOR_OFFSET)` would succeed for commuter ids since 3<<32 > 2<<32):

  ```rust
  fn rendering_shopper_ids(materialized: &MaterializedTraders) -> std::collections::BTreeSet<u64> {
      use crate::economy::commuters::COMMUTER_ACTOR_OFFSET;
      use crate::economy::shoppers::SHOPPER_ACTOR_OFFSET;
      materialized
          .0
          .keys()
          .filter(|a| a.0 >= SHOPPER_ACTOR_OFFSET && a.0 < COMMUTER_ACTOR_OFFSET)
          .filter_map(|a| a.0.checked_sub(SHOPPER_ACTOR_OFFSET))
          .collect()
  }
  ```

  Add the commuter sibling:

  ```rust
  fn rendering_commuter_ids(materialized: &MaterializedTraders) -> std::collections::BTreeSet<u64> {
      use crate::economy::commuters::COMMUTER_ACTOR_OFFSET;
      materialized
          .0
          .keys()
          .filter(|a| a.0 >= COMMUTER_ACTOR_OFFSET)
          .filter_map(|a| a.0.checked_sub(COMMUTER_ACTOR_OFFSET))
          .collect()
  }
  ```

  Also bound `rendering_shipment_ids` is already bounded by `a.0 < SHOPPER_ACTOR_OFFSET` (no change). In `materialize_traders_system`, in BOTH lifecycle blocks (the pre-render block at lines 372-385 and the post-apply block at lines 474-485), add commuter expiry alongside the shopper one:

  ```rust
  let c_rendering = rendering_commuter_ids(world.resource::<MaterializedTraders>());
  crate::economy::commuters::expire_arrived_commuters(
      &mut world.resource_mut::<crate::economy::CommuterTrips>(),
      tick,
      &c_rendering,
  );
  ```

  In the `resource_scope` render-input loop, AFTER the existing shopper-visits loop (which ends at line 451, just before `out`), add a commuter-trips loop. **The bindings `graph`, `hpa`, `markets`, `cache` (the `Mut<FlowFieldCache>` closure param), and `out` are all already in scope inside this closure — the loop reuses them exactly as the shopper loop does:**

  ```rust
  for c in world.resource::<crate::economy::CommuterTrips>().0.values() {
      let Some(market) = markets.0.get(&c.market) else {
          continue;
      };
      if let Some(poly) = leg_polyline(graph, hpa, &mut cache, c.origin_node, market.node_id) {
          out.push((
              EconomicActorId(crate::economy::commuters::COMMUTER_ACTOR_OFFSET + c.id),
              poly,
              c.progress(tick),
              c.arrived(tick),
          ));
      }
  }
  ```

- [ ] **Step 6.8 — Run.**

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core commuters::` → **PASS**.
  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core materialize::` → **PASS** (no regression).
  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests` → **PASS**.

- [ ] **Step 6.9 — Commit.**

  ```
  git add -A
  git commit -m "feat(economy): visible commuter projection from WageTelemetry (pure capture + render bridge)

  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
  ```

---

## Task 7 — persist round-trip + determinism round-trip tests

Prove the full slice persists losslessly and is deterministic across a serialize/deserialize round-trip, per the spec's §9 determinism regression contract.

### Files
- **Modify** `backend/crates/sim-core/src/economy/tests/persist.rs` — demand-pool new-field persist coverage.
- **Modify** `backend/crates/sim-core/src/economy/tests/wages.rs` — determinism round-trip test.

### Test
`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core wages::determinism`

### Steps

- [ ] **Step 7.1 — Write the demand-pool-fields persist test.**

  In `backend/crates/sim-core/src/economy/tests/persist.rs`, add (the imports `DemandPool, DemandPools, EconomicActorId, GOOD_TOOLS, Quantity, apply_into_world, extract_from_world, EconomyPersistSnapshot` are already in scope at the top of this file):

  ```rust
  #[test]
  fn demand_pool_wage_fields_round_trip() {
      let mut world = install_economy();
      let actor = EconomicActorId(42);
      world.resource_mut::<DemandPools>().0.insert(actor, DemandPool {
          actor, market: crate::economy::MarketId(9_002), good: GOOD_TOOLS,
          desired_qty_per_tick: Quantity(7),
          max_price: crate::economy::Money(2_000),
          urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1,
          last_generated_tick: Some(3), last_consumed_tick: Some(2),
          income_last_tick: crate::economy::Money(1_234),
          mpc_bps: 7_500, autonomous: crate::economy::Money(4_321),
      });
      let snap = extract_from_world(&world);
      let bytes = serde_json::to_vec(&snap).unwrap();
      let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
      let mut fresh = install_economy();
      apply_into_world(&mut fresh, &decoded);
      let restored = fresh.resource::<DemandPools>().0[&actor];
      assert_eq!(restored.income_last_tick, crate::economy::Money(1_234));
      assert_eq!(restored.mpc_bps, 7_500);
      assert_eq!(restored.autonomous, crate::economy::Money(4_321));
      assert_eq!(snap, extract_from_world(&fresh), "identity round-trip");
  }
  ```

- [ ] **Step 7.2 — Write the determinism round-trip test.**

  Append to `tests/wages.rs`. Extend the top import block to add `EconomyPersistSnapshot, apply_into_world, extract_from_world` (merge into the single `use crate::economy::{...}`). The `build()` and `run_one_more` closures BOTH install `CorePlugin::default()` + `MobilityPlugin` + `EconomyPlugin` (via the `full_economy_world` harness / inline) — NEVER EconomyPlugin alone — and let the plugins own the `Tick`:

  ```rust
  #[test]
  fn determinism_same_snapshot_same_tick_yields_identical_desired_qty() {
      // Build a closed-loop world, run a few ticks to non-trivial income, snapshot it,
      // then run ONE more tick from the snapshot — twice, and across a serde round-trip —
      // and assert byte-identical desired_qty_per_tick per pool.
      fn build() -> (World, bevy_ecs::schedule::Schedule) {
          let (mut world, schedule) = full_economy_world();
          let supplier = EconomicActorId(8_001);
          let consumer = EconomicActorId(8_002);
          let m = MarketId(1);
          world.resource_mut::<InventoryBook>().deposit(supplier, GOOD_TOOLS, Quantity(1_000_000)).unwrap();
          world.resource_mut::<AccountBook>().deposit(consumer, Money(1_000_000)).unwrap();
          world.resource_mut::<SupplyPools>().0.insert(supplier, SupplyPool {
              actor: supplier, market: m, good: GOOD_TOOLS,
              offered_qty_per_tick: Quantity(1_000), min_price: Money(500),
              interval_ticks: 1, last_generated_tick: None,
          });
          let mut pool = consumer_pool(consumer, m);
          pool.good = GOOD_TOOLS;
          world.resource_mut::<DemandPools>().0.insert(consumer, pool);
          world.insert_resource(HouseholdSector {
              population: 1_000_000,
              pool_weights: BTreeMap::from([(consumer, 1_i64)]),
          });
          (world, schedule)
      }

      let (mut warm, mut warm_sched) = build();
      for _ in 0..5 {
          warm_sched.run(&mut warm);
      }
      let snap = extract_from_world(&warm);

      // Run one more tick from the snapshot, into a fresh fully-wired world.
      let run_one_more = |snap: &EconomyPersistSnapshot| -> BTreeMap<EconomicActorId, i64> {
          let mut world = World::new();
          let mut schedule = bevy_ecs::schedule::Schedule::default();
          CorePlugin::default().install(&mut world, &mut schedule);
          crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
          EconomyPlugin.install(&mut world, &mut schedule);
          apply_into_world(&mut world, snap);
          schedule.run(&mut world);
          world.resource::<DemandPools>().0.iter()
              .map(|(a, p)| (*a, p.desired_qty_per_tick.0))
              .collect()
      };

      let a = run_one_more(&snap);
      let b = run_one_more(&snap);
      assert_eq!(a, b, "same snapshot + same tick → identical desired_qty");

      let bytes = serde_json::to_vec(&snap).unwrap();
      let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
      let c = run_one_more(&decoded);
      assert_eq!(a, c, "serde round-trip preserves determinism");
  }
  ```

  Note: `apply_into_world` overwrites `HouseholdSector` (from the snapshot) and leaves ephemerals at the plugin defaults — so this also implicitly checks that `HouseholdSector` survives and `SellerReceipts`/`WageTelemetry`/`CommuterTrips`/`NextCommuterId` start clean post-restore. Both `run_one_more` worlds start with the MobilityPlugin `Tick` at 0; `apply_into_world` does not touch `Tick`, so the single `schedule.run` runs the same tick deterministically in both.

- [ ] **Step 7.3 — Run.**

  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core persist::demand_pool_wage_fields` → **PASS**.
  `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core wages::determinism` → **PASS**.

- [ ] **Step 7.4 — Commit.**

  ```
  git add -A
  git commit -m "test(economy): persist round-trip for wage fields + determinism round-trip for desired_qty

  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
  ```

---

## Task 8 — full local gate

Run the complete validation gate (Rust fmt-check, clippy, full sim-core + workspace tests, plus the frontend/e2e gate per the project's "run full CI gate before push" rule). Each step is a single discrete action.

### Files
- None expected (gate only). Any fix is scoped and re-committed.

### Steps

- [ ] **Step 8.1 — Clear cargo orphans.** `pgrep -f cargo` → if any, investigate/clear before proceeding (per CLAUDE.md: never run two cargo at once).

- [ ] **Step 8.2 — Rust fmt-check.** `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check` → **PASS** (if it fails, the local toolchain may be older than CI `@stable` — `rustup update stable`, reformat, re-gate; see the "rustfmt skew" memory).

- [ ] **Step 8.3 — Clippy (scoped to sim-core, deny warnings as CI does).** `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml -p sim-core --all-targets -- -D warnings` → **PASS**.

- [ ] **Step 8.4 — Full sim-core test suite (background + poll).** `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core` (run in background, poll to stay responsive) → **PASS**.

- [ ] **Step 8.5 — Workspace test (single broad run, ONLY after the scoped run is clean — never concurrent).** `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml` → **PASS**.

- [ ] **Step 8.6a — Frontend typecheck.** From the repo root, run `npm run typecheck` (= `tsc -p tsconfig.typecheck.json`; this is the project's configured typecheck of src+tests+scripts) → **PASS**.

- [ ] **Step 8.6b — Frontend unit tests.** `npm run test` (= `vitest run`) → **PASS**.

- [ ] **Step 8.6c — Frontend build.** `npm run build` (= `npm run generate:proto && node scripts/build.mjs`) → **PASS**.

- [ ] **Step 8.6d — e2e / render-smoke.** `npm run test:e2e` (= `npm run build && playwright test`) → **PASS**. Rationale: this slice does NOT add a new frontend↔backend WS message and does not touch `src/render` (commuters render via the existing materialize/agent-delta path), so the CLAUDE.md browser-smoke trigger is not strictly fired. Still, to confirm the commuter sprites reach the client, adapt `scripts/smoke-shoppers.mjs` into a `scripts/smoke-commuters.mjs` that launches the dev stack and asserts at least one `commuter:`-prefixed agent id appears in an agent-delta frame; pin the expected count the way the shopper smoke does. Run it once → **PASS**.

- [ ] **Step 8.7 — Final verification statement.** With the exact command outputs in hand (evidence before assertions): fmt-check clean, clippy clean, sim-core + workspace tests green, frontend typecheck/test/build/e2e green, commuter smoke green. Only then is the slice complete.

- [ ] **Step 8.8 — Final commit (if any gate fix or the smoke script was added).**

  ```
  git add -A
  git commit -m "chore(economy): satisfy full local gate for the wage/consumption loop slice

  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
  ```

---

## Deployment note (carry into the PR description)

This PR adds three non-default `DemandPool` fields (`income_last_tick`, `mpc_bps`, `autonomous`) and one new top-level snapshot field (`household_sector`) with NO serde-default shims. Old `economy_snapshots` rows will fail to deserialize (the #69 `market_distances` precedent). **Run `DELETE FROM economy_snapshots` exactly once before deploying this PR.** All schema changes land in this single PR, so one migration covers everything.

## Honest limits (carry into the PR description, from spec §12)

- **Endowment wind-down:** v0 firms liquidate a finite endowment rather than producing continuously; revenue ↓ → wage ↓ → income ↓ → consumption → autonomous floor, financed from shrinking wealth, until a clean audited `InsufficientFunds` halt (the audited-halt branch is unit-tested in `wages::pay_wages_firm_short_of_wage_emits_audited_halt_and_skips_its_bill`). A permanently self-sustaining loop needs continuous production (a later slice).
- **Stability:** money is conservative and `MPC·labor_share = 0.8·0.6 = 0.48 < 1`, so the 1-tick-lag recurrence is a contraction (multiplier ≈ 1.92) — monotone convergence, no limit cycle.
- **Non-wage 40%** stays as retained firm cash (no dividend channel yet — a later slice).
