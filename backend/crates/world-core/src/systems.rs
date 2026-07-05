//! Schedule wiring for the persistent world (Task 6): the WorldClock advances
//! every tick; the economy chain runs at 1 Hz (every [`ECONOMY_CADENCE_TICKS`]th
//! tick), in the exact harvested order, ending in the fail-fast SFC audit.
//!
//! Rebuilt LEANER than bbd0159's sim-core systems.rs: no Attribution /
//! Materialize / LOD / Dormancy systems (M1 markets are NEVER dormant — every
//! wrapper passes an EMPTY dormant set, see `econ/mod.rs`), no transport-rebate
//! or telemetry stages (not part of the M1 chain; the seeded opening reference
//! prices carry `consumption_update`).

use std::collections::BTreeSet;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;

use crate::citizens::{SeedParams, seed_citizens};
use crate::clock::WorldClock;
use crate::econ::{
    self, AccountBook, BuyerOutlays, CapitaFactor, DemandPools, DirtyMarketGoods, EconomyConfig,
    EconomyError, EconomyEvent, FlowRateEwma, FlowShipments, GoodId, HouseholdSector, InputPools,
    InventoryBook, LastTickMoney, MarketDistances, MarketGoods, MarketId, NextOrderId,
    NextShipmentId, OrderBook, ProducerPolicies, ProductionPools, RawDeposits, RealizedFlows,
    SellerReceipts, SupplyPools, TradeLedger, WageTelemetry, clear_market_good_with_receipts,
    expire_orders_at_tick, generate_pool_orders_at_tick, run_consumption_at_tick,
    run_consumption_update_at_tick, run_distribute_profit_at_tick,
    run_generate_input_orders_at_tick, run_macro_flow_at_tick, run_pay_wages_at_tick,
    run_production_at_tick, run_regen_at_tick, run_tick_audit_at_tick,
};
use crate::model::SimWorld;

/// The economy chain runs every Nth world tick (10 ticks @ 10 Hz = 1 Hz).
/// `EconomyConfig::macro_flow_interval_ticks` (default 10) stays phase-locked:
/// every econ tick is a multiple of 10, so the macro flow runs each econ round.
pub const ECONOMY_CADENCE_TICKS: u64 = 10;

/// EconomyCadence guard: the econ chain fires only on ticks the clock has
/// already advanced ONTO a cadence multiple (the clock advances first, so the
/// first econ round is world_tick 10 — never the pre-advance tick 0).
fn econ_tick(clock: &WorldClock) -> Option<u64> {
    clock
        .world_tick
        .is_multiple_of(ECONOMY_CADENCE_TICKS)
        .then_some(clock.world_tick)
}

/// `SimWorld` is deliberately NOT a `Resource` (it is shared read-only state,
/// also owned by the traffic shell); the newtype makes the `Arc` insertable.
#[derive(Resource, Clone)]
pub struct SharedSimWorld(pub Arc<SimWorld>);

/// Everything needed to install the world sim into an ECS world + schedule.
pub struct WorldCorePlugin {
    pub seed: econ::seed::EconomySeed,
    pub sim_world: Arc<SimWorld>,
    pub seed_params: SeedParams,
}

