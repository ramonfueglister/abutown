use std::collections::BTreeSet;

use bevy_ecs::prelude::*;
use bevy_ecs::query::Or;

use crate::economy::{
    AccountBook, DemandPools, DirtyMarketGoods, DormantMarkets, EconomyError, EconomyEvent,
    InventoryBook, MarketChunks, MarketGoods, Money, NextOrderId, OrderBook, ProductionPools,
    SettlementPolicy, SupplyPools, TradeLedger, Traders, WarmMarkets,
    clear_market_good_with_policy, expire_orders_at_tick, generate_pool_orders_at_tick,
    integer_ewma, run_production_at_tick, run_traders_at_tick, run_warm_market_flow_at_tick,
};
use crate::ids::ChunkCoord;
use crate::mobility::resources::Tick;
use crate::world::components::{ActiveChunk, ChunkCoordComp, HotChunk, WarmChunk};

#[derive(SystemSet, Hash, Eq, PartialEq, Debug, Clone)]
pub enum EconomySet {
    RefreshLod,
    ExpireOrders,
    Production,
    Traders,
    GeneratePoolOrders,
    ClearMarkets,
    WarmFlow,
    Materialize,
    Telemetry,
}

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EconomyConfig {
    pub ewma_alpha_bps: u16,
    pub default_order_ttl_ticks: u64,
    pub transport_cost_per_tile_unit: Money,
    pub trader_tiles_per_tick: u64,
    pub trader_default_ref_price: Money,
    pub warm_flow_interval_ticks: u64,
    pub settlement_policy: SettlementPolicy,
}

impl Default for EconomyConfig {
    fn default() -> Self {
        Self {
            ewma_alpha_bps: 2_000,
            default_order_ttl_ticks: 10,
            transport_cost_per_tile_unit: Money(5),
            trader_tiles_per_tick: 4,
            trader_default_ref_price: Money(1_000),
            warm_flow_interval_ticks: 10,
            settlement_policy: SettlementPolicy::Anchored,
        }
    }
}

pub fn install_systems(schedule: &mut bevy_ecs::schedule::Schedule) {
    schedule.configure_sets(
        (
            EconomySet::RefreshLod,
            EconomySet::ExpireOrders,
            EconomySet::Production,
            EconomySet::Traders,
            EconomySet::GeneratePoolOrders,
            EconomySet::ClearMarkets,
            EconomySet::WarmFlow,
            EconomySet::Materialize,
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
            run_warm_market_flow_system.in_set(EconomySet::WarmFlow),
            update_market_telemetry_system.in_set(EconomySet::Telemetry),
        )
            .before(crate::mobility::systems::tick_increment_system),
    );
    // Render-only trader materialization is an exclusive system (it needs &mut
    // World to spawn/despawn agents), so it is registered separately from the
    // parallel economy systems above. The set chain places it after WarmFlow.
    schedule.add_systems(
        crate::economy::materialize::materialize_traders_system
            .in_set(EconomySet::Materialize)
            .before(crate::mobility::systems::tick_increment_system),
    );
}

/// Bridge: derive `DormantMarkets` and `WarmMarkets` from chunk LOD. A market
/// anchored (in `MarketChunks`) to a chunk that is not Active/Hot is dormant;
/// dormant markets anchored to a WarmChunk are also added to `WarmMarkets`.
/// Cheap: one pass over active/warm chunk coords + one over the anchor map.
/// Deterministic (BTree iteration, set membership).
#[allow(clippy::type_complexity)]
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

pub fn expire_orders_system(
    tick: Res<Tick>,
    mut accounts: ResMut<AccountBook>,
    mut inventory: ResMut<InventoryBook>,
    mut orders: ResMut<OrderBook>,
    mut ledger: ResMut<TradeLedger>,
    mut dirty: ResMut<DirtyMarketGoods>,
) {
    let _ = expire_orders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        tick.0,
    );
}

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
) {
    let keys: Vec<_> = dirty.0.iter().copied().collect();
    dirty.0.clear();
    for key in keys {
        if let Err(reason) = clear_market_good_with_policy(
            &mut accounts,
            &mut inventory,
            &mut orders,
            &mut ledger,
            &mut goods,
            key,
            tick.0,
            config.settlement_policy,
        ) {
            ledger.0.push(EconomyEvent::MarketClearFailed {
                market: key.market,
                good: key.good,
                reason,
            });
        }
    }
}

pub fn run_production_system(
    tick: Res<Tick>,
    mut inventory: ResMut<InventoryBook>,
    mut ledger: ResMut<TradeLedger>,
    mut production: ResMut<ProductionPools>,
) {
    let _ = run_production_at_tick(&mut inventory, &mut ledger, &mut production, tick.0);
}

pub fn update_market_telemetry(
    goods: &mut MarketGoods,
    config: EconomyConfig,
) -> Result<(), EconomyError> {
    for state in goods.0.values_mut() {
        state.ewma_reference_price = integer_ewma(
            state.ewma_reference_price,
            state.last_settlement_price,
            config.ewma_alpha_bps,
        )?;
    }
    Ok(())
}

pub fn update_market_telemetry_system(config: Res<EconomyConfig>, mut goods: ResMut<MarketGoods>) {
    let _ = update_market_telemetry(&mut goods, *config);
}

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
