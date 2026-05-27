# Base World Cutover

**Date:** 2026-05-27
**Status:** Design

## Goal

Replace the demo-era split between frontend-authored map visuals, backend demo chunk seeds, and a separate mobility network with one canonical, versioned Base World Bundle. The backend loads and validates that bundle as the single source of truth; the frontend renders data derived from it; persistence stores only runtime deltas and snapshots on top.

This phase is primarily a cleanup and authority cutover. It removes old demo seeds and fallbacks before adding new feature mechanics.

## Why Now

The current app can render a convincing 256 x 256 Zurich-style city while the backend terrain authority only seeds three chunks and one changed tile per chunk. That is not a production architecture. It creates three conflicting truths:

1. `src/city/zurichWorld.ts` defines the visual world.
2. `data/city/zurich-network.json` defines mobility corridors and arterials.
3. `backend/crates/sim-server/src/runtime.rs` seeds three backend chunk mutations for runtime and persistence tests.

The result is fragile. Vehicles can move on routes that are not materialized as backend road tiles. The renderer can show buildings and roads that the server does not own. Persisted stale mobility snapshots can make a fresh session look broken while the frontend still paints a city.

State-of-the-art browser games and web simulations avoid this split. They use a server-authoritative world model, chunked/streamed world layers, and live deltas scoped by client interest. The client renders; it does not invent the map.

## External Reference Patterns

- Server-authoritative realtime games: frameworks such as Nakama and Colyseus keep authoritative game state on the server and synchronize schema/state changes to clients.
- Vector-tile map systems: Mapbox Vector Tiles encode structured feature layers per tile using Protobuf coordinates local to the tile grid. PostGIS can produce vector tiles through `ST_AsMVT`, `ST_AsMVTGeom`, and `ST_TileEnvelope`.
- Modern browser rendering: WebGPU is the modern browser GPU API, but the data model decision is independent of WebGPU. Canvas can remain temporarily if it renders canonical layers.
- ECS simulation: Bevy 0.18 supports ECS components and relationships, matching the existing `Chunk -> Tile -> domain component` direction from Phase 8a.

## Design Choice

Use a canonical Base World Bundle loaded by the backend, then derive frontend render data and mobility/routing resources from that bundle.

### Option A: Backend Base World Bundle First

Create a versioned local bundle and make the backend load it fail-closed. Frontend rendering is migrated to consume generated bundle-derived layers. This is selected because it removes the core lie first: no parallel frontend map and backend demo map.

### Option B: PostGIS-First Spatial Authoring

Move all base-world geometry into PostGIS immediately and generate tiles from the database. This is directionally right for editors and large worlds, but too much operational scope for the first cutover. It also risks binding hot game startup to database availability before the local bundle format is stable.

### Option C: Keep Frontend World, Add More Backend Seeds

Mirror more roads/buildings from the frontend by hardcoding backend seed logic. This is rejected. It extends the demo architecture and creates more places where data can diverge.

## Target Architecture

```text
data/worlds/zurich-river-city-v1/
  manifest.json
  chunks/*.json or chunks/*.bin
  layers/terrain.*
  layers/transport.*
  layers/buildings.*
  layers/spawns.*

backend BaseWorldLoader
  -> validate manifest and layer versions
  -> materialize chunk entities and dense Tiles
  -> materialize routing graph inputs
  -> materialize mobility spawn inputs
  -> publish read-view render layers

frontend
  -> fetch/read backend-provided world layers
  -> render canonical layers
  -> subscribe to live mobility chunks
```

The bundle is immutable content. Runtime changes are append-only world events, dirty chunk snapshots, mobility snapshots, and later plugin snapshots. Base content is not rewritten by the simulation tick.

## Base World Bundle

The bundle is a versioned directory. The first implementation may use JSON for readability; binary chunk files can follow after the schema is proven. The schema must be explicit enough that the backend can load without guessing.

### Manifest

Required fields:

```json
{
  "schema_version": 1,
  "world_id": "zurich-river-city-v1",
  "chunk_size": 32,
  "world_tiles": { "width": 256, "height": 256 },
  "coordinate_space": "tile",
  "layers": {
    "terrain": "layers/terrain.json",
    "transport": "layers/transport.json",
    "buildings": "layers/buildings.json",
    "spawns": "layers/spawns.json"
  }
}
```

Rules:

