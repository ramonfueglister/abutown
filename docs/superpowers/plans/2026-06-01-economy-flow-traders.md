# Economy Slice 2 — Visible Flow-Traders Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the off-screen macro flow (#69) as visible "transit traders" — each `MacroFlow{from,to,good,qty}` edge becomes a `TraderAgent` walking the footway route from→to, shown only on the observed portion. Pure read-side projection: no economic effect, no wire/protobuf change, conservation-trivial, ephemeral.

**Architecture:** A new render-only `FlowShipments` resource is captured inside `run_macro_flow_at_tick` (Ok-arm, `src != dst`). The existing `materialize_traders_system` (#64/#66) is extended — via a *parameterized* render-actor lifecycle (not a `Trader`-shaped one) — to also materialize one `TraderAgent` per active shipment at linear travel progress, reusing the ghost-free Spawn/Update/Despawn machine + `trader:` sprite path (no wire change). Shipments expire on arrival; nothing is persisted. The economic `Trader`/auction is untouched.

**Tech Stack:** Rust, `bevy_ecs`, fixed-point i64 (`ECONOMY_SCALE=1000`), BTreeMap determinism. Implements `docs/superpowers/specs/2026-06-01-economy-flow-traders-design.md`. **Backend-only sim-core changes + one seed tweak + one new browser-smoke; crosses the frontend↔backend boundary → browser-smoke is MANDATORY (CLAUDE.md).** Cargo MUST route through the isolated template; **benches/long runs are out of scope here.**

**Cargo command template (every run/test step):**
```
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core <filter>
```
All paths relative to `/Users/ramonfuglister/Coding/abutown-vtraders`. Run `cargo fmt --manifest-path backend/Cargo.toml --all` before EVERY commit (CI fmt-check is strict). One cargo at a time; never bare `cargo` / `--workspace` during iteration.

---

## File Structure

| File | C/M/D | Responsibility |
| --- | --- | --- |
| `backend/crates/sim-core/src/economy/flow_shipments.rs` | **C** | `FlowShipment`, `FlowShipments`, `NextShipmentId` resources; `capture_shipment` + `expire_arrived` pure helpers. |
| `backend/crates/sim-core/src/economy/mod.rs` | M | `pub mod flow_shipments; pub use flow_shipments::*;`; register the two resources in `EconomyPlugin::install` (ephemeral, like `MaterializedTraders`). |
| `backend/crates/sim-core/src/economy/macro_flow.rs` | M | `run_macro_flow_at_tick` gains `&mut FlowShipments` + `&mut NextShipmentId`; capture a shipment in the `Ok(event)` arm when `flow.src != flow.dst`. |
| `backend/crates/sim-core/src/economy/systems.rs` | M | `run_macro_flow_system` threads the two new `ResMut`s into `run_macro_flow_at_tick`. |
| `backend/crates/sim-core/src/economy/materialize.rs` | M | Parameterize `plan_mutations` to a generic render-actor list; materialize shipment-traders alongside demo-traders; expire arrived shipments. |
| `backend/crates/sim-core/src/economy/seed.rs` | M | Add a far-apart dormant market pair (≥2 chunks from a transit chunk, shared grass row, avoiding pinned chunk (3,2)) with a standing imbalance → recurring `MacroFlow`. |
| `backend/crates/sim-core/src/economy/tests/flow_shipments.rs` | **C** | Capture/expiry/travel/conservation/determinism/not-persisted unit tests. |
| `backend/crates/sim-core/src/economy/tests/materialize.rs` | M | Extend the lifecycle + `materialize_does_not_touch_money_or_goods` tests to cover shipment-traders. |
| `backend/crates/sim-core/src/economy/tests/mod.rs` | M | `mod flow_shipments;`. |
| `scripts/smoke-flow-traders.mjs` | **C** | Browser-smoke: zoom IN + pan to the transit chunk (markets dormant), assert a `trader:` agent appears there + moves. |

**Reserved actor-id namespace:** shipment-trader `EconomicActorId = SHIPMENT_ACTOR_OFFSET + shipment_id`, with `SHIPMENT_ACTOR_OFFSET = 1 << 32` (exceeds all seedable ids: seeded ids 8001-8012, demo trader 8003). The offset reconstruction must be identical wherever the `trader:{actor.0}` id/sprite is built (Spawn) and removed (Despawn).

---

## Task 1: `FlowShipment` / `FlowShipments` / `NextShipmentId` resources

**Files:** Create `backend/crates/sim-core/src/economy/flow_shipments.rs`; Modify `economy/mod.rs`, `economy/tests/mod.rs`; Create `economy/tests/flow_shipments.rs`.

- [ ] **1.1** Create `backend/crates/sim-core/src/economy/flow_shipments.rs`:
```rust
//! Render-only projection of the macro flow (#69): each accepted cross-market
//! `MacroFlow` edge becomes an in-transit `FlowShipment` that the materialize
//! system draws as a walking `TraderAgent`. Pure view — NO economic state, NOT
//! persisted (ephemeral, regenerated from the resumed flow on restart).

use bevy_ecs::prelude::*;
use std::collections::BTreeMap;

use crate::economy::{GoodId, MarketId, Quantity};

/// Reserved actor-id offset for shipment-traders so they never collide with
/// seeded economic actors (8001-8012) or the demo trader (8003).
pub const SHIPMENT_ACTOR_OFFSET: u64 = 1 << 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowShipment {
    pub id: u64,
    pub from_market: MarketId,
    pub to_market: MarketId,
    pub good: GoodId,
    pub qty: Quantity,
    pub start_tick: u64,
    pub travel_ticks: u64,
}

impl FlowShipment {
    /// Linear travel progress in [0,1] at `tick` (>= start_tick).
    pub fn progress(&self, tick: u64) -> f32 {
        let elapsed = tick.saturating_sub(self.start_tick);
        (elapsed as f32 / self.travel_ticks.max(1) as f32).clamp(0.0, 1.0)
    }
    /// True once the shipment has reached its destination.
    pub fn arrived(&self, tick: u64) -> bool {
        tick.saturating_sub(self.start_tick) >= self.travel_ticks
    }
}

/// Active in-transit shipments, keyed by id (deterministic counter).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct FlowShipments(pub BTreeMap<u64, FlowShipment>);

/// Monotone shipment-id counter. EPHEMERAL — NOT persisted (resets to 0 on
/// restore alongside the empty `FlowShipments`), unlike the persisted `NextOrderId`.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NextShipmentId(pub u64);

impl NextShipmentId {
    pub fn next(&mut self) -> u64 {
        let id = self.0;
        self.0 += 1;
        id
    }
}
```

- [ ] **1.2** Wire the module + tests. In `economy/mod.rs` add (alphabetically with the other `pub mod`s) `pub mod flow_shipments;` and `pub use flow_shipments::*;`. In `economy/tests/mod.rs` add `mod flow_shipments;`. Create `economy/tests/flow_shipments.rs` with the first failing test:
```rust
use crate::economy::{FlowShipment, FlowShipments, GoodId, MarketId, NextShipmentId, Quantity};

#[test]
fn shipment_progress_and_arrival() {
    let s = FlowShipment {
        id: 0, from_market: MarketId(1), to_market: MarketId(2),
        good: GoodId(0), qty: Quantity(10), start_tick: 100, travel_ticks: 10,
    };
    assert_eq!(s.progress(100), 0.0);
    assert_eq!(s.progress(105), 0.5);
    assert_eq!(s.progress(110), 1.0);
    assert!(!s.arrived(109));
    assert!(s.arrived(110));
    let mut n = NextShipmentId::default();
    assert_eq!(n.next(), 0);
    assert_eq!(n.next(), 1);
    assert_eq!(FlowShipments::default().0.len(), 0);
}
```

- [ ] **1.3** Run — expect FAIL to compile (module/types not found):
```
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests::flow_shipments::shipment_progress_and_arrival
```

- [ ] **1.4** Register resources in `EconomyPlugin::install` (`economy/mod.rs`), next to `MaterializedTraders::default()`:
```rust
world.insert_resource(crate::economy::flow_shipments::FlowShipments::default());
world.insert_resource(crate::economy::flow_shipments::NextShipmentId::default());
```

- [ ] **1.5** Run the test — expect PASS. Then `cargo fmt --manifest-path backend/Cargo.toml --all` and commit:
```
git add -A && git commit -m "feat(economy): FlowShipments + NextShipmentId render-only resources

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Capture shipments in `run_macro_flow_at_tick`

Per spec §3 (corrected): capture is INSIDE `run_macro_flow_at_tick`'s `Ok(event)` arm where `flow.src != flow.dst` — the system never sees individual events. `travel_ticks` is derived from the baked `MarketDistances` and the demo trader's tile-per-tick speed so flow-traders walk at the same visible pace.

**Files:** Modify `economy/macro_flow.rs`, `economy/systems.rs`; Test in `economy/tests/flow_shipments.rs`.

- [ ] **2.1** Add a failing test (drives the new `run_macro_flow_at_tick` signature + capture). It builds a 2-dormant-market scenario with a profitable cross-edge, runs one flow interval, asserts exactly one shipment with the right fields, and that a self-edge / no-flow produces none:
```rust
use crate::economy::macro_flow::run_macro_flow_at_tick;
use crate::economy::{
    AccountBook, DemandPool, DemandPools, DirtyMarketGoods, EconomicActorId, EconomyConfig,
    InventoryBook, MarketDistances, MarketGoodKey, MarketGoodState, MarketGoods, Money,
    SupplyPool, SupplyPools, TradeLedger,
};
use std::collections::BTreeSet;

#[test]
fn macro_flow_captures_one_shipment_per_cross_edge() {
    let a = MarketId(1);
    let b = MarketId(2);
    let good = GoodId(0);
    let seller = EconomicActorId(10);
    let buyer = EconomicActorId(20);

    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    accounts.deposit(buyer, Money(1_000_000)).unwrap();
    inventory.deposit(seller, good, Quantity(1_000)).unwrap();

    let mut supply = SupplyPools::default();
    supply.0.insert(seller, SupplyPool {
        actor: seller, market: a, good, offered_qty_per_tick: Quantity(100),
        min_price: Money(500), interval_ticks: 1, last_generated_tick: None,
    });
    let mut demand = DemandPools::default();
    demand.0.insert(buyer, DemandPool {
        actor: buyer, market: b, good, desired_qty_per_tick: Quantity(100),
        max_price: Money(2_000), urgency_bps: 0, elasticity_bps: 0,
        interval_ticks: 1, last_generated_tick: None,
    });

    let mut mg = MarketGoods::default();
    mg.0.insert(MarketGoodKey { market: a, good }, MarketGoodState::new(MarketGoodKey { market: a, good }));
    mg.0.insert(MarketGoodKey { market: b, good }, MarketGoodState::new(MarketGoodKey { market: b, good }));

    let mut dist = MarketDistances::default();
    dist.0.insert((a, b), 40);
    dist.0.insert((b, a), 40);
    let dormant: BTreeSet<MarketId> = [a, b].into_iter().collect();

    let config = EconomyConfig { transport_cost_per_tile_unit: Money(50), ..Default::default() };
    let dirty = DirtyMarketGoods::default();
    let mut ledger = TradeLedger::default();
    let mut shipments = FlowShipments::default();
    let mut next_id = NextShipmentId::default();

    run_macro_flow_at_tick(
        &mut accounts, &mut inventory, &mut ledger, &demand, &supply, &mut mg,
        &dirty, &dormant, &dist, &config, /*tick=*/ 0,
        &mut shipments, &mut next_id,
    ).unwrap();

    assert_eq!(shipments.0.len(), 1, "one shipment for the A->B cross edge");
    let s = shipments.0.values().next().unwrap();
    assert_eq!((s.from_market, s.to_market, s.good), (a, b, good));
    assert_eq!(s.start_tick, 0);
    assert!(s.travel_ticks > 0);
    assert_eq!(next_id.0, 1);
}
```
(Add `use crate::economy::{FlowShipments, GoodId, NextShipmentId, Quantity};` to the file's imports.)

- [ ] **2.2** Run — expect FAIL to compile (`run_macro_flow_at_tick` arity is wrong).

- [ ] **2.3** Edit `run_macro_flow_at_tick` (`economy/macro_flow.rs:560`): append two params to the signature:
```rust
    shipments: &mut crate::economy::FlowShipments,
    next_shipment_id: &mut crate::economy::NextShipmentId,
```
and in the `Ok(event)` arm of the `for flow in &flows` loop (≈macro_flow.rs:99-104), after `events.push(event);`, capture the shipment for cross-edges only:
```rust
            Ok(event) => {
                next_accounts = na;
                next_inventory = ni;
                if flow.src != flow.dst {
                    let id = next_shipment_id.next();
                    let travel_ticks =
                        crate::economy::flow_shipments::shipment_travel_ticks(flow.dist, config);
                    shipments.0.insert(id, crate::economy::FlowShipment {
                        id,
                        from_market: flow.src,
                        to_market: flow.dst,
                        good: flow.good,
                        qty: crate::economy::Quantity(flow.q),
                        start_tick: current_tick,
                        travel_ticks,
                    });
                }
                events.push(event);
            }
```
(Match the exact local names in the existing Ok arm — `na`/`ni` are illustrative; use whatever the arm already binds. Capture only mutates `shipments`/`next_shipment_id`, never the books, so conservation is untouched.)

- [ ] **2.4** Add `shipment_travel_ticks` to `flow_shipments.rs` (derives ticks from distance at the demo trader's speed, so flow-traders pace like the #64 trader). Verify `EconomyConfig.trader_tiles_per_tick` is the field name (systems.rs:48); use it:
```rust
use crate::economy::EconomyConfig;

/// Visible travel time for a shipment over `dist` tiles, at the same tile/tick
/// speed the demo trader walks (so flow-traders pace identically). >= 1.
pub fn shipment_travel_ticks(dist: i64, config: &EconomyConfig) -> u64 {
    let speed = config.trader_tiles_per_tick.max(1);
    (dist.max(0) as u64).div_ceil(speed as u64).max(1)
}
```

- [ ] **2.5** Thread the new params through `run_macro_flow_system` (`economy/systems.rs:228`): add `mut shipments: ResMut<FlowShipments>, mut next_shipment_id: ResMut<NextShipmentId>` to its params and pass `&mut shipments, &mut next_shipment_id` into `run_macro_flow_at_tick`. Add `FlowShipments, NextShipmentId` to the systems.rs economy import group. **Fix every other caller of `run_macro_flow_at_tick`** (the `economy::tests::macro_flow` tests pass it directly): update them to pass `&mut FlowShipments::default(), &mut NextShipmentId::default()` (they don't assert on shipments).

- [ ] **2.6** Run the new test + the existing macro-flow suite — expect PASS:
```
... scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests
```
Then `fmt` + commit: `feat(economy): capture FlowShipments in run_macro_flow_at_tick (cross-edges only)`.

---

## Task 3: Shipment expiry helper

**Files:** Modify `economy/flow_shipments.rs`; Test in `economy/tests/flow_shipments.rs`.

- [ ] **3.1** Failing test:
```rust
#[test]
fn expire_arrived_drops_only_arrived() {
    let mut s = FlowShipments::default();
    s.0.insert(0, FlowShipment { id: 0, from_market: MarketId(1), to_market: MarketId(2), good: GoodId(0), qty: Quantity(1), start_tick: 0, travel_ticks: 10 });
    s.0.insert(1, FlowShipment { id: 1, from_market: MarketId(1), to_market: MarketId(2), good: GoodId(0), qty: Quantity(1), start_tick: 5, travel_ticks: 10 });
    crate::economy::flow_shipments::expire_arrived(&mut s, /*tick=*/ 12);
    assert_eq!(s.0.keys().copied().collect::<Vec<_>>(), vec![1], "shipment 0 arrived (0+10<=12), 1 not (5+10>12)");
}
```

- [ ] **3.2** Run — expect FAIL (no `expire_arrived`).

- [ ] **3.3** Add to `flow_shipments.rs`:
```rust
/// Drop shipments that have reached their destination by `tick`. Deterministic
/// (BTreeMap retain). Called once per tick by the materialize system.
pub fn expire_arrived(shipments: &mut FlowShipments, tick: u64) {
    shipments.0.retain(|_, s| !s.arrived(tick));
}
```

- [ ] **3.4** Run — expect PASS. `fmt` + commit: `feat(economy): expire arrived flow shipments`.

---

## Task 4: Parameterize `plan_mutations` to a generic render-actor list

Per spec §4 (M1): `plan_mutations` currently iterates `&Traders` and derives progress from `trader_travel`/`leg_progress`, but positions via `world_coord_at_progress_slice(polyline, t)` + a routes map. Refactor so the Spawn/Update/Despawn machine (the #66 ghost-free logic) operates on a generic list of render-actors `{ actor_id, polyline, progress }`, and the demo-`Trader` path becomes one producer of that list. This keeps the lifecycle identical while letting shipments feed it.

**Files:** Modify `economy/materialize.rs`; Test extends `economy/tests/materialize.rs`.

- [ ] **4.1** Add a `RenderActor` input struct + a `plan_render_mutations` that contains the existing Spawn/Update/Update-leaving/Despawn match (lifted verbatim from `plan_mutations`, materialize.rs:142-202), keyed off `actor_id`/`polyline`/`progress` + the `materialized`/`observed` state. Keep `plan_mutations(&Traders, …)` as a thin adapter that builds `Vec<RenderActor>` from traders (computing `progress` via the existing `trader_travel`+`leg_progress`) and calls `plan_render_mutations`, so its existing callers + tests are unchanged. Code (place near `plan_mutations`):
```rust
pub(crate) struct RenderActor<'a> {
    pub actor: EconomicActorId,
    pub polyline: &'a [(f32, f32)],
    pub progress: f32, // [0,1]
}

