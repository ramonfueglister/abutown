# Schematic Map Renderer — Design

**Date:** 2026-06-10
**Status:** Approved (brainstormed with visual companion; style/zoom/scope decisions made interactively)
**Related:** `2026-05-16-visible-backend-mobility-design.md` (mobility wire), `2026-05-29-abutopia-minimal-world-design.md` (world model), PR #79 (on-map economy view), `docs/superpowers/plans/2026-05-17-chunk-lod-mobility.md` (future aggregation)

## Problem

The current minimal map renders a flat green field with near-invisible glyphs. The
simulation underneath (markets, tâtonnement prices, flow traders, aging citizens) is
rich but visually mute. We want a 2026-grade look that stays fast and — above all —
readable.

## Decisions (made during brainstorming)

1. **Art direction: schematic** (Mini-Metro school, chosen over flat-with-depth and
   pixel/Urbek). Muted paper ground, roads as thick rounded lines, buildings as icon
   shapes, agents as bold dots, economy flows as lines. Readability *is* the style.
2. **All four data channels** are first-class: economy flows, market states, agent
   states, network/buildings.
3. **Semantic zoom** (chosen over uniform info density): zoomed out the map reads as
   an economy map (market nodes, flow lines, agent density shimmer); zoomed in as a
   city map (individual agents with states, market detail). Continuous opacity
   blending, no popping.
4. **Entity model unchanged**: dynamic things (agents, vehicles, markets) are
   backend-tracked with stable ids; static things (roads, buildings, terrain) remain
   authored world layers. Nothing drawn is invented by the renderer.
5. **Approach: evolutionary Canvas2D refactor** (chosen over WebGL rewrite). WebGL is
   explicitly deferred until the zoomed-in view must show tens of thousands of
   individuals. Two-stacked-canvas split is a documented escalation step, built only
   on profiling evidence.

## 1. Visual vocabulary

Single source of truth: a new `src/render/designTokens.ts` (palette, stroke widths,
glyph geometry, zoom bands). No hex constants anywhere else in the renderer.

| Element | Encoding |
|---|---|
| Ground | muted paper (`#e9ede1`-family), not saturated green |
| Roads | thick dark rounded lines, dashed center in ground color |
| Buildings | rounded icon shapes, one silhouette per type (residential/commercial/civic/industrial) |
| Water / park | quiet desaturated fields |
| Agent walking | filled ink dot |
| Agent waiting / at activity | ring (stroked, unfilled) |
| Trader | red dot, slightly larger |
| Agents zoomed out | density shimmer (low-opacity dots aggregated from real positions) |
| Market node | orange disc; **radius = trade activity (EWMA of traded qty)**; **ring fill = satisfied-demand share**; settlement pulse; price-trend arrow (▲▼ from tâtonnement direction) |
| Flow line | curve between market anchors; **width = flow rate**, chevron = direction, **color = good** (RAW orange, FOOD green, TOOLS violet) |
| Selection | thin halo ring (existing click-inspector) |

**Zoom bands** (camera scale today runs 0.18–2.8):
- **Economy band** (scale < 0.6): flows + market nodes at full strength, agents as shimmer.
- **Transition band** (0.6–1.0): linear opacity cross-fade.
- **City band** (> 1.0): individual agents at full strength, flows faded to a hint.

All transitions are continuous opacity ramps computed by a pure function (see §3).

## 2. Data flow & wire

**Already on the wire — no changes needed:**
- L0 network/buildings: authored world layers (terrain, transport, buildings, decorations).
- L1 market nodes: `EconomySnapshot.markets` + `EconomySnapshot.goods` already carry
  `last_settlement_price`, `ewma_reference_price`, `traded_qty_last_tick`,
  `unmet_demand_last_tick`, `unsold_supply_last_tick`. Node size, ring fill and trend
  arrow are pure frontend derivations.
- L2 agents: mobility chunk snapshots/deltas carry `world_coord`, `direction`,
  `state`. Glyph states map directly onto the existing state enum. Existing frame
  interpolation (`InterpolatedEntry`) is untouched.

**One additive wire extension (L3 flows):**

```proto
message EconomySnapshot {
  // ...existing fields 1..5...
  repeated EconomyFlow flows = 6;
}

message EconomyFlow {
  uint32 src_market_id = 1;
  uint32 dst_market_id = 2;
  uint32 good_id = 3;
  int64 rate = 4; // ECONOMY_SCALE-scaled, EWMA-smoothed
}
```

