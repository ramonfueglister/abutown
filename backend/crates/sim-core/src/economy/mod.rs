pub mod accounts;
pub mod auction;
pub mod audit;
pub mod flow_shipments;
pub mod goods;
pub mod ids;
pub mod inventory;
pub mod ledger;
pub mod macro_flow;
pub mod market;
pub mod materialize;
pub mod money;
pub mod orders;
pub mod persist;
pub mod pools;
pub mod production;
pub mod seed;
pub mod systems;
pub mod trader_render;
pub mod traders;
pub mod transport;

pub use accounts::*;
pub use auction::*;
pub use audit::*;
pub use flow_shipments::*;
pub use goods::*;
pub use ids::*;
pub use inventory::*;
pub use ledger::*;
pub use macro_flow::*;
pub use market::*;
pub use materialize::MaterializedTraders;
pub use money::*;
pub use orders::*;
pub use persist::*;
pub use pools::*;
pub use production::*;
pub use systems::*;
pub use trader_render::*;
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
        world.insert_resource(Traders::default());
        world.insert_resource(MarketChunks::default());
        world.insert_resource(DormantMarkets::default());
        world.insert_resource(MarketDistances::default());
        world.insert_resource(crate::economy::materialize::MaterializedTraders::default());
        world.insert_resource(crate::economy::audit::LedgerAuditCursor::default());
        world.insert_resource(crate::economy::flow_shipments::FlowShipments::default());
        world.insert_resource(crate::economy::flow_shipments::NextShipmentId::default());
        install_systems(schedule);
    }
}

#[cfg(test)]
mod tests;
