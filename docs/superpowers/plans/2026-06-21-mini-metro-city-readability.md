# Mini Metro City Readability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the default Abutown city view read as a calm Mini-Metro-style city with agents, while keeping economy flow diagnostics available only in economy zoom.

**Architecture:** Keep the existing canvas-vector renderer and backend protocol. Fix the visual hierarchy at the render layer: semantic zoom decides which overlays are visible, city view suppresses economy guide/flow overlays, and individual agents become quiet occupancy marks instead of dominant particles. The economy view still exposes market guide edges and moving goods flows for smoke diagnostics.

**Tech Stack:** TypeScript, Vite, Canvas 2D, Vitest, existing `scripts/smoke-schematic.mjs` browser smoke.

---

## File Structure

- Modify `src/render/designTokens.ts`
  - Owns semantic zoom constants and visual opacity constants.
- Modify `src/render/layerBlend.ts`
  - Owns render-layer visibility policy for `agents`, `flows`, `marketGuides`, `markets`, and `network`.
- Modify `tests/render/layerBlend.test.ts`
  - Locks city/economy overlay visibility.
- Modify `src/render/drawMarketGuides.ts`
  - Makes market guide edges obey layer opacity.
- Modify `tests/render/drawMarketGuides.test.ts`
  - Verifies guide edges draw in economy mode and disappear in city mode.
- Modify `src/render/drawFlows.ts`
  - Keeps existing economy drawing behavior; relies on layer opacity to suppress city flows.
- Modify `tests/render/drawFlows.test.ts`
  - Verifies zero-opacity flows do not stroke curves or draw cargo.
- Modify `src/render/drawAgents.ts`
  - Makes non-trader agents smaller and quieter in city view; traders remain readable.
- Modify `tests/render/drawAgents.test.ts`
  - Verifies the agent visual policy.
- Modify `src/render/minimalMapRenderer.ts`
  - Wires `marketGuides` through `layerBlend`, and keeps diagnostics coherent.
- Modify `scripts/smoke-schematic.mjs`
  - Asserts economy overlays are present in economy zoom and absent in city zoom.

---

### Task 1: Make Semantic Overlay Policy Explicit

**Files:**
- Modify: `src/render/designTokens.ts`
- Modify: `src/render/layerBlend.ts`
- Test: `tests/render/layerBlend.test.ts`

- [ ] **Step 1: Write the failing layer policy test**

Replace the `flows` test and add a `marketGuides` test in `tests/render/layerBlend.test.ts`:

```ts
it('flows: visible in the economy band, hidden in the city band, monotone between', () => {
  expect(layerBlend('flows', 0.18)).toEqual({ opacity: 1, detail: 'individual' });
  expect(layerBlend('flows', ZOOM_CITY_MIN)).toEqual({ opacity: 0, detail: 'aggregate' });
  expect(layerBlend('flows', 2.8)).toEqual({ opacity: 0, detail: 'aggregate' });
  const mid = layerBlend('flows', (ZOOM_ECONOMY_MAX + ZOOM_CITY_MIN) / 2).opacity;
  expect(mid).toBeLessThan(1);
  expect(mid).toBeGreaterThan(0);
  expect(layerBlend('flows', ZOOM_CITY_MIN - 0.01).detail).toBe('individual');
});

it('market guide edges: visible only outside the city band', () => {
  expect(layerBlend('marketGuides', 0.18)).toEqual({ opacity: 1, detail: 'individual' });
  expect(layerBlend('marketGuides', ZOOM_CITY_MIN)).toEqual({ opacity: 0, detail: 'aggregate' });
  expect(layerBlend('marketGuides', 2.8)).toEqual({ opacity: 0, detail: 'aggregate' });
});

it('clamps outside the camera range', () => {
  expect(layerBlend('agents', 0.0001).opacity).toBeCloseTo(AGENT_SHIMMER_OPACITY);
  expect(layerBlend('flows', 100).opacity).toBe(0);
  expect(layerBlend('marketGuides', 100).opacity).toBe(0);
});
```

Remove `FLOW_MIN_OPACITY` from the imports in that test file.

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
npm test -- tests/render/layerBlend.test.ts
```

Expected: FAIL because `LayerKey` does not include `marketGuides` and city-band flows still return the old hint opacity.

- [ ] **Step 3: Implement layer policy**

In `src/render/designTokens.ts`, replace the old city-flow hint with an explicit hidden opacity:

```ts
export const ECONOMY_OVERLAY_CITY_OPACITY = 0;
export const AGENT_SHIMMER_OPACITY = 0.55;
```

In `src/render/layerBlend.ts`, update imports and the layer union:

```ts
import {
  AGENT_SHIMMER_OPACITY,
  ECONOMY_OVERLAY_CITY_OPACITY,
  ZOOM_CITY_MIN,
  ZOOM_ECONOMY_MAX,
} from './designTokens';

