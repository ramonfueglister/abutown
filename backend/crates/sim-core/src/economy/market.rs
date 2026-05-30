use bevy_ecs::prelude::*;

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct Markets;

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct MarketGoods;

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct DirtyMarketGoods;
