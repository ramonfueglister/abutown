# Procedural Population & Shared Path Network Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Archived/closed in the 2026-05-29 documentation cleanup. This checklist is historical; `progress.md` and later plans are authoritative for current implementation status.

**Goal:** Scale `MobilityWorld` to ~1000 visible entities (~600 walking agents + ~400 car-Vehicles each driven by an Agent + 4 trams) sourced from a shared JSON city descriptor generated from the existing TS world build. Retire Phase-1's `RoadVehicleWorld` parallel subsystem in favor of the SUMO/MATSim agent+vehicle doctrine from `agent-mobility-foundation`.

**Architecture:** Generator script writes `data/city/zurich-network.json` from `buildZurichWorld`+`buildZurichTransport`+`buildPedestrianCorridors`. Backend loads it at startup, runs `mobility::seed::from_network(network, density)` producing walking agents, car-Vehicles+drivers, and trams. `VehicleRecord.kind: Car|Tram` plus polyline `LinkGeometry`. `RoadVehicleWorld` and its protocol/persistence/frontend surface are removed in this same slice.

**Tech Stack:** Rust 2024, Tokio, `serde_json`, Node ESM (generator), Vite JSON imports, existing mobility foundation.

**Spec:** `docs/superpowers/specs/2026-05-16-procedural-population-design.md`
**Roadmap:** `docs/superpowers/specs/2026-05-16-million-agent-roadmap-design.md` (Phase 3 of 8)

**One-time pre-deploy step (operator must run before this branch hits a backend with prior data):**

```sql
DELETE FROM mobility_snapshots;
```

Old rows lack the `kind` field on vehicle records inside the JSONB payload and will fail hydration. Documented in the plan and in the final progress.md note.

---

## File Structure

Backend new files:
- `scripts/generate-city-network.mjs` — Node ESM script that calls the TS city builders and writes `data/city/zurich-network.json`.
- `data/city/zurich-network.json` — generated, committed.
- `backend/crates/sim-core/src/city_network.rs` — JSON loader + typed struct.
- `backend/crates/sim-server/migrations/202605160005_drop_road_vehicle_snapshots.sql` — drops the now-unused table.

Backend modified:
- `backend/crates/protocol/src/lib.rs` — `VehicleKind` enum, `kind` on `VehicleMobilityDto`; remove `RoadVehicleDto`/`RoadVehicleSnapshotDto`/`RoadVehicleDeltaDto`; remove `ServerMessageDto::RoadVehicleDelta`.
- `backend/crates/sim-core/src/ids.rs` — remove `RoadVehicleId`.
- `backend/crates/sim-core/src/mobility.rs` — `VehicleKind` field on `VehicleRecord`; `seed::from_network`; rename `seed::initial_world` to `seed::tiny_world`; filter `InVehicle` agents out of `MobilityDelta::changed_agents`.
- `backend/crates/sim-core/src/mobility_geometry.rs` — polyline `LinkGeometry { points: Vec<(f32, f32)> }`, `world_coord_at_progress`, `direction_at_progress`.
- `backend/crates/sim-core/src/persistence.rs` — remove `RoadVehicleSnapshotStore` trait + InMemory impl + error type.
- `backend/crates/sim-core/src/lib.rs` — add `pub mod city_network;`; remove `pub mod road_vehicles;`.
- `backend/crates/sim-server/src/runtime.rs` — drop `road_vehicle_world` and `road_vehicle_snapshot_store` fields; revert `hydrate_from_stores` and `new_with_full_stores` to 3-arg (the `_all_stores` form Phase 2 had); drop `persist_road_vehicle_snapshot` and `road_vehicle_snapshot_dto`; drop `next_server_messages` road-vehicle push; new `hydrate_from_stores` and `new_with_all_stores` accept a `CityNetwork` for seeding new worlds.
- `backend/crates/sim-server/src/app.rs` — remove `/road-vehicles` route + handler; remove road-vehicle persist from snapshot loop; load `CityNetwork` at startup.
- `backend/crates/sim-server/src/lib.rs` — remove `pub mod postgres_road_vehicles;`.
- `backend/crates/sim-server/tests/http.rs` — remove `road_vehicles_endpoint_returns_seeded_snapshot`, `postgres_road_vehicle_state_survives_runtime_restart`; update other tests for new counts and shape.
- `backend/crates/sim-server/tests/websocket.rs` — remove `RoadVehicleDelta` read from `websocket_sends_hello_and_tile_pulse`.

Backend deleted:
- `backend/crates/sim-core/src/road_vehicles.rs`
- `backend/crates/sim-server/src/postgres_road_vehicles.rs`

Frontend modified:
- `package.json` — add `"generate:city-network": "node scripts/generate-city-network.mjs"`.
- `src/backend/mobilityProtocol.ts` — drop `RoadVehicleDeltaServerMessage`; add `VehicleKind` type and `kind` field on `VehicleMobilityDto`.
- `src/backend/mobilityState.ts` — drop `roadVehicles` field, `applyRoadVehicleSnapshotToState`, road-vehicle delta dispatch.
- `src/backend/mobilityClient.ts` — drop `/road-vehicles` fetch and any reconnect handling for it.
- `src/render/backendMobilityDrawables.ts` — `carsFromMobilityState` filters `state.vehicles` by `kind === 'car'`; `pedestriansFromMobilityState` excludes agents whose `state.type === 'in_vehicle'`.
- `src/main.ts` — diagnostic counts shift to read from `mobilityState.vehicles` filtered by kind.
- `tests/backend/mobilityProtocol.test.ts` — drop road-vehicle protocol assertions.
- `tests/backend/mobilityState.test.ts` — drop road-vehicle state tests.
- `tests/backend/mobilityClient.test.ts` — drop `/road-vehicles` fetch mocks.
- `tests/render/backendMobilityDrawables.test.ts` — update car/pedestrian projection tests for the new sources.
- `tests/e2e/render-smoke.spec.ts` — update entity-count expectations and replace `mobilityVehicles` shape if needed.

Frontend deleted:
- `src/backend/roadVehicleProtocol.ts`
- `src/backend/roadVehicleState.ts`
- `tests/backend/roadVehicleState.test.ts`

Docs:
- `progress.md` — record this phase, including the one-time DB delete step.

---

## Task 1: Generate City Network JSON

**Files:**
- Create: `scripts/generate-city-network.mjs`
- Create: `data/city/zurich-network.json` (generated artifact)
- Modify: `package.json`

- [x] **Step 1: Add npm script**

Edit `package.json` and add to the `scripts` object:

```json
"generate:city-network": "node scripts/generate-city-network.mjs"
```

- [x] **Step 2: Write generator script**

Create `scripts/generate-city-network.mjs`:

```js
#!/usr/bin/env node
import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { register } from 'node:module';

// Use the same TS runtime Vite uses by registering tsx as a loader.
register('tsx/esm', import.meta.url);

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');

const { buildZurichWorld } = await import(resolve(root, 'src/city/zurichWorld.ts'));
const { buildZurichTransport } = await import(resolve(root, 'src/city/zurichTransport.ts'));
const { buildPedestrianCorridors } = await import(resolve(root, 'src/city/pedestrianCorridors.ts'));

const world = buildZurichWorld({ seed: 1848 });
const transport = buildZurichTransport(world);
const corridors = buildPedestrianCorridors(transport.roads, { minLength: 5, maxCorridors: 260 });

const network = {
  version: 1,
  world_id: 'zurich-river-city-v1',
  chunk_size: 32,
  world_tiles: { width: world.width, height: world.height },
  arterial_paths: transport.arterialPaths.map((path) => path.map(({ x, y }) => ({ x, y }))),
  pedestrian_corridors: corridors.map((path) => path.map(({ x, y }) => ({ x, y }))),
};

const outPath = resolve(root, 'data/city/zurich-network.json');
mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, JSON.stringify(network, null, 2) + '\n');

console.log(
  `wrote ${outPath} — ${network.arterial_paths.length} arterial paths, ${network.pedestrian_corridors.length} pedestrian corridors`,
);
```

`tsx/esm` is the standard TS loader for Node ESM scripts. If it isn't already a dev-dep, add it:

```bash
npm install --save-dev tsx
```

- [x] **Step 3: Run the generator**

```bash
npm run generate:city-network
```

Expected: prints `wrote .../zurich-network.json — N arterial paths, M pedestrian corridors` with `N > 5` and `M > 30`.

- [x] **Step 4: Verify the output**

```bash
test -s data/city/zurich-network.json && head -20 data/city/zurich-network.json
```

Expected: JSON starts with `{`, has `"version": 1`, `"world_id": "zurich-river-city-v1"`, `"arterial_paths"`, `"pedestrian_corridors"`.

- [x] **Step 5: Commit**

```bash
git add scripts/generate-city-network.mjs data/city/zurich-network.json package.json package-lock.json
git commit -m "feat: generate shared zurich city network json"
```

---

## Task 2: Rust City Network Loader

**Files:**
- Create: `backend/crates/sim-core/src/city_network.rs`
- Modify: `backend/crates/sim-core/src/lib.rs`

- [x] **Step 1: Add module to lib.rs**

In `backend/crates/sim-core/src/lib.rs`, alongside the other `pub mod` lines, add:

```rust
pub mod city_network;
```

- [x] **Step 2: Write failing tests**