export type LayerKey = 'network' | 'markets' | 'agents' | 'flows' | 'marketGuides';
```

Then update the switch:

```ts
    case 'flows':
    case 'marketGuides':
      return {
        opacity: t >= 1 ? ECONOMY_OVERLAY_CITY_OPACITY : 1 - (1 - ECONOMY_OVERLAY_CITY_OPACITY) * t,
        detail: t < 1 ? 'individual' : 'aggregate',
      };
```

- [ ] **Step 4: Run test to verify it passes**

Run:

```bash
npm test -- tests/render/layerBlend.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/render/designTokens.ts src/render/layerBlend.ts tests/render/layerBlend.test.ts
git commit -m "fix: separate city and economy overlay visibility"
```

---

### Task 2: Hide Market Guide Edges In City View

**Files:**
- Modify: `src/render/drawMarketGuides.ts`
- Modify: `src/render/minimalMapRenderer.ts`
- Test: `tests/render/drawMarketGuides.test.ts`

- [ ] **Step 1: Write failing guide-edge opacity tests**

Update `tests/render/drawMarketGuides.test.ts` imports:

```ts
import type { LayerBlend } from '../../src/render/layerBlend';
```

Add helpers near `project` setup:

```ts
const economyBlend: LayerBlend = { opacity: 1, detail: 'individual' };
const cityBlend: LayerBlend = { opacity: 0, detail: 'aggregate' };
```

Update existing calls to pass `economyBlend` before `cameraScale`:

```ts
const drawn = drawMarketGuideEdges(
  ctx,
  (coord) => ({ x: coord.x * 18 + 9, y: coord.y * 18 + 9 }),
  [
    { from: { x: 32, y: 23.49 }, to: { x: 48, y: 24.51 } },
    { from: { x: 16, y: 24.51 }, to: { x: 64, y: 23.49 } },
  ],
  economyBlend,
  0.32,
);
```

Add this test:

```ts
it('draws nothing when the layer is hidden in city view', () => {
  const ctx = fakeCtx();
  const drawn = drawMarketGuideEdges(
    ctx,
    (coord) => coord,
    [{ from: { x: 1, y: 1 }, to: { x: 2, y: 2 } }],
    cityBlend,
    1,
  );

  expect(drawn).toBe(0);
  expect(ctx.operations).toHaveLength(0);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
npm test -- tests/render/drawMarketGuides.test.ts
```

Expected: FAIL because `drawMarketGuideEdges` still accepts `cameraScale` as the fourth argument.

- [ ] **Step 3: Implement guide-edge layer blend**

In `src/render/drawMarketGuides.ts`, import `LayerBlend`:

```ts
import type { LayerBlend } from './layerBlend';
```

Change the function signature:

```ts
export function drawMarketGuideEdges(
  ctx: CanvasRenderingContext2D,
  project: (coord: Point) => Point,
  edges: readonly MarketGuideEdge[],
  blend: LayerBlend,
  cameraScale: number,
): number {
  if (edges.length === 0 || blend.opacity <= 0) return 0;
```

Multiply alpha by `blend.opacity`:

```ts
    ctx.globalAlpha = 0.72 * blend.opacity;
```

and:

```ts
    ctx.globalAlpha = 0.56 * blend.opacity;
```

In `src/render/minimalMapRenderer.ts`, update the call:

```ts
  lastMarketGuideEdgesDrawn = drawMarketGuideEdges(
    state.ctx,
    (coord) => iso(state, coord),
    state.marketGuideEdges ?? [],
    layerBlend('marketGuides', state.camera.scale),
    state.camera.scale,
  );
```

- [ ] **Step 4: Run guide tests and renderer tests**

Run:

```bash
npm test -- tests/render/drawMarketGuides.test.ts tests/render/minimalMapRenderer.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/render/drawMarketGuides.ts src/render/minimalMapRenderer.ts tests/render/drawMarketGuides.test.ts
git commit -m "fix: hide market guide edges in city view"
```

---

### Task 3: Keep Goods Flows Economy-Only

**Files:**
- Modify: `tests/render/drawFlows.test.ts`
- Modify: `src/render/drawFlows.ts` only if the test reveals cargo still draws at zero opacity

- [ ] **Step 1: Write a stronger zero-opacity flow test**

Replace the current proxy-only `draws nothing at zero layer opacity` test in `tests/render/drawFlows.test.ts` with an operation-recording context:

```ts
it('strokes no curves and draws no cargo at zero layer opacity', () => {
  const operations: string[] = [];
  const ctx = new Proxy({} as CanvasRenderingContext2D, {
    get: (_target, prop) => {
      if (prop === 'canvas') return undefined;
      if (prop === 'stroke') return () => operations.push('stroke');
      if (prop === 'fill') return () => operations.push('fill');
      if (prop === 'beginPath') return () => operations.push('beginPath');
      return () => undefined;
    },
    set: () => true,
  });

  const drawn = drawFlows(
    ctx,
    project,
    markets,
    [flow(9003, 9004, 250)],
    { opacity: 0, detail: 'aggregate' },
  );

  expect(drawn).toBe(0);
  expect(operations).toEqual([]);
});
```

- [ ] **Step 2: Run test to verify behavior**

Run:

```bash
npm test -- tests/render/drawFlows.test.ts
```

Expected: PASS if the existing early return already prevents both curves and cargo. If it fails, continue to Step 3.

- [ ] **Step 3: Implement minimal guard if needed**

If Step 2 fails, keep the existing early return as the first executable line in `drawFlows`:

```ts
  if (blend.opacity <= 0) return 0;
```

Do not add any separate cargo-specific condition if the early return is sufficient.

- [ ] **Step 4: Run test to verify it passes**

Run:

```bash
npm test -- tests/render/drawFlows.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tests/render/drawFlows.test.ts src/render/drawFlows.ts
git commit -m "test: lock zero-opacity flow rendering"
```

---

### Task 4: Calm Non-Trader Agent Rendering

**Files:**
- Modify: `src/render/drawAgents.ts`
- Test: `tests/render/drawAgents.test.ts`

- [ ] **Step 1: Write failing agent visual policy tests**

Update imports in `tests/render/drawAgents.test.ts`:

```ts
import {
  agentGlyph,
  pedestrianOpacity,
  pedestrianRadiusScale,
} from '../../src/render/drawAgents';
```

Add tests:

```ts
describe('pedestrian visual policy', () => {
  it('renders ordinary city agents as quiet occupancy marks', () => {
    expect(pedestrianOpacity('pedestrian', { opacity: 1, detail: 'individual' })).toBeCloseTo(0.38);
    expect(pedestrianRadiusScale('pedestrian', { opacity: 1, detail: 'individual' })).toBeCloseTo(0.72);
  });

  it('keeps traders readable because they explain economy motion', () => {
    expect(pedestrianOpacity('trader', { opacity: 1, detail: 'individual' })).toBeCloseTo(0.95);
    expect(pedestrianRadiusScale('trader', { opacity: 1, detail: 'individual' })).toBeCloseTo(1.35);
  });

  it('keeps aggregate economy agents visible but secondary', () => {
    expect(pedestrianOpacity('pedestrian', { opacity: 0.55, detail: 'aggregate' })).toBeCloseTo(0.55);
    expect(pedestrianRadiusScale('pedestrian', { opacity: 0.55, detail: 'aggregate' })).toBeCloseTo(1);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
npm test -- tests/render/drawAgents.test.ts
```

Expected: FAIL because `pedestrianOpacity` and `pedestrianRadiusScale` do not exist.

- [ ] **Step 3: Implement visual policy helpers**

In `src/render/drawAgents.ts`, export these helpers after `agentGlyph`:

```ts
export function pedestrianOpacity(kind: BackendPedestrian['kind'], blend: LayerBlend): number {
  if (kind === 'trader') return Math.max(0.95, blend.opacity);
  if (blend.detail === 'individual') return Math.min(0.38, blend.opacity);
  return blend.opacity;
}

export function pedestrianRadiusScale(kind: BackendPedestrian['kind'], blend: LayerBlend): number {
  if (kind === 'trader') return 1.35;
  if (blend.detail === 'individual') return 0.72;
  return 1;
}
```

Then replace the current alpha/radius handling inside `drawPedestrian`:

```ts
  const radiusScale = pedestrianRadiusScale(pedestrian.kind, blend);
  ctx.globalAlpha *= pedestrianOpacity(pedestrian.kind, blend);
```

Use `radiusScale` in all pedestrian arc calls:

```ts
    ctx.arc(0, 0, style.radius * radiusScale, 0, Math.PI * 2);
```

For rings, combine both scales:

```ts
    ctx.arc(0, 0, style.radius * glyph.radiusScale * radiusScale, 0, Math.PI * 2);
```

For filled dots, combine both scales:

```ts
    ctx.arc(0, 0, style.radius * glyph.radiusScale * radiusScale, 0, Math.PI * 2);
```

- [ ] **Step 4: Run agent tests**

Run:

```bash
npm test -- tests/render/drawAgents.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/render/drawAgents.ts tests/render/drawAgents.test.ts
git commit -m "fix: calm city agent glyphs"
```

---

### Task 5: Lock The City/Economy Split In Browser Smoke

**Files:**
- Modify: `scripts/smoke-schematic.mjs`

- [ ] **Step 1: Add city overlay diagnostics to the smoke script**

After existing city diagnostics:

```js
const cityAgentCount = cityDiag?.city?.mobility?.agents ?? 0;
const cityPedestrians = cityDiag?.city?.pedestrians ?? 0;
```

add:

```js
const cityFlowCount = cityDiag?.city?.economyFlowCount ?? 0;
const cityMarketGuideEdgeCount = cityDiag?.city?.marketGuideEdgeCount ?? 0;
```

Then extend `checks`:

```js
  city_flow_overlay_hidden: cityFlowCount === 0,
  city_market_guides_hidden: cityMarketGuideEdgeCount === 0,
```

Extend `summary`:

```js
  city_flow_count_at_city_zoom: cityFlowCount,
  city_market_guide_edge_count_at_city_zoom: cityMarketGuideEdgeCount,
```

- [ ] **Step 2: Run focused JS lint-free smoke syntax check**

Run:

```bash
node --check scripts/smoke-schematic.mjs
```

Expected: no output and exit code 0.

- [ ] **Step 3: Commit**

```bash
git add scripts/smoke-schematic.mjs
git commit -m "test: assert city view hides economy overlays"
```

---

### Task 6: Full Verification Against Running Stack

**Files:**
- No source modifications.

- [ ] **Step 1: Run frontend unit tests**

Run:

```bash
npm test -- tests/render/layerBlend.test.ts tests/render/drawMarketGuides.test.ts tests/render/drawFlows.test.ts tests/render/drawAgents.test.ts tests/render/minimalMapRenderer.test.ts
```

Expected: all listed test files pass.

- [ ] **Step 2: Run typecheck**

Run:

```bash
npm run typecheck
```

Expected: exit code 0.

- [ ] **Step 3: Run browser smoke against the current dev stack**

Run:

```bash
REUSE_STACK=1 BACKEND_PORT=8080 FRONTEND_PORT=5175 FLOW_POLL_TIMEOUT_MS=20000 node scripts/smoke-schematic.mjs
```

Expected summary fields:

```json
{
  "status": "ok",
  "economy_flow_count_at_economy_zoom": 1,
  "market_guide_edge_count_at_economy_zoom": 3,
  "city_flow_count_at_city_zoom": 0,
  "city_market_guide_edge_count_at_city_zoom": 0,
  "city_agent_count_at_city_zoom": 1
}
```

The exact agent count can be greater than 1; the required condition is `city_agent_count_at_city_zoom > 0`.

- [ ] **Step 4: Inspect the city screenshot**

Open:

```bash
/Users/ramonfuglister/Coding/abutown/smoke-schematic-city.png
```

Expected visual result:
- no long gray guide lines in the city screenshot
- no brown animated goods line in the city screenshot
- agents remain present but read as small occupancy marks, not as the dominant layer
- station glyphs and district fields are the primary visible structure

- [ ] **Step 5: Commit verification-only script changes if not committed**

If `scripts/smoke-schematic.mjs` was not committed in Task 5:

```bash
git add scripts/smoke-schematic.mjs
git commit -m "test: verify mini metro city readability"
```

---

## Self-Review

**Spec coverage:** The plan directly addresses the reported gray guide lines, brown moving goods lines, and wild-looking points. Economy diagnostics remain available in economy zoom. City view keeps stations, districts, and agents visible.

**Placeholder scan:** No task contains TBD/TODO/fill-later language. Each code-changing task includes exact files, code snippets, commands, and expected output.

**Type consistency:** `LayerKey` gains `marketGuides`; `drawMarketGuideEdges` receives `LayerBlend` before `cameraScale`; `minimalMapRenderer` passes `layerBlend('marketGuides', state.camera.scale)`; tests import `LayerBlend` from `src/render/layerBlend`.
