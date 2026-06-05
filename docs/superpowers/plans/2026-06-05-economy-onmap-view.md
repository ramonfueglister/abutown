# On-Map Economy View — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the mean-field economy readable on the one canonical isometric map — markets become first-class authored single-tile glyphs with a read-only click-inspector — without changing any economy dynamics.

**Architecture:** A pure read-only projection layer over an unchanged, server-authoritative economy, in three sequential sub-slices: **A** sources markets from an authored `markets.json` world layer via a pool-factory (byte-identical to today's seed); **B** ships market locations + per-(market,good) state over a new additive `ServerMessage` case; **C** renders the single-tile market glyph and the read-only inspector, proven by a mandatory browser-smoke.

**Tech Stack:** Rust (bevy_ecs sim-core, axum sim-server, prost protobuf), TypeScript/Vite frontend (@bufbuild/protobuf), Postgres persistence.

**Spec:** `docs/superpowers/specs/2026-06-05-economy-onmap-view-design.md`

---

## Verified facts (pinned against `origin/main` d5dfffa, in worktree `abutown-vtraders`)

All anchors are in `/Users/ramonfuglister/Coding/abutown-vtraders/`. Backend = `backend/crates/...`; frontend = `src/...`. **Re-confirm a line anchor with a quick read before editing — numbers may drift.**

### Cargo discipline (MANDATORY — CLAUDE.md + memory)
Every cargo invocation goes through the serial wrapper on the isolated tmp target. **Never** `--workspace --all-targets` during iteration; fmt uses `--all`. First run rebuilds from scratch (slow) — run in background and poll.
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core <FILTER>
# fmt gate: scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
```
Run all cargo from the worktree root `/Users/ramonfuglister/Coding/abutown-vtraders`.

### Economy core types — `backend/crates/sim-core/src/economy/`
- `ids.rs`: `GoodId(pub u16)` :4 · `MarketId(pub u32)` :7 · `EconomicActorId(pub u64)` :13 — all derive `Copy,Clone,Debug,Eq,PartialEq,Ord,PartialOrd,Hash,Serialize,Deserialize`.
- `goods.rs`: `GOOD_FOOD = GoodId(1)` :3 · `GOOD_TOOLS = GoodId(4)` :6 · `GOOD_RAW = GoodId(5)` :13.
- `money.rs`: `ECONOMY_SCALE: i128 = 1_000` :1 · `Money(pub i64)` :20 (`Money::ZERO` :26) · `Quantity(pub i64)` :23.
- `market.rs`: `MarketSite { id: MarketId, node_id: crate::routing::NodeId, name: String }` :8 · `MarketGoodKey { market: MarketId, good: GoodId }` :15 · `MarketGoodState { key, last_settlement_price: Money, ewma_reference_price: Money, traded_qty_last_tick: Quantity, unmet_demand_last_tick: Quantity, unsold_supply_last_tick: Quantity, consumed_qty_last_tick: Quantity, dirty: bool, last_cleared_tick: u64 }` :23 · `Markets(pub BTreeMap<MarketId, MarketSite>)` :58 · `MarketGoods(pub BTreeMap<MarketGoodKey, MarketGoodState>)` :61 · `MarketChunks(pub BTreeMap<MarketId, ChunkCoord>)` :71 · `MarketDistances(pub BTreeMap<(MarketId, MarketId), i64>)` :84.
- `pools.rs`: `DemandPool { actor, market, good, desired_qty_per_tick: Quantity, max_price: Money, urgency_bps: i32, elasticity_bps: i32, interval_ticks: u64, last_generated_tick: Option<u64>, last_consumed_tick: Option<u64>, income_last_tick: Money, mpc_bps: i32, autonomous: Money }` :11 · `SupplyPool { actor, market, good, offered_qty_per_tick: Quantity, min_price: Money, interval_ticks: u64, last_generated_tick: Option<u64> }` :41 · `DemandPools`/`SupplyPools` BTreeMap wrappers :52/:55.
- `production.rs`: `Recipe { inputs: Vec<(GoodId, Quantity)>, outputs: Vec<(GoodId, Quantity)> }` :10 · `ProductionPool { actor, recipe, interval_ticks, last_generated_tick }` :16 · `ProductionPools` :24 · `EXTRACTOR_TOOLS = 8_031` :72 · `EXTRACTOR_FOOD_A = 8_032` :77 · `EXTRACTOR_FOOD_FA = 8_033` :78 · `RawDeposit { good, qty_per_interval: Quantity, interval_ticks, last_regen_tick }` :83 · `RawDeposits` :91.
- `wages.rs`: `HOUSEHOLD_SECTOR = EconomicActorId(u64::MAX - 1)` :20 · `WageTelemetry(pub BTreeMap<MarketId, Money>)` :35 (ephemeral) · `HouseholdSector { population: u64, pool_weights: BTreeMap<EconomicActorId, i64> }` :47 (no serde; persisted via `HouseholdSectorSnapshot`).
- `systems.rs`: `EconomyConfig.trader_default_ref_price` defaults to `Money(1_000)` :146.
- **No `#[serde(default)]` anywhere in the economy module.** `MarketGoodState` field order is serialization-sensitive.

### Seed + plugin + persistence
- `seed.rs`: `pub fn seed_demo_economy(world: &mut World)` :41; idempotency guard `if !world.resource::<Markets>().0.is_empty() { return; }` :47. Full current contents are the source of truth for the authored data (markets 9_001..9_004, REF_A(2,3)/REF_B(13,3)/REF_FA(16,48)/REF_FB(208,48), pools 8_001/8_002/8_011/8_012/8_021/8_022, extractors 8_031@m_a/8_032@m_a/8_033@m_fa, qty 10, min_price 500, max_price 2_000, opening inv/cash 1_000_000, opening prices 1_000, household equal weights).
- `mod.rs`: `EconomyPlugin::install(&self, world, schedule)` :60 inserts all economy resources then `install_systems(schedule)`.
- `persist.rs`: `EconomyPersistSnapshot` :35 with fields `accounts, inventory, bids, asks, next_order_id, markets, market_goods, demand_pools, supply_pools, production_pools, raw_deposits, market_chunks, ledger_tail, market_distances, household_sector`. `extract_from_world(&World) -> EconomyPersistSnapshot` :66 · `apply_into_world(&mut World, &EconomyPersistSnapshot)` :121 · `EconomySnapshotProvider::collect` uses `schema_version: 1` :166-178.
- Server startup `backend/crates/sim-server/src/runtime/mod.rs`: fresh path installs `RoutingPlugin` :207, `EconomyPlugin` :219, `seed_demo_economy(&mut world)` :223; hydrate path installs `EconomyPlugin` :358, restores `econ_snap` :385, `seed_demo_economy(world)` :396 (idempotent). **`seed_demo_economy` must run after `RoutingPlugin` (needs `Graph` + `NodeSpatialIndex`).**

### World bundle / loader — `backend/crates/sim-core/src/base_world.rs`
- `SUPPORTED_SCHEMA_VERSION: u32 = 1` :12 · `BaseWorldManifest { schema_version: u32, world_id: String, display_name: String, chunk_size: u16, world_tiles: WorldTiles, layers: BaseWorldLayerFiles }` :51 · `BaseWorldLayerFiles { terrain, transport, buildings, decorations, spawns: String }` :61 · `BaseWorldBundle { manifest, terrain, transport, buildings, decorations, spawns }` :193 · `SpawnLayer { schema_version: u32, world_id: String, pedestrian_groups, car_groups, tram_lines }` :148 · `load_from_dir(root) -> Result<Self, BaseWorldError>` :204 (reads manifest, joins each layer path, `read_json`, `validate()`). `read_json` :511 fails closed (no defaults). `BaseWorldError` :14 variants include `Read`, `Parse`, `UnsupportedSchema`, `WorldIdMismatch`, `EmptyLayer(&'static str)`, `OutOfBounds`. Manifest at `data/worlds/abutopia/manifest.json` (schema_version 1, chunk_size 32, layers map of 5 paths). Spawn layer at `data/worlds/abutopia/layers/spawns.json`.

### Protocol / wire — `backend/crates/protocol/`
- `proto/abutown.proto` `ServerMessage.body` oneof :55-62 has `hello=1, tile_pulse=2, mobility_chunk_delta=3, mobility_chunk_snapshot=4, world_event=5, error=6`. **Next free tag = 7.** `reserved 100 to max`.
- Proto package `abutown.v1`; prost-generated into `protocol::v1` via `build.rs` (**backend regen is automatic on `cargo build`** — no manual command). Re-exported in `sim-server` as `use abutown_protocol::v1 as w;`.
- `protocol/src/lib.rs:10`: `pub const PROTOCOL_VERSION: u16 = 1;`.
- Body construction pattern: `w::ServerMessage { body: Some(w::server_message::Body::Hello(w::Hello { .. })) }`. Roundtrip test pattern: `proto_roundtrip_tests` in `protocol/src/lib.rs:494` (`assert_roundtrip(&msg)`).

### Server send sites — `backend/crates/sim-server/src/app/mod.rs`
- `build_read_view_from_runtime(runtime, per_chunk, pulse_sequence) -> RuntimeReadView` :277. Reads economy resources via `runtime.mobility()` which returns `&sim_core::bevy_ecs::world::World` (the SAME world EconomyPlugin installs into — see :293 comment + `chunk_snapshot_to_dto(world: &World, ...)` :1077). `RuntimeReadView` is in `runtime_view.rs:72`, stores proto `w::` types (e.g. `world_summary: w::WorldSummary`).
- Hello sent once on WS connect: `stream_world_deltas` :734, Hello built+sent :738-746 — **insert the initial EconomySnapshot send right after the Hello send (:748)**.
- Per-tick global broadcast: `tick_once` :1014; the global `deltas: broadcast::Sender<w::ServerMessage>` carries TilePulse at :1074 (`deltas.send(pulse_msg)`) — **insert the per-tick EconomySnapshot send next to it**. `view.store(Arc::new(new_view))` at :1047 publishes the read view (economy field included).
- `npm run generate:proto` → `node scripts/generate-proto-ts.mjs` regenerates `src/backend/proto/abutown_pb.ts` (buf + @bufbuild/protoc-gen-es). Adding fields/messages is non-breaking (new tags) — `npm run lint:proto-breaking` stays green.

### Frontend — `src/`
- Renderer `render/minimalMapRenderer.ts`: `drawScene(state)` :147; draw order :157-189 — **insert `drawEconomyMarkets` after `drawRiverSurfaceLayer` (:159), before `drawRoads` (:181)**. `drawBuilding(state, building)` :388, `drawTerrainOverlayLayer(state, coords)` :207, `isCoordVisible(coord, rect)` :619. Color consts :103-125 (`TRADER_COLOR='#c0392b'`, building hues). **No client `ECONOMY_SCALE` exists — introduce one.**
- Projection `render/minimalMapProjection.ts`: `MINIMAL_MAP_TILE_SIZE = {width:18,height:18}` :3 · `mapProject(coord, tile)` :5 · `mapUnproject(point, tile)` :15.
- Inspector `render/inspectorPanelPainter.ts`: `drawInspectorPanel(ctx, inspector, theme, pixelRatio)` :61 · `AGENT_INSPECTOR_PANEL {x:12,y:12,...}` :22 · `VEHICLE_INSPECTOR_PANEL {x:12,y:128,...}` :29 · `ctx.setTransform(pixelRatio,0,0,pixelRatio,0,0)` HUD idiom :71.
- Selection `app/entitySelection.ts`: `selectAtScreenPoint(point)` :19 · `findNearestProjectedEntity(entities, worldPoint, radius, projectedPoint)` :69 · mutual exclusion sets `(vehicleId,null)` or `(agentId,null)` :40-54.
- Decode `backend/mobilityState.ts`: `MobilityOverlayState` :46 · `applyServerMessage(state, message, now)` :180; switch on `message.body.case` :185-209 (`'mobilityChunkSnapshot'|'mobilityChunkDelta'|'hello'|'tilePulse'|'worldEvent'|'error'|undefined`). Decode `backend/mobilityClient.ts:204` (`fromBinary(ServerMessageSchema, bytes)`); proto→DTO converters in `backend/mobilityProtocol.ts`.
- Boot/main `main.ts`: `applyInitialRuntimeState(initial)` :132 (HTTP `/mobility` only) · `onMobilityState` callback wiring :113-115 · `frame(now)` :194 · `installRuntimeDiagnostics(window, {...})` :423.
- Browser-smoke: adapt `scripts/smoke-visible-traders.mjs` (current economy-adjacent smoke that decodes the binary wire); `npm run smoke:visible-traders`. (`scripts/smoke-7a.mjs` predates the binary wire — do NOT use it.)

### The 14 invariants the pool-factory must reproduce byte-for-byte
(See spec §Invariants — markets 9_001..9_004; REF anchors; chunk_of(.,32); distances ONLY (m_a↔m_b),(m_fa↔m_fb) no diagonal; pool/actor ids; extractor consts; qty=10/min=500/max=2_000/REGEN=10/interval=1; opening prices=1_000 for (m_b,TOOLS),(m_b,FOOD),(m_fb,FOOD); household equal weights over all consumer pools, extractors excluded; flow-demo chunk distances; determinism BTreeMap/no-RNG; money+goods conservation; persistence round-trip; actor-id band 8_0xx.)

---

## Sub-Slice A — Authored markets + pool-factory

**File structure:**
- Modify: `backend/crates/sim-core/src/base_world.rs` (add markets layer to manifest/bundle/loader/validation).
- Create: `data/worlds/abutopia/layers/markets.json` (the authored economy topology).
- Modify: `data/worlds/abutopia/manifest.json` (schema_version 1→2, add `markets` layer path).
- Create: `backend/crates/sim-core/src/economy/markets_layer.rs` (the `MarketLayer` serde types + `seed_from_markets_layer` pool-factory).
- Modify: `backend/crates/sim-core/src/economy/mod.rs` + `seed.rs` (export the factory; delete `seed_demo_economy` in A5).
- Modify: `backend/crates/sim-server/src/runtime/mod.rs` (call the factory with the bundle's market layer).

### Task A1: markets layer in the world bundle + loader

**Files:**
- Modify: `backend/crates/sim-core/src/base_world.rs:12,51-68,148-201,204-228,230-322`
- Modify: `data/worlds/abutopia/manifest.json`
- Test: `backend/crates/sim-core/src/base_world.rs` (tests module) or `backend/crates/sim-core/tests/base_world.rs`

- [ ] **Step 1: Write the failing test** — add to the base_world tests module:

```rust
#[test]
fn loads_markets_layer_from_abutopia_bundle() {
    let bundle = BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia")
        .expect("abutopia bundle loads with a markets layer");
    assert_eq!(bundle.manifest.schema_version, 2);
    // Four authored markets, ids 9001..9004, ascending.
    let ids: Vec<u32> = bundle.markets.markets.iter().map(|m| m.id).collect();
    assert_eq!(ids, vec![9001, 9002, 9003, 9004]);
    // Cross-market distances: ONLY the two intended pairs (each one entry; the
    // factory mirrors both directions at seed time).
    let pairs: Vec<(u32, u32)> =
        bundle.markets.distances.iter().map(|d| (d.from, d.to)).collect();
    assert_eq!(pairs, vec![(9001, 9002), (9003, 9004)]);
}

#[test]
fn markets_layer_rejects_malformed_json() {
    // A markets layer that fails schema parse must surface BaseWorldError::Parse,
    // never a silent default (NO-FALLBACK).
    let dir = tempfile::tempdir().unwrap();
    // ... write a manifest pointing at a markets.json with a non-numeric id ...
    // (use the existing malformed-layer test helper pattern in this module)
}
```

(If the module has no `tempfile` harness, model `markets_layer_rejects_malformed_json` on the nearest existing malformed-layer test; keep it asserting a `BaseWorldError::Parse`/`Read`.)

- [ ] **Step 2: Run it — expect FAIL** (`bundle.markets` field + `MarketLayer` type do not exist):
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core loads_markets_layer
```
Expected: compile error / FAIL.

- [ ] **Step 3: Add the layer serde types + bundle wiring** in `base_world.rs`:

```rust
// Bump supported schema (markets layer is a breaking world-data change).
pub const SUPPORTED_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MarketLayer {
    pub schema_version: u32,
    pub world_id: String,
    pub markets: Vec<MarketSpec>,
    pub distances: Vec<MarketDistanceSpec>,
    pub supply: Vec<SupplySpec>,
    pub demand: Vec<DemandSpec>,
    pub extractors: Vec<ExtractorSpec>,
    pub household: HouseholdSpec,
    pub opening_prices: Vec<OpeningPriceSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MarketSpec { pub id: u32, pub name: String, pub anchor: [f32; 2] }
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MarketDistanceSpec { pub from: u32, pub to: u32 }
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SupplySpec { pub actor: u64, pub market: u32, pub good: u16, pub qty: i64, pub min_price: i64, pub opening_inventory: i64 }
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DemandSpec { pub actor: u64, pub market: u32, pub good: u16, pub qty: i64, pub max_price: i64, pub mpc_bps: i32, pub autonomous: i64, pub opening_cash: i64 }
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExtractorSpec { pub actor: u64, pub market: u32, pub in_good: u16, pub out_good: u16, pub qty: i64, pub min_price: i64 }
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HouseholdSpec { pub population: u64 }
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpeningPriceSpec { pub market: u32, pub good: u16, pub price: i64 }
```

Add `pub markets: String` to `BaseWorldLayerFiles`; add `pub markets: MarketLayer` to `BaseWorldBundle`; in `load_from_dir`, load `root.join(&manifest.layers.markets)` via `read_json` and place it in the bundle (mirror the spawns load). In `validate()`, add a check that `markets.world_id == manifest.world_id` (reuse `WorldIdMismatch`) and that `markets.markets` is non-empty (reuse `EmptyLayer("markets.markets")`).

- [ ] **Step 4: Author the manifest change** — `data/worlds/abutopia/manifest.json`: set `"schema_version": 2` and add `"markets": "layers/markets.json"` to the `layers` object. (The `markets.json` file itself is Task A2; A1's loader test will fail until A2 exists — author a minimal valid `markets.json` now to make A1 green, then fill it fully in A2.)

- [ ] **Step 5: Run the tests — expect PASS:**
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core base_world
```
Expected: PASS.

- [ ] **Step 6: fmt + commit:**
```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all
git add backend/crates/sim-core/src/base_world.rs data/worlds/abutopia/manifest.json data/worlds/abutopia/layers/markets.json
git commit -m "feat(world): add authored markets layer to the world bundle + loader"
```

### Task A2: author `markets.json` reproducing the demo economy exactly

**Files:**
- Create/overwrite: `data/worlds/abutopia/layers/markets.json`

- [ ] **Step 1: Author the file** — transcribe the seed (`seed.rs`) verbatim into data. Goods: RAW=5, FOOD=1, TOOLS=4.

```json
{
  "schema_version": 2,
  "world_id": "abutopia",
  "markets": [
    { "id": 9001, "name": "Demo A",      "anchor": [2.0, 3.0] },
    { "id": 9002, "name": "Demo B",      "anchor": [13.0, 3.0] },
    { "id": 9003, "name": "Flow Demo A", "anchor": [16.0, 48.0] },
    { "id": 9004, "name": "Flow Demo B", "anchor": [208.0, 48.0] }
  ],
  "distances": [ { "from": 9001, "to": 9002 }, { "from": 9003, "to": 9004 } ],
  "supply": [
    { "actor": 8001, "market": 9001, "good": 4, "qty": 10, "min_price": 500, "opening_inventory": 1000000 },
    { "actor": 8011, "market": 9001, "good": 1, "qty": 10, "min_price": 500, "opening_inventory": 1000000 },
    { "actor": 8021, "market": 9003, "good": 1, "qty": 10, "min_price": 500, "opening_inventory": 1000000 }
  ],
  "demand": [
    { "actor": 8002, "market": 9002, "good": 4, "qty": 10, "max_price": 2000, "mpc_bps": 8000, "autonomous": 5000, "opening_cash": 1000000 },
    { "actor": 8012, "market": 9002, "good": 1, "qty": 10, "max_price": 2000, "mpc_bps": 8000, "autonomous": 5000, "opening_cash": 1000000 },
    { "actor": 8022, "market": 9004, "good": 1, "qty": 10, "max_price": 2000, "mpc_bps": 8000, "autonomous": 5000, "opening_cash": 1000000 }
  ],
  "extractors": [
    { "actor": 8031, "market": 9001, "in_good": 5, "out_good": 4, "qty": 10, "min_price": 500 },
    { "actor": 8032, "market": 9001, "in_good": 5, "out_good": 1, "qty": 10, "min_price": 500 },
    { "actor": 8033, "market": 9003, "in_good": 5, "out_good": 1, "qty": 10, "min_price": 500 }
  ],
  "household": { "population": 1000000 },
  "opening_prices": [
    { "market": 9002, "good": 4, "price": 1000 },
    { "market": 9002, "good": 1, "price": 1000 },
    { "market": 9004, "good": 1, "price": 1000 }
  ]
}
```

- [ ] **Step 2: Validate it loads** (re-run A1's `loads_markets_layer_from_abutopia_bundle`):
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core loads_markets_layer
```
Expected: PASS.

- [ ] **Step 3: Commit:**
```bash
git add data/worlds/abutopia/layers/markets.json
git commit -m "feat(world): author abutopia markets.json (verbatim demo economy)"
```

### Task A3: pool-factory `seed_from_markets_layer` + byte-identical equality test

**Files:**
- Create: `backend/crates/sim-core/src/economy/markets_layer.rs`
- Modify: `backend/crates/sim-core/src/economy/mod.rs` (`mod markets_layer; pub use markets_layer::seed_from_markets_layer;`)
- Test: add the temporary equality test to `backend/crates/sim-core/src/economy/tests/seed.rs` — that submodule already has the private `seed_world()` builder (:23) and imports the economy resources. `seed_world()` does **not** install `RoutingPlugin`; it hand-inserts `Graph` + `NodeSpatialIndex` with the 4 reference nodes at (2,3)/(13,3)/(16,48)/(208,48) (exactly the REF anchors) plus `EconomyPlugin`, then seeds — which is the scaffold the factory needs.

- [ ] **Step 1: Write the failing equality test** in `economy/tests/seed.rs` (TDD — proves byte-identity to the legacy seed). Read `seed_world()` first: if it bundles the graph scaffold and the economy seed inseparably, factor the scaffold (everything up to but excluding the `seed_demo_economy` call) into a local helper `unseeded_world()` so both seeders run on identical bare worlds:

```rust
#[test]
fn layer_seed_matches_legacy_seed_byte_for_byte() {
    use crate::economy::persist::extract_from_world;
    // World 1: legacy code seed (scaffold = Graph+NodeSpatialIndex+EconomyPlugin, as in seed_world()).
    let mut legacy = unseeded_world();
    crate::economy::seed::seed_demo_economy(&mut legacy);
    let snap_legacy = extract_from_world(&legacy);
    // World 2: layer-driven seed using the authored abutopia markets layer.
    // CWD for sim-core unit/integration tests is the crate root (backend/crates/sim-core),
    // so repo-root data/ is three levels up.
    let bundle = crate::base_world::BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia").unwrap();
    let mut authored = unseeded_world();
    crate::economy::seed_from_markets_layer(&mut authored, &bundle.markets);
    let snap_authored = extract_from_world(&authored);
    // ledger_tail is empty pre-tick-0 in both; EconomyPersistSnapshot derives PartialEq.
    assert_eq!(snap_authored, snap_legacy);
}
```

- [ ] **Step 2: Run — expect FAIL** (`seed_from_markets_layer` undefined):
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core layer_seed_matches_legacy
```
Expected: FAIL.

- [ ] **Step 3: Implement the factory** in `markets_layer.rs`. It mirrors `seed_demo_economy` exactly but reads every value from `&MarketLayer`. Structure:

```rust
use bevy_ecs::prelude::*;
use crate::base_world::MarketLayer;
use crate::economy::{/* AccountBook, InventoryBook, Markets, MarketSite, MarketChunks,
    MarketDistances, MarketGoods, MarketGoodKey, MarketGoodState, SupplyPool, SupplyPools,
    DemandPool, DemandPools, HouseholdSector, MarketId, EconomicActorId, GoodId, Money, Quantity */};
use crate::economy::production::{ProductionPool, ProductionPools, RawDeposit, RawDeposits, Recipe};
use crate::economy::transport::manhattan_tiles;
use crate::routing::{Graph, NodeSpatialIndex};

/// Data-driven economy seeder. Idempotent (no-ops if `Markets` is non-empty),
/// runs after RoutingPlugin (reads Graph + NodeSpatialIndex). Reproduces the
/// exact resources `seed_demo_economy` produced, sourced from `layer`.
pub fn seed_from_markets_layer(world: &mut World, layer: &MarketLayer) {
    if !world.resource::<Markets>().0.is_empty() {
        return;
    }
    // 1) Snap each market anchor -> nearest footway node; insert MarketSite + MarketChunks.
    //    On any anchor that fails to snap (graph too small): return early (matches the
    //    legacy "graph too small" no-op). Two markets must not snap to the same node.
    // 2) Bake MarketDistances for each distance pair (both directions) via manhattan_tiles.
    // 3) For each SupplySpec: InventoryBook.deposit(actor, good, opening_inventory);
    //    SupplyPools.insert(actor, SupplyPool { offered_qty_per_tick: qty, min_price, interval_ticks: 1, last_generated_tick: None, .. }).
    // 4) For each DemandSpec: AccountBook.deposit(actor, opening_cash);
    //    DemandPools.insert(actor, DemandPool { desired_qty_per_tick: qty, max_price, urgency_bps: 0,
    //      elasticity_bps: 0, interval_ticks: 1, last_generated_tick: None, last_consumed_tick: None,
    //      income_last_tick: Money::ZERO, mpc_bps, autonomous: Money(autonomous), .. }).
    // 5) For each ExtractorSpec: InventoryBook.deposit(actor, in_good, qty) opening RAW;
    //    RawDeposits.insert(actor, RawDeposit { good: in_good, qty_per_interval: qty, interval_ticks: 1, last_regen_tick: None });
    //    ProductionPools.insert(actor, ProductionPool { recipe: Recipe { inputs: vec![(in_good, qty)], outputs: vec![(out_good, qty)] }, interval_ticks: 1, last_generated_tick: None });
    //    SupplyPools.insert(actor, SupplyPool { good: out_good, offered_qty_per_tick: qty, min_price, market, interval_ticks: 1, last_generated_tick: None }).
    // 6) HouseholdSector { population: layer.household.population, pool_weights: equal weight 1 over EVERY DemandSpec.actor (BTreeMap, ascending) }. Assert HOUSEHOLD_SECTOR not among actors.
    // 7) Opening prices: for each OpeningPriceSpec, MarketGoods entry for (market,good) with
    //    ewma_reference_price = last_settlement_price = Money(price) when currently <= 0.
}
```

Implement each numbered block with the verbatim field values from the verified-facts list. **Order of insertion matters for the byte-identical snapshot** — match `seed.rs`'s ordering (markets first, then m_a/m_b pools, then flow pools, then extractors, then household, then opening prices). Iterate `layer.*` Vec order (which the JSON already lays out to match the legacy order).

- [ ] **Step 4: Run the equality test — expect PASS** (iterate until byte-identical):
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core layer_seed_matches_legacy
```
Expected: PASS. If it fails, diff the two `EconomyPersistSnapshot`s field-by-field and fix the factory until equal.

- [ ] **Step 5: fmt + commit:**
```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all
git add backend/crates/sim-core/src/economy/markets_layer.rs backend/crates/sim-core/src/economy/mod.rs backend/crates/sim-core/src/economy/tests/seed.rs
git commit -m "feat(economy): data-driven seed_from_markets_layer (byte-identical to demo seed)"
```

### Task A4: rewire the server to seed from the bundle's market layer

**Files:**
- Modify: `backend/crates/sim-server/src/runtime/mod.rs:223,396`

- [ ] **Step 1: Replace both call sites** (the in-scope bundle binding differs per path — verified):
  - Fresh path `new_with_event_store_and_base_world` (:223) — the owned bundle is named `bundle`: replace `seed_demo_economy(&mut world)` with `sim_core::economy::seed_from_markets_layer(&mut world, &bundle.markets)`.
  - Hydrate path `hydrate_from_stores` (:396) — the param is `base_world: &BaseWorldBundle`: replace `seed_demo_economy(world)` with `sim_core::economy::seed_from_markets_layer(world, &base_world.markets)`.

  Keep the same ordering (after `RoutingPlugin`, so `Graph` + `NodeSpatialIndex` are available); the factory's `Markets`-empty idempotency guard preserves the hydrate-path no-op.

- [ ] **Step 2: Build the server crate — expect compile OK:**
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server
```
Expected: builds (the runtime now seeds from data on fresh worlds).

- [ ] **Step 3: Commit:**
```bash
git add backend/crates/sim-server/src/runtime/mod.rs
git commit -m "feat(server): seed economy from the authored markets layer"
```

### Task A5: permanent invariant tests; delete legacy seed; document the one-time DELETE

**Files:**
- Modify: `backend/crates/sim-core/src/economy/markets_layer.rs` (permanent tests)
- Modify: `backend/crates/sim-core/src/economy/tests/seed.rs` (rewire `seed_world()` to the factory; drop the `seed_demo_economy` import + the temporary equality test)
- Delete: `backend/crates/sim-core/src/economy/seed.rs`; drop `mod seed;` / `pub use seed::*` from `economy/mod.rs`.

- [ ] **Step 1: Write permanent invariant tests** (these survive the legacy deletion) asserting, on a world seeded via `seed_from_markets_layer(&bundle.markets)`:
  - exactly markets `{9001,9002,9003,9004}` with names `Demo A/Demo B/Flow Demo A/Flow Demo B`;
  - `MarketDistances` keys are exactly `{(9001,9002),(9002,9001),(9003,9004),(9004,9003)}` (no diagonal);
  - `SupplyPools`/`DemandPools`/`ProductionPools`/`RawDeposits` actor-id sets match the spec lists; each pool's `interval_ticks==1`, supply `min_price==Money(500)`, demand `max_price==Money(2000)`, qty fields `==10`;
  - `HouseholdSector.pool_weights` keys are exactly `{8002,8012,8022}`, all weight 1; extractors absent; `HOUSEHOLD_SECTOR` absent;
  - opening `MarketGoodState` for `(9002,4),(9002,1),(9004,1)` has `ewma_reference_price==Money(1000)` and `last_settlement_price==Money(1000)`.

- [ ] **Step 2: Run them — expect PASS:**
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core markets_layer
```
Expected: PASS.

- [ ] **Step 3: Rewire the test builder, then delete the legacy seed.** `seed_world()` (`economy/tests/seed.rs:23`) and the other economy tests seed via `seed_demo_economy`; the factory must take over so they stay green and exercise the authored path:
  1. In `economy/tests/seed.rs`, change `seed_world()` to seed via `crate::economy::seed_from_markets_layer(&mut world, &crate::base_world::BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia").unwrap().markets)` instead of `seed_demo_economy`; remove `use crate::economy::seed::seed_demo_economy;` (:3); delete the temporary `layer_seed_matches_legacy_seed_byte_for_byte` test (it referenced the now-removed legacy seed).
  2. Delete `backend/crates/sim-core/src/economy/seed.rs` and drop `mod seed;` / any `pub use seed::*` from `economy/mod.rs`.
  3. Grep for residual references and fix every one:
```bash
rg -n "seed_demo_economy" backend/crates
```
Expected: zero matches.

- [ ] **Step 4: Re-run the full sim-core economy tests + fmt:**
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core economy
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
```
Expected: PASS, fmt clean.

- [ ] **Step 5: Document the one-time deploy DELETE.** Add a one-line note (commit message + the spec's persistence section already covers it): the authored seed produces byte-identical data, so persisted worlds restore unchanged; to make the authored loader the live source on the canonical dev world, run `DELETE FROM economy_snapshots;` ONCE at deploy. No serde-default shim.

- [ ] **Step 6: Commit:**
```bash
git add -A
git commit -m "feat(economy): promote markets to authored data; drop legacy demo seed"
```

---

## Sub-Slice B — Economy on the wire

**File structure:**
- Modify: `backend/crates/protocol/proto/abutown.proto` (new messages + oneof case 7).
- Modify: `backend/crates/sim-server/src/runtime_view.rs` (`economy: w::EconomySnapshot` field).
- Modify: `backend/crates/sim-server/src/app/mod.rs` (`build_economy_snapshot`, build into view, send on connect + per tick).

### Task B1: protocol — `EconomySnapshot` message + ServerMessage case 7

**Files:**
- Modify: `backend/crates/protocol/proto/abutown.proto:53-63` (+ new messages)
- Test: `backend/crates/protocol/src/lib.rs` `proto_roundtrip_tests`

- [ ] **Step 1: Add proto messages + oneof case.** In `abutown.proto`, add to `ServerMessage.body`: `EconomySnapshot economy_snapshot = 7;`. Add the messages (single thin snapshot — full state each send; markets are a handful):

```proto
message EconomySnapshot {
  uint32 protocol_version = 1;
  string world_id = 2;
  uint64 tick = 3;
  repeated EconomyMarket markets = 4;
  repeated EconomyMarketGood goods = 5;
}

message EconomyMarket {
  uint32 market_id = 1;
  string name = 2;
  sint32 tile_x = 3;
  sint32 tile_y = 4;
  int64 wage_paid_last_tick = 5; // raw i64; divide by ECONOMY_SCALE (1000) for display
}

message EconomyMarketGood {
  uint32 market_id = 1;
  uint32 good_id = 2;
  int64 last_settlement_price = 3;
  int64 ewma_reference_price = 4;
  int64 traded_qty_last_tick = 5;
  int64 unmet_demand_last_tick = 6;
  int64 unsold_supply_last_tick = 7;
}
```

- [ ] **Step 2: Write the roundtrip test** (mirror `roundtrip_hello` at lib.rs:530):

```rust
#[test]
fn roundtrip_economy_snapshot() {
    let msg = ServerMessage {
        body: Some(server_message::Body::EconomySnapshot(EconomySnapshot {
            protocol_version: u32::from(PROTOCOL_VERSION),
            world_id: "abutopia".into(),
            tick: 42,
            markets: vec![EconomyMarket { market_id: 9001, name: "Demo A".into(), tile_x: 2, tile_y: 3, wage_paid_last_tick: 320 }],
            goods: vec![EconomyMarketGood { market_id: 9002, good_id: 4, last_settlement_price: 5000, ewma_reference_price: 5100, traded_qty_last_tick: 10, unmet_demand_last_tick: 0, unsold_supply_last_tick: 0 }],
        })),
    };
    assert_roundtrip(&msg);
}
```

- [ ] **Step 3: Build the protocol crate (prost regenerates `v1`) — expect PASS:**
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p abutown-protocol roundtrip_economy
```
Expected: PASS.

- [ ] **Step 4: Commit:**
```bash
git add backend/crates/protocol/proto/abutown.proto backend/crates/protocol/src/lib.rs
git commit -m "feat(protocol): add EconomySnapshot server message (oneof tag 7)"
```

### Task B2: build the economy snapshot into the RuntimeReadView

**Files:**
- Modify: `backend/crates/sim-server/src/runtime_view.rs:72-86` (add `economy` field)
- Modify: `backend/crates/sim-server/src/app/mod.rs:277-346` (build it)
- Test: `backend/crates/sim-server/src/app/mod.rs` tests (or a focused test module)

- [ ] **Step 1: Add the field.** In `RuntimeReadView`, add `pub economy: w::EconomySnapshot,`.

- [ ] **Step 2: Write `build_economy_snapshot`** in `app/mod.rs` (reads economy resources from the world):

```rust
fn build_economy_snapshot(
    world: &sim_core::bevy_ecs::world::World,
    world_id: &abutown_protocol::WorldId,
    tick: u64,
) -> w::EconomySnapshot {
    use sim_core::economy::{Markets, MarketGoods, WageTelemetry};
    use sim_core::routing::Graph;
    let markets_res = world.resource::<Markets>();
    let goods_res = world.resource::<MarketGoods>();
    let wages = world.resource::<WageTelemetry>();
    let graph = world.resource::<Graph>();
    let markets = markets_res.0.iter().map(|(id, site)| {
        let pos = graph.node(site.node_id).position; // (f32, f32) tile coords
        w::EconomyMarket {
            market_id: id.0,
            name: site.name.clone(),
            tile_x: pos.0.floor() as i32,
            tile_y: pos.1.floor() as i32,
            wage_paid_last_tick: wages.0.get(id).copied().unwrap_or(sim_core::economy::Money::ZERO).0,
        }
    }).collect();
    let goods = goods_res.0.iter().map(|(key, st)| w::EconomyMarketGood {
        market_id: key.market.0,
        good_id: u32::from(key.good.0),
        last_settlement_price: st.last_settlement_price.0,
        ewma_reference_price: st.ewma_reference_price.0,
        traded_qty_last_tick: st.traded_qty_last_tick.0,
        unmet_demand_last_tick: st.unmet_demand_last_tick.0,
        unsold_supply_last_tick: st.unsold_supply_last_tick.0,
    }).collect();
    w::EconomySnapshot { protocol_version: u32::from(abutown_protocol::PROTOCOL_VERSION), world_id: world_id.0.clone(), tick, markets, goods }
}
```

(Fact: `Graph::node(&self, id) -> &Node` (`routing/graph.rs:93`) and `Node.position: (f32, f32)` tile coords (`routing/graph.rs:28`) — so `pos.0.floor() as i32` / `pos.1.floor() as i32` is correct. `wages.0.get(id).copied().unwrap_or(Money::ZERO)` reads ephemeral telemetry (`WageTelemetry::default()` is inserted by `EconomyPlugin` at `economy/mod.rs:87`, so `world.resource::<WageTelemetry>()` never panics) — the `unwrap_or` is "no wage paid this tick", not a forbidden data fallback.)

- [ ] **Step 3: Populate the view.** In `build_read_view_from_runtime`, after the existing fields, compute `let economy = build_economy_snapshot(runtime.mobility(), &world_id, mobility_tick);` and add `economy,` to the `RuntimeReadView { .. }` literal.

- [ ] **Step 4: Write a test** that a seeded runtime's view exposes 4 markets with the expected tiles + the three opening-priced goods. Place it where existing `build_read_view`/runtime tests live; build a runtime from the abutopia bundle, tick once, assert `view.economy.markets.len() == 4` and a known `(market_id, good_id)` has `ewma_reference_price == 1000` initially or a settled value after a tick.

- [ ] **Step 5: Run + fmt:**
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server economy
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all
```
Expected: PASS.

- [ ] **Step 6: Commit:**
```bash
git add backend/crates/sim-server/src/runtime_view.rs backend/crates/sim-server/src/app/mod.rs
git commit -m "feat(server): build EconomySnapshot into the runtime read view"
```

### Task B3: send the snapshot on connect + each tick

**Files:**
- Modify: `backend/crates/sim-server/src/app/mod.rs:746-748` (on connect) and `:1074` (per tick)

- [ ] **Step 1: Send on WS connect.** In `stream_world_deltas`, right after the Hello send succeeds (:748), send the current economy snapshot from the view:

```rust
let economy_msg = w::ServerMessage {
    body: Some(w::server_message::Body::EconomySnapshot(state.view().load().economy.clone())),
};
if send_server_message(&mut socket, economy_msg).await.is_err() {
    return;
}
```

- [ ] **Step 2: Send each tick.** In `tick_once`, after the TilePulse `deltas.send(pulse_msg)` (:1074), broadcast the freshly-published economy snapshot on the same global channel:

```rust
let economy_msg = w::ServerMessage {
    body: Some(w::server_message::Body::EconomySnapshot(view.load().economy.clone())),
};
let _ = deltas.send(economy_msg);
```

(Per-tick full snapshot is justified: ~4 markets × ~3 goods is tiny. Dirty-only via `DirtyMarketGoods`/`last_cleared_tick` and per-chunk replication are documented YAGNI escape hatches — do NOT build them now.)

- [ ] **Step 3: Build the server — expect OK:**
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server
```
Expected: builds.

- [ ] **Step 4: Commit:**
```bash
git add backend/crates/sim-server/src/app/mod.rs
git commit -m "feat(server): broadcast EconomySnapshot on connect and each tick"
```

---

## Sub-Slice C — Frontend glyph + inspector + browser-smoke

**File structure:**
- Regenerate: `src/backend/proto/abutown_pb.ts` (via `npm run generate:proto`).
- Modify: `src/backend/mobilityProtocol.ts` (proto→DTO converter + DTO types).
- Create: `src/backend/economyState.ts` (`EconomyOverlayState` + `applyEconomyServerMessage`).
- Modify: `src/backend/mobilityClient.ts` (dispatch decoded message to the economy reducer too).
- Modify: `src/render/minimalMapRenderer.ts` (`drawEconomyMarkets` + `ECONOMY_SCALE` + market drawable in render state).
- Modify: `src/app/entitySelection.ts` (`findNearestMarket` + `selectedMarketCoord`).
- Modify: `src/render/inspectorPanelPainter.ts` (market panel theme + content).
- Modify: `src/main.ts` (hold economy state, pass to render).
- Create: `scripts/smoke-economy-markets.mjs` (browser-smoke).

### Task C1: regenerate the TS proto + DTO converter

**Files:**
- Run: `npm run generate:proto`
- Modify: `src/backend/mobilityState.ts` (add the no-op `economySnapshot` switch case — REQUIRED or `tsc` breaks)
- Modify: `src/backend/mobilityProtocol.ts`
- Create: `src/backend/mobilityProtocol.test.ts` (new file — only `src/backend/proto/roundtrip.test.ts` exists today)

- [ ] **Step 1: Regenerate the proto bindings:**
```bash
npm run generate:proto
```
Expected: `src/backend/proto/abutown_pb.ts` now exports `EconomySnapshot`, `EconomyMarket`, `EconomyMarketGood` and the `economySnapshot` oneof case on `ServerMessage`.

- [ ] **Step 2: Patch the mobility reducer's switch (BLOCKER — regen breaks `tsc` otherwise).** `applyServerMessage` in `src/backend/mobilityState.ts` (switch at :185-209) has **no** `default` arm and is typed `: MobilityOverlayState` (non-optional). Once regen adds `'economySnapshot'` to the `message.body.case` union, the unhandled case makes the function able to return `undefined` → `npm run typecheck` (strict) FAILS, and at runtime an economy message would return `undefined` and corrupt mobility state. Add a named no-op case mirroring the existing `case 'tilePulse': return state;` (~:194):

```ts
    case 'economySnapshot':
      return state; // economy rides a separate overlay state; mobility ignores it
```

- [ ] **Step 3: Add DTO types + converter** in `mobilityProtocol.ts`:

```ts
export type MarketLocationDto = { marketId: number; name: string; tileX: number; tileY: number; wagePaidLastTick: number };
export type MarketGoodDto = { marketId: number; goodId: number; lastSettlementPrice: number; ewmaReferencePrice: number; tradedQtyLastTick: number; unmetDemandLastTick: number; unsoldSupplyLastTick: number };
export type EconomySnapshotDto = { tick: number; markets: MarketLocationDto[]; goods: MarketGoodDto[] };

export function economySnapshotFromProto(p: EconomySnapshot): EconomySnapshotDto {
  return {
    tick: Number(p.tick),
    markets: p.markets.map((m) => ({ marketId: m.marketId, name: m.name, tileX: m.tileX, tileY: m.tileY, wagePaidLastTick: Number(m.wagePaidLastTick) })),
    goods: p.goods.map((g) => ({ marketId: g.marketId, goodId: g.goodId, lastSettlementPrice: Number(g.lastSettlementPrice), ewmaReferencePrice: Number(g.ewmaReferencePrice), tradedQtyLastTick: Number(g.tradedQtyLastTick), unmetDemandLastTick: Number(g.unmetDemandLastTick), unsoldSupplyLastTick: Number(g.unsoldSupplyLastTick) }));
}
```

(BigInt `int64`/`uint64` fields from protobuf-es are `bigint` — wrap each in `Number(...)`, matching how `tick` is handled for mobility.)

- [ ] **Step 4: Write a converter unit test** in the new file `src/backend/mobilityProtocol.test.ts` (model it on `src/backend/proto/roundtrip.test.ts`): construct an `EconomySnapshot` proto with `create(EconomySnapshotSchema, {...})`, call `economySnapshotFromProto`, assert the DTO fields (incl. that `bigint` price/qty fields became `number`).

- [ ] **Step 5: Run typecheck + the test:**
```bash
npm run typecheck && npx vitest run src/backend/mobilityProtocol
```
Expected: PASS (typecheck green confirms the Step 2 switch patch landed).

- [ ] **Step 6: Commit:**
```bash
git add src/backend/proto/abutown_pb.ts src/backend/mobilityState.ts src/backend/mobilityProtocol.ts src/backend/mobilityProtocol.test.ts
git commit -m "feat(client): decode EconomySnapshot proto to DTO + handle case in mobility reducer"
```

### Task C2: economy overlay state + reducer + WS dispatch

**Files:**
- Create: `src/backend/economyState.ts`
- Modify: `src/backend/mobilityClient.ts`
- Test: `src/backend/economyState.test.ts`

- [ ] **Step 1: Write the failing reducer test:**

```ts
import { describe, it, expect } from "vitest";
import { createEconomyOverlayState, applyEconomyServerMessage } from "./economyState";
// build a ServerMessage with an economySnapshot body and assert markets/goods maps populate.
```

- [ ] **Step 2: Implement `economyState.ts`:**

```ts
import type { ServerMessage } from "./proto/abutown_pb";
import { economySnapshotFromProto, type MarketLocationDto, type MarketGoodDto } from "./mobilityProtocol";

export type EconomyOverlayState = {
  tick: number;
  markets: Map<number, MarketLocationDto>;          // by marketId
  goods: Map<string, MarketGoodDto>;                // key `${marketId}:${goodId}`
};

export function createEconomyOverlayState(): EconomyOverlayState {
  return { tick: 0, markets: new Map(), goods: new Map() };
}

export function applyEconomyServerMessage(state: EconomyOverlayState, message: ServerMessage): EconomyOverlayState {
  if (message.body.case !== "economySnapshot") return state;
  const dto = economySnapshotFromProto(message.body.value);
  const markets = new Map(dto.markets.map((m) => [m.marketId, m]));
  const goods = new Map(dto.goods.map((g) => [`${g.marketId}:${g.goodId}`, g]));
  return { tick: dto.tick, markets, goods };
}
```

- [ ] **Step 3: Dispatch in the WS client.** In `mobilityClient.ts`, where each decoded `ServerMessage` is applied to the mobility state, also apply it to an economy state and expose an `onEconomyState` callback (mirror the existing `onMobilityState` wiring exactly). Safe because the mobility reducer now has the explicit `case 'economySnapshot': return state;` no-op (added in C1 Step 2) and `applyEconomyServerMessage` returns `state` unchanged for every non-economy case — so dispatching every decoded message to both reducers is a no-op on the irrelevant one.

- [ ] **Step 4: Run tests + typecheck:**
```bash
npm run typecheck && npx vitest run src/backend/economyState
```
Expected: PASS.

- [ ] **Step 5: Commit:**
```bash
git add src/backend/economyState.ts src/backend/economyState.test.ts src/backend/mobilityClient.ts
git commit -m "feat(client): economy overlay state + reducer + WS dispatch"
```

### Task C3: render the single-tile market glyph

**Files:**
- Modify: `src/render/minimalMapRenderer.ts:103-125,147-189`
- Create: `tests/render/economyMarkets.test.ts` (no renderer unit harness exists today; `drawScene` is private — so test the pure visibility helper, not the canvas draw)

- [ ] **Step 1: Add the constant + state field.** Add `const ECONOMY_SCALE = 1000;` and `const MARKET_COLOR = '#d98c3a';` (a warm hue inside the muted palette, distinct from `TRADER_COLOR`). Extend `MinimalMapRendererState` with `markets?: readonly MarketLocationDto[]` (the array from `EconomyOverlayState.markets.values()`).

- [ ] **Step 2: Write the failing test** for an EXPORTED pure helper (canvas-free). The renderer exposes only `renderMinimalMap`; `drawScene`/`drawBuilding` are private and need a full ctx — so extract and `export` a pure function and test that:

```ts
// tests/render/economyMarkets.test.ts
import { visibleMarketGlyphs } from "../../src/render/minimalMapRenderer";
// visibleMarketGlyphs(markets, visibleGrid) -> markets whose tile is inside visibleGrid
it("keeps only markets whose tile is within the visible grid rect", () => {
  const markets = [{ marketId: 1, name: "A", tileX: 5, tileY: 5, wagePaidLastTick: 0 },
                   { marketId: 2, name: "B", tileX: 999, tileY: 999, wagePaidLastTick: 0 }];
  const grid = { minX: 0, minY: 0, maxX: 32, maxY: 32 };
  expect(visibleMarketGlyphs(markets, grid).map((m) => m.marketId)).toEqual([1]);
});
```

(Match `GridRect`'s real field names from `isCoordVisible`'s 2nd-arg type in `minimalMapRenderer.ts`; the helper just wraps `isCoordVisible({x:tileX,y:tileY}, grid)`.)

- [ ] **Step 3: Implement the helper + the draw.** Export the pure helper and add `drawEconomyMarkets(state, visibleGrid)`, calling it in `drawScene` between `drawRiverSurfaceLayer` (:159) and `drawRoads` (:181) — **mirroring the existing `drawEdgeConnections(state, visibleGrid)` call at :183** (`visibleGrid` is the local `const visibleGrid = visibleGridRect(state)` at :150, NOT a state field):

```ts
export function visibleMarketGlyphs(
  markets: readonly MarketLocationDto[] | undefined,
  visibleGrid: GridRect,
): MarketLocationDto[] {
  if (!markets) return [];
  return markets.filter((m) => isCoordVisible({ x: m.tileX, y: m.tileY }, visibleGrid));
}

function drawEconomyMarkets(state: MinimalMapRendererState, visibleGrid: GridRect): void {
  for (const m of visibleMarketGlyphs(state.markets, visibleGrid)) {
    drawMarketGlyph(state, { x: m.tileX, y: m.tileY }, MARKET_COLOR); // single-tile, world transform
  }
}
```

`drawMarketGlyph` mirrors `drawBuilding`'s single-tile rounded-rect path (same projection + LOD) but fills `MARKET_COLOR` and draws no roof — a flat one-tile marker, inside the existing world `ctx` transform (NOT the HUD transform).

- [ ] **Step 4: Run tests + typecheck:**
```bash
npm run typecheck && npx vitest run tests/render/economyMarkets
```
Expected: PASS.

- [ ] **Step 5: Commit:**
```bash
git add src/render/minimalMapRenderer.ts tests/render/economyMarkets.test.ts
git commit -m "feat(render): draw single-tile market glyphs (existing iso style)"
```

### Task C4: click-to-inspect — selection + read-only market panel

**Files:**
- Modify: `src/app/entitySelection.ts:19-69`
- Modify: `src/render/inspectorPanelPainter.ts:22-61`
- Test (selection): extend the EXISTING `tests/app/entitySelection.test.ts` (it has the `createEntitySelection` harness).
- Test (panel): create `tests/render/marketInspector.test.ts` — no painter harness exists; `drawInspectorPanel` needs a full ctx. Test a pure exported formatter instead.

- [ ] **Step 1: Write the failing selection test** in `tests/app/entitySelection.test.ts` (reuse `createEntitySelection`) — clicking near a market tile sets `selectedMarketCoord` and clears `selectedAgentId`/`selectedVehicleId`; clicking an agent clears `selectedMarketCoord`.

- [ ] **Step 2: Implement market selection.** Add `selectedMarketCoord: { x: number; y: number } | null` to the selection state, and a `findNearestMarket(worldPoint, markets, radius)` using the same projected-distance approach as `findNearestProjectedEntity` (project each market tile via `mapProject`). In `selectAtScreenPoint`, extend the mutual-exclusion ladder so exactly one of `{market, vehicle, agent}` is selected (market takes priority on a direct tile hit; otherwise fall through to vehicle/agent and null the market).

- [ ] **Step 3: Write the failing panel test** in `tests/render/marketInspector.test.ts` against a PURE exported formatter (no canvas): `marketInspectorRows(market, goods)` returns the title + one row string per good `"<GOOD>  p=<settlement/1000>  short=<unmet>  glut=<unsold>"` + a `"wages=<wage/1000>"` line. Assert the `/ECONOMY_SCALE` formatting (e.g. settlement `5000` → `"5.00"`).

- [ ] **Step 4: Implement the formatter + the panel draw.** In `inspectorPanelPainter.ts` add and `export` the pure `marketInspectorRows(market: MarketLocationDto, goods: MarketGoodDto[]): string[]` (the testable unit), plus `MARKET_INSPECTOR_PANEL: InspectorPanelTheme = { x: 12, y: 244, accent: '#f0a85a', stroke: 'rgba(240,168,90,0.8)' }` (non-overlapping with agent@12 / vehicle@128) and `drawMarketInspectorPanel(ctx, market, goods, theme, pixelRatio)` that reuses `inspectorPanelLayout` (:38) + the `setTransform(pixelRatio,…)` HUD idiom (:71) to draw `marketInspectorRows(...)`. Wire it into the render pass after the vehicle panel, drawn only when `selectedMarketCoord` is set and matches a known market.

- [ ] **Step 5: Run tests + typecheck:**
```bash
npm run typecheck && npx vitest run tests/app/entitySelection tests/render/marketInspector
```
Expected: PASS.

- [ ] **Step 6: Commit:**
```bash
git add src/app/entitySelection.ts src/render/inspectorPanelPainter.ts tests/app/entitySelection.test.ts tests/render/marketInspector.test.ts
git commit -m "feat(client): read-only market inspector (selection + panel)"
```

### Task C5: wire economy state through main + render

**Files:**
- Modify: `src/main.ts:109-138,194-202`

- [ ] **Step 1: Hold + thread the state.** Add a module-level `let economyState = createEconomyOverlayState();`, wire `onEconomyState: (s) => { economyState = s; }` alongside `onMobilityState` in `startRuntime`, and pass `markets: [...economyState.markets.values()]` + the goods map into the renderer state assembled in `render()`/`frame()`. Expose `economyMarketCount` via the `installRuntimeDiagnostics` hook (for the browser-smoke).

- [ ] **Step 2: Typecheck + unit build:**
```bash
npm run typecheck && npx vitest run
```
Expected: PASS.

- [ ] **Step 3: Commit:**
```bash
git add src/main.ts
git commit -m "feat(client): thread economy overlay state into the render loop"
```

### Task C6: MANDATORY browser-smoke (frontend↔wire boundary)

**Files:**
- Create: `scripts/smoke-economy-markets.mjs`
- Modify: `package.json` (add `"smoke:economy-markets"` script)

- [ ] **Step 1: Write the smoke** by adapting `scripts/smoke-visible-traders.mjs` (decodes the binary wire). It must assert, against the real dev stack:
  1. at least one `economySnapshot` `ServerMessage` is received over the WS with ≥4 markets;
  2. after panning to a seeded market chunk, the `runtimeDiagnostics` `economyMarketCount` ≥ 1 (a glyph is in view);
  3. a synthetic click at a market tile opens the inspector (assert via a diagnostics flag `selectedMarketCoord != null` or a DOM/canvas probe).

- [ ] **Step 2: Add the npm script:** `"smoke:economy-markets": "node scripts/smoke-economy-markets.mjs"`.

- [ ] **Step 3: Run the full build + smoke** (build wrapper required per CLAUDE.md):
```bash
npm run build && npm run smoke:economy-markets
```
Expected: the smoke prints the received economy frame, a non-zero market glyph count, and an inspector-opened confirmation. **"unit tests pass" is NOT a substitute** — this smoke is the acceptance gate for the wire-crossing feature.

- [ ] **Step 4: Commit:**
```bash
git add scripts/smoke-economy-markets.mjs package.json
git commit -m "test(smoke): browser-smoke for on-map economy markets + inspector"
```

---

## Final verification (before finishing the branch)

Run the full local gate (per memory `run-full-ci-gate-before-push`), all cargo via the serial wrapper on the isolated target:

```bash
# Rust: fmt-check, clippy, scoped tests
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml -p sim-core -p sim-server -p abutown-protocol -- -D warnings
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core -p sim-server -p abutown-protocol
# Frontend: typecheck (src+tests+scripts), vitest, build, the economy smoke, e2e render-smoke
npm run typecheck && npx vitest run && npm run build && npm run smoke:economy-markets && npm run test:e2e
```

All green → use **superpowers:finishing-a-development-branch** to deliver via PR. Note in the PR body the one-time `DELETE FROM economy_snapshots;` deploy step.

---

## Self-review

**1. Spec coverage.** A (authored markets.json + pool-factory + byte-identical + one-time DELETE) → A1-A5. B (new ServerMessage + RuntimeReadView field + send) → B1-B3. C (single-tile glyph + read-only inspector + mandatory browser-smoke) → C1-C6. 14 invariants → enforced by A3 equality test + A5 permanent invariant tests + the #78 SFC audit + the B2/round-trip tests. ECONOMY_SCALE display → C3/C4. Observed-gating → C3 `isCoordVisible`. Read-only → no ClientMessage added. ✓

**2. Deviation from spec (flagged honestly) + one correction.** Deviation: the spec said "dirty-only deltas via DirtyMarketGoods"; the plan sends a full thin `EconomySnapshot` each tick instead, because the data is tiny (~4 markets) and this avoids cross-schedule dirty plumbing while still being a "global thin snapshot." Dirty-only + per-chunk are documented YAGNI escape hatches. Correction (not a deviation): the new oneof tag is **7**, not 6 — `ServerError error=6` already occupies 6 (`abutown.proto:61`). Both noted in B1/B3.

**3. Placeholder scan.** Numbered factory blocks in A3 carry exact field values; `graph.node().position` is now stated as the settled fact `(f32,f32)` (B2). The remaining "mirror the existing `onMobilityState` wiring" (C2/C3 dispatch + C5) references an established in-repo pattern with anchors rather than inventing a signature — the implementer reads that anchor. No "TBD"/"add error handling"/"similar to Task N".

**5. Adversarial review pass.** This plan was reviewed against the real code by three lenses (signature accuracy, spec/invariants, executability). Verdict: ready-after-fixes; all 3 blockers + 3 important + minors fixed inline — the `applyServerMessage` non-exhaustive-switch break (C1 Step 2), the non-existent `build_test_world_with_routing` → `seed_world()`/`unseeded_world()` in `economy/tests/seed.rs` (A3), the non-existent `state.visibleGrid` → `drawEconomyMarkets(state, visibleGrid)` (C3), the `bundle` vs `base_world` binding per seed site (A4), the real frontend test-file paths + canvas-free pure helpers (C3/C4), `graph.node().position` as fact (B2), and the `seed_world()` rewire on legacy-seed deletion (A5). Verified-solid (do not re-touch): proto tag-7 wiring, all markets.json values byte-faithful to seed.rs, opening-price byte-identity, `EconomyPersistSnapshot: PartialEq`, read-only invariant, render insertion point.

**4. Type consistency.** `MarketLayer`/`MarketSpec`/… (A1) are used verbatim by `seed_from_markets_layer` (A3) and `bundle.markets` (A4). `EconomySnapshot`/`EconomyMarket`/`EconomyMarketGood` proto names (B1) match `build_economy_snapshot` (B2), the send sites (B3), and the TS `economySnapshotFromProto` (C1). `EconomyOverlayState.markets/goods` (C2) match `drawEconomyMarkets` (C3) and the inspector (C4). `selectedMarketCoord` consistent across C4/C5. ✓
