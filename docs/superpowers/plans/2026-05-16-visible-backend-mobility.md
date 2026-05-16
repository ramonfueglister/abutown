# Visible Backend Mobility Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the frontend's locally-built `pedestrians` and `cars` arrays with backend-authoritative mobility entities. Add a road-vehicle subsystem on the backend, extend mobility DTOs with `world_coord` + `direction` + `sprite_key`, and route the existing canvas sprite pipeline through backend state.

**Architecture:** Backend gains `RoadVehicleWorld` parallel to the existing transit `MobilityWorld`. A new `mobility_geometry` module provides deterministic tile-space coordinates for the seeded routes/links/stops. All mobility DTOs carry a server-computed `WorldCoordDto`, an 8-way `DirectionDto`, and a `sprite_key: String`. Frontend `buildPedestrians` and `buildCars` are removed; a new `backendMobilityDrawables` module projects `mobilityState` into the same `Pedestrian`/`Car` shapes that the existing `drawPedestrian`/`drawCar` consume.

**Tech Stack:** Rust 2024, Tokio, `async_trait`, `sqlx` (Postgres), Serde, Axum WebSocket broadcast. TypeScript, Vite, Vitest, Playwright. Existing pak128 sprite catalog.

**Spec:** `docs/superpowers/specs/2026-05-16-visible-backend-mobility-design.md`
**Roadmap:** `docs/superpowers/specs/2026-05-16-million-agent-roadmap-design.md` (Phase 1 of 8)

---

## File Structure

Backend:
- Modify `backend/crates/sim-core/src/ids.rs` — add `TileCoord`, `RoadVehicleId`.
- Modify `backend/crates/protocol/src/lib.rs` — add `WorldCoordDto`, `DirectionDto`; extend `AgentMobilityDto`/`VehicleMobilityDto`; add `RoadVehicleDto`/`RoadVehicleSnapshotDto`/`RoadVehicleDeltaDto`; add `ServerMessageDto::RoadVehicleDelta` variant.
- Create `backend/crates/sim-core/src/mobility_geometry.rs` — hardcoded link/stop/route/path coordinates plus lookup helpers.
- Modify `backend/crates/sim-core/src/mobility.rs` — `world_coord_for_agent`, `direction_for_agent`, `world_coord_for_vehicle`, `direction_for_vehicle`, `sprite_key_for_agent`, `sprite_key_for_vehicle`; snapshot/delta DTO builders updated to populate the new fields.
- Create `backend/crates/sim-core/src/road_vehicles.rs` — `RoadVehicleRecord`, `RoadVehicleWorld`, `tick_road_vehicles`, `world_coord`, `direction`, `sprite_key`, `seed::initial_road_vehicles`, snapshot/delta DTO builders.
- Modify `backend/crates/sim-core/src/persistence.rs` — add `RoadVehicleSnapshotStore` trait + `InMemoryRoadVehicleSnapshotStore`.
- Modify `backend/crates/sim-core/src/lib.rs` — export `mobility_geometry`, `road_vehicles`.
- Create `backend/crates/sim-server/migrations/202605160003_road_vehicle_snapshots.sql`.
- Create `backend/crates/sim-server/src/postgres_road_vehicles.rs` — Postgres adapter.
- Modify `backend/crates/sim-server/src/lib.rs` — export `postgres_road_vehicles`.
- Modify `backend/crates/sim-server/src/runtime.rs` — add `road_vehicle_world` + `road_vehicle_snapshot_store` fields; constructors and `hydrate_from_stores` extended; new `persist_road_vehicle_snapshot`; tick coupling in `next_server_messages`.
- Modify `backend/crates/sim-server/src/app.rs` — `GET /road-vehicles`; WebSocket broadcast includes road-vehicle deltas; snapshot loop calls the new persist; `build_app_from_config` passes the new store.
- Modify `backend/crates/sim-server/tests/http.rs` — integration tests for `/road-vehicles`, road-vehicle persistence round-trip (opt-in Postgres).

Frontend:
- Create `src/backend/roadVehicleProtocol.ts` — DTO types + runtime guards for road-vehicle snapshot/delta.
- Create `src/backend/roadVehicleState.ts` — pure reducer/read model for road-vehicle state.
- Modify `src/backend/mobilityProtocol.ts` — extend `AgentMobilityDto`/`VehicleMobilityDto` with `world_coord`, `direction`, `sprite_key`.
- Modify `src/backend/mobilityState.ts` — embed road-vehicle state; expose combined diagnostics.
- Modify `src/backend/mobilityClient.ts` — fetch `GET /road-vehicles` at boot; subscribe to `road_vehicle_delta` messages; reconnect handling covers both.
- Create `src/render/backendMobilityDrawables.ts` — `pedestriansFromMobilityState(state, catalog)`, `carsFromMobilityState(state, catalog)`.
- Modify `src/main.ts` — remove `buildCars`/`buildPedestrians`; per-frame projection from `mobilityState`; selection by backend IDs; diagnostics report `backend-mobility` as the source.
- Delete `src/render/pedestrianAgents.ts`, `src/render/pedestrianAgentInspector.ts`, `src/render/localRoadVehicles.ts`, `src/render/roadVehicleInspector.ts` — replaced by backend-driven equivalents.
- Modify or create corresponding tests in `tests/render/` and `tests/backend/`.
- Modify `tests/e2e/render-smoke.spec.ts` — assert backend-sourced mobility.

Docs:
- Modify `progress.md` — record this phase's completion.

---

## Task 1: Protocol DTOs and New IDs

**Files:**
- Modify: `backend/crates/sim-core/src/ids.rs`
- Modify: `backend/crates/protocol/src/lib.rs`

- [ ] **Step 1: Add new IDs to sim-core**

