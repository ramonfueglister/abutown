use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;

use crate::econ::{GoodId, MarketId, Money, Quantity};

/// A market anchored at a real place in local world meters (M1 world model:
/// entities as truth, no tile raster). Replaces the old graph-anchored
/// `MarketSite` (which carried a routing `node_id`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MarketSite {
    pub id: MarketId,
    pub name: String,
    pub x: f32,
    pub z: f32,
}

#[derive(
    Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct MarketGoodKey {
    pub market: MarketId,
    pub good: GoodId,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MarketGoodState {
    pub key: MarketGoodKey,
    pub last_settlement_price: Money,
    pub ewma_reference_price: Money,
    pub traded_qty_last_tick: Quantity,
    pub unmet_demand_last_tick: Quantity,
    pub unsold_supply_last_tick: Quantity,
    /// Goods finally consumed at this market-good last tick (the demand-side sink).
    /// Written ONLY by `run_consumption_at_tick` (reset-all-then-accumulate); the visible
    /// shopper count projects this (economically-real demand realized as consumption).
    pub consumed_qty_last_tick: Quantity,
    pub dirty: bool,
    pub last_cleared_tick: u64,
}

impl MarketGoodState {
    /// Fresh state for a market-good that has never cleared. Initial
    /// `last_settlement_price` is `ZERO`; the first clearing's settlement price
    /// therefore clamps up to the marginal ask (see `settlement_price`).
    pub fn new(key: MarketGoodKey) -> Self {
        Self {
            key,
            last_settlement_price: Money::ZERO,
            ewma_reference_price: Money::ZERO,
            traded_qty_last_tick: Quantity::ZERO,
            unmet_demand_last_tick: Quantity::ZERO,
            unsold_supply_last_tick: Quantity::ZERO,
            consumed_qty_last_tick: Quantity::ZERO,
            dirty: false,
            last_cleared_tick: 0,
        }
    }
}

#[derive(Resource, Default)]
pub struct Markets(pub BTreeMap<MarketId, MarketSite>);

#[derive(Resource, Default, Clone)]
pub struct MarketGoods(pub BTreeMap<MarketGoodKey, MarketGoodState>);

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct DirtyMarketGoods(pub BTreeSet<MarketGoodKey>);

/// MarketId-pair -> euclidean distance in whole meters (`crate::econ::euclid_m`
/// over the two `MarketSite` positions), stored DIRECTED both ways ((a,b) and
/// (b,a) both present) for O(1) symmetric lookup. Baked once by the market
/// seeder (Task 5). M1 has no chunk/LOD machinery: markets are NEVER dormant,
/// so the old `MarketChunks`/`DormantMarkets` resources are gone.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct MarketDistances(pub BTreeMap<(MarketId, MarketId), i64>);
