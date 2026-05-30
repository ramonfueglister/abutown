use crate::economy::{EconomyError, Money, Quantity, checked_order_value};
use crate::routing::{Graph, NodeId};

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
