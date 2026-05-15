# Backend-Required Mobility Runtime Design

## Goal

Make the browser game require the Rust backend before any runtime simulation starts, then use the backend mobility snapshot and websocket stream as the authoritative mobility source.

## Current State

The frontend renders the pak128 Zurich map immediately and derives local pedestrians and road vehicles from canvas runtime data. The Rust backend already exposes `/health`, `/mobility`, and `/ws`, and the frontend already has protocol and reducer modules for backend mobility data. That bridge is not wired into `src/main.ts`, and existing local mobility diagnostics still report `local-mobility` even though the project direction is backend-required.

Card hand is already backend-required via `127.0.0.1:8080`; mobility should follow the same rule.

## Requirements

- The browser must not boot the canvas runtime unless `GET http://127.0.0.1:8080/health` returns a valid healthy backend response.
- If the backend is missing or unhealthy, the page must show a clear “Backend required” message and must not set `#game[data-ready="true"]`.
- The frontend must load `GET /mobility` from the same backend before starting the canvas render loop.
- `window.render_game_to_text()` must report mobility as backend-sourced, including backend connection status, tick, agent count, vehicle count, stop count, invalid message count, and last error.
- Existing pak128 map rendering, local pedestrian click selection, and local road vehicle click selection remain visible and usable in this slice.
- Existing local `localAgents` and `localVehicles` diagnostics remain available, but they are explicitly separate from backend mobility counts.
- There is no alternate local runtime path to Vite-origin `/mobility`, `/cards`, or `/card-hand`.
- Missing card definitions, invalid mobility payloads, and unhealthy backend responses are hard errors.
- Playwright E2E must start both the Rust backend and frontend preview so the test environment matches the required runtime contract.

## Architecture

Add a focused backend gate module that owns backend URL resolution, health response validation, and a `requireBackend()` startup check. `src/main.ts` calls this gate before card-hand mount, asset loading, entity generation, event listeners, and animation scheduling. On failure, it renders a small fatal runtime status panel and stops.

Extend the existing mobility client so `127.0.0.1:8080` is the default backend URL and so an initial mobility snapshot can be required before canvas startup. After that required snapshot succeeds, `connectMobilityBackend()` keeps the mobility state fresh through `/ws` deltas.

Update diagnostics to make backend mobility authoritative while keeping local manifest entities separate:

- `city.mobility.source = "backend"`
- `city.mobility.status = "connected" | "connecting" | "disconnected"`
- `city.mobility.agents`, `vehicles`, and `stops` come from backend mobility state.
- `city.mobility.localAgents` and `city.mobility.localVehicles` expose the currently manifested canvas counts.

## Error Handling

Backend startup failures are fatal to the frontend runtime. The user-facing message should name the backend URL and the concrete failure. Runtime websocket disconnects after startup are reported in backend mobility diagnostics; the map can remain visible because it was started from a valid backend snapshot.

## Testing

- Unit-test backend health validation and URL defaults.
- Unit-test required mobility snapshot loading and backend URL defaults.
- Update render smoke E2E to assert backend-sourced mobility and run with both backend and frontend preview servers.
- Verify the no-backend path by directly testing the backend gate module; the E2E suite should model the normal required-stack path.
