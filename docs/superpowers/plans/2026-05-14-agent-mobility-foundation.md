# Agent Mobility Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first authoritative backend slice where an agent can walk to a stop, board a vehicle, ride as a passenger, alight, and continue walking.

**Status 2026-05-15:** Backend foundation implemented and verified with `cargo test --manifest-path backend/Cargo.toml --workspace`. The current branch exposes `/mobility` and streams `mobility_delta` messages over `/ws`; frontend rendering of mobility is still outside this plan's completed backend scope.

**Architecture:** Keep agent intent/mobility separate from traffic. `sim-core` owns deterministic mobility data and systems; `sim-server` exposes snapshots and deltas. Vehicles are independent simulated objects, and agents refer to vehicles through an explicit `InVehicle` location instead of being moved separately while riding.

**Tech Stack:** Rust 2024, `bevy_ecs` for materialized hot entities, pure Rust structs for dense deterministic mobility state, Axum HTTP/WebSocket DTOs through `abutown-protocol`, TDD with targeted Cargo package tests.

---

## Literature Context

Use the local literature index before implementing:

- `docs/literature/agent-simulation/README.md`
- `docs/literature/agent-simulation/sources/sumo-persons.html`
- `docs/literature/agent-simulation/sources/sumo-public-transport.html`
- `docs/literature/agent-simulation/sources/matsim-book-part-one-latest.pdf`
- `docs/literature/agent-simulation/sources/bevy-ecs-docs.html`
- `docs/literature/agent-simulation/sources/dynamic-lod-large-scale-agent-urban-simulations-aamas2011.pdf`

The design follows these source-backed rules:

- Agent plans are activity/leg based, as in MATSim.
- Mobility states include walking, waiting, boarding, riding, and alighting, as in SUMO persons.
- Vehicles are separate simulated objects with capacity and route state.
- Hot path data stays stable and cache-friendly; do not add/remove ECS components per mobility micro-state.
- The first slice has deterministic route movement and stop dwell time, not full traffic AI.

## File Structure

- Modify `backend/crates/protocol/src/lib.rs`: add serializable mobility DTOs and a `MobilityDelta` websocket message.
- Modify `backend/crates/sim-core/src/ids.rs`: add stable IDs for agents, vehicles, stops, routes, and links.
- Create `backend/crates/sim-core/src/mobility.rs`: agent plans, mobility states, stops, routes, vehicles, deterministic `MobilityWorld`, movement tick, boarding tick, and snapshot building.
- Modify `backend/crates/sim-core/src/lib.rs`: export `mobility`.
- Modify `backend/crates/sim-core/src/ecs_runtime.rs`: add `MaterializedKind::Agent` and `MaterializedKind::Vehicle`.
- Modify `backend/crates/sim-server/src/runtime.rs`: seed a small mobility scenario and expose mobility snapshot/delta methods.
- Modify `backend/crates/sim-server/src/app.rs`: add `GET /mobility` and include mobility deltas in the existing websocket broadcast loop.
- Modify `backend/crates/sim-server/tests/http.rs`: test the mobility snapshot endpoint.
- Modify `backend/crates/sim-server/tests/websocket.rs`: test mobility deltas.
- Modify `backend/README.md`: document targeted mobility test commands and the traffic boundary.

## Scope Boundaries

This plan builds the basics only:

- one seeded deterministic route,
- one stop pair,
- one vehicle with capacity,
- one agent plan,
- fixed tick movement,
- stop dwell and boarding/alighting transitions,
- HTTP snapshot and WebSocket delta.

This plan does not implement lane traffic, congestion, pathfinding, parking, player commands, Supabase persistence, frontend rendering, or LLM/cognitive agents.

### Task 1: Protocol Mobility DTOs

**Files:**
- Modify: `backend/crates/protocol/src/lib.rs`

- [ ] **Step 1: Add failing protocol serialization tests**

Append these tests inside the existing `#[cfg(test)] mod tests` in `backend/crates/protocol/src/lib.rs`:

```rust
#[test]
fn mobility_snapshot_serializes_agents_vehicles_and_stops() {
    let snapshot = MobilitySnapshotDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        tick: 3,
        agents: vec![AgentMobilityDto {
            id: EntityId("agent:seed:0".to_string()),
            state: AgentMobilityStateDto::InVehicle {
                vehicle_id: EntityId("vehicle:tram:0".to_string()),
                seat_index: 0,
            },
            plan_cursor: 1,
        }],
        vehicles: vec![VehicleMobilityDto {
            id: EntityId("vehicle:tram:0".to_string()),
            route_id: "route:demo".to_string(),
            link_index: 0,
            progress: 0.5,
            capacity: 24,
            occupants: vec![EntityId("agent:seed:0".to_string())],
            dwell_ticks_remaining: 0,
        }],
        stops: vec![StopMobilityDto {
            id: "stop:old-town".to_string(),
            route_id: "route:demo".to_string(),
            link_index: 0,
            progress: 0.0,
            waiting_agents: vec![],
        }],
    };

    let json = serde_json::to_value(&snapshot).expect("mobility snapshot serializes");

    assert_eq!(json["protocol_version"], 1);
    assert_eq!(json["world_id"], "abutown-main");
    assert_eq!(json["tick"], 3);
    assert_eq!(json["agents"][0]["id"], "agent:seed:0");
    assert_eq!(json["agents"][0]["state"]["type"], "in_vehicle");
    assert_eq!(json["agents"][0]["state"]["vehicle_id"], "vehicle:tram:0");
    assert_eq!(json["vehicles"][0]["occupants"][0], "agent:seed:0");
    assert_eq!(json["stops"][0]["id"], "stop:old-town");
}

#[test]
fn websocket_mobility_delta_serializes_with_type_tag() {
    let message = ServerMessageDto::MobilityDelta(MobilityDeltaDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        tick: 8,
        changed_agents: vec![AgentMobilityDto {
            id: EntityId("agent:seed:0".to_string()),
            state: AgentMobilityStateDto::WaitingAtStop {
                stop_id: "stop:old-town".to_string(),
            },
            plan_cursor: 0,
        }],
        changed_vehicles: vec![],
    });

    let json = serde_json::to_value(&message).expect("mobility delta serializes");

    assert_eq!(json["type"], "mobility_delta");
    assert_eq!(json["tick"], 8);
    assert_eq!(json["changed_agents"][0]["state"]["type"], "waiting_at_stop");
    assert_eq!(json["changed_agents"][0]["state"]["stop_id"], "stop:old-town");
}
```

