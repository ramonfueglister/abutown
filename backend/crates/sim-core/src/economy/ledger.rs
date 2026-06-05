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
    /// Final consumption by an end-buyer (the demand-side sink), distinct from
    /// production's recipe-input `Consumed`. Both are goods-removals; splitting the
    /// variant keeps intermediate vs final consumption distinguishable in the audit log.
    FinalConsumed {
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
    /// One firm's wage payment to the household sector this tick (labor share of
    /// revenue). Emitted in ascending firm-id order for determinism.
    WagePaid {
        firm: EconomicActorId,
        market: MarketId,
        amount: Money,
    },
    /// The `EXTRACTOR_TOOLS` faucet deposited `qty` of a raw good this interval (goods-only,
    /// no money). The sole source of new `GOOD_RAW`; pairs with the recipe `Consumed`
    /// events in the per-good conservation balance.
    Regenerated {
        actor: EconomicActorId,
        good: GoodId,
        qty: Quantity,
    },
    /// One firm's profit (revenue − wage) distributed to the labor households this tick.
    /// Full-distribution v0: no owner/capitalist actor — profit flows to the existing
    /// consumer pools via the SAME pool_weights as wages. Emitted per (firm, market).
    ProfitDistributed {
        firm: EconomicActorId,
        market: MarketId,
        amount: Money,
    },
    /// The accumulated TRANSPORT_OPERATOR balance rebated to the labor households at a
    /// macro-flow interval boundary (the buyers paid the fee; it returns to them).
    TransportRebate { amount: Money },
    /// Per-tick SFC conservation heartbeat: the total money in circulation at the end of
    /// this tick. Emitted every tick by the audit system; the queryable conservation trace.
    TickAudit { tick: u64, total_money: Money },
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
            Self::FinalConsumed { .. } => "final_consumed",
            Self::OrderRejected { .. } => "order_rejected",
            Self::MarketClearFailed { .. } => "market_clear_failed",
            Self::TransportPaid { .. } => "transport_paid",
            Self::MacroFlow { .. } => "macro_flow",
            Self::WagePaid { .. } => "wage_paid",
            Self::Regenerated { .. } => "regenerated",
            Self::ProfitDistributed { .. } => "profit_distributed",
            Self::TransportRebate { .. } => "transport_rebate",
            Self::TickAudit { .. } => "tick_audit",
        }
    }
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct TradeLedger(pub Vec<EconomyEvent>);
