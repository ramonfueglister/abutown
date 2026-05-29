# App Runtime Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Archived/closed in the 2026-05-29 documentation cleanup. This checklist is historical; `progress.md` and later plans are authoritative for current implementation status.

**Goal:** Split the browser runtime out of `src/main.ts` into focused, testable modules while preserving current app behavior.

**Architecture:** `src/main.ts` becomes a composition root. New `src/app/*` modules own startup, fail-closed backend UI, entity selection, input wiring, static Zurich context, and runtime diagnostics. `src/render/minimalMapRenderer.ts` owns the current canvas draw path after the startup and diagnostics boundaries are protected by tests.

**Tech Stack:** TypeScript, Vite, Vitest in `node` environment, Playwright e2e, existing Rust backend for smoke checks.

---

## File Structure

Create:

- `src/app/appRuntime.ts` - backend-required startup orchestration and shutdown handle.
- `src/app/backendRequiredView.ts` - fail-closed backend-required panel and canvas clear.
- `src/app/entitySelection.ts` - pure selected pedestrian/vehicle state and hit testing.
- `src/app/interaction.ts` - canvas pointer/wheel/resize event wiring.
- `src/app/runtimeDiagnostics.ts` - `render_game_to_text` / `advanceTime` installation and payload builder.
- `src/app/zurichRuntimeContext.ts` - static Zurich world context and diagnostics.
- `src/render/minimalMapRenderer.ts` - current minimal vector canvas renderer.
- `tests/app/appRuntime.test.ts`
- `tests/app/backendRequiredView.test.ts`
- `tests/app/entitySelection.test.ts`
- `tests/app/runtimeDiagnostics.test.ts`
- `tests/app/zurichRuntimeContext.test.ts`

Modify:

- `src/main.ts` - shrink to composition root and bridge the modules.
- `tests/backend/cardHandView.test.ts` - remove static `main.ts` source assertion after `appRuntime` has a real wiring test.
- `tests/e2e/render-smoke.spec.ts` - keep assertions strong; update only if diagnostics module moves field construction without changing JSON shape.

Do not modify:

- Backend mobility simulation.
- Proto schema.
- Supabase login behavior beyond preserving its runtime mount.
- Visual styling except imports required by moved modules.

---

## Task 1: Extract Fail-Closed Backend-Required View

**Files:**

- Create: `src/app/backendRequiredView.ts`
- Create: `tests/app/backendRequiredView.test.ts`
- Modify: `src/main.ts`

- [x] **Step 1: Write the failing tests**

Create `tests/app/backendRequiredView.test.ts`:

```ts
import { afterEach, describe, expect, it, vi } from 'vitest';
import { escapeHtml, renderBackendRequired } from '../../src/app/backendRequiredView';

type FakeElement = {
  className: string;
  dataset: Record<string, string>;
  innerHTML: string;
  remove: () => void;
};

function installFakeDom(existingPanel = false) {
  let panel: FakeElement | null = existingPanel
    ? { className: '', dataset: { backendRequired: 'true' }, innerHTML: 'old', remove: () => { panel = null; } }
    : null;
  const document = {
    body: {
      appendChild: vi.fn((element: FakeElement) => {
        panel = element;
      }),
    },
    createElement: vi.fn(() => {
      const element: FakeElement = {
        className: '',
        dataset: {},
        innerHTML: '',
        remove: () => {
          if (panel === element) panel = null;
        },
      };
      return element;
    }),
    querySelector: vi.fn(() => panel),
    querySelectorAll: vi.fn(() => (panel ? [panel] : [])),
  };
  vi.stubGlobal('document', document);
  vi.stubGlobal('window', { innerWidth: 800, innerHeight: 600, devicePixelRatio: 1 });
  return { currentPanel: () => panel };
}

function createCanvas(): HTMLCanvasElement {
  const context = {
    save: vi.fn(),
    restore: vi.fn(),
    setTransform: vi.fn(),
    fillRect: vi.fn(),
    fillStyle: '',
  } as unknown as CanvasRenderingContext2D;
  return {
    dataset: {},
    getContext: vi.fn(() => context),
  } as unknown as HTMLCanvasElement;
}

describe('backendRequiredView', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('escapes text inserted into the backend-required view', () => {
    expect(escapeHtml('<backend "down" & unsafe>')).toBe('&lt;backend &quot;down&quot; &amp; unsafe&gt;');
  });

  it('renders a fail-closed backend-required panel and marks the canvas not ready', () => {
    const dom = installFakeDom();
    const canvas = createCanvas();
    const ctx = canvas.getContext('2d');

    renderBackendRequired({
      canvas,
      ctx: ctx as CanvasRenderingContext2D,
      baseUrl: 'http://127.0.0.1:8080',
      background: '#f6f0e3',
      error: new Error('network <down>'),
      viewport: { width: 800, height: 600, devicePixelRatio: 2 },
      logError: vi.fn(),
    });

    const panel = dom.currentPanel();
    expect(canvas.dataset.ready).toBe('false');
    expect(canvas.dataset.backendRequired).toBe('true');
    expect(panel).not.toBeNull();
    expect(panel?.innerHTML).toContain('Backend required');
    expect(panel?.innerHTML).toContain('network &lt;down&gt;');
    expect(panel?.innerHTML).toContain('http://127.0.0.1:8080');
  });

  it('replaces any existing backend-required panel instead of stacking panels', () => {
    const dom = installFakeDom(true);
    const canvas = createCanvas();
    const ctx = canvas.getContext('2d') as CanvasRenderingContext2D;

    renderBackendRequired({
      canvas,
      ctx,
      baseUrl: 'http://127.0.0.1:8080',
      background: '#f6f0e3',
      error: 'Backend required',
      viewport: { width: 800, height: 600, devicePixelRatio: 1 },
      logError: vi.fn(),
    });

    expect(dom.currentPanel()?.innerHTML).not.toContain('old');
    expect(dom.currentPanel()?.innerHTML).toContain('Backend required');
  });
});
```

- [x] **Step 2: Run the tests and verify they fail**

Run:

```bash
npx vitest run tests/app/backendRequiredView.test.ts --passWithNoTests
```

Expected: FAIL because `src/app/backendRequiredView.ts` does not exist.

- [x] **Step 3: Implement the fail-closed view module**

Create `src/app/backendRequiredView.ts`:

```ts
import { backendErrorMessage } from '../backend/backendGate';

export type BackendRequiredViewport = {
  width: number;
  height: number;
  devicePixelRatio: number;
};

export type RenderBackendRequiredOptions = {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  baseUrl: string;
  background: string;
  error: unknown;
  viewport?: BackendRequiredViewport;
  logError?: (message: string) => void;
};

export function renderBackendRequired(options: RenderBackendRequiredOptions): void {
  const viewport = options.viewport ?? {
    width: window.innerWidth,
    height: window.innerHeight,
    devicePixelRatio: window.devicePixelRatio || 1,
  };
  const message = backendErrorMessage(options.error);

  options.canvas.dataset.ready = 'false';
  options.canvas.dataset.backendRequired = 'true';
  options.ctx.save();
  options.ctx.setTransform(viewport.devicePixelRatio, 0, 0, viewport.devicePixelRatio, 0, 0);
  options.ctx.fillStyle = options.background;
  options.ctx.fillRect(0, 0, viewport.width, viewport.height);
  options.ctx.restore();

  document.querySelector<HTMLElement>('[data-backend-required]')?.remove();
  const panel = document.createElement('section');
  panel.className = 'backend-required-panel';
  panel.dataset.backendRequired = 'true';
  panel.innerHTML = `
    <h1>Backend required</h1>
    <p>Start Abutown backend at ${escapeHtml(options.baseUrl)} and reload.</p>
    <pre>cargo run --manifest-path backend/Cargo.toml -p sim-server</pre>
    <small>${escapeHtml(message)}</small>
  `;
  document.body.appendChild(panel);
  (options.logError ?? console.error)(`Abutown backend required: ${message}`);
}

export function escapeHtml(value: unknown): string {
  return String(value ?? '').replace(/[&<>"']/g, (char) => ({
    '&': '&amp;',
    '<': '&lt;',
    '>': '&gt;',
    '"': '&quot;',
    "'": '&#39;',
  })[char] ?? char);
}
```

- [x] **Step 4: Wire `src/main.ts` to use the module**

In `src/main.ts`:

1. Add:

```ts
import { renderBackendRequired as renderBackendRequiredView } from './app/backendRequiredView';
```

2. Replace the body of `renderBackendRequired(error: unknown)` with:

```ts
function renderBackendRequired(error: unknown): void {
  renderBackendRequiredView({
    canvas,
    ctx,
    baseUrl: backendBaseUrl,
    background: MAP_BACKGROUND,
    error,
  });
}
```

