# Economy Slice 3 — Visible Shoppers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When an observed market has unmet demand, spawn dedicated ephemeral "shopper" render-agents (count ∝ `unmet_demand_last_tick`, deterministic, capped) that walk from a nearby footway node TO the market and despawn on arrival — the demand-side twin of #70's flow-traders. Pure read-side projection: no economic effect, ephemeral, renders as pedestrians via a `shopper:` sprite-key.

**Architecture:** A render-only `ShopperVisits` resource is filled by `run_shopper_capture_system` (new `EconomySet::ShopperCapture`, after `MacroFlow`, before `Materialize`) from observed markets' `unmet_demand_last_tick`. `materialize_traders_system` (already parameterized via `plan_render_mutations` in #70) also materializes shopper visits, reusing the ghost-free Spawn/Update/Despawn lifecycle. Shoppers use actor id `SHOPPER_ACTOR_OFFSET (2<<32) + visit.id` and a `shopper:` sprite-key (→ client renders `kind:'pedestrian'`, no client render change). Conservation-trivial (reads `unmet_demand` only), ephemeral (not persisted). The economy is untouched.

**Tech Stack:** Rust, `bevy_ecs`, fixed-point i64, BTreeMap determinism. Implements `docs/superpowers/specs/2026-06-01-economy-shoppers-design.md`. Backend sim-core + a 2-line frontend count-exclusion + 1 new browser-smoke; **crosses the frontend↔backend boundary → browser-smoke MANDATORY.** Cargo via the isolated template; **no benches.**

**Cargo template (every run/test):**
```
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core <filter>
```
All paths relative to `/Users/ramonfuglister/Coding/abutown-vtraders`. Run `cargo fmt --manifest-path backend/Cargo.toml --all` before EVERY commit. One cargo at a time; never bare/`--workspace` during iteration.

---

## File Structure

| File | C/M/D | Responsibility |
| --- | --- | --- |
| `backend/crates/sim-core/src/economy/shoppers.rs` | **C** | `ShopperVisit`, `ShopperVisits`, `NextShopperId`, `SHOPPER_ACTOR_OFFSET`; `expire_arrived_shoppers`; `shopper_travel_ticks`. |
| `backend/crates/sim-core/src/economy/mod.rs` | M | `pub mod shoppers; pub use shoppers::*;`; register `ShopperVisits`+`NextShopperId` in `EconomyPlugin::install`. |
| `backend/crates/sim-core/src/economy/systems.rs` | M | `EconomyConfig.shoppers_per_unit` + `max_shoppers_per_market`; `EconomySet::ShopperCapture` (after MacroFlow, before Materialize); `run_shopper_capture_system`. |
| `backend/crates/sim-core/src/economy/materialize.rs` | M | `id_prefix(actor)` helper (Spawn @189 + Despawn @362); materialize shopper visits in the resource_scope; expire arrived shoppers; exclude shopper ids from `rendering_shipment_ids`. |
| `backend/crates/sim-core/src/economy/tests/shoppers.rs` | **C** | capture/sampling/travel/materialize/conservation/determinism/ephemeral unit tests. |
| `backend/crates/sim-core/src/economy/tests/mod.rs` | M | `mod shoppers;`. |
| `src/app/runtimeDiagnostics.ts` | M | exclude `shopper:` ids from the `pedestrians` count (line 147). |
| `tests/e2e/render-smoke.spec.ts` | M | exclude `shopper:` from the pinned-300 agent filter (line 96). |
| `scripts/smoke-shoppers.mjs` | **C** | browser-smoke: zoom out to chunk (0,0), assert a `shopper:` agent walks toward `m_b`. |

---

## Task 1: Shopper resources

**Files:** Create `economy/shoppers.rs`; Modify `economy/mod.rs`, `economy/tests/mod.rs`; Create `economy/tests/shoppers.rs`.