Create `backend/crates/sim-core/src/city_network.rs` with the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
        "version": 1,
        "world_id": "zurich-river-city-v1",
        "chunk_size": 32,
        "world_tiles": { "width": 256, "height": 256 },
        "arterial_paths": [
            [{"x": 10, "y": 20}, {"x": 14, "y": 20}, {"x": 14, "y": 24}]
        ],
        "pedestrian_corridors": [
            [{"x": 11, "y": 30}, {"x": 15, "y": 30}]
        ]
    }"#;

    #[test]
    fn parses_fixture_with_paths_and_corridors() {
        let network = CityNetwork::from_json(FIXTURE).expect("parses");
        assert_eq!(network.world_id, "zurich-river-city-v1");
        assert_eq!(network.chunk_size, 32);
        assert_eq!(network.arterial_paths.len(), 1);
        assert_eq!(network.arterial_paths[0].len(), 3);
        assert_eq!(network.arterial_paths[0][0], NetworkCoord { x: 10, y: 20 });
        assert_eq!(network.pedestrian_corridors.len(), 1);
    }

    #[test]
    fn rejects_payload_without_required_fields() {
        let bad = r#"{"version": 1}"#;
        assert!(CityNetwork::from_json(bad).is_err());
    }
}
```

- [x] **Step 3: Run to confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core city_network
```

Expected: FAIL — types don't exist.

- [x] **Step 4: Implement the loader**

Add to the same file, above the tests:

```rust
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkCoord {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldTiles {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityNetwork {
    pub version: u32,
    pub world_id: String,
    pub chunk_size: u16,
    pub world_tiles: WorldTiles,
    pub arterial_paths: Vec<Vec<NetworkCoord>>,
    pub pedestrian_corridors: Vec<Vec<NetworkCoord>>,
}

#[derive(Debug, thiserror::Error)]
pub enum CityNetworkError {
    #[error("failed to read city network: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse city network: {0}")]
    Parse(#[from] serde_json::Error),
}

impl CityNetwork {
    pub fn from_json(json: &str) -> Result<Self, CityNetworkError> {
        Ok(serde_json::from_str(json)?)
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, CityNetworkError> {
        let contents = std::fs::read_to_string(path)?;
        Self::from_json(&contents)
    }
}
```

- [x] **Step 5: Verify**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core city_network
```

Expected: PASS.

- [x] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/city_network.rs backend/crates/sim-core/src/lib.rs
git commit -m "feat: load shared city network from json"
```

---

## Task 3: VehicleKind + Kind Field On VehicleMobilityDto

**Files:**
- Modify: `backend/crates/protocol/src/lib.rs`

- [x] **Step 1: Write failing test**

In `backend/crates/protocol/src/lib.rs` test module:

```rust
#[test]
fn vehicle_mobility_dto_carries_kind() {
    let dto = VehicleMobilityDto {
        id: EntityId("vehicle:seed:0".to_string()),
        kind: VehicleKindDto::Car,
        route_id: "route:arterial:0".to_string(),
        link_index: 0,
        progress: 0.5,
        capacity: 1,
        occupants: vec![EntityId("agent:driver:0".to_string())],
        dwell_ticks_remaining: 0,
        world_coord: WorldCoordDto { x: 1.0, y: 2.0 },
        direction: DirectionDto::E,
        sprite_key: "car:0".to_string(),
    };
    let json = serde_json::to_value(&dto).unwrap();
    assert_eq!(json["kind"], "car");

    let back: VehicleMobilityDto = serde_json::from_value(json).unwrap();
    assert_eq!(back.kind, VehicleKindDto::Car);
}
```

- [x] **Step 2: Confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p abutown-protocol vehicle_mobility_dto_carries_kind
```

Expected: FAIL — `VehicleKindDto` and `kind` field don't exist.

- [x] **Step 3: Add the type and field**

In `backend/crates/protocol/src/lib.rs`, near `DirectionDto`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VehicleKindDto {
    Car,
    Tram,
}
```

Update `VehicleMobilityDto`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VehicleMobilityDto {
    pub id: EntityId,
    pub kind: VehicleKindDto,
    pub route_id: String,
    pub link_index: usize,
    pub progress: f32,
    pub capacity: u16,
    pub occupants: Vec<EntityId>,
    pub dwell_ticks_remaining: u16,
    pub world_coord: WorldCoordDto,
    pub direction: DirectionDto,
    pub sprite_key: String,
}
```

Update all existing protocol tests that construct `VehicleMobilityDto` literals to include `kind: VehicleKindDto::Tram` (preserves current tram-only semantic until Task 6 introduces cars).

- [x] **Step 4: Verify**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p abutown-protocol
```

Expected: green.

- [x] **Step 5: Update `From<&VehicleRecord> for VehicleMobilityDto` placeholder impl in mobility.rs**

In `backend/crates/sim-core/src/mobility.rs`, the placeholder `From<&VehicleRecord> for VehicleMobilityDto` impl (added in Phase 1 as a legacy fallback) needs the new `kind` field. Set it to `VehicleKindDto::Tram` as the placeholder — Task 6 will replace this path with `vehicle_dto_for` which reads the real kind from `VehicleRecord`. The build needs to succeed in this commit.

Find the impl and add the `kind` field:

```rust
impl From<&VehicleRecord> for VehicleMobilityDto {
    fn from(value: &VehicleRecord) -> Self {
        Self {
            id: EntityId(value.id.0.clone()),
            kind: abutown_protocol::VehicleKindDto::Tram,
            // ... rest unchanged ...
        }
    }
}
```

Also update the live builder `vehicle_dto_for` in `mobility.rs` (added in Phase 1) to read `value.kind` from `VehicleRecord` — but `VehicleRecord` doesn't have `kind` yet. So for THIS task, hardcode `kind: VehicleKindDto::Tram` in `vehicle_dto_for` too. Task 6 adds the `kind` field on `VehicleRecord` and switches `vehicle_dto_for` to read it.

- [x] **Step 6: Verify workspace**

```bash
cargo test --locked --manifest-path backend/Cargo.toml --workspace
```

Expected: green.

- [x] **Step 7: Commit**

```bash
git add backend/crates/protocol/src/lib.rs backend/crates/sim-core/src/mobility.rs
git commit -m "feat: add VehicleKind to vehicle DTO with tram default"
```

---

## Task 4: Polyline LinkGeometry

**Files:**
- Modify: `backend/crates/sim-core/src/mobility_geometry.rs`

- [x] **Step 1: Add failing tests**

Append to the test module:

```rust
#[test]
fn polyline_world_coord_at_progress_walks_arc_length() {
    let geom = LinkGeometry {
        points: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)],
    };
    // Total arc length = 20. At progress 0.25, we're 5 units in (along first segment).
    assert_eq!(geom.world_coord_at_progress(0.0), (0.0, 0.0));
    let mid_first = geom.world_coord_at_progress(0.25);
    assert!((mid_first.0 - 5.0).abs() < 0.01);
    assert!((mid_first.1 - 0.0).abs() < 0.01);

    // At progress 0.75 we're 15 units in: full first segment (10) + 5 on second.
    let mid_second = geom.world_coord_at_progress(0.75);
    assert!((mid_second.0 - 10.0).abs() < 0.01);
    assert!((mid_second.1 - 5.0).abs() < 0.01);

    let end = geom.world_coord_at_progress(1.0);
    assert!((end.0 - 10.0).abs() < 0.01);
    assert!((end.1 - 10.0).abs() < 0.01);
}

#[test]
fn polyline_direction_at_progress_returns_local_segment_direction() {
    use abutown_protocol::DirectionDto;
    let geom = LinkGeometry {
        points: vec![(0.0, 0.0), (10.0, 0.0), (10.0, -10.0)],
    };
    assert_eq!(geom.direction_at_progress(0.25), DirectionDto::E);
    assert_eq!(geom.direction_at_progress(0.75), DirectionDto::N);
}

#[test]
fn polyline_with_two_points_matches_old_start_end_semantics() {
    let geom = LinkGeometry {
        points: vec![(0.0, 0.0), (10.0, 0.0)],
    };
    assert_eq!(geom.world_coord_at_progress(0.5), (5.0, 0.0));
}
```

- [x] **Step 2: Run to confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core polyline
```

Expected: FAIL — `LinkGeometry.points` and methods don't exist (current shape is `start`/`end`).

- [x] **Step 3: Rewrite LinkGeometry**

Replace the `LinkGeometry` struct in `mobility_geometry.rs`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct LinkGeometry {
    pub points: Vec<(f32, f32)>,
}

impl LinkGeometry {
    pub fn world_coord_at_progress(&self, progress: f32) -> (f32, f32) {
        if self.points.len() < 2 {
            return self.points.first().copied().unwrap_or((0.0, 0.0));
        }
        let t = progress.clamp(0.0, 1.0);
        let total = self.arc_length();
        if total <= 0.0 {
            return self.points[0];
        }
        let target = t * total;
        let mut walked = 0.0;
        for window in self.points.windows(2) {
            let (ax, ay) = window[0];
            let (bx, by) = window[1];
            let seg = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
            if walked + seg >= target {
                let local_t = if seg > 0.0 { (target - walked) / seg } else { 0.0 };
                return (ax + (bx - ax) * local_t, ay + (by - ay) * local_t);
            }
            walked += seg;
        }
        *self.points.last().unwrap()
    }

    pub fn direction_at_progress(&self, progress: f32) -> abutown_protocol::DirectionDto {
        if self.points.len() < 2 {
            return abutown_protocol::DirectionDto::S;
        }
        let t = progress.clamp(0.0, 1.0);
        let total = self.arc_length();
        if total <= 0.0 {
            return abutown_protocol::DirectionDto::S;
        }
        let target = t * total;
        let mut walked = 0.0;
        for window in self.points.windows(2) {
            let (ax, ay) = window[0];
            let (bx, by) = window[1];
            let seg = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
            if walked + seg >= target {
                return direction_from_delta(bx - ax, by - ay);
            }
            walked += seg;
        }
        let (ax, ay) = self.points[self.points.len() - 2];
        let (bx, by) = *self.points.last().unwrap();
        direction_from_delta(bx - ax, by - ay)
    }

    fn arc_length(&self) -> f32 {
        let mut total = 0.0;
        for window in self.points.windows(2) {
            let (ax, ay) = window[0];
            let (bx, by) = window[1];
            total += ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
        }
        total
    }
}
```

