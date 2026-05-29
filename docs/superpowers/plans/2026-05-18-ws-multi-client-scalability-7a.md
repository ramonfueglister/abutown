# Phase 7a — WS Multi-Client Scalability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Archived/closed in the 2026-05-29 documentation cleanup. This checklist is historical; `progress.md` and later plans are authoritative for current implementation status.

**Goal:** Eliminate the two WS multi-client bottlenecks identified after Phase 6: a single `tokio::sync::Mutex` serializing every read path, and a static 64-chunk client subscription that ignores viewport. After this plan: WS broadcast filters run in parallel under `RwLock::read`, and clients subscribe only to visible chunks + 1-chunk margin, updated every 200 ms.

**Architecture:** Backend swaps `Arc<Mutex<SimulationRuntime>>` for `Arc<RwLock<SimulationRuntime>>`, mechanical split of `.lock()` into `.read()` / `.write()` per call site. Frontend gains a pure `visibleChunks` function (driven by camera + viewport), a stateful subscription client that sends diff messages, and a 200 ms polling tick wired through `connectMobilityBackend`.

**Tech Stack:** Rust (`tokio::sync::RwLock`, `bevy_ecs`), TypeScript (Vite, Vitest), existing `screenToWorld` from `cameraController.ts`.

---

## Spec

This plan implements `docs/superpowers/specs/2026-05-18-ws-multi-client-scalability-7a-design.md`. Re-read that spec before starting if any task is unclear.

## File Structure

**New files:**
- `src/render/viewportChunks.ts` — pure `visibleChunks` + `chunkOf` helpers
- `tests/render/viewportChunks.test.ts` — unit tests for the above
- `tests/backend/chunkSubscriptionClient.test.ts` — unit tests for the refactored client

**Modified files:**
- `src/backend/chunkSubscriptionClient.ts` — stateful `update`/`reset` API
- `src/backend/mobilityClient.ts` — install 200 ms polling, accept getters
- `src/main.ts` — pass `getCamera` / `getViewport` / `getWorldDims` to `connectMobilityBackend`
- `backend/crates/sim-server/src/app.rs` — `Arc<Mutex>` → `Arc<RwLock>`
- `backend/crates/sim-server/tests/websocket.rs` — adapt + add 3-client test
- `backend/crates/sim-server/tests/http.rs` — adapt to RwLock if any lock call sites
- `progress.md` — Phase 7a entry

---

## Task 1: `visibleChunks` pure function + unit tests

**Files:**
- Create: `src/render/viewportChunks.ts`
- Create: `tests/render/viewportChunks.test.ts`

- [x] **Step 1: Write the failing test file**