- [ ] **1.1** Create `backend/crates/sim-core/src/economy/shoppers.rs`:
```rust
//! Render-only projection of aggregate DEMAND (twin of flow_shipments.rs): an
//! observed market with unmet demand spawns shopper visits that the materialize
//! system draws as pedestrians walking to the market. PURE VIEW — no economic
//! state, NOT persisted (ephemeral, regenerated from resumed demand on restart).

use bevy_ecs::prelude::*;
use std::collections::BTreeMap;

use crate::economy::{GoodId, MarketId};
use crate::routing::NodeId;

/// Reserved actor-id offset for shopper-agents; distinct from flow-traders' 1<<32.
pub const SHOPPER_ACTOR_OFFSET: u64 = 2 << 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShopperVisit {
    pub id: u64,
    pub market: MarketId,
    pub good: GoodId,
    pub origin_node: NodeId,
    pub start_tick: u64,
    pub travel_ticks: u64,
}

impl ShopperVisit {
    pub fn progress(&self, tick: u64) -> f32 {
        let elapsed = tick.saturating_sub(self.start_tick);
        (elapsed as f32 / self.travel_ticks.max(1) as f32).clamp(0.0, 1.0)
    }
    pub fn arrived(&self, tick: u64) -> bool {
        tick.saturating_sub(self.start_tick) >= self.travel_ticks
    }
}

/// Active shopper visits, keyed by id.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ShopperVisits(pub BTreeMap<u64, ShopperVisit>);

/// Monotone id counter. EPHEMERAL — NOT persisted (resets to 0 on restore).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NextShopperId(pub u64);

impl NextShopperId {
    pub fn next(&mut self) -> u64 {
        let id = self.0;
        self.0 += 1;
        id
    }
}

/// Drop shopper visits that have arrived by `tick` AND whose agent is no longer
/// being rendered (so the ghost-free leave→despawn completes first). `rendering`
/// is the set of shopper ids still materialized.
pub fn expire_arrived_shoppers(
    visits: &mut ShopperVisits,
    tick: u64,
    rendering: &std::collections::BTreeSet<u64>,
) {
    visits.0.retain(|id, v| !v.arrived(tick) || rendering.contains(id));
}
```

- [ ] **1.2** Wire module + tests. In `economy/mod.rs`: `pub mod shoppers;` + `pub use shoppers::*;`. In `economy/tests/mod.rs`: `mod shoppers;`. Create `economy/tests/shoppers.rs` with a first failing test:
```rust
use crate::economy::{GoodId, MarketId, NextShopperId, ShopperVisit, ShopperVisits};
use crate::routing::NodeId;

#[test]
fn shopper_progress_arrival_and_id() {
    let v = ShopperVisit {
        id: 0, market: MarketId(1), good: GoodId(0), origin_node: NodeId(7),
        start_tick: 100, travel_ticks: 10,
    };
    assert_eq!(v.progress(105), 0.5);
    assert!(!v.arrived(109));
    assert!(v.arrived(110));
    let mut n = NextShopperId::default();
    assert_eq!((n.next(), n.next()), (0, 1));
    assert_eq!(ShopperVisits::default().0.len(), 0);
}
```
(Confirm `NodeId`'s constructor/field shape against `routing`; adjust `NodeId(7)` if it's a named field.)

- [ ] **1.3** Run — expect FAIL to compile:
```
... scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests::shoppers::shopper_progress_arrival_and_id
```

- [ ] **1.4** Register in `EconomyPlugin::install` (`economy/mod.rs`), next to the FlowShipments registration:
```rust
world.insert_resource(crate::economy::shoppers::ShopperVisits::default());
world.insert_resource(crate::economy::shoppers::NextShopperId::default());
```

- [ ] **1.5** Run — expect PASS. `fmt` + commit: `feat(economy): ShopperVisits + NextShopperId render-only resources`.

---

## Task 2: EconomyConfig shopper tuning

**Files:** Modify `economy/systems.rs` (the `EconomyConfig` struct + its `Default`); Test in `economy/tests/shoppers.rs`.

- [ ] **2.1** Failing test:
```rust
#[test]
fn economy_config_has_shopper_tuning_defaults() {
    let c = crate::economy::EconomyConfig::default();
    assert!(c.shoppers_per_unit >= 1);
    assert!(c.max_shoppers_per_market >= 1);
    assert!(c.shopper_radius_tiles > 0.0);
}
```

- [ ] **2.2** Run — expect FAIL (fields missing).

