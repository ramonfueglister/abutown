# Mobility Client Bridge Design

Date: 2026-05-15

## Status

Approved for implementation in the `codex/mobility` worktree by the user's request to plan and execute a mobility slice.

## Goal

Connect the existing browser city to the Rust mobility backend so the client can show server-owned agent mobility state without replacing the local canvas simulation.

## Context

The Rust backend already exposes:

- `GET /mobility` for a seeded mobility snapshot.
- `GET /ws` for `tile_pulse` and `mobility_delta` messages.
- Core mobility states for walking, waiting, riding, and alighting.

The current Vite client has no `src/backend` bridge and still renders cars, pedestrians, and trains from local deterministic data only. This slice adds a narrow bridge for mobility data and a diagnostic in-scene render path.

## Chosen Approach

Implement a focused TypeScript mobility bridge.

When enabled, the bridge fetches `/mobility`, subscribes to `/ws`, reduces `mobility_delta` messages into a local read model, and leaves the existing local animation untouched. Rendering stays diagnostic: a small server-agent marker, waiting stop markers, and a connection summary in `render_game_to_text`.

This is preferable to replacing local pedestrians because the backend currently seeds only one demo agent and one shuttle. A full replacement would make the city feel empty and would force premature routing/pathfinding work.

## Alternatives Considered

- Full frontend authority switch to backend mobility: rejected because the backend seed is intentionally tiny and lacks city-scale paths.
- Backend-only extension: rejected because the user-visible gap is that the browser does not consume mobility yet.
- Reusing the old visible-backend plan verbatim: rejected because it is broader than mobility and its planned frontend files are not present in this branch.

## Architecture

Add three focused frontend modules:

- `src/backend/mobilityProtocol.ts`: TypeScript DTOs and runtime guards for mobility snapshots and WebSocket messages.
- `src/backend/mobilityState.ts`: pure reducer/read model for snapshot load, deltas, connection status, and deterministic projection of mobility entities to demo grid coordinates.
- `src/backend/mobilityClient.ts`: browser bridge that fetches the snapshot, opens the WebSocket, applies messages, and reconnects conservatively.

Add one focused renderer:

- `src/render/mobilityOverlay.ts`: canvas drawing helpers for server-owned agent, vehicle, and stop markers.

`src/main.ts` owns lifecycle wiring only: create bridge, update state during boot, render overlay after the scene, and expose mobility diagnostics through `window.render_game_to_text`.

## Data Flow

1. Browser boots the existing local city at `/`.
2. The game projects rendered pedestrians into local agent records by default.
3. The render loop exposes local pedestrian-agent diagnostics through `render_game_to_text`.
4. Clicking a pedestrian selects the corresponding local agent and updates the inspector.

The local pedestrian-agent layer does not require a URL flag, localStorage flag, or backend network request.

## Error Handling

- Invalid snapshot JSON marks mobility as disconnected and keeps rendering local city.
- Invalid WebSocket messages are ignored and counted.
- WebSocket close/error marks the bridge disconnected and schedules reconnect with a bounded delay.
- No browser alert or blocking UI is introduced.

## Testing

- Unit tests cover protocol guards for snapshot and mobility-delta payloads.
- Unit tests cover reducer behavior for snapshots, deltas, invalid messages, and projection.
- Render tests cover marker generation without requiring a browser canvas.
- Existing `npm test`, `npm run build`, and backend Cargo tests must pass.

## Non-Goals

- No full backend-driven population.
- No city-scale routing, pathfinding, lane traffic, congestion, or parking.
- No replacement of local traffic actors, pedestrians, or train.
- No player commands or persistence changes.

## Success Criteria

- With only Vite running, the city renders local pedestrian agents by default and reports mobility as `local-pedestrians`.
- The browser loads from `http://127.0.0.1:5175/` without query parameters.
- `window.render_game_to_text()` includes mobility status, local pedestrian-agent count, selected agent id, and current selected-agent inspector state.
- Tests and build pass in the mobility worktree.

## Self-Review

- Placeholder scan: no unfinished placeholder markers.
- Internal consistency: the bridge reads existing backend endpoints and does not alter backend authority.
- Scope check: one frontend integration slice; backend changes are not required.
- Ambiguity check: rendering is diagnostic and additive, not a city-scale simulation replacement.
