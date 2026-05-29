# Visible Backend Mobility Design

**Date:** 2026-05-16
**Status:** Approved for planning — Phase 1 of the Million-Agent Roadmap.
**Roadmap:** `docs/superpowers/specs/2026-05-16-million-agent-roadmap-design.md`
**Predecessors:** `2026-05-15-mobility-client-bridge-design.md` (bridge built diagnostic-only), `2026-05-16-mobility-population-design.md` (20 agents seeded), `2026-05-15-local-road-vehicles` plan (frontend separation of people/vehicles).
**Successor (out of scope):** Phase 2 frame interpolation, Phase 3 procedural population scale-up, Phases 4+ viewport/ECS/LOD.

---

## Goal

After this phase, the browser canvas shows **all visible road traffic and pedestrians as projections of server-authoritative mobility state**, with no parallel frontend simulation. The Phase-0 backend already simulates 20 agents + 4 transit vehicles deterministically; this phase adds the missing road-vehicle subsystem, extends DTOs with world coordinates and sprite hints, replaces the frontend's `buildPedestrians()` and `buildCars()` with backend-driven drawables, and wires selection/inspector to backend entity IDs.

## Non-Goals

- Frame interpolation between server ticks (Phase 2).
- Procedural population scaling beyond ~300 entities total (Phase 3).
- Viewport-filtered replication (Phase 4).
- ECS migration of mobility storage (Phase 5).
- Chunk-LOD simulation (Phase 6).
- Per-chunk mobility persistence (Phase 7).
- Pathfinding / dynamic routing — paths stay deterministic from a hardcoded geometry module.
- Player commands for mobility — backend simulates, players observe.
- Train migration — trains stay frontend-only; they are not part of `MobilityWorld`.
- WebGPU/WebGL rendering — canvas 2D with existing sprite pipeline only.

## Architecture

Three coordinated changes, all sharing one PR:

1. **Backend gains a road-vehicle subsystem** distinct from the existing transit `VehicleRecord`. The new entity has path + offset + speed + sprite hint and nothing else — no plan, no boarding, no route/stop coupling. This mirrors the explicit "cars are not agents" boundary already established on the frontend by the `local-road-vehicles` plan.

2. **Mobility DTOs carry world coordinates and direction hints**, computed server-side each tick from the relevant geometric primitives (`link_id`+`progress` for walking agents; `path`+`offset` for road vehicles; `route_id`+`link_index`+`progress` for transit vehicles). The frontend no longer needs to know how to project abstract IDs into tile space.

3. **The frontend canvas treats backend mobility as the only visual source.** `buildPedestrians()` and `buildCars()` and their seeded `pedestrians`/`cars` arrays are removed. A new drawable projector reads `mobilityState.agents` (existing) and `mobilityState.roadVehicles` (new) and emits the `Pedestrian` and `Car` records the existing `drawPedestrian`/`drawCar` functions consume. Sprite catalogs (`pedestrianSprites`, `vehicleSprites`) remain, but each backend entity carries a `sprite_key` string the frontend looks up. Selection IDs are now backend-assigned.

The backend remains the **only mutation authority**. Transit `VehicleRecord` keeps its existing semantics (routes/stops/boarding). The new `RoadVehicleWorld` is purely animation-server: deterministic offset advance per tick, wrapping at path end.

## Data Flow

```text
Backend tick:
  - MobilityWorld.tick_mobility()      (existing: agents walk/board/ride/alight, transit vehicles advance)
  - RoadVehicleWorld.tick_road_vehicles()  (new: road vehicle offsets advance along paths)
  - For each entity, compute world_coord + direction from its geometric primitive

WebSocket /ws (existing channel):
  - mobility_delta carries agents/transit-vehicles (existing shape, extended with world_coord + direction)
  - road_vehicle_delta (new message) carries road vehicles (id, world_coord, direction, sprite_key)
  - Initial snapshot still served from GET /mobility + GET /road-vehicles

Frontend per-frame render:
  - backendMobilityDrawables module reads mobilityState.agents + mobilityState.roadVehicles
  - Projects each entity to a transient Pedestrian-shaped or Car-shaped record
  - Existing drawPedestrian()/drawCar() draws using world_coord and direction-selected sprite frame
  - No local mobility state, no buildPedestrians(), no buildCars()
```