/// Insert all world/economy resources, seed the economy from the authored
/// seed (idempotent — safe on the hydrate path), and register the tick
/// systems. Seed failure is a boot-time authoring/config bug ⇒ panic.
pub fn install_world_systems(world: &mut World, schedule: &mut Schedule, plugin: &WorldCorePlugin) {
    world.init_resource::<WorldClock>();
    world.init_resource::<EconomyConfig>();
    world.init_resource::<CapitaFactor>();
    world.init_resource::<AccountBook>();
    world.init_resource::<InventoryBook>();
    world.init_resource::<OrderBook>();
    world.init_resource::<TradeLedger>();
    world.init_resource::<DirtyMarketGoods>();
    world.init_resource::<NextOrderId>();
    world.init_resource::<SellerReceipts>();
    world.init_resource::<BuyerOutlays>();
    world.init_resource::<WageTelemetry>();
    world.init_resource::<FlowShipments>();
    world.init_resource::<NextShipmentId>();
    world.init_resource::<RealizedFlows>();
    world.init_resource::<FlowRateEwma>();
    world.init_resource::<LastTickMoney>();
    world.insert_resource(SharedSimWorld(Arc::clone(&plugin.sim_world)));

    econ::seed::seed_economy(world, &plugin.seed, &plugin.sim_world)
        .expect("seed_economy: the authored economy.json seed must apply cleanly at boot");

    // Bürger NACH der Wirtschaft: seed_citizens koppelt die Kopfzahl in die
    // eben geseedete `HouseholdSector`-Resource. Beide Seeds sind idempotent
    // (hydrate-sicher). SeedParams bleibt als Resource liegen — der
    // Tagesrhythmus (Task 8) liest denselben deterministischen `seed`.
    world.insert_resource(plugin.seed_params);
    seed_citizens(world, &plugin.sim_world, &plugin.seed_params);

    schedule.add_systems(
        (
            advance_world_clock_system,
            reset_seller_receipts_system,
            expire_orders_system,
            run_regen_system,
            run_production_system,
            generate_pool_orders_system,
            clear_dirty_markets_system,
            run_macro_flow_system,
            run_pay_wages_system,
            run_distribute_profit_system,
            run_consumption_system,
            run_adjust_reservation_prices_system,
            run_consumption_update_system,
            run_tick_audit_system,
        )
            .chain(),
    );
}

/// The very first system every tick: time exists before anything reads it.
pub fn advance_world_clock_system(mut clock: ResMut<WorldClock>) {
    clock.advance();
}

/// Econ-round start: clear `SellerReceipts`/`BuyerOutlays` so wages/profit see
/// exactly one round of revenue/charges (the harvested reset-then-accumulate
/// contract of `run_pay_wages_at_tick`).
pub fn reset_seller_receipts_system(
    clock: Res<WorldClock>,
    mut receipts: ResMut<SellerReceipts>,
    mut outlays: ResMut<BuyerOutlays>,
) {
    if econ_tick(&clock).is_none() {
        return;
    }
    receipts.0.clear();
    outlays.0.clear();
}

pub fn expire_orders_system(
    clock: Res<WorldClock>,
    mut accounts: ResMut<AccountBook>,
    mut inventory: ResMut<InventoryBook>,
    mut orders: ResMut<OrderBook>,
    mut ledger: ResMut<TradeLedger>,
    mut dirty: ResMut<DirtyMarketGoods>,
) {
    let Some(tick) = econ_tick(&clock) else {
        return;
    };
    let _ = expire_orders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        tick,
    );
}

/// The goods-only raw faucet. `.expect`: the flow-capped faucet cannot
/// overflow a sane i64 balance — an Err is a bug, surfaced loudly.
pub fn run_regen_system(
    clock: Res<WorldClock>,
    capita: Res<CapitaFactor>,
    mut inventory: ResMut<InventoryBook>,
    mut ledger: ResMut<TradeLedger>,
    mut deposits: ResMut<RawDeposits>,
) {
    let Some(tick) = econ_tick(&clock) else {
        return;
    };
    run_regen_at_tick(&mut inventory, &mut ledger, &mut deposits, tick, capita.0)
        .expect("run_regen_at_tick is infallible by construction; an Err is a bug");
}

pub fn run_production_system(
    clock: Res<WorldClock>,
    capita: Res<CapitaFactor>,
    mut inventory: ResMut<InventoryBook>,
    mut ledger: ResMut<TradeLedger>,
    mut production: ResMut<ProductionPools>,
) {
    let Some(tick) = econ_tick(&clock) else {
        return;
    };
    run_production_at_tick(&mut inventory, &mut ledger, &mut production, tick, capita.0).expect(
        "run_production_at_tick: an Err (Overflow from a too-large capita scale) is a bug — fail loud, never silently skip production",
    );
}

