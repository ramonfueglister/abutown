pub mod accounts;
pub mod auction;
pub mod goods;
pub mod ids;
pub mod inventory;
pub mod ledger;
pub mod market;
pub mod money;
pub mod orders;
pub mod pools;
pub mod production;
pub mod systems;
pub mod traders;
pub mod transport;

pub use accounts::*;
pub use auction::*;
pub use goods::*;
pub use ids::*;
pub use inventory::*;
pub use ledger::*;
pub use market::*;
pub use money::*;
pub use orders::*;
pub use pools::*;
pub use production::*;
pub use systems::*;
pub use traders::*;
pub use transport::*;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::Schedule;

pub struct EconomyPlugin;

impl crate::world::schedule::SimPlugin for EconomyPlugin {
    fn name(&self) -> &'static str {
        "economy"
    }

    fn install(&self, world: &mut World, schedule: &mut Schedule) {
        world.insert_resource(AccountBook::default());
        world.insert_resource(InventoryBook::default());
        world.insert_resource(OrderBook::default());
        world.insert_resource(TradeLedger::default());
        world.insert_resource(Markets::default());
        world.insert_resource(MarketGoods::default());
        world.insert_resource(DirtyMarketGoods::default());
        world.insert_resource(DemandPools::default());
        world.insert_resource(SupplyPools::default());
        world.insert_resource(ProductionPools::default());
        world.insert_resource(NextOrderId::default());
        world.insert_resource(EconomyConfig::default());
        install_systems(schedule);
    }
}

#[cfg(test)]
mod tests;