## Components

### `sim-core` additions

- `sim-core/src/road_vehicles.rs` — new module.
  - `RoadVehicleId(pub String)` newtype in `sim-core/src/ids.rs`.
  - `RoadVehicleRecord { id, path: Vec<TileCoord>, offset: f32, speed: f32, sprite_key: String }`. `TileCoord { x: i32, y: i32 }` already exists in `sim-core/src/ids.rs`? — verify; if not, add as `pub struct TileCoord { pub x: i32, pub y: i32 }` (distinct from `ChunkCoord` which addresses chunks, not tiles).
  - `RoadVehicleWorld { tick: u64, vehicles: HashMap<RoadVehicleId, RoadVehicleRecord> }` — HashMap storage matches mobility today; ECS migration is Phase 5.
  - `tick_road_vehicles(&mut self)` advances every offset by speed; wraps `offset` modulo `path.len() as f32`.
  - `world_coord(&self, id) -> Option<(f32, f32)>` interpolates between `path[floor(offset)]` and `path[(floor(offset)+1) % path.len()]` by fractional part.
  - `direction(&self, id) -> Option<DirectionDto>` computes from `next_tile - current_tile` delta.
  - `pub mod seed { pub fn initial_road_vehicles() -> RoadVehicleWorld { ... } }` — analogous to mobility seeder, hardcoded ~80 vehicles on 4 path corridors around the seeded chunks.
  - Serde-derived for persistence.

- `sim-core/src/mobility_geometry.rs` — new module.
  - `pub struct LinkGeometry { pub start: (f32, f32), pub end: (f32, f32) }`.
  - `pub struct StopGeometry { pub coord: (f32, f32) }`.
  - `pub struct RouteGeometry { pub link_path: Vec<(f32, f32)> }` — denormalized for fast `route_id+link_index+progress` → coord lookup.
  - `pub fn link_geometry(link_id: &str) -> Option<LinkGeometry>` etc. — hardcoded data for the existing seeded routes (`route:horizontal`, `route:vertical`, links between (4,4)↔(5,4) and (4,4)↔(4,5) chunks). Tile coordinates in playable-map space.
  - Pure data + lookups, no I/O.

- `sim-core/src/mobility.rs` — additions, not replacements.
  - `MobilityWorld::world_coord_for_agent(agent_id) -> Option<(f32, f32)>` walks the agent's current `AgentMobilityState`:
    - `Walking { link_id, progress }` → interpolate `link_geometry(link_id)` by `progress`.
    - `WaitingAtStop { stop_id }` → `stop_geometry(stop_id)`.
    - `Boarding/InVehicle/Alighting` → delegate to the vehicle's coord.
    - `AtActivity` → fixed activity coord (added to geometry module per activity_id).
  - `MobilityWorld::direction_for_agent(agent_id)` similar pattern.
  - `MobilityWorld::world_coord_for_vehicle(vehicle_id)` reads `route_id`+`link_index`+`progress` and interpolates via `RouteGeometry`.

### Protocol additions (`abutown-protocol`)

- New enum `pub enum DirectionDto { N, NE, E, SE, S, SW, W, NW }`. Mapping to the existing `vehicleFrameForGridDelta` directions in frontend stays consistent.
- New `pub struct WorldCoordDto { pub x: f32, pub y: f32 }`.
- `AgentMobilityDto` gains `pub world_coord: WorldCoordDto`, `pub direction: DirectionDto`, `pub sprite_key: String`.
- `VehicleMobilityDto` gains the same three fields.
- New `pub struct RoadVehicleDto { pub id: String, pub world_coord: WorldCoordDto, pub direction: DirectionDto, pub sprite_key: String }`.
- New `pub struct RoadVehicleSnapshotDto { protocol_version, world_id, tick, vehicles: Vec<RoadVehicleDto> }`.
- New `pub struct RoadVehicleDeltaDto { protocol_version, world_id, tick, changed: Vec<RoadVehicleDto> }`.
- `ServerMessageDto::RoadVehicleDelta(RoadVehicleDeltaDto)` variant added.

### Backend HTTP/WS surface (`sim-server`)

