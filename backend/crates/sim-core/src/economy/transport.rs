use crate::economy::{
    AccountBook, DemandPools, EconomicActorId, EconomyError, EconomyEvent, HOUSEHOLD_SECTOR,
    HouseholdSector, Money, Quantity, TradeLedger, apportion_cash, checked_order_value,
};
use crate::routing::{Graph, NodeId};

/// Reserved account that receives transport-cost payments (keeps money conserved).
pub const TRANSPORT_OPERATOR: EconomicActorId = EconomicActorId(u64::MAX);

/// Integer Manhattan distance in whole tiles. Positions are rounded to integer
/// tiles first, then subtracted as integers — no float subtraction/sqrt, so the
/// result is identical on every platform.
pub fn manhattan_tiles(graph: &Graph, from: NodeId, to: NodeId) -> i64 {
    let a = graph.node(from).position;
    let b = graph.node(to).position;
    let ax = a.0.round() as i64;
    let ay = a.1.round() as i64;
    let bx = b.0.round() as i64;
    let by = b.1.round() as i64;
    (ax - bx).abs() + (ay - by).abs()
}

/// Cost to move `qty` of a good across `distance_tiles` at `rate` (Money per
/// tile per unit). Reuses the fixed-point order-value helper; all checked i128.
/// Rate must be > 0 (v0 requirement). Returns `EconomyError::InvalidOrder` for
/// negative distances and `EconomyError::Overflow` on arithmetic overflow.
pub fn transport_cost(
    distance_tiles: i64,
    qty: Quantity,
    rate: Money,
) -> Result<Money, EconomyError> {
    if distance_tiles < 0 {
        return Err(EconomyError::InvalidOrder);
    }
    if distance_tiles == 0 {
        return Ok(Money(0));
    }
    let per_tile = checked_order_value(rate, qty)?;
    let raw = (per_tile.0 as i128)
        .checked_mul(distance_tiles as i128)
        .ok_or(EconomyError::Overflow)?;
    i64::try_from(raw)
        .map(Money)
        .map_err(|_| EconomyError::Overflow)
}

/// Transport cost between two market nodes for `qty`.
pub fn transport_cost_between(
    graph: &Graph,
    from: NodeId,
    to: NodeId,
    qty: Quantity,
    rate: Money,
) -> Result<Money, EconomyError> {
    transport_cost(manhattan_tiles(graph, from, to), qty, rate)
}

/// Recycle the accumulated `TRANSPORT_OPERATOR` balance back to the labor households (the
/// buyers paid the transport fee, so it returns to them). Drains the ENTIRE operator
/// balance → `HOUSEHOLD_SECTOR`, then `apportion_cash` over the SAME `pool_weights` as
/// wages → households, crediting `income_last_tick` (ADD). Emits `TransportRebate` with
/// the drained amount. Conservation: only `transfer`s ⇒ `total_money` byte-invariant; own
/// independent `HOUSEHOLD_SECTOR` net-zero `debug_assert`. The macro-flow-interval gating
/// (phase-locked to the operator CREDIT in macro_flow, no persisted cursor) is applied in
/// the system wrapper, NOT here — this function always drains whatever is present.
pub fn run_transport_rebate_at_tick(
    accounts: &mut AccountBook,
    demand: &mut DemandPools,
    household: &HouseholdSector,
    ledger: &mut TradeLedger,
) -> Result<(), EconomyError> {
    let amount = accounts.account(TRANSPORT_OPERATOR).available;
    if amount.0 <= 0 {
        return Ok(());
    }
    let payees: Vec<EconomicActorId> = household.pool_weights.keys().copied().collect();
    let weights: Vec<i64> = household.pool_weights.values().copied().collect();
    let weight_sum: i128 = weights.iter().map(|w| *w as i128).sum();
    if weight_sum <= 0 {
        return Ok(()); // no payout target: leave the fee in the operator (never strand it elsewhere)
    }

    // LEG 1: operator → HOUSEHOLD_SECTOR (the operator holds exactly `amount`).
    accounts.transfer(TRANSPORT_OPERATOR, HOUSEHOLD_SECTOR, amount)?;
    // LEG 2: HOUSEHOLD_SECTOR → households (sum-preserving).
    let splits = apportion_cash(&weights, amount.0);
    for (idx, actor) in payees.iter().enumerate() {
        let share = Money(splits[idx]);
        if share.0 <= 0 {
            continue;
        }
        let pool = demand
            .0
            .get_mut(actor)
            .expect("pool_weights key must reference a seeded demand pool");
        accounts.transfer(HOUSEHOLD_SECTOR, *actor, share)?;
        pool.income_last_tick = pool.income_last_tick.checked_add(share)?;
    }
    ledger.0.push(EconomyEvent::TransportRebate { amount });

    debug_assert_eq!(
        accounts.account(HOUSEHOLD_SECTOR).available,
        Money::ZERO,
        "HOUSEHOLD_SECTOR must net to zero after transport rebate (sentinel-stranded cash)"
    );
    Ok(())
}