```ts
// tests/render/viewportChunks.test.ts
import { describe, it, expect } from 'vitest';
import { chunkOf, visibleChunks } from '../../src/render/viewportChunks';
import { createCameraState } from '../../src/cameraController';

describe('chunkOf', () => {
  it('maps positive world coords to chunks via floor', () => {
    expect(chunkOf(0, 0, 32)).toEqual({ x: 0, y: 0 });
    expect(chunkOf(31.9, 31.9, 32)).toEqual({ x: 0, y: 0 });
    expect(chunkOf(32, 32, 32)).toEqual({ x: 1, y: 1 });
    expect(chunkOf(65, 100, 32)).toEqual({ x: 2, y: 3 });
  });

  it('maps negative world coords via floor (matches backend div_euclid)', () => {
    expect(chunkOf(-1, -1, 32)).toEqual({ x: -1, y: -1 });
    expect(chunkOf(-32, -32, 32)).toEqual({ x: -1, y: -1 });
    expect(chunkOf(-33, -33, 32)).toEqual({ x: -2, y: -2 });
  });
});

describe('visibleChunks', () => {
  // World: 256 tiles square @ chunkSize 32 → 8×8 chunk grid (coords 0..=7).
  const world = { widthTiles: 256, heightTiles: 256 };
  const chunkSize = 32;
  const viewport = { width: 256, height: 256 };

  it('returns the chunk under a camera centred on world origin at scale 1, with 0 margin', () => {
    // Camera placed so screen (0,0) maps to world (0,0): camera.{x,y}=0, scale=1.
    // Screen corners (0,0)..(256,256) → world (0,0)..(256,256) → chunks (0,0)..(7,7) (inclusive at edges).
    // But (256,256) is exactly the boundary; chunkOf(256,...) = 8, clamped to 7.
    const camera = createCameraState({ x: 0, y: 0, scale: 1 });
    const result = visibleChunks(camera, viewport, world, chunkSize, 0);
    // 8 × 8 = 64 chunks (the entire world is on screen).
    expect(result).toHaveLength(64);
    expect(result).toContainEqual({ x: 0, y: 0 });
    expect(result).toContainEqual({ x: 7, y: 7 });
  });

  it('clamps negative chunk indices to 0 when camera is past the world top-left', () => {
    // Camera at world (−64, −64) means screen (0,0) shows world (−64,−64).
    // screenToWorld: world.x = (point.x - camera.x) / scale = (0 - 64)/1 = -64
    // So camera.x must be +64 for screen(0,0) to map to world(−64,−64). screenToWorld returns (0-64)/1 = -64. Good.
    const camera = createCameraState({ x: 64, y: 64, scale: 1 });
    const result = visibleChunks(camera, viewport, world, chunkSize, 0);
    // No chunk x or y should be negative.
    for (const c of result) {
      expect(c.x).toBeGreaterThanOrEqual(0);
      expect(c.y).toBeGreaterThanOrEqual(0);
    }
  });

  it('clamps past-end chunk indices to worldChunks-1', () => {
    // Push camera so screen shows past-end world coords.
    const camera = createCameraState({ x: -1000, y: -1000, scale: 1 });
    const result = visibleChunks(camera, viewport, world, chunkSize, 0);
    for (const c of result) {
      expect(c.x).toBeLessThanOrEqual(7);
      expect(c.y).toBeLessThanOrEqual(7);
    }
  });

  it('adds a 1-chunk ring when margin=1', () => {
    // Zoom in tightly so only ~1 chunk is centred under camera.
    // scale=32 means 1 world unit = 32 screen px. viewport 256×256 covers 8 world units → 1 chunk visible.
    const camera = createCameraState({ x: 128 - 16 * 32, y: 128 - 16 * 32, scale: 32 });
    // Camera now centres world chunk (0,0); 1 chunk visible.
    const zero = visibleChunks(camera, viewport, world, chunkSize, 0);
    const one = visibleChunks(camera, viewport, world, chunkSize, 1);
    expect(zero.length).toBeLessThan(one.length);
    // 1-chunk ring around 1 visible chunk: 3×3 = 9 (clamped to world bounds may reduce).
    expect(one.length).toBeLessThanOrEqual(9);
  });

  it('emits no duplicate chunk coords', () => {
    const camera = createCameraState({ x: 0, y: 0, scale: 1 });
    const result = visibleChunks(camera, viewport, world, chunkSize, 1);
    const seen = new Set(result.map((c) => `${c.x},${c.y}`));
    expect(seen.size).toBe(result.length);
  });
});
```

- [x] **Step 2: Run test to verify it fails**

Run:
```bash
npx vitest run tests/render/viewportChunks.test.ts
```

Expected: FAIL — `Cannot find module '../../src/render/viewportChunks'`.

- [x] **Step 3: Write the minimal implementation**

