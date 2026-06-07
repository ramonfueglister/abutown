# Abutopia Economy Visible (Blocker-1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Re-anchor abutopia market 9002 onto the residential corridor so the 300 pedestrians bind `home_market = 9002` and the attribution shop channel routes them (`routed > 0`), making the demographics↔economy merge visible — proven by deterministic backend tests.

**Architecture:** A single world-data edit (`markets.json` 9002 anchor `[13.0,3.0]` → `[111.5,64.51]`, chunk (3,2)) plus repairs to three economy test-helpers that hard-code 9002's old position, plus two new deterministic tests (binding correctness from the real bundle; end-to-end routed>0). No production Rust/TS code changes — the binding + attribution logic already work; only the world data was misaligned.

**Tech Stack:** Rust (bevy_ecs ECS), `sim-core` crate. Cargo MUST be routed through `scripts/cargo-serial.sh` (never two cargo at once). Determinism is mandatory: `BTreeMap`/sorted only, no `HashMap` iteration / RNG / wall-clock.

**Worktree:** `/Users/ramonfuglister/Coding/abutown-blocker1`, branch `feat/abutopia-economy-visible` (off `origin/main` `06d3828`). Spec: `docs/superpowers/specs/2026-06-07-abutopia-economy-visible-design.md`.

---

## Background the implementer needs

- **Why routed=0 today:** all 300 pedestrians spawn on `corridor:sidewalk:south` (tiles x≈106–117, y=64.51, chunk (3,2)). `assign_binding` picks the nearest market by distance over market **node** positions. From the corridor the nearest market is 9003 (supply, no consumption → shop channel reads `consumed_qty=0`); the consumption market 9002 sits far away at (13,3) with zero bound citizens. Moving 9002 onto the corridor makes it the nearest market → `home_market=9002` → shop channel routes.
- **Key symbols (verified):**
  - `sim_core::mobility::market_binding::assign_binding(pos: (f32,f32), markets: &[(u32,(f32,f32))]) -> Option<MarketBinding>` (nearest=home, second=work, tie-break lower id).
  - `sim_core::mobility::market_binding::markets_with_positions(world: &World) -> Vec<(u32,(f32,f32))>` (reads `Markets` + `Graph`; returns snapped node positions; empty if either resource absent).
  - `sim_core::mobility::seed::from_base_world_bundle(&BaseWorldBundle) -> Result<(World, Schedule), SeedError>` — builds the FULL graph + `NodeSpatialIndex` (incl. corridor footway nodes) and MobilityPlugin, and spawns pedestrians. **It does NOT seed the economy**, so a test must install `EconomyPlugin` + call `seed_from_markets_layer` afterward to get `Markets`.
  - `sim_core::economy::EconomyPlugin`, `sim_core::economy::seed_from_markets_layer(&mut World, &MarketLayer)`.
  - `sim_core::economy::attribution::run_citizen_attribution_system(&mut World)` — populates `CitizenEconomicTargets` (a `BTreeMap<AgentId, NodeId>`); `routed = CitizenEconomicTargets.0.len()`.
  - Attribution shop channel needs: market node in an Active/Hot chunk (`ChunkCoordComp` + `ActiveChunk` entity) AND `MarketGoodState.consumed_qty_last_tick > 0` AND citizens with `(AgentMarker, StableAgentId, MarketBinding{home_market})` bound to that market.
  - `sim_core::mobility::api::spawn_agent_from_record_with_position(&mut World, AgentRecord, (f32,f32))` — inserts the agent; if `record.home_market==0` it computes the binding from the given position against the live `Markets` (this is how the runtime binds at materialization).
- **Existing test patterns to copy:** `economy/tests/seed.rs` (bundle load via `BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia")`); the attribution unit tests in `economy/attribution.rs` (how to spawn an `ActiveChunk` entity + set `MarketGoods.consumed_qty_last_tick` + spawn bound citizens); `economy/tests/capita.rs` `run_density_scenario` (attribution wiring).

## File Structure

- **Modify** `data/worlds/abutopia/layers/markets.json` — the 9002 anchor (the behavior change).
- **Modify** `backend/crates/sim-core/src/economy/tests/seed.rs` — two 4-node test helpers (lines ~28 and ~277) hard-code node(1) at 9002's old anchor.
- **Modify** `backend/crates/sim-core/src/economy/markets_layer.rs` — the `unseeded_world()` test helper (line ~296) hard-codes a node at 9002's old anchor.
- **Create** `backend/crates/sim-core/src/economy/tests/abutopia_visible.rs` — the two new tests.
- **Modify** `backend/crates/sim-core/src/economy/tests/mod.rs` — register `mod abutopia_visible;`.