- [ ] **Step 2: Run the failing protocol tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol mobility_
```

Expected: FAIL with missing `MobilitySnapshotDto`, `AgentMobilityDto`, `VehicleMobilityDto`, `StopMobilityDto`, `AgentMobilityStateDto`, `MobilityDeltaDto`, and `ServerMessageDto::MobilityDelta`.

- [ ] **Step 3: Add protocol DTOs**

In `backend/crates/protocol/src/lib.rs`, add these DTOs above `ServerMessageDto`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MobilitySnapshotDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub tick: u64,
    pub agents: Vec<AgentMobilityDto>,
    pub vehicles: Vec<VehicleMobilityDto>,
    pub stops: Vec<StopMobilityDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MobilityDeltaDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub tick: u64,
    pub changed_agents: Vec<AgentMobilityDto>,
    pub changed_vehicles: Vec<VehicleMobilityDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentMobilityDto {
    pub id: EntityId,
    pub state: AgentMobilityStateDto,
    pub plan_cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMobilityStateDto {
    AtActivity { activity_id: String },
    Walking { link_id: String, progress: f32 },
    WaitingAtStop { stop_id: String },
    Boarding { vehicle_id: EntityId, stop_id: String },
    InVehicle { vehicle_id: EntityId, seat_index: u16 },
    Alighting { vehicle_id: EntityId, stop_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VehicleMobilityDto {
    pub id: EntityId,
    pub route_id: String,
    pub link_index: usize,
    pub progress: f32,
    pub capacity: u16,
    pub occupants: Vec<EntityId>,
    pub dwell_ticks_remaining: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StopMobilityDto {
    pub id: String,
    pub route_id: String,
    pub link_index: usize,
    pub progress: f32,
    pub waiting_agents: Vec<EntityId>,
}
```

Then extend `ServerMessageDto`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessageDto {
    Hello(ServerHelloDto),
    TilePulse(TilePulseDeltaDto),
    MobilityDelta(MobilityDeltaDto),
    Error(ServerErrorDto),
}
```

- [ ] **Step 4: Verify protocol tests pass**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol mobility_
```

Expected: PASS for the two mobility protocol tests.

- [ ] **Step 5: Commit protocol DTOs**

Run:

```bash
git add backend/crates/protocol/src/lib.rs
git commit -m "feat: add mobility protocol DTOs"
```

### Task 2: Core Mobility Domain Model

**Files:**
- Modify: `backend/crates/sim-core/src/ids.rs`
- Create: `backend/crates/sim-core/src/mobility.rs`
- Modify: `backend/crates/sim-core/src/lib.rs`

- [ ] **Step 1: Write failing domain tests**

Create `backend/crates/sim-core/src/mobility.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{AgentId, LinkId, RouteId, StopId, VehicleId};

    #[test]
    fn seeded_world_starts_with_agent_walking_to_pickup_stop() {
        let world = MobilityWorld::seeded_demo();
        let agent = world
            .agent(&AgentId("agent:seed:0".to_string()))
            .expect("seed agent exists");
        let vehicle = world
            .vehicle(&VehicleId("vehicle:shuttle:0".to_string()))
            .expect("seed vehicle exists");
        let stop = world
            .stop(&StopId("stop:old-town".to_string()))
            .expect("seed stop exists");

        assert_eq!(agent.plan_cursor, 0);
        assert_eq!(
            agent.state,
            AgentMobilityState::Walking {
                link_id: LinkId("link:home-to-old-town-stop".to_string()),
                progress: 0.0
            }
        );
        assert_eq!(vehicle.route_id, RouteId("route:old-town-loop".to_string()));
        assert_eq!(vehicle.capacity, 4);
        assert_eq!(stop.route_id, RouteId("route:old-town-loop".to_string()));
    }
}
```

- [ ] **Step 2: Run the failing domain test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core seeded_world_starts_with_agent_walking_to_pickup_stop
```

Expected: FAIL because the mobility module and ID newtypes do not exist.

- [ ] **Step 3: Add stable mobility IDs**

In `backend/crates/sim-core/src/ids.rs`, add these newtypes below `StableEntityId`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VehicleId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StopId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RouteId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LinkId(pub String);
```

- [ ] **Step 4: Add the mobility module export**

In `backend/crates/sim-core/src/lib.rs`, add:

```rust
pub mod mobility;
```

- [ ] **Step 5: Implement the domain model**

Replace `backend/crates/sim-core/src/mobility.rs` with:

```rust
use std::collections::{HashMap, VecDeque};

use abutown_protocol::{
    AgentMobilityDto, AgentMobilityStateDto, EntityId, MobilityDeltaDto, MobilitySnapshotDto,
    PROTOCOL_VERSION, StopMobilityDto, VehicleMobilityDto, WorldId,
};

use crate::ids::{AgentId, LinkId, RouteId, StopId, VehicleId};

#[derive(Debug, Clone, PartialEq)]
pub enum AgentMobilityState {
    AtActivity { activity_id: String },
    Walking { link_id: LinkId, progress: f32 },
    WaitingAtStop { stop_id: StopId },
    Boarding { vehicle_id: VehicleId, stop_id: StopId },
    InVehicle { vehicle_id: VehicleId, seat_index: u16 },
    Alighting { vehicle_id: VehicleId, stop_id: StopId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanStage {
    WalkToStop { link_id: LinkId, stop_id: StopId },
    RideToStop { route_id: RouteId, stop_id: StopId },
    WalkToActivity { link_id: LinkId, activity_id: String },
    Activity { activity_id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentRecord {
    pub id: AgentId,
    pub state: AgentMobilityState,
    pub plan: Vec<PlanStage>,
    pub plan_cursor: usize,
    pub walk_speed_per_tick: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VehicleRecord {
    pub id: VehicleId,
    pub route_id: RouteId,
    pub link_index: usize,
    pub progress: f32,
    pub speed_per_tick: f32,
    pub capacity: u16,
    pub occupants: Vec<AgentId>,
    pub dwell_ticks_remaining: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StopRecord {
    pub id: StopId,
    pub route_id: RouteId,
    pub link_index: usize,
    pub progress: f32,
    pub waiting_agents: VecDeque<AgentId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteRecord {
    pub id: RouteId,
    pub links: Vec<LinkId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MobilitySnapshot {
    pub agents: Vec<AgentRecord>,
    pub vehicles: Vec<VehicleRecord>,
    pub stops: Vec<StopRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MobilityDelta {
    pub changed_agents: Vec<AgentRecord>,
    pub changed_vehicles: Vec<VehicleRecord>,
}

#[derive(Debug, Default)]
pub struct MobilityWorld {
    tick: u64,
    agents: HashMap<AgentId, AgentRecord>,
    vehicles: HashMap<VehicleId, VehicleRecord>,
    stops: HashMap<StopId, StopRecord>,
    routes: HashMap<RouteId, RouteRecord>,
}

impl MobilityWorld {
    pub fn seeded_demo() -> Self {
        let route_id = RouteId("route:old-town-loop".to_string());
        let pickup_stop_id = StopId("stop:old-town".to_string());
        let dropoff_stop_id = StopId("stop:station".to_string());
        let walk_to_pickup = LinkId("link:home-to-old-town-stop".to_string());
        let vehicle_link = LinkId("link:old-town-to-station".to_string());
        let walk_to_activity = LinkId("link:station-to-work".to_string());
        let agent_id = AgentId("agent:seed:0".to_string());
        let vehicle_id = VehicleId("vehicle:shuttle:0".to_string());

        let mut routes = HashMap::new();
        routes.insert(
            route_id.clone(),
            RouteRecord {
                id: route_id.clone(),
                links: vec![vehicle_link],
            },
        );

        let mut stops = HashMap::new();
        stops.insert(
            pickup_stop_id.clone(),
            StopRecord {
                id: pickup_stop_id.clone(),
                route_id: route_id.clone(),
                link_index: 0,
                progress: 0.0,
                waiting_agents: VecDeque::new(),
            },
        );
        stops.insert(
            dropoff_stop_id.clone(),
            StopRecord {
                id: dropoff_stop_id.clone(),
                route_id: route_id.clone(),
                link_index: 0,
                progress: 1.0,
                waiting_agents: VecDeque::new(),
            },
        );

        let mut agents = HashMap::new();
        agents.insert(
            agent_id.clone(),
            AgentRecord {
                id: agent_id,
                state: AgentMobilityState::Walking {
                    link_id: walk_to_pickup.clone(),
                    progress: 0.0,
                },
                plan: vec![
                    PlanStage::WalkToStop {
                        link_id: walk_to_pickup,
                        stop_id: pickup_stop_id,
                    },
                    PlanStage::RideToStop {
                        route_id: route_id.clone(),
                        stop_id: dropoff_stop_id,
                    },
                    PlanStage::WalkToActivity {
                        link_id: walk_to_activity,
                        activity_id: "activity:work".to_string(),
                    },
                    PlanStage::Activity {
                        activity_id: "activity:work".to_string(),
                    },
                ],
                plan_cursor: 0,
                walk_speed_per_tick: 0.5,
            },
        );

        let mut vehicles = HashMap::new();
        vehicles.insert(
            vehicle_id.clone(),
            VehicleRecord {
                id: vehicle_id,
                route_id,
                link_index: 0,
                progress: 0.0,
                speed_per_tick: 0.5,
                capacity: 4,
                occupants: Vec::new(),
                dwell_ticks_remaining: 2,
            },
        );

        Self {
            tick: 0,
            agents,
            vehicles,
            stops,
            routes,
        }
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn agent(&self, id: &AgentId) -> Option<&AgentRecord> {
        self.agents.get(id)
    }

    pub fn vehicle(&self, id: &VehicleId) -> Option<&VehicleRecord> {
        self.vehicles.get(id)
    }

    pub fn stop(&self, id: &StopId) -> Option<&StopRecord> {
        self.stops.get(id)
    }

    pub fn snapshot(&self) -> MobilitySnapshot {
        let mut agents: Vec<AgentRecord> = self.agents.values().cloned().collect();
        agents.sort_by(|left, right| left.id.0.cmp(&right.id.0));
        let mut vehicles: Vec<VehicleRecord> = self.vehicles.values().cloned().collect();
        vehicles.sort_by(|left, right| left.id.0.cmp(&right.id.0));
        let mut stops: Vec<StopRecord> = self.stops.values().cloned().collect();
        stops.sort_by(|left, right| left.id.0.cmp(&right.id.0));
        MobilitySnapshot {
            agents,
            vehicles,
            stops,
        }
    }
}

impl From<&AgentRecord> for AgentMobilityDto {
    fn from(value: &AgentRecord) -> Self {
        Self {
            id: EntityId(value.id.0.clone()),
            state: AgentMobilityStateDto::from(&value.state),
            plan_cursor: value.plan_cursor,
        }
    }
}

impl From<&AgentMobilityState> for AgentMobilityStateDto {
    fn from(value: &AgentMobilityState) -> Self {
        match value {
            AgentMobilityState::AtActivity { activity_id } => Self::AtActivity {
                activity_id: activity_id.clone(),
            },
            AgentMobilityState::Walking { link_id, progress } => Self::Walking {
                link_id: link_id.0.clone(),
                progress: *progress,
            },
            AgentMobilityState::WaitingAtStop { stop_id } => Self::WaitingAtStop {
                stop_id: stop_id.0.clone(),
            },
            AgentMobilityState::Boarding {
                vehicle_id,
                stop_id,
            } => Self::Boarding {
                vehicle_id: EntityId(vehicle_id.0.clone()),
                stop_id: stop_id.0.clone(),
            },
            AgentMobilityState::InVehicle {
                vehicle_id,
                seat_index,
            } => Self::InVehicle {
                vehicle_id: EntityId(vehicle_id.0.clone()),
                seat_index: *seat_index,
            },
            AgentMobilityState::Alighting {
                vehicle_id,
                stop_id,
            } => Self::Alighting {
                vehicle_id: EntityId(vehicle_id.0.clone()),
                stop_id: stop_id.0.clone(),
            },
        }
    }
}

impl From<&VehicleRecord> for VehicleMobilityDto {
    fn from(value: &VehicleRecord) -> Self {
        Self {
            id: EntityId(value.id.0.clone()),
            route_id: value.route_id.0.clone(),
            link_index: value.link_index,
            progress: value.progress,
            capacity: value.capacity,
            occupants: value
                .occupants
                .iter()
                .map(|agent_id| EntityId(agent_id.0.clone()))
                .collect(),
            dwell_ticks_remaining: value.dwell_ticks_remaining,
        }
    }
}

impl From<&StopRecord> for StopMobilityDto {
    fn from(value: &StopRecord) -> Self {
        Self {
            id: value.id.0.clone(),
            route_id: value.route_id.0.clone(),
            link_index: value.link_index,
            progress: value.progress,
            waiting_agents: value
                .waiting_agents
                .iter()
                .map(|agent_id| EntityId(agent_id.0.clone()))
                .collect(),
        }
    }
}

pub fn build_mobility_snapshot_dto(
    world_id: &WorldId,
    tick: u64,
    snapshot: MobilitySnapshot,
) -> MobilitySnapshotDto {
    MobilitySnapshotDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: world_id.clone(),
        tick,
        agents: snapshot.agents.iter().map(AgentMobilityDto::from).collect(),
        vehicles: snapshot
            .vehicles
            .iter()
            .map(VehicleMobilityDto::from)
            .collect(),
        stops: snapshot.stops.iter().map(StopMobilityDto::from).collect(),
    }
}

pub fn build_mobility_delta_dto(
    world_id: &WorldId,
    tick: u64,
    delta: MobilityDelta,
) -> MobilityDeltaDto {
    MobilityDeltaDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: world_id.clone(),
        tick,
        changed_agents: delta
            .changed_agents
            .iter()
            .map(AgentMobilityDto::from)
            .collect(),
        changed_vehicles: delta
            .changed_vehicles
            .iter()
            .map(VehicleMobilityDto::from)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{AgentId, LinkId, RouteId, StopId, VehicleId};

    #[test]
    fn seeded_world_starts_with_agent_walking_to_pickup_stop() {
        let world = MobilityWorld::seeded_demo();
        let agent = world
            .agent(&AgentId("agent:seed:0".to_string()))
            .expect("seed agent exists");
        let vehicle = world
            .vehicle(&VehicleId("vehicle:shuttle:0".to_string()))
            .expect("seed vehicle exists");
        let stop = world
            .stop(&StopId("stop:old-town".to_string()))
            .expect("seed stop exists");

        assert_eq!(agent.plan_cursor, 0);
        assert_eq!(
            agent.state,
            AgentMobilityState::Walking {
                link_id: LinkId("link:home-to-old-town-stop".to_string()),
                progress: 0.0
            }
        );
        assert_eq!(vehicle.route_id, RouteId("route:old-town-loop".to_string()));
        assert_eq!(vehicle.capacity, 4);
        assert_eq!(stop.route_id, RouteId("route:old-town-loop".to_string()));
    }
}
```

