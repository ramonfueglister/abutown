use std::collections::{HashMap, HashSet, VecDeque};

use abutown_protocol::{
    AgentMobilityDto, AgentMobilityStateDto, EntityId, MobilityDeltaDto, MobilitySnapshotDto,
    PROTOCOL_VERSION, StopMobilityDto, VehicleMobilityDto, WorldId,
};
use serde::{Deserialize, Serialize};

use crate::ids::{AgentId, LinkId, RouteId, StopId, VehicleId};

fn stable_index(id: &str) -> u32 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    hasher.finish() as u32
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AgentMobilityState {
    AtActivity {
        activity_id: String,
    },
    Walking {
        link_id: LinkId,
        progress: f32,
    },
    WaitingAtStop {
        stop_id: StopId,
    },
    Boarding {
        vehicle_id: VehicleId,
        stop_id: StopId,
    },
    InVehicle {
        vehicle_id: VehicleId,
        seat_index: u16,
    },
    Alighting {
        vehicle_id: VehicleId,
        stop_id: StopId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanStage {
    WalkToStop {
        link_id: LinkId,
        stop_id: StopId,
    },
    RideToStop {
        route_id: RouteId,
        stop_id: StopId,
    },
    WalkToActivity {
        link_id: LinkId,
        activity_id: String,
    },
    Activity {
        activity_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRecord {
    pub id: AgentId,
    pub state: AgentMobilityState,
    pub plan: Vec<PlanStage>,
    pub plan_cursor: usize,
    pub walk_speed_per_tick: f32,
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StopRecord {
    pub id: StopId,
    pub route_id: RouteId,
    pub link_index: usize,
    pub progress: f32,
    pub waiting_agents: VecDeque<AgentId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct MobilityWorld {
    tick: u64,
    agents: HashMap<AgentId, AgentRecord>,
    vehicles: HashMap<VehicleId, VehicleRecord>,
    stops: HashMap<StopId, StopRecord>,
    routes: HashMap<RouteId, RouteRecord>,
    pub link_polylines: HashMap<LinkId, Vec<(f32, f32)>>,
}

impl MobilityWorld {
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

    pub fn tick_mobility(&mut self) -> MobilityDelta {
        self.tick += 1;
        let mut changed_agents = Vec::new();
        let mut changed_vehicle_ids = HashSet::new();

        for (agent_id, vehicle_id) in self.tick_boarding() {
            if let Some(agent) = self.agents.get(&agent_id) {
                changed_agents.push(agent.clone());
            }
            changed_vehicle_ids.insert(vehicle_id);
        }

        let agent_ids: Vec<AgentId> = self.agents.keys().cloned().collect();
        for agent_id in agent_ids {
            if self.tick_walking_agent(&agent_id)
                && let Some(agent) = self.agents.get(&agent_id)
            {
                changed_agents.push(agent.clone());
            }
        }

        for (agent_id, vehicle_id) in self.tick_alighting() {
            if let Some(agent) = self.agents.get(&agent_id) {
                changed_agents.push(agent.clone());
            }
            changed_vehicle_ids.insert(vehicle_id);
        }

        let vehicle_ids: Vec<VehicleId> = self.vehicles.keys().cloned().collect();
        for vehicle_id in vehicle_ids {
            if self.tick_vehicle(&vehicle_id) {
                changed_vehicle_ids.insert(vehicle_id);
            }
        }

        let mut changed_vehicle_ids: Vec<VehicleId> = changed_vehicle_ids.into_iter().collect();
        changed_vehicle_ids.sort_by(|left, right| left.0.cmp(&right.0));
        let changed_vehicles = changed_vehicle_ids
            .into_iter()
            .filter_map(|vehicle_id| self.vehicles.get(&vehicle_id).cloned())
            .collect();

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

        match agent.plan.get(agent.plan_cursor).cloned() {
            Some(PlanStage::WalkToStop { stop_id, .. }) => {
                agent.plan_cursor += 1;
                agent.state = AgentMobilityState::WaitingAtStop {
                    stop_id: stop_id.clone(),
                };

                if let Some(stop) = self.stops.get_mut(&stop_id)
                    && !stop.waiting_agents.contains(agent_id)
                {
                    stop.waiting_agents.push_back(agent_id.clone());
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
        if route.links.is_empty() || vehicle.progress >= 1.0 {
            return false;
        }

        vehicle.progress = (vehicle.progress + vehicle.speed_per_tick).min(1.0);
        true
    }

    fn tick_boarding(&mut self) -> Vec<(AgentId, VehicleId)> {
        let mut changed = Vec::new();
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
                    vehicle_id: vehicle_id.clone(),
                    seat_index,
                };
                changed.push((next_agent_id, vehicle_id));
            }
        }

        changed
    }

    fn tick_alighting(&mut self) -> Vec<(AgentId, VehicleId)> {
        let mut changed = Vec::new();
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
                    vehicle
                        .occupants
                        .retain(|occupant_id| occupant_id != &agent_id);
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
                    changed.push((agent_id, vehicle_id.clone()));
                }
            }
        }

        changed
    }

    fn resolve_link_polyline(
        &self,
        link_id: &LinkId,
    ) -> Option<crate::mobility_geometry::LinkGeometry> {
        if let Some(points) = self.link_polylines.get(link_id) {
            return Some(crate::mobility_geometry::LinkGeometry {
                points: points.clone(),
            });
        }
        crate::mobility_geometry::link_geometry(&link_id.0)
    }

    pub fn world_coord_for_agent(&self, agent_id: &AgentId) -> Option<(f32, f32)> {
        use crate::mobility_geometry::{activity_geometry, stop_geometry};
        let agent = self.agents.get(agent_id)?;
        match &agent.state {
            AgentMobilityState::AtActivity { activity_id } => {
                activity_geometry(activity_id).map(|g| g.coord)
            }
            AgentMobilityState::Walking { link_id, progress } => {
                let geom = self.resolve_link_polyline(link_id)?;
                Some(geom.world_coord_at_progress(*progress))
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

    pub fn direction_for_agent(
        &self,
        agent_id: &AgentId,
    ) -> Option<abutown_protocol::DirectionDto> {
        let agent = self.agents.get(agent_id)?;
        match &agent.state {
            AgentMobilityState::Walking { link_id, progress } => {
                let geom = self.resolve_link_polyline(link_id)?;
                Some(geom.direction_at_progress(*progress))
            }
            AgentMobilityState::InVehicle { vehicle_id, .. } => {
                self.direction_for_vehicle(vehicle_id)
            }
            _ => Some(abutown_protocol::DirectionDto::S),
        }
    }

    pub fn world_coord_for_vehicle(&self, vehicle_id: &VehicleId) -> Option<(f32, f32)> {
        let vehicle = self.vehicles.get(vehicle_id)?;
        let route = self.routes.get(&vehicle.route_id)?;
        let link_id = route.links.get(vehicle.link_index)?;
        let geom = self.resolve_link_polyline(link_id)?;
        Some(geom.world_coord_at_progress(vehicle.progress))
    }

    pub fn direction_for_vehicle(
        &self,
        vehicle_id: &VehicleId,
    ) -> Option<abutown_protocol::DirectionDto> {
        use crate::mobility_geometry::direction_from_delta;
        let vehicle = self.vehicles.get(vehicle_id)?;
        let route = self.routes.get(&vehicle.route_id)?;
        let link_id = route.links.get(vehicle.link_index)?;
        let geom = self.resolve_link_polyline(link_id)?;
        let here = geom.world_coord_at_progress(vehicle.progress);
        let ahead = geom.world_coord_at_progress((vehicle.progress + 0.1).min(1.0));
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

    /// Builds an AgentMobilityDto for the given agent id, including the computed
    /// world_coord / direction / sprite_key. Returns None if the agent does not exist.
    pub fn agent_dto_for(&self, agent_id: &AgentId) -> Option<abutown_protocol::AgentMobilityDto> {
        let agent = self.agents.get(agent_id)?;
        let (cx, cy) = self.world_coord_for_agent(agent_id).unwrap_or((0.0, 0.0));
        let direction = self
            .direction_for_agent(agent_id)
            .unwrap_or(abutown_protocol::DirectionDto::S);
        let sprite_key = self
            .sprite_key_for_agent(agent_id)
            .unwrap_or_else(|| "pedestrian:0".to_string());
        Some(abutown_protocol::AgentMobilityDto {
            id: abutown_protocol::EntityId(agent.id.0.clone()),
            state: abutown_protocol::AgentMobilityStateDto::from(&agent.state),
            plan_cursor: agent.plan_cursor,
            world_coord: abutown_protocol::WorldCoordDto { x: cx, y: cy },
            direction,
            sprite_key,
        })
    }

    pub fn vehicle_dto_for(
        &self,
        vehicle_id: &VehicleId,
    ) -> Option<abutown_protocol::VehicleMobilityDto> {
        let vehicle = self.vehicles.get(vehicle_id)?;
        let (cx, cy) = self
            .world_coord_for_vehicle(vehicle_id)
            .unwrap_or((0.0, 0.0));
        let direction = self
            .direction_for_vehicle(vehicle_id)
            .unwrap_or(abutown_protocol::DirectionDto::S);
        let sprite_key = self
            .sprite_key_for_vehicle(vehicle_id)
            .unwrap_or_else(|| "tram:0".to_string());
        Some(abutown_protocol::VehicleMobilityDto {
            id: abutown_protocol::EntityId(vehicle.id.0.clone()),
            kind: vehicle.kind.into(),
            route_id: vehicle.route_id.0.clone(),
            link_index: vehicle.link_index,
            progress: vehicle.progress,
            capacity: vehicle.capacity,
            occupants: vehicle
                .occupants
                .iter()
                .map(|agent_id| abutown_protocol::EntityId(agent_id.0.clone()))
                .collect(),
            dwell_ticks_remaining: vehicle.dwell_ticks_remaining,
            world_coord: abutown_protocol::WorldCoordDto { x: cx, y: cy },
            direction,
            sprite_key,
        })
    }
}

impl From<&AgentRecord> for AgentMobilityDto {
    fn from(value: &AgentRecord) -> Self {
        // Placeholder values for world_coord, direction, sprite_key.
        // Task 3 of the visible-backend-mobility plan replaces this `From`
        // path with a `MobilityWorld`-aware builder that computes the real
        // coordinates and sprite hints per tick.
        Self {
            id: EntityId(value.id.0.clone()),
            state: AgentMobilityStateDto::from(&value.state),
            plan_cursor: value.plan_cursor,
            world_coord: abutown_protocol::WorldCoordDto { x: 0.0, y: 0.0 },
            direction: abutown_protocol::DirectionDto::S,
            sprite_key: String::from("pedestrian:0"),
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
        // Placeholder values for world_coord, direction, sprite_key — Task 3
        // replaces this `From` path with a `MobilityWorld`-aware builder.
        Self {
            id: EntityId(value.id.0.clone()),
            kind: value.kind.into(),
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
            world_coord: abutown_protocol::WorldCoordDto { x: 0.0, y: 0.0 },
            direction: abutown_protocol::DirectionDto::S,
            sprite_key: String::from("tram:0"),
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
    world: &MobilityWorld,
) -> MobilitySnapshotDto {
    let mut agent_ids: Vec<AgentId> = world.agents.keys().cloned().collect();
    agent_ids.sort_by(|left, right| left.0.cmp(&right.0));
    let agents = agent_ids
        .iter()
        .filter_map(|id| world.agent_dto_for(id))
        .collect();

    let mut vehicle_ids: Vec<VehicleId> = world.vehicles.keys().cloned().collect();
    vehicle_ids.sort_by(|left, right| left.0.cmp(&right.0));
    let vehicles = vehicle_ids
        .iter()
        .filter_map(|id| world.vehicle_dto_for(id))
        .collect();

    let mut stops: Vec<StopMobilityDto> = world.stops.values().map(StopMobilityDto::from).collect();
    stops.sort_by(|left, right| left.id.cmp(&right.id));

    MobilitySnapshotDto {
        protocol_version: PROTOCOL_VERSION,
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
) -> MobilityDeltaDto {
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
    MobilityDeltaDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: world_id.clone(),
        tick,
        changed_agents,
        changed_vehicles,
        left_agents: vec![],
        left_vehicles: vec![],
    }
}

pub mod seed {
    use std::collections::{HashMap, VecDeque};

    use super::{
        AgentMobilityState, AgentRecord, MobilityWorld, PlanStage, RouteRecord, StopRecord,
        VehicleKind, VehicleRecord,
    };
    use crate::ids::{AgentId, LinkId, RouteId, StopId, VehicleId};

    /// Backward-compatible wrapper — delegates to [`tiny_world`].
    pub fn initial_world() -> MobilityWorld {
        tiny_world()
    }

    /// Build a deterministic populated mobility world for fresh server starts.
    ///
    /// Two routes traverse the seeded chunk neighbourhood; 4 vehicles and
    /// 20 agents are spawned with cyclic plans. Calling this function twice
    /// returns equal worlds.
    pub fn tiny_world() -> MobilityWorld {
        let horizontal_route = RouteId("route:horizontal".to_string());
        let vertical_route = RouteId("route:vertical".to_string());
        let horizontal_link = LinkId("link:horizontal:main".to_string());
        let vertical_link = LinkId("link:vertical:main".to_string());

        let horizontal_pickup = StopId("stop:horizontal:pickup".to_string());
        let horizontal_dropoff = StopId("stop:horizontal:dropoff".to_string());
        let vertical_pickup = StopId("stop:vertical:pickup".to_string());
        let vertical_dropoff = StopId("stop:vertical:dropoff".to_string());

        let walk_link = LinkId("link:walk:default".to_string());
        let work_activity = "activity:work".to_string();

        let mut routes = HashMap::new();
        routes.insert(
            horizontal_route.clone(),
            RouteRecord {
                id: horizontal_route.clone(),
                links: vec![horizontal_link.clone()],
            },
        );
        routes.insert(
            vertical_route.clone(),
            RouteRecord {
                id: vertical_route.clone(),
                links: vec![vertical_link.clone()],
            },
        );

        let mut stops = HashMap::new();
        for (stop_id, route_id, progress) in [
            (&horizontal_pickup, &horizontal_route, 0.0_f32),
            (&horizontal_dropoff, &horizontal_route, 1.0_f32),
            (&vertical_pickup, &vertical_route, 0.0_f32),
            (&vertical_dropoff, &vertical_route, 1.0_f32),
        ] {
            stops.insert(
                stop_id.clone(),
                StopRecord {
                    id: stop_id.clone(),
                    route_id: route_id.clone(),
                    link_index: 0,
                    progress,
                    waiting_agents: VecDeque::new(),
                },
            );
        }

        let mut vehicles = HashMap::new();
        for offset in 0..4u32 {
            let route_id = if offset % 2 == 0 {
                horizontal_route.clone()
            } else {
                vertical_route.clone()
            };
            let vehicle_id = VehicleId(format!("vehicle:seed:{offset}"));
            vehicles.insert(
                vehicle_id.clone(),
                VehicleRecord {
                    id: vehicle_id,
                    kind: VehicleKind::Tram,
                    route_id,
                    link_index: 0,
                    progress: (offset as f32) * 0.25,
                    speed_per_tick: 0.1,
                    capacity: 4,
                    occupants: Vec::new(),
                    dwell_ticks_remaining: 0,
                },
            );
        }

        let mut agents = HashMap::new();
        for offset in 0..20u32 {
            let agent_id = AgentId(format!("agent:seed:{offset}"));
            let (pickup, dropoff, route_id) = if offset % 2 == 0 {
                (&horizontal_pickup, &horizontal_dropoff, &horizontal_route)
            } else {
                (&vertical_pickup, &vertical_dropoff, &vertical_route)
            };

            agents.insert(
                agent_id.clone(),
                AgentRecord {
                    id: agent_id,
                    state: AgentMobilityState::Walking {
                        link_id: walk_link.clone(),
                        progress: (offset as f32) * 0.05,
                    },
                    plan: vec![
                        PlanStage::WalkToStop {
                            link_id: walk_link.clone(),
                            stop_id: pickup.clone(),
                        },
                        PlanStage::RideToStop {
                            route_id: route_id.clone(),
                            stop_id: dropoff.clone(),
                        },
                        PlanStage::WalkToActivity {
                            link_id: walk_link.clone(),
                            activity_id: work_activity.clone(),
                        },
                        PlanStage::Activity {
                            activity_id: work_activity.clone(),
                        },
                    ],
                    plan_cursor: 0,
                    walk_speed_per_tick: 0.5,
                },
            );
        }

        MobilityWorld {
            tick: 0,
            agents,
            vehicles,
            stops,
            routes,
            link_polylines: HashMap::new(),
        }
    }

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
            let points: Vec<(f32, f32)> = corridor
                .iter()
                .map(|NetworkCoord { x, y }| (*x as f32, *y as f32))
                .collect();
            links.push((link_id, points));
        }

        // Spawn walking agents distributed across corridors.
        if !network.pedestrian_corridors.is_empty() {
            let pedestrian_count =
                network.pedestrian_corridors.len() as u32 * density.pedestrians_per_corridor;
            for n in 0..pedestrian_count {
                let corridor_index = (n as usize) % network.pedestrian_corridors.len();
                let agent_id = AgentId(format!("agent:walk:{n}"));
                let link_id = LinkId(format!("link:walk:corridor:{corridor_index}"));
                let progress = ((n as f32) / (density.pedestrians_per_corridor as f32)).fract();
                world.agents.insert(
                    agent_id.clone(),
                    AgentRecord {
                        id: agent_id,
                        state: AgentMobilityState::Walking { link_id, progress },
                        plan: vec![PlanStage::Activity {
                            activity_id: format!("activity:wander:{corridor_index}"),
                        }],
                        plan_cursor: 0,
                        walk_speed_per_tick: 0.05,
                    },
                );
            }
        }

        // Register arterial paths as routes for cars (one route per arterial, one link).
        for (index, arterial) in network.arterial_paths.iter().enumerate() {
            let route_id = RouteId(format!("route:arterial:{index}"));
            let link_id = LinkId(format!("link:arterial:{index}"));
            let points: Vec<(f32, f32)> = arterial
                .iter()
                .map(|NetworkCoord { x, y }| (*x as f32, *y as f32))
                .collect();
            links.push((link_id.clone(), points));
            routes.insert(
                route_id.clone(),
                RouteRecord {
                    id: route_id.clone(),
                    links: vec![link_id],
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
                        route_id,
                        link_index: 0,
                        progress: if density.cars_per_arterial > 0 {
                            (n as f32) / (density.cars_per_arterial as f32)
                        } else {
                            0.0
                        },
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
                            vehicle_id,
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

        // Trams: reuse the existing tiny_world tram vehicles, routes, and stops.
        // We do NOT copy the tiny_world pedestrian agents — they belong to
        // the tiny seeded world only; this world seeds its own walkers above.
        if density.trams_total > 0 {
            let tram_seed = tiny_world();
            for vehicle in tram_seed.vehicles.values() {
                world.vehicles.insert(vehicle.id.clone(), vehicle.clone());
            }
            for (id, record) in &tram_seed.routes {
                routes.insert(id.clone(), record.clone());
            }
            for stop in tram_seed.stops.values() {
                world.stops.insert(stop.id.clone(), stop.clone());
            }
        }

        world.routes = routes;
        world.link_polylines = links.into_iter().collect();
        world
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{AgentId, LinkId, RouteId, StopId, VehicleId};

    #[test]
    fn initial_world_seeds_expected_population() {
        let world = seed::initial_world();

        assert_eq!(world.tick(), 0);
        assert_eq!(world.routes.len(), 2, "expected 2 routes");

        let snapshot = world.snapshot();
        assert_eq!(snapshot.stops.len(), 4, "expected 4 stops");
        assert_eq!(snapshot.vehicles.len(), 4, "expected 4 vehicles");
        assert_eq!(snapshot.agents.len(), 20, "expected 20 agents");

        for agent in &snapshot.agents {
            assert!(
                !agent.plan.is_empty(),
                "every agent must have at least one plan stage"
            );
        }
        for vehicle in &snapshot.vehicles {
            assert!(vehicle.capacity > 0, "vehicle capacity must be positive");
        }
    }

    #[test]
    fn initial_world_is_deterministic() {
        let a = seed::initial_world();
        let b = seed::initial_world();
        assert_eq!(a, b, "initial_world() must be deterministic across calls");
    }

    #[test]
    fn sample_world_starts_with_agent_walking_to_pickup_stop() {
        let world = sample_world();
        let agent = world
            .agent(&AgentId("agent:pedestrian:0".to_string()))
            .expect("sample agent exists");
        let vehicle = world
            .vehicle(&VehicleId("vehicle:shuttle:0".to_string()))
            .expect("sample vehicle exists");
        let stop = world
            .stop(&StopId("stop:old-town".to_string()))
            .expect("sample stop exists");

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

    #[test]
    fn walking_agent_reaches_pickup_stop_and_waits() {
        let mut world = sample_world();
        let agent_id = AgentId("agent:pedestrian:0".to_string());

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
        assert_eq!(
            stop.waiting_agents.iter().cloned().collect::<Vec<_>>(),
            vec![agent_id]
        );
        assert_eq!(second_delta.changed_agents.len(), 1);
    }

    #[test]
    fn vehicle_respects_initial_dwell_then_moves_on_route() {
        let mut world = sample_world();
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

    #[test]
    fn agent_boards_rides_alights_and_walks_to_activity() {
        let mut world = sample_world();
        let agent_id = AgentId("agent:pedestrian:0".to_string());
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

    #[test]
    fn mobility_world_serde_round_trip_preserves_state() {
        let original = sample_world();
        let json = serde_json::to_value(&original).expect("serialize");
        let restored: MobilityWorld = serde_json::from_value(json).expect("deserialize");
        assert_eq!(restored, original);
    }

    fn sample_world() -> MobilityWorld {
        let route_id = RouteId("route:old-town-loop".to_string());
        let pickup_stop_id = StopId("stop:old-town".to_string());
        let dropoff_stop_id = StopId("stop:station".to_string());
        let walk_to_pickup = LinkId("link:home-to-old-town-stop".to_string());
        let vehicle_link = LinkId("link:old-town-to-station".to_string());
        let walk_to_activity = LinkId("link:station-to-work".to_string());
        let agent_id = AgentId("agent:pedestrian:0".to_string());
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
                kind: VehicleKind::Tram,
                route_id,
                link_index: 0,
                progress: 0.0,
                speed_per_tick: 0.5,
                capacity: 4,
                occupants: Vec::new(),
                dwell_ticks_remaining: 2,
            },
        );

        MobilityWorld {
            tick: 0,
            agents,
            vehicles,
            stops,
            routes,
            link_polylines: HashMap::new(),
        }
    }

    #[test]
    fn world_coord_for_walking_agent_interpolates_link() {
        use crate::mobility_geometry::link_geometry;

        let mut world = seed::initial_world();
        let agent_id = AgentId("agent:seed:0".to_string());
        if let Some(agent) = world.agents.get_mut(&agent_id) {
            agent.state = AgentMobilityState::Walking {
                link_id: LinkId("link:walk:default".to_string()),
                progress: 0.5,
            };
        }

        let geom = link_geometry("link:walk:default").unwrap();
        let coord = world
            .world_coord_for_agent(&agent_id)
            .expect("agent resolves to coord");
        let expected = geom.world_coord_at_progress(0.5);
        assert!((coord.0 - expected.0).abs() < 0.01);
        assert!((coord.1 - expected.1).abs() < 0.01);
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
        let coord = world
            .world_coord_for_vehicle(&vehicle_id)
            .expect("vehicle resolves");
        assert!((coord.0 - (4.0 * 32.0 + 16.0 + 16.0)).abs() < 0.01);
    }

    #[test]
    fn sprite_key_for_agent_is_deterministic_by_id_hash() {
        let world = seed::initial_world();
        let a = world
            .sprite_key_for_agent(&AgentId("agent:seed:0".to_string()))
            .unwrap();
        let b = world
            .sprite_key_for_agent(&AgentId("agent:seed:0".to_string()))
            .unwrap();
        assert_eq!(
            a, b,
            "sprite key must be deterministic across calls for the same id"
        );
        assert!(a.starts_with("pedestrian:"));
    }

    #[test]
    fn agent_dto_built_through_world_includes_world_coord_direction_and_sprite_key() {
        let world = seed::initial_world();
        let agent_id = AgentId("agent:seed:0".to_string());
        let dto = world.agent_dto_for(&agent_id).expect("agent exists");
        assert!(dto.sprite_key.starts_with("pedestrian:"));
        assert!(dto.world_coord.x.is_finite());
    }

    #[test]
    fn seeded_world_vehicles_default_to_tram_kind() {
        let world = seed::initial_world();
        for vehicle in world.vehicles.values() {
            assert_eq!(vehicle.kind, VehicleKind::Tram);
        }
    }

    #[test]
    fn from_network_produces_expected_population_counts() {
        use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};

        let network = CityNetwork {
            version: 1,
            world_id: "test".to_string(),
            chunk_size: 32,
            world_tiles: WorldTiles {
                width: 256,
                height: 256,
            },
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

        let walking_agents = world
            .agents
            .values()
            .filter(|a| matches!(a.state, AgentMobilityState::Walking { .. }))
            .count();
        let driving_agents = world
            .agents
            .values()
            .filter(|a| matches!(a.state, AgentMobilityState::InVehicle { .. }))
            .count();
        let cars = world
            .vehicles
            .values()
            .filter(|v| v.kind == VehicleKind::Car)
            .count();
        let trams = world
            .vehicles
            .values()
            .filter(|v| v.kind == VehicleKind::Tram)
            .count();

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
            world_tiles: WorldTiles {
                width: 256,
                height: 256,
            },
            arterial_paths: vec![vec![
                NetworkCoord { x: 0, y: 0 },
                NetworkCoord { x: 10, y: 0 },
            ]],
            pedestrian_corridors: vec![vec![
                NetworkCoord { x: 0, y: 5 },
                NetworkCoord { x: 10, y: 5 },
            ]],
        };
        let density = seed::SeedDensity {
            pedestrians_per_corridor: 3,
            cars_per_arterial: 2,
            trams_total: 0,
        };
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
            world_tiles: WorldTiles {
                width: 256,
                height: 256,
            },
            arterial_paths: vec![vec![
                NetworkCoord { x: 0, y: 0 },
                NetworkCoord { x: 10, y: 0 },
            ]],
            pedestrian_corridors: vec![],
        };
        let density = seed::SeedDensity {
            pedestrians_per_corridor: 0,
            cars_per_arterial: 2,
            trams_total: 0,
        };
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

    #[test]
    fn delta_dto_excludes_in_vehicle_agents() {
        use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};
        let network = CityNetwork {
            version: 1,
            world_id: "test".to_string(),
            chunk_size: 32,
            world_tiles: WorldTiles {
                width: 256,
                height: 256,
            },
            arterial_paths: vec![vec![
                NetworkCoord { x: 0, y: 0 },
                NetworkCoord { x: 10, y: 0 },
            ]],
            pedestrian_corridors: vec![vec![
                NetworkCoord { x: 0, y: 5 },
                NetworkCoord { x: 10, y: 5 },
            ]],
        };
        let density = seed::SeedDensity {
            pedestrians_per_corridor: 2,
            cars_per_arterial: 2,
            trams_total: 0,
        };
        let world = seed::from_network(&network, density);
        let drivers: Vec<_> = world
            .agents
            .values()
            .filter(|a| matches!(a.state, AgentMobilityState::InVehicle { .. }))
            .collect();
        assert!(
            !drivers.is_empty(),
            "test setup should contain at least one in_vehicle driver agent"
        );

        let world_id = WorldId("test".to_string());
        let delta_input = MobilityDelta {
            changed_agents: world.agents.values().cloned().collect(),
            changed_vehicles: vec![],
        };
        let dto = build_mobility_delta_dto(&world_id, world.tick(), &world, &delta_input);
        for agent in &dto.changed_agents {
            if let abutown_protocol::AgentMobilityStateDto::InVehicle { .. } = &agent.state {
                panic!("in_vehicle agent leaked into delta: {}", agent.id.0);
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
            world_tiles: WorldTiles {
                width: 256,
                height: 256,
            },
            arterial_paths: vec![vec![
                NetworkCoord { x: 0, y: 0 },
                NetworkCoord { x: 10, y: 0 },
            ]],
            pedestrian_corridors: vec![],
        };
        let density = seed::SeedDensity {
            pedestrians_per_corridor: 0,
            cars_per_arterial: 2,
            trams_total: 0,
        };
        let world = seed::from_network(&network, density);
        let world_id = WorldId("test".to_string());
        let snap = build_mobility_snapshot_dto(&world_id, world.tick(), &world);
        assert_eq!(
            snap.agents.len(),
            2,
            "snapshot must include in_vehicle drivers so clients can hydrate state"
        );
    }
}
