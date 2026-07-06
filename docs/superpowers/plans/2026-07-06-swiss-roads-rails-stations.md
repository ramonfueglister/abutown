# Swiss Roads, Rails & Stations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Terrain-graded road/rail corridors (embankments & cuts), VSS-style Swiss markings, real rail geometry (steel rails, sleepers, ballast, catenary masts), and SBB-Perronkante station platforms.

**Architecture:** A new deterministic grading stage mutates the DEM grid right after DEM load in `scripts/geo/bake-world.mjs` (Step 2), so buildings' `baseY`, the road graph, tree placement, and all three tile levels sample graded ground by construction. Rendering additions (markings, rail look, platforms) are pure builders next to `buildRoads` that drape onto the graded `groundYAt`. Three PRs: grading → markings → rail look + stations.

**Tech Stack:** Node bake scripts (`scripts/geo/`), three.js/WebGPU frontend (`src/diorama/`), vitest, protobuf-es world tiles.

**Spec:** `docs/superpowers/specs/2026-07-06-swiss-roads-rails-stations-design.md`

## Global Constraints

- NO FALLBACKS, honest errors (project rule): missing input file / dataset gap / uncovered station = hard error or loud logged absence — never a silent substitute source or default.
- Determinism: double-bake byte-identical (existing `encodeTile` golden pattern); all new iteration over sorted keys; coordinates quantized 0.01 m where persisted.
- One shared projection: `scripts/geo/lib/project.mjs` `ANCHOR = {lon: 8.7285, lat: 47.5069}`, x=east, z=south.
- ALL cargo via `scripts/cargo-serial.sh` — but NOTE: this plan touches NO Rust; do not run cargo at all.
- Browser smoke/captures mandatory before claiming a frontend-visible task complete (CLAUDE.md).
- Worktree: `/Users/ramonfuglister/Coding/abutown/.claude/worktrees/run-app`, branch `swiss-roads` (PR 1) then `swiss-markings`, `swiss-rail-stations` off fresh main. Gitignored inputs (`scratch/geo/*`, `data/winterthur/world/*`) are already present in this worktree.
- Numbers from spec (verbatim): road smoothing window 40 m, max grade 12 %, shoulder 1.5 m, blend 8 m; rail window 200 m, max grade 2.5 %, shoulder 2 m; burial budget max < 0.3 m, p99 < 0.15 m; Leitlinie 6 m dash / 6 m gap / 0.15 m wide; Randlinie 0.10 m inset 0.3 m; Fussgängerstreifen yellow 0.5 m bars / 0.5 m gaps; kerb +6 cm; gauge 1435 mm; rails 0.07 m wide, 0.12 m proud; sleepers 2.4 × 0.24 m at 0.6 m; masts ~50 m; P55 = 0.55 m; safety line 0.10 m inset 0.8 m.
- `tsconfig.json` includes only `src` — tests are not type-checked; keep test call signatures in sync with production by hand.

## File Structure (all three PRs)

- `scripts/geo/lib/grading.mjs` — NEW: corridor rasterization + profile smoothing over the DEM grid (pure, testable).
- `scripts/geo/lib/gradewidths.mjs` — NEW: bake-side lane-floor widths (mjs twin of `src/diorama/traffic/roadWidths.ts`).
- `scripts/geo/bake-world.mjs` — MODIFY: insert grading stage after DEM load; wire corridor mask into nature; log report + burial metric.
- `scripts/geo/lib/transform.mjs` — MODIFY: `transformRoads` carries `bridge` flag; NEW `transformCrossings`.
- `scripts/geo/fetch-winterthur.mjs` — MODIFY: Overpass query adds `node["highway"="crossing"]`.
- `scripts/geo/burial-metric.mjs` — NEW: CLI printing the §9 metric against a world bake.
- `src/diorama/ksw/geo/roadMarkings.ts` — NEW: Leitlinien/Randlinien/Fussgängerstreifen builders (pure geometry + mesh wrapper).
- `src/diorama/ksw/geo/roads.ts` — MODIFY: per-class surface colors, kerb strips; rail ribbons replaced in PR 3.
- `src/diorama/designTokens.ts` — MODIFY: marking colors + adjusted `roadYs` ladder.
- `scripts/geo/fetch-stations.mjs` — NEW: SBB Perronkante download (hard-fail, no fallback).
- `scripts/geo/bake-stations.mjs` — NEW: perronkante.geojson → `data/winterthur/stations.json` (committed).
- `src/diorama/ksw/geo/railLook.ts` — NEW: ballast trapezoid, twin steel rails, SleeperLayer (near-LOD), catenary masts.
- `src/diorama/ksw/geo/platforms.ts` — NEW: platform bodies at P55 + yellow safety line.
- `src/diorama/ksw/main.ts` — MODIFY: wire markings (PR 2), railLook + platforms (PR 3).
- Tests: `tests/geo/grading.test.ts`, `tests/geo/gradewidths-parity.test.ts`, `tests/geo/burial.test.ts`, `tests/diorama/roadMarkings.test.ts`, `tests/diorama/railLook.test.ts`, `tests/diorama/platforms.test.ts`.