/// Consumer/supplier pool orders + Leontief input orders in one system (the
/// two passes touch disjoint actor sets — the seed-enforced invariant).
/// M1: dormant set is ALWAYS empty (markets never sleep, see econ/mod.rs).
#[allow(clippy::too_many_arguments)]
pub fn generate_pool_orders_system(
    clock: Res<WorldClock>,
    config: Res<EconomyConfig>,
    capita: Res<CapitaFactor>,
    mut accounts: ResMut<AccountBook>,
    mut inventory: ResMut<InventoryBook>,
    mut orders: ResMut<OrderBook>,
    mut ledger: ResMut<TradeLedger>,
    mut dirty: ResMut<DirtyMarketGoods>,
    mut next: ResMut<NextOrderId>,
    mut demand: ResMut<DemandPools>,
    mut supply: ResMut<SupplyPools>,
    mut input_pools: ResMut<InputPools>,
    policies: Res<ProducerPolicies>,
    market_goods: Res<MarketGoods>,
) {
    let Some(tick) = econ_tick(&clock) else {
        return;
    };
    let dormant = BTreeSet::new();
    generate_pool_orders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut demand,
        &mut supply,
        tick,
        config.default_order_ttl_ticks,
        &dormant,
        capita.0,
    )
    .expect(
        "generate_pool_orders_at_tick: an Err (Overflow or ZeroPrice) is a bug/mis-scale — fail loud rather than silently drop orders",
    );
    run_generate_input_orders_at_tick(
        &mut accounts,
        &mut orders,
        &inventory,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut input_pools,
        &policies,
        &market_goods,
        &config,
        tick,
        config.default_order_ttl_ticks,
        &dormant,
        capita.0,
    )
    .expect(
        "run_generate_input_orders_at_tick: an Err (ZeroPrice, Overflow, or InputPool/ProducerPolicy mismatch) is a bug — fail loud rather than silently drop input orders",
    );
}

#[allow(clippy::too_many_arguments)]
pub fn clear_dirty_markets_system(
    clock: Res<WorldClock>,
    config: Res<EconomyConfig>,
    mut accounts: ResMut<AccountBook>,
    mut inventory: ResMut<InventoryBook>,
    mut orders: ResMut<OrderBook>,
    mut ledger: ResMut<TradeLedger>,
    mut goods: ResMut<MarketGoods>,
    mut dirty: ResMut<DirtyMarketGoods>,
    mut receipts: ResMut<SellerReceipts>,
    mut outlays: ResMut<BuyerOutlays>,
) {
    let Some(tick) = econ_tick(&clock) else {
        return;
    };
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
            tick,
            config.settlement_policy,
            &mut receipts.0,
            &mut outlays.0,
        ) {
            ledger.0.push(EconomyEvent::MarketClearFailed {
                market: key.market,
                good: key.good,
                reason,
            });
        }
    }
}

/// Bundled params keeping `run_macro_flow_system` within Bevy's 16-param
/// limit: shipment/telemetry state + the producer input-demand resources the
/// flow sources firm demand from.
#[derive(SystemParam)]
pub struct MacroFlowParams<'w> {
    pub shipments: ResMut<'w, FlowShipments>,
    pub next_id: ResMut<'w, NextShipmentId>,
    pub realized: ResMut<'w, RealizedFlows>,
    pub ewma: ResMut<'w, FlowRateEwma>,
    pub input_pools: ResMut<'w, InputPools>,
    pub policies: Res<'w, ProducerPolicies>,
    pub capita: Res<'w, CapitaFactor>,
}

