# On-Map Economy View — Design Spec

**Date:** 2026-06-05
**Status:** Design (approved in brainstorming; pending spec review → writing-plans)
**Branch:** `plan/economy-onmap-view` (off `origin/main` `d5dfffa` = #78 SFC audit)

## Goal

Make the mean-field economy **readable on the one canonical map**, in the
**existing graphic style**, by (1) promoting the four code-seeded markets into
**first-class authored world data**, (2) rendering each market as a **single
visible tile glyph**, and (3) adding a **read-only click-inspector** showing
per-good prices, scarcity, and wages. Purely additive: **the economy's dynamics
do not change.**

## Context & the four locked decisions

The world is already a single canonical instance ("abutopia", 224×128 tiles,
32-tile chunks, authored from `data/worlds/abutopia/manifest.json` + five JSON
layers). The "demo" is not a second world — it is the **economy**: today
`seed_demo_economy` (`backend/crates/sim-core/src/economy/seed.rs`) mints four
markets in Rust at runtime, snapped to footway nodes, and **renders nothing**.
Zero economy state crosses the wire (`ServerMessage` oneof has no economy case).

Product-owner decisions taken in brainstorming:

1. **Market = exactly one tile** (point feature), rendered as a subtle glyph.
2. **Perspective & graphic style unchanged** — reuse the existing iso projection
   and vector primitives (recolor/reuse), no new art direction.
3. **Overlay = Minimal + click-inspector** — no ambient heatmap, no flow lines;
   click a market tile → a read-only panel with the numbers.
4. **Markets become authored data** (`markets.json` layer + pool-factory), a
   clean cutover (no compat shim, per the NO-FALLBACK rule).
5. **Read-only** — no client→server commands; the client is a pure observer.

## Spec-congruence verdict

An adversarial check against the committed specs (macro-flow, flow-traders,
shoppers, visible-traders, self-sustaining-loop, food-self-sufficiency, free-prices,
SFC-audit, persistence) returned **congruent, zero contradictions**:

- **Mean-field authority preserved.** Markets-as-glyphs + inspector + the
  existing shopper/commuter/flow-trader walkers add **no per-agent economic
  simulation**. The macro layer stays the sole economic authority; the visible
  agents remain "additive rendering only … ephemeral, not persisted"
  (flow-traders/shoppers specs).
- **Server-authoritative / read-only client.** All price discovery, balance, and
  flow run backend-side; the client only renders (visible-traders §C).
- **Observed-only rendering.** A glyph renders only while its chunk is Active/Hot,
  matching the existing dormancy gating (`systems.rs` `refresh_dormant_markets_system`,
  `observed_markets`).
- **Inspector fields already exist** as persisted `MarketGoodState` fields — the
  inspector is pure UI, no new backend compute.

The authored-markets cutover and the new wire case are **new decisions the specs
are silent on (not violations)** — and are safe **only if the pool-factory
reproduces the `seed.rs` invariants byte-for-byte** (§Invariants).

## Architecture

A pure, read-only **projection layer** over an unchanged economy. Three sub-slices:

- **A — Authored markets + pool-factory** (backend, data): markets sourced from
  world data; the economy is reconstructed bit-identically from it.
- **B — Economy on the wire** (backend, protocol): a new additive `ServerMessage`
  case replicates market locations + per-(market,good) state to the client.
- **C — Frontend glyph + inspector + browser-smoke** (frontend): draw the
  single-tile glyph and the read-only inspector; prove it over the wire.

Each sub-slice is independently testable; A produces a green economy with no
behavioural change, B adds data on the wire with no UI, C adds the visuals.

---

## A — Authored markets + pool-factory

### A.1 `markets.json` world layer

Add a sixth layer `markets.json` to `data/worlds/abutopia/manifest.json` and bump
the **world-bundle `schema_version` 1 → 2**. Write a market-layer loader mirroring
the existing spawn-layer loader (`BaseWorldBundle`). This is **world data
versioning** (version-controlled JSON), **not** a DB migration.

The layer authors the **market topology as data**. Proposed schema (final field
set pinned in the plan against `seed.rs`):

```jsonc
{
  "schema_version": 2,
  "markets": [
    { "id": 9001, "name": "Demo A",      "anchor": [2, 3] },
    { "id": 9002, "name": "Demo B",      "anchor": [13, 3] },
    { "id": 9003, "name": "Flow Demo A", "anchor": [16, 48] },
    { "id": 9004, "name": "Flow Demo B", "anchor": [208, 48] }
  ],
  "distances": [ [9001, 9002], [9003, 9004] ],   // directed both ways; NO diagonal
  "supply":   [ { "actor": 8001, "market": 9001, "good": "TOOLS", "qty": 10, "min_price": 500 }, … ],
  "demand":   [ { "actor": 8002, "market": 9002, "good": "TOOLS", "qty": 10, "max_price": 2000,
                  "mpc_bps": 8000, "autonomous": 5000 }, … ],
  "extractors": [ { "actor": 8031, "market": 9001, "in": "RAW", "out": "TOOLS", "qty": 10 }, … ],
  "household": { "population": 1000000 },         // pool_weights derived = equal over demand[]
  "opening_prices": [ { "market": 9002, "good": "TOOLS", "price": 1000 }, … ]
}
```

`anchor` is a **reference point**, not a baked node: the factory snaps it to the
nearest footway node exactly as today (`NodeSpatialIndex.nearest`), so resolved
nodes are identical. `pool_weights` is **derived** (equal weight 1 over every
authored consumer pool) so it cannot drift from the demand list.

### A.2 Pool-factory

Replace `seed_demo_economy` with `seed_from_markets_layer(world, &MarketsLayer)`:
the current seeding logic, parameterised by the authored data instead of
hardcoded `REF_*`/ids/quantities. It runs on the same fresh-world trigger
(`Markets` empty), snaps anchors → nodes, computes `MarketChunks` via
`mobility::chunk_of(.., 32)`, bakes `MarketDistances` via `manhattan_tiles`, and
inserts every supply/demand/extractor pool, the `HouseholdSector`, and opening
`MarketGoodState` prices — all from data. The snapping and distance baking stay
in Rust (they need `Graph` + `NodeSpatialIndex`, available after `RoutingPlugin`).

### A.3 Persistence stance (the cost-saving finding)

`EconomyPersistSnapshot` (`economy/persist.rs`) **already** round-trips
`MarketSite`, `MarketChunks`, `MarketDistances`, `RawDeposits`, and the pools.
Because the factory produces **byte-identical data**, **no economy-snapshot
schema bump and no forced DELETE** is required — only the *source* of fresh-world
seeding moves from `seed.rs` to the world loader.

We nonetheless perform **exactly one lossless re-seed**: a single
`DELETE FROM economy_snapshots` so the authored loader is the actual
source-of-truth path that runs on the canonical world (and is validated there).
Since the data is identical, this changes **nothing** in the economy. **No
serde-default shim, no heal-on-restore guard** (NO-FALLBACK rule). A
byte-identical round-trip test (§Testing) guards this.

---

## B — Economy on the wire

### B.1 Protocol

Add a new `ServerMessage` oneof **case tag 7** (`EconomySnapshot`, **global**
not per-chunk — see B.3; the plan ships one thin snapshot on connect + per tick
rather than a separate delta) in
`backend/crates/protocol/proto/abutown.proto` (tags 1–6 are
taken: Hello, TilePulse, mobility delta, mobility snapshot, WorldEvent, ServerError).
Bump `PROTOCOL_VERSION`. Populate from a **new economy field on `RuntimeReadView`**
(`backend/crates/sim-server/src/runtime_view.rs`, currently mobility-only).

### B.2 Payload

- **Market locations** (sent once at snapshot/hello, rarely change): `id`, `name`,
  `tile {x, y}` (derived from the market's footway-node position).
- **Per-(market, good) state**: `last_settlement_price`, `ewma_reference_price`,
  `unmet_demand_last_tick`, `unsold_supply_last_tick`, `traded_qty_last_tick`,
  and wage paid (from `WageTelemetry`). All are existing computed/persisted fields.

Money/price values ship as **raw `i64`** (no conversion server-side).

### B.3 Cadence & scope

- **Global thin snapshot** at hello + **dirty-only deltas** keyed off the existing
  `DirtyMarketGoods` set (free trigger; only changed `(market, good)` entries are
  sent). Markets are a handful, so global is justified and far simpler than
  per-chunk partitioning. **Per-chunk replication is the documented scale-out path**
  if markets ever multiply densely (YAGNI now — call it out, don't build it).
- Rendering is still **observed-gated on the client** (viewport tile cull), so
  off-screen markets cost zero render even though their tiny data is present.

---

## C — Frontend glyph + inspector + browser-smoke

### C.1 Decode & state

Extend `applyServerMessage` in `src/backend/mobilityState.ts` with an economy
case + reducer, into a dedicated `EconomyOverlayState` (`markets: Map<id, {tile,
name}>`, `goods: Map<"market:good", MarketGoodView>`). Mirror the mobility
plumbing in `src/main.ts`: HTTP snapshot at boot, then a live economy callback
feeding `state.economyState` into `frame()`.

### C.2 Market glyph (single tile, existing style)

In `src/render/minimalMapRenderer.ts` `drawScene`, insert `drawEconomyMarkets`
between infrastructure and actors. Each market is a **recolored single-tile
primitive** (a `drawBuilding`-style rounded-rect variant or a `detailRenderPolicy`
marker category) at the market's tile, using the existing iso projection
(`isoProjection.ts` / `minimalMapProjection.ts`) and `screenStableWorldSize` LOD.
**Viewport-culled** via the existing `isCoordVisible` path (= observed-gating).
No new art style, no animation.

### C.3 Read-only inspector

- **Selection:** add `findNearestMarket(worldPoint)` to `src/app/entitySelection.ts`
  and a `selectedMarketCoord: {x,y} | null`, **mutually exclusive** with
  `selectedAgentId`/`selectedVehicleId` (existing pattern). Markets keyed by
  `"x:y"` (tileKey idiom).
- **Panel:** `drawMarketInspectorPanel` following the `inspectorPanelPainter.ts`
  pattern, drawn in the fixed-screen HUD stack after the vehicle panel. Shows, per
  good: price (settlement + reference), unmet-demand/unsold-supply (Mangel/Glut),
  traded qty, wages paid. **Display divides by `ECONOMY_SCALE = 1000`** (the
  fixed-point→display contract; server ships raw `i64`).

### C.4 Coordinate-system care (the Phase-7a lesson)

Two conversions must be exactly right, or the feature is 100% broken while unit
tests pass: market node-pos → tile → iso (glyph placement) and screen → world →
tile → market (click selection). These are validated by the mandatory
browser-smoke, not unit tests alone.

---

## Invariants the pool-factory MUST reproduce byte-for-byte

Guarded by a byte-identical snapshot round-trip test + the #78 SFC audit.

1. **Market identity:** exactly four markets `9_001 9_002 9_003 9_004`.
2. **Anchoring/snap:** nearest footway node from REF `(2,3) (13,3) (16,48)
   (208,48)`; never an unreachable node.
3. **Chunk anchors:** `chunk_of(node.pos, 32)`.
4. **Cross-market topology:** `MarketDistances` contains **only** `(m_a↔m_b)` and
   `(m_fa↔m_fb)`, both directions — **no `m_a↔m_fb` diagonal**.
5. **Pool topology + actor ids:** TOOLS supplier `8_001@m_a` / consumer
   `8_002@m_b`; FOOD supplier `8_011@m_a` / consumer `8_012@m_b`; flow FOOD
   supplier `8_021@m_fa` / consumer `8_022@m_fb`.
6. **Extractors:** `8_031@m_a` (RAW→TOOLS), `8_032@m_a` (RAW→FOOD),
   `8_033@m_fa` (RAW→FOOD); each = RawDeposit faucet + 1:1 ProductionPool +
   SupplyPool, `interval_ticks=1`, qty 10. **RAW is non-tradable** — never on a
   pool/market.
7. **Sizing:** every supply/demand/extractor qty = 10; `min_price=500`,
   `max_price=2000`. Routing-aware faucet coverage holds (each consumer's
   reachable faucet rate ≥ its demand); the two FOOD extractors must **not** both
   sit at `m_a` (`8_022@m_fb` would lose its only reachable faucet).
8. **Opening prices (data, not fallback):** `(m_b,TOOLS) (m_b,FOOD) (m_fb,FOOD)`
   seeded with `ewma_reference_price` and `last_settlement_price = 1_000 > 0`;
   the live auction MUST overwrite them — never used as the settlement price.
9. **SFC household:** `HouseholdSector { population: 1_000_000, pool_weights }`
   with every consumer pool (`8_002 8_012 8_022`) at equal weight 1, built after
   all demand inserts; `HOUSEHOLD_SECTOR = u64::MAX-1` collides with nothing;
   extractors are firms, **not** in `pool_weights`.
10. **Flow-demo geometry:** `m_fa/m_fb` at chunks `(0,1)`/`(6,1)`, ≥2–3 chunks
    from the transit chunk `(3,1)` (flow-trader visibility rule).
11. **Determinism:** BTreeMap-keyed, fixed ids, no RNG/wall-clock in init.
12. **Conservation:** money via `deposit`/`transfer` only; total money + total
    goods byte-invariant across visibility transitions (#78 audit).
13. **Persistence round-trip:** authored data survives `extract → apply`
    byte-identically.
14. **Actor-id namespace:** authored economic actors stay in the `8_0xx` band
    (`8_001..8_033`); render-projection offsets stay reserved (flow `1<<32`,
    shoppers `2<<32`, household `u64::MAX-1`).

## Error handling (NO-FALLBACK)

- Malformed/missing `markets.json` → loud `Err` at world load, not a silent
  default world. Anchor that snaps to an unreachable node → loud error.
- Wire decode of an unknown economy field → error, not silent skip.
- No serde-default on new proto/snapshot fields; no heal-on-restore.
- The one-time snapshot DELETE is an explicit deploy step, not a runtime guard.

## Testing

- **Rust:** factory produces a snapshot **byte-identical** to today's
  `seed_demo_economy`; money + goods conservation across an Active→dormant→visible
  cycle; persistence round-trip; markets.json loader parse + loud-error cases.
- **Frontend (vitest):** economy reducer applies snapshot/delta; glyph placement
  at the correct tile; inspector renders divided-by-1000 values; selection
  mutual-exclusion with agents/vehicles.
- **Browser-smoke (MANDATORY — frontend↔wire boundary, per CLAUDE.md):** real
  headless chromium against the dev stack confirms the economy `ServerMessage`
  goes over the wire, a market glyph draws at its tile, and click → inspector
  panel shows live numbers. "All unit tests pass" is **not** accepted as a
  substitute. Adapt `scripts/smoke-7a.mjs`.

## Non-goals (explicitly out)

No interactivity / client→server commands · no ambient heatmap · no goods-flow
lines · no new art style or perspective · no per-agent buying simulation · no new
world instances · no per-chunk economy replication (scale-out path only).

## Deployment (one-time migration — required on the FIRST deploy past this branch)

This is a clean schema cutover with **no compatibility path** (intentional, per the
NO-FALLBACK rule). On the **first** deploy of this branch, two one-time operational
steps are required; **subsequent** deploys need nothing (the authored data is
unchanged and byte-identical to the old seed).

1. **`DELETE FROM economy_snapshots;` once.** Markets are now sourced from the
   authored `markets.json` layer via `seed_from_markets_layer`, which is **byte-identical**
   to the deleted legacy `seed_demo_economy`. A persisted economy snapshot from before
   this branch still carries the old code-seeded markets and would shadow the authored
   loader. Clearing it once lets the authored loader become source-of-truth on the
   canonical world. Because the data is identical, this is **lossless** — it changes
   nothing in the economy.
2. **Base-world bundle must be at `schema_version: 2`.** `SUPPORTED_SCHEMA_VERSION` is
   now `2` (the markets layer was added) and the client (`baseWorldClient.ts`) accepts
   **only** schema 2. Any persisted/served base-world record still at schema 1 fails to
   load **loudly**. The authored `data/worlds/abutopia/*` is already at schema 2; if a
   deployment caches/persists a base-world bundle elsewhere, regenerate it at schema 2.

Note: the additive `economy_snapshot` message rides oneof tag 7 and is fully backward-compatible (old clients ignore the unknown tag), so **no `PROTOCOL_VERSION` bump is required** and none is made — the backend stays at protocol version 1. (Observation, out of scope for this feature: the frontend command client hardcodes a different protocol-version value than the backend reports — a pre-existing mismatch on `origin/main`, not introduced here; worth a separate follow-up.)
