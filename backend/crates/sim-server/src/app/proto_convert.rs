// ===== legacy serde DTO → proto wire helpers =====
//
// These convert the runtime's existing serde-DTO API surface into the
// protobuf wire types now used on the WS hot path. They are internal
// to sim-server; the legacy DTOs in `abutown_protocol::*` survive
// until Task 7 because HTTP / DB persistence still consume them.

use abutown_protocol::v1 as w;

pub(crate) fn direction_to_proto(d: abutown_protocol::DirectionDto) -> w::Direction {
    use abutown_protocol::DirectionDto as L;
    match d {
        L::N => w::Direction::N,
        L::Ne => w::Direction::Ne,
        L::E => w::Direction::E,
        L::Se => w::Direction::Se,
        L::S => w::Direction::S,
        L::Sw => w::Direction::Sw,
        L::W => w::Direction::W,
        L::Nw => w::Direction::Nw,
    }
}

pub(crate) fn tile_kind_to_proto(k: abutown_protocol::TileKindDto) -> w::TileKind {
    use abutown_protocol::TileKindDto as L;
    match k {
        L::Grass => w::TileKind::Grass,
        L::Water => w::TileKind::Water,
        L::Road => w::TileKind::Road,
        L::BuildingFootprint => w::TileKind::BuildingFootprint,
    }
}

pub(crate) fn chunk_state_to_proto(s: abutown_protocol::ChunkStateDto) -> w::ChunkState {
    use abutown_protocol::ChunkStateDto as L;
    match s {
        L::Asleep => w::ChunkState::Asleep,
        L::Warm => w::ChunkState::Warm,
        L::Active => w::ChunkState::Active,
        L::Hot => w::ChunkState::Hot,
    }
}

pub(crate) fn agent_dto_to_proto(dto: abutown_protocol::AgentMobilityDto) -> w::AgentMobility {
    use abutown_protocol::AgentMobilityStateDto as Legacy;
    let state = match dto.state {
        Legacy::Walking { link_id, progress } => {
            w::agent_state::State::Walking(w::Walking { link_id, progress })
        }
        Legacy::WaitingAtStop { stop_id } => {
            w::agent_state::State::WaitingAtStop(w::WaitingAtStop { stop_id })
        }
        Legacy::InVehicle {
            vehicle_id,
            seat_index,
        } => w::agent_state::State::InVehicle(w::InVehicle {
            vehicle_id: vehicle_id.0,
            seat_index: seat_index as u32,
        }),
        Legacy::Boarding {
            vehicle_id,
            stop_id,
        } => w::agent_state::State::Boarding(w::Boarding {
            vehicle_id: vehicle_id.0,
            stop_id,
        }),
        Legacy::Alighting {
            vehicle_id,
            stop_id,
        } => w::agent_state::State::Alighting(w::Alighting {
            vehicle_id: vehicle_id.0,
            stop_id,
        }),
        Legacy::AtActivity { activity_id } => {
            w::agent_state::State::AtActivity(w::AtActivity { activity_id })
        }
    };
    w::AgentMobility {
        id: dto.id.0,
        state: Some(w::AgentState { state: Some(state) }),
        world_coord: Some(w::WorldCoord {
            x: dto.world_coord.x,
            y: dto.world_coord.y,
        }),
        direction: direction_to_proto(dto.direction) as i32,
        sprite_key: dto.sprite_key,
        plan_cursor: dto.plan_cursor as u32,
        age_seconds: dto.age_seconds,
    }
}

pub(crate) fn vehicle_dto_to_proto(
    dto: abutown_protocol::VehicleMobilityDto,
) -> w::VehicleMobility {
    use abutown_protocol::VehicleKindDto;
    let kind = match dto.kind {
        VehicleKindDto::Car => w::VehicleKind::Car,
    };
    w::VehicleMobility {
        id: dto.id.0,
        kind: kind as i32,
        route_id: dto.route_id,
        link_index: dto.link_index as u32,
        progress: dto.progress,
        capacity: dto.capacity as u32,
        occupants: dto.occupants.into_iter().map(|e| e.0).collect(),
        dwell_ticks_remaining: dto.dwell_ticks_remaining as u32,
        world_coord: Some(w::WorldCoord {
            x: dto.world_coord.x,
            y: dto.world_coord.y,
        }),
        direction: direction_to_proto(dto.direction) as i32,
        sprite_key: dto.sprite_key,
    }
}