Update all existing constructors of `LinkGeometry` in the same file:

```rust
LinkGeometry { points: vec![chunk_center(4, 4), chunk_center(5, 4)] }
```

(In each `link_geometry(...)` match arm. The previous `start: chunk_center(4,4), end: chunk_center(5,4)` becomes a 2-point polyline.)

Update `route_link_world_coord` to use `world_coord_at_progress`:

```rust
pub fn route_link_world_coord(route_id: &str, link_index: usize, progress: f32) -> Option<(f32, f32)> {
    let link_id = match (route_id, link_index) {
        ("route:horizontal", 0) => "link:horizontal:main",
        ("route:vertical", 0) => "link:vertical:main",
        _ => return None,
    };
    let geom = link_geometry(link_id)?;
    Some(geom.world_coord_at_progress(progress))
}
```

Update the `link_geometry_lookup_returns_seeded_routes` test to use the new `points` field instead of `start`/`end`:

```rust
fn link_geometry_lookup_returns_seeded_routes() {
    let h = link_geometry("link:horizontal:main").expect("horizontal link defined");
    assert_eq!(h.points.first(), Some(&(4.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0)));
    assert_eq!(h.points.last(), Some(&(5.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0)));
    assert_eq!(h.points.len(), 2);
    // ... rest unchanged ...
}
```

The interpolation test in `route_link_geometry_interpolates_progress` keeps working because at `progress=0.5` on a 2-point polyline, you get the midpoint of those 2 points.

- [x] **Step 4: Update MobilityWorld helpers using start/end**

In `backend/crates/sim-core/src/mobility.rs`, find `world_coord_for_agent` and `direction_for_agent`. They currently read `geom.start` and `geom.end`. Replace with the new helpers:

```rust
AgentMobilityState::Walking { link_id, progress } => {
    let geom = link_geometry(&link_id.0)?;
    Some(geom.world_coord_at_progress(*progress))
}
```

```rust
AgentMobilityState::Walking { link_id, .. } => {
    let geom = link_geometry(&link_id.0)?;
    let agent = self.agents.get(agent_id)?;
    let progress = match &agent.state {
        AgentMobilityState::Walking { progress, .. } => *progress,
        _ => 0.5,
    };
    Some(geom.direction_at_progress(progress))
}
```

Adjust nearby code paths likewise. Any use of `geom.start.0 + (geom.end.0 - geom.start.0) * t` becomes `geom.world_coord_at_progress(t)`.

- [x] **Step 5: Verify workspace**

```bash
cargo test --locked --manifest-path backend/Cargo.toml --workspace
```

Expected: green. If any existing test broke because it asserted on `start`/`end` fields, update it to use `points.first()`/`points.last()`.

- [x] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/mobility_geometry.rs backend/crates/sim-core/src/mobility.rs
git commit -m "feat: polyline link geometry"
```

---

## Task 5: VehicleKind Field On VehicleRecord

**Files:**
- Modify: `backend/crates/sim-core/src/mobility.rs`

- [x] **Step 1: Write failing test**

Append to the mobility.rs test module:

```rust
#[test]
fn seeded_world_vehicles_default_to_tram_kind() {
    let world = seed::initial_world();
    for vehicle in world.vehicles.values() {
        assert_eq!(vehicle.kind, VehicleKind::Tram);
    }
}
```

- [x] **Step 2: Run to confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core seeded_world_vehicles_default_to_tram_kind
```

Expected: FAIL — `VehicleKind` and `vehicle.kind` don't exist.

- [x] **Step 3: Add VehicleKind enum and field**

In `backend/crates/sim-core/src/mobility.rs`, add near the other public types:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VehicleKind {
    Car,
    Tram,
}

impl From<VehicleKind> for abutown_protocol::VehicleKindDto {
    fn from(value: VehicleKind) -> Self {
        match value {
            VehicleKind::Car => abutown_protocol::VehicleKindDto::Car,
            VehicleKind::Tram => abutown_protocol::VehicleKindDto::Tram,
        }
    }
}
```

Update `VehicleRecord`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VehicleRecord {
    pub id: VehicleId,
    pub kind: VehicleKind,
    pub route_id: RouteId,
    pub link_index: usize,
    pub progress: f32,
    pub speed_per_tick: f32,
    pub capacity: u16,
    pub occupants: Vec<AgentId>,
    pub dwell_ticks_remaining: u16,
}
```

