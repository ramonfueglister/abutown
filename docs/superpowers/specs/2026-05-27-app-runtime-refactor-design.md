# App Runtime Refactor

**Date:** 2026-05-27
**Status:** Design

## Goal

Split the browser app runtime into focused modules without changing gameplay, visuals, backend protocol behavior, or Supabase login behavior.

The first refactor target is `src/main.ts`. It currently owns boot, backend gating, card-hand login mounting, canvas rendering, camera/input, selection, diagnostics, map data conversion, and test hooks in one file. This makes small runtime changes risky: the login button disappeared when the minimal-map renderer was merged because the mount call was easy to drop from the monolith.

## Scope

In scope:

- Keep the current minimal-motorways visual output.
- Keep backend-required startup semantics.
- Keep backend-driven pedestrians, vehicles, and trains visible and moving.
- Keep card-hand login mounted when Supabase configuration exists.
- Keep the signed-out card-hand status hidden.
- Keep `window.render_game_to_text` and `window.advanceTime` behavior stable for Playwright and smoke tests.
- Extract cohesive modules from `src/main.ts` with explicit interfaces.
- Add regression tests for runtime wiring so UI mounts and diagnostics are not lost again.
- Update existing e2e tests only where imports or diagnostics move, not to weaken assertions.

Out of scope:

- No visual redesign.
- No new UI controls.
- No backend mobility algorithm changes.
- No route execution, flow-field, or protocol changes.
- No new asset pipeline.
- No broad CSS rewrite beyond moving styles only if a component boundary requires it.
- No replacement of existing render primitives with a framework.

## Design Choice

Use a behavior-preserving modular extraction.

### Option A: Runtime composition first

Extract boot, backend gate, card-hand mount, diagnostics, input, and render orchestration behind small interfaces while keeping the map renderer mostly unchanged at first. This is selected because it directly prevents repeated missing-mount regressions and gives later visual work a stable shell.

### Option B: Renderer rewrite first

Move every draw function into a renderer class immediately. This would reduce `main.ts` more aggressively, but it touches the largest visual surface before runtime wiring is safe.

### Option C: Big-bang app architecture rewrite

Introduce a full app state container, scene graph, and component system. This is too broad for the current app and would hide behavior regressions behind structural churn.

## Target Architecture

`src/main.ts` becomes the composition root:

```text
src/main.ts
  - import CSS
  - resolve backend URL
  - find the canvas
  - build the Zurich world context
  - call startAppRuntime(...)
```

The extracted modules own the following responsibilities.

### `src/app/appRuntime.ts`

Owns startup and shutdown:

- `startAppRuntime(options): Promise<AppRuntimeHandle>`
- Calls `requireBackend`.
- Calls `requireMobilitySnapshot`.
- Mounts the card-hand view with the resolved backend URL.
- Starts canvas boot/render loop only after backend and initial mobility state are available.
- Starts `connectMobilityBackend`.
- Registers `beforeunload` cleanup.
- Calls backend-required rendering when startup fails.

The runtime must make card-hand mounting explicit in code and tests. The call must not be hidden inside renderer construction.

### `src/app/backendRequiredView.ts`

Owns the backend-required fail-closed DOM/canvas view:

- `renderBackendRequired(options): void`
- `escapeHtml(value): string`

This module removes and re-creates only `[data-backend-required]`. It does not know about card-hand login or mobility state.

### `src/app/interaction.ts`

Owns browser input:

- Pointer drag.
- Wheel zoom.
- Click selection.
- Resize listener attachment.
- Camera constraints via callbacks.

It receives functions for screen-to-world conversion and entity selection. It does not import mobility state directly.

### `src/app/entitySelection.ts`

Owns selected entity ids and hit testing:

- `createEntitySelection(options): EntitySelection`
- Select nearest backend pedestrian or vehicle by screen point.
- Clear the other selection category when one category is selected.
- Expose selected pedestrian and selected vehicle getters.

This keeps selection behavior testable without a canvas.

### `src/render/minimalMapRenderer.ts`

Owns canvas drawing for the current minimal map:

- Terrain.
- Roads and edge exits.
- Rails and train.
- Details, buildings, trees.
- Backend cars and pedestrians.
- Inspector panels.
- Perimeter mist.

It receives a `MinimalMapRenderState` object and a camera state. It does not fetch backend data and does not mutate global browser state.

The first implementation moves draw functions mechanically and preserves their current draw order. Optimization and aesthetic cleanup are later work.

### `src/app/zurichRuntimeContext.ts`

Builds the static world context:

- Zurich world.
- Transport.
- Placement.
- Validation.
- Runtime maps for terrain, roads, rails, buildings, trees, details, stations, and train paths.
- Static diagnostics helpers.

