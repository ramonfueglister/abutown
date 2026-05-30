use bevy_ecs::prelude::*;

use crate::economy::{
    AccountBook, DemandPools, DirtyMarketGoods, EconomyError, EconomyEvent, InventoryBook,
    MarketGoods, NextOrderId, OrderBook, SupplyPools, TradeLedger, clear_market_good,
    expire_orders_at_tick, generate_pool_orders_at_tick, integer_ewma,
};
use crate::mobility::resources::Tick;

#[derive(SystemSet, Hash, Eq, PartialEq, Debug, Clone)]
pub enum EconomySet {
    ExpireOrders,
    GeneratePoolOrders,
    ClearMarkets,
    Telemetry,
}

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EconomyConfig {
    pub ewma_alpha_bps: u16,
    pub default_order_ttl_ticks: u64,
}

impl Default for EconomyConfig {
    fn default() -> Self {
        Self {
            ewma_alpha_bps: 2_000,
            default_order_ttl_ticks: 10,
        }
    }
}

pub fn install_systems(schedule: &mut bevy_ecs::schedule::Schedule) {
    schedule.configure_sets(
        (
            EconomySet::ExpireOrders,
            EconomySet::GeneratePoolOrders,
            EconomySet::ClearMarkets,
            EconomySet::Telemetry,
        )
            .chain(),
    );
    schedule.add_systems(
        (
            expire_orders_system.in_set(EconomySet::ExpireOrders),
            generate_pool_orders_system.in_set(EconomySet::GeneratePoolOrders),
            clear_dirty_markets_system.in_set(EconomySet::ClearMarkets),
            update_market_telemetry_system.in_set(EconomySet::Telemetry),
        )
            .before(crate::mobility::systems::tick_increment_system),
    );
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
    );
}

#[allow(clippy::too_many_arguments)]
pub fn clear_dirty_markets_system(
    tick: Res<Tick>,
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
        if let Err(reason) = clear_market_good(
            &mut accounts,
            &mut inventory,
            &mut orders,
            &mut ledger,
            &mut goods,
            key,
            tick.0,
        ) {
            ledger.0.push(EconomyEvent::MarketClearFailed {
                market: key.market,
                good: key.good,
                reason,
            });
        }
    }
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