3. Delete the local `escapeHtml` function from `src/main.ts`.

- [x] **Step 5: Run focused verification**

Run:

```bash
npx vitest run tests/app/backendRequiredView.test.ts --passWithNoTests
npx tsc --noEmit --pretty false
```

Expected: both exit 0.

- [x] **Step 6: Commit**

```bash
git add src/app/backendRequiredView.ts tests/app/backendRequiredView.test.ts src/main.ts
git commit -m "refactor: extract backend required view"
```

---

## Task 2: Add Testable Runtime Startup Orchestration

**Files:**

- Create: `src/app/appRuntime.ts`
- Create: `tests/app/appRuntime.test.ts`
- Modify: `src/main.ts`
- Modify: `tests/backend/cardHandView.test.ts`

- [x] **Step 1: Write the failing runtime tests**

Create `tests/app/appRuntime.test.ts`:

```ts
import { describe, expect, it, vi } from 'vitest';
import { createMobilityOverlayState, type MobilityOverlayState } from '../../src/backend/mobilityState';
import { startAppRuntime, type AppRuntimeDependencies } from '../../src/app/appRuntime';

const backendStatus = {
  service: 'abutown-sim' as const,
  world_id: 'abutown-main',
  ok: true as const,
  protocol_version: 1,
};

function dependencies(overrides: Partial<AppRuntimeDependencies> = {}): AppRuntimeDependencies {
  const state = createMobilityOverlayState();
  return {
    requireBackend: vi.fn().mockResolvedValue(backendStatus),
    requireMobilitySnapshot: vi.fn().mockResolvedValue({ state, tickPeriodMs: 100 }),
    mountCardHandView: vi.fn(),
    boot: vi.fn().mockResolvedValue(undefined),
    connectMobilityBackend: vi.fn().mockReturnValue({ state: () => state, reconnect: vi.fn(), stop: vi.fn() }),
    renderBackendRequired: vi.fn(),
    addBeforeUnloadListener: vi.fn(),
    ...overrides,
  };
}

describe('startAppRuntime', () => {
  it('requires backend and mobility before mounting login and booting the canvas runtime', async () => {
    const calls: string[] = [];
    const deps = dependencies({
      requireBackend: vi.fn().mockImplementation(async () => {
        calls.push('backend');
        return backendStatus;
      }),
      requireMobilitySnapshot: vi.fn().mockImplementation(async () => {
        calls.push('mobility');
        return { state: createMobilityOverlayState(), tickPeriodMs: 100 };
      }),
      mountCardHandView: vi.fn().mockImplementation(() => calls.push('card-hand')),
      boot: vi.fn().mockImplementation(async () => {
        calls.push('boot');
      }),
      connectMobilityBackend: vi.fn().mockImplementation(() => {
        calls.push('connect');
        return { state: () => createMobilityOverlayState(), reconnect: vi.fn(), stop: vi.fn() };
      }),
    });
    const initialStates: Array<{ mobilityState: MobilityOverlayState; tickPeriodMs: number }> = [];

    await startAppRuntime({
      backendBaseUrl: 'http://127.0.0.1:8080',
      viewport: {
        getScreenToTile: () => (point) => point,
        getViewport: () => ({ width: 800, height: 600 }),
        getWorldDims: () => ({ widthTiles: 256, heightTiles: 256, chunkSize: 32 }),
      },
      onInitialState: (initial) => initialStates.push({
        mobilityState: initial.mobilityState,
        tickPeriodMs: initial.mobilityTickPeriodMs,
      }),
      onMobilityState: vi.fn(),
      dependencies: deps,
    });

    expect(calls).toEqual(['backend', 'mobility', 'card-hand', 'boot', 'connect']);
    expect(deps.mountCardHandView).toHaveBeenCalledWith({ baseUrl: 'http://127.0.0.1:8080' });
    expect(initialStates).toHaveLength(1);
  });

  it('renders fail-closed backend-required view and does not boot when backend startup fails', async () => {
    const deps = dependencies({
      requireBackend: vi.fn().mockRejectedValue(new Error('backend unavailable')),
    });

    const handle = await startAppRuntime({
      backendBaseUrl: 'http://127.0.0.1:8080',
      viewport: {
        getScreenToTile: () => (point) => point,
        getViewport: () => ({ width: 800, height: 600 }),
        getWorldDims: () => ({ widthTiles: 256, heightTiles: 256, chunkSize: 32 }),
      },
      onInitialState: vi.fn(),
      onMobilityState: vi.fn(),
      dependencies: deps,
    });

    expect(deps.renderBackendRequired).toHaveBeenCalledWith(new Error('backend unavailable'));
    expect(deps.mountCardHandView).not.toHaveBeenCalled();
    expect(deps.boot).not.toHaveBeenCalled();
    expect(deps.connectMobilityBackend).not.toHaveBeenCalled();
    expect(handle.mobilityBackendBridge).toBeNull();
  });
});
```

- [x] **Step 2: Run the tests and verify they fail**

Run:

```bash
npx vitest run tests/app/appRuntime.test.ts --passWithNoTests
```

Expected: FAIL because `src/app/appRuntime.ts` does not exist.

- [x] **Step 3: Implement `src/app/appRuntime.ts`**

Create `src/app/appRuntime.ts`:

```ts
import { requireBackend, type BackendHealthDto } from '../backend/backendGate';
import {
  connectMobilityBackend,
  requireMobilitySnapshot,
  type MobilityBackendBridge,
  type MobilityBackendBridgeOptions,
  type MobilityViewportGetters,
} from '../backend/mobilityClient';
import type { MobilityOverlayState } from '../backend/mobilityState';
import { mountCardHandView } from '../cardHand/cardHandView';

export type AppRuntimeInitialState = {
  backendStatus: BackendHealthDto;
  mobilityState: MobilityOverlayState;
  mobilityTickPeriodMs: number;
};

export type AppRuntimeHandle = {
  mobilityBackendBridge: MobilityBackendBridge | null;
  stop: () => void;
};

export type AppRuntimeDependencies = {
  requireBackend: typeof requireBackend;
  requireMobilitySnapshot: typeof requireMobilitySnapshot;
  mountCardHandView: typeof mountCardHandView;
  boot: (initialState: AppRuntimeInitialState) => Promise<void> | void;
  connectMobilityBackend: typeof connectMobilityBackend;
  renderBackendRequired: (error: unknown) => void;
  addBeforeUnloadListener: (listener: () => void) => void;
};

export type StartAppRuntimeOptions = {
  backendBaseUrl: string;
  viewport: MobilityViewportGetters;
  onInitialState: (state: AppRuntimeInitialState) => void;
  onMobilityState: (state: MobilityOverlayState) => void;
  dependencies: AppRuntimeDependencies;
};

export function browserBeforeUnload(listener: () => void): void {
  window.addEventListener('beforeunload', listener, { once: true });
}

export function defaultAppRuntimeDependencies(boot: AppRuntimeDependencies['boot'], renderBackendRequired: (error: unknown) => void): AppRuntimeDependencies {
  return {
    requireBackend,
    requireMobilitySnapshot,
    mountCardHandView,
    boot,
    connectMobilityBackend,
    renderBackendRequired,
    addBeforeUnloadListener: browserBeforeUnload,
  };
}

export async function startAppRuntime(options: StartAppRuntimeOptions): Promise<AppRuntimeHandle> {
  let mobilityBackendBridge: MobilityBackendBridge | null = null;
  try {
    const backendStatus = await options.dependencies.requireBackend({ baseUrl: options.backendBaseUrl });
    const required = await options.dependencies.requireMobilitySnapshot({ baseUrl: options.backendBaseUrl });
    const initialState: AppRuntimeInitialState = {
      backendStatus,
      mobilityState: required.state,
      mobilityTickPeriodMs: required.tickPeriodMs,
    };
    options.onInitialState(initialState);
    options.dependencies.mountCardHandView({ baseUrl: options.backendBaseUrl });
    await options.dependencies.boot(initialState);
    mobilityBackendBridge = options.dependencies.connectMobilityBackend({
      baseUrl: options.backendBaseUrl,
      initialState: required.state,
      onState: options.onMobilityState,
      viewport: options.viewport,
    } satisfies MobilityBackendBridgeOptions);
    options.dependencies.addBeforeUnloadListener(() => mobilityBackendBridge?.stop());
  } catch (error) {
    options.dependencies.renderBackendRequired(error);
  }

  return {
    mobilityBackendBridge,
    stop: () => mobilityBackendBridge?.stop(),
  };
}
```

- [x] **Step 4: Wire `src/main.ts` through `startAppRuntime`**

In `src/main.ts`:

1. Replace these imports:

```ts
import { backendErrorMessage, requireBackend, resolveBackendBaseUrl, type BackendHealthDto } from './backend/backendGate';
import { connectMobilityBackend, requireMobilitySnapshot, type MobilityBackendBridge } from './backend/mobilityClient';
import { mountCardHandView } from './cardHand/cardHandView';
```

with:

```ts
import { resolveBackendBaseUrl, type BackendHealthDto } from './backend/backendGate';
import type { MobilityBackendBridge } from './backend/mobilityClient';
import { defaultAppRuntimeDependencies, startAppRuntime, type AppRuntimeInitialState } from './app/appRuntime';
```

2. Replace `void startRuntime();` and the current `startRuntime()` function with:

```ts
void startRuntime();

async function startRuntime(): Promise<void> {
  const handle = await startAppRuntime({
    backendBaseUrl,
    viewport: {
      getScreenToTile: () => (screen) => worldToGrid(screenToWorld(screen)),
      getViewport: () => ({ width: window.innerWidth, height: window.innerHeight }),
      getWorldDims: () => ({
        widthTiles: zurichWorld.width,
        heightTiles: zurichWorld.height,
        chunkSize: zurichWorld.chunkSize,
      }),
    },
    onInitialState: applyInitialRuntimeState,
    onMobilityState: (state) => {
      mobilityState = state;
    },
    dependencies: defaultAppRuntimeDependencies(boot, renderBackendRequired),
  });
  mobilityBackendBridge = handle.mobilityBackendBridge;
}

function applyInitialRuntimeState(initial: AppRuntimeInitialState): void {
  backendStatus = initial.backendStatus;
  mobilityState = initial.mobilityState;
  mobilityTickPeriodMs = initial.mobilityTickPeriodMs;
}
```

3. Keep `boot()` and `renderBackendRequired()` in `main.ts` until later tasks extract more code.

- [x] **Step 5: Replace the static card-hand runtime test**

In `tests/backend/cardHandView.test.ts`, remove:

```ts
import { readFileSync } from 'node:fs';
```

Remove the whole `describe('card hand view runtime integration', ...)` block. The real runtime mount is now covered by `tests/app/appRuntime.test.ts`.

- [x] **Step 6: Run focused verification**

Run:

```bash
npx vitest run tests/app/appRuntime.test.ts tests/backend/cardHandView.test.ts --passWithNoTests
npx tsc --noEmit --pretty false
```

Expected: both commands exit 0.

- [x] **Step 7: Commit**

```bash
git add src/app/appRuntime.ts tests/app/appRuntime.test.ts src/main.ts tests/backend/cardHandView.test.ts
git commit -m "refactor: isolate app runtime startup"
```

---

## Task 3: Extract Pure Entity Selection

**Files:**

- Create: `src/app/entitySelection.ts`
- Create: `tests/app/entitySelection.test.ts`
- Modify: `src/main.ts`

- [x] **Step 1: Write the failing tests**

Create `tests/app/entitySelection.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { createEntitySelection, type SelectableEntity } from '../../src/app/entitySelection';

const pedestrian: SelectableEntity = { id: 'agent:1', path: [{ x: 10, y: 10 }, { x: 11, y: 10 }] };
const car: SelectableEntity = { id: 'vehicle:1', path: [{ x: 10, y: 10 }, { x: 11, y: 10 }] };

describe('entitySelection', () => {
  it('selects vehicles before pedestrians when both are hit', () => {
    const selection = createEntitySelection({
      getPedestrians: () => [pedestrian],
      getVehicles: () => [car],
      screenToWorld: (point) => point,
      projectPedestrian: (entity) => entity.path[0],
      projectVehicle: (entity) => entity.path[0],
      pedestrianRadius: () => 8,
      vehicleRadius: () => 10,
    });

    selection.selectAtScreenPoint({ x: 10, y: 10 });

    expect(selection.selectedAgentId()).toBeNull();
    expect(selection.selectedVehicleId()).toBe('vehicle:1');
    expect(selection.selectedPedestrian()).toBeNull();
    expect(selection.selectedVehicle()?.id).toBe('vehicle:1');
  });

  it('selects pedestrians and clears vehicle selection when no vehicle is hit', () => {
    const selection = createEntitySelection({
      getPedestrians: () => [pedestrian],
      getVehicles: () => [{ ...car, path: [{ x: 100, y: 100 }] }],
      screenToWorld: (point) => point,
      projectPedestrian: (entity) => entity.path[0],
      projectVehicle: (entity) => entity.path[0],
      pedestrianRadius: () => 8,
      vehicleRadius: () => 10,
    });

    selection.selectAtScreenPoint({ x: 10, y: 10 });

    expect(selection.selectedAgentId()).toBe('agent:1');
    expect(selection.selectedVehicleId()).toBeNull();
    expect(selection.selectedPedestrian()?.id).toBe('agent:1');
    expect(selection.selectedVehicle()).toBeNull();
  });

  it('clears both selections when no entity is hit', () => {
    const selection = createEntitySelection({
      getPedestrians: () => [pedestrian],
      getVehicles: () => [car],
      screenToWorld: (point) => point,
      projectPedestrian: (entity) => entity.path[0],
      projectVehicle: (entity) => entity.path[0],
      pedestrianRadius: () => 8,
      vehicleRadius: () => 10,
    });

    selection.selectAtScreenPoint({ x: 100, y: 100 });

    expect(selection.selectedAgentId()).toBeNull();
    expect(selection.selectedVehicleId()).toBeNull();
  });
});
```

- [x] **Step 2: Run the tests and verify they fail**

Run:

```bash
npx vitest run tests/app/entitySelection.test.ts --passWithNoTests
```

Expected: FAIL because `src/app/entitySelection.ts` does not exist.

- [x] **Step 3: Implement the pure selection module**

Create `src/app/entitySelection.ts`:

```ts
export type Coord = { x: number; y: number };

export type SelectableEntity = {
  id: string;
  path: Coord[];
};

export type EntitySelectionOptions<P extends SelectableEntity, V extends SelectableEntity> = {
  getPedestrians: () => readonly P[];
  getVehicles: () => readonly V[];
  screenToWorld: (point: Coord) => Coord;
  projectPedestrian: (entity: P) => Coord;
  projectVehicle: (entity: V) => Coord;
  pedestrianRadius: () => number;
  vehicleRadius: () => number;
};

export type EntitySelection<P extends SelectableEntity, V extends SelectableEntity> = {
  selectAtScreenPoint: (point: Coord) => void;
  selectedAgentId: () => string | null;
  selectedVehicleId: () => string | null;
  selectedPedestrian: () => P | null;
  selectedVehicle: () => V | null;
};

export function createEntitySelection<P extends SelectableEntity, V extends SelectableEntity>(
  options: EntitySelectionOptions<P, V>,
): EntitySelection<P, V> {
  let selectedAgentId: string | null = null;
  let selectedVehicleId: string | null = null;

  return {
    selectAtScreenPoint: (point) => {
      const worldPoint = options.screenToWorld(point);
      const vehicleHit = findNearestProjectedEntity(options.getVehicles(), worldPoint, options.vehicleRadius(), options.projectVehicle);
      if (vehicleHit) {
        selectedVehicleId = vehicleHit.id;
        selectedAgentId = null;
        return;
      }

      const agentHit = findNearestProjectedEntity(options.getPedestrians(), worldPoint, options.pedestrianRadius(), options.projectPedestrian);
      selectedAgentId = agentHit?.id ?? null;
      selectedVehicleId = null;
    },
    selectedAgentId: () => selectedAgentId,
    selectedVehicleId: () => selectedVehicleId,
    selectedPedestrian: () => options.getPedestrians().find((agent) => agent.id === selectedAgentId) ?? null,
    selectedVehicle: () => options.getVehicles().find((vehicle) => vehicle.id === selectedVehicleId) ?? null,
  };
}

export function findNearestProjectedEntity<T extends SelectableEntity>(
  entities: readonly T[],
  worldPoint: Coord,
  radius: number,
  projectedPoint: (entity: T) => Coord,
): T | null {
  let nearest: { entity: T; distance: number } | null = null;
  for (const entity of entities) {
    const projected = projectedPoint(entity);
    const distance = Math.hypot(projected.x - worldPoint.x, projected.y - worldPoint.y);
    if (distance > radius) continue;
    if (!nearest || distance < nearest.distance) nearest = { entity, distance };
  }
  return nearest?.entity ?? null;
}
```

- [x] **Step 4: Wire `src/main.ts` to use `createEntitySelection`**

In `src/main.ts`:

1. Add:

```ts
import { createEntitySelection } from './app/entitySelection';
```

2. Replace:

```ts
let selectedAgentId: string | null = null;
let selectedVehicleId: string | null = null;
```

with:

```ts
const entitySelection = createEntitySelection<BackendPedestrian, BackendCar>({
  getPedestrians: () => pedestriansFromMobilityState(mobilityState, pedestrianSprites, Date.now(), mobilityTickPeriodMs),
  getVehicles: () => carsFromMobilityState(mobilityState, vehicleSprites, Date.now(), mobilityTickPeriodMs),
  screenToWorld,
  projectPedestrian: (agent) => iso(agent.path[0]),
  projectVehicle: carVisualWorldPoint,
  pedestrianRadius: () => Math.max(8, 20 / camera.scale),
  vehicleRadius: () => Math.max(10, 24 / camera.scale),
});
```

This declaration must stay after `screenToWorld`, `iso`, and `carVisualWorldPoint` are available. If the current function declaration order makes this awkward, create the selection after all function declarations by changing it to:

```ts
let entitySelection: ReturnType<typeof createEntitySelection<BackendPedestrian, BackendCar>>;
```

and initialize it immediately before `void startRuntime();`:

```ts
entitySelection = createEntitySelection<BackendPedestrian, BackendCar>({
  getPedestrians: () => pedestriansFromMobilityState(mobilityState, pedestrianSprites, Date.now(), mobilityTickPeriodMs),
  getVehicles: () => carsFromMobilityState(mobilityState, vehicleSprites, Date.now(), mobilityTickPeriodMs),
  screenToWorld,
  projectPedestrian: (agent) => iso(agent.path[0]),
  projectVehicle: carVisualWorldPoint,
  pedestrianRadius: () => Math.max(8, 20 / camera.scale),
  vehicleRadius: () => Math.max(10, 24 / camera.scale),
});
```

3. Replace `selectedBackendPedestrian()` with:

```ts
function selectedBackendPedestrian(): BackendPedestrian | null {
  return entitySelection.selectedPedestrian();
}
```

4. Replace `selectedBackendCar()` with:

```ts
function selectedBackendCar(): BackendCar | null {
  return entitySelection.selectedVehicle();
}
```

5. Replace `selectMobilityEntityAtScreenPoint(point: Coord)` with:

```ts
function selectMobilityEntityAtScreenPoint(point: Coord): void {
  entitySelection.selectAtScreenPoint(point);
}
```

6. Replace diagnostics references:

```ts
selectedId: selectedAgentId,
selectedId: selectedVehicleId,
```

with:

```ts
selectedId: entitySelection.selectedAgentId(),
selectedId: entitySelection.selectedVehicleId(),
```

7. Delete local `findNearestProjectedEntity` from `src/main.ts`.

- [x] **Step 5: Run focused verification**

Run:

```bash
npx vitest run tests/app/entitySelection.test.ts --passWithNoTests
npx tsc --noEmit --pretty false
```

Expected: both commands exit 0.

- [x] **Step 6: Commit**

```bash
git add src/app/entitySelection.ts tests/app/entitySelection.test.ts src/main.ts
git commit -m "refactor: extract mobility entity selection"
```

---

## Task 4: Extract Canvas Interaction Wiring

**Files:**

- Create: `src/app/interaction.ts`
- Modify: `src/main.ts`

- [x] **Step 1: Create the interaction module**

Create `src/app/interaction.ts`:

```ts
import {
  panCameraTarget,
  zoomCameraAt,
  type CameraState,
} from '../cameraController';

export type Coord = { x: number; y: number };

export type AttachMapInteractionOptions = {
  canvas: HTMLCanvasElement;
  camera: CameraState;
  constrainCamera: (allowOverscroll: boolean) => void;
  selectAtScreenPoint: (point: Coord) => void;
  minScale: number;
  maxScale: number;
};

export function attachMapInteraction(options: AttachMapInteractionOptions): void {
  let pointerDown: Coord | null = null;
  const { canvas, camera } = options;

  canvas.addEventListener('pointerdown', (event) => {
    camera.dragging = true;
    pointerDown = { x: event.clientX, y: event.clientY };
    camera.lastX = event.clientX;
    camera.lastY = event.clientY;
    canvas.setPointerCapture(event.pointerId);
  });

  canvas.addEventListener('pointermove', (event) => {
    if (!camera.dragging) return;
    panCameraTarget(camera, event.clientX - camera.lastX, event.clientY - camera.lastY);
    options.constrainCamera(true);
    camera.lastX = event.clientX;
    camera.lastY = event.clientY;
  });

  canvas.addEventListener('pointerup', (event) => {
    const clickDistance = pointerDown ? Math.hypot(event.clientX - pointerDown.x, event.clientY - pointerDown.y) : Infinity;
    camera.dragging = false;
    if (clickDistance < 4) options.selectAtScreenPoint({ x: event.clientX, y: event.clientY });
    pointerDown = null;
    options.constrainCamera(false);
  });

  canvas.addEventListener('pointercancel', () => {
    camera.dragging = false;
    pointerDown = null;
    options.constrainCamera(false);
  });

  canvas.addEventListener('wheel', (event) => {
    event.preventDefault();
    zoomCameraAt(camera, { x: event.clientX, y: event.clientY }, event.deltaY, event.deltaMode, {
      minScale: options.minScale,
      maxScale: options.maxScale,
    });
    options.constrainCamera(false);
  }, { passive: false });
}
```

- [x] **Step 2: Wire `src/main.ts` to use it**

In `src/main.ts`:

1. Add:

```ts
import { attachMapInteraction } from './app/interaction';
```

2. Replace `attachCamera()` body with:

```ts
function attachCamera(): void {
  attachMapInteraction({
    canvas,
    camera,
    constrainCamera,
    selectAtScreenPoint: selectMobilityEntityAtScreenPoint,
    minScale: CAMERA_MIN_SCALE,
    maxScale: CAMERA_MAX_SCALE,
  });
}
```

3. Remove now-unused `panCameraTarget` and `zoomCameraAt` imports from `src/main.ts`.

- [x] **Step 3: Run focused verification**

Run:

```bash
npx tsc --noEmit --pretty false
npx vitest run tests/app/entitySelection.test.ts --passWithNoTests
```

Expected: both commands exit 0.

- [x] **Step 4: Commit**

```bash
git add src/app/interaction.ts src/main.ts
git commit -m "refactor: extract canvas interaction wiring"
```

---

## Task 5: Extract Runtime Diagnostics Builder and Install Hook

**Files:**

- Create: `src/app/runtimeDiagnostics.ts`
- Create: `tests/app/runtimeDiagnostics.test.ts`
- Modify: `src/main.ts`

- [x] **Step 1: Write the failing diagnostics tests**

Create `tests/app/runtimeDiagnostics.test.ts`:

```ts
import { describe, expect, it, vi } from 'vitest';
import { createMobilityOverlayState } from '../../src/backend/mobilityState';
import { buildRuntimeDiagnosticsPayload, installRuntimeDiagnostics } from '../../src/app/runtimeDiagnostics';

function baseOptions() {
  return {
    coordinateSystem: 'grid origin north-west, x east, y south, top-down minimal map projection',
    world: { id: 'zurich-river-city-v1', width: 256, height: 256, chunkSize: 32 },
    visualStyle: { id: 'minimal-motorways', renderer: 'canvas-vector', spriteDrawing: 'disabled' as const },
    visualAssets: { id: 'minimal-vector', tile: { width: 18, height: 18 } },
    getBackend: () => ({ required: true as const, baseUrl: 'http://127.0.0.1:8080', status: null }),
    getMobilityState: () => createMobilityOverlayState(),
    getMobilityTickPeriodMs: () => 100,
    getPedestrianSprites: () => [],
    getVehicleSprites: () => [],
    getCamera: () => ({
      current: { x: 0, y: 0, scale: 1 },
      target: { x: 0, y: 0, scale: 1 },
      dragging: false,
      bounds: { minX: -8, maxX: 263, minY: -8, maxY: 263 },
      edgeTreatment: { outskirtsTiles: 12, exitTiles: 7 },
    }),
    getCounts: () => ({
      roadTiles: 1,
      railTiles: 1,
      bridges: 0,
      buildings: 1,
      trees: 1,
      trains: 0,
      railStations: 0,
      railYardTracks: 0,
      reserveTiles: 0,
    }),
    getDiagnostics: () => ({}),
    getDetails: () => ({ total: 0 }),
    getValidation: () => ({
      validationErrors: 0,
      roadRailOverlap: 0,
      railCrossings: 0,
      invalidBuildings: 0,
      treeBuildingOverlap: 0,
    }),
    getSelected: () => ({
      agentId: null,
      vehicleId: null,
      agentInspector: null,
      vehicleInspector: null,
    }),
    projectEntityScreen: (coord: { x: number; y: number }) => coord,
    carVisualWorldPoint: (vehicle: { path: { x: number; y: number }[] }) => vehicle.path[0],
    getTrain: () => null,
    now: () => 1000,
  };
}

describe('runtimeDiagnostics', () => {
  it('preserves the minimal renderer contract', () => {
    const payload = buildRuntimeDiagnosticsPayload(baseOptions());

    expect(payload.city.visualStyle).toEqual({
      id: 'minimal-motorways',
      renderer: 'canvas-vector',
      spriteDrawing: 'disabled',
    });
    expect(payload.city.visualAssets).toEqual({
      id: 'minimal-vector',
      tile: { width: 18, height: 18 },
    });
    expect(payload.city.loadedRasterAssetPaths).toEqual([]);
  });

  it('installs render_game_to_text and advanceTime hooks', () => {
    const target = {} as Pick<Window, 'render_game_to_text' | 'advanceTime'>;
    const advanceTime = vi.fn();

    installRuntimeDiagnostics(target, { ...baseOptions(), advanceTime });

    expect(JSON.parse(target.render_game_to_text?.() ?? '{}').city.visualStyle.id).toBe('minimal-motorways');
    target.advanceTime?.(2500);
    expect(advanceTime).toHaveBeenCalledWith(2500);
  });
});
```