Update the existing seed `initial_world` (or `tiny_world` after rename — for this task it's still `initial_world`) to set `kind: VehicleKind::Tram` for the 4 seeded vehicles.

Update `vehicle_dto_for` to read the real kind:

```rust
pub fn vehicle_dto_for(&self, vehicle_id: &VehicleId) -> Option<abutown_protocol::VehicleMobilityDto> {
    let vehicle = self.vehicles.get(vehicle_id)?;
    // ... rest ...
    Some(abutown_protocol::VehicleMobilityDto {
        id: abutown_protocol::EntityId(vehicle.id.0.clone()),
        kind: vehicle.kind.into(),
        // ... rest unchanged ...
    })
}
```

Update the `From<&VehicleRecord> for VehicleMobilityDto` placeholder similarly to read `value.kind.into()` instead of hardcoded.

- [x] **Step 4: Verify**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core
```

Expected: green. Mobility tests will need to satisfy serde — the JSON-serialised `MobilityWorld` now has `kind` on each vehicle. If any test compares serialised JSON strings, update those.

- [x] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/mobility.rs
git commit -m "feat: VehicleKind field on VehicleRecord"
```

---

## Task 6: Seed From Network

**Files:**
- Modify: `backend/crates/sim-core/src/mobility.rs`

- [x] **Step 1: Add tests**

Append to mobility.rs test module:

```rust
#[test]
fn from_network_produces_expected_population_counts() {
    use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};

    let network = CityNetwork {
        version: 1,
        world_id: "test".to_string(),
        chunk_size: 32,
        world_tiles: WorldTiles { width: 256, height: 256 },
        arterial_paths: vec![
            vec![NetworkCoord { x: 10, y: 20 }, NetworkCoord { x: 30, y: 20 }],
            vec![NetworkCoord { x: 40, y: 60 }, NetworkCoord { x: 60, y: 60 }],
        ],
        pedestrian_corridors: vec![
            vec![NetworkCoord { x: 11, y: 30 }, NetworkCoord { x: 31, y: 30 }],
            vec![NetworkCoord { x: 41, y: 70 }, NetworkCoord { x: 61, y: 70 }],
            vec![NetworkCoord { x: 71, y: 80 }, NetworkCoord { x: 91, y: 80 }],
        ],
    };

    let density = seed::SeedDensity {
        pedestrians_per_corridor: 6,
        cars_per_arterial: 4,
        trams_total: 4,
    };
    let world = seed::from_network(&network, density);

    let walking_agents = world.agents.values().filter(|a| matches!(a.state, AgentMobilityState::Walking { .. })).count();
    let driving_agents = world.agents.values().filter(|a| matches!(a.state, AgentMobilityState::InVehicle { .. })).count();
    let cars = world.vehicles.values().filter(|v| v.kind == VehicleKind::Car).count();
    let trams = world.vehicles.values().filter(|v| v.kind == VehicleKind::Tram).count();

    assert_eq!(walking_agents, 18, "3 corridors x 6 = 18 walkers");
    assert_eq!(cars, 8, "2 arterials x 4 = 8 cars");
    assert_eq!(driving_agents, 8, "one driver per car");
    assert_eq!(trams, 4);
}

#[test]
fn from_network_is_deterministic() {
    use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};
    let network = CityNetwork {
        version: 1,
        world_id: "test".to_string(),
        chunk_size: 32,
        world_tiles: WorldTiles { width: 256, height: 256 },
        arterial_paths: vec![vec![NetworkCoord { x: 0, y: 0 }, NetworkCoord { x: 10, y: 0 }]],
        pedestrian_corridors: vec![vec![NetworkCoord { x: 0, y: 5 }, NetworkCoord { x: 10, y: 5 }]],
    };
    let density = seed::SeedDensity { pedestrians_per_corridor: 3, cars_per_arterial: 2, trams_total: 0 };
    let a = seed::from_network(&network, density);
    let b = seed::from_network(&network, density);
    assert_eq!(a, b);
}

#[test]
fn from_network_assigns_drivers_to_cars() {
    use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};
    let network = CityNetwork {
        version: 1,
        world_id: "test".to_string(),
        chunk_size: 32,
        world_tiles: WorldTiles { width: 256, height: 256 },
        arterial_paths: vec![vec![NetworkCoord { x: 0, y: 0 }, NetworkCoord { x: 10, y: 0 }]],
        pedestrian_corridors: vec![],
    };
    let density = seed::SeedDensity { pedestrians_per_corridor: 0, cars_per_arterial: 2, trams_total: 0 };
    let world = seed::from_network(&network, density);

    assert_eq!(world.vehicles.len(), 2);
    for vehicle in world.vehicles.values() {
        assert_eq!(vehicle.kind, VehicleKind::Car);
        assert_eq!(vehicle.capacity, 1);
        assert_eq!(vehicle.occupants.len(), 1, "each car has its driver");
        let driver_id = &vehicle.occupants[0];
        let driver = world.agents.get(driver_id).expect("driver agent exists");
        match &driver.state {
            AgentMobilityState::InVehicle { vehicle_id, .. } => {
                assert_eq!(vehicle_id, &vehicle.id);
            }
            other => panic!("driver state expected InVehicle, got {other:?}"),
        }
    }
}
```

- [x] **Step 2: Confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core from_network
```

Expected: FAIL.

- [x] **Step 3: Rename initial_world to tiny_world**

In `backend/crates/sim-core/src/mobility.rs`, find the `pub mod seed { pub fn initial_world() -> MobilityWorld { ... } }`. Rename the function to `tiny_world`. Then add `pub fn initial_world()` that just delegates to `tiny_world()` so callers don't break:

```rust
pub fn initial_world() -> MobilityWorld {
    tiny_world()
}

pub fn tiny_world() -> MobilityWorld {
    // ... existing body ...
}
```

Callers of `seed::initial_world` (e.g., in `SimulationRuntime::new_with_stores` and `hydrate_from_stores`) keep working without code changes.

- [x] **Step 4: Add seed::from_network**

In the same `seed` module, add:

```rust
#[derive(Debug, Clone, Copy)]
pub struct SeedDensity {
    pub pedestrians_per_corridor: u32,
    pub cars_per_arterial: u32,
    pub trams_total: u32,
}

impl Default for SeedDensity {
    fn default() -> Self {
        Self {
            pedestrians_per_corridor: 6,
            cars_per_arterial: 4,
            trams_total: 4,
        }
    }
}

pub fn from_network(
    network: &crate::city_network::CityNetwork,
    density: SeedDensity,
) -> MobilityWorld {
    use crate::city_network::NetworkCoord;

    let mut world = MobilityWorld::default();
    let mut routes: HashMap<RouteId, RouteRecord> = HashMap::new();
    let mut links: Vec<(LinkId, Vec<(f32, f32)>)> = Vec::new();

    // Register pedestrian corridors as walking links.
    for (index, corridor) in network.pedestrian_corridors.iter().enumerate() {
        let link_id = LinkId(format!("link:walk:corridor:{index}"));
        let points: Vec<(f32, f32)> = corridor.iter().map(|NetworkCoord { x, y }| (*x as f32, *y as f32)).collect();
        links.push((link_id.clone(), points));
    }

    // Spawn walking agents distributed across corridors.
    let pedestrian_count = network.pedestrian_corridors.len() as u32 * density.pedestrians_per_corridor;
    for n in 0..pedestrian_count {
        let corridor_index = (n as usize) % network.pedestrian_corridors.len();
        let agent_id = AgentId(format!("agent:walk:{n}"));
        let link_id = LinkId(format!("link:walk:corridor:{corridor_index}"));
        let progress = ((n as f32) / (density.pedestrians_per_corridor as f32)).fract();
        world.agents.insert(
            agent_id.clone(),
            AgentRecord {
                id: agent_id,
                state: AgentMobilityState::Walking {
                    link_id: link_id.clone(),
                    progress,
                },
                plan: vec![PlanStage::Activity {
                    activity_id: format!("activity:wander:{corridor_index}"),
                }],
                plan_cursor: 0,
                walk_speed_per_tick: 0.05,
            },
        );
    }

    // Register arterial paths as routes for cars (one route per arterial, one link).
    for (index, arterial) in network.arterial_paths.iter().enumerate() {
        let route_id = RouteId(format!("route:arterial:{index}"));
        let link_id = LinkId(format!("link:arterial:{index}"));
        let points: Vec<(f32, f32)> = arterial.iter().map(|NetworkCoord { x, y }| (*x as f32, *y as f32)).collect();
        links.push((link_id.clone(), points));
        routes.insert(
            route_id.clone(),
            RouteRecord {
                id: route_id.clone(),
                links: vec![link_id.clone()],
            },
        );
    }

    // Spawn cars + drivers.
    let mut driver_index: u32 = 0;
    for (arterial_index, _arterial) in network.arterial_paths.iter().enumerate() {
        for n in 0..density.cars_per_arterial {
            let vehicle_id = VehicleId(format!("vehicle:car:{arterial_index}:{n}"));
            let route_id = RouteId(format!("route:arterial:{arterial_index}"));
            let driver_id = AgentId(format!("agent:driver:{driver_index}"));
            driver_index += 1;
            world.vehicles.insert(
                vehicle_id.clone(),
                VehicleRecord {
                    id: vehicle_id.clone(),
                    kind: VehicleKind::Car,
                    route_id: route_id.clone(),
                    link_index: 0,
                    progress: (n as f32) / (density.cars_per_arterial as f32),
                    speed_per_tick: 0.02,
                    capacity: 1,
                    occupants: vec![driver_id.clone()],
                    dwell_ticks_remaining: 0,
                },
            );
            world.agents.insert(
                driver_id.clone(),
                AgentRecord {
                    id: driver_id,
                    state: AgentMobilityState::InVehicle {
                        vehicle_id: vehicle_id.clone(),
                        seat_index: 0,
                    },
                    plan: vec![PlanStage::Activity {
                        activity_id: format!("activity:drive:{arterial_index}"),
                    }],
                    plan_cursor: 0,
                    walk_speed_per_tick: 0.05,
                },
            );
        }
    }

    // Trams: keep simple — reuse the existing tiny_world trams if needed.
    if density.trams_total > 0 {
        let tram_seed = tiny_world();
        for vehicle in tram_seed.vehicles.values() {
            world.vehicles.insert(vehicle.id.clone(), vehicle.clone());
        }
        for agent in tram_seed.agents.values() {
            world.agents.insert(agent.id.clone(), agent.clone());
        }
        // Preserve transit routes/stops from tiny_world (so the trams have something to ride on).
        for (id, record) in &tram_seed.routes {
            routes.insert(id.clone(), record.clone());
        }
        for stop in tram_seed.stops.values() {
            world.stops.insert(stop.id.clone(), stop.clone());
        }
    }

    world.routes = routes;
    world
}
```

The `mobility_geometry::link_geometry` lookup currently only knows about the hardcoded test links. For dynamic corridor links produced by `from_network` we need a way to resolve them at runtime. Add a stored set of polylines on `MobilityWorld`:

```rust
pub struct MobilityWorld {
    // ... existing fields ...
    pub link_polylines: HashMap<LinkId, Vec<(f32, f32)>>,
}
```

Set `world.link_polylines = links.into_iter().collect();` at the end of `from_network`. Initialize to empty in `MobilityWorld::default()` and `tiny_world()` (the tiny test world keeps using the hardcoded `mobility_geometry::link_geometry` lookups).

Update `MobilityWorld::world_coord_for_agent` and `direction_for_agent` so the polyline source is: first check `self.link_polylines`, fall back to `mobility_geometry::link_geometry`. Sketch:

```rust
fn resolve_link_polyline(&self, link_id: &LinkId) -> Option<LinkGeometry> {
    if let Some(points) = self.link_polylines.get(link_id) {
        return Some(LinkGeometry { points: points.clone() });
    }
    crate::mobility_geometry::link_geometry(&link_id.0)
}
```

Then in `world_coord_for_agent`:

```rust
AgentMobilityState::Walking { link_id, progress } => {
    let geom = self.resolve_link_polyline(link_id)?;
    Some(geom.world_coord_at_progress(*progress))
}
```

Same in `direction_for_agent`. Same in `world_coord_for_vehicle` and `direction_for_vehicle` for the per-link polyline lookup.

`world_coord_for_vehicle` currently calls `route_link_world_coord(route_id, link_index, progress)`. Extend it to read from `world.routes` for dynamic routes:

```rust
pub fn world_coord_for_vehicle(&self, vehicle_id: &VehicleId) -> Option<(f32, f32)> {
    let vehicle = self.vehicles.get(vehicle_id)?;
    let route = self.routes.get(&vehicle.route_id)?;
    let link_id = route.links.get(vehicle.link_index)?;
    let geom = self.resolve_link_polyline(link_id)?;
    Some(geom.world_coord_at_progress(vehicle.progress))
}
```

`direction_for_vehicle` similarly via `geom.direction_at_progress(vehicle.progress)`.

(Drop the legacy `route_link_world_coord` if no longer used, or keep it as a thin wrapper around the new lookup. For YAGNI: keep it for `tiny_world`'s benefit since its routes don't populate `link_polylines`.)

- [x] **Step 5: Verify**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core
```

Expected: green. New tests pass, old tests pass.

- [x] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/mobility.rs
git commit -m "feat: seed mobility from city network"
```

---

## Task 7: Filter InVehicle Agents From Delta Broadcasts

**Files:**
- Modify: `backend/crates/sim-core/src/mobility.rs`

- [x] **Step 1: Add failing test**

```rust
#[test]
fn delta_dto_excludes_in_vehicle_agents() {
    use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};
    let network = CityNetwork {
        version: 1,
        world_id: "test".to_string(),
        chunk_size: 32,
        world_tiles: WorldTiles { width: 256, height: 256 },
        arterial_paths: vec![vec![NetworkCoord { x: 0, y: 0 }, NetworkCoord { x: 10, y: 0 }]],
        pedestrian_corridors: vec![vec![NetworkCoord { x: 0, y: 5 }, NetworkCoord { x: 10, y: 5 }]],
    };
    let density = seed::SeedDensity { pedestrians_per_corridor: 2, cars_per_arterial: 2, trams_total: 0 };
    let world = seed::from_network(&network, density);

    let world_id = WorldId("test".to_string());
    let delta_input = MobilityDelta {
        changed_agents: world.agents.values().cloned().collect(),
        changed_vehicles: vec![],
    };
    let dto = build_mobility_delta_dto(&world_id, world.tick(), &world, &delta_input);

    for agent in &dto.changed_agents {
        assert_ne!(
            agent.state,
            abutown_protocol::AgentMobilityStateDto::InVehicle {
                vehicle_id: agent.state.in_vehicle_id_for_test().unwrap_or(abutown_protocol::EntityId(String::new())),
                seat_index: 0,
            },
            "delta should not include in_vehicle agents"
        );
    }

    // Simpler check: no agent in the broadcast carries an in_vehicle state tag.
    for agent in &dto.changed_agents {
        match &agent.state {
            abutown_protocol::AgentMobilityStateDto::InVehicle { .. } => panic!("in_vehicle agent leaked into delta"),
            _ => {}
        }
    }
}
```

Drop the first redundant assertion — the loop-based check is cleaner. Replace the test body:

```rust
#[test]
fn delta_dto_excludes_in_vehicle_agents() {
    use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};
    let network = CityNetwork {
        version: 1,
        world_id: "test".to_string(),
        chunk_size: 32,
        world_tiles: WorldTiles { width: 256, height: 256 },
        arterial_paths: vec![vec![NetworkCoord { x: 0, y: 0 }, NetworkCoord { x: 10, y: 0 }]],
        pedestrian_corridors: vec![vec![NetworkCoord { x: 0, y: 5 }, NetworkCoord { x: 10, y: 5 }]],
    };
    let density = seed::SeedDensity { pedestrians_per_corridor: 2, cars_per_arterial: 2, trams_total: 0 };
    let world = seed::from_network(&network, density);
    let drivers: Vec<_> = world
        .agents
        .values()
        .filter(|a| matches!(a.state, AgentMobilityState::InVehicle { .. }))
        .count();
    assert!(drivers > 0, "test setup should contain at least one in_vehicle driver agent");

    let world_id = WorldId("test".to_string());
    let delta_input = MobilityDelta {
        changed_agents: world.agents.values().cloned().collect(),
        changed_vehicles: vec![],
    };
    let dto = build_mobility_delta_dto(&world_id, world.tick(), &world, &delta_input);
    for agent in &dto.changed_agents {
        match &agent.state {
            abutown_protocol::AgentMobilityStateDto::InVehicle { .. } => {
                panic!("in_vehicle agent leaked into delta: {}", agent.id.0);
            }
            _ => {}
        }
    }
}