In `backend/crates/sim-core/src/ids.rs`, append:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Hash, Eq, Serialize, Deserialize)]
pub struct TileCoord {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoadVehicleId(pub String);
```

- [ ] **Step 2: Write failing protocol DTO tests**

In `backend/crates/protocol/src/lib.rs`, inside `#[cfg(test)] mod tests`, append:

```rust
#[test]
fn world_coord_dto_round_trips() {
    let coord = WorldCoordDto { x: 12.5, y: -3.25 };
    let json = serde_json::to_value(&coord).unwrap();
    assert_eq!(json["x"], 12.5);
    assert_eq!(json["y"], -3.25);
    let back: WorldCoordDto = serde_json::from_value(json).unwrap();
    assert_eq!(back, coord);
}

#[test]
fn direction_dto_serializes_as_compass_string() {
    assert_eq!(serde_json::to_value(&DirectionDto::N).unwrap(), serde_json::json!("n"));
    assert_eq!(serde_json::to_value(&DirectionDto::Sw).unwrap(), serde_json::json!("sw"));
    let parsed: DirectionDto = serde_json::from_value(serde_json::json!("ne")).unwrap();
    assert_eq!(parsed, DirectionDto::Ne);
}

#[test]
fn agent_mobility_dto_carries_world_coord_direction_and_sprite_key() {
    let dto = AgentMobilityDto {
        id: EntityId("agent:seed:0".to_string()),
        state: AgentMobilityStateDto::Walking { link_id: "link:demo".to_string(), progress: 0.5 },
        plan_cursor: 0,
        world_coord: WorldCoordDto { x: 1.0, y: 2.0 },
        direction: DirectionDto::E,
        sprite_key: "pedestrian:0".to_string(),
    };
    let json = serde_json::to_value(&dto).unwrap();
    assert_eq!(json["world_coord"]["x"], 1.0);
    assert_eq!(json["world_coord"]["y"], 2.0);
    assert_eq!(json["direction"], "e");
    assert_eq!(json["sprite_key"], "pedestrian:0");
}

#[test]
fn road_vehicle_dto_serializes_full_shape() {
    let dto = RoadVehicleDto {
        id: "road_vehicle:seed:0".to_string(),
        world_coord: WorldCoordDto { x: 5.0, y: 6.0 },
        direction: DirectionDto::N,
        sprite_key: "vehicle:0".to_string(),
    };
    let json = serde_json::to_value(&dto).unwrap();
    assert_eq!(json["id"], "road_vehicle:seed:0");
    assert_eq!(json["world_coord"]["y"], 6.0);
    assert_eq!(json["direction"], "n");
    assert_eq!(json["sprite_key"], "vehicle:0");
}

#[test]
fn road_vehicle_delta_serializes_with_type_tag() {
    let delta = ServerMessageDto::RoadVehicleDelta(RoadVehicleDeltaDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        tick: 4,
        changed: vec![RoadVehicleDto {
            id: "road_vehicle:seed:0".to_string(),
            world_coord: WorldCoordDto { x: 5.0, y: 6.0 },
            direction: DirectionDto::N,
            sprite_key: "vehicle:0".to_string(),
        }],
    });
    let json = serde_json::to_value(&delta).unwrap();
    assert_eq!(json["type"], "road_vehicle_delta");
    assert_eq!(json["tick"], 4);
    assert_eq!(json["changed"][0]["id"], "road_vehicle:seed:0");
}
```

- [ ] **Step 3: Run tests to confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p abutown-protocol world_coord direction_dto agent_mobility_dto_carries road_vehicle
```

Expected: FAIL — none of the new types or fields exist.

- [ ] **Step 4: Add the DTOs**

In `backend/crates/protocol/src/lib.rs`, add near the existing mobility DTOs:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WorldCoordDto {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectionDto {
    N,
    Ne,
    E,
    Se,
    S,
    Sw,
    W,
    Nw,
}
```

Update `AgentMobilityDto`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentMobilityDto {
    pub id: EntityId,
    pub state: AgentMobilityStateDto,
    pub plan_cursor: usize,
    pub world_coord: WorldCoordDto,
    pub direction: DirectionDto,
    pub sprite_key: String,
}
```

Update `VehicleMobilityDto`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VehicleMobilityDto {
    pub id: EntityId,
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

Add road-vehicle DTOs:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoadVehicleDto {
    pub id: String,
    pub world_coord: WorldCoordDto,
    pub direction: DirectionDto,
    pub sprite_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoadVehicleSnapshotDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub tick: u64,
    pub vehicles: Vec<RoadVehicleDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoadVehicleDeltaDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub tick: u64,
    pub changed: Vec<RoadVehicleDto>,
}
```

Add the new `ServerMessageDto` variant. Replace the existing enum:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessageDto {
    Hello(ServerHelloDto),
    TilePulse(TilePulseDeltaDto),
    MobilityDelta(MobilityDeltaDto),
    RoadVehicleDelta(RoadVehicleDeltaDto),
    WorldEvent { event: WorldEventDto },
    Error(ServerErrorDto),
}
```

Update the existing `mobility_snapshot_serializes_agents_vehicles_and_stops` test in this file: every literal `AgentMobilityDto` / `VehicleMobilityDto` constructor must also supply `world_coord`, `direction`, `sprite_key`. Use representative values (e.g. `WorldCoordDto { x: 0.0, y: 0.0 }`, `DirectionDto::E`, `"pedestrian:0".to_string()`). Same for the existing `websocket_mobility_delta_serializes_with_type_tag` test.

- [ ] **Step 5: Run tests to confirm pass**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p abutown-protocol
```

Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/ids.rs backend/crates/protocol/src/lib.rs
git commit -m "feat: add world coord, direction and road vehicle DTOs"
```

---

## Task 2: Mobility Geometry Module

**Files:**
- Create: `backend/crates/sim-core/src/mobility_geometry.rs`
- Modify: `backend/crates/sim-core/src/lib.rs`

- [ ] **Step 1: Add module to lib.rs**

In `backend/crates/sim-core/src/lib.rs`, after the existing `pub mod mobility;` line, add:

```rust
pub mod mobility_geometry;
```

- [ ] **Step 2: Write failing tests**

Create `backend/crates/sim-core/src/mobility_geometry.rs` with only a test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_geometry_lookup_returns_seeded_routes() {
        let h = link_geometry("link:horizontal:main").expect("horizontal link defined");
        assert_eq!(h.start, (4.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0));
        assert_eq!(h.end, (5.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0));

        let v = link_geometry("link:vertical:main").expect("vertical link defined");
        assert_eq!(v.start, (4.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0));
        assert_eq!(v.end, (4.0 * 32.0 + 16.0, 5.0 * 32.0 + 16.0));

        assert!(link_geometry("link:walk:default").is_some(), "walk link must be defined for seeded agents");
    }

    #[test]
    fn stop_geometry_lookup_returns_seeded_stops() {
        let pickup = stop_geometry("stop:horizontal:pickup").expect("horizontal pickup defined");
        assert_eq!(pickup.coord, (4.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0));
        let dropoff = stop_geometry("stop:horizontal:dropoff").expect("horizontal dropoff defined");
        assert_eq!(dropoff.coord, (5.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0));
    }

    #[test]
    fn activity_geometry_falls_back_to_default_when_unknown() {
        let known = activity_geometry("activity:work").expect("work activity defined");
        assert!(known.coord.0 >= 0.0);
        assert!(activity_geometry("activity:unknown").is_some(), "unknown activities must still resolve to a default coord");
    }

    #[test]
    fn route_link_geometry_interpolates_progress() {
        let coord = route_link_world_coord("route:horizontal", 0, 0.5).expect("route exists");
        assert!((coord.0 - (4.0 * 32.0 + 16.0 + 16.0)).abs() < 0.01);
        assert!((coord.1 - (4.0 * 32.0 + 16.0)).abs() < 0.01);
    }

    #[test]
    fn direction_from_delta_matches_compass() {
        use abutown_protocol::DirectionDto;
        assert_eq!(direction_from_delta(1.0, 0.0), DirectionDto::E);
        assert_eq!(direction_from_delta(0.0, -1.0), DirectionDto::N);
        assert_eq!(direction_from_delta(-1.0, 0.0), DirectionDto::W);
        assert_eq!(direction_from_delta(0.0, 1.0), DirectionDto::S);
        assert_eq!(direction_from_delta(1.0, 1.0), DirectionDto::Se);
        assert_eq!(direction_from_delta(0.0, 0.0), DirectionDto::S);
    }
}
```

The seeded coordinates use chunk-center semantics: chunk `(cx, cy)` covers tiles `cx*32..(cx+1)*32` on each axis, so its center is `(cx*32+16, cy*32+16)`. The horizontal link runs between centers of chunks `(4,4)` and `(5,4)`, etc.

- [ ] **Step 3: Run tests to confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core mobility_geometry
```

Expected: FAIL — module is empty.

- [ ] **Step 4: Implement the module**

Replace the file contents with:

```rust
use abutown_protocol::DirectionDto;

const CHUNK_SIZE_F: f32 = 32.0;

#[inline]
fn chunk_center(cx: i32, cy: i32) -> (f32, f32) {
    (cx as f32 * CHUNK_SIZE_F + CHUNK_SIZE_F / 2.0,
     cy as f32 * CHUNK_SIZE_F + CHUNK_SIZE_F / 2.0)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkGeometry {
    pub start: (f32, f32),
    pub end: (f32, f32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StopGeometry {
    pub coord: (f32, f32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActivityGeometry {
    pub coord: (f32, f32),
}

pub fn link_geometry(link_id: &str) -> Option<LinkGeometry> {
    match link_id {
        "link:horizontal:main" => Some(LinkGeometry {
            start: chunk_center(4, 4),
            end: chunk_center(5, 4),
        }),
        "link:vertical:main" => Some(LinkGeometry {
            start: chunk_center(4, 4),
            end: chunk_center(4, 5),
        }),
        "link:walk:default" => Some(LinkGeometry {
            start: chunk_center(4, 4),
            end: chunk_center(5, 4),
        }),
        _ => None,
    }
}

pub fn stop_geometry(stop_id: &str) -> Option<StopGeometry> {
    match stop_id {
        "stop:horizontal:pickup" => Some(StopGeometry { coord: chunk_center(4, 4) }),
        "stop:horizontal:dropoff" => Some(StopGeometry { coord: chunk_center(5, 4) }),
        "stop:vertical:pickup" => Some(StopGeometry { coord: chunk_center(4, 4) }),
        "stop:vertical:dropoff" => Some(StopGeometry { coord: chunk_center(4, 5) }),
        _ => None,
    }
}

pub fn activity_geometry(activity_id: &str) -> Option<ActivityGeometry> {
    match activity_id {
        "activity:work" => Some(ActivityGeometry { coord: chunk_center(5, 4) }),
        _ => Some(ActivityGeometry { coord: chunk_center(4, 4) }),
    }
}

/// Returns the world coordinate along a route at `(link_index, progress)`.
/// Used when computing transit-vehicle positions.
pub fn route_link_world_coord(route_id: &str, link_index: usize, progress: f32) -> Option<(f32, f32)> {
    let link_id = match (route_id, link_index) {
        ("route:horizontal", 0) => "link:horizontal:main",
        ("route:vertical", 0) => "link:vertical:main",
        _ => return None,
    };
    let geom = link_geometry(link_id)?;
    let t = progress.clamp(0.0, 1.0);
    Some((
        geom.start.0 + (geom.end.0 - geom.start.0) * t,
        geom.start.1 + (geom.end.1 - geom.start.1) * t,
    ))
}

/// Maps a unit-ish movement delta to the closest 8-way direction.
/// `(0,0)` returns `S` as a stable default for stationary entities.
pub fn direction_from_delta(dx: f32, dy: f32) -> DirectionDto {
    if dx == 0.0 && dy == 0.0 {
        return DirectionDto::S;
    }
    let angle = dy.atan2(dx); // -PI..PI, with E = 0, S = PI/2, W = ±PI, N = -PI/2
    let sector = ((angle / std::f32::consts::FRAC_PI_4).round() as i32).rem_euclid(8);
    match sector {
        0 => DirectionDto::E,
        1 => DirectionDto::Se,
        2 => DirectionDto::S,
        3 => DirectionDto::Sw,
        4 => DirectionDto::W,
        5 => DirectionDto::Nw,
        6 => DirectionDto::N,
        7 => DirectionDto::Ne,
        _ => DirectionDto::S,
    }
}
```

Re-add the existing `#[cfg(test)] mod tests { ... }` block under the implementation (it stays as-is from Step 2).

- [ ] **Step 5: Run tests to confirm pass**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core mobility_geometry
```

Expected: all 5 tests pass.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/mobility_geometry.rs backend/crates/sim-core/src/lib.rs
git commit -m "feat: mobility geometry lookup module"
```

---

## Task 3: MobilityWorld Geometry Helpers + DTO Builders

**Files:**
- Modify: `backend/crates/sim-core/src/mobility.rs`

- [ ] **Step 1: Write failing tests**

Append to the existing `#[cfg(test)] mod tests` block in `mobility.rs`:

```rust
#[test]
fn world_coord_for_walking_agent_interpolates_link() {
    use crate::mobility_geometry::link_geometry;

    let mut world = seed::initial_world();
    let agent_id = AgentId("agent:seed:0".to_string());
    // Force a known state: walking along link:walk:default at progress 0.5.
    if let Some(agent) = world.agents.get_mut(&agent_id) {
        agent.state = AgentMobilityState::Walking {
            link_id: LinkId("link:walk:default".to_string()),
            progress: 0.5,
        };
    }

    let geom = link_geometry("link:walk:default").unwrap();
    let coord = world.world_coord_for_agent(&agent_id).expect("agent resolves to coord");
    assert!((coord.0 - (geom.start.0 + (geom.end.0 - geom.start.0) * 0.5)).abs() < 0.01);
    assert!((coord.1 - (geom.start.1 + (geom.end.1 - geom.start.1) * 0.5)).abs() < 0.01);
}

#[test]
fn world_coord_for_agent_waiting_at_stop_uses_stop_coord() {
    let mut world = seed::initial_world();
    let agent_id = AgentId("agent:seed:0".to_string());
    if let Some(agent) = world.agents.get_mut(&agent_id) {
        agent.state = AgentMobilityState::WaitingAtStop {
            stop_id: StopId("stop:horizontal:pickup".to_string()),
        };
    }
    let coord = world.world_coord_for_agent(&agent_id).unwrap();
    assert_eq!(coord, (4.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0));
}

#[test]
fn world_coord_for_transit_vehicle_interpolates_route() {
    let mut world = seed::initial_world();
    let vehicle_id = VehicleId("vehicle:seed:0".to_string());
    if let Some(vehicle) = world.vehicles.get_mut(&vehicle_id) {
        vehicle.route_id = RouteId("route:horizontal".to_string());
        vehicle.link_index = 0;
        vehicle.progress = 0.5;
    }
    let coord = world.world_coord_for_vehicle(&vehicle_id).expect("vehicle resolves");
    assert!((coord.0 - (4.0 * 32.0 + 16.0 + 16.0)).abs() < 0.01);
}

#[test]
fn sprite_key_for_agent_is_deterministic_by_id_hash() {
    let world = seed::initial_world();
    let a = world.sprite_key_for_agent(&AgentId("agent:seed:0".to_string())).unwrap();
    let b = world.sprite_key_for_agent(&AgentId("agent:seed:0".to_string())).unwrap();
    assert_eq!(a, b, "sprite key must be deterministic across calls for the same id");
    assert!(a.starts_with("pedestrian:"));
}

#[test]
fn build_mobility_snapshot_dto_includes_world_coord_direction_and_sprite_key() {
    let world = seed::initial_world();
    let world_id = WorldId("abutown-main".to_string());
    let dto = build_mobility_snapshot_dto(&world_id, world.tick(), world.snapshot_view(&world_id));

    let first_agent = dto.agents.first().expect("at least one agent in seed");
    assert!(first_agent.sprite_key.starts_with("pedestrian:"));
    // world_coord and direction should be populated (not the f32 default sentinel pair).
    assert!(first_agent.world_coord.x.is_finite());
}
```

Note: `world.snapshot_view(&world_id)` is a new helper introduced in Step 4 that returns whatever struct `build_mobility_snapshot_dto` consumes (today the `MobilitySnapshot` struct from this module). The test asserts behavior, not the helper signature. If the existing `MobilityWorld::snapshot()` is what's currently passed in, keep that name; this plan uses `snapshot_view` only as a placeholder if a rename helps clarity. If you keep the existing name, replace `world.snapshot_view(&world_id)` with whatever the existing builder call site uses (e.g. `world.snapshot()`).

- [ ] **Step 2: Run tests to confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core world_coord_for build_mobility_snapshot_dto_includes
```

Expected: FAIL — helpers don't exist yet; snapshot DTO doesn't include the new fields.

- [ ] **Step 3: Implement helpers on MobilityWorld**

Add inside `impl MobilityWorld` in `mobility.rs`:

```rust
pub fn world_coord_for_agent(&self, agent_id: &AgentId) -> Option<(f32, f32)> {
    use crate::mobility_geometry::{activity_geometry, link_geometry, stop_geometry};
    let agent = self.agents.get(agent_id)?;
    match &agent.state {
        AgentMobilityState::AtActivity { activity_id } => {
            activity_geometry(activity_id).map(|g| g.coord)
        }
        AgentMobilityState::Walking { link_id, progress } => {
            let geom = link_geometry(&link_id.0)?;
            let t = progress.clamp(0.0, 1.0);
            Some((
                geom.start.0 + (geom.end.0 - geom.start.0) * t,
                geom.start.1 + (geom.end.1 - geom.start.1) * t,
            ))
        }
        AgentMobilityState::WaitingAtStop { stop_id }
        | AgentMobilityState::Boarding { stop_id, .. }
        | AgentMobilityState::Alighting { stop_id, .. } => {
            stop_geometry(&stop_id.0).map(|g| g.coord)
        }
        AgentMobilityState::InVehicle { vehicle_id, .. } => {
            self.world_coord_for_vehicle(vehicle_id)
        }
    }
}

pub fn direction_for_agent(&self, agent_id: &AgentId) -> Option<abutown_protocol::DirectionDto> {
    use crate::mobility_geometry::{direction_from_delta, link_geometry};
    let agent = self.agents.get(agent_id)?;
    match &agent.state {
        AgentMobilityState::Walking { link_id, .. } => {
            let geom = link_geometry(&link_id.0)?;
            Some(direction_from_delta(geom.end.0 - geom.start.0, geom.end.1 - geom.start.1))
        }
        AgentMobilityState::InVehicle { vehicle_id, .. } => self.direction_for_vehicle(vehicle_id),
        _ => Some(abutown_protocol::DirectionDto::S),
    }
}

pub fn world_coord_for_vehicle(&self, vehicle_id: &VehicleId) -> Option<(f32, f32)> {
    use crate::mobility_geometry::route_link_world_coord;
    let vehicle = self.vehicles.get(vehicle_id)?;
    route_link_world_coord(&vehicle.route_id.0, vehicle.link_index, vehicle.progress)
}

pub fn direction_for_vehicle(&self, vehicle_id: &VehicleId) -> Option<abutown_protocol::DirectionDto> {
    use crate::mobility_geometry::{direction_from_delta, route_link_world_coord};
    let vehicle = self.vehicles.get(vehicle_id)?;
    let here = route_link_world_coord(&vehicle.route_id.0, vehicle.link_index, vehicle.progress)?;
    let ahead = route_link_world_coord(
        &vehicle.route_id.0,
        vehicle.link_index,
        (vehicle.progress + 0.1).min(1.0),
    )?;
    Some(direction_from_delta(ahead.0 - here.0, ahead.1 - here.1))
}

pub fn sprite_key_for_agent(&self, agent_id: &AgentId) -> Option<String> {
    if !self.agents.contains_key(agent_id) {
        return None;
    }
    Some(format!("pedestrian:{}", stable_index(&agent_id.0) % 16))
}

pub fn sprite_key_for_vehicle(&self, vehicle_id: &VehicleId) -> Option<String> {
    if !self.vehicles.contains_key(vehicle_id) {
        return None;
    }
    Some(format!("tram:{}", stable_index(&vehicle_id.0) % 4))
}
```

Add a small free helper near the top of the file (after the imports):

```rust
fn stable_index(id: &str) -> u32 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    (hasher.finish() as u32)
}
```

- [ ] **Step 4: Extend the snapshot/delta DTO builders**

Find the existing functions `build_mobility_snapshot_dto` and `build_mobility_delta_dto` in `mobility.rs`. The current signatures take `&self` indirectly via a `MobilitySnapshot` value object. The simplest change is to thread the live `MobilityWorld` into those builders so they can call the new helpers.

Replace the two functions with these signatures and bodies:

```rust
pub fn build_mobility_snapshot_dto(
    world_id: &WorldId,
    tick: u64,
    world: &MobilityWorld,
) -> abutown_protocol::MobilitySnapshotDto {
    let agents = world
        .agents
        .values()
        .map(|agent| agent_to_dto(world, agent))
        .collect();
    let vehicles = world
        .vehicles
        .values()
        .map(|vehicle| vehicle_to_dto(world, vehicle))
        .collect();
    let stops = world.stops.values().map(stop_to_dto).collect();
    abutown_protocol::MobilitySnapshotDto {
        protocol_version: abutown_protocol::PROTOCOL_VERSION,
        world_id: world_id.clone(),
        tick,
        agents,
        vehicles,
        stops,
    }
}

pub fn build_mobility_delta_dto(
    world_id: &WorldId,
    tick: u64,
    world: &MobilityWorld,
    delta: &MobilityDelta,
) -> abutown_protocol::MobilityDeltaDto {
    let changed_agents = delta
        .changed_agents
        .iter()
        .map(|agent| agent_to_dto(world, agent))
        .collect();
    let changed_vehicles = delta
        .changed_vehicles
        .iter()
        .map(|vehicle| vehicle_to_dto(world, vehicle))
        .collect();
    abutown_protocol::MobilityDeltaDto {
        protocol_version: abutown_protocol::PROTOCOL_VERSION,
        world_id: world_id.clone(),
        tick,
        changed_agents,
        changed_vehicles,
    }
}

fn agent_to_dto(world: &MobilityWorld, agent: &AgentRecord) -> abutown_protocol::AgentMobilityDto {
    let world_coord = world
        .world_coord_for_agent(&agent.id)
        .unwrap_or((0.0, 0.0));
    let direction = world
        .direction_for_agent(&agent.id)
        .unwrap_or(abutown_protocol::DirectionDto::S);
    let sprite_key = world
        .sprite_key_for_agent(&agent.id)
        .unwrap_or_else(|| "pedestrian:0".to_string());
    abutown_protocol::AgentMobilityDto {
        id: abutown_protocol::EntityId(agent.id.0.clone()),
        state: agent_state_to_dto(&agent.state),
        plan_cursor: agent.plan_cursor,
        world_coord: abutown_protocol::WorldCoordDto { x: world_coord.0, y: world_coord.1 },
        direction,
        sprite_key,
    }
}

fn vehicle_to_dto(world: &MobilityWorld, vehicle: &VehicleRecord) -> abutown_protocol::VehicleMobilityDto {
    let world_coord = world
        .world_coord_for_vehicle(&vehicle.id)
        .unwrap_or((0.0, 0.0));
    let direction = world
        .direction_for_vehicle(&vehicle.id)
        .unwrap_or(abutown_protocol::DirectionDto::S);
    let sprite_key = world
        .sprite_key_for_vehicle(&vehicle.id)
        .unwrap_or_else(|| "tram:0".to_string());
    abutown_protocol::VehicleMobilityDto {
        id: abutown_protocol::EntityId(vehicle.id.0.clone()),
        route_id: vehicle.route_id.0.clone(),
        link_index: vehicle.link_index,
        progress: vehicle.progress,
        capacity: vehicle.capacity,
        occupants: vehicle
            .occupants
            .iter()
            .map(|id| abutown_protocol::EntityId(id.0.clone()))
            .collect(),
        dwell_ticks_remaining: vehicle.dwell_ticks_remaining,
        world_coord: abutown_protocol::WorldCoordDto { x: world_coord.0, y: world_coord.1 },
        direction,
        sprite_key,
    }
}
```

Whatever the existing helper names are for `agent_state_to_dto` / `stop_to_dto`, reuse them. If they're inlined today, factor them out using the same shape.

Update every call site of `build_mobility_snapshot_dto`/`build_mobility_delta_dto` in `sim-core` and `sim-server` to pass `&MobilityWorld` (and for delta also pass `&MobilityDelta`). The runtime today calls these from `next_mobility_delta` and from snapshot construction — update both. Search:

```bash
grep -rn "build_mobility_snapshot_dto\|build_mobility_delta_dto" backend/
```

Adjust every result. If the runtime stored a separate `MobilitySnapshot` value before calling the builder, just pass `&self.mobility` (the `MobilityWorld`) instead — the builder now reads everything it needs from the world.

- [ ] **Step 5: Run tests to confirm pass**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core mobility
cargo test --locked --manifest-path backend/Cargo.toml --workspace
```

Expected: all green. If an existing test asserted on the old snapshot DTO shape (without world_coord/direction/sprite_key), update its expected literal to include the new fields. Do not delete assertions.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/mobility.rs backend/crates/sim-server/src/runtime.rs
git commit -m "feat: mobility DTOs carry world coord, direction and sprite key"
```

(Include `runtime.rs` if you had to update the call sites there.)

---

## Task 4: RoadVehicleWorld

**Files:**
- Create: `backend/crates/sim-core/src/road_vehicles.rs`
- Modify: `backend/crates/sim-core/src/lib.rs`

- [ ] **Step 1: Add module to lib.rs**

Append:

```rust
pub mod road_vehicles;
```

- [ ] **Step 2: Write failing tests**

Create `backend/crates/sim-core/src/road_vehicles.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{RoadVehicleId, TileCoord};

    #[test]
    fn tick_advances_offset_by_speed_and_wraps_at_path_end() {
        let mut world = RoadVehicleWorld::default();
        let id = RoadVehicleId("road_vehicle:test:0".to_string());
        world.insert(RoadVehicleRecord {
            id: id.clone(),
            path: vec![
                TileCoord { x: 0, y: 0 },
                TileCoord { x: 4, y: 0 },
                TileCoord { x: 4, y: 4 },
                TileCoord { x: 0, y: 4 },
            ],
            offset: 3.5,
            speed: 1.0,
            sprite_key: "vehicle:0".to_string(),
        });

        world.tick_road_vehicles();
        let stored = world.get(&id).unwrap();
        assert!((stored.offset - 0.5).abs() < 1e-5, "offset wraps past path length");
    }

    #[test]
    fn world_coord_interpolates_between_path_segments() {
        let mut world = RoadVehicleWorld::default();
        let id = RoadVehicleId("road_vehicle:test:0".to_string());
        world.insert(RoadVehicleRecord {
            id: id.clone(),
            path: vec![TileCoord { x: 0, y: 0 }, TileCoord { x: 4, y: 0 }],
            offset: 0.5,
            speed: 1.0,
            sprite_key: "vehicle:0".to_string(),
        });
        let coord = world.world_coord(&id).expect("coord exists");
        assert!((coord.0 - 2.0).abs() < 1e-5);
        assert!((coord.1 - 0.0).abs() < 1e-5);
    }

    #[test]
    fn direction_matches_path_orientation() {
        use abutown_protocol::DirectionDto;
        let mut world = RoadVehicleWorld::default();
        let id = RoadVehicleId("road_vehicle:test:0".to_string());
        world.insert(RoadVehicleRecord {
            id: id.clone(),
            path: vec![TileCoord { x: 0, y: 0 }, TileCoord { x: 0, y: -4 }],
            offset: 0.0,
            speed: 1.0,
            sprite_key: "vehicle:0".to_string(),
        });
        assert_eq!(world.direction(&id).unwrap(), DirectionDto::N);
    }

    #[test]
    fn initial_road_vehicles_seeds_a_useful_population() {
        let world = seed::initial_road_vehicles();
        assert!(world.vehicles.len() >= 80, "seed must populate at least 80 road vehicles");
        for vehicle in world.vehicles.values() {
            assert!(vehicle.path.len() >= 2, "every road vehicle path needs two points");
            assert!(vehicle.speed > 0.0);
            assert!(!vehicle.sprite_key.is_empty());
        }
    }

    #[test]
    fn seed_is_deterministic() {
        let a = seed::initial_road_vehicles();
        let b = seed::initial_road_vehicles();
        assert_eq!(a, b);
    }
}
```

- [ ] **Step 3: Run tests to confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core road_vehicles
```

Expected: FAIL — types don't exist.

- [ ] **Step 4: Implement RoadVehicleWorld**

Replace `backend/crates/sim-core/src/road_vehicles.rs` with:

```rust
use std::collections::HashMap;

use abutown_protocol::{DirectionDto, RoadVehicleDto, WorldCoordDto, WorldId};
use serde::{Deserialize, Serialize};

use crate::ids::{RoadVehicleId, TileCoord};
use crate::mobility_geometry::direction_from_delta;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoadVehicleRecord {
    pub id: RoadVehicleId,
    pub path: Vec<TileCoord>,
    pub offset: f32,
    pub speed: f32,
    pub sprite_key: String,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoadVehicleWorld {
    pub tick: u64,
    pub vehicles: HashMap<RoadVehicleId, RoadVehicleRecord>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct RoadVehicleDelta {
    pub changed: Vec<RoadVehicleId>,
}

impl RoadVehicleWorld {
    pub fn insert(&mut self, vehicle: RoadVehicleRecord) {
        self.vehicles.insert(vehicle.id.clone(), vehicle);
    }

    pub fn get(&self, id: &RoadVehicleId) -> Option<&RoadVehicleRecord> {
        self.vehicles.get(id)
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn tick_road_vehicles(&mut self) -> RoadVehicleDelta {
        self.tick = self.tick.wrapping_add(1);
        let mut changed = Vec::with_capacity(self.vehicles.len());
        for vehicle in self.vehicles.values_mut() {
            if vehicle.path.len() < 2 {
                continue;
            }
            let len = vehicle.path.len() as f32;
            vehicle.offset = (vehicle.offset + vehicle.speed).rem_euclid(len);
            changed.push(vehicle.id.clone());
        }
        RoadVehicleDelta { changed }
    }

    pub fn world_coord(&self, id: &RoadVehicleId) -> Option<(f32, f32)> {
        let vehicle = self.vehicles.get(id)?;
        let (a, b, t) = interpolate_path(vehicle)?;
        Some((
            a.x as f32 + (b.x - a.x) as f32 * t,
            a.y as f32 + (b.y - a.y) as f32 * t,
        ))
    }

    pub fn direction(&self, id: &RoadVehicleId) -> Option<DirectionDto> {
        let vehicle = self.vehicles.get(id)?;
        let (a, b, _t) = interpolate_path(vehicle)?;
        Some(direction_from_delta((b.x - a.x) as f32, (b.y - a.y) as f32))
    }
}

fn interpolate_path(vehicle: &RoadVehicleRecord) -> Option<(TileCoord, TileCoord, f32)> {
    if vehicle.path.len() < 2 {
        return None;
    }
    let len = vehicle.path.len();
    let base = vehicle.offset.floor() as usize % len;
    let next = (base + 1) % len;
    let t = vehicle.offset - vehicle.offset.floor();
    Some((vehicle.path[base], vehicle.path[next], t))
}

pub fn build_road_vehicle_dto(world: &RoadVehicleWorld, id: &RoadVehicleId) -> Option<RoadVehicleDto> {
    let vehicle = world.vehicles.get(id)?;
    let coord = world.world_coord(id).unwrap_or((0.0, 0.0));
    let direction = world.direction(id).unwrap_or(DirectionDto::S);
    Some(RoadVehicleDto {
        id: vehicle.id.0.clone(),
        world_coord: WorldCoordDto { x: coord.0, y: coord.1 },
        direction,
        sprite_key: vehicle.sprite_key.clone(),
    })
}

pub fn build_road_vehicle_snapshot_dto(
    world_id: &WorldId,
    world: &RoadVehicleWorld,
) -> abutown_protocol::RoadVehicleSnapshotDto {
    let vehicles = world
        .vehicles
        .keys()
        .filter_map(|id| build_road_vehicle_dto(world, id))
        .collect();
    abutown_protocol::RoadVehicleSnapshotDto {
        protocol_version: abutown_protocol::PROTOCOL_VERSION,
        world_id: world_id.clone(),
        tick: world.tick,
        vehicles,
    }
}

pub fn build_road_vehicle_delta_dto(
    world_id: &WorldId,
    world: &RoadVehicleWorld,
    delta: &RoadVehicleDelta,
) -> abutown_protocol::RoadVehicleDeltaDto {
    let changed = delta
        .changed
        .iter()
        .filter_map(|id| build_road_vehicle_dto(world, id))
        .collect();
    abutown_protocol::RoadVehicleDeltaDto {
        protocol_version: abutown_protocol::PROTOCOL_VERSION,
        world_id: world_id.clone(),
        tick: world.tick,
        changed,
    }
}

pub mod seed {
    use super::*;

    pub fn initial_road_vehicles() -> RoadVehicleWorld {
        let mut world = RoadVehicleWorld::default();
        // Four hardcoded corridors around the seeded chunks, each replicated with
        // road vehicles spread by offset for visual variety.
        let corridors: [Vec<TileCoord>; 4] = [
            // Horizontal across chunks (4,4) and (5,4) along their centers.
            vec![
                TileCoord { x: 4 * 32 + 4, y: 4 * 32 + 16 },
                TileCoord { x: 5 * 32 + 28, y: 4 * 32 + 16 },
            ],
            vec![
                TileCoord { x: 5 * 32 + 28, y: 4 * 32 + 16 },
                TileCoord { x: 4 * 32 + 4, y: 4 * 32 + 16 },
            ],
            // Vertical across chunks (4,4) and (4,5).
            vec![
                TileCoord { x: 4 * 32 + 16, y: 4 * 32 + 4 },
                TileCoord { x: 4 * 32 + 16, y: 5 * 32 + 28 },
            ],
            vec![
                TileCoord { x: 4 * 32 + 16, y: 5 * 32 + 28 },
                TileCoord { x: 4 * 32 + 16, y: 4 * 32 + 4 },
            ],
        ];

        // 80 vehicles spread across 4 corridors with deterministic offsets/speeds.
        for index in 0..80u32 {
            let corridor = corridors[(index as usize) % corridors.len()].clone();
            let id = RoadVehicleId(format!("road_vehicle:seed:{index}"));
            world.vehicles.insert(
                id.clone(),
                RoadVehicleRecord {
                    id,
                    offset: (index as f32) * 0.25,
                    speed: 0.05 + (index % 5) as f32 * 0.01,
                    sprite_key: format!("vehicle:{}", index % 8),
                    path: corridor,
                },
            );
        }
        world
    }
}
```

- [ ] **Step 5: Run tests to confirm pass**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core road_vehicles
```

Expected: all 5 tests pass.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/road_vehicles.rs backend/crates/sim-core/src/lib.rs
git commit -m "feat: road vehicle world and deterministic seeder"
```

---

## Task 5: RoadVehicleSnapshotStore Trait + InMemory

**Files:**
- Modify: `backend/crates/sim-core/src/persistence.rs`

- [ ] **Step 1: Write failing tests**

Append to the test module in `persistence.rs`:

```rust
#[tokio::test]
async fn road_vehicle_snapshot_store_writes_and_reads() {
    use crate::road_vehicles::seed;

    let mut store = InMemoryRoadVehicleSnapshotStore::default();
    let world = seed::initial_road_vehicles();

    RoadVehicleSnapshotStore::write(&mut store, "abutown-main", world.tick(), &world)
        .await
        .unwrap();

    let (tick, restored) = RoadVehicleSnapshotStore::read(&store, "abutown-main")
        .await
        .unwrap()
        .expect("snapshot exists");

    assert_eq!(tick, world.tick());
    assert_eq!(restored, world);
}

#[tokio::test]
async fn road_vehicle_snapshot_store_read_returns_none_for_unknown_world() {
    let store = InMemoryRoadVehicleSnapshotStore::default();
    assert!(RoadVehicleSnapshotStore::read(&store, "missing").await.unwrap().is_none());
}
```

- [ ] **Step 2: Run tests to confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core road_vehicle_snapshot_store
```

Expected: FAIL.

- [ ] **Step 3: Implement trait + in-memory store**

In `backend/crates/sim-core/src/persistence.rs`, add:

```rust
use crate::road_vehicles::RoadVehicleWorld;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{message}")]
pub struct RoadVehicleSnapshotStoreError {
    message: String,
}

impl RoadVehicleSnapshotStoreError {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
}

#[async_trait]
pub trait RoadVehicleSnapshotStore: std::fmt::Debug + Send {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &RoadVehicleWorld,
    ) -> Result<(), RoadVehicleSnapshotStoreError>;

    async fn read(
        &self,
        world_id: &str,
    ) -> Result<Option<(u64, RoadVehicleWorld)>, RoadVehicleSnapshotStoreError>;
}

#[derive(Debug, Default)]
pub struct InMemoryRoadVehicleSnapshotStore {
    snapshots: HashMap<String, (u64, RoadVehicleWorld)>,
}

#[async_trait]
impl RoadVehicleSnapshotStore for InMemoryRoadVehicleSnapshotStore {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &RoadVehicleWorld,
    ) -> Result<(), RoadVehicleSnapshotStoreError> {
        self.snapshots.insert(world_id.to_string(), (tick, snapshot.clone()));
        Ok(())
    }

    async fn read(
        &self,
        world_id: &str,
    ) -> Result<Option<(u64, RoadVehicleWorld)>, RoadVehicleSnapshotStoreError> {
        Ok(self.snapshots.get(world_id).cloned())
    }
}
```

- [ ] **Step 4: Run tests to confirm pass**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core
```

Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/persistence.rs
git commit -m "feat: in-memory road vehicle snapshot store"
```

---

## Task 6: Postgres Migration For Road Vehicle Snapshots

**Files:**
- Create: `backend/crates/sim-server/migrations/202605160003_road_vehicle_snapshots.sql`

- [ ] **Step 1: Write the migration**

```sql
CREATE TABLE IF NOT EXISTS road_vehicle_snapshots (
    world_id TEXT PRIMARY KEY,
    tick BIGINT NOT NULL CHECK (tick >= 0),
    payload JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

- [ ] **Step 2: Verify ordering**

```bash
ls backend/crates/sim-server/migrations/
```

Expected list includes the new file at the end alphabetically:

```
202605150001_world_events.sql
202605150002_card_hand_core.sql
202605150003_chunk_snapshots.sql
202605160001_chunk_recovery.sql
202605160002_mobility_snapshots.sql
202605160003_road_vehicle_snapshots.sql
```

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-server/migrations/202605160003_road_vehicle_snapshots.sql
git commit -m "feat: migrate road_vehicle_snapshots table"
```

---

## Task 7: PostgresRoadVehicleSnapshotStore

**Files:**
- Create: `backend/crates/sim-server/src/postgres_road_vehicles.rs`
- Modify: `backend/crates/sim-server/src/lib.rs`

- [ ] **Step 1: Add module export**

In `backend/crates/sim-server/src/lib.rs`, after the existing `pub mod postgres_mobility;` line, add:

```rust
pub mod postgres_road_vehicles;
```

- [ ] **Step 2: Implement adapter with opt-in integration test**

Create `backend/crates/sim-server/src/postgres_road_vehicles.rs`:

```rust
use async_trait::async_trait;
use serde_json::Value;
use sim_core::persistence::{RoadVehicleSnapshotStore, RoadVehicleSnapshotStoreError};
use sim_core::road_vehicles::RoadVehicleWorld;
use sqlx::{PgPool, postgres::PgPoolOptions};

const ROAD_VEHICLE_SNAPSHOTS_MIGRATION: &str =
    include_str!("../migrations/202605160003_road_vehicle_snapshots.sql");

#[derive(Debug)]
pub struct PostgresRoadVehicleSnapshotStore {
    pool: PgPool,
}

impl PostgresRoadVehicleSnapshotStore {
    pub async fn connect(database_url: &str) -> Result<Self, RoadVehicleSnapshotStoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|error| RoadVehicleSnapshotStoreError::unavailable(error.to_string()))?;

        for statement in ROAD_VEHICLE_SNAPSHOTS_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|error| RoadVehicleSnapshotStoreError::unavailable(error.to_string()))?;
        }