---

### Task 1: Re-anchor 9002 + repair economy test helpers (TDD via the binding test)

**Files:**
- Create: `backend/crates/sim-core/src/economy/tests/abutopia_visible.rs`
- Modify: `backend/crates/sim-core/src/economy/tests/mod.rs`
- Modify: `data/worlds/abutopia/layers/markets.json:6`
- Modify: `backend/crates/sim-core/src/economy/tests/seed.rs` (≈ lines 28, 277)
- Modify: `backend/crates/sim-core/src/economy/markets_layer.rs` (≈ line 296)

- [ ] **Step 1: Write the failing binding test**

Create `backend/crates/sim-core/src/economy/tests/abutopia_visible.rs`:

```rust
//! Blocker-1: prove the abutopia world data makes residential-corridor pedestrians
//! bind home_market to a co-located consumption market (9002), so attribution can
//! route them. These tests load the REAL abutopia bundle.

use crate::base_world::BaseWorldBundle;
use crate::economy::{seed_from_markets_layer, EconomyPlugin};
use crate::mobility::market_binding::{assign_binding, markets_with_positions};
use crate::mobility::seed::from_base_world_bundle;

/// Build the full abutopia world (graph + NodeSpatialIndex via the mobility builder)
/// and seed the economy on top, so `markets_with_positions` returns the snapped
/// market node positions. Returns the live world.
fn abutopia_world_with_economy() -> bevy_ecs::world::World {
    let bundle = BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia")
        .expect("abutopia bundle loads");
    let (mut world, mut schedule) =
        from_base_world_bundle(&bundle).expect("abutopia world builds from bundle");
    // from_base_world_bundle is mobility-only; add the economy so Markets exist + snap.
    EconomyPlugin.install(&mut world, &mut schedule);
    seed_from_markets_layer(&mut world, &bundle.markets);
    world
}

/// Corridor:sidewalk:south spans tiles x≈106..117 at y=64.51. After re-anchoring 9002
/// onto the corridor, every pedestrian there must bind home_market = 9002 (nearest).
#[test]
fn corridor_pedestrians_bind_home_market_9002() {
    let world = abutopia_world_with_economy();
    let positions = markets_with_positions(&world);
    assert_eq!(positions.len(), 4, "all four abutopia markets snapped to graph nodes");

    for px in [106.0_f32, 111.5, 117.0] {
        let binding = assign_binding((px, 64.51), &positions)
            .expect("binding exists with four live markets");
        assert_eq!(
            binding.home_market, 9002,
            "pedestrian at ({px}, 64.51) must bind home_market=9002 (the co-located consumption market); got {}",
            binding.home_market
        );
    }
}
```

Register it: add `mod abutopia_visible;` to `backend/crates/sim-core/src/economy/tests/mod.rs` (match the existing `mod <name>;` lines).

- [ ] **Step 2: Run the test to verify it FAILS (9002 still at old anchor)**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core abutopia_visible -- --nocapture`
Expected: FAIL — `corridor_pedestrians_bind_home_market_9002` asserts `home_market=9002` but gets `9003` (9002 is still at (13,3), far from the corridor). (If it instead fails to compile, fix imports against the real signatures above before proceeding.)

- [ ] **Step 3: Re-anchor 9002 in the world data**

In `data/worlds/abutopia/layers/markets.json` line 6, change the 9002 anchor:

```json
    { "id": 9002, "name": "Demo B",      "anchor": [111.5, 64.51] },
```

(Only the anchor changes. `distances`, `supply`, `demand`, `extractors`, `opening_prices`, `household` are untouched — distances auto-recompute from positions; opening_prices key on market id.)

- [ ] **Step 4: Repair the three economy test-helper nodes that hard-code 9002's old anchor**

Moving 9002 means its anchor no longer snaps to the helper's `node(1)` at `(13.0, 3.0)`; it would collide with another node and `seed_from_markets_layer` would silently no-op, cascading failures across every `seed_world()`/`unseeded_world()` test. Update all three to 9002's new anchor.

In `backend/crates/sim-core/src/economy/tests/seed.rs`, BOTH 4-node lists (≈ line 28 and ≈ line 277):

```rust
        node(0, 2.0, 3.0),
        node(1, 111.5, 64.51),
        node(2, 16.0, 48.0),
        node(3, 208.0, 48.0),