#[test]
fn snapshot_dto_includes_all_agents_even_in_vehicle() {
    use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};
    let network = CityNetwork {
        version: 1,
        world_id: "test".to_string(),
        chunk_size: 32,
        world_tiles: WorldTiles { width: 256, height: 256 },
        arterial_paths: vec![vec![NetworkCoord { x: 0, y: 0 }, NetworkCoord { x: 10, y: 0 }]],
        pedestrian_corridors: vec![],
    };
    let density = seed::SeedDensity { pedestrians_per_corridor: 0, cars_per_arterial: 2, trams_total: 0 };
    let world = seed::from_network(&network, density);
    let world_id = WorldId("test".to_string());
    let snap = build_mobility_snapshot_dto(&world_id, world.tick(), &world);
    assert_eq!(snap.agents.len(), 2, "snapshot must include in_vehicle drivers so clients can hydrate state");
}
```

(The snapshot keeps the in_vehicle agents — only deltas filter them. Initial hydration needs the full picture; deltas don't because the client tracks agent state across messages.)

- [x] **Step 2: Confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core delta_dto_excludes_in_vehicle snapshot_dto_includes_all_agents
```

Expected: FAIL — current `build_mobility_delta_dto` includes all changed agents.

- [x] **Step 3: Filter in the delta builder**

In `mobility.rs`, find `build_mobility_delta_dto`. Update:

```rust
pub fn build_mobility_delta_dto(
    world_id: &WorldId,
    tick: u64,
    world: &MobilityWorld,
    delta: &MobilityDelta,
) -> abutown_protocol::MobilityDeltaDto {
    let changed_agents = delta
        .changed_agents
        .iter()
        .filter(|agent| !matches!(agent.state, AgentMobilityState::InVehicle { .. }))
        .filter_map(|agent| world.agent_dto_for(&agent.id))
        .collect();
    let changed_vehicles = delta
        .changed_vehicles
        .iter()
        .filter_map(|vehicle| world.vehicle_dto_for(&vehicle.id))
        .collect();
    abutown_protocol::MobilityDeltaDto {
        protocol_version: abutown_protocol::PROTOCOL_VERSION,
        world_id: world_id.clone(),
        tick,
        changed_agents,
        changed_vehicles,
    }
}
```

`build_mobility_snapshot_dto` stays as-is — it includes ALL agents including in_vehicle ones so clients can hydrate state on first load.

- [x] **Step 4: Verify**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core
```

Expected: green.

- [x] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/mobility.rs
git commit -m "feat: filter in_vehicle agents out of mobility delta broadcasts"
```

---

## Task 8: Remove RoadVehicleWorld Backend Stack

This task removes a substantial amount of code across several files in a single coherent commit. It MUST keep the workspace green at the end.

**Files (delete):**
- `backend/crates/sim-core/src/road_vehicles.rs`
- `backend/crates/sim-server/src/postgres_road_vehicles.rs`

**Files (modify):**
- `backend/crates/sim-core/src/lib.rs`
- `backend/crates/sim-core/src/ids.rs`
- `backend/crates/sim-core/src/persistence.rs`
- `backend/crates/sim-server/src/lib.rs`
- `backend/crates/sim-server/src/runtime.rs`
- `backend/crates/sim-server/src/app.rs`
- `backend/crates/sim-server/tests/http.rs`
- `backend/crates/sim-server/tests/websocket.rs`
- `backend/crates/protocol/src/lib.rs`

- [x] **Step 1: Delete the road-vehicle modules**

```bash
git rm backend/crates/sim-core/src/road_vehicles.rs backend/crates/sim-server/src/postgres_road_vehicles.rs
```

- [x] **Step 2: Drop the module declarations**

In `backend/crates/sim-core/src/lib.rs`, remove the line `pub mod road_vehicles;`.

In `backend/crates/sim-server/src/lib.rs`, remove the line `pub mod postgres_road_vehicles;`.

- [x] **Step 3: Remove `RoadVehicleId` from ids.rs**

In `backend/crates/sim-core/src/ids.rs`, delete the entire `pub struct RoadVehicleId(pub String);` block.

- [x] **Step 4: Drop `RoadVehicleSnapshotStore` from persistence.rs**

In `backend/crates/sim-core/src/persistence.rs`, delete:
- `pub struct RoadVehicleSnapshotStoreError` and its `impl`.
- `pub trait RoadVehicleSnapshotStore` and its async methods.
- `pub struct InMemoryRoadVehicleSnapshotStore` and its `Default`/`impl` blocks.
- The `use crate::road_vehicles::RoadVehicleWorld;` import.
- The two `road_vehicle_snapshot_store_*` tests at the bottom of the test module.

- [x] **Step 5: Drop road-vehicle DTOs from protocol/lib.rs**

In `backend/crates/protocol/src/lib.rs`, delete:
- `pub struct RoadVehicleDto { ... }`.
- `pub struct RoadVehicleSnapshotDto { ... }`.
- `pub struct RoadVehicleDeltaDto { ... }`.
- The `ServerMessageDto::RoadVehicleDelta(RoadVehicleDeltaDto)` variant.
- Any protocol tests asserting on RoadVehicle* DTOs (e.g. `road_vehicle_dto_serializes_full_shape`, `road_vehicle_delta_serializes_with_type_tag`).

- [x] **Step 6: Drop road-vehicle from SimulationRuntime**

In `backend/crates/sim-server/src/runtime.rs`:
- Remove the `use sim_core::road_vehicles::*;` and any `use sim_core::persistence::{InMemoryRoadVehicleSnapshotStore, RoadVehicleSnapshotStore, RoadVehicleSnapshotStoreError}` imports.
- Remove the `use crate::postgres_road_vehicles::PostgresRoadVehicleSnapshotStore;` import (if present here; main location is `app.rs`).
- Remove the `road_vehicle_world` and `road_vehicle_snapshot_store` fields from `SimulationRuntime`.
- Remove the `HydrationError::RoadVehicle(...)` variant.
- Remove the `new_with_full_stores` constructor entirely — the 3-arg `new_with_all_stores` is sufficient. Callers must use it instead.
- Revert `hydrate_from_stores` to 3-arg: `event_store, snapshot_store, mobility_snapshot_store`. Drop the `road_vehicle_snapshot_store` read.
- Remove `persist_road_vehicle_snapshot` method.
- Remove `road_vehicle_snapshot_dto` getter.
- Remove `set_road_vehicle_world_for_test` and `road_vehicle_world_clone_for_test` test helpers.
- In `next_server_messages`, remove the road-vehicle delta tick + push.
- In the `Self { ... }` blocks of `new_with_stores`, `new()`, `new_with_event_store`, and `hydrate_from_stores`, remove the `road_vehicle_world` and `road_vehicle_snapshot_store` field initializers.

- [x] **Step 7: Drop road-vehicle from app.rs**

In `backend/crates/sim-server/src/app.rs`:
- Remove the `use crate::postgres_road_vehicles::PostgresRoadVehicleSnapshotStore;` import.
- Remove the `.route("/road-vehicles", get(road_vehicles))` line.
- Remove the `road_vehicles` handler function.
- In `build_app_from_config`, remove the `let road_vehicle_snapshot_store = PostgresRoadVehicleSnapshotStore::connect(...)` line.
- Update the `SimulationRuntime::hydrate_from_stores(...)` call to pass only 3 stores (drop the road-vehicle arg).
- In `persist_snapshots_once`, remove the `runtime.persist_road_vehicle_snapshot()` block.