- `world_id` is stable and must match persisted base-world metadata.
- `chunk_size` must match backend chunk materialization.
- `coordinate_space` is `tile` for this phase. No latitude/longitude or Web Mercator conversion happens in the hot runtime.
- Every listed layer path must exist.
- Unknown required fields are rejected by schema version, not silently ignored.

### Terrain Layer

Terrain is the dense base tile payload for every chunk. It owns:

- grass
- water
- riverbank
- park
- forest
- reserve
- plaza

The backend maps this to existing `TileKind` where possible and adds metadata resources for visual terrain kinds where `TileKind` is too coarse. This phase must not degrade the visual map to only four tile kinds.

### Transport Layer

Transport owns road, rail, station, bridge, arterial, pedestrian corridor, and route geometry. It replaces the current split between frontend road topology and `zurich-network.json`.

Routing, mobility seeding, and rendering consume this layer. There is no separate mobility-only network file after the cutover.

### Building Layer

Buildings are structured features, not random frontend decorations. A building record has at least:

- id
- footprint or tile anchor
- use class: residential, commercial, civic, industrial
- visual class
- optional capacity metadata for later domain tiles

This phase does not implement economy or residents, but it must preserve building identity so Phase 8g can attach Home, Workplace, and Storage components without re-authoring the map.

### Spawn Layer

Spawn points are explicit and deterministic:

- pedestrian spawn groups
- vehicle spawn groups
- tram/train seed positions
- optional debug camera start

The backend rejects a world bundle that has no valid mobility spawn layer when mobility is enabled.

## Trash Inventory

The implementation plan must remove or replace these demo-era paths.

### Backend Demo Seeds

- `SEEDED_CHUNKS` in `backend/crates/sim-server/src/runtime.rs`
- the offset-based `TileKind::Road`, `TileKind::Water`, `TileKind::BuildingFootprint` seed mutations
- tiny fallback worlds that are used in production startup
- `CityNetwork::empty_for_world` as a production fallback
- any startup branch that turns a missing world file into an empty world

Test-only tiny fixtures may survive only inside tests with names that make test scope explicit.

### Frontend Parallel World Authority

- `src/city/zurichWorld.ts` must stop being runtime authority for the real map.
- `src/city/generateCity.ts`, `zurichPlacement`, transport placement, and related files may remain temporarily only as offline bundle generation tooling.
- Runtime rendering must not call frontend procedural map builders after the cutover.

### Legacy Asset Surface

- Pak128/Simutrans assets are not part of the selected minimal-vector style. Runtime code must not request them.
- Retired asset guard stays and is expanded to cover new bundle runtime paths.
- Public asset files can be deleted in a separate cleanup task only after a guard proves no runtime/test code references them.

### Fallbacks

The following behaviors are forbidden in production startup:

- missing bundle -> empty world
- missing terrain -> grass-only map
- missing transport -> no-road map
- missing spawn layer -> no agents
- stale persisted mobility snapshot with incompatible base world -> accepted silently
- unknown schema version -> best-effort parse

Every one of these must fail with a clear error, a logged cause, and a backend-required style UI state.

## Runtime Data Flow

Startup:

```text
SimulationRuntime::new()
  -> BaseWorldLoader::load(path)
  -> BaseWorldBundle::validate()
  -> materialize chunks into Bevy World
  -> install routing resources from transport layer
  -> seed mobility from spawn + transport layers
  -> hydrate compatible persisted deltas/snapshots
  -> publish RuntimeReadView
```

Hydration:

```text
read base_world_id from bundle
read persisted snapshot metadata
if snapshot.base_world_id != bundle.world_id:
  reject stale snapshot for this runtime
else:
  apply snapshot and tail events
```

Rendering:

```text
frontend startup
  -> fetch backend world summary and initial render layers
  -> render canonical terrain/transport/building layers
  -> subscribe to visible mobility chunks
  -> apply mobility snapshots and deltas
```

The frontend can cache static layers, but cache keys must include `world_id` and `schema_version`.

## Persistence Strategy

Persisted runtime state is layered over immutable base content.

Required metadata on persisted snapshots:

- `world_id`
- `base_world_id`
- `base_schema_version`
- `chunk_size`
- snapshot schema version