- [ ] **Step 6: Verify the domain test passes**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core seeded_world_starts_with_agent_walking_to_pickup_stop
```

Expected: PASS.

- [ ] **Step 7: Commit the domain model**

Run:

```bash
git add backend/crates/sim-core/src/ids.rs backend/crates/sim-core/src/lib.rs backend/crates/sim-core/src/mobility.rs
git commit -m "feat: add mobility domain model"
```

### Task 3: Walking And Waiting Tick

**Files:**
- Modify: `backend/crates/sim-core/src/mobility.rs`

- [ ] **Step 1: Add failing walking tests**

Append these tests to the existing mobility test module:

```rust
#[test]
fn walking_agent_reaches_pickup_stop_and_waits() {
    let mut world = MobilityWorld::seeded_demo();
    let agent_id = AgentId("agent:seed:0".to_string());

    let first_delta = world.tick_mobility();
    let agent = world.agent(&agent_id).expect("agent exists");
    assert_eq!(
        agent.state,
        AgentMobilityState::Walking {
            link_id: LinkId("link:home-to-old-town-stop".to_string()),
            progress: 0.5
        }
    );
    assert_eq!(first_delta.changed_agents.len(), 1);

    let second_delta = world.tick_mobility();
    let agent = world.agent(&agent_id).expect("agent exists");
    let stop = world
        .stop(&StopId("stop:old-town".to_string()))
        .expect("pickup stop exists");

    assert_eq!(
        agent.state,
        AgentMobilityState::WaitingAtStop {
            stop_id: StopId("stop:old-town".to_string())
        }
    );
    assert_eq!(agent.plan_cursor, 1);
    assert_eq!(stop.waiting_agents.iter().cloned().collect::<Vec<_>>(), vec![agent_id]);
    assert_eq!(second_delta.changed_agents.len(), 1);
}
```

- [ ] **Step 2: Run the failing walking test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core walking_agent_reaches_pickup_stop_and_waits
```

Expected: FAIL because `tick_mobility` does not exist.

- [ ] **Step 3: Implement walking progression**

In the `impl MobilityWorld` block in `backend/crates/sim-core/src/mobility.rs`, add:

```rust
pub fn tick_mobility(&mut self) -> MobilityDelta {
    self.tick += 1;
    let mut changed_agents = Vec::new();
    let mut changed_vehicles = Vec::new();

    let agent_ids: Vec<AgentId> = self.agents.keys().cloned().collect();
    for agent_id in agent_ids {
        if self.tick_walking_agent(&agent_id) {
            if let Some(agent) = self.agents.get(&agent_id) {
                changed_agents.push(agent.clone());
            }
        }
    }

    MobilityDelta {
        changed_agents,
        changed_vehicles,
    }
}

fn tick_walking_agent(&mut self, agent_id: &AgentId) -> bool {
    let Some(agent) = self.agents.get_mut(agent_id) else {
        return false;
    };

    let AgentMobilityState::Walking { link_id, progress } = &agent.state else {
        return false;
    };

    let next_progress = (*progress + agent.walk_speed_per_tick).min(1.0);
    let link_id = link_id.clone();

    if next_progress < 1.0 {
        agent.state = AgentMobilityState::Walking {
            link_id,
            progress: next_progress,
        };
        return true;
    }

    let Some(PlanStage::WalkToStop { stop_id, .. }) = agent.plan.get(agent.plan_cursor) else {
        return false;
    };
    let stop_id = stop_id.clone();
    agent.plan_cursor += 1;
    agent.state = AgentMobilityState::WaitingAtStop {
        stop_id: stop_id.clone(),
    };

    if let Some(stop) = self.stops.get_mut(&stop_id) {
        if !stop.waiting_agents.contains(agent_id) {
            stop.waiting_agents.push_back(agent_id.clone());
        }
    }

    true
}
```