- [x] **Step 8: Update tests in sim-server**

In `backend/crates/sim-server/tests/http.rs`:
- Delete the `road_vehicles_endpoint_returns_seeded_snapshot` test.
- Delete the `postgres_road_vehicle_state_survives_runtime_restart` test.
- Update any other test that references `new_with_full_stores` to use `new_with_all_stores` (3-arg).
- Update any other test that constructs `hydrate_from_stores(..., 4_args)` to use 3 args.
- Update tests that mock or assert on mobility counts — Phase 1's count was 20 agents + 4 vehicles + 80 road-vehicles in three buckets. Now it's whatever `seed::initial_world() = tiny_world()` produces (20 + 4 trams in one bucket).
- Drop any assertions on `state.city.mobilityVehicles` vs `state.city.mobilityAgents` shape if they reference road-vehicle specifics.

In `backend/crates/sim-server/tests/websocket.rs`:
- In `websocket_sends_hello_and_tile_pulse`, remove the `read_server_message(&mut stream)` call that expected a `RoadVehicleDelta` message. After the `MobilityDelta` read, the next message should be the second tick's `TilePulse`.
- Drop any other reference to `ServerMessageDto::RoadVehicleDelta`.

- [x] **Step 9: Verify build**

```bash
cargo build --locked --manifest-path backend/Cargo.toml --workspace
```

Expected: compiles. Any remaining references to removed types must be fixed.

- [x] **Step 10: Verify tests**

```bash
cargo test --locked --manifest-path backend/Cargo.toml --workspace
```

Expected: all green.

- [x] **Step 11: Verify clippy**

```bash
cargo clippy --locked --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```

Expected: clean.

- [x] **Step 12: Commit**

```bash
git add -A
git commit -m "refactor: retire RoadVehicleWorld in favor of MobilityWorld vehicles"
```

---

## Task 9: Drop Road Vehicle Snapshots Migration

**Files:**
- Create: `backend/crates/sim-server/migrations/202605160005_drop_road_vehicle_snapshots.sql`

- [x] **Step 1: Write the migration**

```sql
DROP TABLE IF EXISTS road_vehicle_snapshots;
```

- [x] **Step 2: Wire the migration into a runner**

The current migration loader is split across `postgres_events.rs`, `postgres_snapshots.rs`, `postgres_mobility.rs` — each loads its module-specific migration. There's no central runner.

Add the new migration as an `include_str!` constant in `postgres_mobility.rs` (the closest semantic neighbor — both touch mobility-related tables), and run it after the existing migration:

```rust
const MOBILITY_SNAPSHOTS_MIGRATION: &str =
    include_str!("../migrations/202605160002_mobility_snapshots.sql");
const DROP_ROAD_VEHICLE_SNAPSHOTS_MIGRATION: &str =
    include_str!("../migrations/202605160005_drop_road_vehicle_snapshots.sql");
```

After the existing `for statement in MOBILITY_SNAPSHOTS_MIGRATION ...` loop in `connect()`, add an analogous loop for the drop:

```rust
for statement in DROP_ROAD_VEHICLE_SNAPSHOTS_MIGRATION
    .split(';')
    .map(str::trim)
    .filter(|statement| !statement.is_empty())
{
    sqlx::query(statement)
        .execute(&pool)
        .await
        .map_err(|error| MobilitySnapshotStoreError::unavailable(error.to_string()))?;
}
```

- [x] **Step 3: Verify**

```bash
cargo test --locked --manifest-path backend/Cargo.toml --workspace
```

Expected: green. The drop runs on every connect; idempotent thanks to `IF EXISTS`.

- [x] **Step 4: Commit**

```bash
git add backend/crates/sim-server/migrations/202605160005_drop_road_vehicle_snapshots.sql backend/crates/sim-server/src/postgres_mobility.rs
git commit -m "feat: drop unused road_vehicle_snapshots table on backend startup"
```

---

## Task 10: Load CityNetwork At Backend Startup And Seed From It

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [x] **Step 1: Add SeedDensity to SimulationRuntime constructors**

In `backend/crates/sim-server/src/runtime.rs`, add a `pub const SEED_DENSITY: sim_core::mobility::seed::SeedDensity = sim_core::mobility::seed::SeedDensity { pedestrians_per_corridor: 6, cars_per_arterial: 4, trams_total: 4 };` alongside the existing constants.

Add a new constructor `new_from_network(network: &sim_core::city_network::CityNetwork) -> Self` that builds a runtime using `seed::from_network(network, SEED_DENSITY)` instead of `seed::tiny_world()`:

```rust
pub fn new_from_network(network: &sim_core::city_network::CityNetwork) -> Self {
    let mut runtime = Self::new();
    runtime.mobility = sim_core::mobility::seed::from_network(network, SEED_DENSITY);
    runtime
}
```

For hydration: extend `hydrate_from_stores` to accept an optional `&CityNetwork` argument. If provided AND the mobility store has no snapshot, seed from the network; otherwise the existing fallback to `seed::tiny_world()` applies (or use `from_network` as the default).

Concrete signature:

```rust
pub async fn hydrate_from_stores(
    event_store: Box<dyn WorldEventStore + Send>,
    snapshot_store: Box<dyn ChunkSnapshotStore + Send>,
    mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send>,
    network: &sim_core::city_network::CityNetwork,
) -> Result<Self, HydrationError>
```

The mobility fallback within `hydrate_from_stores` changes from `seed::tiny_world()` to `seed::from_network(network, SEED_DENSITY)`.

- [x] **Step 2: Load network in build_app_from_config**

In `backend/crates/sim-server/src/app.rs`:

```rust
const CITY_NETWORK_DEFAULT_PATH: &str = "data/city/zurich-network.json";

fn resolve_city_network_path() -> String {
    std::env::var("ABUTOWN_CITY_NETWORK_PATH").unwrap_or_else(|_| CITY_NETWORK_DEFAULT_PATH.to_string())
}

pub async fn build_app_from_config(config: &ServerConfig) -> anyhow::Result<Router> {
    let network = sim_core::city_network::CityNetwork::from_path(resolve_city_network_path())?;
    let event_store = PostgresWorldEventStore::connect(&config.database_url).await?;
    let snapshot_store = PostgresChunkSnapshotStore::connect(&config.database_url).await?;
    let mobility_snapshot_store = PostgresMobilitySnapshotStore::connect(&config.database_url).await?;

    let runtime = SimulationRuntime::hydrate_from_stores(
        Box::new(event_store),
        Box::new(snapshot_store),
        Box::new(mobility_snapshot_store),
        &network,
    )
    .await?;

    Ok(build_app_with_runtime(runtime))
}
```

`build_app()` (in-memory variant) should also seed from network if the env path resolves to a real file — else fall back to `tiny_world`:

```rust
pub fn build_app() -> Router {
    let runtime = match CityNetwork::from_path(resolve_city_network_path()) {
        Ok(network) => SimulationRuntime::new_from_network(&network),
        Err(_) => SimulationRuntime::new(),
    };
    build_app_with_runtime(runtime)
}
```