Existing Postgres schemas may be migrated or extended, but the loader must be explicit. A snapshot without base metadata is legacy data. Production startup must not silently hydrate it for a canonical base world. Local tests can load legacy fixtures only through explicit legacy-test helpers.

## Frontend Strategy

The frontend remains a renderer and interaction shell.

Allowed runtime responsibilities:

- draw terrain, transport, buildings, trees/details from backend/bundle-derived layers
- draw live mobility entities from backend state
- interpolate backend positions between ticks
- provide camera, selection, and diagnostics

Forbidden runtime responsibilities:

- procedurally defining the canonical map
- inventing roads/buildings not present in the bundle
- routing agents locally
- hiding missing backend data with placeholder city data

## Error Handling

World loading errors are fatal for gameplay startup. The UI should show a backend-required/fail-closed state with the exact failed subsystem:

- manifest missing
- layer missing
- schema mismatch
- chunk materialization failed
- transport graph invalid
- spawn layer invalid
- persisted snapshot incompatible with base world

The app may still render a plain error panel. It must not render a playable fake city.

## Testing

Required coverage:

- `BaseWorldLoader` rejects missing manifest.
- `BaseWorldLoader` rejects missing listed layer.
- `BaseWorldLoader` rejects unknown schema version.
- Runtime materializes 256 x 256 / 32 chunk topology from `zurich-river-city-v1`.
- Runtime has more than the old three seeded chunks.
- Runtime transport layer produces roads, rails, stations, and routing inputs from one source.
- Runtime mobility seed creates pedestrians, cars, and trams from bundle data.
- Frontend runtime does not call `buildZurichWorld` during app startup.
- Browser smoke proves visible map, visible vehicles, moving agents, no retired asset requests, and no console errors.
- Grep acceptance blocks production fallback language and demo seed symbols.

## Migration Plan Shape

The implementation plan must proceed in this order:

1. Add tests/guards that expose the current demo split.
2. Introduce the bundle schema and checked loader.
3. Generate or author `zurich-river-city-v1` from the current frontend world and network data.
4. Materialize backend chunks from the bundle.
5. Materialize routing and mobility seed inputs from the same transport/spawn layers.
6. Move frontend runtime rendering to bundle-derived layers.
7. Delete production fallback seeds and demo runtime paths.
8. Add stale snapshot compatibility checks.
9. Run full local and browser verification.

No step may preserve a production fallback "temporarily" unless that fallback is test-only and impossible to call from normal startup.

## Out of Scope

- Economy, ledger, production chains, or money.
- Full map editor.
- PostGIS authoring backend.
- WebGPU renderer rewrite.
- Multiplayer matchmaking.
- New visual art direction.
- Replacing Bevy ECS.

These are later phases. This phase is the base-world authority cutover.

## Acceptance Criteria

The phase is complete when:

1. Production startup has no hardcoded `SEEDED_CHUNKS` or offset-based demo tile mutations.
2. Backend startup fails if the canonical Base World Bundle is missing or invalid.
3. Backend chunk entities are materialized from `zurich-river-city-v1`.
4. Frontend runtime does not procedurally define the playable map.
5. Roads, rails, buildings, terrain, routing, and mobility spawn inputs come from one base-world source.
6. Persisted snapshots are checked against `base_world_id` before hydration.
7. Retired Pak128/Simutrans runtime requests remain blocked.
8. Browser smoke passes against the cutover world.
9. CI runs unit, Rust, build, and browser-smoke gates.
10. Progress notes document the new base-world authority and explicitly state that no production fallbacks remain.

## Source References

- Mapbox Vector Tile Specification 2.1: https://mapbox.github.io/vector-tile-spec/
- PostGIS vector tile functions: https://postgis.net/docs/reference.html
- PostGIS `ST_TileEnvelope`: https://postgis.net/docs/manual-3.4/de/ST_TileEnvelope.html
- Supabase PostGIS extension: https://supabase.com/docs/guides/database/extensions/postgis
- Bevy 0.18 release: https://bevy.org/news/bevy-0-18/
- Bevy Component relationships: https://docs.rs/bevy/latest/bevy/ecs/component/derive.Component.html
- Nakama authoritative multiplayer: https://heroiclabs.com/docs/nakama/concepts/multiplayer/authoritative/
- Colyseus state synchronization: https://docs.colyseus.io/state
- MDN WebGPU API: https://developer.mozilla.org/docs/Web/API/WebGPU_API
