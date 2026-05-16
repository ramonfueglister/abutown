# Mobility Frame Interpolation Design

**Date:** 2026-05-16
**Status:** Approved for planning — Phase 2 of the Million-Agent Roadmap.
**Roadmap:** `docs/superpowers/specs/2026-05-16-million-agent-roadmap-design.md`
**Predecessor:** `docs/superpowers/specs/2026-05-16-visible-backend-mobility-design.md` (Phase 1 merged on `main`)
**Successor (out of scope):** Phase 3 procedural population scale-up.

---

## Goal

Make mobility entities glide smoothly across the browser canvas. Today the backend ticks at 1 Hz and entities visibly jump once per second; after this phase the backend ticks at 10 Hz and the frontend interpolates linearly between the last two server states per entity, producing 60 fps smooth motion without changing the simulation model.

## Non-Goals

- Per-tile pathfinding or dynamic routing — paths remain pre-baked.
- Hermite splines or velocity-hint DTOs — linear `lerp` only.
- Direction smoothing (mid-tick sprite rotation) — sprite frame remains discrete per tick.
- Server-side sub-tick simulation or extrapolation past the last snapshot.
- Viewport-filtered replication (Phase 4).
- ECS storage migration (Phase 5).
- Bandwidth optimization (Phase 4+).
- Changing the seeded speed values — the existing `speed_per_tick` numbers stay the same. At 10 Hz they produce ~10× the per-second movement of the 1 Hz era, which is the desired "walking pace" range without any seeder edits.

## Architecture

Three coordinated changes:

1. **Backend ticks at 10 Hz.** `SIMULATION_TICK_INTERVAL` becomes `Duration::from_millis(100)`. Snapshot loop is unchanged. World-summary DTO exposes `tick_period_ms: u32 = 100` so the client knows how to advance its interpolation cursor.

2. **Frontend state buffers the previous and current server position per entity.** `MobilityOverlayState`'s `agents` and `vehicles` maps, and `RoadVehicleOverlayState`'s `vehicles` map, change shape from `Map<id, Dto>` to `Map<id, { prev, current, lastTickAt }>`. Reducers maintain the buffer: snapshot sets `prev = current = dto` (no animation on initial load), delta moves the previous `current` into `prev` and sets the new dto as `current`.

3. **Drawables projector interpolates per render frame.** `pedestriansFromMobilityState` and `carsFromMobilityState` take a `now: number` parameter. For each entity, `t = clamp((now - lastTickAt) / tickPeriodMs, 0, 1)`, interpolated coord is `lerp(prev.world_coord, current.world_coord, t)`. Direction is read from `current` (no interpolation — discrete sprite frame).

After this phase: backend authority unchanged, network shape unchanged (just at 10× rate), frontend renders smoothly between tick boundaries.

## Data Model Changes

### Protocol

`WorldSummaryDto` adds one field:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSummaryDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub chunk_size: u16,
    pub loaded_chunks: Vec<ChunkCoordDto>,
    pub tick_period_ms: u32,
}
```

No changes to `AgentMobilityDto`, `VehicleMobilityDto`, `RoadVehicleDto`, `MobilityDeltaDto`, `RoadVehicleDeltaDto`. The wire shape of mobility messages is unchanged.

### Backend constants

`backend/crates/sim-server/src/app.rs`:

```rust
const SIMULATION_TICK_INTERVAL: Duration = Duration::from_millis(100);
```

(Was `Duration::from_secs(1)`.)

`SNAPSHOT_INTERVAL` unchanged at 5 s.

### Frontend state shape

`src/backend/mobilityState.ts`:

```ts
export type InterpolatedEntry<T> = {
  prev: T;
  current: T;
  lastTickAt: number;
};

export type MobilityOverlayState = {
  status: MobilityConnectionStatus;
  tick: number;
  agents: Map<string, InterpolatedEntry<AgentMobilityDto>>;
  vehicles: Map<string, InterpolatedEntry<VehicleMobilityDto>>;
  stops: Map<string, StopMobilityDto>;
  roadVehicles: RoadVehicleOverlayState;
  invalidMessages: number;
  lastError: string | null;
  lastUpdatedAt: number;
};
```

`StopMobilityDto` stays unbuffered (stops don't move).

`src/backend/roadVehicleState.ts`:

```ts
export type RoadVehicleOverlayState = {
  tick: number;
  vehicles: Map<string, InterpolatedEntry<RoadVehicleDto>>;
  invalidMessages: number;
  lastUpdatedAt: number;
};
```

## Data Flow

### Backend tick

```text
every 100 ms:
  runtime.next_server_messages():
    - MobilityDelta(changed_agents, changed_vehicles)
    - RoadVehicleDelta(changed road_vehicles)
  broadcast to all /ws clients
