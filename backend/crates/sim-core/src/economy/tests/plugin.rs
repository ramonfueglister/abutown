use bevy_ecs::prelude::*;

use crate::economy::{
    AccountBook, DirtyMarketGoods, EconomyConfig, EconomyPlugin, MarketGoods, Markets, NextOrderId,
    OrderBook, TradeLedger,
};
use crate::world::plugin::CorePlugin;
use crate::world::schedule::SimPlugin;

#[test]
fn economy_plugin_installs_books_orderbook_ledger_and_sets() {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();

    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    assert!(world.contains_resource::<AccountBook>());
    assert!(world.contains_resource::<crate::economy::InventoryBook>());
    assert!(world.contains_resource::<OrderBook>());
    assert!(world.contains_resource::<TradeLedger>());
    assert!(world.contains_resource::<Markets>());
    assert!(world.contains_resource::<MarketGoods>());
    assert!(world.contains_resource::<DirtyMarketGoods>());
    assert!(world.contains_resource::<NextOrderId>());
    assert!(world.contains_resource::<EconomyConfig>());

    schedule.run(&mut world);
}