- [ ] **Step 4: Verify walking test passes**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core walking_agent_reaches_pickup_stop_and_waits
```

Expected: PASS.

- [ ] **Step 5: Commit walking tick**

Run:

```bash
git add backend/crates/sim-core/src/mobility.rs
git commit -m "feat: move agents from walking to waiting"
```

### Task 4: Vehicle Route Tick And Stop Dwell

**Files:**
- Modify: `backend/crates/sim-core/src/mobility.rs`

- [ ] **Step 1: Add failing vehicle tick test**

Append this test to the mobility test module:

```rust
#[test]
fn vehicle_respects_initial_dwell_then_moves_on_route() {
    let mut world = MobilityWorld::seeded_demo();
    let vehicle_id = VehicleId("vehicle:shuttle:0".to_string());

    let first_delta = world.tick_mobility();
    let vehicle = world.vehicle(&vehicle_id).expect("vehicle exists");
    assert_eq!(vehicle.progress, 0.0);
    assert_eq!(vehicle.dwell_ticks_remaining, 1);
    assert_eq!(first_delta.changed_vehicles.len(), 1);

    let second_delta = world.tick_mobility();
    let vehicle = world.vehicle(&vehicle_id).expect("vehicle exists");
    assert_eq!(vehicle.progress, 0.0);
    assert_eq!(vehicle.dwell_ticks_remaining, 0);
    assert_eq!(second_delta.changed_vehicles.len(), 1);

    let third_delta = world.tick_mobility();
    let vehicle = world.vehicle(&vehicle_id).expect("vehicle exists");
    assert_eq!(vehicle.progress, 0.5);
    assert_eq!(vehicle.dwell_ticks_remaining, 0);
    assert_eq!(third_delta.changed_vehicles.len(), 1);
}
```

- [ ] **Step 2: Run the failing vehicle test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core vehicle_respects_initial_dwell_then_moves_on_route
```

Expected: FAIL because `tick_mobility` only changes agents.

- [ ] **Step 3: Implement vehicle movement**

In `tick_mobility`, after the walking-agent loop and before returning `MobilityDelta`, add:

```rust
let vehicle_ids: Vec<VehicleId> = self.vehicles.keys().cloned().collect();
for vehicle_id in vehicle_ids {
    if self.tick_vehicle(&vehicle_id) {
        if let Some(vehicle) = self.vehicles.get(&vehicle_id) {
            changed_vehicles.push(vehicle.clone());
        }
    }
}
```

Add this helper inside `impl MobilityWorld`:

```rust
fn tick_vehicle(&mut self, vehicle_id: &VehicleId) -> bool {
    let Some(vehicle) = self.vehicles.get_mut(vehicle_id) else {
        return false;
    };

    if vehicle.dwell_ticks_remaining > 0 {
        vehicle.dwell_ticks_remaining -= 1;
        return true;
    }

    let Some(route) = self.routes.get(&vehicle.route_id) else {
        return false;
    };
    if route.links.is_empty() {
        return false;
    }

    vehicle.progress += vehicle.speed_per_tick;
    if vehicle.progress >= 1.0 {
        vehicle.progress = 1.0;
    }

    true
}
```

- [ ] **Step 4: Verify vehicle test passes**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core vehicle_respects_initial_dwell_then_moves_on_route
```

Expected: PASS.

- [ ] **Step 5: Commit vehicle route tick**

Run:

```bash
git add backend/crates/sim-core/src/mobility.rs
git commit -m "feat: add deterministic vehicle movement"
```

### Task 5: Boarding, Riding, And Alighting

**Files:**
- Modify: `backend/crates/sim-core/src/mobility.rs`

- [ ] **Step 1: Add failing boarding and alighting test**

Append this test to the mobility test module:

```rust
#[test]
fn agent_boards_rides_alights_and_walks_to_activity() {
    let mut world = MobilityWorld::seeded_demo();
    let agent_id = AgentId("agent:seed:0".to_string());
    let vehicle_id = VehicleId("vehicle:shuttle:0".to_string());

    world.tick_mobility();
    world.tick_mobility();

    let waiting = world.agent(&agent_id).expect("agent exists");
    assert_eq!(
        waiting.state,
        AgentMobilityState::WaitingAtStop {
            stop_id: StopId("stop:old-town".to_string())
        }
    );

    world.tick_mobility();
    let boarded = world.agent(&agent_id).expect("agent exists");
    let vehicle = world.vehicle(&vehicle_id).expect("vehicle exists");
    assert_eq!(
        boarded.state,
        AgentMobilityState::InVehicle {
            vehicle_id: vehicle_id.clone(),
            seat_index: 0
        }
    );
    assert_eq!(vehicle.occupants, vec![agent_id.clone()]);

    world.tick_mobility();
    let riding = world.agent(&agent_id).expect("agent exists");
    assert!(matches!(riding.state, AgentMobilityState::InVehicle { .. }));

    world.tick_mobility();
    let alighted = world.agent(&agent_id).expect("agent exists");
    let vehicle = world.vehicle(&vehicle_id).expect("vehicle exists");
    assert_eq!(vehicle.occupants, Vec::<AgentId>::new());
    assert_eq!(
        alighted.state,
        AgentMobilityState::Walking {
            link_id: LinkId("link:station-to-work".to_string()),
            progress: 0.0
        }
    );
    assert_eq!(alighted.plan_cursor, 2);

    world.tick_mobility();
    world.tick_mobility();
    let arrived = world.agent(&agent_id).expect("agent exists");
    assert_eq!(
        arrived.state,
        AgentMobilityState::AtActivity {
            activity_id: "activity:work".to_string()
        }
    );
    assert_eq!(arrived.plan_cursor, 3);
}
```

- [ ] **Step 2: Run the failing boarding test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core agent_boards_rides_alights_and_walks_to_activity
```

Expected: FAIL because boarding and alighting are not implemented.

- [ ] **Step 3: Extend walking completion for final activity**

Replace `tick_walking_agent` with this version:

```rust
fn tick_walking_agent(&mut self, agent_id: &AgentId) -> bool {
    let Some(agent) = self.agents.get_mut(agent_id) else {
        return false;
    };

    let AgentMobilityState::Walking { link_id, progress } = &agent.state else {
        return false;
    };

    let next_progress = (*progress + agent.walk_speed_per_tick).min(1.0);
    let link_id = link_id.clone();

    if next_progress < 1.0 {
        agent.state = AgentMobilityState::Walking {
            link_id,
            progress: next_progress,
        };
        return true;
    }

    match agent.plan.get(agent.plan_cursor).cloned() {
        Some(PlanStage::WalkToStop { stop_id, .. }) => {
            agent.plan_cursor += 1;
            agent.state = AgentMobilityState::WaitingAtStop {
                stop_id: stop_id.clone(),
            };

            if let Some(stop) = self.stops.get_mut(&stop_id) {
                if !stop.waiting_agents.contains(agent_id) {
                    stop.waiting_agents.push_back(agent_id.clone());
                }
            }
            true
        }
        Some(PlanStage::WalkToActivity { activity_id, .. }) => {
            agent.plan_cursor += 1;
            agent.state = AgentMobilityState::AtActivity { activity_id };
            true
        }
        _ => false,
    }
}
```

- [ ] **Step 4: Add boarding and alighting phases**

Replace `tick_mobility` with this phased order. Boarding happens before walking, so an agent that reaches a stop boards on the next tick. Alighting happens before vehicle movement, so an agent rides through the tick where the vehicle arrives and exits on the next tick.

```rust
pub fn tick_mobility(&mut self) -> MobilityDelta {
    self.tick += 1;
    let mut changed_agents = Vec::new();
    let mut changed_vehicles = Vec::new();

    for agent_id in self.tick_boarding() {
        if let Some(agent) = self.agents.get(&agent_id) {
            changed_agents.push(agent.clone());
        }
    }

    let agent_ids: Vec<AgentId> = self.agents.keys().cloned().collect();
    for agent_id in agent_ids {
        if self.tick_walking_agent(&agent_id) {
            if let Some(agent) = self.agents.get(&agent_id) {
                changed_agents.push(agent.clone());
            }
        }
    }

    for agent_id in self.tick_alighting() {
        if let Some(agent) = self.agents.get(&agent_id) {
            changed_agents.push(agent.clone());
        }
    }

    let vehicle_ids: Vec<VehicleId> = self.vehicles.keys().cloned().collect();
    for vehicle_id in vehicle_ids {
        if self.tick_vehicle(&vehicle_id) {
            if let Some(vehicle) = self.vehicles.get(&vehicle_id) {
                changed_vehicles.push(vehicle.clone());
            }
        }
    }

    MobilityDelta {
        changed_agents,
        changed_vehicles,
    }
}
```

Add these helpers inside `impl MobilityWorld`:

```rust
fn tick_boarding(&mut self) -> Vec<AgentId> {
    let mut changed_agents = Vec::new();
    let stop_ids: Vec<StopId> = self.stops.keys().cloned().collect();

    for stop_id in stop_ids {
        let Some((route_id, link_index, stop_progress, next_agent_id)) =
            self.stops.get(&stop_id).and_then(|stop| {
                stop.waiting_agents.front().cloned().map(|agent_id| {
                    (
                        stop.route_id.clone(),
                        stop.link_index,
                        stop.progress,
                        agent_id,
                    )
                })
            })
        else {
            continue;
        };

        let Some(vehicle_id) = self
            .vehicles
            .values()
            .find(|vehicle| {
                vehicle.route_id == route_id
                    && vehicle.link_index == link_index
                    && vehicle.progress == stop_progress
                    && vehicle.occupants.len() < usize::from(vehicle.capacity)
            })
            .map(|vehicle| vehicle.id.clone())
        else {
            continue;
        };

        let seat_index = {
            let vehicle = self
                .vehicles
                .get_mut(&vehicle_id)
                .expect("selected vehicle exists");
            let seat_index = vehicle.occupants.len() as u16;
            vehicle.occupants.push(next_agent_id.clone());
            seat_index
        };

        let stop = self.stops.get_mut(&stop_id).expect("selected stop exists");
        let popped = stop.waiting_agents.pop_front();
        assert_eq!(popped, Some(next_agent_id.clone()));

        if let Some(agent) = self.agents.get_mut(&next_agent_id) {
            agent.state = AgentMobilityState::InVehicle {
                vehicle_id,
                seat_index,
            };
            changed_agents.push(next_agent_id);
        }
    }

    changed_agents
}

fn tick_alighting(&mut self) -> Vec<AgentId> {
    let mut changed_agents = Vec::new();
    let vehicle_ids: Vec<VehicleId> = self.vehicles.keys().cloned().collect();

    for vehicle_id in vehicle_ids {
        let Some((route_id, link_index, progress, occupants)) =
            self.vehicles.get(&vehicle_id).map(|vehicle| {
                (
                    vehicle.route_id.clone(),
                    vehicle.link_index,
                    vehicle.progress,
                    vehicle.occupants.clone(),
                )
            })
        else {
            continue;
        };

        let Some(stop_id) = self
            .stops
            .values()
            .find(|stop| {
                stop.route_id == route_id
                    && stop.link_index == link_index
                    && stop.progress == progress
                    && stop.progress == 1.0
            })
            .map(|stop| stop.id.clone())
        else {
            continue;
        };

        for agent_id in occupants {
            let should_alight = self
                .agents
                .get(&agent_id)
                .and_then(|agent| agent.plan.get(agent.plan_cursor))
                .is_some_and(|stage| {
                    matches!(
                        stage,
                        PlanStage::RideToStop {
                            stop_id: target_stop_id,
                            ..
                        } if *target_stop_id == stop_id
                    )
                });

            if !should_alight {
                continue;
            }

            if let Some(vehicle) = self.vehicles.get_mut(&vehicle_id) {
                vehicle.occupants.retain(|occupant_id| occupant_id != &agent_id);
            }

            if let Some(agent) = self.agents.get_mut(&agent_id) {
                agent.plan_cursor += 1;
                match agent.plan.get(agent.plan_cursor).cloned() {
                    Some(PlanStage::WalkToActivity { link_id, .. }) => {
                        agent.state = AgentMobilityState::Walking {
                            link_id,
                            progress: 0.0,
                        };
                    }
                    Some(PlanStage::Activity { activity_id }) => {
                        agent.plan_cursor += 1;
                        agent.state = AgentMobilityState::AtActivity { activity_id };
                    }
                    _ => {
                        agent.state = AgentMobilityState::Alighting {
                            vehicle_id: vehicle_id.clone(),
                            stop_id: stop_id.clone(),
                        };
                    }
                }
                changed_agents.push(agent_id);
            }
        }
    }

    changed_agents
}
```