        Ok(Self { pool })
    }

    pub fn pool_for_test(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
impl RoadVehicleSnapshotStore for PostgresRoadVehicleSnapshotStore {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &RoadVehicleWorld,
    ) -> Result<(), RoadVehicleSnapshotStoreError> {
        let tick_i64 = i64::try_from(tick)
            .map_err(|_| RoadVehicleSnapshotStoreError::unavailable("tick exceeds i64"))?;
        let payload: Value = serde_json::to_value(snapshot)
            .map_err(|error| RoadVehicleSnapshotStoreError::unavailable(error.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO road_vehicle_snapshots (world_id, tick, payload)
            VALUES ($1, $2, $3)
            ON CONFLICT (world_id) DO UPDATE
              SET tick = EXCLUDED.tick,
                  payload = EXCLUDED.payload,
                  updated_at = now()
            "#,
        )
        .bind(world_id)
        .bind(tick_i64)
        .bind(payload)
        .execute(&self.pool)
        .await
        .map_err(|error| RoadVehicleSnapshotStoreError::unavailable(error.to_string()))?;

        Ok(())
    }

    async fn read(
        &self,
        world_id: &str,
    ) -> Result<Option<(u64, RoadVehicleWorld)>, RoadVehicleSnapshotStoreError> {
        let row: Option<(i64, Value)> = sqlx::query_as(
            "SELECT tick, payload FROM road_vehicle_snapshots WHERE world_id = $1",
        )
        .bind(world_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| RoadVehicleSnapshotStoreError::unavailable(error.to_string()))?;

        match row {
            None => Ok(None),
            Some((tick, payload)) => {
                let world: RoadVehicleWorld = serde_json::from_value(payload).map_err(|error| {
                    RoadVehicleSnapshotStoreError::unavailable(error.to_string())
                })?;
                let tick = u64::try_from(tick).map_err(|_| {
                    RoadVehicleSnapshotStoreError::unavailable("negative tick in row")
                })?;
                Ok(Some((tick, world)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn postgres_road_vehicle_round_trip_when_database_url_is_set() {
        use sim_core::road_vehicles::seed;

        let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
            eprintln!("skipping; ABUTOWN_TEST_DATABASE_URL not set");
            return;
        };

        let mut store = PostgresRoadVehicleSnapshotStore::connect(&database_url).await.unwrap();
        let world = seed::initial_road_vehicles();
        let world_id = format!("test:road_vehicle:{}", uuid::Uuid::now_v7());

        store.write(&world_id, world.tick(), &world).await.unwrap();
        let (tick, restored) = store.read(&world_id).await.unwrap().expect("present");
        assert_eq!(tick, world.tick());
        assert_eq!(restored, world);

        let _ = sqlx::query("DELETE FROM road_vehicle_snapshots WHERE world_id = $1")
            .bind(&world_id)
            .execute(store.pool_for_test())
            .await;
    }
}
```

- [ ] **Step 3: Verify build**

```bash
cargo build --locked --manifest-path backend/Cargo.toml -p sim-server
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server postgres_road_vehicle
```

Expected: builds; opt-in test silently returns when env unset.

- [ ] **Step 4: Commit**

```bash
git add backend/crates/sim-server/src/postgres_road_vehicles.rs backend/crates/sim-server/src/lib.rs
git commit -m "feat: postgres road vehicle snapshot adapter"
```

---

## Task 8: Runtime Wiring

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [ ] **Step 1: Extend imports**

In `runtime.rs`, locate the existing `use sim_core::persistence::{...}` block and add the new types:

```rust
use sim_core::persistence::{
    ChunkSnapshotStore, ChunkSnapshotStoreError, InMemoryChunkSnapshotStore,
    InMemoryMobilitySnapshotStore, MobilitySnapshotStore, MobilitySnapshotStoreError,
    InMemoryRoadVehicleSnapshotStore, RoadVehicleSnapshotStore, RoadVehicleSnapshotStoreError,
};
use sim_core::road_vehicles::{self, RoadVehicleWorld, build_road_vehicle_delta_dto};
```

- [ ] **Step 2: Add fields to SimulationRuntime**

After `mobility_snapshot_store`, add:

```rust
road_vehicle_world: RoadVehicleWorld,
road_vehicle_snapshot_store: Box<dyn RoadVehicleSnapshotStore + Send>,
```

- [ ] **Step 3: Update constructors**

In `new_with_stores`, initialize both new fields with in-memory defaults:

```rust
road_vehicle_world: road_vehicles::seed::initial_road_vehicles(),
road_vehicle_snapshot_store: Box::new(InMemoryRoadVehicleSnapshotStore::default()),
```

Add a new constructor for full-control tests:

```rust
pub fn new_with_full_stores(
    event_store: Box<dyn WorldEventStore + Send>,
    snapshot_store: Box<dyn ChunkSnapshotStore + Send>,
    mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send>,
    road_vehicle_snapshot_store: Box<dyn RoadVehicleSnapshotStore + Send>,
) -> Self {
    let mut runtime = Self::new_with_all_stores(
        event_store,
        snapshot_store,
        mobility_snapshot_store,
    );
    runtime.road_vehicle_snapshot_store = road_vehicle_snapshot_store;
    runtime
}
```

- [ ] **Step 4: Couple road-vehicle tick to runtime tick**

Locate `next_server_messages` (the function called every `SIMULATION_TICK_INTERVAL`). After the existing mobility tick + `MobilityDelta` push, append:

```rust
let road_delta = self.road_vehicle_world.tick_road_vehicles();
messages.push(abutown_protocol::ServerMessageDto::RoadVehicleDelta(
    build_road_vehicle_delta_dto(&self.world_id, &self.road_vehicle_world, &road_delta),
));
```

(Exact local variable names depend on the current code; preserve the existing pattern.)

- [ ] **Step 5: Add persist method**

Alongside `persist_mobility_snapshot`:

```rust
pub async fn persist_road_vehicle_snapshot(
    &mut self,
) -> Result<(), RoadVehicleSnapshotStoreError> {
    self.road_vehicle_snapshot_store
        .write(&self.world_id.0, self.road_vehicle_world.tick(), &self.road_vehicle_world)
        .await
}
```

- [ ] **Step 6: Add snapshot getter for HTTP**

```rust
pub fn road_vehicle_snapshot_dto(&self) -> abutown_protocol::RoadVehicleSnapshotDto {
    road_vehicles::build_road_vehicle_snapshot_dto(&self.world_id, &self.road_vehicle_world)
}
```

- [ ] **Step 7: Write a runtime-level unit test**

Append to the runtime test module:

```rust
#[tokio::test]
async fn runtime_ticks_road_vehicles_and_persists_snapshot() {
    use sim_core::persistence::InMemoryRoadVehicleSnapshotStore;

    let mut runtime = SimulationRuntime::new_with_full_stores(
        Box::new(InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(InMemoryRoadVehicleSnapshotStore::default()),
    );

    let initial_tick = runtime.road_vehicle_world.tick();
    runtime.next_server_messages();
    assert_eq!(runtime.road_vehicle_world.tick(), initial_tick + 1);

    runtime.persist_road_vehicle_snapshot().await.unwrap();
    let stored = runtime
        .road_vehicle_snapshot_store
        .read(&runtime.world_id.0)
        .await
        .unwrap()
        .expect("persisted snapshot");
    assert_eq!(stored.0, runtime.road_vehicle_world.tick());
}
```

- [ ] **Step 8: Verify**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server runtime_ticks_road_vehicles
cargo test --locked --manifest-path backend/Cargo.toml --workspace
```

Expected: all green.

- [ ] **Step 9: Commit**

```bash
git add backend/crates/sim-server/src/runtime.rs
git commit -m "feat: simulation runtime owns road vehicle world"
```

---

## Task 9: Hydrate Road Vehicles On Startup

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs`
- Modify: `backend/crates/sim-server/src/app.rs`
- Modify: `backend/crates/sim-server/tests/http.rs` (only call sites)

- [ ] **Step 1: Extend `HydrationError`**

Add a new variant:

```rust
#[derive(Debug, thiserror::Error)]
pub enum HydrationError {
    // ... existing variants ...
    #[error("road vehicle store error: {0}")]
    RoadVehicle(sim_core::persistence::RoadVehicleSnapshotStoreError),
}
```

- [ ] **Step 2: Write a failing hydration test**

Append to the runtime test module:

```rust
#[tokio::test]
async fn hydrate_restores_road_vehicles_from_store_when_present() {
    use sim_core::persistence::{InMemoryRoadVehicleSnapshotStore, RoadVehicleSnapshotStore};
    use sim_core::road_vehicles::seed;

    let mut store = InMemoryRoadVehicleSnapshotStore::default();
    let mut authored = seed::initial_road_vehicles();
    authored.tick_road_vehicles();
    let persisted_tick = authored.tick();
    RoadVehicleSnapshotStore::write(&mut store, "abutown-main", persisted_tick, &authored)
        .await
        .unwrap();

    let runtime = SimulationRuntime::hydrate_from_stores(
        Box::new(InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(store),
    )
    .await
    .unwrap();

    assert_eq!(runtime.road_vehicle_world.tick(), persisted_tick);
    assert_eq!(runtime.road_vehicle_world, authored);
}

#[tokio::test]
async fn hydrate_seeds_road_vehicles_when_store_is_empty() {
    use sim_core::persistence::InMemoryRoadVehicleSnapshotStore;

    let runtime = SimulationRuntime::hydrate_from_stores(
        Box::new(InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(InMemoryRoadVehicleSnapshotStore::default()),
    )
    .await
    .unwrap();

    assert!(runtime.road_vehicle_world.vehicles.len() >= 80);
}
```

- [ ] **Step 3: Run to confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server hydrate_restores_road_vehicles hydrate_seeds_road_vehicles
```

Expected: FAIL — `hydrate_from_stores` is currently 3-arg.

- [ ] **Step 4: Extend `hydrate_from_stores` signature**

Change to:

```rust
pub async fn hydrate_from_stores(
    event_store: Box<dyn WorldEventStore + Send>,
    snapshot_store: Box<dyn ChunkSnapshotStore + Send>,
    mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send>,
    road_vehicle_snapshot_store: Box<dyn RoadVehicleSnapshotStore + Send>,
) -> Result<Self, HydrationError>
```

Read the road-vehicle store near the start (next to the mobility read):

```rust
let road_vehicle_world = match road_vehicle_snapshot_store
    .read(&world_id.0)
    .await
    .map_err(HydrationError::RoadVehicle)?
{
    Some((_tick, world)) => world,
    None => sim_core::road_vehicles::seed::initial_road_vehicles(),
};
```

In the final `Self { ... }` block, replace the default `road_vehicle_world: ... initial_road_vehicles()` and `road_vehicle_snapshot_store: Box::new(InMemoryRoadVehicleSnapshotStore::default())` with `road_vehicle_world,` and `road_vehicle_snapshot_store,` from the locals.

- [ ] **Step 5: Update production caller (app.rs)**

In `build_app_from_config`, after the existing mobility store connect, add:

```rust
let road_vehicle_snapshot_store =
    PostgresRoadVehicleSnapshotStore::connect(&config.database_url).await?;
```

Update the `hydrate_from_stores` call:

```rust
let runtime = SimulationRuntime::hydrate_from_stores(
    Box::new(event_store),
    Box::new(snapshot_store),
    Box::new(mobility_snapshot_store),
    Box::new(road_vehicle_snapshot_store),
)
.await?;
```

Import: add `use crate::postgres_road_vehicles::PostgresRoadVehicleSnapshotStore;`.

- [ ] **Step 6: Update all other `hydrate_from_stores` call sites**

Run:

```bash
grep -rn "hydrate_from_stores" backend/
```

Update every call site to pass a 4th `Box::new(InMemoryRoadVehicleSnapshotStore::default())` argument (test code) or the postgres variant (production). Existing tests in `runtime.rs` and `tests/http.rs` will fail to compile otherwise — fix them all in this commit.

- [ ] **Step 7: Verify**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server hydrate
cargo test --locked --manifest-path backend/Cargo.toml --workspace
```

Expected: all green.

- [ ] **Step 8: Commit**

```bash
git add backend/crates/sim-server/src/runtime.rs backend/crates/sim-server/src/app.rs backend/crates/sim-server/tests/http.rs
git commit -m "feat: hydrate road vehicle world on runtime startup"
```

---

## Task 10: App HTTP Endpoint + Snapshot Loop

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs`

- [ ] **Step 1: Add the `/road-vehicles` route**

In the function that constructs the `Router` (look for the existing `.route("/mobility", ...)` line), add an analogous line:

```rust
.route("/road-vehicles", get(road_vehicles_handler))
```

Add the handler near the existing `/mobility` one:

```rust
async fn road_vehicles_handler(
    State(state): State<AppState>,
) -> Json<abutown_protocol::RoadVehicleSnapshotDto> {
    let runtime = state.runtime();
    let runtime = runtime.lock().await;
    Json(runtime.road_vehicle_snapshot_dto())
}
```

- [ ] **Step 2: Update the snapshot loop**

Locate `persist_snapshots_once`. Add a third persist call after the existing mobility one:

```rust
if let Err(error) = guard.persist_road_vehicle_snapshot().await {
    tracing::warn!(%error, "failed to persist road vehicle snapshot");
}
```

Place this after the existing `persist_mobility_snapshot` call, before the `Ok(written)`.

- [ ] **Step 3: Write a failing http test**

In `backend/crates/sim-server/tests/http.rs`, add:

```rust
#[tokio::test]
async fn road_vehicles_endpoint_returns_seeded_snapshot() {
    let app = build_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/road-vehicles")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["protocol_version"], 1);
    assert_eq!(json["world_id"], "abutown-main");
    let vehicles = json["vehicles"].as_array().expect("vehicles array");
    assert!(vehicles.len() >= 80, "seed must populate at least 80 vehicles");
    assert!(vehicles[0]["sprite_key"].is_string());
    assert!(vehicles[0]["direction"].is_string());
    assert!(vehicles[0]["world_coord"]["x"].is_number());
}
```

- [ ] **Step 4: Run tests to confirm pass**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server road_vehicles_endpoint
```

Expected: PASS.

- [ ] **Step 5: Workspace test + build**

```bash
cargo test --locked --manifest-path backend/Cargo.toml --workspace
```

Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-server/src/app.rs backend/crates/sim-server/tests/http.rs
git commit -m "feat: serve /road-vehicles and persist road vehicle snapshots"
```

---

## Task 11: Frontend Protocol + State Modules For Road Vehicles

**Files:**
- Create: `src/backend/roadVehicleProtocol.ts`
- Create: `src/backend/roadVehicleState.ts`
- Create: `tests/backend/roadVehicleState.test.ts`
- Modify: `src/backend/mobilityProtocol.ts`

- [ ] **Step 1: Extend mobilityProtocol.ts with new fields**

In `src/backend/mobilityProtocol.ts`, update `AgentMobilityDto`:

```ts
export type DirectionDto = 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w' | 'nw';

export type WorldCoordDto = { x: number; y: number };

export type AgentMobilityDto = {
  id: string;
  state: AgentMobilityStateDto;
  plan_cursor: number;
  world_coord: WorldCoordDto;
  direction: DirectionDto;
  sprite_key: string;
};

export type VehicleMobilityDto = {
  id: string;
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

Update `isMobilitySnapshotDto` and `isMobilityDeltaDto` guards to validate the three new fields on each entry (`world_coord.x` finite number, `direction` string member of the enum, `sprite_key` non-empty string).

- [ ] **Step 2: Create roadVehicleProtocol.ts**

```ts
import { isFiniteNumber, isString, type DirectionDto, type WorldCoordDto } from './mobilityProtocol';

export type RoadVehicleDto = {
  id: string;
  world_coord: WorldCoordDto;
  direction: DirectionDto;
  sprite_key: string;
};

export type RoadVehicleSnapshotDto = {
  protocol_version: number;
  world_id: string;
  tick: number;
  vehicles: RoadVehicleDto[];
};

export type RoadVehicleDeltaDto = {
  protocol_version: number;
  world_id: string;
  tick: number;
  changed: RoadVehicleDto[];
};

const DIRECTIONS: ReadonlySet<DirectionDto> = new Set(['n','ne','e','se','s','sw','w','nw']);

function isDirection(value: unknown): value is DirectionDto {
  return typeof value === 'string' && DIRECTIONS.has(value as DirectionDto);
}

function isWorldCoord(value: unknown): value is WorldCoordDto {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return isFiniteNumber(v.x) && isFiniteNumber(v.y);
}

function isRoadVehicleDto(value: unknown): value is RoadVehicleDto {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return isString(v.id) && isWorldCoord(v.world_coord) && isDirection(v.direction) && isString(v.sprite_key);
}

export function isRoadVehicleSnapshotDto(value: unknown): value is RoadVehicleSnapshotDto {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.protocol_version === 'number' &&
    isString(v.world_id) &&
    isFiniteNumber(v.tick) &&
    Array.isArray(v.vehicles) &&
    v.vehicles.every(isRoadVehicleDto)
  );
}

export function isRoadVehicleDeltaDto(value: unknown): value is RoadVehicleDeltaDto {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.protocol_version === 'number' &&
    isString(v.world_id) &&
    isFiniteNumber(v.tick) &&
    Array.isArray(v.changed) &&
    v.changed.every(isRoadVehicleDto)
  );
}
```

Note: `isFiniteNumber` and `isString` are existing helpers in `mobilityProtocol.ts`. If they're not exported, export them.

- [ ] **Step 3: Create roadVehicleState.ts**

```ts
import {
  isRoadVehicleDeltaDto,
  isRoadVehicleSnapshotDto,
  type RoadVehicleDeltaDto,
  type RoadVehicleDto,
  type RoadVehicleSnapshotDto,
} from './roadVehicleProtocol';

export type RoadVehicleOverlayState = {
  tick: number;
  vehicles: Map<string, RoadVehicleDto>;
  invalidMessages: number;
  lastUpdatedAt: number;
};

export function createRoadVehicleOverlayState(): RoadVehicleOverlayState {
  return { tick: 0, vehicles: new Map(), invalidMessages: 0, lastUpdatedAt: 0 };
}

export function applyRoadVehicleSnapshot(
  state: RoadVehicleOverlayState,
  snapshot: RoadVehicleSnapshotDto,
  now = Date.now(),
): RoadVehicleOverlayState {
  return {
    ...state,
    tick: snapshot.tick,
    vehicles: new Map(snapshot.vehicles.map((vehicle) => [vehicle.id, vehicle])),
    lastUpdatedAt: now,
  };
}

export function applyRoadVehicleDelta(
  state: RoadVehicleOverlayState,
  delta: RoadVehicleDeltaDto,
  now = Date.now(),
): RoadVehicleOverlayState {
  const vehicles = new Map(state.vehicles);
  for (const vehicle of delta.changed) {
    vehicles.set(vehicle.id, vehicle);
  }
  return { ...state, tick: delta.tick, vehicles, lastUpdatedAt: now };
}

export function applyRoadVehicleMessage(
  state: RoadVehicleOverlayState,
  value: unknown,
  now = Date.now(),
): RoadVehicleOverlayState {
  if (isRoadVehicleDeltaDto(value)) {
    return applyRoadVehicleDelta(state, value, now);
  }
  if (isRoadVehicleSnapshotDto(value)) {
    return applyRoadVehicleSnapshot(state, value, now);
  }
  return { ...state, invalidMessages: state.invalidMessages + 1 };
}
```

- [ ] **Step 4: Write a vitest unit test**

Create `tests/backend/roadVehicleState.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import {
  applyRoadVehicleDelta,
  applyRoadVehicleSnapshot,
  createRoadVehicleOverlayState,
} from '../../src/backend/roadVehicleState';

const snapshot = {
  protocol_version: 1,
  world_id: 'abutown-main',
  tick: 3,
  vehicles: [{ id: 'road_vehicle:seed:0', world_coord: { x: 1.0, y: 2.0 }, direction: 'e' as const, sprite_key: 'vehicle:0' }],
};

describe('road vehicle state', () => {
  it('applies snapshot then delta', () => {
    const initial = createRoadVehicleOverlayState();
    const afterSnap = applyRoadVehicleSnapshot(initial, snapshot);
    expect(afterSnap.tick).toBe(3);
    expect(afterSnap.vehicles.get('road_vehicle:seed:0')?.direction).toBe('e');

    const afterDelta = applyRoadVehicleDelta(afterSnap, {
      protocol_version: 1,
      world_id: 'abutown-main',
      tick: 4,
      changed: [{ id: 'road_vehicle:seed:0', world_coord: { x: 5.0, y: 2.0 }, direction: 'n', sprite_key: 'vehicle:0' }],
    });
    expect(afterDelta.tick).toBe(4);
    expect(afterDelta.vehicles.get('road_vehicle:seed:0')?.world_coord.x).toBe(5.0);
  });
});
```

- [ ] **Step 5: Run tests**

```bash
npx vitest run tests/backend/roadVehicleState.test.ts
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/backend/roadVehicleProtocol.ts src/backend/roadVehicleState.ts src/backend/mobilityProtocol.ts tests/backend/roadVehicleState.test.ts
git commit -m "feat: frontend road vehicle protocol and state"
```

---

## Task 12: Frontend mobilityClient Extension

**Files:**
- Modify: `src/backend/mobilityClient.ts`
- Modify: `src/backend/mobilityState.ts`

- [ ] **Step 1: Embed road-vehicle state inside mobility state**

In `src/backend/mobilityState.ts`, extend `MobilityOverlayState`:

```ts
import { type RoadVehicleOverlayState, createRoadVehicleOverlayState } from './roadVehicleState';

export type MobilityOverlayState = {
  status: MobilityConnectionStatus;
  tick: number;
  agents: Map<string, AgentMobilityDto>;
  vehicles: Map<string, VehicleMobilityDto>;
  stops: Map<string, StopMobilityDto>;
  roadVehicles: RoadVehicleOverlayState;
  invalidMessages: number;
  lastError: string | null;
  lastUpdatedAt: number;
};
```

Update `createMobilityOverlayState()` to initialize `roadVehicles: createRoadVehicleOverlayState()`.

Extend `applyServerMessage` to dispatch road-vehicle messages:

```ts
import { applyRoadVehicleMessage } from './roadVehicleState';

// inside applyServerMessage, after the existing mobility_delta branch:
if (message?.type === 'road_vehicle_delta') {
  return { ...state, roadVehicles: applyRoadVehicleMessage(state.roadVehicles, message, now) };
}
```

Extend `mobilityDiagnostics` so the overlay status report includes `roadVehicles` count:

```ts
export type MobilityDiagnostics = {
  // ... existing fields ...
  roadVehicles: number;
};

// inside mobilityDiagnostics:
return {
  // existing fields,
  roadVehicles: state.roadVehicles.vehicles.size,
};
```

- [ ] **Step 2: Update mobilityClient.ts to fetch /road-vehicles**

Locate the existing `requireMobilitySnapshot` (or equivalent) that fetches `/mobility`. Add an analogous fetch of `/road-vehicles` and apply the result via `applyRoadVehicleSnapshot`. Both fetches must succeed for boot; either failure throws.

```ts
import { isRoadVehicleSnapshotDto } from './roadVehicleProtocol';
import { applyRoadVehicleSnapshot } from './roadVehicleState';

export async function requireMobilitySnapshot(options: { baseUrl: string }): Promise<MobilityOverlayState> {
  const mobilityRes = await fetch(`${options.baseUrl}/mobility`);
  if (!mobilityRes.ok) throw new Error(`mobility ${mobilityRes.status}`);
  const mobilityJson = await mobilityRes.json();
  if (!isMobilitySnapshotDto(mobilityJson)) throw new Error('invalid mobility snapshot');

  const roadRes = await fetch(`${options.baseUrl}/road-vehicles`);
  if (!roadRes.ok) throw new Error(`road-vehicles ${roadRes.status}`);
  const roadJson = await roadRes.json();
  if (!isRoadVehicleSnapshotDto(roadJson)) throw new Error('invalid road vehicle snapshot');

  let state = createMobilityOverlayState();
  state = applyMobilitySnapshot(state, mobilityJson);
  state = { ...state, roadVehicles: applyRoadVehicleSnapshot(state.roadVehicles, roadJson) };
  return state;
}
```

(Adapt to the actual function shape — function name, error type, and signature may differ slightly; preserve existing behavior for the mobility path and add road-vehicle alongside.)

- [ ] **Step 3: Test**

```bash
npx vitest run tests/backend/mobilityState.test.ts
npx vitest run tests/backend/mobilityClient.test.ts  # if it exists
```

Expected: existing tests pass; if any test relied on the absent road-vehicle state, update it to include `roadVehicles: createRoadVehicleOverlayState()`.

- [ ] **Step 4: Commit**

```bash
git add src/backend/mobilityClient.ts src/backend/mobilityState.ts tests/
git commit -m "feat: mobility client fetches and merges road vehicle state"
```

---

## Task 13: Backend Mobility Drawables Module

**Files:**
- Create: `src/render/backendMobilityDrawables.ts`
- Create: `tests/render/backendMobilityDrawables.test.ts`

- [ ] **Step 1: Write failing tests**

Create `tests/render/backendMobilityDrawables.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { pedestriansFromMobilityState, carsFromMobilityState } from '../../src/render/backendMobilityDrawables';

const pedestrianSprites = [
  { sheet: 'pak128/peds.0', frameWidth: 16, frameHeight: 32 },
  { sheet: 'pak128/peds.1', frameWidth: 16, frameHeight: 32 },
];
const vehicleSprites = [
  { sheet: 'pak128/cars.0', frameWidth: 32, frameHeight: 32, scale: 1, role: 'vehicle.0' },
  { sheet: 'pak128/cars.1', frameWidth: 32, frameHeight: 32, scale: 1, role: 'vehicle.1' },
];

const mobilityState = {
  status: 'connected' as const,
  tick: 1,
  agents: new Map([
    ['agent:seed:0', {
      id: 'agent:seed:0',
      state: { type: 'walking' as const, link_id: 'link:walk:default', progress: 0.5 },
      plan_cursor: 0,
      world_coord: { x: 10.5, y: 20.0 },
      direction: 'e' as const,
      sprite_key: 'pedestrian:0',
    }],
  ]),
  vehicles: new Map(),
  stops: new Map(),
  roadVehicles: {
    tick: 1,
    vehicles: new Map([
      ['road_vehicle:seed:0', {
        id: 'road_vehicle:seed:0',
        world_coord: { x: 32.0, y: 32.0 },
        direction: 'n' as const,
        sprite_key: 'vehicle:0',
      }],
    ]),
    invalidMessages: 0,
    lastUpdatedAt: 0,
  },
  invalidMessages: 0,
  lastError: null,
  lastUpdatedAt: 0,
};

describe('backendMobilityDrawables', () => {
  it('projects agents into pedestrians with backend world_coord', () => {
    const pedestrians = pedestriansFromMobilityState(mobilityState, pedestrianSprites);
    expect(pedestrians).toHaveLength(1);
    expect(pedestrians[0].path[0]).toEqual({ x: 10.5, y: 20.0 });
    expect(pedestrians[0].sprite.sheet).toBe('pak128/peds.0');
    expect(pedestrians[0].id).toBe('agent:seed:0');
  });

  it('projects road vehicles into cars with backend world_coord', () => {
    const cars = carsFromMobilityState(mobilityState, vehicleSprites);
    expect(cars).toHaveLength(1);
    expect(cars[0].path[0]).toEqual({ x: 32.0, y: 32.0 });
    expect(cars[0].sprite.role).toBe('vehicle.0');
    expect(cars[0].id).toBe('road_vehicle:seed:0');
  });
});
```

- [ ] **Step 2: Run to confirm failure**

```bash
npx vitest run tests/render/backendMobilityDrawables.test.ts
```

Expected: FAIL — module not implemented.

- [ ] **Step 3: Implement the projector**

Create `src/render/backendMobilityDrawables.ts`:

```ts
import type { MobilityOverlayState } from '../backend/mobilityState';
import type { AgentMobilityDto, DirectionDto, VehicleMobilityDto } from '../backend/mobilityProtocol';
import type { RoadVehicleDto } from '../backend/roadVehicleProtocol';

export type Coord = { x: number; y: number };

export type SimutransPedestrianSprite = { sheet: string; frameWidth: number; frameHeight: number };
export type VehicleSprite = { sheet: string; frameWidth: number; frameHeight: number; scale: number; role: string };

export type BackendPedestrian = {
  id: string;
  path: Coord[];
  offset: number;
  speed: number;
  laneOffset: number;
  sprite: SimutransPedestrianSprite;
  direction: DirectionDto;
};

export type BackendCar = {
  id: string;
  path: Coord[];
  offset: number;
  speed: number;
  sprite: VehicleSprite;
  direction: DirectionDto;
};

const DIRECTION_VECTORS: Record<DirectionDto, Coord> = {
  n: { x: 0, y: -1 },
  ne: { x: 1, y: -1 },
  e: { x: 1, y: 0 },
  se: { x: 1, y: 1 },
  s: { x: 0, y: 1 },
  sw: { x: -1, y: 1 },
  w: { x: -1, y: 0 },
  nw: { x: -1, y: -1 },
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
  sprites: SimutransPedestrianSprite[],
): BackendPedestrian[] {
  if (sprites.length === 0) return [];
  const out: BackendPedestrian[] = [];
  for (const agent of state.agents.values()) {
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
  sprites: VehicleSprite[],
): BackendCar[] {
  if (sprites.length === 0) return [];
  const out: BackendCar[] = [];
  for (const vehicle of state.roadVehicles.vehicles.values()) {
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

`syntheticPath` returns a two-element path so the existing `drawPedestrian`/`drawCar` functions (which read `path[0]` as current and `path[1]` as next for direction inference) work without code change. Phase 2 (frame interpolation) will replace the synthetic path with an actually-interpolated `[prev_coord, current_coord]` pair.

- [ ] **Step 4: Run tests to confirm pass**

```bash
npx vitest run tests/render/backendMobilityDrawables.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/render/backendMobilityDrawables.ts tests/render/backendMobilityDrawables.test.ts
git commit -m "feat: project backend mobility state into pedestrian and car drawables"
```

---

## Task 14: main.ts Switch To Backend-Driven Rendering

**Files:**
- Modify: `src/main.ts`
- Delete: `src/render/pedestrianAgents.ts`
- Delete: `src/render/pedestrianAgentInspector.ts`
- Delete: `src/render/localRoadVehicles.ts`
- Delete: `src/render/roadVehicleInspector.ts`
- Delete: corresponding tests under `tests/render/`

This is the largest single task in the plan. Approach it surgically.

- [ ] **Step 1: Inventory the local-mobility wiring in main.ts**

Run:

```bash
grep -n "buildCars\|buildPedestrians\|cars:\|pedestrians:\|localPedestrianAgents\|localRoadVehicles\|pedestrianAgentId\|localRoadVehicleId\|selectedAgentId\|selectedVehicleId" src/main.ts
```

You should see the local builders, the local-agent projection helpers, the per-frame `cars`/`pedestrians` arrays, the selection state variables, and the diagnostics block. Each of these must change.

- [ ] **Step 2: Replace local entity builders with backend projector**

Locate where `cars = buildCars(...)` and `pedestrians = buildPedestrians(...)` are called. Remove those calls and the surrounding declarations. Remove the imports of `buildPedestrians`/`buildCars` and types `Car`/`Pedestrian` if they live in main.ts (they do — they were inline). Keep the `Train` type and `buildTrains` flow unchanged.

Where the drawable list is composed each frame, replace the existing `cars.map(...)` / `pedestrians.map(...)` blocks with:

```ts
const pedestrians = pedestriansFromMobilityState(mobilityState, pedestrianSprites);
const cars = carsFromMobilityState(mobilityState, vehicleSprites);
```

Adjust the drawable mapper to read `item.car.path` / `item.pedestrian.path` as before — the projector outputs the same shape.

- [ ] **Step 3: Update selection**

Replace `selectedAgentId` and `selectedVehicleId` lookups:

```ts
function selectedPedestrianAgent(): BackendPedestrian | null {
  return pedestriansFromMobilityState(mobilityState, pedestrianSprites)
    .find((agent) => agent.id === selectedAgentId) ?? null;
}

function selectedVehicle(): BackendCar | null {
  return carsFromMobilityState(mobilityState, vehicleSprites)
    .find((vehicle) => vehicle.id === selectedVehicleId) ?? null;
}
```

Hit-test functions: re-use the existing pixel-distance scan but iterate the projected lists. Drop `findNearestPedestrianAgent` / `findNearestLocalRoadVehicle` imports — replace with inline `Math.hypot` loops that compare projected screen coords to the click point.

- [ ] **Step 4: Update diagnostics in render_game_to_text**

Replace the `mobility` block to source counts from `state.agents.size` and `state.roadVehicles.vehicles.size`. Set `mobility.source = 'backend'`. Remove `localAgents`/`localVehicles` sub-blocks — they no longer exist as a separate concept.

- [ ] **Step 5: Delete the now-unused frontend modules**

```bash
git rm src/render/pedestrianAgents.ts src/render/pedestrianAgentInspector.ts src/render/localRoadVehicles.ts src/render/roadVehicleInspector.ts
git rm tests/render/pedestrianAgents.test.ts tests/render/pedestrianAgentInspector.test.ts tests/render/localRoadVehicles.test.ts tests/render/roadVehicleInspector.test.ts
```

If the inspectors are still needed (selected-entity sidebar), recreate them as simple formatters that take a `BackendPedestrian` / `BackendCar` and return the same rows the e2e test inspects. Do not over-engineer — the previous modules were short.

- [ ] **Step 6: Verify build + tests**

```bash
npx vitest run
npm run build
```

Expected: green. Vitest may surface tests in `tests/render/*` that referenced the deleted modules — delete or rewrite those tests so they target the new `backendMobilityDrawables` projector.

- [ ] **Step 7: Commit**

```bash
git add src/main.ts src/render/ tests/render/
git commit -m "feat: render mobility from backend state instead of local builders"
```

---

## Task 15: E2E Smoke Test Asserts Backend-Sourced Mobility

**Files:**
- Modify: `tests/e2e/render-smoke.spec.ts`

- [ ] **Step 1: Update assertions**

Inside `render-smoke.spec.ts`, find the block that reads `render_game_to_text()` output and asserts `mobility.source === 'local-pedestrians'` or similar. Replace with:

```ts
expect(json.city.mobility.source).toBe('backend');
expect(json.city.mobility.agents).toBeGreaterThanOrEqual(20);
expect(json.city.mobility.roadVehicles).toBeGreaterThanOrEqual(80);
```

Add an assertion that the canvas actually drew at least one car and one pedestrian — easiest is to check non-zero counts in the diagnostics returned from `render_game_to_text()` for `cars` and `pedestrians`:

```ts
expect(json.city.cars).toBeGreaterThan(0);
expect(json.city.pedestrians).toBeGreaterThan(0);
```

- [ ] **Step 2: Run E2E**

The e2e harness expects both backend and frontend running. Run:

```bash
npm run test:e2e -- render-smoke
```

Expected: PASS. If the test discovers a mismatch (e.g., `localAgents` field is still expected), update the test to reflect the new diagnostics shape.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/render-smoke.spec.ts
git commit -m "test: render smoke asserts backend-sourced mobility"
```

---

## Task 16: Postgres Recovery Integration Test For Road Vehicles

**Files:**
- Modify: `backend/crates/sim-server/tests/http.rs`

- [ ] **Step 1: Write the integration test**

Append:

```rust
#[tokio::test]
async fn postgres_road_vehicle_state_survives_runtime_restart() {
    use sim_core::events::InMemoryWorldEventStore;
    use sim_core::persistence::{InMemoryChunkSnapshotStore, InMemoryMobilitySnapshotStore, RoadVehicleSnapshotStore};
    use sim_core::road_vehicles::seed;
    use sim_server::postgres_road_vehicles::PostgresRoadVehicleSnapshotStore;
    use sim_server::runtime::SimulationRuntime;

    let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
        eprintln!(
            "skipping postgres_road_vehicle_state_survives_runtime_restart; \
             ABUTOWN_TEST_DATABASE_URL not set"
        );
        return;
    };

    let world_id = format!("test:road_vehicle:{}", uuid::Uuid::now_v7());

    let persisted_tick;
    let persisted_world;
    {
        let road_store = PostgresRoadVehicleSnapshotStore::connect(&database_url)
            .await
            .expect("connect road vehicle store");
        let mut runtime = SimulationRuntime::new_with_full_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            Box::new(road_store),
        );
        runtime.override_world_id_for_test(&world_id);
        runtime.set_road_vehicle_world_for_test(seed::initial_road_vehicles());

        for _ in 0..3 {
            runtime.next_server_messages();
        }
        persisted_tick = runtime.road_vehicle_world.tick();
        persisted_world = runtime.road_vehicle_world.clone();
        runtime.persist_road_vehicle_snapshot().await.expect("persist");
    }

    let store = PostgresRoadVehicleSnapshotStore::connect(&database_url)
        .await
        .expect("reconnect");
    let (tick, restored) = RoadVehicleSnapshotStore::read(&store, &world_id)
        .await
        .expect("read")
        .expect("snapshot present");
    assert_eq!(tick, persisted_tick);
    assert_eq!(restored, persisted_world);

    let _ = sqlx::query("DELETE FROM road_vehicle_snapshots WHERE world_id = $1")
        .bind(&world_id)
        .execute(store.pool_for_test())
        .await;
}
```

- [ ] **Step 2: Add test helpers on `SimulationRuntime`**

In `backend/crates/sim-server/src/runtime.rs`, add to `impl SimulationRuntime`:

```rust
pub fn set_road_vehicle_world_for_test(&mut self, world: RoadVehicleWorld) {
    self.road_vehicle_world = world;
}
```

`override_world_id_for_test` already exists from the mobility-population work.

- [ ] **Step 3: Verify**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server
```

Expected: green; the new test silently skips when env var unset.

- [ ] **Step 4: Commit**

```bash
git add backend/crates/sim-server/tests/http.rs backend/crates/sim-server/src/runtime.rs
git commit -m "test: cover postgres road vehicle recovery end-to-end"
```

---

## Task 17: Final Quality Gate

**Files:** `progress.md` (only).

- [ ] **Step 1: Format check + workspace tests + clippy**

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
cargo test --locked --manifest-path backend/Cargo.toml --workspace
cargo clippy --locked --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```

Expected: all three succeed. If fmt diverges, run `cargo fmt --manifest-path backend/Cargo.toml --all` and stage the changes.

- [ ] **Step 2: Frontend tests + build**

```bash
npx vitest run
npm run build
```

Expected: green.

- [ ] **Step 3: Append progress note**

Append to `progress.md`:

```
2026-05-16T<HH:MM:SS>.000Z - Visible backend mobility: added road-vehicle subsystem (RoadVehicleWorld + Postgres snapshot table), extended Agent/Vehicle/RoadVehicle DTOs with world_coord+direction+sprite_key (server-computed each tick), replaced frontend buildPedestrians/buildCars with backendMobilityDrawables projector. Canvas now renders ~200 pedestrians + ~80 road vehicles + 4 transit vehicles from server-authoritative state.
```

- [ ] **Step 4: Commit**

```bash
git add progress.md backend/
git commit -m "chore: format and record visible backend mobility progress"
```

---

## Self-Review

- **Spec coverage:**
  - Phase 1 spec § "Backend gains a road-vehicle subsystem" → Tasks 4, 5, 6, 7, 8.
  - Phase 1 spec § "Mobility DTOs carry world coordinates and direction hints" → Tasks 1, 2, 3.
  - Phase 1 spec § "Frontend canvas treats backend mobility as the only visual source" → Tasks 11, 12, 13, 14.
  - Phase 1 spec § "Persistence" → Tasks 5, 6, 7, 16.
  - Phase 1 spec § "Error handling" → Task 9 (hydrate fallback) + Task 10 (logged-and-continued snapshot loop).
  - Phase 1 spec § "Testing strategy" → unit tests in Tasks 1–13, integration in Task 16, e2e in Task 15.
  - Phase 1 spec § "Risks" — `direction` jitter mitigated in Task 3 via 0.1-progress lookahead; `sprite_key` collision fallback in Task 13 (`spriteIndexFromKey` always returns a valid index).
- **Placeholder scan:** Every task has concrete code; no TBDs. One step ("Step 7: Update production caller in app.rs" in Task 9) tells the engineer to grep for callers — that's a routine search-and-replace, not a placeholder.
- **Type consistency:** `RoadVehicleWorld`/`RoadVehicleRecord`/`RoadVehicleId`/`TileCoord`/`DirectionDto`/`WorldCoordDto` used consistently. `world_coord_for_agent`/`world_coord_for_vehicle`/`world_coord` (the road-vehicle method) all return `Option<(f32, f32)>`. `sprite_key` is `String` everywhere. Frontend `BackendPedestrian`/`BackendCar` shapes mirror what `drawPedestrian`/`drawCar` consume.
- **Out of spec scope:** No frame interpolation, no viewport filter, no ECS migration, no LOD — all explicitly Phase 2+ per the roadmap.
