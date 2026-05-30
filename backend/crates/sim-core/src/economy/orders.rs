use bevy_ecs::prelude::*;

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct OrderBook;

#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NextOrderId(pub u64);