---

# PR 1 — Terrain grading (branch `swiss-roads`)

### Task 1: Grading kernel — profile smoothing + corridor rasterization

**Files:**
- Create: `scripts/geo/lib/grading.mjs`
- Test: `tests/geo/grading.test.ts`

**Interfaces:**
- Consumes: DEM grid object from `parseAAIGrid` (fields `{ncols, nrows, cellsize, xll, yll, data: Float64Array}` — VERIFY exact field names in `scripts/geo/lib/dem.mjs` before coding and adapt; the grading mutates a copy of `data`).
- Produces:
  - `smoothProfile(samples: number[], stepM: number, windowM: number, maxGrade: number): number[]` — moving-average then two-pass grade clamp.
  - `gradeDem(dem, ways, opts) -> {report}` where `ways = [{pts: number[][], halfWidthM, blendM, windowM, maxGrade, kind: 'road'|'rail'}]`, `opts = {waterRings: number[][][], sampleHeightAt: (x,z)=>number}`. Mutates `dem` in place. Rails override roads (§4.3): pass roads first, rails second — the function processes its input in the given order with later ways overriding earlier blended cells inside their corridor.
  - `makeCorridorMask(ways) -> (x, z) => boolean` (point-in-corridor via the same distance math; used for tree clearing).
  - `report = {cellsChanged, waterSkippedCells, bridgeSites: [{x, z, kind}], originDeltaM}`.

**Algorithm (implement exactly):**
1. Per way: densify `pts` to ≤ 2 m steps; sample pre-grading heights; `smoothProfile` (moving average over `windowM`, then forward pass clamping rise to `maxGrade·step` and backward pass clamping fall — classic two-pass grade limiter).
2. Rasterize: bounding box of way + `halfWidthM + blendM`; for each DEM cell centre in the box compute nearest point on the densified centreline (distance `d`, profile height `h`); accumulate per-cell `(sumW, sumWH)` with `w = 1` if `d ≤ halfWidthM`, else `w = 1 − smoothstep((d − halfWidthM)/blendM)` (0 beyond). Roads accumulate into one layer; after all roads, cells with `sumW > 0` get `height = (sumWH + (1−min(sumW,1))·orig·…)` — concretely: `t = min(sumW, 1); height = t·(sumWH/sumW) + (1−t)·orig`.
3. Rails: same, but into a second accumulator applied AFTER roads (overriding: `t_rail` blends against the road-graded value).
4. Water: cells whose centre lies inside any `waterRings` polygon (ray cast) are never written; count them. A way whose corridor contains ≥ 3 consecutive skipped water cells → one `bridgeSites` entry at the first skipped cell (deduplicate per way).
5. `originDeltaM = |gradedHeightAt(0,0) − origHeightAt(0,0)|` in the report; caller gates.

- [ ] **Step 1:** Write failing tests in `tests/geo/grading.test.ts` (import from `../../scripts/geo/lib/grading.mjs`; vitest resolves .mjs):

