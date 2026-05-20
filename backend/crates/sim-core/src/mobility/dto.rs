use abutown_protocol::{
    AgentMobilityDto, AgentMobilityStateDto, EntityId, MobilitySnapshotDto, PROTOCOL_VERSION,
    StopMobilityDto, VehicleMobilityDto, WorldId,
};

use bevy_ecs::world::World;

use crate::mobility::records::*;

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
    world: &World,
) -> MobilitySnapshotDto {
    let agents = crate::mobility::api::agents(world)
        .iter()
        .filter_map(|record| crate::mobility::api::agent_dto_for(world, &record.id))
        .collect();

    let vehicles = crate::mobility::api::vehicles(world)
        .iter()
        .filter_map(|record| crate::mobility::api::vehicle_dto_for(world, &record.id))
        .collect();

    let mut stops: Vec<StopMobilityDto> = crate::mobility::api::stops(world)
        .iter()
        .map(StopMobilityDto::from)
        .collect();
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
