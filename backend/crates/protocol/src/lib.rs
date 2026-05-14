use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorldId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkCoordDto {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkStateDto {
    Asleep,
    Warm,
    Active,
    Hot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TileKindDto {
    Grass,
    Water,
    Road,
    BuildingFootprint,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub service: String,
    pub world_id: WorldId,
    pub ok: bool,
    pub protocol_version: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSummaryDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub chunk_size: u16,
    pub loaded_chunks: Vec<ChunkCoordDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TileMutationDto {
    pub local_index: u16,
    pub kind: TileKindDto,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkSnapshotDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub coord: ChunkCoordDto,
    pub chunk_state: ChunkStateDto,
    pub chunk_version: u64,
    pub tile_count: u16,
    pub dirty_tiles: Vec<TileMutationDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessageDto {
    Hello(ServerHelloDto),
    TilePulse(TilePulseDeltaDto),
    MobilityDelta(MobilityDeltaDto),
    Error(ServerErrorDto),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerHelloDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub chunk_size: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TilePulseDeltaDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub tick: u64,
    pub version: u64,
    pub coord: ChunkCoordDto,
    pub local_index: u16,
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerErrorDto {
    pub protocol_version: u16,
    pub world_id: Option<WorldId>,
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_response_serializes_versioned_world() {
        let response = HealthResponse {
            service: "abutown-sim".to_string(),
            world_id: WorldId("abutown-main".to_string()),
            ok: true,
            protocol_version: PROTOCOL_VERSION,
        };

        let json = serde_json::to_string(&response).expect("health response serializes");

        assert_eq!(
            json,
            r#"{"service":"abutown-sim","world_id":"abutown-main","ok":true,"protocol_version":1}"#
        );
    }

    #[test]
    fn websocket_hello_serializes_with_type_tag() {
        let message = ServerMessageDto::Hello(ServerHelloDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            chunk_size: 32,
        });

        let json = serde_json::to_string(&message).expect("hello serializes");

        assert_eq!(
            json,
            r#"{"type":"hello","protocol_version":1,"world_id":"abutown-main","chunk_size":32}"#
        );
    }

    #[test]
    fn websocket_tile_pulse_serializes_chunk_and_version() {
        let message = ServerMessageDto::TilePulse(TilePulseDeltaDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            tick: 7,
            version: 11,
            coord: ChunkCoordDto { x: 0, y: 0 },
            local_index: 231,
        });

        let json = serde_json::to_string(&message).expect("tile pulse serializes");

        assert_eq!(
            json,
            r#"{"type":"tile_pulse","protocol_version":1,"world_id":"abutown-main","tick":7,"version":11,"coord":{"x":0,"y":0},"local_index":231}"#
        );
    }

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
        assert_eq!(
            json["changed_agents"][0]["state"]["type"],
            "waiting_at_stop"
        );
        assert_eq!(
            json["changed_agents"][0]["state"]["stop_id"],
            "stop:old-town"
        );
    }
}
