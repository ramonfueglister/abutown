# Visible Backend Slice

Date: 2026-05-14

## Status

Approved design for the first browser-to-Rust vertical slice after the simulation foundation.

## Goal

Prove the core runtime path visibly inside the existing Abutown browser scene:

```text
Rust authoritative state -> HTTP snapshot -> WebSocket deltas -> visible browser reaction
```

The player should still see the current Zurich world. On top of it, the client shows that Rust is live and authoritative for a small server-owned world slice.

## Non-Goals

- No Supabase/Postgres persistence.
- No authentication or player accounts.
- No economy, citizens, logistics, ledger, combat, or production mechanics.
- No attempt to server-author the full Zurich map.
- No 2000-player load target in this slice.

## User Experience

The existing city remains the primary view. After boot, the client connects to the Rust server and shows a small in-world backend signal:

- a compact `RUST LIVE` badge with connection, world, chunk, tick, and version state,
- a visible outline for the authoritative server chunk,
- a short pulse or marker when the server emits a tile/chunk delta.

If the backend is unavailable, the city still renders locally and the badge shows a disconnected state. The failure mode should be obvious but not block visual development.

## Backend Design

`sim-server` keeps a small in-memory authoritative runtime for `abutown-main`.

Required server surfaces:

- `GET /health`: existing protocol/version health.
- `GET /world`: loaded chunk summary.
- `GET /chunks/{x}/{y}`: current chunk snapshot.
- `GET /ws`: WebSocket stream for versioned server deltas.

The server owns tick/version counters and emits a low-frequency visible delta, roughly once per second. The first delta can be intentionally simple, such as a marker position or tile pulse inside chunk `0:0`. The important property is that the browser does not invent the event; it renders what Rust sends.

## Protocol Design

Extend `abutown-protocol` with versioned WebSocket messages:

- server hello with protocol version and world ID,
- chunk/tile delta message with tick, version, chunk coordinate, tile index or marker coordinate,
- optional error or resync hint for unsupported protocol/gap cases.

Messages must remain JSON for this slice so they are easy to inspect in browser devtools and Rust tests. Binary packing is a later optimization.

## Frontend Design

Add a small client-side backend bridge that is separate from the renderer:

- fetches `/health`, `/world`, and the first visible chunk snapshot,
- opens the WebSocket stream,
- stores connection status, latest tick/version, loaded chunk, and recent pulses,
- exposes a minimal render-friendly state object.

The existing canvas renderer then draws the backend overlay after the city scene: chunk outline, pulse marker, and status badge. The overlay is diagnostic but visible in the real game view, not a separate debug page.

## Error Handling

- HTTP failure: mark backend disconnected and keep local city rendering.
- WebSocket close/error: show stale/disconnected badge and attempt conservative reconnect.
- Protocol mismatch: show incompatible state and do not apply deltas.
- Unknown chunk/tile delta: ignore the delta and record a lightweight client warning.

## Testing

Backend:

- protocol serialization tests for WebSocket messages,
- HTTP tests remain green,
- WebSocket integration test receives hello plus at least one delta.

Frontend:

- unit tests for backend state transitions and protocol guards,
- renderer/overlay test for visible pulse state where practical,
- existing `npm test` and `npm run build` remain green.

End-to-end manual verification:

- run Rust server,
- run Vite client,
- open the game,
- confirm city renders,
- confirm `RUST LIVE` badge appears,
- confirm server-driven pulse/marker changes over time.

## Success Criteria

The slice is successful when a user can run the Rust server and browser client locally and see a live backend-owned signal inside the existing game scene without changing the core city renderer into a server-only renderer.
