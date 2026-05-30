# Economy Transport Cost v0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. TDD, commit per task.

**Goal:** Deterministic integer transport-cost primitive for `sim_core::economy`, per `docs/superpowers/specs/2026-05-30-economy-transport-v0-design.md`.

**Branch/isolation:** worktree `/Users/ramonfuglister/Coding/abutown-transport` on `plan/economy-transport-v0` (from `origin/main` 6a7efb8). `export CARGO_TARGET_DIR=/tmp/abutown-transport-target`. cargo via `scripts/cargo-serial.sh`, one at a time, `pgrep -f cargo` first. `fmt --check` each task.

## Grounding (verified)
- `crate::routing::Graph::node(NodeId) -> &Node`; `Node.position: (f32, f32)`; `crate::routing::NodeId`.
- `EconomyConfig` (economy/systems.rs:21) `#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]` with `ewma_alpha_bps: u16`, `default_order_ttl_ticks: u64` + a `Default` impl.
- `EconomyError` has `Overflow`, `InvalidOrder`, etc. `checked_order_value(price: Money, qty: Quantity) -> Result<Money, EconomyError>` exists in money.rs (`(price·qty)/ECONOMY_SCALE`, checked i128).
- `Money`/`Quantity` are `Copy` fixed-point i64 (×`ECONOMY_SCALE`=1000).

---

### Task 1: `transport.rs` cost primitive

**Files:** Create `backend/crates/sim-core/src/economy/transport.rs`; Modify `economy/mod.rs` (`pub mod transport; pub use transport::*;`); Create `economy/tests/transport.rs`; Modify `economy/tests/mod.rs` (`mod transport;`).

- [ ] **Step 1: failing tests** — `tests/transport.rs`:
```rust
use crate::economy::{
    manhattan_tiles, transport_cost, transport_cost_between, EconomyError, Money, Quantity,
};
use crate::routing::{Edge, EdgeId, EdgeKind, Graph, Node, NodeId, NodeKind};

fn two_node_graph(ax: f32, ay: f32, bx: f32, by: f32) -> Graph {
    let nodes = vec![
        Node { id: NodeId(0), position: (ax, ay), kind: NodeKind::Intersection, legacy_id: None },
        Node { id: NodeId(1), position: (bx, by), kind: NodeKind::Intersection, legacy_id: None },
    ];
    // one footway edge so the graph is well-formed (length unused by Manhattan cost)
    let edges = vec![Edge {
        id: EdgeId(0), from: NodeId(0), to: NodeId(1), kind: EdgeKind::Footway,
        length: 1.0, polyline: vec![(ax, ay), (bx, by)], legacy_id: None,
    }];
    Graph::new(nodes, edges)
}

#[test]
fn manhattan_tiles_is_integer_and_symmetric() {
    let g = two_node_graph(106.0, 64.51, 117.0, 64.51);
    assert_eq!(manhattan_tiles(&g, NodeId(0), NodeId(1)), 11); // |117-106| + |65-65| (64.51 rounds to 65 both)
    assert_eq!(manhattan_tiles(&g, NodeId(1), NodeId(0)), 11);
    assert_eq!(manhattan_tiles(&g, NodeId(0), NodeId(0)), 0);
}

#[test]
fn transport_cost_scales_with_distance_and_qty() {
    // rate 1.0/tile/unit, qty 1 unit, dist 10 -> 10.0
    assert_eq!(transport_cost(10, Quantity(1_000), Money(1_000)), Ok(Money(10_000)));
    assert_eq!(transport_cost(20, Quantity(1_000), Money(1_000)), Ok(Money(20_000)));
    assert_eq!(transport_cost(10, Quantity(2_000), Money(1_000)), Ok(Money(20_000)));
}

#[test]
fn transport_cost_zero_distance_is_zero() {
    assert_eq!(transport_cost(0, Quantity(5_000), Money(1_000)), Ok(Money(0)));
}

#[test]
fn transport_cost_rejects_negative_distance() {
    assert_eq!(transport_cost(-1, Quantity(1_000), Money(1_000)), Err(EconomyError::InvalidOrder));
}

#[test]
fn transport_cost_overflow_returns_error() {
    assert_eq!(transport_cost(i64::MAX, Quantity(i64::MAX), Money(i64::MAX)), Err(EconomyError::Overflow));
}

#[test]
fn transport_cost_between_uses_graph_node_positions() {
    let g = two_node_graph(0.0, 0.0, 3.0, 4.0);
    // manhattan = 3 + 4 = 7; rate 1.0/tile/unit, qty 1 unit -> 7.0
    assert_eq!(transport_cost_between(&g, NodeId(0), NodeId(1), Quantity(1_000), Money(1_000)), Ok(Money(7_000)));
}
```
(Confirm the exact `Node`/`Edge` field set + `Graph::new` signature against routing/graph.rs before finalizing the helper-graph builder — mirror how `mobility_lod_lifecycle.rs` or routing tests construct a `Graph`. Adjust `NodeKind`/`EdgeKind` variant names to the real enum.)
RUN: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core transport` → FAIL.

- [ ] **Step 2: implement** `transport.rs`:
```rust
use crate::economy::{checked_order_value, EconomyError, Money, Quantity};
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
pub fn transport_cost(
    distance_tiles: i64,
    qty: Quantity,
    rate: Money,
) -> Result<Money, EconomyError> {
    if distance_tiles < 0 {
        return Err(EconomyError::InvalidOrder);
    }
    let per_tile = checked_order_value(rate, qty)?;
    let raw = (per_tile.0 as i128)
        .checked_mul(distance_tiles as i128)
        .ok_or(EconomyError::Overflow)?;
    i64::try_from(raw).map(Money).map_err(|_| EconomyError::Overflow)
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
```
Note: `checked_order_value` rejects `rate.0 == 0` (ZeroPrice) and `rate.0 < 0` — so a zero/negative rate errors; that's acceptable (rate must be positive). If a zero rate should be allowed (free transport), the test uses `Money(1_000)`; document that rate must be > 0 in v0.
Add `pub mod transport; pub use transport::*;` to `economy/mod.rs`; `mod transport;` to `tests/mod.rs`.

- [ ] **Step 3: RUN** → PASS. clippy `-p sim-core --all-targets -D warnings`, fmt --check clean.
- [ ] **Step 4: commit** `feat(economy): deterministic transport-cost primitive` (+ Co-Authored-By trailer).

---

### Task 2: config rate field

**Files:** Modify `economy/systems.rs` (`EconomyConfig`).

- [ ] **Step 1:** add `pub transport_cost_per_tile_unit: Money,` to `EconomyConfig`; in its `Default`, set `transport_cost_per_tile_unit: Money(5)`. Import `Money` in systems.rs if not already. (Add a brief test in `tests/systems.rs`: `EconomyConfig::default().transport_cost_per_tile_unit == Money(5)`.) RUN the test (RED then GREEN). Confirm `EconomyConfig` still `Copy` (Money is Copy).
- [ ] **Step 2:** Verify: `cargo test -p sim-core economy` all green; clippy/fmt clean.
- [ ] **Step 3: commit** `feat(economy): add transport_cost_per_tile_unit to EconomyConfig`.

---

### Task 3: Final gate
- [ ] `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check`
- [ ] `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings`
- [ ] `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace`
- [ ] `scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server`
- [ ] (orchestrator) PR → CI green → merge → cleanup.

## Deferred
Path-cost via HPA*/flow-field (vs Manhattan); wiring transport cost into trader delivery decisions (slice 3).
