# Make the Abutopia Economy↔Demographics Merge Visible — Design (Blocker-1)

Date: 2026-06-07

## Status

Approved from brainstorm. This is Blocker-1 from
`docs/superpowers/plans/2026-06-06-slice2b-followups.md`: Slices 1+2+2b are
mechanically correct but produce `economy::liveness routed = 0` in the live
abutopia world, so the per-capita citizen density is invisible on the map.

Settled decisions (brainstorm):
1. **Approach A — re-anchor world data** (not a binding-rule code change, not
   adding demand at a distant market). Only `markets.json` changes.
2. **Shop-only** — one consumption market is co-located with the residential
   corridor; the wage/commute channel is out of scope.
3. **Acceptance = in-repo proof** — a deterministic backend test (+ hermetic e2e
   browser-smoke + full CI). The live remote-stack screenshot stays deferred
   (still gated by Blocker-3's persistence-stale health gate).

**Base branch.** `feat/abutopia-economy-visible` off `origin/main` (`06d3828`,
Slice 2b).

## Goal

Make the abutopia map visibly show citizens participating in the economy: the
300 residential pedestrians bind their `home_market` to a consumption market
co-located in their own chunk, so the attribution shop channel routes them
(`routed > 0`, scaled by `CapitaFactor`) and they cluster at / head to that
market right where they live. Prove it deterministically in-repo.

## Background — verified current state (`06d3828`)

Confirmed by reading code + data:

- **People:** all 300 pedestrians spawn on one corridor, `corridor:sidewalk:south`,
  tiles x≈106–117, y≈64.51 → **chunk (3,2)** (`spawns.json` 300 agents;
  `transport.json` corridor points). `corridor:sidewalk:north` (chunk 3,1) has 0
  agents.
- **Markets:** 9001 "Demo A" (2,3) chunk(0,0) = supply+extractor; **9002 "Demo B"
  (13,3) chunk(0,0) = demand/consumption** (goods 4 & 1, actors 8002/8012); 9003
  "Flow Demo A" (16,48) chunk(0,1) = supply+extractor; 9004 "Flow Demo B"
  (208,48) chunk(6,1) = demand (good 1, actor 8022).
- **Binding rule** (`mobility/market_binding.rs`): `home_market` = nearest market
  by Euclidean distance over market **node** positions (tie-break lower id),
  `work_market` = second-nearest. Computed once at spawn when `home_market==0`,
  then **persisted** in `AgentRecord` and **preserved** on restore/birth — never
  recomputed.
- **Why routed=0:** from the corridor, nearest is 9003 (supply, no consumption →
  shop channel reads `consumed_qty=0`); second-nearest is 9004 (demand-only, no
  seller → `WageTelemetry=0`). The consumption market 9002 has zero bound
  citizens. Both attribution channels route nobody.
- **Attribution** (`economy/attribution.rs`): shop channel keys on `home_market`,
  gated by `MarketGoodState.consumed_qty_last_tick > 0`, count =
  `min(consumed/shoppers_per_unit, max_shoppers_per_market × CapitaFactor,
  candidates)`; a market is "observed" only if its node sits in an Active/Hot
  chunk. `routed = CitizenEconomicTargets.0.len()`.
- **Transport:** `transport_cost_per_tile_unit = Money(5)`; delivered price =
  reference + 5 × distance(tiles) per unit.

## The Change

A single world-data edit in `data/worlds/abutopia/layers/markets.json`:

- Re-anchor market **9002** from `[13.0, 3.0]` to the residential corridor,
  recommended `[111.5, 64.5]` (chunk (3,2)). The exact anchor is pinned by the
  binding test and MUST satisfy:
  1. Snaps (via `NodeSpatialIndex::nearest`) to a sidewalk-south footway node in
     chunk (3,2), distinct from the nodes the other three markets snap to.
  2. Is unambiguously the nearest market to every pedestrian on the corridor
     (x≈106–117, y≈64.51) — trivially true vs. the other markets that are
     thousands of tile² away.

No other field changes: `distances` still declares the 9001↔9002 pair (its value
auto-recomputes from the new positions); `opening_prices` for 9002 key on market
id, not position; `household`/`supply`/`demand`/`extractors` unchanged.

## Behavior (data flow)

1. **Bind:** `assign_binding` at every corridor spawn position → `home_market =
   9002` (now nearest); `work_market` = the distant second-nearest (≈9003),
   unused here.
2. **Supply→demand:** on a fresh seed, 9001 (chunk 0,0) supplies 9002 (chunk 3,2)
   over the ~171-tile edge → +855/unit transport → delivered ≈ 1855 < consumer
   `max_price` 2000 → `consumed_qty_last_tick > 0` at 9002. (171 < the existing,
   working 192-tile 9003→9004 leg, so a demonstrated-feasible distance.)
3. **Observe + attribute:** when the client views the pedestrians, chunk (3,2) is
   Active/Hot → 9002 is observed → shop channel routes
   `min(consumed/3, 4×CapitaFactor, 300)` shoppers (≈120 at factor 30) onto 9002's
   node — citizens visibly head to / cluster at the market in their neighborhood.

## Components / units touched

- **`data/worlds/abutopia/layers/markets.json`** — the 9002 anchor (the whole
  behavior change).
- **Backend test(s)** in `backend/crates/sim-core/src/economy/tests/` (and/or
  `mobility/`) — see Testing.
- **Any position-specific assertion on 9002** (e.g. in `economy/tests/seed.rs`,
  `mobility` corridor/world tests, or `abutopia_bundle` tests) must be updated to
  the new anchor. Corridor assertions are unaffected (corridors don't move). The
  flow-demo ≥2-chunk constraint applies to 9003/9004 only (unchanged).

## Testing (acceptance = in-repo proof)

1. **Binding test (crux):** load the real abutopia bundle
   (`BaseWorldBundle::load_from_dir`), build the graph + `NodeSpatialIndex`, derive
   market node positions, and call `assign_binding` at the corridor midpoint (and
   both corridor endpoints) → assert `home_market == 9002`. This proves the data
   change fixes the root cause using real authored world data.
2. **Routed>0 integration test:** build the abutopia world (graph + economy +
   corridor pedestrians), mark chunk (3,2) `ActiveChunk`, run the
   consume→attribution path for a few ticks on a fresh seed, then assert
   `CitizenEconomicTargets` is non-empty and its targets resolve to 9002's node.
   (If constructing the full world graph in a unit test proves too heavy, an
   acceptable equivalent is: seed economy from the bundle, spawn citizens whose
   binding is computed from the real corridor positions against the moved anchor,
   then exercise `run_citizen_attribution_system` — the point is to prove the real
   data yields a non-empty routed set, not to re-test the already-covered
   attribution mechanics.)
3. **Existing suites stay green** — `economy/tests/seed.rs` (4 markets, distance
   pairs), capita density/safety tests, attribution tests; update only assertions
   that hard-coded 9002's old position.
4. **Mandatory browser-smoke** via the hermetic `e2e_server` (no remote DB): the
   render smoke's 300-pin holds (agent count is unaffected by a market move); the
   economy glyph for 9002 now renders in chunk (3,2). Confirm no render/coordinate
   regression.
5. **Full CI gate** — Rust workspace (fmt-check / clippy / test), frontend
   (typecheck / vitest / build), e2e.

## Determinism, persistence, performance

- **Determinism:** pure data move; `assign_binding` is a deterministic function of
  positions (no RNG/wall-clock). Binding tie-breaks by lower id.
- **Persistence / deploy:** market positions are economy STATE (in
  `economy_snapshots` as `MarketSite.node_id`) and citizen bindings are mobility
  STATE (in `mobility_snapshots`); both are preserved on hydrate because the seed
  is idempotent. So the moved anchor only takes effect on a fresh seed. Deploy
  step: a one-time **`DELETE FROM economy_snapshots` + `DELETE FROM
  mobility_snapshots`** for the abutopia world. This full fresh seed
  simultaneously resolves **Blocker-2** (the degenerate price-ceiling economy
  resets to opening prices). No schema bump, no new snapshot field
  (`schema_version` stays 2).
- **Performance:** unchanged — same market/agent counts; only one anchor differs.

## Acceptance Criteria

- `markets.json` re-anchors 9002 into chunk (3,2) on the residential corridor.
- The binding test proves corridor pedestrians bind `home_market == 9002` from
  real bundle data.
- The routed>0 test proves a non-empty `CitizenEconomicTargets` (targets → 9002's
  node) for the observed corridor chunk on a fresh seed.
- Existing suites stay green (position-specific 9002 assertions updated).
- Full CI green incl. the hermetic browser-smoke.

## Risks & Mitigations

- **Anchor snaps to a colliding / wrong-chunk node:** the binding test pins the
  exact anchor and asserts the resulting `home_market == 9002`; if a candidate
  anchor collides with another market's node (seed no-ops) or lands outside chunk
  (3,2), pick another corridor-adjacent point. Mitigated by the test.
- **Long transport leg prices 9002 out (re-creating Blocker-2):** the leg (171
  tiles, delivered ≈1855) is below `max_price` 2000 and shorter than the working
  192-tile flow-demo leg; the routed>0 test runs on a fresh seed in early ticks
  (before any long-run tâtonnement drift), so it confirms consumption realizes.
  Long-run price divergence is Blocker-2 (separate, deferred).
- **Coordinate-boundary regression:** moving the glyph touches render coords →
  mandatory browser-smoke (hermetic `e2e_server`) catches any render break; the
  300-pin agent count is independent of market position.
- **Deploy without the wipe shows no effect:** documented above — the fix needs
  both snapshot DELETEs (full fresh seed); ship-time runbook note.

## Deferred

- Wage/commute channel (co-locating a workplace/supply market for `work_market`).
- The live remote-stack screenshot (Blocker-3 persistence-stale gate).
- Economic-role-aware binding rule (Blocker-1 Option C).
