use abutown_protocol::{
    AgentMobilityDto, AgentMobilityStateDto, EntityId, MobilityDeltaDto, MobilitySnapshotDto,
    PROTOCOL_VERSION, StopMobilityDto, VehicleMobilityDto, WorldId,
};

use crate::ids::{AgentId, VehicleId};
use crate::mobility::records::*;

use super::MobilityWorld;

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
    let mut agent_records = world.agents();
    agent_records.sort_by(|left, right| left.id.0.cmp(&right.id.0));
    let agents = agent_records
        .iter()
        .filter_map(|record| world.agent_dto_for(&record.id))
        .collect();

    let mut vehicle_records = world.vehicles();
    vehicle_records.sort_by(|left, right| left.id.0.cmp(&right.id.0));
    let vehicles = vehicle_records
        .iter()
        .filter_map(|record| world.vehicle_dto_for(&record.id))
        .collect();

    let mut stops: Vec<StopMobilityDto> = world.stops().iter().map(StopMobilityDto::from).collect();
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

pub fn build_filtered_mobility_delta_dto(
    world_id: &WorldId,
    tick: u64,
    world: &MobilityWorld,
    delta: &MobilityDelta,
    subscription: &std::collections::HashSet<crate::ids::ChunkCoord>,
    last_visible_agents: &mut std::collections::HashSet<abutown_protocol::EntityId>,
    last_visible_vehicles: &mut std::collections::HashSet<abutown_protocol::EntityId>,
) -> abutown_protocol::MobilityDeltaDto {
    use super::chunk_of;
    const CHUNK_SIZE: u16 = 32;

    let mut current_visible_agents: std::collections::HashSet<abutown_protocol::EntityId> =
        std::collections::HashSet::new();
    for agent in world.agents() {
        if matches!(agent.state, AgentMobilityState::InVehicle { .. }) {
            continue;
        }
        if let Some((x, y)) = world.world_coord_for_agent(&agent.id)
            && subscription.contains(&chunk_of(x, y, CHUNK_SIZE))
        {
            current_visible_agents.insert(abutown_protocol::EntityId(agent.id.0.clone()));
        }
    }
    let mut current_visible_vehicles: std::collections::HashSet<abutown_protocol::EntityId> =
        std::collections::HashSet::new();
    for vehicle in world.vehicles() {
        if let Some((x, y)) = world.world_coord_for_vehicle(&vehicle.id)
            && subscription.contains(&chunk_of(x, y, CHUNK_SIZE))
        {
            current_visible_vehicles.insert(abutown_protocol::EntityId(vehicle.id.0.clone()));
        }
    }

    // entered_*: newly visible — emit full DTO
    let mut changed_agents: Vec<abutown_protocol::AgentMobilityDto> = Vec::new();
    for entity_id in current_visible_agents.iter() {
        if !last_visible_agents.contains(entity_id)
            && let Some(dto) = world.agent_dto_for(&AgentId(entity_id.0.clone()))
        {
            changed_agents.push(dto);
        }
    }
    let mut changed_vehicles: Vec<abutown_protocol::VehicleMobilityDto> = Vec::new();
    for entity_id in current_visible_vehicles.iter() {
        if !last_visible_vehicles.contains(entity_id)
            && let Some(dto) = world.vehicle_dto_for(&VehicleId(entity_id.0.clone()))
        {
            changed_vehicles.push(dto);
        }
    }

    // still-visible-changed: in delta AND already known to this connection
    for agent in &delta.changed_agents {
        let entity_id = abutown_protocol::EntityId(agent.id.0.clone());
        if current_visible_agents.contains(&entity_id)
            && last_visible_agents.contains(&entity_id)
            && let Some(dto) = world.agent_dto_for(&agent.id)
        {
            changed_agents.push(dto);
        }
    }
    for vehicle in &delta.changed_vehicles {
        let entity_id = abutown_protocol::EntityId(vehicle.id.0.clone());
        if current_visible_vehicles.contains(&entity_id)
            && last_visible_vehicles.contains(&entity_id)
            && let Some(dto) = world.vehicle_dto_for(&vehicle.id)
        {
            changed_vehicles.push(dto);
        }
    }

    // left_*: previously visible, now not
    let left_agents: Vec<abutown_protocol::EntityId> = last_visible_agents
        .iter()
        .filter(|id| !current_visible_agents.contains(id))
        .cloned()
        .collect();
    let left_vehicles: Vec<abutown_protocol::EntityId> = last_visible_vehicles
        .iter()
        .filter(|id| !current_visible_vehicles.contains(id))
        .cloned()
        .collect();

    *last_visible_agents = current_visible_agents;
    *last_visible_vehicles = current_visible_vehicles;

    abutown_protocol::MobilityDeltaDto {
        protocol_version: abutown_protocol::PROTOCOL_VERSION,
        world_id: world_id.clone(),
        tick,
        changed_agents,
        changed_vehicles,
        left_agents,
        left_vehicles,
    }
}