- [x] **Step 2: Run the tests and verify they fail**

Run:

```bash
npx vitest run tests/app/runtimeDiagnostics.test.ts --passWithNoTests
```

Expected: FAIL because `src/app/runtimeDiagnostics.ts` does not exist.

- [x] **Step 3: Implement `runtimeDiagnostics.ts`**

Create `src/app/runtimeDiagnostics.ts` with this exported shape:

```ts
import { mobilityDiagnostics, type MobilityOverlayState } from '../backend/mobilityState';
import {
  carsFromMobilityState,
  pedestriansFromMobilityState,
  type BackendCar,
  type BackendPedestrian,
} from '../render/backendMobilityDrawables';
import type { MinimalPedestrianSprite } from '../render/minimalPedestrianSprites';
import type { VehicleSprite } from '../render/vehicleSprites';

export type Coord = { x: number; y: number };
export type RuntimeInspector = { title: string; rows: { label: string; value: string }[] } | null;

export type RuntimeDiagnosticsOptions = {
  coordinateSystem: string;
  world: { id: string; width: number; height: number; chunkSize: number };
  visualStyle: { id: string; renderer: string; spriteDrawing: 'disabled' };
  visualAssets: { id: string; tile: { width: number; height: number } };
  getBackend: () => { required: true; baseUrl: string; status: unknown };
  getMobilityState: () => MobilityOverlayState;
  getMobilityTickPeriodMs: () => number;
  getPedestrianSprites: () => readonly MinimalPedestrianSprite[];
  getVehicleSprites: () => readonly VehicleSprite[];
  getCamera: () => {
    current: { x: number; y: number; scale: number };
    target: { x: number; y: number; scale: number };
    dragging: boolean;
    bounds: { minX: number; maxX: number; minY: number; maxY: number };
    edgeTreatment: { outskirtsTiles: number; exitTiles: number };
  };
  getCounts: () => {
    roadTiles: number;
    railTiles: number;
    bridges: number;
    buildings: number;
    trees: number;
    trains: number;
    railStations: number;
    railYardTracks: number;
    reserveTiles: number;
  };
  getDiagnostics: () => Record<string, number>;
  getDetails: () => Record<string, number>;
  getValidation: () => {
    validationErrors: number;
    roadRailOverlap: number;
    railCrossings: number;
    invalidBuildings: number;
    treeBuildingOverlap: number;
  };
  getSelected: () => {
    agentId: string | null;
    vehicleId: string | null;
    agentInspector: RuntimeInspector;
    vehicleInspector: RuntimeInspector;
  };
  projectEntityScreen: (coord: Coord) => Coord;
  carVisualWorldPoint: (vehicle: BackendCar) => Coord;
  getTrain: () => null | {
    position: Coord;
    alpha: number;
    speed: number;
    fadeTiles: number;
    direction: string;
  };
  now: () => number;
  advanceTime?: (ms: number) => void;
};

export type DiagnosticsWindow = Pick<Window, 'render_game_to_text' | 'advanceTime'>;

export function installRuntimeDiagnostics(target: DiagnosticsWindow, options: RuntimeDiagnosticsOptions): void {
  target.render_game_to_text = () => JSON.stringify(buildRuntimeDiagnosticsPayload(options));
  target.advanceTime = (ms: number) => options.advanceTime?.(ms);
}

export function buildRuntimeDiagnosticsPayload(options: RuntimeDiagnosticsOptions): Record<string, unknown> {
  const backend = options.getBackend();
  const mobilityState = options.getMobilityState();
  const mobilityTickPeriodMs = options.getMobilityTickPeriodMs();
  const pedestrianSprites = options.getPedestrianSprites();
  const vehicleSprites = options.getVehicleSprites();
  const camera = options.getCamera();
  const counts = options.getCounts();
  const diagnostics = options.getDiagnostics();
  const details = options.getDetails();
  const validation = options.getValidation();
  const selected = options.getSelected();
  const train = options.getTrain();
  const backendMobility = mobilityDiagnostics(mobilityState);
  const projectedPedestrians = pedestriansFromMobilityState(
    mobilityState,
    pedestrianSprites,
    options.now(),
    mobilityTickPeriodMs,
  );
  const projectedCars = carsFromMobilityState(
    mobilityState,
    vehicleSprites,
    options.now(),
    mobilityTickPeriodMs,
  );
  const mobilityAgentEntries = projectedPedestrians.map((agent) => ({
    id: agent.id,
    kind: 'pedestrian' as const,
    state: 'walking' as const,
    coord: agent.path[0],
    screen: options.projectEntityScreen(agent.path[0]),
    direction: agent.direction,
    spriteSheet: agent.sprite.sheet,
  }));
  const mobilityVehicleEntries = projectedCars.map((vehicle) => ({
    id: vehicle.id,
    kind: 'car' as const,
    state: 'driving' as const,
    coord: vehicle.path[0],
    screen: options.projectEntityScreen(options.carVisualWorldPoint(vehicle)),
    direction: vehicle.direction,
    spriteSheet: vehicle.sprite.sheet,
  }));
  const selectedMobilityAgentEntry = selected.agentId
    ? mobilityAgentEntries.find((entry) => entry.id === selected.agentId) ?? null
    : null;
  const selectedMobilityVehicleEntry = selected.vehicleId
    ? mobilityVehicleEntries.find((entry) => entry.id === selected.vehicleId) ?? null
    : null;

  return {
    coordinateSystem: options.coordinateSystem,
    city: {
      worldId: options.world.id,
      visualStyle: options.visualStyle,
      visualAssets: options.visualAssets,
      loadedRasterAssetPaths: [],
      width: options.world.width,
      height: options.world.height,
      roadTiles: counts.roadTiles,
      railTiles: counts.railTiles,
      bridges: counts.bridges,
      buildings: counts.buildings,
      trees: counts.trees,
      cars: projectedCars.length,
      trains: counts.trains,
      train,
      pedestrians: projectedPedestrians.length,
      pedestrianSprites: pedestrianSprites.length,
      pedestrianSpriteSheets: [...new Set(pedestrianSprites.map((sprite) => sprite.sheet))],
      vehicleSprites: vehicleSprites.length,
      vehicleSheets: [...new Set(vehicleSprites.map((sprite) => sprite.sheet))],
      backend,
      mobility: {
        source: 'backend',
        status: backendMobility.status,
        tick: backendMobility.tick,
        agents: backendMobility.agents,
        vehicles: backendMobility.vehicles,
        stops: backendMobility.stops,
        invalidMessages: backendMobility.invalidMessages,
        lastError: backendMobility.lastError,
      },
      mobilityAgents: {
        count: mobilityAgentEntries.length,
        selectedId: selected.agentId,
        selected: selectedMobilityAgentEntry,
        agents: mobilityAgentEntries,
      },
      mobilityVehicles: {
        count: mobilityVehicleEntries.length,
        selectedId: selected.vehicleId,
        selected: selectedMobilityVehicleEntry,
        vehicles: mobilityVehicleEntries,
      },
      agentInspector: selected.agentInspector,
      vehicleInspector: selected.vehicleInspector,
      railStations: counts.railStations,
      railYardTracks: counts.railYardTracks,
      details,
      reserveTiles: counts.reserveTiles,
      validationErrors: validation.validationErrors,
      roadRailOverlap: validation.roadRailOverlap,
      railCrossings: validation.railCrossings,
      invalidBuildings: validation.invalidBuildings,
      treeBuildingOverlap: validation.treeBuildingOverlap,
      railStationsOnRoad: diagnostics.railStationsOnRoad ?? 0,
      railStationsOnBuildings: diagnostics.railStationsOnBuildings ?? 0,
      railStationsOnRails: diagnostics.railStationsOnRails ?? 0,
      railStationsOnTrees: diagnostics.railStationsOnTrees ?? 0,
      diagnostics,
      camera: {
        mode: 'bounded-fixed-map',
        current: camera.current,
        target: camera.target,
        dragging: camera.dragging,
        bounds: camera.bounds,
        edgeTreatment: camera.edgeTreatment,
      },
    },
  };
}
```

