use bevy_ecs::prelude::*;

use crate::economy::{EconomicActorId, EconomyError, GoodId, MarketId, Money, OrderId, Quantity};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EconomyEvent {
    OrderCreated {
        order: OrderId,
        actor: EconomicActorId,
        market: MarketId,
        good: GoodId,
    },
    OrderExpired {
        order: OrderId,
        actor: EconomicActorId,
        market: MarketId,
        good: GoodId,
    },
    Trade {
        market: MarketId,
        good: GoodId,
        buyer: EconomicActorId,
        seller: EconomicActorId,
        qty: Quantity,
        price: Money,
        total: Money,
    },
    CashLocked {
        actor: EconomicActorId,
        amount: Money,
    },
    CashReleased {
        actor: EconomicActorId,
        amount: Money,
    },
    GoodsLocked {
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    },
    GoodsReleased {
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    },
    OrderRejected {
        actor: EconomicActorId,
        market: MarketId,
        good: GoodId,
        reason: EconomyError,
    },
    MarketClearFailed {
        market: MarketId,
        good: GoodId,
        reason: EconomyError,
    },
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct TradeLedger(pub Vec<EconomyEvent>);
