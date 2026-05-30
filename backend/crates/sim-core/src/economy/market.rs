use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;

use crate::economy::{GoodId, MarketId, Money, Quantity};

pub struct MarketSite {
    pub id: MarketId,
    pub node_id: crate::routing::NodeId,
    pub name: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct MarketGoodKey {
    pub market: MarketId,
    pub good: GoodId,
}

pub struct MarketGoodState {
    pub key: MarketGoodKey,
    pub last_settlement_price: Money,
    pub ewma_reference_price: Money,
    pub traded_qty_last_tick: Quantity,
    pub unmet_demand_last_tick: Quantity,
    pub unsold_supply_last_tick: Quantity,
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
            dirty: false,
            last_cleared_tick: 0,
        }
    }
}

#[derive(Resource, Default)]
pub struct Markets(pub BTreeMap<MarketId, MarketSite>);

#[derive(Resource, Default)]
pub struct MarketGoods(pub BTreeMap<MarketGoodKey, MarketGoodState>);

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct DirtyMarketGoods(pub BTreeSet<MarketGoodKey>);