- [ ] **Step 5: Verify boarding test passes**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core agent_boards_rides_alights_and_walks_to_activity
```

Expected: PASS.

- [ ] **Step 6: Run all mobility core tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core mobility
```

Expected: PASS for all mobility module tests.

- [ ] **Step 7: Commit boarding and alighting**

Run:

```bash
git add backend/crates/sim-core/src/mobility.rs
git commit -m "feat: board agents into vehicles"
```

### Task 6: Materialized Runtime Kinds

**Files:**
- Modify: `backend/crates/sim-core/src/ecs_runtime.rs`

- [ ] **Step 1: Add failing materialization test**

Append this test to the existing `ecs_runtime.rs` test module:

```rust
#[test]
fn agents_and_vehicles_can_be_materialized_as_hot_entities() {
    let mut runtime = MaterializedRuntime::default();

    let agent = runtime.spawn_materialized(
        StableEntityId("agent:seed:0".to_string()),
        ChunkCoord { x: 4, y: 4 },
        MaterializedKind::Agent,
    );
    let vehicle = runtime.spawn_materialized(
        StableEntityId("vehicle:shuttle:0".to_string()),
        ChunkCoord { x: 4, y: 4 },
        MaterializedKind::Vehicle,
    );

    assert_ne!(agent, vehicle);
    assert_eq!(runtime.materialized_count(), 2);
}
```

- [ ] **Step 2: Run the failing materialization test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core agents_and_vehicles_can_be_materialized_as_hot_entities
```

Expected: FAIL because `MaterializedKind::Agent` and `MaterializedKind::Vehicle` do not exist.

- [ ] **Step 3: Add materialized kinds**

Change `MaterializedKind` in `backend/crates/sim-core/src/ecs_runtime.rs` to:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterializedKind {
    Agent,
    Vehicle,
    Player,
    Item,
    Machine,
}
```

- [ ] **Step 4: Verify materialization test passes**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core agents_and_vehicles_can_be_materialized_as_hot_entities
```

Expected: PASS.

- [ ] **Step 5: Commit materialized kinds**

Run:

```bash
git add backend/crates/sim-core/src/ecs_runtime.rs
git commit -m "feat: materialize agents and vehicles"
```

### Task 7: Runtime Mobility Snapshot

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs`
- Modify: `backend/crates/sim-server/src/app.rs`
- Modify: `backend/crates/sim-server/tests/http.rs`

- [ ] **Step 1: Add failing HTTP snapshot test**

Append this test to `backend/crates/sim-server/tests/http.rs`:

```rust
#[tokio::test]
async fn mobility_snapshot_is_available() {
    let app = build_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/mobility")
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
    assert_eq!(json["tick"], 0);
    assert_eq!(json["agents"][0]["id"], "agent:seed:0");
    assert_eq!(json["agents"][0]["state"]["type"], "walking");
    assert_eq!(
        json["agents"][0]["state"]["link_id"],
        "link:home-to-old-town-stop"
    );
    assert_eq!(json["vehicles"][0]["id"], "vehicle:shuttle:0");
    assert_eq!(json["vehicles"][0]["capacity"], 4);
    assert_eq!(json["stops"].as_array().unwrap().len(), 2);
}
```

- [ ] **Step 2: Run the failing HTTP snapshot test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server mobility_snapshot_is_available
```

Expected: FAIL with 404 for `/mobility`.

- [ ] **Step 3: Add mobility state to `SimulationRuntime`**

Modify the imports in `backend/crates/sim-server/src/runtime.rs` to include:

```rust
use abutown_protocol::{
    ChunkCoordDto, ChunkSnapshotDto, HealthResponse, MobilityDeltaDto, MobilitySnapshotDto,
    PROTOCOL_VERSION, ServerHelloDto, ServerMessageDto, TilePulseDeltaDto, WorldId,
    WorldSummaryDto,
};
use sim_core::{
    chunk::Chunk,
    ids::ChunkCoord,
    mobility::{build_mobility_delta_dto, build_mobility_snapshot_dto, MobilityWorld},
    scheduler::ChunkActivity,
    tile::TileKind,
};
```

Add a field to `SimulationRuntime`:

```rust
mobility: MobilityWorld,
```

Initialize it in `SimulationRuntime::new()`:

```rust
mobility: MobilityWorld::seeded_demo(),
```

Add these methods to `impl SimulationRuntime`:

```rust
pub fn mobility_snapshot(&self) -> MobilitySnapshotDto {
    build_mobility_snapshot_dto(&self.world_id, self.mobility.tick(), self.mobility.snapshot())
}

pub fn next_mobility_delta(&mut self) -> MobilityDeltaDto {
    let delta = self.mobility.tick_mobility();
    build_mobility_delta_dto(&self.world_id, self.mobility.tick(), delta)
}
```

- [ ] **Step 4: Add `/mobility` route**

In `backend/crates/sim-server/src/app.rs`, add `MobilitySnapshotDto` to the protocol import:

```rust
use abutown_protocol::{
    ChunkSnapshotDto, HealthResponse, MobilitySnapshotDto, ServerMessageDto, WorldSummaryDto,
};
```

Add the route in `build_app_with_runtime`:

```rust
.route("/mobility", get(mobility))
```

Add the handler:

```rust
async fn mobility(State(state): State<AppState>) -> Json<MobilitySnapshotDto> {
    let runtime = state.runtime();
    let runtime = runtime.lock().await;
    Json(runtime.mobility_snapshot())
}
```

- [ ] **Step 5: Verify HTTP snapshot test passes**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server mobility_snapshot_is_available
```

Expected: PASS.

- [ ] **Step 6: Commit runtime snapshot**

Run:

```bash
git add backend/crates/sim-server/src/runtime.rs backend/crates/sim-server/src/app.rs backend/crates/sim-server/tests/http.rs
git commit -m "feat: expose mobility snapshot"
```