```

In `backend/crates/sim-core/src/economy/markets_layer.rs`, the `unseeded_world()` helper node (≈ line 296), change its position:

```rust
                position: (111.5, 64.51),
```

- [ ] **Step 5: Run the binding test + the affected suites to verify they PASS**

Run, one at a time:
- `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core abutopia_visible -- --nocapture` → PASS (`home_market=9002` at all three corridor x positions).
- `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy::tests::seed` → all pass (4 markets, distance pairs, capita_baseline reapply still green — the helper now snaps 9002 to node 1 at the new position).
- `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core markets_layer` → all pass (4 markets / 2 directed distance pairs / opening prices for 9002).

Expected: all green. If any `seed_world`/`unseeded_world`-based test fails with "Markets empty" or a missing-key panic, a helper node was missed — recheck Step 4.

- [ ] **Step 6: Commit**

```bash
git add data/worlds/abutopia/layers/markets.json \
        backend/crates/sim-core/src/economy/tests/abutopia_visible.rs \
        backend/crates/sim-core/src/economy/tests/mod.rs \
        backend/crates/sim-core/src/economy/tests/seed.rs \
        backend/crates/sim-core/src/economy/markets_layer.rs
git commit -m "feat(world): re-anchor abutopia market 9002 onto the residential corridor

Pedestrians on corridor:sidewalk:south now bind home_market=9002 (a consumption
market co-located in their chunk), so the attribution shop channel can route them.
New binding test proves home_market=9002 from the real bundle; three economy
test-helper nodes updated to 9002's new anchor.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: End-to-end routed>0 test

**Files:**
- Modify: `backend/crates/sim-core/src/economy/tests/abutopia_visible.rs`

Prove the full chain on real data: a citizen at the corridor binds `home_market=9002`; with 9002 observed and consuming, attribution routes the citizen to 9002's node. Consumption is injected deterministically (the macro flow's ability to deliver over the 171-tile leg — delivered ≈1855 < max_price 2000 — is verified by arithmetic and covered by the existing economy schedule suites; this test isolates the binding→observe→route chain).

- [ ] **Step 1: Write the routed>0 test**

Append to `backend/crates/sim-core/src/economy/tests/abutopia_visible.rs`:

```rust
/// Full chain on real data: spawn a citizen at the corridor (binds home=9002 against
/// the live economy), mark 9002's chunk observed, give 9002 realized consumption, run
/// attribution → the citizen is routed to 9002's node (routed > 0).
#[test]
fn corridor_citizen_is_routed_to_9002_when_observed_and_consuming() {
    use crate::economy::{MarketGoodKey, MarketGoodState, MarketGoods, MarketId, Markets, Quantity};
    use crate::ids::ChunkCoord;
    use crate::mobility::resources::CitizenEconomicTargets;
    use crate::world::components::{ActiveChunk, ChunkCoordComp};

    let mut world = abutopia_world_with_economy();

    // 9002's snapped node + chunk (the market sits on the corridor in chunk (3,2)).
    let node_9002 = world
        .resource::<Markets>()
        .0
        .get(&MarketId(9002))
        .expect("market 9002 seeded")
        .node_id;
    let pos = world.resource::<crate::routing::Graph>().node(node_9002).position;
    let chunk = crate::mobility::chunk_of(pos.0, pos.1, 32);

    // Spawn a handful of citizens at the corridor; with the live economy present they
    // bind home_market=9002 (record carries home_market==0 → assign at spawn).
    for i in 0..5 {
        let rec = crate::mobility::records::AgentRecord::new_born_at(
            crate::ids::AgentId(format!("agent:walk:{i}")),
            crate::mobility::records::AgentMobilityState::Walking {
                link_id: "link:walk:corridor:0".to_string(),
                progress: 0.5,
            },
            vec![],
            0.05,
            0,
        );
        crate::mobility::api::spawn_agent_from_record_with_position(&mut world, rec, (111.5, 64.51));
    }

    // Mark 9002's chunk observed.
    world.spawn((ChunkCoordComp(chunk), ActiveChunk));

    // Give 9002 realized consumption this tick (both authored goods at 9002).
    for good in [crate::economy::GOOD_TOOLS, crate::economy::GOOD_FOOD] {
        let key = MarketGoodKey { market: MarketId(9002), good };
        let mut goods = world.resource_mut::<MarketGoods>();
        let st = goods.0.entry(key).or_insert_with(|| MarketGoodState::new(key));
        st.consumed_qty_last_tick = Quantity(30);
    }

    crate::economy::attribution::run_citizen_attribution_system(&mut world);

    let targets = &world.resource::<CitizenEconomicTargets>().0;
    assert!(!targets.is_empty(), "corridor citizens must be routed (routed>0); got 0");
    for (_, node) in targets.iter() {
        assert_eq!(*node, node_9002, "routed citizens target market 9002's node");
    }
    println!("routed={} all → node {:?}", targets.len(), node_9002);
}
```