- [x] **Step 4: Wire diagnostics from `src/main.ts`**

In `src/main.ts`:

1. Add:

```ts
import { installRuntimeDiagnostics } from './app/runtimeDiagnostics';
```

2. Replace the `window.render_game_to_text = ...` assignment and `window.advanceTime = ...` assignment with:

```ts
installRuntimeDiagnostics(window, {
  coordinateSystem: 'grid origin north-west, x east, y south, top-down minimal map projection',
  world: { id: zurichWorld.id, width: WIDTH, height: HEIGHT, chunkSize: zurichWorld.chunkSize },
  visualStyle: { id: VISUAL_STYLE_ID, renderer: 'canvas-vector', spriteDrawing: 'disabled' },
  visualAssets: { id: 'minimal-vector', tile: tileSize },
  getBackend: () => ({ required: true, baseUrl: backendBaseUrl, status: backendStatus }),
  getMobilityState: () => mobilityState,
  getMobilityTickPeriodMs: () => mobilityTickPeriodMs,
  getPedestrianSprites: () => pedestrianSprites,
  getVehicleSprites: () => vehicleSprites,
  getCamera: () => ({
    current: { x: camera.x, y: camera.y, scale: camera.scale },
    target: { x: camera.targetX, y: camera.targetY, scale: camera.targetScale },
    dragging: camera.dragging,
    bounds: {
      minX: -CAMERA_EDGE_MARGIN,
      maxX: WIDTH - 1 + CAMERA_EDGE_MARGIN,
      minY: -CAMERA_EDGE_MARGIN,
      maxY: HEIGHT - 1 + CAMERA_EDGE_MARGIN,
    },
    edgeTreatment: { outskirtsTiles: OUTSKIRTS_TILES, exitTiles: EDGE_EXIT_TILES },
  }),
  getCounts: () => ({
    roadTiles: roads.size,
    railTiles: rails.size,
    bridges: [...roads.values()].filter((road) => road.kind === 'bridge').length,
    buildings: buildings.length,
    trees: trees.length,
    trains: trains.length,
    railStations: railStations.length,
    railYardTracks: Math.max(0, railPaths.length - 2),
    reserveTiles: zurichPlacement.reserveTiles.size,
  }),
  getDiagnostics: () => cityDiagnostics(),
  getDetails: () => detailCountsByCategory(),
  getValidation: () => ({
    validationErrors: zurichValidation.errors.length,
    roadRailOverlap: zurichValidation.stats.roadRailOverlap,
    railCrossings: zurichValidation.stats.railCrossings,
    invalidBuildings: zurichValidation.stats.invalidBuildings,
    treeBuildingOverlap: zurichValidation.stats.treeBuildingOverlap,
  }),
  getSelected: () => ({
    agentId: entitySelection.selectedAgentId(),
    vehicleId: entitySelection.selectedVehicleId(),
    agentInspector: buildBackendPedestrianInspector(selectedBackendPedestrian()),
    vehicleInspector: buildBackendCarInspector(selectedBackendCar()),
  }),
  projectEntityScreen: (coord) => ({
    x: camera.x + iso(coord).x * camera.scale,
    y: camera.y + iso(coord).y * camera.scale,
  }),
  carVisualWorldPoint,
  getTrain: () => trains[0]
    ? {
        position: trainPosition(trains[0]),
        alpha: trainFadeAlpha(trainPosition(trains[0]), { height: HEIGHT, fadeTiles: trains[0].fadeTiles }),
        speed: trains[0].speed,
        fadeTiles: trains[0].fadeTiles,
        direction: 'northbound',
      }
    : null,
  now: Date.now,
  advanceTime: (ms) => {
    for (const train of trains) train.offset = trainWrappedOffset(train.offset + train.speed * (ms / 1000), train.path);
    render();
  },
});
```

3. Delete `loadedRasterAssetPaths()` from `src/main.ts`.

- [x] **Step 5: Run focused verification**

Run:

```bash
npx vitest run tests/app/runtimeDiagnostics.test.ts --passWithNoTests
npx tsc --noEmit --pretty false
```

Expected: both commands exit 0.

- [x] **Step 6: Commit**

```bash
git add src/app/runtimeDiagnostics.ts tests/app/runtimeDiagnostics.test.ts src/main.ts
git commit -m "refactor: extract runtime diagnostics"
```

---

## Task 6: Extract Zurich Runtime Context

**Files:**

- Create: `src/app/zurichRuntimeContext.ts`
- Create: `tests/app/zurichRuntimeContext.test.ts`
- Modify: `src/main.ts`

- [x] **Step 1: Write the failing context tests**

Create `tests/app/zurichRuntimeContext.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { createZurichRuntimeContext } from '../../src/app/zurichRuntimeContext';

describe('zurichRuntimeContext', () => {
  it('builds the current minimal-motorways Zurich runtime context', () => {
    const context = createZurichRuntimeContext({ seed: 1848 });

    expect(context.world.id).toBe('zurich-river-city-v1');
    expect(context.world.width).toBe(256);
    expect(context.world.height).toBe(256);
    expect(context.transport.roads.size).toBeGreaterThan(1800);
    expect(context.transport.rails.size).toBe(256);
    expect(context.placement.buildings.length).toBeGreaterThan(2250);
    expect(context.placement.trees.length).toBeGreaterThan(3000);
    expect(context.validation.errors).toHaveLength(0);
    expect(context.runtime.roads.size).toBe(context.transport.roads.size);
    expect(context.runtime.rails.size).toBe(context.transport.rails.size);
  });

  it('reports static diagnostics without invented values', () => {
    const context = createZurichRuntimeContext({ seed: 1848 });
    const diagnostics = context.staticDiagnostics();

    expect(diagnostics.invalidBuildings).toBe(0);
    expect(diagnostics.roadRailOverlap).toBe(0);
    expect(diagnostics.railStationsOnRoad).toBe(0);
  });
});
```

- [x] **Step 2: Run the tests and verify they fail**

Run:

```bash
npx vitest run tests/app/zurichRuntimeContext.test.ts --passWithNoTests
```

Expected: FAIL because `src/app/zurichRuntimeContext.ts` does not exist.

- [x] **Step 3: Implement the context module**

Create `src/app/zurichRuntimeContext.ts` by moving the existing static world setup from `src/main.ts` into a factory. Use these exported types and function names:

```ts
import {
  countBuildingsWithoutDirectStreetAdjacency,
  hasDirectStreetAdjacency,
  hasVisibleStreetFrontage,
} from '../city/buildingFrontage';
import { countAdjacentParallelRoadRuns } from '../city/roadParallelCleanup';
import { countInvalidRoadDeadEnds } from '../city/roadTopology';
import { buildZurichPlacement, type ZurichPlacement } from '../city/zurichPlacement';
import { buildZurichTransport, type ZurichTransport } from '../city/zurichTransport';
import { validateZurichCity } from '../city/zurichValidation';
import { buildZurichWorld } from '../city/zurichWorld';
import type { Coord, ZurichBuilding, ZurichDetail, ZurichWorld } from '../city/worldTypes';

export type RuntimeTerrain = 'grass' | 'water' | 'riverbank' | 'park';
export type RuntimeRoadKind = 'street' | 'bridge';
export type RuntimeRoadTile = { coord: Coord; kind: RuntimeRoadKind; mask: number };
export type RuntimeRailTile = { coord: Coord; mask: number };
export type RuntimeBuilding = {
  coord: Coord;
  sheet: ZurichBuilding['sheet'];
  frame: number;
  district: string;
};

export type ZurichRuntimeContext = {
  world: ZurichWorld;
  transport: ZurichTransport;
  placement: ZurichPlacement;
  validation: ReturnType<typeof validateZurichCity>;
  runtime: {
    terrain: Map<string, RuntimeTerrain>;
    roads: Map<string, RuntimeRoadTile>;
    rails: Map<string, RuntimeRailTile>;
    railCrossings: Set<string>;
    railReserved: Set<string>;
    railPaths: Coord[][];
    railStations: [];
    buildings: RuntimeBuilding[];
    trees: Coord[];
    details: ZurichDetail[];
  };
  staticDiagnostics: () => Record<string, number>;
};

export function createZurichRuntimeContext(options: { seed: number }): ZurichRuntimeContext {
  const world = buildZurichWorld({ seed: options.seed });
  const transport = buildZurichTransport(world);
  const placement = buildZurichPlacement(world, transport);
  const validation = validateZurichCity(world, transport, placement);
  const runtime = {
    terrain: new Map([...world.terrain].map(([tileKey, tile]) => [tileKey, toRuntimeTerrain(tile.kind)])),
    roads: new Map([...transport.roads].map(([tileKey, road]) => [tileKey, { coord: road.coord, kind: road.kind, mask: road.mask }])),
    rails: new Map([...transport.rails].map(([tileKey, rail]) => [tileKey, { coord: rail.coord, mask: rail.mask }])),
    railCrossings: transport.railCrossings,
    railReserved: new Set(transport.rails.keys()),
    railPaths: transport.railPaths,
    railStations: [] as [],
    buildings: placement.buildings.map(toRuntimeBuilding),
    trees: placement.trees,
    details: placement.details,
  };

  return {
    world,
    transport,
    placement,
    validation,
    runtime,
    staticDiagnostics: () => buildStaticDiagnostics(runtime, world.width, world.height),
  };
}

export function toRuntimeTerrain(kind: string): RuntimeTerrain {
  if (kind === 'water') return 'water';
  if (kind === 'riverbank') return 'riverbank';
  if (kind === 'park' || kind === 'forest' || kind === 'reserve' || kind === 'plaza') return 'park';
  return 'grass';
}

export function toRuntimeBuilding(building: ZurichBuilding): RuntimeBuilding {
  return {
    coord: building.coord,
    sheet: building.sheet,
    frame: building.frame,
    district: building.zoneId,
  };
}
```