#[allow(clippy::too_many_arguments)]
pub fn run_macro_flow_system(
    clock: Res<WorldClock>,
    config: Res<EconomyConfig>,
    dirty: Res<DirtyMarketGoods>,
    distances: Res<MarketDistances>,
    mut accounts: ResMut<AccountBook>,
    mut inventory: ResMut<InventoryBook>,
    mut ledger: ResMut<TradeLedger>,
    demand: Res<DemandPools>,
    supply: Res<SupplyPools>,
    mut market_goods: ResMut<MarketGoods>,
    mut flow: MacroFlowParams,
    mut orders: ResMut<OrderBook>,
    mut next_order_id: ResMut<NextOrderId>,
    mut receipts: ResMut<SellerReceipts>,
    mut outlays: ResMut<BuyerOutlays>,
) {
    let Some(tick) = econ_tick(&clock) else {
        return;
    };
    let dormant = BTreeSet::new();
    match run_macro_flow_at_tick(
        &mut accounts,
        &mut inventory,
        &mut ledger,
        &demand,
        &supply,
        &mut flow.input_pools,
        &flow.policies,
        flow.capita.0,
        &mut market_goods,
        &dirty,
        &dormant,
        &distances,
        &config,
        tick,
        &mut flow.shipments,
        &mut flow.next_id,
        &mut flow.realized,
        &mut orders,
        &mut next_order_id,
        &mut receipts.0,
        &mut outlays.0,
    ) {
        Ok(()) => {
            // Fold this interval's realized flows into the on-wire EWMA —
            // gated on the SAME interval condition the macro flow uses
            // internally, so smoothing steps once per macro-flow run.
            if config.macro_flow_interval_ticks != 0
                && tick.is_multiple_of(config.macro_flow_interval_ticks)
            {
                econ::update_flow_rate_ewma(&mut flow.ewma, &flow.realized);
            }
        }
        Err(reason) => {
            // A whole-interval failure is audited; the atomic boundary left
            // the books unchanged. market/good = sentinel for a tick-level
            // fault not attributable to one (market, good).
            ledger.0.push(EconomyEvent::MarketClearFailed {
                market: MarketId(0),
                good: GoodId(0),
                reason,
            });
        }
    }
}

/// SFC wage step: firms pay the labor share of this round's value added into
/// the household sector. Runs after BOTH settle paths (auction + macro flow).
#[allow(clippy::too_many_arguments)]
pub fn run_pay_wages_system(
    clock: Res<WorldClock>,
    config: Res<EconomyConfig>,
    receipts: Res<SellerReceipts>,
    outlays: Res<BuyerOutlays>,
    household: Res<HouseholdSector>,
    mut accounts: ResMut<AccountBook>,
    mut demand: ResMut<DemandPools>,
    mut wage_telemetry: ResMut<WageTelemetry>,
    mut ledger: ResMut<TradeLedger>,
) {
    if econ_tick(&clock).is_none() {
        return;
    }
    run_pay_wages_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut wage_telemetry,
        &mut ledger,
        &config,
        &outlays,
    )
    .expect("run_pay_wages_at_tick is infallible by construction (wage <= just-credited value-added, Σweights guards); an Err is a bug");
}

/// Profit distribution, after wages (deterministic income order: wage credit
/// first, profit on top). A genuine Err is audited as `MarketClearFailed`;
/// a `ConservationViolation` (sentinel-stranded cash) halts — SFC is broken.
#[allow(clippy::too_many_arguments)]
pub fn run_distribute_profit_system(
    clock: Res<WorldClock>,
    config: Res<EconomyConfig>,
    receipts: Res<SellerReceipts>,
    household: Res<HouseholdSector>,
    outlays: Res<BuyerOutlays>,
    policies: Res<ProducerPolicies>,
    input_pools: Res<InputPools>,
    capita: Res<CapitaFactor>,
    mut accounts: ResMut<AccountBook>,
    mut demand: ResMut<DemandPools>,
    mut ledger: ResMut<TradeLedger>,
) {
    if econ_tick(&clock).is_none() {
        return;
    }
    if let Err(reason) = run_distribute_profit_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut ledger,
        &config,
        &outlays,
        &policies,
        &input_pools,
        capita.0,
    ) {
        assert_ne!(
            reason,
            EconomyError::ConservationViolation,
            "CONSERVATION VIOLATION: HOUSEHOLD_SECTOR sentinel non-zero after profit distribution — the SFC invariant is broken; halting. This must never happen."
        );
        ledger.0.push(EconomyEvent::MarketClearFailed {
            market: MarketId(0),
            good: GoodId(0),
            reason,
        });
    }
}

