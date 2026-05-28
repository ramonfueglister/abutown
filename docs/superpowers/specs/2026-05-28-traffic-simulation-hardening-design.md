# Traffic Simulation Hardening Design

Date: 2026-05-28
Branch: `codex/traffic-simulation-hardening`

## Decision

Abutown's mobility runtime should be road-traffic-first again. The current
implementation reuses `TransitLines` as the generic vehicle route catalog, then
aliases `route:arterial:*` into those lines. That made trams work, but it also
left cars coupled to transit concepts and made the runtime hard to reason
about.

This design removes Tram/Transit Lines from the simulation runtime entirely:

- No backend-seeded tram vehicles.
- No runtime `TransitLines` resource.
- No vehicle movement over `EdgeKind::TramTrack`.
- No frontend `mobilityTrams` success expectation.
- No frontend fallback or fake movement path.

Rail/track tiles may remain as passive map graphics because they are part of
the authored city visual layer. They do not create mobility routes and they do
not drive vehicles in this slice.

## Goals

- Make cars backend-authoritative and visibly moving on road geometry.
- Replace the `TransitLines` route catalog with a road-focused route catalog
  that only exposes `EdgeKind::Road` routes for cars.
- Delete the old tram/transit mobility path instead of leaving fallback code.
- Preserve the Mini Metro-style vector renderer and existing card/login shell.
- Keep all simulation data deterministic so smoke tests and snapshots are
  reproducible.
- Add diagnostics that make stuck cars, unmapped road routes, and route-end
  loops obvious in `render_game_to_text()`.

## Non-Goals

- Do not build full SUMO-level microscopic traffic in this slice.
- Do not add frontend-predicted car movement.
- Do not add emergency fallback data when the backend does not provide traffic.
- Do not remove passive rail visuals from the map because this slice removes
  runtime transit behavior, not authored background geometry.
- Do not reintroduce raster Pak/OpenTTD/Simutrans assets.

## Architecture

### Route Catalog

Create a road-only route catalog named `TrafficRoutes`, built by the routing
graph builder after `Graph` construction.

`TrafficRoutes` owns stable route ids such as `route:arterial:0` and maps each
route to ordered `EdgeId`s whose graph edges are `EdgeKind::Road`. The catalog
must not include tram-track edges. Route lookup replaces
`TransitLines::line_by_legacy()`.

Cars keep a compact route position:

```rust
pub struct RoutePosition {
    pub route_id: TrafficRouteId,
    pub edge_index: usize,
    pub progress: f32,
    pub speed: f32,
}
```

The wire shape continues to expose `route_id: String` and `link_index: usize`,
but those values are produced from `TrafficRoutes`, not from transit lines.

### Vehicle Model

`VehicleKind::Car` remains. `VehicleKind::Tram` is removed from the runtime
types, protocol conversion, frontend DTO validation, rendering diagnostics, and
E2E expectations.

The backend seeder spawns only:

- walking agents from pedestrian corridors
- cars from authored car groups / arterial paths
- one driver agent per car in `InVehicle` state

`seed_trams_from_bundle`, `SeededTransitLine`, and tram-line validation errors
are deleted.

### Movement

Car movement remains fixed-tick and backend-authoritative:

1. Resolve the current route through `TrafficRoutes`.
2. Resolve the current edge through `Graph`.
3. Advance progress by `speed`.
4. At `progress >= 1.0`, advance to the next road edge in the route and reset
   progress to `0.0`.
5. At route end, loop the route deterministically.

Visible cars must not depend on frontend movement logic. Interpolation can
still smooth between backend snapshots, but every sampled target coordinate is
backend-produced.

Car advancement remains chunk-LOD gated for performance, with one explicit
rule: every car included in a client-visible overlay must be in a simulated
chunk and must move within the smoke-test sampling window.

### Diagnostics

Add a compact traffic diagnostics block to `render_game_to_text()`:

```json
{
  "traffic": {
    "routes": 0,
    "cars": 0,
    "movingCars": 0,
    "stuckCars": 0,
    "invalidRouteCars": 0
  }
}
```

The meaning of each field is stable:

- `routes`: road routes currently available from backend traffic metadata.
- `cars`: rendered backend car count.
- `movingCars`: cars whose sampled backend coordinates changed across recent
  samples.
- `stuckCars`: cars observed for long enough with no coordinate movement.
- `invalidRouteCars`: cars rejected or missing because route resolution failed.

No diagnostic may mask missing backend data by creating entities.

## Data Flow

1. Base world bundle loads authored road, pedestrian, car-group, and passive
   rail visual data.
2. Backend graph builder creates road edges from arterial paths and footway
   edges from seeded walks.
3. Backend graph builder creates `TrafficRoutes` from road edges only.
4. Seeder spawns pedestrians and cars from bundle spawn groups.
5. Mobility systems advance walkers and cars on the backend schedule.
6. Backend DTO extraction emits agents and car vehicles.
7. Frontend receives snapshots/deltas, interpolates backend coordinates, and
   renders car glyphs on the Mini Metro-style map.
8. Diagnostics expose backend connection, car counts, movement, and invalid
   traffic state.

## Error Handling

- Missing car arterial ids remain hard `SeedError`s.
- Unknown car route ids return hard errors at spawn time in backend boundaries;
  they must not silently fall back to route zero.
- Protocol validation rejects unsupported vehicle kinds.
- Frontend state reducers count unsupported vehicles as invalid messages and do
  not render alternate entities.
- Smoke tests fail if retired raster asset names are requested or if cars are
  absent/stationary.

## Testing

Backend tests:

- `TrafficRoutes` builds one route per authored arterial with only road edges.
- Car seeding resolves every `route:arterial:*` through `TrafficRoutes`.
- Vehicle advancement loops road routes and never touches tram-track edges.
- Base-world seeding creates cars and drivers, and creates zero trams.
- Removed tram route ids fail loudly if referenced in mobility runtime code.

Frontend tests:

- DTO validation supports `car` and rejects old `tram` runtime vehicles.
- `carsFromMobilityState` renders only backend car DTOs.
- `render_game_to_text()` reports `mobilityVehicles` and `traffic`, and no
  `mobilityTrams` success contract remains.
- State reducer tests keep interpolation but do not synthesize missing cars.

E2E smoke:

- App boots with backend required.
- Old raster asset requests stay zero.
- Cars exist, are visible, and move between backend samples.
- Tram runtime diagnostics are absent.
- Console errors remain empty.

## Rollout

This ships as one focused traffic-runtime branch:

1. Replace route catalog types and backend seeding tests.
2. Migrate vehicle movement and extraction from `TransitLines` to
   `TrafficRoutes`.
3. Delete tram/transit runtime branches.
4. Update frontend DTOs, diagnostics, renderer tests, and E2E smoke.
5. Run cargo, clippy, typecheck, vitest, and Playwright smoke before pushing.

## Open Choices Resolved

- Trams are not merely hidden; they are removed from mobility runtime.
- Road traffic does not borrow transit concepts.
- Passive rail visuals stay because they are not fallback simulation code.
- No fallback entities or local fake traffic are allowed.