/// Ghost-free Spawn/Update/Despawn lifecycle (the #66 logic), generic over the
/// source of (actor, polyline, progress). Demo traders and flow shipments both
/// feed this.
pub(crate) fn plan_render_mutations(
    actors: &[RenderActor<'_>],
    materialized: &MaterializedTraders,
    observed: &BTreeSet<ChunkCoord>,
) -> Vec<TraderMutation> {
    let mut muts = Vec::new();
    let mut live: BTreeSet<EconomicActorId> = BTreeSet::new();
    for ra in actors {
        live.insert(ra.actor);
        // <lift materialize.rs:142-193 verbatim, using ra.polyline / ra.progress / ra.actor
        //  in place of `polyline` / `t` / `*actor`; keep the (observed_now, was_observed)
        //  match arms byte-identical so the ghost-free dirty-then-despawn is preserved>
    }
    // Despawn any materialized actor no longer in `actors`.
    for actor in materialized.0.keys() {
        if !live.contains(actor) {
            muts.push(TraderMutation::Despawn { actor: *actor });
        }
    }
    muts
}
```

- [ ] **4.2** Rewrite `plan_mutations` as the adapter (preserving its signature + behavior):
```rust
pub(crate) fn plan_mutations(
    traders: &Traders,
    config: &EconomyConfig,
    materialized: &MaterializedTraders,
    routes: &BTreeMap<EconomicActorId, Vec<(f32, f32)>>,
    observed: &BTreeSet<ChunkCoord>,
) -> Vec<TraderMutation> {
    let actors: Vec<RenderActor<'_>> = traders.0.iter().filter_map(|(actor, trader)| {
        let polyline = routes.get(actor)?;
        if polyline.is_empty() { return None; }
        let progress = leg_progress(&trader.state, trader_travel(trader, config));
        Some(RenderActor { actor: *actor, polyline, progress })
    }).collect();
    plan_render_mutations(&actors, materialized, observed)
}
```
(Confirm the no-route despawn semantics from the original — an actor with no route but previously materialized must still be despawned; preserve that by including it among the `materialized.0.keys()` despawn sweep, which already happens since it won't be in `actors`.)

- [ ] **4.3** Run the existing materialize suite — expect PASS unchanged (pure refactor):
```
... scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests::materialize
```

- [ ] **4.4** `fmt` + commit: `refactor(economy): parameterize trader render lifecycle (plan_render_mutations)`.

---

## Task 5: Materialize shipment-traders

Per spec §4: in `materialize_traders_system`, build `RenderActor`s from `FlowShipments` (route via the existing `leg_polyline`, progress = `shipment.progress(tick)`, actor = `SHIPMENT_ACTOR_OFFSET + id`), combine with the demo-trader render-actors, run `plan_render_mutations` once over both, then `expire_arrived`.

**Files:** Modify `economy/materialize.rs`; Test extends `economy/tests/materialize.rs`.

- [ ] **5.1** Failing integration test (full plugin install + a dormant cross-flow whose route crosses an observed chunk → a `trader:` shipment agent appears, then despawns on arrival). Mirror the existing `tests/materialize.rs` world-construction idiom (install EconomyPlugin + routing + set Tick); assert a `TraderAgent` with a `trader:` sprite at the shipment's progressed position exists while observed and is gone after arrival. (Use the existing test's helper for building the routed world + observed chunks; assert via `MaterializedTraders` containing the `SHIPMENT_ACTOR_OFFSET + 0` actor mid-flight and not after arrival.)

- [ ] **5.2** Run — expect FAIL (shipments not yet materialized).

- [ ] **5.3** Extend `materialize_traders_system` (materialize.rs:303). The system already: early-returns if `Graph`/`HpaIndex`/`FlowFieldCache` are absent (keep that); reads `tick` + builds `observed`; builds demo-trader routes inside a `world.resource_scope(|world, mut cache: Mut<FlowFieldCache>| { … })` via `leg_polyline(graph, hpa, &mut cache, a, b)`; then calls `plan_mutations(...)` + `apply_mutations`. Change the resource_scope to build a **combined render-actor list** (demo traders + shipments) and call `plan_render_mutations`, then expire arrived shipments. The whole shipment read happens inside the same `resource_scope` (graph/hpa/cache/markets all borrowed there), so it stays one borrow-clean exclusive `fn(&mut World)`:
```rust
    // (unchanged early-return + `tick` + `observed` above)

    // Build owned render inputs (actor, polyline, progress) for demo traders AND
    // flow shipments inside one cache scope, then plan + apply.
    let render_inputs: Vec<(EconomicActorId, Vec<(f32, f32)>, f32)> =
        world.resource_scope(|world: &mut World, mut cache: Mut<FlowFieldCache>| {
            let graph = world.resource::<Graph>();
            let hpa = world.resource::<HpaIndex>();
            let markets = world.resource::<Markets>();
            let config = world.resource::<EconomyConfig>();
            let mut out: Vec<(EconomicActorId, Vec<(f32, f32)>, f32)> = Vec::new();
            // demo traders (existing endpoints/outbound logic)
            for (actor, trader) in &world.resource::<Traders>().0 {
                let Some((src, dst)) = endpoints(markets, trader) else { continue };
                let (a, b) = if is_outbound(&trader.state) { (src, dst) } else { (dst, src) };
                if let Some(poly) = leg_polyline(graph, hpa, &mut cache, a, b) {
                    let progress = leg_progress(&trader.state, trader_travel(trader, config));
                    out.push((*actor, poly, progress));
                }
            }
            // flow shipments (NEW): route from->to, linear progress, reserved actor id
            for s in world.resource::<crate::economy::FlowShipments>().0.values() {
                let (Some(from), Some(to)) =
                    (markets.0.get(&s.from_market), markets.0.get(&s.to_market)) else { continue };
                if let Some(poly) = leg_polyline(graph, hpa, &mut cache, from.node_id, to.node_id) {
                    out.push((
                        EconomicActorId(crate::economy::flow_shipments::SHIPMENT_ACTOR_OFFSET + s.id),
                        poly,
                        s.progress(tick),
                    ));
                }
            }
            out
        });

    let muts = {
        let materialized = world.resource::<MaterializedTraders>();
        let actors: Vec<RenderActor<'_>> = render_inputs
            .iter()
            .map(|(actor, poly, progress)| RenderActor { actor: *actor, polyline: poly, progress: *progress })
            .collect();
        plan_render_mutations(&actors, materialized, &observed)
    };
    apply_mutations(world, tick, muts);

    // Drop shipments that have arrived this tick (after their final position rendered).
    expire_arrived(&mut world.resource_mut::<crate::economy::FlowShipments>(), tick);