/// Demand-side sink: consume delivered goods (`FinalConsumed`), after both
/// delivery paths, before next round's order generation.
pub fn run_consumption_system(
    clock: Res<WorldClock>,
    mut inventory: ResMut<InventoryBook>,
    mut ledger: ResMut<TradeLedger>,
    mut demand: ResMut<DemandPools>,
    mut market_goods: ResMut<MarketGoods>,
) {
    let Some(tick) = econ_tick(&clock) else {
        return;
    };
    let _ = run_consumption_at_tick(
        &mut inventory,
        &mut ledger,
        &mut demand,
        &mut market_goods,
        tick,
    );
}

/// Cadence-gated reservation-price nudge (same slow timescale as macro_flow).
/// Two independent passes; either pass's Err is audited and the other still
/// runs (neither moves money, any partial result is safe).
pub fn run_adjust_reservation_prices_system(
    clock: Res<WorldClock>,
    config: Res<EconomyConfig>,
    market_goods: Res<MarketGoods>,
    realized: Res<RealizedFlows>,
    mut demand: ResMut<DemandPools>,
    mut supply: ResMut<SupplyPools>,
    mut ledger: ResMut<TradeLedger>,
) {
    let Some(tick) = econ_tick(&clock) else {
        return;
    };
    if config.macro_flow_interval_ticks == 0
        || !tick.is_multiple_of(config.macro_flow_interval_ticks)
    {
        return;
    }
    let (skip_demand, skip_supply) = econ::pricing::flow_coupled_keys(&realized);
    if let Err(reason) = econ::pricing::run_adjust_reservation_prices_at_tick(
        &mut demand,
        &mut supply,
        &market_goods,
        &config,
        &skip_demand,
        &skip_supply,
    ) {
        ledger.0.push(EconomyEvent::MarketClearFailed {
            market: MarketId(0),
            good: GoodId(0),
            reason,
        });
    }
    if let Err(reason) = econ::pricing::run_flow_margin_feedback_at_tick(
        &mut demand,
        &mut supply,
        &realized,
        &config,
    ) {
        ledger.0.push(EconomyEvent::MarketClearFailed {
            market: MarketId(0),
            good: GoodId(0),
            reason,
        });
    }
}

/// Part B of the consumption loop: rewrite desired quantities from income +
/// the smoothed reference price (the explicit 1-round income→consumption lag).
pub fn run_consumption_update_system(
    clock: Res<WorldClock>,
    mut demand: ResMut<DemandPools>,
    goods: Res<MarketGoods>,
    capita: Res<CapitaFactor>,
) {
    if econ_tick(&clock).is_none() {
        return;
    }
    run_consumption_update_at_tick(&mut demand, &goods, capita.0)
        .expect("run_consumption_update_at_tick is infallible once every consumer market has a seeded opening price; an Err is a bug");
}

/// End-of-round SFC conservation audit. Fail-fast: a drift is an
/// unrecoverable invariant break ⇒ panic with the tick + error.
pub fn run_tick_audit_system(
    clock: Res<WorldClock>,
    accounts: Res<AccountBook>,
    mut ledger: ResMut<TradeLedger>,
    mut last: ResMut<LastTickMoney>,
) {
    let Some(tick) = econ_tick(&clock) else {
        return;
    };
    if let Err(err) = run_tick_audit_at_tick(&accounts, &mut ledger, &mut last, tick) {
        panic!(
            "CONSERVATION VIOLATION at world_tick {tick}: {err:?} — total_money changed between econ rounds (money minted/destroyed); halting. This must never happen."
        );
    }
}
