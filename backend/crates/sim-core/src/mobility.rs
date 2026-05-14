use std::collections::{HashMap, HashSet, VecDeque};

use abutown_protocol::{
    AgentMobilityDto, AgentMobilityStateDto, EntityId, MobilityDeltaDto, MobilitySnapshotDto,
    PROTOCOL_VERSION, StopMobilityDto, VehicleMobilityDto, WorldId,
};

use crate::ids::{AgentId, LinkId, RouteId, StopId, VehicleId};

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
        assert_eq!(
            stop.waiting_agents.iter().cloned().collect::<Vec<_>>(),
            vec![agent_id]
        );
        assert_eq!(second_delta.changed_agents.len(), 1);
    }

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
}