```

### Frontend snapshot ingestion

On boot:

1. `GET /world` → cache `tickPeriodMs = world.tick_period_ms` (100).
2. `GET /mobility` snapshot → for every agent/vehicle: `prev = current = dto`, `lastTickAt = now`.
3. `GET /road-vehicles` snapshot → same pattern.
4. Open WebSocket.

### Frontend delta ingestion

On each `mobility_delta` / `road_vehicle_delta`:

For every entity in the delta's changed list:
- If the entity already exists in the map: `prev = entry.current`, `current = dto`, `lastTickAt = now`.
- If the entity is new (first delta for this id): `prev = current = dto`, `lastTickAt = now`.

Entities not present in the delta are not touched (they remain at their last `current` with `t` ramping toward 1.0).

### Frontend render frame

Each animation frame:

```ts
const now = performance.now();
const pedestrians = pedestriansFromMobilityState(mobilityState, pedestrianSprites, tickPeriodMs, now);
const cars = carsFromMobilityState(mobilityState, vehicleSprites, tickPeriodMs, now);
```

Inside the projector:

```ts
const t = Math.min(1, Math.max(0, (now - entry.lastTickAt) / tickPeriodMs));
const x = entry.prev.world_coord.x + (entry.current.world_coord.x - entry.prev.world_coord.x) * t;
const y = entry.prev.world_coord.y + (entry.current.world_coord.y - entry.prev.world_coord.y) * t;
const direction = entry.current.direction;
```

`prev.world_coord` and `current.world_coord` are the canonical positions; `direction` is read from `current` only.

## Components

### Backend

- `backend/crates/protocol/src/lib.rs`: add `tick_period_ms: u32` to `WorldSummaryDto`. Update existing protocol tests.
- `backend/crates/sim-server/src/app.rs`: change `SIMULATION_TICK_INTERVAL` to `Duration::from_millis(100)`.
- `backend/crates/sim-server/src/runtime.rs` (or wherever `world_summary()` lives): populate `tick_period_ms: 100`.
- Existing tests asserting snapshot/delta latency may need adjustment (the websocket test that checks "no extra message within 500 ms" now needs to expect 5 ticks within 500 ms, not 0).

### Frontend

- `src/backend/mobilityState.ts`:
  - New `InterpolatedEntry<T>` type.
  - `MobilityOverlayState.agents` / `.vehicles` change to `Map<string, InterpolatedEntry<…>>`.
  - `applyMobilitySnapshot`: initialize `prev = current = dto`.
  - `applyMobilityDelta`: shift prev←current then set current=new dto.
  - Helper `interpolatedAgents(state, now, tickPeriodMs)` and `interpolatedVehicles(...)` return frame-local `Dto[]` with lerped `world_coord`.
- `src/backend/roadVehicleState.ts`:
  - `RoadVehicleOverlayState.vehicles` map shape changes.
  - Helper `interpolatedRoadVehicles(state, now, tickPeriodMs)`.
- `src/backend/mobilityProtocol.ts`: add `tick_period_ms: number` to the `WorldSummaryDto` shape if it's mirrored on the frontend. (Verify; if not currently typed, no change.)
- `src/backend/mobilityClient.ts`: `requireMobilitySnapshot` returns both the initial state and the `tickPeriodMs` from `/world`. Wire `/world` fetch into the existing boot path or accept that `mobilityClient` does not own the world summary — the existing `backendGate` may already fetch `/world` for the health check.
- `src/render/backendMobilityDrawables.ts`: `pedestriansFromMobilityState(state, sprites, now, tickPeriodMs)` and `carsFromMobilityState(state, sprites, now, tickPeriodMs)`. Internally call `interpolatedAgents` / `interpolatedRoadVehicles` then build drawables.
- `src/main.ts`: read `tickPeriodMs` once at boot (default 100 if `/world` doesn't provide it for backward-compat), pass `now` and `tickPeriodMs` to each projector call inside the render loop.

## Tests

### Backend unit tests

- `world_summary_includes_tick_period_ms`: `runtime.world_summary().tick_period_ms == 100`.
- `simulation_tick_interval_matches_world_summary`: cross-check the constant.

### Backend integration tests

- HTTP smoke: `GET /world` returns `tick_period_ms: 100`.
- WebSocket: at least 3 mobility deltas arrive within 500 ms (verifies the new tick rate).

### Frontend unit tests

- `applyMobilitySnapshot` produces entries with `prev === current` for every agent.
- `applyMobilityDelta` for an existing agent: `prev` equals the pre-delta `current`; `current` equals the new dto.
- `applyMobilityDelta` for a new agent: `prev === current`.
- `interpolatedAgents` at `t = 0` returns prev positions; at `t = 0.5` returns midpoint; at `t = 1` returns current; beyond `tickPeriodMs` clamps to 1.
- Same set of tests for road vehicles.

### Frontend integration (vitest)

- Mock `requireMobilitySnapshot` + delta → verify projected drawable position is the interpolated coord at the simulated `now`.

### E2E (Playwright)

- `render-smoke.spec.ts`: after `advanceTime(500)` (which now spans 5 backend ticks), assert the agent that was at known position P1 in the initial snapshot is at position P5+lerp, not still at P1. The exact assertion: agent positions differ between two render reads taken 50 ms apart (i.e. they animate).
- Reconnect path: simulate WS disconnect, verify `lastError` set and that the next snapshot/reconnect restores prev=current behavior.

## Error Handling

- `now - lastTickAt` going negative (system clock skew or backwards time): clamp `t = max(0, …)`.
- `tickPeriodMs == 0` or absent from `/world`: default to `100` on the client.
- New entity (no `prev`): `prev = current`, `t` always 1 → entity appears at exact position.
- WebSocket disconnect: the next snapshot fetch on reconnect resets every entry to `prev = current` again. No animation across the disconnect gap.
- Entity disappears from mobility state (not in subsequent deltas, not in next snapshot): the entry stays in the map until the next snapshot prunes it. Phase 4 viewport filtering will introduce explicit "left viewport" semantics; for now, untouched entities just freeze at their last `current`.

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| Bandwidth grows 10× from 1 Hz to 10 Hz | ~100 entities × ~200 B × 10 Hz = ~200 KB/sec/client. Acceptable for the seeded 100-entity world; Phase 4 viewport-filtering is the right level to address at scale. Documented as a Phase 4 dependency. |
| Backend CPU grows from 10× tick rate | Mobility tick is trivially cheap for 100 entities (HashMap loops). Phase 5 ECS migration handles the future scaling. |
| Animation looks "rubber-bandy" if a delta arrives late | At 10 Hz the inter-tick gap is 100 ms; lerping over that range hides reasonable jitter. If `now - lastTickAt > tickPeriodMs`, `t` clamps to 1 and the entity rests at `current` until the next delta — visually identical to today's per-tick jump but on the 100 ms scale instead of 1 s. |
| Direction snap (sprite frame discrete) looks abrupt at 10 Hz | Less visible than at 1 Hz because direction changes per tick are smaller in spatial terms. Phase 2 keeps direction discrete; smoothing is a future polish slice if needed. |
| Existing tests assert specific tick timings | Plan must update affected websocket / HTTP tests. The plan task list calls them out explicitly. |

## File Structure (anticipated)

- Modify `backend/crates/protocol/src/lib.rs` — add `tick_period_ms`
- Modify `backend/crates/sim-server/src/app.rs` — change tick interval
- Modify `backend/crates/sim-server/src/runtime.rs` — populate tick_period_ms in world_summary
- Modify `backend/crates/sim-server/tests/http.rs` — update timing-dependent tests, assert new field
- Modify `backend/crates/sim-server/tests/websocket.rs` — update tick-cadence assertions
- Modify `src/backend/mobilityProtocol.ts` — typed `WorldSummaryDto` if applicable
- Modify `src/backend/mobilityState.ts` — `InterpolatedEntry`, reducer changes, `interpolatedAgents` / `interpolatedVehicles` helpers
- Modify `src/backend/roadVehicleState.ts` — analogous changes
- Modify `src/backend/mobilityClient.ts` — fetch and surface `tickPeriodMs`
- Modify `src/render/backendMobilityDrawables.ts` — accept `now` + `tickPeriodMs`, use interpolated coords
- Modify `src/main.ts` — read tick period at boot, pass to projector
- Modify `tests/backend/mobilityState.test.ts` and `tests/backend/roadVehicleState.test.ts` — update for buffered shape
- Modify `tests/render/backendMobilityDrawables.test.ts` — interpolation assertions
- Modify `tests/e2e/render-smoke.spec.ts` — assert smooth motion between renders
- Update `progress.md`

## Resolved Questions

- **Tick rate**: 10 Hz (raised from 1 Hz). Matches what the Phase 1 design docs assumed.
- **Interpolation function**: linear `lerp`. Hermite / velocity-aware is deferred.
- **Seeded speeds**: unchanged. At 10× tick rate, current speeds produce visible movement without further edits.
- **Reconnect behavior**: hard snap — buffer reset to `prev = current` for every entity.
- **First-snapshot behavior**: `prev = current` so the entity appears at exact position.
- **Direction**: discrete per tick, read from `current`, no smoothing.

## Out of Scope (deferred to roadmap successors)

- Frame extrapolation past the last snapshot
- Hermite / velocity-hint interpolation
- Procedural population scale (Phase 3)
- Viewport-filtered replication (Phase 4)
- ECS storage (Phase 5)
- Chunk-LOD (Phase 6)
- Persistence partitioning (Phase 7)
- Production hardening (Phase 8)
