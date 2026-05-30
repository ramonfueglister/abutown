use bevy_ecs::prelude::*;

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct DemandPools;

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct SupplyPools;