- New `GET /road-vehicles` returning `RoadVehicleSnapshotDto`.
- `GET /mobility` unchanged shape but agent/vehicle DTOs now include the three new fields.
- `/ws` adds `RoadVehicleDelta` to the per-tick broadcast (same `tokio::broadcast` channel as existing mobility_delta).
- `SimulationRuntime` gains a `road_vehicle_world: RoadVehicleWorld` field.
- `SimulationRuntime::tick(...)` calls both `mobility.tick_mobility()` and `road_vehicle_world.tick_road_vehicles()`.
- `SimulationRuntime::new_with_all_stores(...)` extended with a new `road_vehicle_snapshot_store: Box<dyn RoadVehicleSnapshotStore + Send>` argument. Default in-memory variant for tests.
- `hydrate_from_stores(...)` extended analogously; falls back to `road_vehicles::seed::initial_road_vehicles()` if no persisted snapshot.

### Persistence (`sim-server`)

- New trait `RoadVehicleSnapshotStore` in `sim-core/src/persistence.rs`, parallel to `MobilitySnapshotStore`:
  ```rust
  async fn write(&mut self, world_id: &str, tick: u64, snapshot: &RoadVehicleWorld) -> Result<(), Err>;
  async fn read(&self, world_id: &str) -> Result<Option<(u64, RoadVehicleWorld)>, Err>;
  ```
- `InMemoryRoadVehicleSnapshotStore` (sim-core).
- New migration `backend/crates/sim-server/migrations/202605160003_road_vehicle_snapshots.sql`:
  ```sql
  CREATE TABLE IF NOT EXISTS road_vehicle_snapshots (
      world_id TEXT PRIMARY KEY,
      tick BIGINT NOT NULL CHECK (tick >= 0),
      payload JSONB NOT NULL,
      updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
  );
  ```
- `PostgresRoadVehicleSnapshotStore` in `sim-server/src/postgres_road_vehicles.rs` — same UPSERT-style adapter as `PostgresMobilitySnapshotStore`.
- Snapshot loop (`persist_snapshots_once` in `app.rs`) gains a third persist call after `persist_mobility_snapshot`. Failure is logged-and-continued, matching the chunk and mobility patterns.

### Frontend

- New `src/render/backendMobilityDrawables.ts` — pure functions:
  - `pedestriansFromMobilityState(state, spriteCatalog): Pedestrian[]`. Maps each `AgentMobilityDto` to a frame-local `Pedestrian` record with `path: [world_coord, next_predicted_coord]` of length 2 (sufficient for `drawPedestrian` which only reads two consecutive entries), `offset: 0`, `speed`, `laneOffset`, `sprite` looked up by `sprite_key`. Phase 2 (interpolation) will improve the `path` to enable lerp.
  - `carsFromMobilityState(state, spriteCatalog): Car[]`. Same pattern using `mobilityState.roadVehicles`.
- New `src/backend/roadVehicleProtocol.ts` and `roadVehicleState.ts` — analogous to `mobilityProtocol.ts`/`mobilityState.ts`, but for the new `RoadVehicleSnapshotDto` and `RoadVehicleDeltaDto`.
- `src/backend/mobilityClient.ts` — extended to subscribe to `RoadVehicleDelta` messages alongside the existing `MobilityDelta` and to fetch `GET /road-vehicles` at boot.
- `src/main.ts`:
  - Remove `buildCars()`, `buildPedestrians()`, `cars: Car[]`, `pedestrians: Pedestrian[]` declarations.
  - Frame builder uses `pedestriansFromMobilityState(...)` and `carsFromMobilityState(...)` each frame.
  - Selection state stays scoped by stable backend entity ID (`agent:seed:N`, `road_vehicle:seed:M`).
  - `selectedAgent()` / `selectedVehicle()` look up by ID in `mobilityState`.
  - `drawAgentInspectorPanel` / `drawRoadVehicleInspectorPanel` receive backend-shaped data; existing inspector signatures stay compatible because they read `id`, `coord`, `state`, etc. — fields that are present in the new projected drawable.

### Sprite-key catalog

The backend chooses sprite keys deterministically per entity. The frontend has the catalogs already (`vehicleSprites`, `pedestrianSprites`); the question is which sprite the backend names.