```ts
import { describe, expect, it } from 'vitest';
import { smoothProfile, gradeDem, makeCorridorMask } from '../../scripts/geo/lib/grading.mjs';

function flatDem(n: number, cell: number, h: (x: number, z: number) => number) {
  // Minimal stand-in matching the real parseAAIGrid grid shape used by gradeDem.
  const data = new Float64Array(n * n);
  for (let j = 0; j < n; j++) for (let i = 0; i < n; i++) data[j * n + i] = h(i * cell, j * cell);
  return { ncols: n, nrows: n, cellsize: cell, xll: 0, yll: 0, data };
}

describe('smoothProfile', () => {
  it('clamps grade to the limit in both directions', () => {
    const raw = [0, 0, 10, 10]; // 10 m jump over one 2 m step = 500 %
    const out = smoothProfile(raw, 2, 4, 0.12);
    for (let i = 1; i < out.length; i++) expect(Math.abs(out[i] - out[i - 1])).toBeLessThanOrEqual(0.24 + 1e-9);
  });
  it('is deterministic', () => {
    const raw = [3, 1, 4, 1, 5, 9, 2, 6];
    expect(smoothProfile(raw, 2, 4, 0.12)).toEqual(smoothProfile(raw, 2, 4, 0.12));
  });
});

describe('gradeDem', () => {
  it('levels the corridor cross-slope and blends back over blendM', () => {
    const dem = flatDem(60, 1, (x, z) => z * 0.5); // 50 % cross-slope
    const way = { pts: [[5, 30], [55, 30]], halfWidthM: 4, blendM: 8, windowM: 40, maxGrade: 0.12, kind: 'road' };
    gradeDem(dem, [way], { waterRings: [] });
    const at = (x: number, z: number) => dem.data[Math.round(z) * 60 + Math.round(x)];
    expect(Math.abs(at(30, 28) - at(30, 32))).toBeLessThan(0.05); // level across corridor
    expect(at(30, 50)).toBeCloseTo(25, 5); // untouched far field
  });
  it('rail overrides road inside the rail corridor', () => {
    const dem = flatDem(60, 1, () => 0);
    const road = { pts: [[30, 5], [30, 55]], halfWidthM: 4, blendM: 8, windowM: 40, maxGrade: 0.12, kind: 'road' };
    const rail = { pts: [[5, 30], [55, 30]], halfWidthM: 3, blendM: 8, windowM: 200, maxGrade: 0.025, kind: 'rail' };
    // Pre-shape terrain so road profile != rail profile at the crossing.
    for (let j = 0; j < 60; j++) for (let i = 0; i < 60; i++) dem.data[j * 60 + i] = i * 0.1;
    gradeDem(dem, [road, rail], { waterRings: [] });
    const railMid = dem.data[30 * 60 + 30];
    const railRef = dem.data[30 * 60 + 20];
    expect(Math.abs(railMid - (railRef + 1.0))).toBeLessThan(0.3); // rail grade continuity wins at crossing
  });
  it('never touches water cells and reports the bridge site', () => {
    const dem = flatDem(60, 1, () => 7);
    const way = { pts: [[5, 30], [55, 30]], halfWidthM: 4, blendM: 8, windowM: 40, maxGrade: 0.12, kind: 'road' };
    const water = [[[20, 20], [40, 20], [40, 40], [20, 40]]]; // square river
    const { report } = { report: gradeDem(dem, [way], { waterRings: water }).report ?? gradeDem(dem, [way], { waterRings: water }) } as never;
    // NOTE to implementer: settle the return shape as { report } and update this test to a single call.
    expect(dem.data[30 * 60 + 30]).toBe(7); // water cell untouched
  });
  it('corridor mask covers the carriageway, not the far field', () => {
    const mask = makeCorridorMask([{ pts: [[0, 0], [100, 0]], halfWidthM: 5 }]);
    expect(mask(50, 2)).toBe(true);
    expect(mask(50, 30)).toBe(false);
  });
});
```

- [ ] **Step 2:** `npx vitest run tests/geo/grading.test.ts` → FAIL (module not found). Fix the water-test to the final single-call shape when writing the module.
- [ ] **Step 3:** Implement `scripts/geo/lib/grading.mjs` per the algorithm block (pure, no I/O; read the real DEM grid field names from `scripts/geo/lib/dem.mjs` FIRST and match them; if they differ from `{ncols,nrows,cellsize,xll,yll,data}`, adapt tests + module together).
- [ ] **Step 4:** `npx vitest run tests/geo/grading.test.ts` → PASS.
- [ ] **Step 5:** Commit `feat(geo): terrain grading kernel — profile smoothing + corridor rasterization`.

### Task 2: Bake-side lane-floor widths (parity with roadWidths.ts)

**Files:**
- Create: `scripts/geo/lib/gradewidths.mjs`
- Test: `tests/geo/gradewidths-parity.test.ts`

**Interfaces:**
- Consumes: `data/winterthur/trafficnet.json` (`{edges: [{id, from, to, laneCount, lanes}], lanes: [{id, edge, pts}]}`), road list `[{class, width, pts}]`.
- Produces: `laneFloorWidths(roads, trafficNetDoc): number[]` — EXACTLY the semantics of `correctRoadWidths` in `src/diorama/traffic/roadWidths.ts` (constants `LANE_W = 3.0`, `SHOULDER_M = 0.8`, `MATCH_DIST_M = 5.0`, parallel dot ≥ 0.5, 16 m hash cells). Cross-reference comments in BOTH files: changes must be mirrored.

- [ ] **Step 1:** Failing parity test:

```ts
import { describe, expect, it } from 'vitest';
import { laneFloorWidths } from '../../scripts/geo/lib/gradewidths.mjs';
import { correctRoadWidths } from '../../src/diorama/traffic/roadWidths';
import trafficNet from '../../data/winterthur/trafficnet.json';
import roadsJson from '../../data/winterthur/roads.json';

it('mjs twin matches the TS implementation on the real net (sampled)', () => {
  const roads = (roadsJson as { roads: { class: string; width: number; pts: number[][] }[] }).roads
    .filter((_, i) => i % 7 === 0); // ~300 roads, keeps the test fast
  const ts = correctRoadWidths(roads);
  const mjs = laneFloorWidths(roads, trafficNet as never);
  expect(mjs).toEqual(ts);
});
```

- [ ] **Step 2:** Run → FAIL (module missing). **Step 3:** Port `roadWidths.ts` line-by-line to `gradewidths.mjs` (same constants, same spatial hash) and add the mirror comments. **Step 4:** Run → PASS. **Step 5:** Commit `feat(geo): bake-side lane-floor widths (parity-tested twin of roadWidths.ts)`.

### Task 3: Bridge flags + water rings into the bake inputs

**Files:**
- Modify: `scripts/geo/lib/transform.mjs` (`transformRoads` gains `bridge`), `scripts/geo/lib/landuse.mjs` (export water rings)
- Test: extend `tests/geo/` (existing transform tests file if present, else `tests/geo/transform-bridge.test.ts`)

**Interfaces:**
- Produces: each `RoadPath` gains optional `bridge: true` when the OSM way has `tags.bridge` (any non-"no" value). `waterRingsFrom(landuse)` returns `number[][][]` (all kind-6 rings). Existing consumers ignore the extra field (additive).

- [ ] **Step 1:** Failing test: a synthetic OSM way with `tags: {highway: 'primary', bridge: 'yes'}` → transformed road has `bridge === true`; without the tag → property absent.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement (in `transformRoads`, after `roadStyle`: `if (way.tags.bridge && way.tags.bridge !== 'no') path.bridge = true;`; in landuse module export `const waterRings = (items) => items.filter((l) => l.kind === 6).map((l) => l.ring);`). **Step 4:** PASS. **Step 5:** Commit `feat(geo): carry OSM bridge flags + expose water rings for grading`.

### Task 4: Wire grading into bake-world.mjs (order: after DEM, before everything)

**Files:**
- Modify: `scripts/geo/bake-world.mjs`

**Interfaces:**
- Consumes: Task 1 `gradeDem`/`makeCorridorMask`, Task 2 `laneFloorWidths`, Task 3 water rings + bridge flags; existing stage objects (`dem`, `osmRoads`→roads/rails via `transformRoads`, landuse).
- Produces: graded DEM used by ALL later stages; corridor mask filters `nature.trees`; console report block; hard gates.

**Wiring (verbatim intent, adapt to actual local variable names in the file):**

```js
// Step 2b — terrain grading (spec 2026-07-06 §4). Order matters: buildings
// (baseY), road graph, trees and every tile level sample the graded DEM.
const { roads, rails } = transformRoads({ osmRoads, projector });
const floors = laneFloorWidths(roads, trafficNetDoc); // trafficNetDoc = JSON.parse(readFileSync('data/winterthur/trafficnet.json'))
const originBefore = dem.heightAt(0, 0);
const ways = [
  ...roads.map((r, i) => ({
    pts: r.pts, kind: 'road',
    halfWidthM: Math.max(r.width, floors[i]) / 2 + 1.5, // render/lane width + 1.5 m shoulder (§4.1, §4.4 lane coverage)
    blendM: 8, windowM: 40, maxGrade: 0.12,
    bridge: r.bridge === true,
  })),
  ...rails.map((r) => ({
    pts: r.pts, kind: 'rail',
    halfWidthM: (r.width + 2.2) / 2 + 2.0, // ballast bed + 2 m shoulder (§4.2)
    blendM: 8, windowM: 200, maxGrade: 0.025,
  })),
];
const report = gradeDem(dem, ways, { waterRings: waterRingsFrom(landuse) });
const originDelta = Math.abs(dem.heightAt(0, 0) - originBefore);
if (originDelta > 2) fail(`grading moved the anchor by ${originDelta.toFixed(2)} m (> 2 m gate)`);
console.log(`grading: ${report.cellsChanged} cells, ${report.waterSkippedCells} water cells skipped, ` +
  `${report.bridgeSites.length} bridge sites (untagged water/rail crossings), anchor Δ ${originDelta.toFixed(2)} m`);
const inCorridor = makeCorridorMask(ways);
// Later, where nature.trees is assembled:
const keptTrees = nature.trees.filter((t) => !inCorridor(t.x, t.z));
console.log(`corridor clearing: ${nature.trees.length - keptTrees.length} trees removed from carriageways`);
```