This module absorbs map-building helper functions that are still required at runtime. Functions that are no longer used after extraction are deleted rather than preserved as second-path code.

### `src/app/runtimeDiagnostics.ts`

Owns test and debug hooks:

- `installRuntimeDiagnostics(options): void`
- `window.render_game_to_text`.
- `window.advanceTime`.
- `loadedRasterAssetPaths()` remains `[]` for the minimal vector renderer.

Diagnostics must keep the existing JSON shape unless a test and spec explicitly update it. The e2e suite depends on this output as an app contract.

## Data Flow

Startup:

```text
main.ts
  -> createZurichRuntimeContext()
  -> startAppRuntime()
      -> requireBackend()
      -> requireMobilitySnapshot()
      -> mountCardHandView()
      -> boot canvas renderer
      -> connectMobilityBackend()
```

Render loop:

```text
requestAnimationFrame
  -> update train offsets
  -> constrain and damp camera
  -> project backend mobility state into drawables
  -> minimalMapRenderer.render(...)
```

Input:

```text
pointer / wheel event
  -> interaction module
  -> camera module or entitySelection
  -> selected ids
  -> next render draws selected inspector
```

Diagnostics:

```text
window.render_game_to_text()
  -> runtimeDiagnostics reads current context/state through getters
  -> returns the same JSON contract as today
```

## Error Handling

- Backend startup failure still renders the backend-required panel and marks the canvas with `data-ready="false"` and `data-backend-required="true"`.
- Missing canvas or 2D context remains a hard startup error.
- Card-hand login mounting returns before DOM mutation when Supabase env vars are missing.
- Runtime modules fail loudly for missing required dependencies. They must not invent coordinates, entities, snapshots, render state, or second data paths.

## Testing

Required regression coverage:

- Unit test that `startAppRuntime` mounts the card-hand login view with the backend base URL after the initial mobility snapshot succeeds.
- Unit test that startup failure calls `renderBackendRequired` and does not start the render loop.
- Unit test for `entitySelection` pedestrian/vehicle selection precedence and clearing behavior.
- Unit test that `runtimeDiagnostics` preserves the minimal renderer contract:
  - `visualStyle.id === "minimal-motorways"`
  - `visualStyle.spriteDrawing === "disabled"`
  - `loadedRasterAssetPaths === []`
- Existing `tests/e2e/render-smoke.spec.ts` remains the browser acceptance test for:
  - canvas ready
  - backend mobility counts
  - visible vehicles
  - movement over time
  - no retired asset requests
  - entity click selection

Quality gates:

```bash
npm test
npm run build
npm run test:e2e
rg -n "fallback|fall back|unwrap_or\\(\\(0\\.0, 0\\.0\\)\\)|at_activity with empty|synthetic link|global A\\*" backend/crates/sim-core/src backend/crates/sim-server/src src tests -g '!src/backend/proto/**' -g '!backend/target/**'
```

The forbidden-path grep must return no production behavior that degrades into invented state, invented coordinates, invented entities, invented snapshots, or second routing/rendering paths. Touched code must not add comments that describe rejected invented behavior.

## Migration Strategy

Implement in narrow, reviewable commits:

1. Add seams and tests around runtime startup without moving renderer code.
2. Extract backend-required view.
3. Extract runtime startup orchestration.
4. Extract entity selection and interaction.
5. Extract diagnostics.
6. Extract minimal map renderer and static Zurich runtime context.
7. Delete unused historical helpers from `main.ts`.
8. Run full browser smoke, forbidden-path, and retired-asset sweeps.

After each extraction, the app must compile and the focused tests for that boundary must pass. The final state reduces `src/main.ts` to a small composition root and leaves each extracted module independently testable.

## Acceptance Criteria

- `src/main.ts` is no longer the owner of rendering, diagnostics, input, and backend startup details.
- Card-hand login remains mounted after successful backend startup.
- The signed-out card-hand status remains hidden.
- The minimal vector map still reports no raster assets.
- Pedestrians and cars are still backend-driven and visibly moving.
- Entity selection still works for pedestrians and vehicles.
- Backend-required startup failure still produces the existing panel.
- No retired Pak128/Simutrans/OpenGFX asset references return in runtime or tests.
- No invented or second-path behavior is introduced.
- Full test and build gates pass.

## Risks

- Moving draw helpers can subtly change render order. The first renderer extraction is mechanical and preserves function order.
- Diagnostics are used as an e2e contract. Move them behind getters instead of rewriting their JSON shape.
- `startAppRuntime` can become a new monolith if it absorbs rendering details. It orchestrates dependencies and does not draw.
- Existing login behavior depends on environment configuration. Tests assert the mount call and existing no-config non-mount separately.
