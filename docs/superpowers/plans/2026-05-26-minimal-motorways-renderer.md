# Minimal Motorways Renderer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the default pak128/isometric scene renderer with a Mini Motorways-inspired vector map while preserving backend-driven cars, agents, selection, camera, and viewport subscriptions.

**Architecture:** Add a reversible flat map projection and use it from `src/main.ts` in place of the isometric projection. Keep existing world generation and backend mobility state. Replace image draw calls in the active render path with canvas vector primitives; keep old asset metadata available only for sprite-key compatibility.

**Tech Stack:** TypeScript, Vite, Canvas 2D, Vitest, Playwright.

---

## File Structure

- Create `src/render/minimalMapProjection.ts` — flat projection and inverse projection.
- Create `tests/render/minimalMapProjection.test.ts` — projection round-trip and tile-size tests.
- Modify `src/main.ts` — renderer constants, boot image loading, projection wiring, vector draw functions, diagnostics metadata.
- Modify `src/style.css` — light background and non-pixelated canvas rendering.
- Modify `tests/e2e/render-smoke.spec.ts` — expected visual style metadata and canvas pixel checks.
- Modify `progress.md` — record the renderer change after verification.

## Tasks

### Task 1: Projection

- [x] Write a failing Vitest test for flat map projection round-tripping.
- [x] Run `npm test -- tests/render/minimalMapProjection.test.ts` and confirm it fails because the module does not exist.
- [x] Add `src/render/minimalMapProjection.ts` with `MINIMAL_MAP_TILE_SIZE`, `mapProject`, and `mapUnproject`.
- [x] Re-run the projection test and confirm it passes.

### Task 2: Renderer Swap

- [x] Modify `src/main.ts` to use `mapProject`/`mapUnproject` for `iso()` and `worldToGrid()`.
- [x] Change the default renderer metadata to `minimal-motorways`.
- [x] Skip runtime pak128 image loading in `boot()` and initialize vehicle/pedestrian sprite catalogs directly from metadata.
- [x] Replace active draw functions for terrain, roads, rail, buildings, trees, cars, pedestrians, trains, outskirts, and perimeter treatment with vector canvas primitives.
- [x] Keep selection/hit-testing paths using the same projected coordinates as rendering.

### Task 3: Smoke Expectations

- [x] Update `tests/e2e/render-smoke.spec.ts` for top-down/vector metadata.
- [x] Add a canvas colored-pixel check that proves the new map is light and nonblank.
- [x] Keep backend mobility and selection assertions intact.

### Task 4: Verification

- [x] Run `npm test -- tests/render/minimalMapProjection.test.ts tests/render/backendMobilityDrawables.test.ts`.
- [x] Run `npm run build`.
- [x] Run `npm run test:e2e` or an equivalent backend-backed browser smoke.
- [x] Open the local app in a browser and inspect the rendered scene for the new visual direction.
- [x] Update `progress.md` only after verification.

### Task 5: Fresh Backend QA + Entity Legibility

- [x] Add `ABUTOWN_SERVER_MODE=memory` so visual QA can boot fresh backend state without mutating Postgres snapshots.
- [x] Preserve vehicle entities across Warm LOD demotion so cars do not collapse into generic pedestrian flow.
- [x] Add screen-stable minimal glyph sizing for cars, pedestrians, roads, and rail.
- [x] Re-run backend, frontend, build, and Playwright render smoke verification against the fresh in-memory backend.