- [ ] **Step 1:** Read the actual bake-world.mjs stage code; insert grading directly after the DEM stage; ensure `transformRoads` is not computed twice (reuse for the later road-graph stage). NOTE: `dem.heightAt` must read the MUTATED grid — verify `makeDemSampler` samples `data` lazily (it does if it closes over the grid; if it precomputes, rebuild the sampler after grading and use the rebuilt one everywhere after).
- [ ] **Step 2:** Run the real bake: `node --max-old-space-size=6144 scripts/geo/bake-world.mjs` (~minutes). Expect: grading report line, anchor gate green, in-script determinism check OK, artifact budget gate green.
- [ ] **Step 3:** Determinism: run the bake a second time; `shasum data/winterthur/world/manifest.pb data/winterthur/world/tiles/L2/*.pb | shasum` identical across both runs.
- [ ] **Step 4:** Commit `feat(geo): grade road/rail corridors into the DEM (embankments, cuts, level junction aprons)`.

### Task 5: Burial metric — regression test + CLI

**Files:**
- Create: `scripts/geo/burial-metric.mjs`, `tests/geo/burial.test.ts`

**Interfaces:**
- Produces: `burialStats(roads, widths, heightAt): {maxM, p99M, offenders: [{x, z, devM}]}` — cross-sections every 10 m along every road, rail positions at ±(width/2) sampled against `heightAt`; deviation = |edgeGroundY − centreGroundY| (the planar ribbon's cross-slope error). CLI prints the stats table for the current world bake (decodes tiles via `scripts/geo/proto/world_pb.js` + `makeHeightSampler`-equivalent bilinear sampling in mjs).

- [ ] **Step 1:** Failing unit test: synthetic 1-cell-slope DEM + one road → `burialStats` reports the analytic deviation; plus the folded-in #131 longitudinal regression: a bumpy `heightAt` and a 100 m segment → after grading (use Task 1 `gradeDem` on a synthetic grid) `maxM < 0.3`.
- [ ] **Step 2:** FAIL → implement → PASS.
- [ ] **Step 3:** Run the CLI against the Task-4 bake: `node scripts/geo/burial-metric.mjs`. Record output. **Acceptance (spec §9): max < 0.3 m, p99 < 0.15 m.** If violated: offenders list tells you where; expected causes are corridor width vs. lane extents (raise the floor wiring) or blend width. Fix wiring constants (NOT the metric) and re-bake until green; document final numbers for the PR body.
- [ ] **Step 4:** Commit `test(geo): burial metric — cross-slope + longitudinal budgets enforced`.

### Task 6: PR 1 — visual proof, gate, ship

- [ ] **Step 1:** Frontend gate: `npx tsc -p tsconfig.typecheck.json`, `npx vitest run`, `npm run build`. (No Rust in this PR — skip cargo entirely.)
- [ ] **Step 2:** Browser proof on the running preview stack (vite port 5187, backend rush-pinned): reload, then capture (a) worst pre-fix site `window.__traffic.lookAt(-347, 1073, {radius: 140, pitch: 0.9})`, (b) rail corridor embankment near the HB approach, (c) a hillside road that previously sank. Screenshots must show the corridor bench (level road, shoulder blending into slope). Save under `scratch/captures/grading-*.png`.
- [ ] **Step 3:** Push `swiss-roads`, open PR "Swiss roads 1/3 — terrain-graded corridors (Dämme & Einschnitte)" with the burial numbers before/after + captures; wait ALL checks green (`gh pr checks --watch`); merge; delete branch.

---

# PR 2 — Markings & surfaces (branch `swiss-markings` off fresh main)

### Task 7: Crossing nodes through fetch + transform

**Files:**
- Modify: `scripts/geo/fetch-winterthur.mjs` (Overpass: add `node["highway"="crossing"](bbox);` to the roads query), `scripts/geo/lib/transform.mjs` (NEW `transformCrossings({osmRoads, projector}): {x: number, z: number}[]` from crossing nodes, 0.01 m quantized, sorted by (x, z)), bake step that writes `data/winterthur/roads.json` gains a `crossings` array.
- Test: `tests/geo/transform-crossings.test.ts` — synthetic Overpass payload with one crossing node → one `{x, z}` entry; missing tag → empty array.

- [ ] Steps: failing test → FAIL → implement → PASS → re-run `npm run geo:fetch` (Overpass, minutes; hard error if it 404s — do NOT hand-edit scratch files) → re-run the roads bake → verify `node -e "console.log(require('./data/winterthur/roads.json').crossings.length)"` > 50 (Winterthur has hundreds) → commit `feat(geo): OSM pedestrian-crossing nodes in roads.json`.

### Task 8: Marking geometry builders (pure) + mesh

**Files:**
- Create: `src/diorama/ksw/geo/roadMarkings.ts`
- Modify: `src/diorama/designTokens.ts`
- Test: `tests/diorama/roadMarkings.test.ts`

**Interfaces:**
- Consumes: `RoadPath[]`, `crossings: {x, z}[]`, `GroundYAt`, `miterStrip` (reused for strip geometry), tokens.
- Produces:
  - Tokens: `kswCity.roadYs.markings = 0.125` (between carriage 0.10 and rail 0.16; polygonOffset −2), `kswCity.markingColors = { line: 0xf2f2f2, zebra: 0xf7d84b }`.
  - `dashSegments(pts: number[][], dashM: number, gapM: number): Array<number[][]>` — sub-polylines for each dash, phase 0 at the path start (deterministic).
  - `crossingBars(crossing: {x,z}, road: RoadPath, widthM: number): Array<{cx, cz, yaw, lenM}>` — bars perpendicular to the nearest road segment, spanning the carriage width, 0.5 m bar / 0.5 m gap.
  - `buildRoadMarkings(roads, widths, crossings, groundYAt): THREE.Group` — Leitlinien on `primary|secondary|tertiary|unclassified` (6/6/0.15), Randlinien on `primary|secondary` (0.10, inset 0.3 from edge, both edges — via `offsetRight`-style perpendicular offset of the centreline by ±(width/2 − 0.3)), zebras at crossings snapped to the nearest carriage road within 10 m (crossings further away are dropped AND counted in a `console.warn` — honest absence).

- [ ] **Step 1:** Failing tests (deterministic dash phasing: same input → identical output; dash count for a 60 m line = 5 dashes; crossing bar yaw perpendicular: for a road along +x, bars have yaw 0 and span across z; unmatched crossing dropped).
- [ ] **Step 2:** FAIL → implement pure helpers → PASS.
- [ ] **Step 3:** Mesh assembly (one merged BufferGeometry per color, `ribbonMat(color, -2)` pattern from roads.ts) + wire in `main.ts` next to `buildRoads`: `cityRoot.add(buildRoadMarkings(cityRoads, correctRoadWidths(carriageOnly), roadsJson.crossings, groundYAt))` — reuse the exact carriage filter + widths from `buildRoads` (export the `FOOT` set from roads.ts instead of duplicating).
- [ ] **Step 4:** `npx tsc -p tsconfig.typecheck.json` + `npx vitest run` green. Commit `feat(diorama): VSS markings — Leitlinien, Randlinien, gelbe Fussgängerstreifen`.

### Task 9: Surfaces + kerbs

**Files:**
- Modify: `src/diorama/ksw/geo/roads.ts`, `src/diorama/designTokens.ts`
- Test: extend `tests/geo/roads.test.ts`

**Interfaces:**
- Tokens: `roadColors` gains `gravel: 0xbfae94`, `paver: 0xd8cfc0`; `roadYs.footway` 0.11 → **0.17** (kerb +6 cm above carriage 0.10; rail 0.16 keeps its −3 polygonOffset win — verify no new z-fight at rail×footway in the visual pass).
- `buildRoads` splits: carriage classes render asphalt (existing carriage color); `track|path` → gravel; `pedestrian` → paver (three `stripsMesh` calls with distinct names `gravelRibbons`, `paverRibbons`). Kerb: along every footway ribbon edge add a 45° strip (0.08 m horizontal run, from carriage level 0.10 up to 0.17) — implement as `kerbStrip(pts, width, groundYAt)` in roads.ts producing both edge strips, merged into one `kerbStrips` mesh, kerb color = carriage color darkened ×0.85.

- [ ] Steps: failing tests (named layers exist: `gravelRibbons`, `paverRibbons`, `kerbStrips`; footway y = 0.17 at vertex 0; kerb strip vertex ys span [0.10, 0.17]) → FAIL → implement → PASS → tsc + vitest → commit `feat(diorama): surface differentiation + Trottoir kerbs`.

### Task 10: PR 2 — visual proof, gate, ship

- [ ] **Step 1:** Full frontend gate (tsc, vitest, build).
- [ ] **Step 2:** Browser proof: reload preview; captures of (a) a primary axis with Leitlinie + Randlinien, (b) a zebra crossing near the KSW, (c) kerbed footway. `scratch/captures/markings-*.png`.
- [ ] **Step 3:** PR "Swiss roads 2/3 — VSS markings & surfaces", wait green, merge, delete branch.

---

# PR 3 — Gleis-Look + Bahnhöfe (branch `swiss-rail-stations` off fresh main)

### Task 11: `geo:fetch-stations` — SBB Perronkante (no fallback)

**Files:**
- Create: `scripts/geo/fetch-stations.mjs`; Modify: `package.json` (script `"geo:fetch-stations"`)

**Interfaces:**
- Produces: `scratch/geo/perronkante.geojson` (gitignored). Download: `https://data.sbb.ch/api/explore/v2.1/catalog/datasets/perronkante/exports/geojson` (pin the dataset id in a header comment; VERIFY the exact dataset slug on data.sbb.ch first and use the real one). Filter: features within the Gemeinde boundary bbox + 500 m. HARD ERROR (exit 1 with the URL and HTTP status) on non-200, empty feature list, or missing geometry — NO fallback source (project rule).
- Coordinates: dataset is WGS84 GeoJSON (verify; if LV95, convert via the `lv95ToWgs84` from `fetch-demand-data.mjs` — import it, don't copy).

- [ ] Steps: implement → run `npm run geo:fetch-stations` → verify `node -e "const g=require('./scratch/geo/perronkante.geojson'); console.log(g.features.length)"` ≥ 10 (Winterthur HB + Oberwinterthur + Töss + Seen + Grüze + Wülflingen + Hegi + Wallrüti + Reutlingen…) → commit script only `feat(geo): fetch SBB Perronkante (hard-fail, no fallback)`.

### Task 12: `bake-stations.mjs` → committed `data/winterthur/stations.json`

**Files:**
- Create: `scripts/geo/bake-stations.mjs`; `package.json` script `"geo:bake-stations"`
- Test: `tests/geo/stations-bake.test.ts` (pure transform function exported from the script module)

**Interfaces:**
- Produces: `data/winterthur/stations.json` (committed): `{stations: [{name, platforms: [{edge: number[][] (local, 0.01 m quantized), widthM: number}]}]}` sorted by (name, first-edge x). Platform `widthM` from the dataset field if present, else the bake FAILS listing the affected platforms (honest error; do not invent a width — check the dataset schema first: Perronkante carries platform width in most releases).
- Uses shared projector (`makeProjector(ANCHOR)`).

- [ ] Steps: failing test (synthetic GeoJSON feature → quantized local edge, stable double-run byte equality of `JSON.stringify`) → FAIL → implement → PASS → run real bake → inspect station list log (must include Winterthur HB) → commit incl. the JSON asset `feat(geo): bake SBB platform edges to stations.json`.

### Task 13: Rail look — ballast trapezoid, twin rails, sleepers, masts

**Files:**
- Create: `src/diorama/ksw/geo/railLook.ts`
- Modify: `src/diorama/ksw/geo/roads.ts` (buildRoads stops rendering `railRibbons`/`railBeds`; rails move to railLook), `src/diorama/ksw/main.ts` (wire `buildRailLook(cityRails, groundYAt)` + per-frame `railLook.update(camX, camZ)`)
- Test: `tests/diorama/railLook.test.ts`

**Interfaces:**
- Consumes: `RoadPath[]` rails, `GroundYAt`, `miterStrip`, tree-layer LOD pattern (`buildGrid`/`queryNear` from `treeLayer.ts` — import, don't copy), `lamps.ts` instancing pattern.
- Produces: `buildRailLook(rails: RoadPath[], groundYAt: GroundYAt): {object3d: THREE.Group, update(camX: number, camZ: number): void}`:
  - **Ballast**: per rail path, three merged strips via `miterStrip`: top (width + 2.2, y = 0.06), two slope aprons (offset strips at ±((width+2.2)/2 + 0.45), y = 0.06 → terrain, 1:1.5 slope approximated by one 0.45 m quad row), gravel tone `0xb0a893`.
  - **Twin rails**: pure helper `railOffsets(pts: number[][], gauge = 1.435): {left: number[][], right: number[][]}` (perpendicular offset ±0.7175 m, miter joints like `offsetRight` in trafficnet.mjs); each rendered as `miterStrip(offsetPts, 0.07, 0.18)` with metallic-light tone `0xb9bec6`, polygonOffset −4.
  - **Sleepers**: pure helper `sleeperSpots(pts: number[][], spacingM = 0.6): {x, z, yaw}[]`; `SleeperLayer` = one InstancedMesh (BoxGeometry 2.4 × 0.08 × 0.24, tone 0x7a6f5f, capacity 20000) rebuilt via `queryNear(grid, camX, camZ, 250)` when the camera moves > 32 m (the treeLayer near-LOD pattern).
  - **Masts**: pure `mastSpots(pts, spacingM = 50): {x, z, side}[]` alternating side; static InstancedMesh (pole: cylinder r 0.09, h 6.5; cantilever arm: box 0.06 × 0.06 × 2.4 toward track), tone 0x6b7075.
- [ ] **Step 1:** Failing tests: `railOffsets` gauge distance = 1.435 ± 1e-6 on a straight line and preserved at a 90° miter; `sleeperSpots` count for 6 m = 10, yaw perpendicular; `mastSpots` alternate sides; determinism (same input → same arrays).
- [ ] **Step 2:** FAIL → implement pure helpers → PASS → three.js assembly + main.ts wiring + remove rail strips from buildRoads (update `tests/geo/roads.test.ts` accordingly: rail layers no longer in the group — the test moves to railLook).
- [ ] **Step 3:** tsc + vitest green. Commit `feat(diorama): Gleis-Look — Schotterprofil, 1435-mm-Stahlprofile, Schwellen-LOD, Fahrleitungsmasten`.

### Task 14: Platforms at P55 + yellow safety line

**Files:**
- Create: `src/diorama/ksw/geo/platforms.ts`; Modify: `src/diorama/ksw/main.ts`
- Test: `tests/diorama/platforms.test.ts`

**Interfaces:**
- Consumes: `data/winterthur/stations.json` (static import), `GroundYAt`, `miterStrip`.
- Produces: `buildPlatforms(stations, groundYAt): THREE.Group` — per platform edge: body = strip from the edge polyline offset INWARD by `widthM` (away from track side; the dataset edge runs along the track), top at `groundYAt + 0.06 (ballast) + 0.55` (P55 over SOK), vertical face down to ballast top on the track side; surface tone paver `0xd8cfc0`; **yellow safety line**: `miterStrip(edge offset inward 0.8 m, 0.10, topY + 0.005)` color `0xf7d84b`, polygonOffset −5.
- Pure helper `platformBody(edge: number[][], widthM: number): {top: number[][], back: number[][]}` for tests.

- [ ] Steps: failing tests (offset direction inward = consistent left of edge direction — document the dataset's edge orientation after inspecting two Winterthur features and encode it; top height = ground + 0.61; safety line inset 0.8) → FAIL → implement → PASS → wire in main.ts → tsc + vitest → commit `feat(diorama): SBB-Perrons auf P55 mit gelber Sicherheitslinie`.

### Task 15: PR 3 — visual proof, gate, ship

- [ ] **Step 1:** Full frontend gate (tsc, vitest, build).
- [ ] **Step 2:** Browser proof: captures (a) rail corridor mid-distance — ballast profile + masts + sleepers visible, (b) close-up twin rails with sleepers, (c) **Winterthur HB platforms** with safety lines (`__traffic.lookAt` at the HB coordinates from stations.json). `scratch/captures/rail-*.png`. Verify no z-fight at rail×footway (Task 9 ladder note).
- [ ] **Step 3:** PR "Swiss roads 3/3 — Gleis-Look + SBB-Perrons", wait green, merge, delete branch, tidy worktrees.

---

## Self-Review Notes (done at write time)

- Spec coverage: §4.1–.2→T1/T4, §4.3→T1 (order+override), §4.4→T4 (bake order, tree filter, water, anchor gate, lane floors), §4.5→T4 golden + existing encodeTile checks, §5→(no code; T5 metric enforces), §6→T7–T9, §7→T13, §8→T11/T12/T14, §9→T1/T5/T8/T13/T14 tests + T6/T10/T15 captures, §10→three PR tasks, §11→T7/T11 sources.
- Honest-error discipline: T7 unmatched crossings warn+count; T11/T12 hard-fail on dataset gaps; T4 anchor gate fails the bake. No fallback paths anywhere.
- Known verify-first points for implementers (flagged inline): DEM grid field names (T1), `makeDemSampler` laziness (T4), SBB dataset slug + width field + edge orientation (T11/T12/T14), rail×footway ladder (T9/T15).
- Type consistency: `RoadPath{class,width,pts,bridge?}` used across T3/T4/T8/T9/T13; `GroundYAt` from roads.ts everywhere; `laneFloorWidths` mirrors `correctRoadWidths` (parity test T2).