pub(crate) fn stop_dto_to_proto(s: &abutown_protocol::StopMobilityDto) -> w::Stop {
    w::Stop {
        id: s.id.clone(),
        route_id: s.route_id.clone(),
        link_index: s.link_index as u32,
        progress: s.progress,
        waiting_agents: s.waiting_agents.iter().map(|e| e.0.clone()).collect(),
    }
}

pub(crate) fn world_summary_dto_to_proto(s: &abutown_protocol::WorldSummaryDto) -> w::WorldSummary {
    w::WorldSummary {
        protocol_version: u32::from(s.protocol_version),
        world_id: s.world_id.0.clone(),
        chunk_size: u32::from(s.chunk_size),
        loaded_chunks: s
            .loaded_chunks
            .iter()
            .map(|c| w::ChunkCoord { x: c.x, y: c.y })
            .collect(),
        tick_period_ms: s.tick_period_ms,
        sim_time: s.sim_time,
    }
}

pub(crate) fn health_dto_to_proto(h: &abutown_protocol::HealthResponse) -> w::HealthResponse {
    w::HealthResponse {
        protocol_version: u32::from(h.protocol_version),
        service: h.service.clone(),
        world_id: h.world_id.0.clone(),
        ok: h.ok,
        persistence: None,
    }
}

pub(crate) fn chunk_snapshot_dto_to_proto(
    c: &abutown_protocol::ChunkSnapshotDto,
) -> w::ChunkSnapshot {
    w::ChunkSnapshot {
        protocol_version: u32::from(c.protocol_version),
        world_id: c.world_id.0.clone(),
        coord: Some(w::ChunkCoord {
            x: c.coord.x,
            y: c.coord.y,
        }),
        chunk_version: c.chunk_version,
        chunk_state: chunk_state_to_proto(c.chunk_state) as i32,
        tile_count: u32::from(c.tile_count),
        tiles: c
            .tiles
            .iter()
            .map(|t| w::TileMutation {
                local_index: u32::from(t.local_index),
                kind: tile_kind_to_proto(t.kind) as i32,
                version: t.version,
            })
            .collect(),
    }
}

pub(crate) fn mobility_snapshot_dto_to_proto(
    s: &abutown_protocol::MobilitySnapshotDto,
) -> w::MobilitySnapshot {
    w::MobilitySnapshot {
        protocol_version: u32::from(s.protocol_version),
        world_id: s.world_id.0.clone(),
        tick: s.tick,
        agents: s.agents.iter().cloned().map(agent_dto_to_proto).collect(),
        vehicles: s
            .vehicles
            .iter()
            .cloned()
            .map(vehicle_dto_to_proto)
            .collect(),
        stops: s.stops.iter().map(stop_dto_to_proto).collect(),
    }
}

pub(crate) fn tile_pulse_dto_to_proto(p: &abutown_protocol::TilePulseDeltaDto) -> w::TilePulse {
    w::TilePulse {
        protocol_version: u32::from(p.protocol_version),
        world_id: p.world_id.0.clone(),
        tick: p.tick,
        version: p.version,
        coord: Some(w::ChunkCoord {
            x: p.coord.x,
            y: p.coord.y,
        }),
        local_index: u32::from(p.local_index),
    }
}

pub(crate) fn world_event_dto_to_proto(e: &abutown_protocol::WorldEventDto) -> w::WorldEvent {
    use abutown_protocol::WorldEventDto as L;
    match e {
        L::TileKindSet(tk) => w::WorldEvent {
            event: Some(w::world_event::Event::TileKindSet(w::TileKindSetEvent {
                protocol_version: u32::from(tk.protocol_version),
                event_id: tk.event_id.clone(),
                command_id: tk.command_id.clone(),
                world_id: tk.world_id.0.clone(),
                tick: tk.tick,
                version: tk.version,
                coord: Some(w::ChunkCoord {
                    x: tk.coord.x,
                    y: tk.coord.y,
                }),
                local_index: u32::from(tk.local_index),
                kind: tile_kind_to_proto(tk.kind) as i32,
            })),
        },
    }
}