- [ ] **Step 2: Run the test**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core abutopia_visible -- --nocapture`
Expected: PASS — `routed > 0`, all targets at 9002's node.

If it fails to compile (signature drift on `AgentRecord::new_born_at`, `AgentMobilityState`, `chunk_of`, `GOOD_TOOLS/GOOD_FOOD`, or `MarketGoodState::new`), correct against the real definitions (grep `fn new_born_at`, `enum AgentMobilityState`, `pub fn chunk_of`, `pub const GOOD_`, `impl MarketGoodState`) — the structure is correct; only exact paths/fields may differ. If `routed==0`, verify (a) the spawned citizens actually got `MarketBinding{home_market:9002}` (query `MarketBinding` after spawn) and (b) the `ActiveChunk` chunk equals 9002's node chunk.

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-core/src/economy/tests/abutopia_visible.rs
git commit -m "test(economy): abutopia corridor citizen routes to 9002 (routed>0 end-to-end)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Whole-system gate + e2e browser-smoke (controller-run)

**Files:** none (verification only). Run by the controller, not a fresh implementer subagent.

The change is world data + tests only; the frontend/e2e are unaffected (`render-smoke` asserts only the 300-pin + clock; no market-position assertions), but the move touches the render-coordinate boundary so the smoke is mandatory.

- [ ] **Step 1: Rust workspace gate**

Run (one at a time, via cargo-serial):
- `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check` → `EXIT=0`.
- `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings` → clean.
- `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace` → all suites pass (sim-core incl. the two new tests, sim-server, 0 failures).

- [ ] **Step 2: Frontend gate**

(Symlink `node_modules` from the main worktree if absent: `ln -s /Users/ramonfuglister/Coding/abutown/node_modules ./node_modules` — gitignored.)
- `npm run typecheck` → no errors.
- `npm test` → vitest all pass (incl. `mobilityProtocol.test.ts`, `economyMarkets.test.ts`, `marketInspector.test.ts` — none assert 9002's position).
- `npm run build` → build complete.

- [ ] **Step 3: e2e browser-smoke (hermetic e2e_server, no remote DB)**

Run: `CORS_ALLOWED_ORIGINS="http://127.0.0.1:5173" npm run test:e2e`
Expected: `render-smoke.spec.ts` 2/2 pass (300-pin + clock-advance) — `e2e_server` loads the moved 9002 from `data/worlds/abutopia`; the smoke is agnostic to market position.

- [ ] **Step 4: Finish the branch**

Use `superpowers:finishing-a-development-branch` → push + open PR against `main`. The PR body must include the deploy note: a one-time `DELETE FROM economy_snapshots` + `DELETE FROM mobility_snapshots` for the abutopia world is required on deploy (market positions + bindings are preserved on hydrate; the wipe re-seeds them and simultaneously resolves Blocker-2). No schema bump. Wait for CI green, then squash-merge + clean up.

---

## Self-Review

**1. Spec coverage:** The change (re-anchor 9002) → Task 1. Binding proof → Task 1 Step 1. Routed>0 proof → Task 2. Existing suites stay green / position-assertion updates → Task 1 Step 4–5. Browser-smoke + full CI → Task 3. Deploy/persistence note → Task 3 Step 4. All spec acceptance criteria are covered.

**2. Placeholder scan:** No TBD/placeholders; every code step shows full code and exact commands. The `vec![]` plan stage in Task 2 is intentional (the test never runs the mobility schedule, so an empty plan is fine — noted implicitly by injecting consumption + calling attribution directly).

**3. Type consistency:** `assign_binding`/`markets_with_positions`/`from_base_world_bundle`/`seed_from_markets_layer`/`run_citizen_attribution_system`/`spawn_agent_from_record_with_position` names match the grounded signatures. The anchor `[111.5, 64.51]` is used identically in markets.json and all three helper-node fixes and the binding-test corridor positions. `home_market=9002` is the consistent expected value across tests.
