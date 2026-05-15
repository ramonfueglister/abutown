# Pak128 Mobility Bridge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connect the backend mobility bridge to the Pak128 renderer without replacing Pak128 assets or current visual tuning.

**Architecture:** Copy the proven mobility modules into the Pak128 branch, then wire them into `src/main.ts` as an opt-in bridge. The overlay draws after the Pak128 scene and exposes diagnostics through `render_game_to_text`.

**Tech Stack:** TypeScript, Vite, Vitest, Playwright, Rust backend already running on `127.0.0.1:8080`.

---

### Task 1: Mobility Protocol And State

**Files:**
- Create: `src/backend/mobilityProtocol.ts`
- Create: `src/backend/mobilityState.ts`
- Create: `tests/backend/mobilityProtocol.test.ts`
- Create: `tests/backend/mobilityState.test.ts`

- [ ] **Step 1: Write failing protocol/state tests**

Add tests that import the missing modules and assert valid snapshots, invalid deltas, reducer diagnostics, walking coordinates, waiting stop coordinates, and completed activity coordinates `{ x: 146, y: 126 }`.

- [ ] **Step 2: Run tests to verify failure**

Run: `npm test -- tests/backend/mobilityProtocol.test.ts tests/backend/mobilityState.test.ts`
Expected: fail because `src/backend/mobilityProtocol.ts` and `src/backend/mobilityState.ts` do not exist.

- [ ] **Step 3: Implement protocol and reducer**

Create the two backend files by porting the merged mobility implementation. Keep the completed activity coordinate at `{ x: 146, y: 126 }`.

- [ ] **Step 4: Run tests to verify pass**

Run: `npm test -- tests/backend/mobilityProtocol.test.ts tests/backend/mobilityState.test.ts`
Expected: all tests pass.

### Task 2: Mobility Client And Overlay

**Files:**
- Create: `src/backend/mobilityClient.ts`
- Create: `src/render/mobilityOverlay.ts`
- Create: `tests/render/mobilityOverlay.test.ts`

- [ ] **Step 1: Write failing overlay test**

Add a test for `buildMobilityOverlayDrawItems()` asserting the agent radius is `10`, the agent color is `#f7d76a`, and visibility filtering happens before projection.

- [ ] **Step 2: Run test to verify failure**

Run: `npm test -- tests/render/mobilityOverlay.test.ts`
Expected: fail because `src/render/mobilityOverlay.ts` does not exist.

- [ ] **Step 3: Implement client and overlay**

Port `connectMobilityBackend()` and `drawMobilityOverlay()` from the merged mobility work. Keep the larger agent ring so the completed agent is visible at default Pak128 zoom.

- [ ] **Step 4: Run test to verify pass**

Run: `npm test -- tests/render/mobilityOverlay.test.ts`
Expected: all tests pass.

### Task 3: Pak128 Main Wiring

**Files:**
- Modify: `src/main.ts`
- Modify: `tests/e2e/render-smoke.spec.ts`

- [ ] **Step 1: Write failing E2E expectation**

Keep the Pak128 E2E asset-pack checks and add a default mobility diagnostic assertion:

```ts
expect(state.city.mobility).toEqual(expect.objectContaining({
  status: 'disconnected',
  agents: 0,
  vehicles: 0,
  stops: 0,
}));
```

- [ ] **Step 2: Run E2E to verify failure**

Run: `npm run test:e2e -- tests/e2e/render-smoke.spec.ts`
Expected: fail because `state.city.mobility` is undefined.

- [ ] **Step 3: Wire mobility into `src/main.ts`**

Import the mobility client, state reducer diagnostics, and overlay renderer. Add `mobilityState`, `mobilityBridge`, opt-in backend configuration, boot-time connection setup, overlay drawing after the scene, and mobility diagnostics in `render_game_to_text`.

- [ ] **Step 4: Run E2E to verify pass**

Run: `npm run test:e2e -- tests/e2e/render-smoke.spec.ts`
Expected: pass with Pak128 asset-pack diagnostics and default disconnected mobility diagnostics.

### Task 4: Final Verification

**Files:**
- No planned source changes unless verification exposes a defect.

- [ ] **Step 1: Run unit suite**

Run: `npm test`
Expected: all tests pass.

- [ ] **Step 2: Run build**

Run: `npm run build`
Expected: TypeScript and Vite build pass.

- [ ] **Step 3: Verify browser**

Load `http://127.0.0.1:5175/` from the Pak128 server. Confirm diagnostics show `assetPack.id` as `simutrans-pak128` and mobility status as `local-pedestrians`, then take a screenshot.