(For tests where the descriptor isn't present, `tiny_world` keeps existing assertions stable.)

- [x] **Step 3: Update all `hydrate_from_stores` callers**

`tests/http.rs` integration tests that call `hydrate_from_stores(...)` directly need the new 4th argument. For the in-memory tests, construct a minimal `CityNetwork` literal:

```rust
let network = sim_core::city_network::CityNetwork {
    version: 1,
    world_id: "test".to_string(),
    chunk_size: 32,
    world_tiles: sim_core::city_network::WorldTiles { width: 256, height: 256 },
    arterial_paths: vec![],
    pedestrian_corridors: vec![],
};
```

Empty arterial+corridor lists yield zero new walking/driving agents, so `from_network` produces only the tram fallback (4 trams + their associated agents). Tests asserting specific agent counts must be updated accordingly.

- [x] **Step 4: Verify**

```bash
cargo test --locked --manifest-path backend/Cargo.toml --workspace
```

Expected: green.

- [x] **Step 5: Commit**

```bash
git add backend/crates/sim-server/src/runtime.rs backend/crates/sim-server/src/app.rs backend/crates/sim-server/tests/http.rs
git commit -m "feat: backend seeds mobility from city network at startup"
```

---

## Task 11: Update Frontend Protocol For VehicleKind

**Files:**
- Modify: `src/backend/mobilityProtocol.ts`
- Modify: `tests/backend/mobilityProtocol.test.ts`

- [x] **Step 1: Update VehicleMobilityDto type**

In `src/backend/mobilityProtocol.ts`:

```ts
export type VehicleKindDto = 'car' | 'tram';

export type VehicleMobilityDto = {
  id: string;
  kind: VehicleKindDto;
  route_id: string;
  link_index: number;
  progress: number;
  capacity: number;
  occupants: string[];
  dwell_ticks_remaining: number;
  world_coord: WorldCoordDto;
  direction: DirectionDto;
  sprite_key: string;
};
```

Drop the `RoadVehicleDeltaServerMessage` type and remove it from the `ServerMessageDto` union (it was added in Phase 1).

Update `isVehicleMobilityDto` guard to validate `kind`:

```ts
const VEHICLE_KINDS: ReadonlySet<VehicleKindDto> = new Set(['car', 'tram']);

function isVehicleKindDto(value: unknown): value is VehicleKindDto {
  return typeof value === 'string' && VEHICLE_KINDS.has(value as VehicleKindDto);
}

function isVehicleMobilityDto(value: unknown): value is VehicleMobilityDto {
  if (!isObject(value)) return false;
  return (
    isString(value.id) &&
    isVehicleKindDto(value.kind) &&
    isString(value.route_id) &&
    isNonNegativeInteger(value.link_index) &&
    isFiniteProgress(value.progress) &&
    isNonNegativeInteger(value.capacity) &&
    Array.isArray(value.occupants) &&
    value.occupants.every(isString) &&
    isNonNegativeInteger(value.dwell_ticks_remaining) &&
    isWorldCoordDto(value.world_coord) &&
    isDirectionDto(value.direction) &&
    isString(value.sprite_key)
  );
}
```

In `parseServerMessage`, remove the `road_vehicle_delta` branch.

- [x] **Step 2: Update test**

In `tests/backend/mobilityProtocol.test.ts`, any vehicle fixture must include `kind`. E.g.:

```ts
const vehicle = {
  id: 'vehicle:seed:0',
  kind: 'tram' as const,
  // ... rest ...
};
```

- [x] **Step 3: Verify**

```bash
npx vitest run
```

Expected: green.

- [x] **Step 4: Commit**

```bash
git add src/backend/mobilityProtocol.ts tests/backend/mobilityProtocol.test.ts
git commit -m "feat: typed VehicleKind on frontend; drop road-vehicle protocol"
```

---

## Task 12: Remove Frontend RoadVehicle Stack

**Files (delete):**
- `src/backend/roadVehicleProtocol.ts`
- `src/backend/roadVehicleState.ts`
- `tests/backend/roadVehicleState.test.ts`

**Files (modify):**
- `src/backend/mobilityState.ts`
- `src/backend/mobilityClient.ts`
- `tests/backend/mobilityClient.test.ts`
- `tests/backend/mobilityState.test.ts`

- [x] **Step 1: Delete frontend road-vehicle modules**

```bash
git rm src/backend/roadVehicleProtocol.ts src/backend/roadVehicleState.ts tests/backend/roadVehicleState.test.ts
```

- [x] **Step 2: Drop roadVehicles from MobilityOverlayState**

In `src/backend/mobilityState.ts`:
- Remove the `roadVehicles: RoadVehicleOverlayState` field from `MobilityOverlayState`.
- Remove the `applyRoadVehicleSnapshotToState` export.
- In `applyServerMessage`, drop the `if (message?.type === 'road_vehicle_delta')` branch.
- In `mobilityDiagnostics`, drop the `roadVehicles: state.roadVehicles.vehicles.size` line.
- Remove the imports `applyRoadVehicleSnapshot`, `applyRoadVehicleDelta`, `createRoadVehicleOverlayState`, `RoadVehicleOverlayState`, `RoadVehicleSnapshotDto`.

- [x] **Step 3: Drop /road-vehicles fetch in mobilityClient**

In `src/backend/mobilityClient.ts`:
- Remove the `import { isRoadVehicleSnapshotDto, type RoadVehicleSnapshotDto } from './roadVehicleProtocol';`.
- Remove the `import { applyRoadVehicleSnapshotToState } from './mobilityState';`.
- In `requireMobilitySnapshot`, drop the `requestRoadVehicleSnapshot(...)` call and the `applyRoadVehicleSnapshotToState(...)` line.
- Drop the `requestRoadVehicleSnapshot` helper function.
- In the `connect()` inner function (websocket reconnect path), drop the road-vehicle fetch + apply.

- [x] **Step 4: Update mobilityClient test**

In `tests/backend/mobilityClient.test.ts`:
- Remove fetch-mock branches that handled `/road-vehicles`.
- Update assertions that checked road-vehicle state.

- [x] **Step 5: Update mobilityState test**

In `tests/backend/mobilityState.test.ts`:
- Remove any test asserting `state.roadVehicles` or invoking `road_vehicle_delta` payloads.

- [x] **Step 6: Verify**

```bash
npx vitest run
npx tsc --noEmit
npm run build
```

Expected: green.

- [x] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor: retire frontend RoadVehicle state and client"
```

---

## Task 13: Drawables Read From Vehicles With Kind Filter

**Files:**
- Modify: `src/render/backendMobilityDrawables.ts`
- Modify: `tests/render/backendMobilityDrawables.test.ts`

- [x] **Step 1: Replace existing tests**

In `tests/render/backendMobilityDrawables.test.ts`, the test that asserted cars come from `state.roadVehicles` must change to vehicles filtered by kind. And the pedestrian test must verify in_vehicle agents are skipped.

Rewrite the file:

```ts
import { describe, expect, it } from 'vitest';
import { pedestriansFromMobilityState, carsFromMobilityState } from '../../src/render/backendMobilityDrawables';
import { applyMobilityDelta, applyMobilitySnapshot, createMobilityOverlayState } from '../../src/backend/mobilityState';

const pedestrianSprites = [
  { sheet: 'pak128/peds.0', frameWidth: 16, frameHeight: 32 },
  { sheet: 'pak128/peds.1', frameWidth: 16, frameHeight: 32 },
];
const vehicleSprites = [
  { sheet: 'pak128/cars.0', frameWidth: 32, frameHeight: 32, scale: 1, role: 'vehicle.0' },
  { sheet: 'pak128/cars.1', frameWidth: 32, frameHeight: 32, scale: 1, role: 'vehicle.1' },
];

function makeStateWith(agents: Parameters<typeof applyMobilitySnapshot>[1]['agents'], vehicles: Parameters<typeof applyMobilitySnapshot>[1]['vehicles']) {
  return applyMobilitySnapshot(
    createMobilityOverlayState(),
    { protocol_version: 1, world_id: 'test', tick: 1, agents, vehicles, stops: [] },
    0,
  );
}

describe('backendMobilityDrawables', () => {
  it('cars source from vehicles with kind=car', () => {
    const state = makeStateWith(
      [],
      [
        {
          id: 'vehicle:car:0',
          kind: 'car' as const,
          route_id: 'r', link_index: 0, progress: 0, capacity: 1, occupants: [], dwell_ticks_remaining: 0,
          world_coord: { x: 50, y: 50 }, direction: 'e' as const, sprite_key: 'car:0',
        },
        {
          id: 'vehicle:tram:0',
          kind: 'tram' as const,
          route_id: 'r', link_index: 0, progress: 0, capacity: 24, occupants: [], dwell_ticks_remaining: 0,
          world_coord: { x: 60, y: 60 }, direction: 'e' as const, sprite_key: 'tram:0',
        },
      ],
    );
    const cars = carsFromMobilityState(state, vehicleSprites, 0, 100);
    expect(cars).toHaveLength(1);
    expect(cars[0].id).toBe('vehicle:car:0');
  });

  it('pedestrians exclude in_vehicle agents', () => {
    const state = makeStateWith(
      [
        {
          id: 'agent:walker:0',
          state: { type: 'walking', link_id: 'link', progress: 0 },
          plan_cursor: 0,
          world_coord: { x: 10, y: 10 }, direction: 'e' as const, sprite_key: 'pedestrian:0',
        },
        {
          id: 'agent:driver:0',
          state: { type: 'in_vehicle', vehicle_id: 'vehicle:car:0', seat_index: 0 },
          plan_cursor: 0,
          world_coord: { x: 50, y: 50 }, direction: 'e' as const, sprite_key: 'pedestrian:0',
        },
      ],
      [],
    );
    const peds = pedestriansFromMobilityState(state, pedestrianSprites, 0, 100);
    expect(peds).toHaveLength(1);
    expect(peds[0].id).toBe('agent:walker:0');
  });

  it('returns empty arrays when no sprites are available', () => {
    const state = makeStateWith([], []);
    expect(pedestriansFromMobilityState(state, [], 0, 100)).toEqual([]);
    expect(carsFromMobilityState(state, [], 0, 100)).toEqual([]);
  });
});
```

- [x] **Step 2: Run to confirm failure**

```bash
npx vitest run tests/render/backendMobilityDrawables.test.ts
```

Expected: FAIL — current projector reads `state.roadVehicles` and doesn't filter in_vehicle agents.

- [x] **Step 3: Rewrite the projector**

Replace `src/render/backendMobilityDrawables.ts`:

```ts
import {
  interpolatedAgents,
  interpolatedVehicles,
  type MobilityOverlayState,
} from '../backend/mobilityState';
import type { DirectionDto } from '../backend/mobilityProtocol';

export type Coord = { x: number; y: number };

export type SimutransPedestrianSpriteLike = {
  sheet: string;
  frameWidth?: number;
  frameHeight?: number;
};

export type VehicleSpriteLike = {
  sheet: string;
  frameWidth?: number;
  frameHeight?: number;
  scale?: number;
  role: string;
};

export type BackendPedestrian = {
  id: string;
  path: Coord[];
  offset: number;
  speed: number;
  laneOffset: number;
  sprite: SimutransPedestrianSpriteLike;
  direction: DirectionDto;
};

export type BackendCar = {
  id: string;
  path: Coord[];
  offset: number;
  speed: number;
  sprite: VehicleSpriteLike;
  direction: DirectionDto;
};

const DIRECTION_VECTORS: Record<DirectionDto, Coord> = {
  n: { x: 0, y: -1 }, ne: { x: 1, y: -1 }, e: { x: 1, y: 0 }, se: { x: 1, y: 1 },
  s: { x: 0, y: 1 }, sw: { x: -1, y: 1 }, w: { x: -1, y: 0 }, nw: { x: -1, y: -1 },
};

function spriteIndexFromKey(key: string, modulus: number): number {
  const parts = key.split(':');
  const last = parts[parts.length - 1] ?? '0';
  const n = Number.parseInt(last, 10);
  if (Number.isNaN(n)) return 0;
  return ((n % modulus) + modulus) % modulus;
}

function syntheticPath(start: Coord, direction: DirectionDto): Coord[] {
  const vec = DIRECTION_VECTORS[direction];
  return [start, { x: start.x + vec.x, y: start.y + vec.y }];
}

export function pedestriansFromMobilityState(
  state: MobilityOverlayState,
  sprites: readonly SimutransPedestrianSpriteLike[],
  now: number,
  tickPeriodMs: number,
): BackendPedestrian[] {
  if (sprites.length === 0) return [];
  const agents = interpolatedAgents(state, now, tickPeriodMs)
    .filter((agent) => agent.state.type !== 'in_vehicle')
    .sort((a, b) => a.id.localeCompare(b.id));
  const out: BackendPedestrian[] = [];
  for (const agent of agents) {
    const sprite = sprites[spriteIndexFromKey(agent.sprite_key, sprites.length)];
    out.push({
      id: agent.id,
      path: syntheticPath(agent.world_coord, agent.direction),
      offset: 0,
      speed: 0,
      laneOffset: 0,
      sprite,
      direction: agent.direction,
    });
  }
  return out;
}

export function carsFromMobilityState(
  state: MobilityOverlayState,
  sprites: readonly VehicleSpriteLike[],
  now: number,
  tickPeriodMs: number,
): BackendCar[] {
  if (sprites.length === 0) return [];
  const vehicles = interpolatedVehicles(state, now, tickPeriodMs)
    .filter((vehicle) => vehicle.kind === 'car')
    .sort((a, b) => a.id.localeCompare(b.id));
  const out: BackendCar[] = [];
  for (const vehicle of vehicles) {
    const sprite = sprites[spriteIndexFromKey(vehicle.sprite_key, sprites.length)];
    out.push({
      id: vehicle.id,
      path: syntheticPath(vehicle.world_coord, vehicle.direction),
      offset: 0,
      speed: 0,
      sprite,
      direction: vehicle.direction,
    });
  }
  return out;
}
```

`interpolatedVehicles` already exists in `mobilityState.ts` (added Phase 2 alongside `interpolatedAgents`).

- [x] **Step 4: Verify**

```bash
npx vitest run
npx tsc --noEmit
npm run build
```

Expected: green.

- [x] **Step 5: Commit**

```bash
git add src/render/backendMobilityDrawables.ts tests/render/backendMobilityDrawables.test.ts
git commit -m "feat: drawables source cars from vehicles kind=car; pedestrians skip in_vehicle"
```

---

## Task 14: Diagnostic Counts In main.ts

**Files:**
- Modify: `src/main.ts`

- [x] **Step 1: Update main.ts diagnostics**

In `src/main.ts`, find the `render_game_to_text` function's `mobility` block. The current shape includes `roadVehicles: ...` and the diagnostics derived from `mobilityState.roadVehicles`. Replace:

```ts
const mobility = mobilityDiagnostics(mobilityState);
const visiblePedestrians = pedestriansFromMobilityState(mobilityState, pedestrianSprites, Date.now(), mobilityTickPeriodMs);
const visibleCars = carsFromMobilityState(mobilityState, vehicleSprites, Date.now(), mobilityTickPeriodMs);
```

and below, in the returned JSON's `city` block, drop the `roadVehicles: ...` field if present and add (or update):

```ts
mobility: {
  source: 'backend',
  status: mobility.status,
  tick: mobility.tick,
  agents: mobility.agents,
  vehicles: mobility.vehicles,
  stops: mobility.stops,
  walkingPedestrians: visiblePedestrians.length,
  cars: visibleCars.length,
  invalidMessages: mobility.invalidMessages,
  lastError: mobility.lastError,
},
```

Adjust `mobilityAgents`/`mobilityVehicles` block similarly — they may still reference road-vehicle shape.

- [x] **Step 2: Verify**

```bash
npx vitest run
npx tsc --noEmit
npm run build
```

Expected: green.

- [x] **Step 3: Commit**

```bash
git add src/main.ts
git commit -m "chore: update render_game_to_text diagnostics for unified vehicles"
```

---

## Task 15: E2E Smoke

**Files:**
- Modify: `tests/e2e/render-smoke.spec.ts`

- [x] **Step 1: Update assertions**

In `tests/e2e/render-smoke.spec.ts`:
- The Phase-1 assertions about `mobilityAgents.agents` and `mobilityVehicles.vehicles` need to reflect the new counts. With `seed::from_network(zurich-network, default density)` the expected counts are: walking agents ~600 (varies with corridor count, expect ≥ 50 to be lenient on test reliability); cars ≥ 50; trams ≥ 1.
- Drop assertions on `state.city.mobility.roadVehicles` if any remain.
- The Phase-2 interpolation assertion (`movedX + movedY > 0`) keeps working.
- The mobility-agent id pattern is `agent:walk:N` for walkers now (was `agent:seed:N`); update regex.

Concretely, replace the existing agent/vehicle assertions with:

```ts
expect(state.city.mobilityAgents.agents.length).toBeGreaterThanOrEqual(50);
expect(state.city.mobilityAgents.agents[0]).toEqual(expect.objectContaining({
  id: expect.stringMatching(/^agent:walk:/),
  state: expect.any(String),
}));
expect(state.city.mobilityVehicles.vehicles.length).toBeGreaterThanOrEqual(50);
const carVehicle = state.city.mobilityVehicles.vehicles.find(
  (v: { id: string }) => v.id.startsWith('vehicle:car:'),
);
expect(carVehicle).toBeDefined();
```

The `clickableAgent`/`clickableVehicle` selection blocks update for the new ids.

- [x] **Step 2: Verify ts compile**

```bash
npx tsc --noEmit
```

Expected: clean.

- [x] **Step 3: Commit**

```bash
git add tests/e2e/render-smoke.spec.ts
git commit -m "test: render smoke expects city-wide procedural population"
```

---

## Task 16: Final Quality Gate + progress.md

**Files:**
- Modify: `progress.md`

- [x] **Step 1: Run all gates**

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
cargo test --locked --manifest-path backend/Cargo.toml --workspace
cargo clippy --locked --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
npx vitest run
npm run build
```

Expected: all green. If fmt fails, run `cargo fmt --manifest-path backend/Cargo.toml --all` and stage.

- [x] **Step 2: Append to progress.md**

```
2026-05-16T<HH:MM:SS>.000Z - Procedural population: generated shared data/city/zurich-network.json from the existing TS world build; backend MobilityWorld now seeded across all 64 chunks (~600 walking agents + ~400 car-Vehicles with driver-Agents + 4 trams) via mobility::seed::from_network. Retired Phase-1's RoadVehicleWorld; cars are now Vehicles in MobilityWorld with kind=Car, drivers are Agents in InVehicle state. LinkGeometry is now a polyline. In-vehicle agents filtered from delta broadcasts. One-time pre-deploy: DELETE FROM mobility_snapshots (old rows lack kind field). Phase 3 of the million-agent roadmap.
```

- [x] **Step 3: Commit**

```bash
git add progress.md backend/ src/ tests/
git commit -m "chore: format and record procedural population progress"
```

---

## Self-Review

- **Spec coverage:**
  - Shared descriptor + generator → Task 1.
  - Rust CityNetwork loader → Task 2.
  - VehicleKind protocol + record → Tasks 3, 5.
  - Polyline LinkGeometry → Task 4.
  - seed::from_network → Task 6.
  - InVehicle filter in delta broadcasts → Task 7.
  - RoadVehicleWorld backend retirement → Task 8.
  - Drop table migration → Task 9.
  - Backend startup loads network → Task 10.
  - Frontend protocol typed kind → Task 11.
  - Frontend road-vehicle retirement → Task 12.
  - Drawables kind filter + in_vehicle skip → Task 13.
  - Diagnostics → Task 14.
  - E2E → Task 15.
  - Quality gate + progress → Task 16.

- **Placeholder scan:** every step has concrete code; no TBD/TODO/"similar to". The one judgment call in Task 1 ("add `tsx` dev-dep if not present") gives the engineer a concrete command.

- **Type consistency:**
  - `CityNetwork` / `NetworkCoord` / `WorldTiles` used consistently across Tasks 2, 6, 10.
  - `VehicleKind` (Rust) ↔ `VehicleKindDto` (protocol) ↔ `VehicleKindDto` (TS) all serialize as `'car'|'tram'`.
  - `LinkGeometry.points` (vec/array of (f32,f32)) used in Task 4 and consumed in Task 6.
  - `seed::from_network(network, density)` signature appears in Tasks 6, 10 identically.
  - `hydrate_from_stores` 4-arg signature consistent in Tasks 10, test updates in Tasks 8, 10.
  - `interpolatedVehicles` (already in mobilityState from Phase 2) consumed in Task 13.

- **Risks acknowledged in spec:** polyline math correctness (Task 4 tests cover happy paths + corners); old mobility_snapshots rows fail to deserialize (operator runs `DELETE FROM mobility_snapshots` per Task 16's progress note); generator drift addressed by committing the artifact.

- **Scope check:** This is a substantial single PR but coherent — generator + loader + scaled seeder + retirement of the parallel subsystem. Cannot split because the retirement and the scaled seeder are tightly coupled (cars move from road-vehicle subsystem to mobility-world). Could split into "rip out road-vehicles first, then add procedural seeding" but that leaves a window where cars are absent. Single PR keeps the system always functional.