- For pedestrians: `sprite_key = format!("pedestrian:{}", index % SPRITE_COUNT)` where `index = hash(agent_id) % SPRITE_COUNT`. Frontend resolves to a member of `pedestrianSprites`.
- For road vehicles: same pattern with `vehicleSprites`.
- For transit vehicles: a `sprite_key` like `"tram"` resolves on the frontend to a Tram-style sprite from the existing pak128 catalog.

The hash-based assignment is deterministic so restarts produce the same visual mapping. The number of available sprite keys is a frontend-controlled constant; the backend uses an indexed scheme that survives even if the frontend catalog grows.

## Error Handling

- Backend startup: if `road_vehicle_snapshots` migration fails, server refuses to boot (same fail-fast policy as `mobility_snapshots`).
- Hydration: missing/corrupt snapshot → fall back to `seed::initial_road_vehicles()` and log a warning. (Matches mobility recovery; explicitly weaker than chunk recovery's fail-fast because road vehicles have no event log to replay.)
- Snapshot loop write failures: logged via `tracing::warn!`, loop continues.
- Frontend: unknown `sprite_key` → use a fallback sprite from the existing catalog (the first valid entry), log a console warning. Don't throw.
- DTO version mismatch: existing protocol version check rejects on `/health` already; no new path.

## Testing Strategy

**sim-core unit tests**
- `RoadVehicleWorld::tick_road_vehicles` advances every offset by exactly its speed.
- `tick_road_vehicles` wraps `offset` modulo path length.
- `world_coord` interpolates correctly at fractional offsets (e.g. offset 0.5 between (0,0) and (4,0) → (2.0, 0.0)).
- `direction` computes correct 8-way direction from path delta.
- `mobility_geometry::link_geometry("link:horizontal:main")` returns the expected hardcoded coord.
- `MobilityWorld::world_coord_for_agent` resolves correctly for each `AgentMobilityState` variant (mock geometry).
- `seed::initial_road_vehicles()` produces ≥80 vehicles, all with non-empty paths and capacity-equivalent fields.

**sim-server unit tests**
- `SimulationRuntime` ticks both subsystems on `tick()`.
- `persist_road_vehicle_snapshot` writes through `RoadVehicleSnapshotStore`.
- `hydrate_from_stores` with empty road-vehicle store falls back to seeder.
- `hydrate_from_stores` with persisted road-vehicle store restores the persisted world.
- Existing tests covering `MobilitySnapshotStore` and chunk recovery stay green (regression).

**sim-server integration tests (opt-in `ABUTOWN_TEST_DATABASE_URL`)**
- `GET /road-vehicles` returns the seeded snapshot.
- Round-trip persistence: mutate (tick road vehicles), persist, drop runtime, reconnect, verify state.
- WebSocket emits `road_vehicle_delta` after a tick.

**Frontend unit tests (Vitest)**
- `pedestriansFromMobilityState` produces correct path/offset/sprite for sample mobility state.
- `carsFromMobilityState` likewise for road vehicles.
- `mobilityState` reducer applies `road_vehicle_delta` messages.
- Selection: clicking near a backend entity selects by ID.

**Frontend E2E (Playwright)**
- `render-smoke.spec.ts` updated to assert canvas shows backend-sourced mobility (server tick visible, agent count from backend, no local `pedestrians`/`cars` arrays in `render_game_to_text` output).

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| Removing `buildPedestrians`/`buildCars` mid-PR breaks visual smoke tests | Implement backend sprite projection FIRST under feature flag, validate visually, then remove local builders in the final commits. |
| Backend-computed `direction` jitter at 10 Hz between path-segment boundaries | Server smooths direction by 1-tick lookahead (compare `world_coord(now)` to `world_coord(predicted_next_tick)`) — simpler than per-frame frontend smoothing in this phase; Phase 2 interpolation will refine. |
| Sprite catalog mismatch (backend names a key the frontend doesn't have) | Fallback sprite + console warning. Tracked metric to surface drift early. |
| 200 pedestrians + 80 vehicles broadcast per tick saturates dev WebSocket | At 10 Hz, ~30 entities × small JSON each = ~30 KB/sec uncompressed. Well under any sane budget. Documented; not a real risk this phase. |
| Persistence churn (200+80 entities written every snapshot loop) | Same single-row UPSERT pattern as mobility — already 30 agents writing now without issue. No throttling needed this phase. |
| `RoadVehicleWorld` HashMap iteration cost at scale | Phase 5 (ECS migration) explicit successor; this phase keeps HashMap deliberately. |

## File Structure (anticipated)

- Create `backend/crates/sim-core/src/road_vehicles.rs`
- Create `backend/crates/sim-core/src/mobility_geometry.rs`
- Modify `backend/crates/sim-core/src/lib.rs` — export new modules
- Modify `backend/crates/sim-core/src/ids.rs` — add `RoadVehicleId`, `TileCoord` if missing
- Modify `backend/crates/sim-core/src/mobility.rs` — add `world_coord_for_*` / `direction_for_*` helpers
- Modify `backend/crates/sim-core/src/persistence.rs` — add `RoadVehicleSnapshotStore` trait + InMemory impl
- Modify `backend/crates/protocol/src/lib.rs` — add `DirectionDto`, `WorldCoordDto`, extend Agent/Vehicle DTOs, add RoadVehicle DTOs + delta
- Create `backend/crates/sim-server/migrations/202605160003_road_vehicle_snapshots.sql`
- Create `backend/crates/sim-server/src/postgres_road_vehicles.rs`
- Modify `backend/crates/sim-server/src/lib.rs` — export `postgres_road_vehicles`
- Modify `backend/crates/sim-server/src/runtime.rs` — add field, extend constructors, extend `hydrate_from_stores`, add `persist_road_vehicle_snapshot`
- Modify `backend/crates/sim-server/src/app.rs` — `GET /road-vehicles`, ws broadcast, snapshot loop call
- Modify `backend/crates/sim-server/tests/http.rs` — integration test for road-vehicle recovery
- Create `src/backend/roadVehicleProtocol.ts`
- Create `src/backend/roadVehicleState.ts`
- Modify `src/backend/mobilityClient.ts` — fetch road-vehicles snapshot, handle delta messages
- Modify `src/backend/mobilityState.ts` — embed road-vehicle state alongside existing mobility state
- Create `src/render/backendMobilityDrawables.ts` — projection of backend state into draw records
- Modify `src/main.ts` — remove local builders, wire backend projector
- Update `tests/e2e/render-smoke.spec.ts`
- Update `tests/render/*.test.ts` for new projector + selection
- Update `progress.md`

## Resolved Questions

- **Should `RoadVehicleWorld` use HashMap or bevy_ecs storage?** HashMap. ECS migration is Phase 5 of the roadmap; doing it here would couple Phase 1 to Phase 5 and slow the visible-mobility win.
- **Should we add `road_vehicles` events?** No. Road vehicles have no player-initiated mutation — they're pure animation. Persistence is snapshot-only, like mobility today.
- **Should transit vehicles get the same `sprite_key` field?** Yes — uniformity across all three subsystems (agent/transit/road). Frontend tram sprites resolve via the same catalog lookup.
- **Should `mobility_geometry` be auto-generated from a city descriptor?** No — Phase 3 will do that. Phase 1 hardcodes the geometry for the seeded routes only.
- **What about `localPedestrianAgents`/`localRoadVehicles` selection APIs?** They are removed entirely. Selection points to backend entities; the `LocalPedestrianAgent` / `LocalRoadVehicle` types in `src/render/pedestrianAgents.ts` / `localRoadVehicles.ts` become unused. We delete those modules along with `buildPedestrians`/`buildCars` and update tests accordingly.
- **Train rendering**: trains are NOT in `MobilityWorld` or `RoadVehicleWorld`. They keep their existing frontend-only `buildTrains()`/`drawTrain()` flow. A future roadmap can migrate them, but it is out of scope here.

## Out of Scope (this phase, deferred to roadmap successors)

- Frame interpolation (Phase 2)
- Procedural population beyond ~300 entities (Phase 3)
- Viewport-filtered replication (Phase 4)
- ECS storage (Phase 5)
- Chunk-LOD per-tick budgeting (Phase 6)
- Per-chunk persistence (Phase 7)
- Production hardening / load tests / metrics (Phase 8)
- Player-initiated road-vehicle commands
- Pathfinding / dynamic routing
- Train migration to backend
- Pak128 sprite catalog expansion (use whatever the existing catalogs return)
