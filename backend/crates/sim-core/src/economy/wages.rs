//! The SFC wage / income side of the economy: per-tick seller revenue capture
//! (`SellerReceipts`), the household clearing sentinel (`HOUSEHOLD_SECTOR`), and
//! (added later) the conservative two-leg wage transfer. Money is byte-invariant:
//! every move is an `AccountBook::transfer`; the wage sentinel nets to zero each tick.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::economy::{EconomicActorId, MarketId, Money};

/// Reserved clearing-account sentinel for the household sector, adjacent to
/// `TRANSPORT_OPERATOR = EconomicActorId(u64::MAX)`. Firms pay wages INTO this
/// account; it is fully apportioned out to consumer pools in the same tick, so
/// it nets to ZERO every PayWages (asserted in debug). Distinct from every
/// seeded id (8_001..8_022) and the actor-offset bands (`n << 32`).
pub const HOUSEHOLD_SECTOR: EconomicActorId = EconomicActorId(u64::MAX - 1);

/// Gross sales revenue credited to each `(firm, market)` THIS tick. A non-monetary
/// running statistic (NOT a money store), zeroed at the very start of every tick
/// (`EconomySet::ResetReceipts`) and NEVER persisted. The `(actor, market)` key
/// carries the market dimension for commuter attribution. Captured at the settle
/// points where seller id + market + amount are all in scope (auction + macro flow),
/// so it is coherent with the money move: a fault that discards the settle clone
/// discards its receipts too.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct SellerReceipts(pub BTreeMap<(EconomicActorId, MarketId), Money>);