### Task 8: WebSocket Mobility Deltas

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs`
- Modify: `backend/crates/sim-server/src/app.rs`
- Modify: `backend/crates/sim-server/tests/websocket.rs`

- [ ] **Step 1: Add failing WebSocket mobility test**

Append this test to `backend/crates/sim-server/tests/websocket.rs`:

```rust
#[tokio::test]
async fn websocket_sends_mobility_deltas_after_hello() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, build_app()).await.unwrap();
    });

    let url = format!("ws://{addr}/ws");
    let (mut stream, _) = connect_async(url).await.unwrap();

    let hello = read_server_message(&mut stream).await;
    assert!(matches!(hello, ServerMessageDto::Hello(_)));

    let first = read_server_message(&mut stream).await;
    let second = read_server_message(&mut stream).await;

    let mobility_delta = match (first, second) {
        (ServerMessageDto::MobilityDelta(delta), _) => delta,
        (_, ServerMessageDto::MobilityDelta(delta)) => delta,
        _ => panic!("expected one mobility delta among first two broadcast messages"),
    };

    assert_eq!(mobility_delta.world_id.0, "abutown-main");
    assert_eq!(mobility_delta.tick, 1);
    assert!(!mobility_delta.changed_agents.is_empty());

    server.abort();
}
```

- [ ] **Step 2: Run the failing WebSocket mobility test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server websocket_sends_mobility_deltas_after_hello
```

Expected: FAIL because the broadcast loop only emits tile pulses.

- [ ] **Step 3: Add combined simulation tick messages**

In `backend/crates/sim-server/src/runtime.rs`, add:

```rust
pub fn next_server_messages(&mut self) -> Vec<ServerMessageDto> {
    vec![
        self.next_pulse(),
        ServerMessageDto::MobilityDelta(self.next_mobility_delta()),
    ]
}
```

In `backend/crates/sim-server/src/app.rs`, replace the body of the delta loop with:

```rust
loop {
    interval.tick().await;
    let messages = {
        let mut runtime = runtime.lock().await;
        runtime.next_server_messages()
    };
    for message in messages {
        let _ = deltas.send(message);
    }
}
```

- [ ] **Step 4: Verify WebSocket mobility test passes**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server websocket_sends_mobility_deltas_after_hello
```

Expected: PASS.

- [ ] **Step 5: Run existing WebSocket tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server --test websocket
```

Expected: PASS. If tests expecting exact second/third message ordering fail, update those tests to skip non-tile messages with a helper named `read_next_tile_pulse`.

Use this helper in `websocket.rs`:

```rust
async fn read_next_tile_pulse<S>(stream: &mut S) -> abutown_protocol::TilePulseDeltaDto
where
    S: futures_util::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    loop {
        let message = read_server_message(stream).await;
        if let ServerMessageDto::TilePulse(delta) = message {
            return delta;
        }
    }
}
```

- [ ] **Step 6: Commit WebSocket mobility deltas**

Run:

```bash
git add backend/crates/sim-server/src/runtime.rs backend/crates/sim-server/src/app.rs backend/crates/sim-server/tests/websocket.rs
git commit -m "feat: stream mobility deltas"
```

### Task 9: Documentation And Final Verification

**Files:**
- Modify: `backend/README.md`

- [ ] **Step 1: Update backend README**

Add this section to `backend/README.md` after the common commands:

```markdown
## Agent Mobility Foundation

The first mobility slice follows the local literature notes in
`docs/literature/agent-simulation/README.md`.

Architecture rules:

- Agents are people with plans and mobility state.
- Vehicles are separate traffic/transit entities with route, progress, capacity,
  dwell time, and occupants.
- Riding is represented by `AgentMobilityState::InVehicle`; the passenger
  position is derived from the vehicle position.
- Traffic behavior stays behind the vehicle layer. The initial slice uses
  deterministic route movement and stop dwell time only.

Targeted commands:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol mobility_
cargo test --manifest-path backend/Cargo.toml -p sim-core mobility
cargo test --manifest-path backend/Cargo.toml -p sim-server mobility_snapshot_is_available
cargo test --manifest-path backend/Cargo.toml -p sim-server websocket_sends_mobility_deltas_after_hello
```
```

- [ ] **Step 2: Run targeted verification**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol mobility_
cargo test --manifest-path backend/Cargo.toml -p sim-core mobility
cargo test --manifest-path backend/Cargo.toml -p sim-server mobility_snapshot_is_available
cargo test --manifest-path backend/Cargo.toml -p sim-server websocket_sends_mobility_deltas_after_hello
```

Expected: all commands PASS.

- [ ] **Step 3: Run formatting**

Run:

```bash
cargo fmt --manifest-path backend/Cargo.toml --all
```

Expected: command exits 0.

- [ ] **Step 4: Run clippy on changed backend crates**

Run:

```bash
cargo clippy --manifest-path backend/Cargo.toml -p abutown-protocol -p sim-core -p sim-server --all-targets -- -D warnings
```

Expected: command exits 0.

- [ ] **Step 5: Commit docs and formatting**

Run:

```bash
git add backend/README.md backend/crates/protocol/src/lib.rs backend/crates/sim-core/src backend/crates/sim-server/src backend/crates/sim-server/tests
git commit -m "docs: document mobility foundation"
```

## Self-Review

Spec coverage:

- Agent plans: Task 2 adds `PlanStage`.
- Walking: Task 3 moves agents from walking to stop waiting.
- Vehicle separation: Task 2 adds `VehicleRecord`; Task 4 moves vehicles independently.
- Boarding and riding: Task 5 moves agents into `InVehicle` and vehicle `occupants`.
- Alighting: Task 5 removes occupants and resumes walking.
- ECS hot entity path: Task 6 adds materialized agent/vehicle kinds.
- HTTP snapshot: Task 7 exposes `/mobility`.
- WebSocket deltas: Task 8 streams `MobilityDelta`.
- Documentation: Task 9 records the boundary and commands.

Placeholder scan:

- No unresolved task markers outside checkbox syntax.
- No unspecified test commands.
- No undefined function names in later tasks without an earlier creation step.

Type consistency:

- Protocol uses `EntityId` for agent and vehicle IDs crossing the wire.
- Core uses typed IDs: `AgentId`, `VehicleId`, `StopId`, `RouteId`, `LinkId`.
- Runtime converts core records into protocol DTOs through `build_mobility_snapshot_dto` and `build_mobility_delta_dto`.
