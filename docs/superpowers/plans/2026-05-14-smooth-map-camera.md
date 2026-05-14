# Smooth Map Camera Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the wrapping map camera with a smoother bounded camera and a subtle world-edge treatment so the player always understands they are on one fixed city map.

**Architecture:** Keep the current single-canvas renderer. Move camera math into a small pure TypeScript module, test the camera constraints and zoom anchoring there, then wire `src/main.ts` to render one map plus an atmospheric non-interactive outskirts band instead of nine wrapped copies.

**Tech Stack:** TypeScript, Canvas 2D, Vite, Vitest, Playwright.

---

### Research Notes

- Phaser documents camera bounds as the normal way to keep a camera inside a fixed game world, with camera pan and zoom effects available for smooth motion.
- Mapbox GL JS uses `maxBounds` to restrict panning and keeps zoom changes anchored around a chosen point via camera options such as `around`.
- D3 Zoom treats pan and zoom as a transform, supports pointer, wheel, and touch modalities, and constrains panning through translate extents.
- pixi-viewport exposes the same modern interaction vocabulary: drag, wheel, pinch, deceleration, clamp, clamp zoom, and bounce.
- OpenTTD keeps a finite world readable through discrete zoom levels, extra viewports, and a minimap-oriented mental model rather than wrapping the primary map unexpectedly.

### Task 1: Pure Camera Constraints

**Files:**
- Create: `src/cameraController.ts`
- Create: `tests/render/cameraController.test.ts`

- [ ] **Step 1: Write failing camera tests**

```ts
import { describe, expect, it } from 'vitest';
import {
  constrainCameraTargetToGrid,
  createCameraState,
  dampCamera,
  screenToWorld,
  zoomCameraAt,
} from '../../src/cameraController';

const viewport = { width: 1000, height: 700 };
const iso = (coord: { x: number; y: number }) => ({ x: (coord.x - coord.y) * 32, y: (coord.x + coord.y) * 16 });
const worldToGrid = (point: { x: number; y: number }) => {
  const projectedX = point.x / 32;
  const projectedY = point.y / 16;
  return { x: (projectedY + projectedX) / 2, y: (projectedY - projectedX) / 2 };
};

describe('cameraController', () => {
  it('keeps hard-constrained target center inside the fixed map bounds', () => {
    const camera = createCameraState({ x: 500, y: 350, scale: 1 });
    camera.targetX = 500 - iso({ x: -40, y: -30 }).x;
    camera.targetY = 350 - iso({ x: -40, y: -30 }).y;

    constrainCameraTargetToGrid(camera, viewport, worldToGrid, iso, {
      minX: -8,
      maxX: 103,
      minY: -8,
      maxY: 85,
      softness: 4,
      allowOverscroll: false,
    });

    const center = worldToGrid(screenToWorld(camera, { x: 500, y: 350 }, 'target'));
    expect(center.x).toBeGreaterThanOrEqual(-8);
    expect(center.y).toBeGreaterThanOrEqual(-8);
  });

  it('allows limited overscroll while dragging but damps it near the edge', () => {
    const camera = createCameraState({ x: 500, y: 350, scale: 1 });
    camera.targetX = 500 - iso({ x: -40, y: 40 }).x;
    camera.targetY = 350 - iso({ x: -40, y: 40 }).y;

    constrainCameraTargetToGrid(camera, viewport, worldToGrid, iso, {
      minX: -8,
      maxX: 103,
      minY: -8,
      maxY: 85,
      softness: 4,
      allowOverscroll: true,
    });

    const center = worldToGrid(screenToWorld(camera, { x: 500, y: 350 }, 'target'));
    expect(center.x).toBeLessThan(-8);
    expect(center.x).toBeGreaterThan(-13);
  });

  it('zooms around the pointer on the target camera', () => {
    const camera = createCameraState({ x: 120, y: 80, scale: 1 });
    const pointer = { x: 420, y: 260 };
    const before = screenToWorld(camera, pointer, 'target');

    zoomCameraAt(camera, pointer, -120, 0, { minScale: 0.5, maxScale: 3.2 });

    const after = screenToWorld(camera, pointer, 'target');
    expect(after.x).toBeCloseTo(before.x, 6);
    expect(after.y).toBeCloseTo(before.y, 6);
    expect(camera.targetScale).toBeGreaterThan(1);
  });

  it('damps current camera values toward target values', () => {
    const camera = createCameraState({ x: 0, y: 0, scale: 1 });
    camera.targetX = 100;
    camera.targetY = 50;
    camera.targetScale = 2;

    dampCamera(camera, 0.016, 18);

    expect(camera.x).toBeGreaterThan(0);
    expect(camera.x).toBeLessThan(100);
    expect(camera.scale).toBeGreaterThan(1);
    expect(camera.scale).toBeLessThan(2);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm test -- tests/render/cameraController.test.ts`
Expected: FAIL because `src/cameraController.ts` does not exist.

- [ ] **Step 3: Implement camera controller**

Create `src/cameraController.ts` with a mutable camera state, pointer-anchored zoom, exponential damping, and hard/soft grid-center constraints.

- [ ] **Step 4: Run test to verify it passes**

Run: `npm test -- tests/render/cameraController.test.ts`
Expected: PASS.

### Task 2: Integrate Fixed Smooth Camera

**Files:**
- Modify: `src/main.ts`
- Modify: `tests/e2e/render-smoke.spec.ts`

- [ ] **Step 1: Add an e2e assertion for fixed-map camera metadata**

Extend the smoke test to assert `state.city.camera.mode === 'bounded-fixed-map'` and that the camera target exists.

- [ ] **Step 2: Run e2e test to verify it fails**

Run: `npm run test:e2e -- tests/e2e/render-smoke.spec.ts`
Expected: FAIL because the state still exposes the old camera object.

- [ ] **Step 3: Wire `src/main.ts` to the camera controller**

Replace direct `camera.x/y/scale` mutation with target mutation plus `dampCamera()` in the frame loop. Remove `wrapCameraAroundMap()` and `wrappedSceneOffsets()`. Render only the original city map.

- [ ] **Step 4: Run e2e test to verify it passes**

Run: `npm run test:e2e -- tests/e2e/render-smoke.spec.ts`
Expected: PASS.

### Task 3: Render World Edge Treatment

**Files:**
- Modify: `src/main.ts`

- [ ] **Step 1: Add non-interactive outskirts rendering**

Draw a thin band of low-alpha terrain tiles outside the playable rectangle before drawing the city terrain. Use deterministic hash jitter and lower alpha so the area reads as distance, not extra playable map.

- [ ] **Step 2: Add map-edge exit continuation**

Extend border roads and rails a few tiles into the outskirts when their masks point outward, so transport lines visually disappear into the edge treatment.

- [ ] **Step 3: Add perimeter mist**

Draw a subtle diamond-shaped mist band outside the playable map after world drawables, keeping the city readable while softening the hard rectangular cutoff.

- [ ] **Step 4: Run full verification**

Run: `npm run build`
Expected: PASS.

Run: `npm test`
Expected: PASS.

Run: `npm run test:e2e`
Expected: PASS.

### Self-Review

- Spec coverage: camera smoothness, fixed map identity, bounded zoom, and edge atmosphere are each covered.
- Placeholder scan: no TODO/TBD placeholders.
- Type consistency: all camera functions take `CameraState`, viewport dimensions, and grid/world conversion callbacks consistently.