Add `buildStaticDiagnostics(...)` in the same file by moving the current `cityDiagnostics()`, `buildStreetFrontages()`, `touchesRail()`, `cardinal()`, `key()`, and related static helpers from `src/main.ts`. It must use the `runtime` object passed to it, not globals.

- [x] **Step 4: Wire context from `src/main.ts`**

In `src/main.ts`:

1. Add:

```ts
import { createZurichRuntimeContext } from './app/zurichRuntimeContext';
```

2. Replace:

```ts
const zurichWorld = buildZurichWorld({ seed: 1848 });
const zurichTransport = buildZurichTransport(zurichWorld);
const zurichPlacement = buildZurichPlacement(zurichWorld, zurichTransport);
const zurichValidation = validateZurichCity(zurichWorld, zurichTransport, zurichPlacement);
```

with:

```ts
const zurichContext = createZurichRuntimeContext({ seed: 1848 });
const zurichWorld = zurichContext.world;
const zurichTransport = zurichContext.transport;
const zurichPlacement = zurichContext.placement;
const zurichValidation = zurichContext.validation;
```

3. Replace terrain/road/rail/building initializers with:

```ts
const terrain = zurichContext.runtime.terrain;
const roads = zurichContext.runtime.roads;
const rails = zurichContext.runtime.rails;
const railCrossings = zurichContext.runtime.railCrossings;
const railReserved = zurichContext.runtime.railReserved;
const railPaths = zurichContext.runtime.railPaths;
const railYardPaths: Coord[][] = [];
const railStations = zurichContext.runtime.railStations;
const buildings = zurichContext.runtime.buildings;
const trees = zurichContext.runtime.trees;
const details = zurichContext.runtime.details;
```

4. Replace `cityDiagnostics()` body with:

```ts
function cityDiagnostics(): Record<string, number> {
  return zurichContext.staticDiagnostics();
}
```

5. Delete static map-generation helpers from `src/main.ts` that are no longer called:

- `toRuntimeTerrain`
- `toRuntimeBuilding`
- `buildTerrain`
- `buildRoadNetwork`
- `removeStraightParallelRoads`
- `pruneDeadEnds`
- `buildRailPaths`
- `buildRailReserved`
- `buildRailCrossings`
- `buildRailNetwork`
- `buildRailStations`
- `addDistrictStreets`
- `addStreetSegment`
- `addUrbanBlock`
- `addRoadPoint`
- `buildBuildings`
- `buildStreetFrontages`
- `touchesRail`
- `buildTrees`
- `nearestDistrict`

Keep helpers still used by drawing, routing projection, or diagnostics.

- [x] **Step 5: Run focused verification**

Run:

```bash
npx vitest run tests/app/zurichRuntimeContext.test.ts --passWithNoTests
npx tsc --noEmit --pretty false
```

Expected: both commands exit 0.

- [x] **Step 6: Commit**

```bash
git add src/app/zurichRuntimeContext.ts tests/app/zurichRuntimeContext.test.ts src/main.ts
git commit -m "refactor: extract zurich runtime context"
```

---

## Task 7: Extract Minimal Map Renderer Mechanically

**Files:**

- Create: `src/render/minimalMapRenderer.ts`
- Modify: `src/main.ts`
- Test: existing render and e2e tests

- [x] **Step 1: Create the renderer API shell**

Create `src/render/minimalMapRenderer.ts` with the exported shell first:

```ts
import type { CameraState } from '../cameraController';
import type { MobilityOverlayState } from '../backend/mobilityState';
import type { MinimalPedestrianSprite } from './minimalPedestrianSprites';
import type { VehicleSprite } from './vehicleSprites';
import type {
  RuntimeBuilding,
  RuntimeRailTile,
  RuntimeRoadTile,
  RuntimeTerrain,
} from '../app/zurichRuntimeContext';
import type { Coord, ZurichDetail } from '../city/worldTypes';

export type MinimalMapRendererState = {
  ctx: CanvasRenderingContext2D;
  viewport: { width: number; height: number; devicePixelRatio: number };
  camera: CameraState;
  world: { width: number; height: number };
  tileSize: { width: number; height: number };
  terrain: ReadonlyMap<string, RuntimeTerrain>;
  roads: ReadonlyMap<string, RuntimeRoadTile>;
  rails: ReadonlyMap<string, RuntimeRailTile>;
  railPaths: readonly Coord[][];
  railStations: readonly [];
  buildings: readonly RuntimeBuilding[];
  trees: readonly Coord[];
  details: readonly ZurichDetail[];
  trains: readonly {
    path: Coord[];
    offset: number;
    speed: number;
    fadeTiles: number;
    carSpacing: number;
  }[];
  mobilityState: MobilityOverlayState;
  mobilityTickPeriodMs: number;
  vehicleSprites: readonly VehicleSprite[];
  pedestrianSprites: readonly MinimalPedestrianSprite[];
  selectedAgentId: string | null;
  selectedVehicleId: string | null;
};

export function renderMinimalMap(state: MinimalMapRendererState): void {
  throw new Error('renderMinimalMap shell reached before draw extraction');
}
```

- [x] **Step 2: Move draw functions without behavior changes**

Move the draw-related functions from `src/main.ts` into `src/render/minimalMapRenderer.ts`. Preserve function bodies and order as much as possible:

- `render`
- `drawScene`
- `drawTerrainBase`
- `drawRiverSurface`
- `drawOutskirtsTerrain`
- `drawRoad`
- `drawRail`
- `drawRailPath`
- `drawPolyline`
- `drawRailStation`
- `drawDetail`
- `drawBuilding`
- `buildingJitter`
- `drawTree`
- `drawTileFill`
- `drawMaskLine`
- `drawRoadPass`
- `maskSegments`
- `buildingVectorColor`
- `drawCar`
- `carVisualWorldPoint`
- `screenForwardOffset`
- `drawTrain`
- `drawCapsule`
- `movementAngle`
- `vehicleVectorColor`
- `drawPedestrian`
- `drawAgentInspectorPanel`
- `drawCarInspectorPanel`
- `drawInspectorPanel`
- `roundedRect`
- `drawEdgeConnections`
- `drawPerimeterMist`
- `visibleGridRect`
- `isCoordVisible`
- `isInsidePlayableMap`
- `isWaterSurface`
- `distanceOutsidePlayableMap`
- `drawIsoTile`
- `drawFadingEdgeTile`
- `outwardExits`
- `compareDrawables`

Keep these functions in `src/main.ts` for now because other modules still call them:

- `iso`
- `worldToGrid`
- `screenToWorld`
- `trainPosition`
- `carVisualWorldPoint` only if diagnostics or selection still depend on it. If both need it, export it from `minimalMapRenderer.ts`:

```ts
export function carVisualWorldPoint(car: BackendCar): Coord {
  const current = mapProject(car.path[0], MINIMAL_MAP_TILE_SIZE);
  const next = mapProject(car.path[1] ?? car.path[0], MINIMAL_MAP_TILE_SIZE);
  return screenForwardOffset(current, next, screenRightLaneOffset(car.direction));
}
```

Inside `renderMinimalMap`, use local aliases at the top so moved functions can stay close to their current bodies:

```ts
export function renderMinimalMap(state: MinimalMapRendererState): void {
  const ctx = state.ctx;
  ctx.save();
  ctx.setTransform(state.viewport.devicePixelRatio, 0, 0, state.viewport.devicePixelRatio, 0, 0);
  ctx.imageSmoothingEnabled = true;
  ctx.fillStyle = MAP_BACKGROUND;
  ctx.fillRect(0, 0, state.viewport.width, state.viewport.height);
  ctx.translate(state.camera.x, state.camera.y);
  ctx.scale(state.camera.scale, state.camera.scale);
  drawScene(state, { x: 0, y: 0 });
  ctx.restore();
  drawAgentInspectorPanel(state, buildBackendPedestrianInspector(selectedBackendPedestrian(state)));
  drawCarInspectorPanel(state, buildBackendCarInspector(selectedBackendCar(state)));
}
```

If moved helpers need constants, move the render-only constants with them:

- `MAP_BACKGROUND`
- `MAP_OUTSKIRTS`
- `MAP_WATER`
- `MAP_RIVERBANK`
- `MAP_PARK`
- `MAP_PLAZA`
- `ROAD_CASING`
- `ROAD_CORE`
- `ROAD_BRIDGE_CASING`
- `ROAD_BRIDGE_CORE`
- `RAIL_CASING`
- `RAIL_CORE`
- `TRAIN_CORE`
- `TREE_COLOR`
- `DETAIL_COLOR`
- `BUILDING_RESIDENTIAL`
- `BUILDING_COMMERCIAL`
- `BUILDING_CIVIC`
- `BUILDING_INDUSTRIAL`
- `AGENT_COLOR`
- `VEHICLE_COLORS`
- `VIEWPORT_GRID_PADDING`
- `OUTSKIRTS_TILES`
- `EDGE_EXIT_TILES`

Export `MAP_BACKGROUND` if `backendRequiredView` still needs the same color:

```ts
export const MAP_BACKGROUND = '#f6f0e3';
```

- [x] **Step 3: Replace `render()` in `src/main.ts`**

In `src/main.ts`, add:

```ts
import { MAP_BACKGROUND, renderMinimalMap } from './render/minimalMapRenderer';
```

Replace the local `render()` body with:

```ts
function render(): void {
  renderMinimalMap({
    ctx,
    viewport: {
      width: window.innerWidth,
      height: window.innerHeight,
      devicePixelRatio: window.devicePixelRatio || 1,
    },
    camera,
    world: { width: WIDTH, height: HEIGHT },
    tileSize,
    terrain,
    roads,
    rails,
    railPaths,
    railStations,
    buildings,
    trees,
    details,
    trains,
    mobilityState,
    mobilityTickPeriodMs,
    vehicleSprites,
    pedestrianSprites,
    selectedAgentId: entitySelection.selectedAgentId(),
    selectedVehicleId: entitySelection.selectedVehicleId(),
  });
}
```

- [x] **Step 4: Run focused verification**

Run:

```bash
npx tsc --noEmit --pretty false
npx vitest run tests/render/minimalMapProjection.test.ts tests/render/minimalGlyphScale.test.ts tests/render/minimalBuildingLayout.test.ts tests/app/runtimeDiagnostics.test.ts --passWithNoTests
```

Expected: both commands exit 0.

- [x] **Step 5: Run visual smoke**

Run:

```bash
npm run test:e2e
```

Expected: Playwright passes; current renderer diagnostics still report:

- `visualStyle.id === "minimal-motorways"`
- `visualStyle.spriteDrawing === "disabled"`
- `loadedRasterAssetPaths === []`
- at least one visible vehicle
- moving backend entities over time

- [x] **Step 6: Commit**

```bash
git add src/render/minimalMapRenderer.ts src/main.ts
git commit -m "refactor: extract minimal map renderer"
```

---

## Task 8: Final Composition-Root Cleanup

**Files:**

- Modify: `src/main.ts`
- Modify: imports in new modules if needed

- [x] **Step 1: Remove unused symbols from `src/main.ts`**

Run:

```bash
npx tsc --noEmit --pretty false
```

For every unused import or declaration reported in `src/main.ts`, delete it. Do not keep unused helpers for second paths.

Expected: after cleanup, `src/main.ts` should mainly contain:

```ts
import './style.css';

// imports from app/backend/camera/render modules

const backendBaseUrl = resolveBackendBaseUrl(import.meta.env.VITE_ABUTOWN_BACKEND_URL);
const zurichContext = createZurichRuntimeContext({ seed: 1848 });
const canvasElement = document.querySelector<HTMLCanvasElement>('#game');
if (!canvasElement) throw new Error('Missing game canvas');
const canvas = canvasElement;
const canvasContext = canvas.getContext('2d');
if (!canvasContext) throw new Error('Missing canvas context');
const ctx = canvasContext;

// state variables, runtime setup, resize/frame/render composition
```

- [x] **Step 2: Add a static composition-root regression test**

Create `tests/app/mainComposition.test.ts`:

```ts
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

describe('main composition root', () => {
  it('delegates runtime startup, diagnostics, interaction, context, and rendering to app modules', () => {
    const source = readFileSync(new URL('../../src/main.ts', import.meta.url), 'utf8');

    expect(source).toContain("from './app/appRuntime'");
    expect(source).toContain("from './app/backendRequiredView'");
    expect(source).toContain("from './app/entitySelection'");
    expect(source).toContain("from './app/interaction'");
    expect(source).toContain("from './app/runtimeDiagnostics'");
    expect(source).toContain("from './app/zurichRuntimeContext'");
    expect(source).toContain("from './render/minimalMapRenderer'");
    expect(source).not.toContain('function drawRoad(');
    expect(source).not.toContain('function drawPedestrian(');
    expect(source).not.toContain('function cityDiagnostics(');
  });
});
```

- [x] **Step 3: Run focused verification**

Run:

```bash
npx vitest run tests/app/mainComposition.test.ts --passWithNoTests
npx tsc --noEmit --pretty false
```

Expected: both commands exit 0.

- [x] **Step 4: Commit**

```bash
git add src/main.ts tests/app/mainComposition.test.ts
git commit -m "refactor: reduce main to composition root"
```

---

## Task 9: Full Quality Gate

**Files:**

- No planned source changes unless verification exposes a defect.

- [x] **Step 1: Run unit tests**

Run:

```bash
npm test
```

Expected: all Vitest files pass.

- [x] **Step 2: Run production build**

Run:

```bash
npm run build
```

Expected: proto generation, TypeScript, and Vite build exit 0.

- [x] **Step 3: Run browser e2e**

Run:

```bash
npm run test:e2e
```

Expected: Playwright passes.

- [x] **Step 4: Run forbidden-path grep**

Run:

```bash
rg -n "fallback|fall back|unwrap_or\\(\\(0\\.0, 0\\.0\\)\\)|at_activity with empty|synthetic link|global A\\*" backend/crates/sim-core/src backend/crates/sim-server/src src tests -g '!src/backend/proto/**' -g '!backend/target/**'
```

Expected: no matches.

- [x] **Step 5: Run retired asset grep**

Run:

```bash
npx vitest run tests/render/noRetiredAssets.test.ts --passWithNoTests
```

Expected: pass.

- [x] **Step 6: Browser spot-check current local app**

Open `http://127.0.0.1:5175/` against the current dev stack and verify with the browser:

- canvas `#game[data-ready="true"]`
- visible `Login` button when Supabase env is configured
- no visible "login required" text
- `window.render_game_to_text()` JSON parses
- `city.visualStyle.id === "minimal-motorways"`
- `city.loadedRasterAssetPaths.length === 0`
- `city.mobilityVehicles.count >= 1`

- [x] **Step 7: Commit verification note if docs changed**

If no files changed during verification, do not create an empty commit. If verification required doc updates, commit them:

```bash
git add docs/superpowers/plans/2026-05-27-app-runtime-refactor.md
git commit -m "docs: finalize app runtime refactor plan"
```

---

## Self-Review Checklist

- Spec coverage:
  - Runtime startup is covered by Tasks 1-2.
  - Login mount regression is covered by Task 2.
  - Entity selection is covered by Task 3.
  - Interaction is covered by Task 4.
  - Diagnostics are covered by Task 5.
  - Static Zurich context is covered by Task 6.
  - Minimal renderer extraction is covered by Task 7.
  - Composition-root cleanup is covered by Task 8.
  - Full gates and forbidden-path checks are covered by Task 9.
- Placeholder scan:
  - No placeholder marker text remains in implementation steps.
  - No step points to unspecified future work.
- Type consistency:
  - `startAppRuntime`, `AppRuntimeDependencies`, and `AppRuntimeInitialState` names match across tests, implementation, and production wiring.
  - `createEntitySelection`, `SelectableEntity`, and selected id getters match across tests and production wiring.
  - `installRuntimeDiagnostics` and `buildRuntimeDiagnosticsPayload` names match across tests and production wiring.
  - `createZurichRuntimeContext` names match across tests and production wiring.
