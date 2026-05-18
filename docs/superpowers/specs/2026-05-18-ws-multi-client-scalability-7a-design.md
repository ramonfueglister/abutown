# Phase 7a — WS Multi-Client Scalability (Viewport-Driven Subscriptions + RwLock)

**Status:** Design approved, awaiting implementation plan
**Date:** 2026-05-18
**Predecessor:** Phase 6 chunk-LOD mobility (commit `0f2bdb1`)
**Successor:** Phase 7b — Per-Chunk Broadcast Channels + Arc-Snapshot Reads (not in this spec)

## Problem

After Phase 6 the mobility tick itself is cheap (~13 µs at 100k entities), but
the **WS path serialises every connected client through a single
`tokio::sync::Mutex<SimulationRuntime>`**. With 500+ concurrent clients
targeted, two compounding bottlenecks remain:

1. **Lock contention (#2).** Every per-tick broadcast filter
   (`filtered_mobility_delta_from_dto`) and every chunk-subscribe message
   (`apply_subscription_diff`) acquires the same exclusive `Mutex`. The tick
   loop's `next_server_messages` and the snapshot loop's
   `persist_chunk_snapshots` also take it. 500 clients × O(N_changed) filter
   per tick, run strictly serially, even though the filter only needs
   read access.

2. **Static client subscription fan-out (#3).** `chunkSubscriptionClient.ts`
   subscribes to all 64 chunks of the 256×256 world at WS-open and never
   updates. As the world grows past 256×256, this scales `O(world_chunks)`
   per client — every client pays for every entity regardless of what they
   can actually see.

Phase 7a fixes both as the first step of a Phase 7 multi-client scalability
arc. Phase 7b (separate spec) will move to per-chunk broadcast channels and
Arc-snapshot lock-free reads to eliminate the per-client filter entirely.

## Goals

- WS-broadcast filter calls (`filtered_mobility_delta_from_dto`) run in
  parallel across clients.
- HTTP read endpoints (`/mobility`, `/world`, `/health`) no longer block on
  the tick loop's mutation.
- Frontend subscribes only to chunks intersecting its viewport + a 1-chunk
  margin, recomputed every ~200 ms.
- No regression in existing functionality: 200/200 workspace tests stay
  green, existing WS integration tests (`websocket.rs` ×7) pass unchanged.

## Non-Goals (Phase 7b)

- Per-chunk `tokio::broadcast` channels (would eliminate the per-client
  filter entirely; bigger refactor).
- `Arc<RuntimeSnapshot>` swap for fully lock-free read paths.
- Adaptive margin based on camera velocity.
- Reactive update on each pan/zoom event instead of polling.

## Architecture

### Two changes, shipped together

**Backend (#2):** swap `Arc<tokio::sync::Mutex<SimulationRuntime>>` for
`Arc<tokio::sync::RwLock<SimulationRuntime>>` in `AppState`. WS read paths
take `.read().await`; mutation paths take `.write().await`. The tick loop's
brief write blocks new readers but tokio's RwLock is fair, so writes always
make progress.

**Frontend (#3):** replace `createSubscriptionClient` (one-shot statement)
with a stateful client that holds the currently-subscribed set, is polled
every ~200 ms with the camera state, and sends `chunk_subscribe(added)` /
`chunk_unsubscribe(removed)` diff messages when the set changes.

These two changes are independent — either could ship alone — but together
they unblock the same multi-client-scale workload and share a coherent
narrative.

## Components

### Backend (4 files)

**`backend/crates/sim-server/src/app.rs`**
- `AppState.runtime`: `Arc<Mutex<SimulationRuntime>>` → `Arc<RwLock<SimulationRuntime>>`.
- `state.runtime().lock().await` → `.read().await` or `.write().await`
  depending on what the caller does:
  - Read: `hello()`, `mobility_snapshot()`, `health()`, `world_summary()`,
    `chunk_snapshot()`, `filtered_mobility_delta_from_dto()`,
    `synthetic_mobility_delta_for_subscription()`, `event_count()`.
  - Write: `next_server_messages()` (advances tick),
    `apply_subscription_diff()`, `persist_chunk_snapshots()`,
    `persist_mobility_snapshot()`, HTTP `/commands` apply path.
- `spawn_delta_loop` and `spawn_snapshot_loop` use `.write().await`.

**`backend/crates/sim-server/src/runtime.rs`**
- No signature changes. The existing methods are already correctly split
  between `&self` (read) and `&mut self` (write); the RwLock mirrors that.

**`backend/crates/sim-server/tests/{websocket,http}.rs`**
- Tests that call `state.runtime().lock().await` need to switch to
  `.read()` / `.write()`. Quick mechanical update.
- New test: `three_clients_with_disjoint_subscriptions` (extends the
  existing 2-client test to 3 to exercise the read-side parallelism).

### Frontend (3 files)

**`src/render/viewportChunks.ts` (new)**
- Pure function:
  ```ts
  visibleChunks(
    camera: { x: number; y: number; scale: number },
    viewport: { width: number; height: number },
    world: { widthTiles: number; heightTiles: number },
    chunkSize: number,
    margin: number,
  ): ChunkCoord[]
  ```
- Computes the four screen corners → `screenToWorld(camera, corner)` →
  world coordinates → `chunkOf(x, y, chunkSize)` for each corner →
  axis-aligned bounding box of chunk coords + `margin` ring → clamped to
  `[0, ceil(worldTiles/chunkSize))`.
- `chunkOf` is the local TS equivalent of the backend's `chunk_of`:
  `(Math.floor(x / chunkSize), Math.floor(y / chunkSize))`. Floor (not
  truncation) so negative world coords map consistently with the Rust
  `div_euclid` used server-side. Place `chunkOf` either inline in
  `viewportChunks.ts` or as a sibling export.
- No side effects, no dependencies on camera-state objects beyond reading
  the three fields. Trivial to unit-test.

**`src/backend/chunkSubscriptionClient.ts` (modified)**
- Keep `computeInitialSubscriptionCoords` as a fallback / unused
  (deprecate-comment, remove in 7b once viewport-driven is validated).
  Actually — remove it; the initial subscription should also be
  viewport-driven so we don't ship a dead path.
- New shape:
  ```ts
  createSubscriptionClient({
    send: (text: string) => void,
  }): {
    update(visible: ChunkCoord[]): void,
    reset(): void,
  }
  ```
- Internal state: `Set<string>` (where the string is `"${x},${y}"`) of
  currently-subscribed chunks.
- `update(visible)`:
  1. Build set-of-strings from `visible`.
  2. `added = visible \ prev`, `removed = prev \ visible`.
  3. If both empty → return without sending.
  4. If `added.size > 0` → `send(encodeClientMessage({ type:
     'chunk_subscribe', protocol_version: 1, coords: added }))`.
  5. If `removed.size > 0` → analogous `chunk_unsubscribe`.
  6. Replace internal set.
- `reset()`: clear internal state (called on socket close / reconnect, so
  the next `update` re-subscribes from scratch).

**`src/backend/mobilityClient.ts` (modified)**
- `openSocket()`:
  - Remove the hard-coded `worldWidthTiles: 256, worldHeightTiles: 256,
    chunkSize: 32` literal.
  - Accept a `getCamera() => CameraState | null` and `getViewportSize() =>
    { width, height } | null` and `getWorldDims() => { widthTiles,
    heightTiles, chunkSize }` from the caller (so the WS layer doesn't
    depend on render internals).
  - On `socket.onopen`, create the subscription client and install a
    `setInterval(updateSubscription, 200)` that:
    ```ts
    const camera = getCamera();
    const viewport = getViewportSize();
    const world = getWorldDims();
    if (!camera || !viewport) return;
    const visible = visibleChunks(camera, viewport, world, world.chunkSize, 1);
    subscription.update(visible);
    ```
  - Also call `updateSubscription()` once immediately on open so the
    initial subscribe happens without a 200 ms wait.
  - On `socket.onclose`, `clearInterval` + `subscription.reset()`.

**`src/main.ts` (or wherever the WS client is wired up)**
- Pass the three getters into `mobilityClient`.

## Data Flow

### Connect
1. WS socket opens.
2. Client builds initial `visibleChunks(...)` from current camera state.
3. Client sends a single `chunk_subscribe` with those coords.
4. Internal subscription set populated.

### Viewport tick (every 200 ms)
1. `setInterval` callback fires.
2. Read camera + viewport.
3. Compute `visible = visibleChunks(...)`.
4. Diff against internal set.
5. Send up to two messages (`chunk_subscribe(added)`,
   `chunk_unsubscribe(removed)`), suppressed when empty.
6. Replace internal set.

### Server on diff message
1. `handle_client_message` receives the message.
2. Acquires `runtime.write().await`.
3. Calls `apply_subscription_diff(added, removed)` (already exists from
   Phase 6 review pass).
4. Calls `synthetic_mobility_delta_for_subscription(...)` — this method is
   `&self` on `SimulationRuntime`; the mutable visibility caches it takes
   live on the per-task `ConnectionState`, not on the runtime. Could be
   split into a read after dropping the write, but keeping it inside the
   same write lock costs nothing meaningful (one WS message per client per
   pan-tick) and is simpler.
5. Releases.

### Server on per-tick broadcast
1. Each WS handler receives the raw `MobilityDelta` from the tokio
   broadcast channel.
2. Acquires `runtime.read().await`.
3. Calls `filtered_mobility_delta_from_dto(...)`.
4. Releases. Multiple clients run this in parallel under read locks.
5. If the filtered DTO is non-empty, sends to its socket.

### Disconnect
1. `stream_world_deltas` loop ends.
2. Cleanup block: `runtime.write().await` → `apply_subscription_diff(empty,
   connection.subscription)`.
3. Already implemented in Phase 6, just needs `.write()` instead of
   `.lock()`.

## Error Handling

- **Camera at world edge:** `visibleChunks` clamps the output to
  `[0, world_chunks_x)` × `[0, world_chunks_y)`.
- **Camera not yet initialised** (startup race before first frame): the
  `getCamera()` getter returns `null`; the subscription poll skips that
  iteration without error.
- **Subscription update during socket-not-open:** the `setInterval`
  callback checks `socket.readyState === OPEN` before calling
  `subscription.update`. (Or the `send` callback no-ops.)
- **Lock starvation:** tokio's `RwLock` is fair — pending writes block
  new read acquisitions, so writes always drain. Worst case is a brief
  read-side stall during the tick loop's write window.
- **WS send failure:** existing path — socket onclose handler fires,
  cleanup decrements server-side subscriber counts via
  `apply_subscription_diff(empty, connection.subscription)`.

## Testing

### Backend

**Existing tests pass unchanged:**
- `backend/crates/sim-server/tests/websocket.rs` (7 tests)
- `backend/crates/sim-server/tests/http.rs` (15 tests)
- Workspace `cargo test --workspace` (200/200)

**New tests:**
- `three_clients_with_disjoint_subscriptions_see_only_their_chunks` —
  extends `two_clients_with_different_subscriptions_see_different_entities`
  to 3 clients to exercise read-side parallelism.

### Frontend (vitest)

**New tests:**
- `tests/render/viewportChunks.test.ts`:
  - Returns expected chunks for a centred camera at default zoom.
  - Returns clamped chunks at world corners (no negative coords).
  - Includes the 1-chunk margin ring.
  - Handles `margin=0` (tight).
  - Handles extreme zoom-out (visible chunks > world chunks → entire world,
    clamped).
- `tests/backend/chunkSubscriptionClient.test.ts`:
  - `update` with empty prev sends `chunk_subscribe` only.
  - `update` with full overlap sends nothing (no-op).
  - `update` with partial overlap sends both subscribe and unsubscribe.
  - `update` called twice with same input sends nothing the second time.
  - `reset` clears state so the next `update` re-subscribes from scratch.

### Manual verification

- `npm run dev:stack`, open browser.
- Open DevTools → Network → WS frames.
- Pan camera: observe `chunk_subscribe` + `chunk_unsubscribe` flowing.
- Zoom in: `chunk_unsubscribe` for chunks falling off screen.
- Zoom out: `chunk_subscribe` for newly-visible chunks (capped at world).

## Concrete sequencing

The implementation plan (next step, via `superpowers:writing-plans`) will
split into these milestones, each its own commit:

1. Add `visibleChunks` pure function + its unit tests.
2. Refactor `chunkSubscriptionClient` to stateful `update`/`reset` shape +
   unit tests.
3. Wire `mobilityClient` polling + getters.
4. Remove the now-unused `computeInitialSubscriptionCoords`.
5. Backend: `Arc<Mutex>` → `Arc<RwLock>` mechanical swap in `app.rs`,
   adapt existing tests.
6. Add `three_clients_with_disjoint_subscriptions` integration test.
7. Manual browser smoke + commit progress note.

## Risks

- **Frontend wiring complexity.** The `main.ts` wiring of camera/viewport
  getters into the WS client may reveal that the current main is too
  monolithic. Plan to add small getters; if `main.ts` resists, accept a
  small refactor scoped to the wiring need.
- **RwLock semantics on tokio.** Writes block new readers but not
  currently-held ones. Under sustained write pressure (every 100 ms tick
  +  occasional snapshot every 5 s) reader latency should be measured
  but should stay well under one tick.
- **200 ms poll interval is arbitrary.** Picked because it's small enough
  for smooth subscription tracking and large enough to bound WS traffic
  (500 clients × 5 updates/sec = 2500 msg/s worst case if every tick
  every client moves). Empirical tuning may move this; the interval is
  a single constant.

## Open questions

None for Phase 7a. Phase 7b will brainstorm per-chunk channels separately
once 7a is validated under load.
