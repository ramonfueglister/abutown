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
pub mod systems;

pub use accounts::*;
#[allow(unused_imports)] // filled in Task 6
pub use auction::*;
pub use goods::*;
pub use ids::*;
pub use inventory::*;
pub use ledger::*;
pub use market::*;
pub use money::*;
pub use orders::*;
pub use pools::*;
pub use systems::*;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::Schedule;

pub struct EconomyPlugin;

impl crate::world::schedule::SimPlugin for EconomyPlugin {
    fn name(&self) -> &'static str {
        "economy"
    }

    // Skeleton stubs (Orders/Ledger/Markets/Pools) are still unit structs; the
    // `default_constructed_unit_structs` lint fires here. Suppress until Tasks 4-5
    // replace them with real BTreeMap-backed structs.
    #[allow(clippy::default_constructed_unit_structs)]
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
        world.insert_resource(NextOrderId::default());
        world.insert_resource(EconomyConfig::default());
        install_systems(schedule);
    }
}

#[cfg(test)]
mod tests;
