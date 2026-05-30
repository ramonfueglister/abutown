//! Persistable snapshot of the economy ECS resources. Mirrors the mobility
//! persist pattern: a serde struct plus `extract_from_world` / `apply_into_world`.
//! Every map is represented as a sorted `Vec<(K, V)>` because `serde_json` rejects
//! non-string map keys (`InventoryBook`'s tuple key, `MarketGoods`' struct key);
//! `BTreeMap` iteration yields byte-stable order.

use bevy_ecs::world::World;
use serde::{Deserialize, Serialize};

use crate::economy::{
    AccountBook, Ask, Bid, DemandPool, DemandPools, EconomicActorId, GoodId, InventoryBalance,
    InventoryBook, MarketChunks, MarketGoodKey, MarketGoodState, MarketGoods, MarketId, MarketSite,
    Markets, MoneyAccount, NextOrderId, OrderBook, OrderId, ProductionPool, ProductionPools,
    SupplyPool, SupplyPools, Trader, Traders,
};
use crate::ids::ChunkCoord;
use crate::world::persistence::{MigrationError, SnapshotItem, SnapshotKey, SnapshotProvider};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomyPersistSnapshot {
    pub accounts: Vec<(EconomicActorId, MoneyAccount)>,
    pub inventory: Vec<((EconomicActorId, GoodId), InventoryBalance)>,
    pub bids: Vec<(OrderId, Bid)>,
    pub asks: Vec<(OrderId, Ask)>,
    pub next_order_id: u64,
    pub markets: Vec<(MarketId, MarketSite)>,
    pub market_goods: Vec<(MarketGoodKey, MarketGoodState)>,
    pub demand_pools: Vec<(EconomicActorId, DemandPool)>,
    pub supply_pools: Vec<(EconomicActorId, SupplyPool)>,
    pub production_pools: Vec<(EconomicActorId, ProductionPool)>,
    pub traders: Vec<(EconomicActorId, Trader)>,
    pub market_chunks: Vec<(MarketId, ChunkCoord)>,
}

/// Pull a snapshot out of a live economy `World`. `BTreeMap` iteration is sorted,
/// so the resulting `Vec`s — and the JSON they serialize to — are byte-stable.
pub fn extract_from_world(world: &World) -> EconomyPersistSnapshot {
    let accounts = world.resource::<AccountBook>();
    let inventory = world.resource::<InventoryBook>();
    let orders = world.resource::<OrderBook>();
    let next = world.resource::<NextOrderId>();
    let markets = world.resource::<Markets>();
    let market_goods = world.resource::<MarketGoods>();
    let demand = world.resource::<DemandPools>();
    let supply = world.resource::<SupplyPools>();
    let production = world.resource::<ProductionPools>();
    let traders = world.resource::<Traders>();
    let market_chunks = world.resource::<MarketChunks>();

    EconomyPersistSnapshot {
        accounts: accounts.accounts.iter().map(|(k, v)| (*k, *v)).collect(),
        inventory: inventory.balances.iter().map(|(k, v)| (*k, *v)).collect(),
        bids: orders.bids.iter().map(|(k, v)| (*k, v.clone())).collect(),
        asks: orders.asks.iter().map(|(k, v)| (*k, v.clone())).collect(),
        next_order_id: next.0,
        markets: markets.0.iter().map(|(k, v)| (*k, v.clone())).collect(),
        market_goods: market_goods
            .0
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect(),
        demand_pools: demand.0.iter().map(|(k, v)| (*k, *v)).collect(),
        supply_pools: supply.0.iter().map(|(k, v)| (*k, *v)).collect(),
        production_pools: production.0.iter().map(|(k, v)| (*k, v.clone())).collect(),
        traders: traders.0.iter().map(|(k, v)| (*k, v.clone())).collect(),
        market_chunks: market_chunks.0.iter().map(|(k, v)| (*k, *v)).collect(),
    }
}

/// Rebuild economy resources in a freshly-installed `EconomyPlugin` world from a
/// snapshot. Overwrites the default resources. `DormantMarkets` is left at its
/// default — it is recomputed by the LOD bridge on the next tick.
pub fn apply_into_world(world: &mut World, snap: &EconomyPersistSnapshot) {
    world.insert_resource(AccountBook {
        accounts: snap.accounts.iter().cloned().collect(),
    });
    world.insert_resource(InventoryBook {
        balances: snap.inventory.iter().cloned().collect(),
    });
    world.insert_resource(OrderBook {
        bids: snap.bids.iter().cloned().collect(),
        asks: snap.asks.iter().cloned().collect(),
    });
    world.insert_resource(NextOrderId(snap.next_order_id));
    world.insert_resource(Markets(snap.markets.iter().cloned().collect()));
    world.insert_resource(MarketGoods(snap.market_goods.iter().cloned().collect()));
    world.insert_resource(DemandPools(snap.demand_pools.iter().cloned().collect()));
    world.insert_resource(SupplyPools(snap.supply_pools.iter().cloned().collect()));
    world.insert_resource(ProductionPools(
        snap.production_pools.iter().cloned().collect(),
    ));
    world.insert_resource(Traders(snap.traders.iter().cloned().collect()));
    world.insert_resource(MarketChunks(snap.market_chunks.iter().cloned().collect()));
}

/// A `SnapshotProvider` emitting the full economy state as one JSON item. The
/// persist loop (slice 6b) dispatches by `key.kind == "economy"` to the economy
/// store. Mirrors `MobilitySnapshotProvider`.
pub struct EconomySnapshotProvider {
    pub world_id: String,
}

impl SnapshotProvider for EconomySnapshotProvider {
    fn name(&self) -> &'static str {
        "economy"
    }
    fn schema_version(&self) -> u32 {
        1
    }
    fn collect(&self, world: &World) -> Vec<SnapshotItem> {
        let snapshot = extract_from_world(world);
        let payload = serde_json::to_vec(&snapshot).expect("serde encodes EconomyPersistSnapshot");
        vec![SnapshotItem {
            key: SnapshotKey {
                world_id: self.world_id.clone(),
                kind: "economy",
                identifier: "full".to_string(),
            },
            schema_version: 1,
            payload,
        }]
    }
    fn migrate(&self, raw: SnapshotItem, _from: u32) -> Result<SnapshotItem, MigrationError> {
        Ok(raw)
    }
}