```ts
// src/render/viewportChunks.ts
import { screenToWorld, type CameraState } from '../cameraController';
import type { ChunkCoordDto } from '../backend/mobilityProtocol';

export function chunkOf(x: number, y: number, chunkSize: number): { x: number; y: number } {
  return {
    x: Math.floor(x / chunkSize),
    y: Math.floor(y / chunkSize),
  };
}

export function visibleChunks(
  camera: CameraState,
  viewport: { width: number; height: number },
  world: { widthTiles: number; heightTiles: number },
  chunkSize: number,
  margin: number,
): ChunkCoordDto[] {
  // Map the four screen corners to world coords, then to chunks. The AABB of
  // those four chunks plus `margin` is the visible chunk set.
  const screenCorners = [
    { x: 0, y: 0 },
    { x: viewport.width, y: 0 },
    { x: 0, y: viewport.height },
    { x: viewport.width, y: viewport.height },
  ];
  const cornerChunks = screenCorners
    .map((p) => screenToWorld(camera, p))
    .map((w) => chunkOf(w.x, w.y, chunkSize));

  const xs = cornerChunks.map((c) => c.x);
  const ys = cornerChunks.map((c) => c.y);
  let minX = Math.min(...xs) - margin;
  let maxX = Math.max(...xs) + margin;
  let minY = Math.min(...ys) - margin;
  let maxY = Math.max(...ys) + margin;

  const worldChunksX = Math.ceil(world.widthTiles / chunkSize);
  const worldChunksY = Math.ceil(world.heightTiles / chunkSize);
  minX = Math.max(0, minX);
  minY = Math.max(0, minY);
  maxX = Math.min(worldChunksX - 1, maxX);
  maxY = Math.min(worldChunksY - 1, maxY);

  const out: ChunkCoordDto[] = [];
  if (maxX < minX || maxY < minY) {
    return out;
  }
  for (let y = minY; y <= maxY; y++) {
    for (let x = minX; x <= maxX; x++) {
      out.push({ x, y });
    }
  }
  return out;
}
```

- [x] **Step 4: Run test to verify it passes**

Run:
```bash
npx vitest run tests/render/viewportChunks.test.ts
```

Expected: PASS (6 tests across 2 describe blocks).

- [x] **Step 5: Run tsc to verify types**

Run:
```bash
npx tsc --noEmit
```

Expected: clean.

- [x] **Step 6: Commit**

```bash
git add src/render/viewportChunks.ts tests/render/viewportChunks.test.ts
git commit -m "feat(render): visibleChunks pure function with world-bound clamping"
```

---

## Task 2: Refactor `chunkSubscriptionClient` to stateful update/reset

**Files:**
- Modify: `src/backend/chunkSubscriptionClient.ts`
- Create: `tests/backend/chunkSubscriptionClient.test.ts`

- [x] **Step 1: Write the failing test file**

```ts
// tests/backend/chunkSubscriptionClient.test.ts
import { describe, it, expect, vi } from 'vitest';
import { createSubscriptionClient } from '../../src/backend/chunkSubscriptionClient';

describe('createSubscriptionClient', () => {
  function setup() {
    const send = vi.fn<(text: string) => void>();
    const client = createSubscriptionClient({ send });
    return { client, send };
  }

  it('sends a single chunk_subscribe on the first update with non-empty visible set', () => {
    const { client, send } = setup();
    client.update([{ x: 1, y: 1 }, { x: 2, y: 1 }]);
    expect(send).toHaveBeenCalledTimes(1);
    const msg = JSON.parse(send.mock.calls[0][0]);
    expect(msg.type).toBe('chunk_subscribe');
    expect(msg.coords).toEqual(expect.arrayContaining([{ x: 1, y: 1 }, { x: 2, y: 1 }]));
    expect(msg.coords).toHaveLength(2);
  });

  it('sends nothing when called twice with the same visible set', () => {
    const { client, send } = setup();
    const coords = [{ x: 0, y: 0 }];
    client.update(coords);
    expect(send).toHaveBeenCalledTimes(1);
    client.update(coords);
    expect(send).toHaveBeenCalledTimes(1);
  });

  it('sends subscribe-for-added + unsubscribe-for-removed on partial overlap', () => {
    const { client, send } = setup();
    client.update([{ x: 0, y: 0 }, { x: 1, y: 0 }]);
    send.mockClear();
    client.update([{ x: 1, y: 0 }, { x: 2, y: 0 }]);
    const messages = send.mock.calls.map((c) => JSON.parse(c[0]));
    const subscribe = messages.find((m) => m.type === 'chunk_subscribe');
    const unsubscribe = messages.find((m) => m.type === 'chunk_unsubscribe');
    expect(subscribe).toBeDefined();
    expect(subscribe.coords).toEqual([{ x: 2, y: 0 }]);
    expect(unsubscribe).toBeDefined();
    expect(unsubscribe.coords).toEqual([{ x: 0, y: 0 }]);
  });

  it('sends nothing when called with an empty visible set after an empty set', () => {
    const { client, send } = setup();
    client.update([]);
    expect(send).not.toHaveBeenCalled();
  });

  it('reset() clears internal state so the next update re-subscribes from scratch', () => {
    const { client, send } = setup();
    client.update([{ x: 5, y: 5 }]);
    expect(send).toHaveBeenCalledTimes(1);
    send.mockClear();
    client.reset();
    client.update([{ x: 5, y: 5 }]);
    expect(send).toHaveBeenCalledTimes(1);
    const msg = JSON.parse(send.mock.calls[0][0]);
    expect(msg.type).toBe('chunk_subscribe');
  });
});
```

