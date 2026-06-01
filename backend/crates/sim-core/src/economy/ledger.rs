use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::economy::{EconomicActorId, EconomyError, GoodId, MarketId, Money, OrderId, Quantity};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    Produced {
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    },
    Consumed {
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
    TransportPaid {
        actor: EconomicActorId,
        amount: Money,
    },
    MacroFlow {
        from_market: MarketId,
        to_market: MarketId,
        good: GoodId,
        qty: Quantity,
        price: Money,
        transport: Money,
    },
}

impl EconomyEvent {
    /// Stable variant tag, stored as the indexed `event_type` column in the audit
    /// store so queries can filter by kind without parsing the JSON payload.
    /// Exhaustive match: adding a new `EconomyEvent` variant forces updating this.
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::OrderCreated { .. } => "order_created",
            Self::OrderExpired { .. } => "order_expired",
            Self::Trade { .. } => "trade",
            Self::CashLocked { .. } => "cash_locked",
            Self::CashReleased { .. } => "cash_released",
            Self::GoodsLocked { .. } => "goods_locked",
            Self::GoodsReleased { .. } => "goods_released",
            Self::Produced { .. } => "produced",
            Self::Consumed { .. } => "consumed",
            Self::OrderRejected { .. } => "order_rejected",
            Self::MarketClearFailed { .. } => "market_clear_failed",
            Self::TransportPaid { .. } => "transport_paid",
            Self::MacroFlow { .. } => "macro_flow",
        }
    }
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct TradeLedger(pub Vec<EconomyEvent>);
