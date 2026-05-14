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