- [x] **Step 2: Run test to verify it fails**

Run:
```bash
npx vitest run tests/backend/chunkSubscriptionClient.test.ts
```

Expected: FAIL — current `createSubscriptionClient` returns `{ start() }`, not `{ update, reset }`.

- [x] **Step 3: Rewrite `chunkSubscriptionClient.ts` with the new shape**

```ts
// src/backend/chunkSubscriptionClient.ts
import { encodeClientMessage, type ChunkCoordDto } from './mobilityProtocol';

export type SubscriptionClient = {
  update(visible: ChunkCoordDto[]): void;
  reset(): void;
};

function key(coord: ChunkCoordDto): string {
  return `${coord.x},${coord.y}`;
}

function unkey(k: string): ChunkCoordDto {
  const [x, y] = k.split(',').map((s) => Number.parseInt(s, 10));
  return { x, y };
}

export function createSubscriptionClient(opts: {
  send: (text: string) => void;
}): SubscriptionClient {
  let current = new Set<string>();

  return {
    update(visible) {
      const next = new Set(visible.map(key));
      const added: ChunkCoordDto[] = [];
      const removed: ChunkCoordDto[] = [];
      for (const k of next) {
        if (!current.has(k)) added.push(unkey(k));
      }
      for (const k of current) {
        if (!next.has(k)) removed.push(unkey(k));
      }
      if (added.length > 0) {
        opts.send(encodeClientMessage({
          type: 'chunk_subscribe',
          protocol_version: 1,
          coords: added,
        }));
      }
      if (removed.length > 0) {
        opts.send(encodeClientMessage({
          type: 'chunk_unsubscribe',
          protocol_version: 1,
          coords: removed,
        }));
      }
      current = next;
    },
    reset() {
      current = new Set();
    },
  };
}
```

Note: this deletes the old `computeInitialSubscriptionCoords` and `start()` method. Task 3 wires the new API in `mobilityClient.ts`, so the build will be broken between Tasks 2 and 3 — don't run the full vitest suite until Task 3 finishes.

- [x] **Step 4: Run the subscription-client test to verify it passes**

Run:
```bash
npx vitest run tests/backend/chunkSubscriptionClient.test.ts
```

Expected: PASS (5 tests).

- [x] **Step 5: Commit (build is intentionally broken until Task 3)**

```bash
git add src/backend/chunkSubscriptionClient.ts tests/backend/chunkSubscriptionClient.test.ts
git commit -m "refactor(ws): stateful chunkSubscriptionClient with update/reset diff API"
```

---

## Task 3: Wire `mobilityClient` polling

**Files:**
- Modify: `src/backend/mobilityClient.ts`

