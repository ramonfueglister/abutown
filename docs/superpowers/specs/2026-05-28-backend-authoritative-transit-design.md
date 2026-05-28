# Backend-Authoritative Transit Vehicles

**Date:** 2026-05-28
**Status:** Design

## Goal

Move visible trains/trams from frontend-authored animation into the backend mobility authority. The frontend may still draw static rail geometry from the Base World Bundle, but moving transit vehicles must come from backend `VehicleMobility` state just like cars.

## Current Problem

The Base World Bundle already contains `transport.rail_paths` and `spawns.tram_lines`, and the protobuf protocol already supports `VehicleKind::TRAM`. The runtime still seeds only pedestrian agents and car vehicles from the bundle, while `src/main.ts` builds a separate frontend train using `buildNorthboundTrainPath`, local offset state, and `advanceTime`.

That creates a second simulation path:

- backend cars move through `MobilityWorld`
- frontend train moves through local render time
- diagnostics report `city.trains === 1` even if backend mobility has no tram vehicle
- persisted mobility snapshots cannot own the visible train

This contradicts the base-world cutover rule: runtime rendering must not invent dynamic world state outside the backend.

## Design Choice

Use existing backend mobility vehicles for trams. Do not add a parallel train protocol, local rail animation, or frontend-only fallback.

### Selected Approach: Extend Bundle Seeding

`BaseWorldBundle` becomes the source for transit spawn data. `sim-core::mobility::seed` gets a bundle-aware seeding entrypoint that:

- installs routing resources from the bundle network
- spawns pedestrians from `spawns.pedestrian_groups`
- spawns cars from `spawns.car_groups`
- spawns tram vehicles from `spawns.tram_lines`
- validates referenced corridor, arterial, and rail path ids
- fails closed when spawn data references missing transport paths

The frontend consumes the resulting backend `VehicleMobilityDto` entries. `kind === "car"` renders with road vehicle sprites; `kind === "tram"` renders with the Mini-Metro train glyph on rail. Static rail lines remain bundle-derived render layers.

### Rejected: Frontend Train Offset With Backend Metadata

Keeping `trains[]` and simply reading speed/path metadata from the backend is not enough. It still leaves movement, persistence, chunk visibility, and diagnostics outside backend authority.

### Rejected: New `TrainMobility` Protocol

The protocol already has `VehicleKind::TRAM`, `route_id`, `link_index`, `progress`, `world_coord`, `direction`, and chunk deltas. A second dynamic transit protocol would duplicate semantics and create another migration target.

## Backend Requirements

- Production startup must seed trams from `BaseWorldBundle.spawns.tram_lines`.
- Each tram spawn must reference at least one existing `transport.rail_paths[].id`.
- Missing rail paths, empty `rail_path_ids`, and zero total transit spawn capacity are load errors when transit is enabled by the bundle.
- Tram vehicle ids must be deterministic and stable, for example `vehicle:tram:<line_index>:<n>`.
- Tram `route_id` must resolve to the routing `TransitLines` aliases installed for the corresponding rail path.
- Tram world coordinates and directions must be computed by the existing mobility DTO builders; no placeholder coordinates.
- Mobility snapshots must persist and hydrate tram vehicles with `kind = Tram`.
- Existing car and pedestrian counts from the bundle remain deterministic.

## Frontend Requirements

- Runtime code must not build or advance frontend-only trains.
- `renderMinimalMap` must draw backend tram vehicles from `MobilityOverlayState`.
- Diagnostics must count/report backend tram vehicles, not local train objects.
- `window.advanceTime` may advance interpolation/test time, but it must not mutate a local train offset.
- If the backend returns zero trams for a bundle that declares trams, the app should show zero moving trams rather than inventing one.
- Static rail tiles, rail crossings, and rail paths remain rendered from the canonical Base World Bundle.

## Data Flow

Startup:

```text
BaseWorldBundle::load_from_dir()
  -> validate transport + spawn references
  -> mobility::seed::from_base_world_bundle()
  -> spawn AgentRecord + VehicleRecord(kind=Car/Tram)
  -> apply_into_world()
  -> publish MobilitySnapshot / MobilityChunkSnapshot
```

Render:

```text
MobilityOverlayState
  -> interpolatedVehicles()
  -> vehicle.kind === "car"  -> road vehicle renderer
  -> vehicle.kind === "tram" -> train/tram renderer
```

## Testing

Required regression coverage:

- Rust unit test: base-world seeding produces exactly the bundle-declared tram count.
- Rust unit test: missing rail path referenced by a tram spawn fails closed.
- Rust runtime test: `SimulationRuntime::new_from_base_world_dir` exposes tram vehicles in its mobility snapshot.
- Frontend unit test: backend tram vehicles project into train/tram drawables; car projection still excludes trams.
- Frontend guard: runtime no longer calls `buildNorthboundTrainPath`, `trainWrappedOffset`, or local train offset advancement.
- E2E smoke: visible train/tram movement is derived from backend mobility vehicles, not `city.train`.

## Acceptance Criteria

- No production runtime path creates moving trains locally in TypeScript.
- Backend bundle seeding creates deterministic tram vehicles from `spawns.tram_lines`.
- Frontend renders backend tram vehicles visibly on the rail corridor.
- Diagnostics distinguish backend cars and backend trams.
- Full Rust and frontend gates pass.
- Forbidden fallback grep stays clean.