- [ ] **2.3** Add the fields to `EconomyConfig` (systems.rs) + its `Default` impl. Use the existing struct-update/Default style in that file (do NOT trip `clippy::field_reassign_with_default`):
```rust
// in struct EconomyConfig:
    /// How many unmet-demand units one visible shopper represents.
    pub shoppers_per_unit: i64,
    /// Cap on simultaneous shoppers rendered per market (keeps it a handful, not hundreds).
    pub max_shoppers_per_market: usize,
    /// Radius (tiles) around a market to pick shopper origin nodes.
    pub shopper_radius_tiles: f32,
// in Default:
            shoppers_per_unit: 3,
            max_shoppers_per_market: 4,
            shopper_radius_tiles: 24.0,
```

- [ ] **2.4** Run — expect PASS. `fmt` + commit: `feat(economy): shopper tuning config (per_unit, max_per_market, radius)`.

---

## Task 3: `id_prefix` helper + shopper-aware id reconstruction

Per spec §4: the Spawn side (`plan_render_mutations` materialize.rs:189-190) and Despawn side (`apply_mutations` materialize.rs:362) both build `trader:{actor.0}`. Route both through one helper so shopper actors (`>= SHOPPER_ACTOR_OFFSET`) get a `shopper:` prefix; and exclude shopper ids from `rendering_shipment_ids`.

**Files:** Modify `economy/materialize.rs`; Test in `economy/tests/shoppers.rs`.

- [ ] **3.1** Failing test:
```rust
#[test]
fn id_prefix_distinguishes_shoppers_from_traders() {
    use crate::economy::materialize::id_prefix;
    use crate::economy::shoppers::SHOPPER_ACTOR_OFFSET;
    use crate::economy::flow_shipments::SHIPMENT_ACTOR_OFFSET;
    use crate::economy::EconomicActorId;
    assert_eq!(id_prefix(EconomicActorId(8003)), "trader:");
    assert_eq!(id_prefix(EconomicActorId(SHIPMENT_ACTOR_OFFSET + 1)), "trader:");
    assert_eq!(id_prefix(EconomicActorId(SHOPPER_ACTOR_OFFSET + 1)), "shopper:");
}
```

- [ ] **3.2** Run — expect FAIL (`id_prefix` missing).

- [ ] **3.3** Add to `materialize.rs`:
```rust
/// Sprite/id prefix for a render-actor by its actor-id namespace. Shopper actors
/// (>= SHOPPER_ACTOR_OFFSET) render as pedestrians (`shopper:`), everything else
/// (demo traders + flow shipments) as `trader:`.
pub(crate) fn id_prefix(actor: EconomicActorId) -> &'static str {
    if actor.0 >= crate::economy::shoppers::SHOPPER_ACTOR_OFFSET {
        "shopper:"
    } else {
        "trader:"
    }
}
```
Make it `pub(crate)` (the test imports it via `crate::economy::materialize::id_prefix`) — confirm the module visibility allows it.

- [ ] **3.4** Replace the Spawn construction (materialize.rs:189-190):
```rust
                let p = id_prefix(*actor);
                let agent_id = AgentId(format!("{p}{}", actor.0));
                let sprite = format!("{p}{}", sprite_hash(&agent_id.0));
```
and the Despawn construction (materialize.rs:362):
```rust
                    .remove(&AgentId(format!("{}{}", id_prefix(*actor), actor.0)));
```
(Confirm `sprite_hash` takes the agent_id string; keep its call identical.)

- [ ] **3.5** Fix `rendering_shipment_ids` (materialize.rs:373-380) to NOT mis-attribute shopper ids as shipments — exclude `>= SHOPPER_ACTOR_OFFSET` before the `checked_sub`:
```rust
        .filter(|a| a.0 < crate::economy::shoppers::SHOPPER_ACTOR_OFFSET)
        .filter_map(|a| a.0.checked_sub(SHIPMENT_ACTOR_OFFSET))
```
(Insert the `.filter` before the existing `.filter_map`.)

- [ ] **3.6** Run the materialize suite + the new test — expect PASS (existing flow-trader/demo tests unchanged since their ids are `< SHOPPER_ACTOR_OFFSET` → still `trader:`):
```
... scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests
```
`fmt` + commit: `refactor(economy): id_prefix helper — shopper-aware trader:/shopper: id reconstruction`.

---

## Task 4: `run_shopper_capture_system`

Per spec §3: a new system in `EconomySet::ShopperCapture` (after MacroFlow, before Materialize) that fills `ShopperVisits` from observed markets' `unmet_demand_last_tick`.