- [x] **Step 1: Read current `connectMobilityBackend` signature**

Run:
```bash
grep -n "export function connectMobilityBackend\|connectMobilityBackend(" src/backend/mobilityClient.ts | head -5
```

Read the function body (it's around line 100+). Note the `opts` shape, the place where `openSocket` is called, and the existing `socket.onopen` block at ~line 132–140.

- [x] **Step 2: Extend the opts type and wire the polling**

In `src/backend/mobilityClient.ts`:

1. Import the new helpers:

```ts
import { createSubscriptionClient } from './chunkSubscriptionClient';
import { visibleChunks } from '../render/viewportChunks';
import type { CameraState } from '../cameraController';
```

(Add to existing imports — don't duplicate `createSubscriptionClient`.)

2. Extend the `connectMobilityBackend` options object with three getters:

```ts
export type MobilityViewportGetters = {
  getCamera: () => CameraState | null;
  getViewport: () => { width: number; height: number } | null;
  getWorldDims: () => { widthTiles: number; heightTiles: number; chunkSize: number };
};
```

Add `viewport: MobilityViewportGetters` to the existing `opts` parameter.

3. Replace the `socket.onopen` block:

```ts
socket.onopen = () => {
  const subscription = createSubscriptionClient({
    send: (text) => socket?.send(text),
  });
  const pollSubscription = () => {
    if (socket?.readyState !== WebSocket.OPEN) return;
    const camera = opts.viewport.getCamera();
    const view = opts.viewport.getViewport();
    if (!camera || !view) return;
    const world = opts.viewport.getWorldDims();
    const visible = visibleChunks(camera, view, world, world.chunkSize, 1);
    subscription.update(visible);
  };
  // Initial subscribe immediately so the client doesn't wait 200 ms for entities.
  pollSubscription();
  subscriptionInterval = setInterval(pollSubscription, 200);
};
```

4. Add the interval handle near the existing socket-state vars (top of `openSocket`):

```ts
let subscriptionInterval: ReturnType<typeof setInterval> | null = null;
```

5. Clear it in `socket.onclose` and in `closeSocket`:

```ts
if (subscriptionInterval !== null) {
  clearInterval(subscriptionInterval);
  subscriptionInterval = null;
}
```

- [x] **Step 3: Verify tsc**

Run:
```bash
npx tsc --noEmit
```

Expected: errors at call sites of `connectMobilityBackend` (because `main.ts` doesn't pass `viewport` yet). That's expected — Task 4 fixes the caller. If tsc complains about anything else inside `mobilityClient.ts`, fix it now.

- [x] **Step 4: Commit**

```bash
git add src/backend/mobilityClient.ts
git commit -m "feat(ws): 200ms viewport-driven subscription polling in mobilityClient"
```

---

## Task 4: Pass camera / viewport getters from `main.ts`

**Files:**
- Modify: `src/main.ts`

- [x] **Step 1: Locate the call site**

Run:
```bash
grep -n "connectMobilityBackend(" src/main.ts
```

Expected: one call near line 237.

- [x] **Step 2: Find the viewport size source**

The canvas dimensions are typically already available as a `viewport` or `canvas` variable. Check:

```bash
grep -n "ViewportSize\|canvas\.width\|canvas\.height\|viewport\b" src/main.ts | head -15
```

Identify which expression gives the current `{ width, height }` of the rendering surface, and which expression gives the current world dimensions (look for `worldWidthTiles`, `WORLD_TILES`, or similar — the `zurichWorld.ts` file uses `chunkSize: CHUNK_SIZE`).

- [x] **Step 3: Add the `viewport` argument to the `connectMobilityBackend` call**

The exact code depends on what `main.ts` already has in scope. The pattern:

```ts
mobilityBackendBridge = connectMobilityBackend({
  baseUrl: backendBaseUrl,
  initialState: mobilityState,
  onState: (state) => {
    mobilityState = state;
  },
  viewport: {
    getCamera: () => camera,
    getViewport: () => viewportSize,  // whatever the existing canvas-size variable is
    getWorldDims: () => ({
      widthTiles: WORLD_WIDTH_TILES,   // or world.widthTiles, whatever exists
      heightTiles: WORLD_HEIGHT_TILES,
      chunkSize: CHUNK_SIZE,
    }),
  },
});
```

If `viewportSize` is mutable (resizes on window resize), wrap it: `getViewport: () => ({ width: viewportSize.width, height: viewportSize.height })` so the getter reads the current value at call time.

If `camera` / world dims / viewport size aren't in scope at the call site, hoist them: this is a wiring change, not a refactor — keep the diff minimal.

- [x] **Step 4: Verify tsc**

Run:
```bash
npx tsc --noEmit
```

Expected: clean.

- [x] **Step 5: Run full vitest suite**

Run:
```bash
npx vitest run
```

Expected: all green (the existing 139 + 6 new viewport + 5 new subscription tests = ~150 total).

- [x] **Step 6: Commit**

```bash
git add src/main.ts
git commit -m "feat(main): wire camera + viewport getters into mobilityClient"
```

---

## Task 5: Backend — `Arc<Mutex>` → `Arc<RwLock>` swap

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs`
- Modify: `backend/crates/sim-server/tests/websocket.rs` (only if it accesses the lock directly)
- Modify: `backend/crates/sim-server/tests/http.rs` (same)

- [x] **Step 1: Find every `.lock().await` call site**

Run:
```bash
cd backend && grep -n "\\.lock()\\.await\|Mutex<SimulationRuntime>\|Arc<Mutex" crates/sim-server/src/ crates/sim-server/tests/ -r
```

Expected: ~10–15 hits, mostly in `app.rs`.

- [x] **Step 2: Swap the type in `AppState`**

In `backend/crates/sim-server/src/app.rs`, change:

```rust
use tokio::sync::{Mutex, broadcast};
```

to:

```rust
use tokio::sync::{RwLock, broadcast};
```

(If `Mutex` is also used elsewhere in the file, keep it — but for SimulationRuntime specifically, switch the type.)

Change `AppState`:

```rust
#[derive(Clone)]
pub struct AppState {
    runtime: Arc<RwLock<SimulationRuntime>>,
    deltas: broadcast::Sender<ServerMessageDto>,
    card_hands: CardHandStore,
    auth: AuthVerifier,
}
```

Change the constructor:

```rust
runtime: Arc::new(RwLock::new(runtime)),
```

Change the `runtime()` accessor return type:

```rust
pub(crate) fn runtime(&self) -> Arc<RwLock<SimulationRuntime>> {
    Arc::clone(&self.runtime)
}
```

- [x] **Step 3: Split every call site into `.read()` or `.write()`**

Go through each `.lock().await` hit from Step 1. The rule:

- If the method called is `&self` on `SimulationRuntime`, use `.read().await`.
- If the method called is `&mut self`, use `.write().await`.

Reference (from the spec):

| Method | Lock |
|---|---|
| `hello()` | read |
| `mobility_snapshot()` | read |
| `health()` | read |
| `world_summary()` | read |
| `chunk_snapshot(_)` | read |
| `filtered_mobility_delta_from_dto(...)` | read |
| `synthetic_mobility_delta_for_subscription(...)` | read |
| `next_server_messages()` | write |
| `apply_subscription_diff(...)` | write |
| `persist_chunk_snapshots()` | write |
| `persist_mobility_snapshot()` | write |
| `next_mobility_delta()` | write |
| HTTP `/commands` apply path (whatever method it calls) | write |

For each site: change `let runtime = runtime.lock().await;` to `let runtime = runtime.read().await;` (or `read`), and `let mut runtime = …` to `let mut runtime = runtime.write().await;`.

In `handle_client_message`, the existing block does both `apply_subscription_diff` (write) and `synthetic_mobility_delta_for_subscription` (read). Keep both inside a single `.write().await` — splitting buys nothing here (one WS message per pan-tick per client) and is simpler.

- [x] **Step 4: Verify backend builds**

Run:
```bash
cd backend && cargo build --locked -p sim-server
```

Expected: clean.

- [x] **Step 5: Run existing sim-server tests**

Run:
```bash
cd backend && cargo test --locked -p sim-server
```

If any test fails because it calls `.lock().await` on the runtime: apply the same `.read()`/`.write()` split there.

Expected: all sim-server tests pass.

- [x] **Step 6: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-server/src/app.rs backend/crates/sim-server/tests/
git commit -m "perf(ws): swap Mutex<SimulationRuntime> for RwLock — parallel read filters"
```

---

## Task 6: Add 3-client integration test

**Files:**
- Modify: `backend/crates/sim-server/tests/websocket.rs`

- [x] **Step 1: Locate the existing 2-client test**

Run:
```bash
grep -n "two_clients_with_different_subscriptions" backend/crates/sim-server/tests/websocket.rs
```

Read it — note the helper functions it uses (`send_chunk_subscribe`, `read_next_mobility_delta`, etc.) and the assertion pattern.

- [x] **Step 2: Add the 3-client test next to it**

```rust
#[tokio::test]
async fn three_clients_with_disjoint_subscriptions_see_only_their_chunks() {
    // Identical setup to two_clients_with_different_subscriptions — extends
    // it to three clients to exercise RwLock read-side parallelism. Each
    // client gets one chunk; we assert each receives only its chunk's
    // entities in the filtered delta.
    //
    // Copy the body of the 2-client test; add a third client subscribing to
    // a third chunk; collect first deltas from all three; assert pairwise
    // disjoint entity-id sets.
}
```

Implementation: clone the existing 2-client test body, add a third client subscribing to a different chunk coordinate, collect first delta from each, assert each client's `changed_agents` ids are disjoint from the other two.

- [x] **Step 3: Run the new test**

Run:
```bash
cd backend && cargo test --locked -p sim-server --test websocket three_clients_with_disjoint_subscriptions
```

Expected: PASS.

- [x] **Step 4: Run the full websocket test suite to confirm no regression**

Run:
```bash
cd backend && cargo test --locked -p sim-server --test websocket
```

Expected: 8 tests pass (7 existing + 1 new).

- [x] **Step 5: Commit**

```bash
git add backend/crates/sim-server/tests/websocket.rs
git commit -m "test(ws): three-client disjoint-subscription integration test"
```

---

## Task 7: Final quality gate + progress note

**Files:**
- Modify: `progress.md`

- [x] **Step 1: Full workspace gates**

Run all in sequence:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cd backend && cargo fmt --all && cargo test --locked --workspace && cargo clippy --locked --workspace --all-targets -- -D warnings
cd ..
npx vitest run
npx tsc --noEmit
npm run build
```

Expected:
- `cargo test`: 201/201 (200 + 1 new 3-client test)
- `cargo clippy`: clean
- `vitest`: ~150/150
- `tsc`: clean
- `npm run build`: succeeds (via the `scripts/build.mjs` wrapper)

If any failure: address it before continuing. Do NOT skip.

- [x] **Step 2: Manual browser smoke**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
pkill -f run-dev-stack 2>/dev/null; pkill -f sim-server 2>/dev/null; pkill -f "vite --host" 2>/dev/null
sleep 2
nohup npm run dev:stack > /tmp/abutown-stack.log 2>&1 & disown
until curl -sf http://127.0.0.1:8080/health > /dev/null; do sleep 3; done
echo BACKEND_UP
```

Open `http://127.0.0.1:5175` in a browser. Open DevTools → Network → filter for WS → click the WS connection → Messages tab.

Verify:
- Initial frame from server: `hello` message.
- Initial client → server: `chunk_subscribe` with the chunks intersecting the initial camera.
- Pan the camera: observe `chunk_subscribe` (new chunks) + `chunk_unsubscribe` (no-longer-visible chunks) flowing every ~200 ms.
- Stop panning: messages stop.
- Zoom out: large `chunk_subscribe` burst as more chunks become visible.
- Zoom in: `chunk_unsubscribe` for chunks falling off screen.

If anything is off (e.g., always full 64 chunks, no diff messages): debug before continuing.

After verifying:

```bash
pkill -f run-dev-stack; pkill -f sim-server; pkill -f "vite --host"
```

- [x] **Step 3: Add progress note**

Today's UTC timestamp:

```bash
date -u +%Y-%m-%dT%H:%M:%S.000Z
```

Add an entry to `progress.md` at the top of the entries list (immediately after line 4, the first blank line — match the style of the most recent entries):

```
<TIMESTAMP> - Phase 7a WS multi-client scalability: replaced `Arc<Mutex<SimulationRuntime>>` with `Arc<RwLock<SimulationRuntime>>` so per-client mobility-delta filters (`filtered_mobility_delta_from_dto`) run in parallel under `.read()` while the tick loop / snapshot loop / `apply_subscription_diff` take exclusive `.write()`. Frontend `chunkSubscriptionClient` is now stateful — `update(visible)` diffs against the cached subscription set and emits `chunk_subscribe(added)` / `chunk_unsubscribe(removed)` only on change. A 200 ms poll in `connectMobilityBackend` recomputes `visibleChunks(camera, viewport, world, 32, margin=1)` and feeds the diff. Frontend `visibleChunks` is a pure function (4 screen corners → `screenToWorld` → `chunkOf` AABB + 1-chunk ring → clamp to world bounds). Per-client bandwidth no longer scales with world size, only with viewport size. Three-client disjoint-subscription integration test exercises the new read-side parallelism. Phase 7a of the WS scalability arc; Phase 7b (per-chunk broadcast channels + Arc-snapshot lock-free reads) is the next architectural step.
```

- [x] **Step 4: Final commit + push**

```bash
git add progress.md
git commit -m "chore: phase 7a quality gate + progress note"
git push origin main
```

---

## Self-Review

**1. Spec coverage:**

| Spec requirement | Task |
|---|---|
| `tokio::sync::Mutex` → `RwLock`, split lock sites | Task 5 |
| Frontend `visibleChunks` pure function | Task 1 |
| Stateful `chunkSubscriptionClient` with `update`/`reset` | Task 2 |
| 200 ms poll in `mobilityClient` | Task 3 |
| Wire camera / viewport getters from caller | Task 4 |
| Three-client integration test | Task 6 |
| Final gates (cargo + vitest + tsc + build + browser) | Task 7 |

All covered.

**2. Placeholder scan:** No "TBD" / "implement later". Task 4 Step 3 is the most fluid — the exact `main.ts` expression depends on what's in scope locally. The plan acknowledges this and gives the pattern + escape hatch ("hoist if needed").

**3. Type consistency:**
- `SubscriptionClient`: `{ update, reset }` consistent across Tasks 2, 3.
- `MobilityViewportGetters`: introduced in Task 3, used in Task 4.
- `ChunkCoordDto`: existing type from `mobilityProtocol`, used consistently.
- `CameraState`: existing type from `cameraController`, used by `visibleChunks` (Task 1) and `mobilityClient` opts (Task 3).
- `visibleChunks(camera, viewport, world, chunkSize, margin)`: signature consistent across Tasks 1, 3.
- `read()` / `write()` split per the spec table: consistent in Task 5.

**Order rationale:** Frontend first because its tests are independent (Vitest doesn't need backend). Tasks 1–2 produce isolated, testable units (TDD). Task 3 wires them but breaks the build until Task 4 fixes the caller. Tasks 5–6 are backend, independent of frontend. Task 7 is the integration / gate / push.

**Scope check:** 7 tasks, ~7 commits. Each task is bite-sized (one logical change, one commit). Cohesive within Phase 7a scope.
