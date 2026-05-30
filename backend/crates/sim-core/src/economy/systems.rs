use bevy_ecs::prelude::*;

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
}
