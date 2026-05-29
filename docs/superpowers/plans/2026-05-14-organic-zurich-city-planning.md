# Organic Zurich City Planning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Archived/closed in the 2026-05-29 documentation cleanup. This checklist is historical; `progress.md` and later plans are authoritative for current implementation status.

**Goal:** Make the Zurich map read as a grown European river city through generator rules for district density, river corridor control, forest clusters, and bridge approaches.

**Architecture:** Keep the existing `zurichWorld`, `zurichTransport`, and `zurichPlacement` modules. Add small local helpers inside those modules rather than introducing a new system: road organicity belongs in transport, river/forest terrain shape belongs in world, and building/tree clustering belongs in placement.

**Tech Stack:** TypeScript, Vitest, Playwright, Vite, Canvas/OpenGFX renderer.

---

### Task 1: Tests For City Planning Shape

**Files:**
- Modify: `tests/city/zurichPlacement.test.ts`
- Modify: `tests/city/zurichTransport.test.ts`

- [x] **Step 1: Write failing placement tests**

Add tests that compare old-town density against edge residential density, limit direct river-adjacent buildings, and require forest clusters. The tests should use existing public builders:

```ts
const world = buildZurichWorld({ seed: 1848 });
const transport = buildZurichTransport(world);
const placement = buildZurichPlacement(world, transport);
```

Use `world.zones`, `world.terrain`, `placement.buildings`, and `placement.trees` to calculate metrics. Expected first run: at least one density/corridor/cluster assertion fails on the current generator.

- [x] **Step 2: Write failing transport tests**

Add a transport assertion that residential zones do not produce excessive adjacent parallel road runs. Use the existing `countAdjacentParallelRoadRuns` helper against `transport.roads`. Expected first run: fails until district street generation is less grid-like.

- [x] **Step 3: Run focused tests**

Run:

```bash
npm test -- tests/city/zurichPlacement.test.ts tests/city/zurichTransport.test.ts
```

Expected: FAIL for the new city-planning assertions.

### Task 2: Organic Roads And Bridge Approaches

**Files:**
- Modify: `src/city/zurichTransport.ts`
- Test: `tests/city/zurichTransport.test.ts`

- [x] **Step 1: Keep arterials bridge-enabled**

Preserve `addRoad(coord, true)` for arterial paths so bridge spans stay valid.

- [x] **Step 2: Reduce district grid loops**

Adjust `addDistrictStreetPattern` so lower-density districts get one curved axis plus short spurs instead of full cross grids. Keep high-density old-town/civic districts more connected, but avoid long parallel blocks.

- [x] **Step 3: Verify transport**

Run:

```bash
npm test -- tests/city/zurichTransport.test.ts
```

Expected: PASS with at least three bridge spans and acceptable adjacent parallel road runs.

### Task 3: Clustered Buildings And River Corridor

**Files:**
- Modify: `src/city/zurichPlacement.ts`
- Test: `tests/city/zurichPlacement.test.ts`

- [x] **Step 1: Add placement scoring helpers**

Add local helpers to score each frontage candidate by zone kind, distance to zone center, distance to water, and hash jitter. Keep helpers deterministic.

- [x] **Step 2: Protect raw river banks**

Skip buildings within two tiles of water except for old-town, rail-center, and waterfront zones. Even in those zones, reduce probability near the water instead of filling every frontage.

- [x] **Step 3: Apply density falloff**

Make residential and reserve placement more selective as distance from the zone center grows. Keep old-town dense near its center.

- [x] **Step 4: Prune isolated edge buildings**

After placement, remove buildings in residential/reserve zones that have no nearby building within a small radius and are far from the zone center.

- [x] **Step 5: Verify placement**

Run:

```bash
npm test -- tests/city/zurichPlacement.test.ts
```

Expected: PASS with validation clean and first-row frames preserved.

### Task 4: Forest Patches

**Files:**
- Modify: `src/city/zurichWorld.ts`
- Modify: `src/city/zurichPlacement.ts`
- Test: `tests/city/zurichWorld.test.ts`
- Test: `tests/city/zurichPlacement.test.ts`

- [x] **Step 1: Keep forest terrain as broad zones**

Do not remove the current forest zones. If needed, add deterministic patch scoring inside placement so trees become denser in local pockets.

- [x] **Step 2: Generate irregular tree clusters**

Use hash-noise and distance-to-zone-center to place trees in clumps rather than every third forest tile.

- [x] **Step 3: Verify forest tests**

Run:

```bash
npm test -- tests/city/zurichWorld.test.ts tests/city/zurichPlacement.test.ts
```

Expected: PASS with tree count still above the existing threshold and forest cluster assertions passing.

### Task 5: Visual Verification

**Files:**
- Modify: `progress.md`
- Create ignored screenshot: `artifacts/abutown-zurich-river-city-2026-05-14-v4.png`

- [x] **Step 1: Run complete verification**

Run:

```bash
npm test
npm run build
npm run test:e2e
```

Expected: all pass.

- [x] **Step 2: Capture screenshot**

Use Playwright against the local Vite dev server to capture a screenshot to `artifacts/abutown-zurich-river-city-2026-05-14-v4.png`.

- [x] **Step 3: Update progress**

Append a short line to `progress.md` documenting the city-planning pass and screenshot.

- [x] **Step 4: Commit and push**

Stage the changed source, tests, docs, and progress file. Do not stage `.gitignore 2`. Commit with:

```bash
git commit -m "Improve Zurich city planning generator"
git push
```