- Rate is aggregated **at runtime, in memory** from the macro-flow state and **never
  persisted**. Rationale: every persisted economy-snapshot field so far has cost us a
  one-time `DELETE FROM economy_snapshots` before deploy. After a restart the EWMA
  reconverges within seconds — harmless under the frozen-time persistence model.
- proto3-additive: old clients ignore the field.

**ECS fidelity:**
- Trader dots "on" flow lines are **not an animation** — they are the real trader
  agents at their real `world_coord`. The line is drawn under them.
- The zoomed-out density shimmer is aggregated client-side from real agent positions
  of subscribed chunks (Abutopia has 28 chunks; all subscribable at this world size).
  When the chunk-LOD plan lands FlowCell aggregates, they replace the same interface.

## 3. Components

**New frontend modules:**
- `src/render/designTokens.ts` — the §1 vocabulary as data.
- `src/render/layerBlend.ts` — pure `scale → {opacity, detail}` per layer, where
  `opacity ∈ [0,1]` and `detail ∈ {'aggregate', 'individual'}` selects between the
  zoomed-out and zoomed-in drawing mode of a layer (e.g. shimmer vs. dots for L2,
  chevrons on/off for L3). No canvas dependency; trivially unit-testable.
- `src/render/drawNetwork.ts` (L0), `drawMarkets.ts` (L1), `drawAgents.ts` (L2),
  `drawFlows.ts` (L3) — one drawer per layer, identical signature
  `(ctx, camera, blend, data)`.
- `minimalMapRenderer.ts` shrinks to an orchestrator: culling, projection, draw
  order, calls the four drawers.

**Unchanged:** `minimalMapProjection` (18 px tiles), `viewportChunks`,
`mobilityState` + interpolation, click-inspector (markets already inspectable since
PR #79). `drawOrder.ts` gains only the kinds `flow` and `marketNode`.

**Backend (small):** one aggregation function in the economy crate (macro-flow state
→ `EconomyFlow` list, in-memory EWMA), encoding in the protocol crate, proto regen
via `scripts/generate-proto-ts.mjs`.

**Settlement pulse:** pure render animation on the rAF wall clock, triggered by a
change in `traded_qty_last_tick`. No sim impact, no new wire data, determinism
untouched.

## 4. Errors, tests, performance, rollout

**Error handling — no fallback cruft, no special-case code:**
- Missing `flows` field (older server): proto3 yields an empty list; L3 draws nothing.
- Flow rate ≤ 0: not drawn (no fake minimum width).
- Market without goods data: base-size node, no invented state.
- `layerBlend` hard-clamps to [0, 1].

**Tests:**
- Unit (vitest): `layerBlend` (bands, monotonicity, clamps); state→glyph mapping;
  rate→width and activity→radius; flow-curve anchors at market tiles through the real
  projection (the Phase-7a coordinate lesson).
- Backend: aggregation unit test with a conservation property (sum of reported flows
  equals macro-flow state, in the spirit of the #78 audit); proto roundtrip.
- **Browser smoke (mandatory per CLAUDE.md — the feature crosses the wire):** script
  modeled on `scripts/smoke-7a.mjs` against the dev stack. Asserts: (1) an
  `EconomySnapshot` containing `flows` arrives, (2) zoomed out (~0.3) flow-line
  pixels exist at the expected curve position, (3) zoomed in (~1.5) individual agent
  dots exist. "All tests pass" is not acceptance; the smoke is.
- Existing render-smoke with pinned agent counts stays in the CI gate.

**Performance:** budget 60 fps at today's 300 agents, designed for ~10,000 visible
dots zoomed in (Canvas2D with batching). A simple frame-time counter joins the
existing runtime diagnostics. Escalation step (only on profiling evidence): split
into a base canvas (network, redrawn on camera change) and a dynamic canvas (agents/
flows, every frame).

**Rollout:** purely additive — no DB migration, no `DELETE FROM`, frontend deployed
as usual (build locally, deploy static `dist/`).

## Out of scope / future

- WebGL/PixiJS renderer (revisit when zoomed-in individual counts demand it).
- Chunk-LOD FlowCell aggregates as shimmer source (existing plan; interface-compatible).
- Day/night or weather tinting, sound, minimap.
- New simulation content (more markets, buildings, goods) — the renderer reads what
  exists; world enrichment is its own track.