**Files:** Modify `economy/systems.rs`; Test in `economy/tests/shoppers.rs`.

- [ ] **4.1** Failing test (drives the capture logic; uses a pure helper `capture_shopper_visits` so it's testable without a full schedule):
```rust
#[test]
fn capture_spawns_proportional_to_unmet_demand_deterministically() {
    use crate::economy::shoppers::{capture_shopper_visits, ShopperVisits, NextShopperId};
    use crate::economy::{EconomyConfig, MarketGoodKey, MarketGoodState, MarketGoods, MarketId, Markets, GoodId, Quantity};
    use std::collections::BTreeSet;
    // one observed market m=1 with unmet_demand 9, good 0; cap=4, per_unit=3 -> 3 visits
    let m = MarketId(1);
    let good = GoodId(0);
    let key = MarketGoodKey { market: m, good };
    let mut mg = MarketGoods::default();
    let mut st = MarketGoodState::new(key);
    st.unmet_demand_last_tick = Quantity(9);
    mg.0.insert(key, st);
    // markets / spatial origin provided by the test harness fixture (see note)
    let markets = /* build a Markets with m -> node, + a NodeSpatialIndex with >=3 nearby walkable nodes */ unimplemented_fixture();
    let observed: BTreeSet<MarketId> = [m].into_iter().collect();
    let config = EconomyConfig::default();
    let mut visits = ShopperVisits::default();
    let mut next = NextShopperId::default();
    capture_shopper_visits(&mg, &observed, &markets, /*origins fn*/, &config, /*tick*/ 0, &mut visits, &mut next);
    assert_eq!(visits.0.len(), 3);
    // determinism: same inputs -> same visit set
    let mut visits2 = ShopperVisits::default(); let mut next2 = NextShopperId::default();
    capture_shopper_visits(&mg, &observed, &markets, /*origins fn*/, &config, 0, &mut visits2, &mut next2);
    assert_eq!(visits.0.iter().map(|(_,v)| (v.market, v.origin_node)).collect::<Vec<_>>(),
               visits2.0.iter().map(|(_,v)| (v.market, v.origin_node)).collect::<Vec<_>>());
}
```
**Note for the implementer:** factor the capture into a pure `capture_shopper_visits(...)` (no `World`) taking the observed-market set, a market→node map, an origin-candidate provider `impl Fn(NodeId) -> Vec<NodeId>` (sorted, market-node-excluded, walkable — so the spatial index + routability live in the system wrapper, keeping the core pure + testable), `&config`, `tick`, `&mut ShopperVisits`, `&mut NextShopperId`. The test fills a small in-memory market→node map + a fixed origin-provider returning ≥3 sorted node ids; replace the `unimplemented_fixture()`/`/*…*/` sketch with that concrete fixture (mirror how `tests/shoppers.rs`/`tests/materialize.rs` build minimal economy state). Reconciliation: `target = min(unmet/per_unit, cap)`; spawn `target − current_for_market` new visits using the first-N sorted origin candidates.

- [ ] **4.2** Run — expect FAIL (`capture_shopper_visits` missing).

- [ ] **4.3** Implement `capture_shopper_visits` (pure) in `shoppers.rs` per the note: for each observed `(market, good)` in `MarketGoods` with `unmet_demand_last_tick > 0`, compute `target`, count current visits for that market, and for each shortfall slot take the next sorted+walkable origin candidate (skipping the market node), insert a `ShopperVisit` with `travel_ticks = max(1, manhattan_tiles(origin, market_node)/walk_speed)` (reuse the trader walk speed magnitude / `manhattan_tiles`). Deterministic BTree iteration; ids from `NextShopperId`.

- [ ] **4.4** Run — expect PASS.

- [ ] **4.5** Add the system wrapper `run_shopper_capture_system(world: &mut World)` (or a `Res`-based system) in systems.rs: gather the observed-chunk→market set (like `materialize_traders_system` derives observed Active/Hot chunks), build the market→node map from `Markets`, and an origin-provider closure that queries `NodeSpatialIndex::within_radius(market_pos, config.shopper_radius_tiles)`, **sorts the result** (by `NodeId`), drops the market node, and filters to nodes with a Walk route (or defers routability to materialize — simplest: sort + drop-market-node here, let materialize skip routeless). Call `capture_shopper_visits(...)`. Register it in a NEW `EconomySet::ShopperCapture`, ordered `.after(EconomySet::MacroFlow).before(EconomySet::Materialize)` in `install_systems`' `.chain()`/`configure_sets`.

- [ ] **4.6** Add a schedule-ordering test (or extend an existing economy schedule test) asserting `ShopperCapture` runs after `MacroFlow` and before `Materialize`. Run the economy suite — expect PASS. `fmt` + commit: `feat(economy): run_shopper_capture_system (after MacroFlow, before Materialize)`.

---

## Task 5: Materialize shoppers

Per spec §4: add a shopper branch to `materialize_traders_system`'s resource_scope (mirrors the FlowShipments branch at materialize.rs:447), and expire arrived shoppers.

**Files:** Modify `economy/materialize.rs`; Test in `economy/tests/materialize.rs` (extend).

- [ ] **5.1** Failing integration test (mirror the #70 shipment materialize test): a world with an active `ShopperVisit` whose `origin→market` route crosses an observed chunk → a `shopper:`-prefixed `TraderAgent` is materialized at the visit's progress; after arrival it is despawned. Assert via `MaterializedTraders` containing `SHOPPER_ACTOR_OFFSET + 0` mid-flight and absent after arrival. (Reuse the #70 test's routed-world fixture.)

- [ ] **5.2** Run — expect FAIL.

- [ ] **5.3** In `materialize_traders_system`'s `resource_scope` (after the FlowShipments loop at ~447), add the shopper loop:
```rust
            for v in world.resource::<crate::economy::ShopperVisits>().0.values() {
                let Some(market) = markets.0.get(&v.market) else { continue };
                if let Some(poly) = leg_polyline(graph, hpa, &mut cache, v.origin_node, market.node_id) {
                    out.push((
                        EconomicActorId(crate::economy::shoppers::SHOPPER_ACTOR_OFFSET + v.id),
                        poly,
                        v.progress(tick),
                    ));
                }
            }
```
And add a shopper analog of `rendering_shipment_ids` + the expire calls. Add near `rendering_shipment_ids`:
```rust
fn rendering_shopper_ids(materialized: &MaterializedTraders) -> std::collections::BTreeSet<u64> {
    use crate::economy::shoppers::SHOPPER_ACTOR_OFFSET;
    materialized.0.keys().filter_map(|a| a.0.checked_sub(SHOPPER_ACTOR_OFFSET)).collect()
}
```
At BOTH expire sites (materialize.rs ~394 and ~486, where shipments are expired), add the shopper expire:
```rust
    let s_rendering = rendering_shopper_ids(world.resource::<MaterializedTraders>());
    crate::economy::shoppers::expire_arrived_shoppers(
        &mut world.resource_mut::<crate::economy::ShopperVisits>(), tick, &s_rendering);
```

- [ ] **5.4** Run the new test + full materialize suite — expect PASS (demo + flow-trader tests unchanged). `fmt` + commit: `feat(economy): materialize shopper visits as pedestrian render-agents`.

---

## Task 6: Conservation, determinism, ephemerality tests

**Files:** Modify `economy/tests/materialize.rs` + `economy/tests/shoppers.rs`.

- [ ] **6.1** Extend `materialize_does_not_touch_money_or_goods*` (tests/materialize.rs): add active `ShopperVisits` to the world; assert `total_money`/`total_good` + `AccountBook`/`InventoryBook` unchanged by the shopper-materialize path. Run — expect PASS.

- [ ] **6.2** `shopper_capture_is_deterministic` (tests/shoppers.rs): the Task-4 scenario twice → identical `ShopperVisits` + `NextShopperId`. (Covered by 4.1's determinism assert; add a `build()==build()` over a full plugin tick if a schedule-level determinism test exists for the economy.)

- [ ] **6.3** `shoppers_not_persisted`: build a world, insert a `ShopperVisit`, `extract_from_world`→serialize→`apply_into_world` to a fresh world; assert the fresh `ShopperVisits` is empty and the economy snapshot is byte-identical to one without shoppers (no new persisted field). Run — expect PASS. `fmt` + commit: `test(economy): shopper conservation, determinism, ephemerality`.

---

## Task 7: Frontend `shopper:` count-exclusions

Per spec §6: shoppers render as `kind:'pedestrian'`, so they inflate the pedestrian count unless excluded by id prefix (a NEW exclusion vs #64's `trader:`, which is `kind:'trader'`).

**Files:** Modify `src/app/runtimeDiagnostics.ts`, `tests/e2e/render-smoke.spec.ts`.

- [ ] **7.1** In `src/app/runtimeDiagnostics.ts:147`, exclude `shopper:` from the `pedestrians` count:
```ts
      pedestrians: projectedPedestrians.filter((p) => p.kind === 'pedestrian' && !p.id.startsWith('shopper:')).length,
```
(Confirm `projectedPedestrians` items carry an `id` field; if the field is `agent.id`/`p.id`, match it.)

- [ ] **7.2** In `tests/e2e/render-smoke.spec.ts:96`, extend the agent filter:
```ts
    state.city.mobilityAgents.agents.filter((a: { id: string }) => !a.id.startsWith('trader:') && !a.id.startsWith('shopper:')),
```

- [ ] **7.3** Typecheck + vitest (these are the frontend gate; run from repo root):
```
npm run typecheck && npm test
```
Expected: PASS. Commit: `fix(render): exclude shopper: agents from the pinned base-world pedestrian count`.

---

## Task 8: Browser-smoke `smoke-shoppers.mjs`

Per spec §7 (CLAUDE.md mandatory): shoppers appear at the observed FOOD-demand market `m_b` (chunk (0,0)). Mirror `smoke-visible-traders.mjs` (which zooms OUT to subscribe the demo market chunk regardless of default camera).

**Files:** Create `scripts/smoke-shoppers.mjs` (adapt `scripts/smoke-visible-traders.mjs`).

- [ ] **8.1** Copy `scripts/smoke-visible-traders.mjs` → `scripts/smoke-shoppers.mjs`. Keep its zoom-OUT (subscribes the whole world incl. chunk (0,0) where `m_b` lives — observed markets are exactly what shoppers need). Assert via the client state that chunk (0,0) is subscribed.

- [ ] **8.2** Assert: ≥1 `shopper:`-prefixed agent appears and its `world_coord` advances toward the `m_b` node over time (sample over ~12s; tolerate shoppers spawning/arriving). Reuse the harness/WS plumbing from the template.

- [ ] **8.3** Run against the in-memory stack (the operator/you runs it; same DB-free path as #70 — `e2e_server` on :8080 + vite :5175):
```
node scripts/smoke-shoppers.mjs
```
Expected: ≥1 moving `shopper:` agent near `m_b`. Commit: `test(smoke): shopper transit browser smoke (zoom-out to observed demand market)`.

---

## Task 9: Final gate

**Files:** none (verification).

- [ ] **9.1** Full workspace gate (isolated), confirm green to COMPLETION (wait for the explicit done, do not judge mid-run):
```
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
... scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
... scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace --all-targets
```
(Includes **sim-server** tests — a sim-core seed/economy change can break sim-server `runtime::tests`; this is where #70 was caught.)

- [ ] **9.2** Frontend gate + e2e render-smoke (confirm the pinned-300 holds after the `shopper:` exclusion):
```
npm run typecheck && npm test
PATH=/Users/ramonfuglister/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH npm run test:e2e -- tests/e2e/render-smoke.spec.ts
```

- [ ] **9.3** Verification only, no commit. If red, fix in the owning task + re-gate.

---

## Notes for the implementer

- **Pure projection:** the shopper path touches only `ShopperVisits`/`NextShopperId`/render entities + READS `unmet_demand_last_tick` — never `AccountBook`/`InventoryBook`/`MarketGoods` writes. Test 6.1 guards this.
- **Determinism:** `within_radius` returns UNSORTED — SORT before taking the Nth origin, or replays diverge. BTreeMap/monotone-counter elsewhere; no RNG/float beyond arc-length positioning.
- **`shopper:` must be `shopper:<hash>`** so the client `spriteIndexFromKey` parses a valid pedestrian sprite (the Spawn sprite is `format!("{p}{}", sprite_hash(...))` — already hash-suffixed).
- One cargo at a time, isolated `TMPDIR`/`CARGO_TARGET_DIR`, `fmt` before every commit, **no benches**.
- **Make-or-break:** if no `shopper:` agent ever appears in the smoke, the feature shipped broken. Verify the FOOD-at-`m_b` unmet demand → shoppers path actually renders before claiming done.
