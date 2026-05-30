# Economy — Transport Cost v0 Design

Date: 2026-05-30

## Status

Next economy roadmap slice (deferred slice 2 of economy-v0): "market comparisons
and pool prices incorporate existing routing route costs **without creating
moving traders yet**." Backend-only, deterministic. A small, tested **transport
cost primitive** — groundwork the trader-agent slice (3) consumes.

## Goal

A deterministic integer function that prices moving a quantity of a good between
two market sites, using the routing graph's node geometry. No floats in the cost
path (cross-platform determinism — a float `sqrt`/route-length would drift, the
exact class of bug we hit before). Exposed as a `pub` primitive + a config rate.

## Architecture

### Config
`EconomyConfig` (economy/systems.rs) gains `transport_cost_per_tile_unit: Money`
(cost per tile of distance, per 1.0 unit of good; fixed-point ×`ECONOMY_SCALE`).
Default small, e.g. `Money(5)` (0.005 / tile / unit). `EconomyConfig` stays
`Copy` (`Money` is `Copy`).

### New module `economy/transport.rs`
```rust
/// Integer Manhattan distance in whole tiles between two graph nodes.
/// Positions are rounded to integer tiles FIRST, then subtracted as integers —
/// no float subtraction or sqrt, so the result is identical on every platform.
pub fn manhattan_tiles(graph: &Graph, from: NodeId, to: NodeId) -> i64 {
    let a = graph.node(from).position;
    let b = graph.node(to).position;
    let ax = a.0.round() as i64; let ay = a.1.round() as i64;
    let bx = b.0.round() as i64; let by = b.1.round() as i64;
    (ax - bx).abs() + (ay - by).abs()
}

/// Cost to move `qty` of a good across `distance_tiles` at `rate` (Money per
/// tile per unit). `cost = checked_order_value(rate, qty) * distance_tiles`,
/// all checked i128 — reuses the v0 fixed-point order-value helper so the
/// scaling matches the rest of the economy.
pub fn transport_cost(distance_tiles: i64, qty: Quantity, rate: Money) -> Result<Money, EconomyError>;

/// Convenience: cost between two markets for `qty`.
pub fn transport_cost_between(graph, from: NodeId, to: NodeId, qty, rate) -> Result<Money, EconomyError>;
```

`transport_cost`: `let per_tile = checked_order_value(rate, qty)?; let raw =
(per_tile.0 as i128).checked_mul(distance_tiles as i128).ok_or(Overflow)?;
i64::try_from(raw).map(Money).map_err(|_| Overflow)`. Reject `distance_tiles < 0`
(`InvalidOrder`) — distances are non-negative.

No system, no schedule change, no new resource in v0 — this is a pure helper +
config field. (The trader slice wires it into delivery decisions.)

## Determinism / invariants
- Integer-only cost path (rounded positions, integer subtraction, checked i128).
  No `sqrt`, no float arithmetic on the cost → identical on every platform.
- Same node ⇒ distance 0 ⇒ cost 0. Cost scales linearly in distance and qty.
- Overflow returns `EconomyError::Overflow`, never panics.

## Testing
- `manhattan_tiles_is_integer_and_symmetric` — distance(a,b) == distance(b,a); same node = 0; a hand-computed pair (build a 2-node graph, assert the tile count).
- `transport_cost_scales_with_distance_and_qty` — e.g. rate `Money(1_000)` (1.0/tile/unit), qty `Quantity(1_000)` (1 unit), dist 10 ⇒ `Money(10_000)` (10.0); double the distance ⇒ double the cost; double qty ⇒ double cost.
- `transport_cost_zero_distance_is_zero`.
- `transport_cost_overflow_returns_error` (huge dist × qty × rate).
- `transport_cost_rejects_negative_distance`.
- `transport_cost_between_uses_graph_node_positions` (2-node graph, assert against the manhattan distance).
- Full gate green; economy-v0 + production-v0 tests unaffected.

## What this is NOT
- No path-cost via HPA*/flow-field (a later refinement; v0 uses straight tile
  Manhattan distance as the cost basis — documented). No moving traders, no
  price adjustment wired into pools yet (the consumer is the trader slice).

## Open questions (resolve in planning)
1. Confirm `Graph::node(NodeId) -> &Node` + `Node.position: (f32,f32)` (verified).
2. Default `transport_cost_per_tile_unit` value — pick a small `Money(5)`,
   documented as tunable.
3. `EconomyError::InvalidOrder` exists for the negative-distance guard (confirm;
   else reuse `Overflow`/add a variant).
