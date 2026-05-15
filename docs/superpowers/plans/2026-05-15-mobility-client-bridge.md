# Mobility Client Bridge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connect the Vite client to the Rust mobility backend when explicitly enabled and render additive server-owned mobility markers.

**Architecture:** Keep the Rust backend authoritative and the browser read-only. Add focused TypeScript modules for protocol guards, immutable mobility state reduction, a browser client bridge, and a canvas overlay; wire them into `src/main.ts` without replacing the existing local cars, pedestrians, or train.

**Tech Stack:** TypeScript, Vite, Vitest, Canvas 2D, existing Rust Axum backend on `127.0.0.1:8080`.

---

## File Structure

- Create `src/backend/mobilityProtocol.ts`: protocol DTO types and runtime guards for mobility snapshots and WebSocket messages.
- Create `src/backend/mobilityState.ts`: immutable read model, delta reducer, connection state, demo coordinate projection, and diagnostics.
- Create `src/backend/mobilityClient.ts`: browser-side fetch/WebSocket bridge with conservative reconnect.
- Create `src/render/mobilityOverlay.ts`: marker drawing and draw-item preparation.
- Modify `src/main.ts`: instantiate the mobility bridge, render markers, and expose diagnostics in `render_game_to_text`.
- Create `tests/backend/mobilityProtocol.test.ts`: protocol guard tests.
- Create `tests/backend/mobilityState.test.ts`: reducer and projection tests.
- Create `tests/render/mobilityOverlay.test.ts`: render draw-item tests.

## Task 1: Protocol Guards

**Files:**
- Create: `src/backend/mobilityProtocol.ts`
- Create: `tests/backend/mobilityProtocol.test.ts`

- [ ] **Step 1: Add failing protocol tests**

Create `tests/backend/mobilityProtocol.test.ts` with tests for accepting a valid mobility snapshot, accepting a `mobility_delta` message, and rejecting malformed payloads.

- [ ] **Step 2: Run failing protocol tests**

Run:

```bash
npm test -- tests/backend/mobilityProtocol.test.ts
```

Expected: FAIL because `src/backend/mobilityProtocol.ts` does not exist.

- [ ] **Step 3: Implement protocol guards**

Create `src/backend/mobilityProtocol.ts` with DTO types for `MobilitySnapshotDto`, `MobilityDeltaDto`, `AgentMobilityDto`, `VehicleMobilityDto`, `StopMobilityDto`, `AgentMobilityStateDto`, and `ServerMessageDto`. Export `isMobilitySnapshotDto`, `isMobilityDeltaDto`, and `parseServerMessage`.

- [ ] **Step 4: Verify protocol tests pass**

Run:

```bash
npm test -- tests/backend/mobilityProtocol.test.ts
```

Expected: PASS.

## Task 2: Mobility State Reducer

**Files:**
- Create: `src/backend/mobilityState.ts`
- Create: `tests/backend/mobilityState.test.ts`

- [ ] **Step 1: Add failing state tests**

Create tests that verify: initial disconnected state, snapshot application stores agents/vehicles/stops, `mobility_delta` replaces changed records, invalid messages increment `invalidMessages`, and the seeded walking agent projects to a Zurich-grid coordinate.

- [ ] **Step 2: Run failing state tests**

Run:

```bash
npm test -- tests/backend/mobilityState.test.ts
```

Expected: FAIL because `src/backend/mobilityState.ts` does not exist.

- [ ] **Step 3: Implement state reducer and projection**

Create `src/backend/mobilityState.ts`. It should expose `createMobilityOverlayState`, `applyMobilitySnapshot`, `applyMobilityDelta`, `applyServerMessage`, `markMobilityConnecting`, `markMobilityDisconnected`, `mobilityMarkers`, and `mobilityDiagnostics`.

- [ ] **Step 4: Verify state tests pass**

Run:

```bash
npm test -- tests/backend/mobilityState.test.ts
```

Expected: PASS.

## Task 3: Mobility Overlay Renderer

**Files:**
- Create: `src/render/mobilityOverlay.ts`
- Create: `tests/render/mobilityOverlay.test.ts`

- [ ] **Step 1: Add failing overlay tests**

Create tests that verify `buildMobilityOverlayDrawItems` returns visible agent/vehicle/stop draw items and respects an optional visibility predicate.

- [ ] **Step 2: Run failing overlay tests**

Run:

```bash
npm test -- tests/render/mobilityOverlay.test.ts
```

Expected: FAIL because `src/render/mobilityOverlay.ts` does not exist.

- [ ] **Step 3: Implement overlay helpers**

Create `src/render/mobilityOverlay.ts` with `buildMobilityOverlayDrawItems` and `drawMobilityOverlay`. The draw function should use compact filled shapes and labels only at tiny scale so it stays diagnostic and does not dominate the game scene.

- [ ] **Step 4: Verify overlay tests pass**

Run:

```bash
npm test -- tests/render/mobilityOverlay.test.ts
```

Expected: PASS.

## Task 4: Browser Bridge

**Files:**
- Create: `src/backend/mobilityClient.ts`

- [ ] **Step 1: Implement browser bridge**

Create `src/backend/mobilityClient.ts` with `connectMobilityBackend(options)`. The bridge should fetch `/mobility`, validate it, open `/ws`, apply `mobility_delta` messages, ignore non-mobility messages, mark disconnected on fetch/WebSocket failure, and reconnect with a bounded delay unless stopped. The current game path keeps mobility in the default local-pedestrian layer, so runtime startup must not require URL or localStorage flags for pedestrian agents.

- [ ] **Step 2: Type-check bridge through full build**

Run:

```bash
npm run build
```

Expected: PASS or fail only on later missing `main.ts` wiring, which Task 5 resolves.

## Task 5: Main Runtime Integration

**Files:**
- Modify: `src/main.ts`

- [ ] **Step 1: Wire mobility state into main**

Modify `src/main.ts` to import the bridge/state/overlay modules, create a `mobilityState` variable, start `connectMobilityBackend` during boot, draw `drawMobilityOverlay` after dynamic city drawables, and include `mobilityDiagnostics(mobilityState)` under `city.mobility` in `render_game_to_text`.

- [ ] **Step 2: Verify local no-backend behavior**

Run:

```bash
npm test -- tests/e2e/render-smoke.spec.ts
```

Expected: existing e2e smoke remains green without requiring the Rust server.

## Task 6: Full Verification

**Files:**
- All changed files.

- [ ] **Step 1: Run frontend unit tests**

Run:

```bash
npm test
```

Expected: PASS.

- [ ] **Step 2: Run frontend build**

Run:

```bash
npm run build
```

Expected: PASS.

- [ ] **Step 3: Run backend tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml --workspace
```

Expected: PASS.

- [ ] **Step 4: Commit implementation**

Run:

```bash
git add src/backend src/render/mobilityOverlay.ts src/main.ts tests/backend tests/render/mobilityOverlay.test.ts docs/superpowers/plans/2026-05-15-mobility-client-bridge.md
git commit -m "feat: connect client mobility bridge"
```

## Self-Review

Spec coverage:

- Protocol guards: Task 1.
- Snapshot/delta state: Task 2.
- Diagnostic marker rendering: Task 3.
- Browser fetch/WebSocket bridge: Task 4.
- Runtime integration and diagnostics: Task 5.
- Verification: Task 6.

Placeholder scan:

- No unfinished placeholder markers.

Type consistency:

- Protocol DTO names match backend JSON fields.
- State reducer uses `Map` internally and diagnostics expose counts for JSON output.
- Renderer consumes `mobilityMarkers` output rather than duplicating protocol logic.