```
(`endpoints`, `is_outbound`, `leg_progress`, `trader_travel`, `leg_polyline` are the existing helpers; `RenderActor`/`plan_render_mutations` from Task 4. Resolve the exact `markets.0.get(...).node_id` accessor against `Market` — the demo path uses `endpoints(markets, trader)`; for a market id you read `markets.0.get(&id)?.node_id`.)

- [ ] **5.4** Run the new test + full materialize suite — expect PASS. Confirm the existing demo-trader tests still green (the combined list must not change their behavior). `fmt` + commit: `feat(economy): materialize flow-shipment transit traders`.

---

## Task 6: Conservation + determinism + not-persisted tests

**Files:** Modify `economy/tests/materialize.rs` + `economy/tests/flow_shipments.rs`.

- [ ] **6.1** Extend `tests/materialize.rs:170 materialize_does_not_touch_money_or_goods`: add active `FlowShipments` to the world before running the system; assert `total_money`/`total_good` (and `AccountBook`/`InventoryBook` equality) are unchanged by the shipment-materialize path. Run — expect PASS (capture/render touch no economic resource).

- [ ] **6.2** Add `flow_shipments_capture_is_deterministic` to `tests/flow_shipments.rs`: run the Task-2 scenario twice; assert identical `FlowShipments` maps + `NextShipmentId`. Run — expect PASS.

- [ ] **6.3** Add `flow_shipments_not_persisted`: build a world, insert a shipment, `extract_from_world` → serialize → `apply_into_world` to a fresh world; assert the fresh world's `FlowShipments` is empty and the economy snapshot is byte-identical to one taken without shipments (no new persisted field). Run — expect PASS.

- [ ] **6.4** `fmt` + commit: `test(economy): flow-shipment conservation, determinism, ephemerality`.

---

## Task 7: Seed a demonstrable dormant→dormant flow

Per spec §7: place two markets ≥2 chunks from a chosen transit chunk on a shared grass row, avoiding the pinned chunk (3,2), with a standing imbalance → recurring `MacroFlow` whose straight-line route crosses the transit chunk. The world is 224×128 tiles → 7×4 chunks at chunk_size 32 (chunk cols 0..6, rows 0..3).

**Files:** Modify `economy/seed.rs`; Test extends `economy/tests/seed.rs`.

- [ ] **7.1** Choose geometry on row r=1 (avoids pinned (3,2)): market F_A at tile ≈ (16, 48) [chunk (0,1)], market F_B at tile ≈ (208, 48) [chunk (6,1)]; transit chunk (3,1) (tiles x 96-127, y 32-63), which the straight grass route at y≈48 crosses. Both market chunks are 3 chunks from (3,1) → never pulled Active by a 3×3+ring subscription centered on (3,1). Add to `seed_demo_economy`: two markets (snap to nearest graph node via `NodeSpatialIndex::nearest`), a supplier pool at F_A + consumer pool at F_B for a good (reuse `GOOD_FOOD` or the Slice-1 second good), seeded into `SupplyPools`/`DemandPools` + `MarketChunks` + `MarketDistances` (both directions), and `MarketGoods` states. These markets are NOT pinned and sit outside the default viewport → dormant by default → they flow.

- [ ] **7.2** Failing test in `tests/seed.rs`: after `seed_demo_economy`, assert the two flow-demo markets exist with their supplier/consumer pools and a `MarketDistances` entry; assert their chunks differ from (3,1) and from each other by ≥2 chunks. (Keep the existing `seed_demo_economy_creates_two_markets_and_one_trader` assertions valid — adjust its market/trader counts to the new totals, or assert the ORIGINAL demo markets/trader still present alongside the new pair.)

- [ ] **7.3** Run — expect FAIL, then implement the seed additions, run — expect PASS. `fmt` + commit: `feat(economy): seed a demonstrable dormant cross-market flow`.

---

## Task 8: Browser-smoke `smoke-flow-traders.mjs`

Per spec §7 (CLAUDE.md mandatory): the existing `smoke-visible-traders.mjs` zooms OUT (wrong — would make the markets Active). The new smoke zooms IN + pans to the transit chunk.

**Files:** Create `scripts/smoke-flow-traders.mjs` (adapt `scripts/smoke-visible-traders.mjs`).

- [ ] **8.1** Copy `scripts/smoke-visible-traders.mjs` → `scripts/smoke-flow-traders.mjs`. Replace the zoom-OUT (`wheel` down ×6) with zoom-IN toward `CAMERA_MAX_SCALE` and a pan that centers the viewport on the transit chunk (3,1) (tile ≈ (112, 48)). Assert via the client state that the subscription set covers (3,1)'s neighborhood but NOT the market chunks (0,1)/(6,1) — so the markets stay dormant.

- [ ] **8.2** Assert a `trader:`-prefixed agent appears in the transit chunk and its `world_coord` advances over time (sample over ≥ ~60 ticks to span a shipment's ~`travel_ticks` lifetime; tolerate shipments entering/leaving — assert at least one `trader:` agent is seen moving along the row). Reuse the harness/launch + WS-frame plumbing from the template.

- [ ] **8.3** Run the smoke against the dev stack (the operator/you runs it; it needs the `.env`):
```
node scripts/smoke-flow-traders.mjs
```
Expected: a flow-derived `trader:` agent observed transiting (3,1) with changing `world_coord`. Commit: `test(smoke): flow-trader transit browser smoke (zoom-in to dormant-flow route)`.

---

## Task 9: Final gate + e2e render-smoke unaffected

**Files:** none (verification).

- [ ] **9.1** Full workspace gate (isolated), confirm green:
```
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
... scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
... scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace --all-targets
```

- [ ] **9.2** Confirm the e2e `render-smoke` needs no change: it already filters `!a.id.startsWith('trader:')` for the pinned-300 assertion + uses `count >= 300` (render-smoke.spec.ts:96,99), so flow-traders are auto-excluded; run it to confirm green:
```
PATH=/Users/ramonfuglister/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH npm run test:e2e -- tests/e2e/render-smoke.spec.ts
```

- [ ] **9.3** No commit (verification only). If anything is red, fix in the owning task and re-gate.

---

## Notes for the implementer

- **Pure projection:** never give a shipment-trader economic state. Capture/render/expire must touch only `FlowShipments`/`NextShipmentId`/the ECS render entities — never `AccountBook`/`InventoryBook`/`MarketGoods`. The conservation test (6.1) guards this.
- **Determinism:** BTreeMap/monotone-counter only; no RNG/float beyond the existing arc-length positioning.
- **One cargo at a time**, isolated `TMPDIR`/`CARGO_TARGET_DIR`, `fmt` before every commit. **Never run a criterion bench here** (none in scope).
- **The demonstrability geometry (Task 7) + the zoom-IN smoke (Task 8) are make-or-break** — if no `trader:` agent ever appears, the feature shipped broken (the Phase-7a failure mode). Verify the route actually crosses (3,1) against the real grass-grid Walk path before claiming done.
