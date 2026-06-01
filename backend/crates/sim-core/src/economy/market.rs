use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;

use crate::economy::{GoodId, MarketId, Money, Quantity};
use crate::ids::ChunkCoord;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MarketSite {
    pub id: MarketId,
    pub node_id: crate::routing::NodeId,
    pub name: String,
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

#[derive(Resource, Default, Clone)]
pub struct MarketGoods(pub BTreeMap<MarketGoodKey, MarketGoodState>);

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct DirtyMarketGoods(pub BTreeSet<MarketGoodKey>);

/// MarketId -> the chunk that contains its market node. Populated by the spatial
/// seeder (which owns the routing `Graph`) so the economy core needs no per-tick
/// `Graph` dependency. Markets absent from this map are un-anchored and ALWAYS
/// simulated — this is what keeps pure-economy tests at full fidelity.
#[derive(Resource, Default)]
pub struct MarketChunks(pub BTreeMap<MarketId, ChunkCoord>);

/// The set of currently DORMANT markets: anchored (present in `MarketChunks`) to
/// a chunk that is NOT Active/Hot. Recomputed every tick by
/// `refresh_dormant_markets_system`. Anything not in this set runs full fidelity.
#[derive(Resource, Default)]
pub struct DormantMarkets(pub BTreeSet<MarketId>);

/// MarketId-pair -> Manhattan distance in whole tiles, stored DIRECTED both
/// ways ((a,b) and (b,a) both present) for O(1) symmetric lookup. Baked once
/// in `seed_demo_economy` from the routing `Graph`; persisted (the economy
/// core is graph-free at hydrate, so it cannot be recomputed on restore).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct MarketDistances(pub BTreeMap<(MarketId, MarketId), i64>);

/// Markets anchored (in `MarketChunks`) to a WARM chunk — they run the cheap
/// aggregate warm-flow update instead of the full auction. Subset of
/// `DormantMarkets`. Recomputed each tick by `refresh_dormant_markets_system`.
#[derive(Resource, Default)]
pub struct WarmMarkets(pub BTreeSet<MarketId>);
