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
pub enum ClientCommandDto {
    SetTileKind(SetTileKindCommandDto),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetTileKindCommandDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub command_id: String,
    pub coord: ChunkCoordDto,
    pub local_index: u16,
    pub kind: TileKindDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CommandResponseDto {
    Accepted(CommandAcceptedDto),
    Rejected(CommandRejectedDto),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandAcceptedDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub command_id: String,
    pub event: WorldEventDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRejectedDto {
    pub protocol_version: u16,
    pub world_id: Option<WorldId>,
    pub command_id: Option<String>,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorldEventDto {
    TileKindSet(TileKindSetEventDto),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TileKindSetEventDto {
    pub protocol_version: u16,
    pub event_id: String,
    pub command_id: String,
    pub world_id: WorldId,
    pub tick: u64,
    pub version: u64,
    pub coord: ChunkCoordDto,
    pub local_index: u16,
    pub kind: TileKindDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessageDto {
    Hello(ServerHelloDto),
    TilePulse(TilePulseDeltaDto),
    MobilityDelta(MobilityDeltaDto),
    WorldEvent { event: WorldEventDto },
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
    AtActivity {
        activity_id: String,
    },
    Walking {
        link_id: String,
        progress: f32,
    },
    WaitingAtStop {
        stop_id: String,
    },
    Boarding {
        vehicle_id: EntityId,
        stop_id: String,
    },
    InVehicle {
        vehicle_id: EntityId,
        seat_index: u16,
    },
    Alighting {
        vehicle_id: EntityId,
        stop_id: String,
    },
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
    fn client_set_tile_kind_command_serializes_with_type_tag() {
        let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            command_id: "command:test:1".to_string(),
            coord: ChunkCoordDto { x: 4, y: 4 },
            local_index: 11,
            kind: TileKindDto::Water,
        });

        let json = serde_json::to_string(&command).expect("command serializes");

        assert_eq!(
            json,
            r#"{"type":"set_tile_kind","protocol_version":1,"world_id":"abutown-main","command_id":"command:test:1","coord":{"x":4,"y":4},"local_index":11,"kind":"water"}"#
        );
    }

    #[test]
    fn accepted_command_response_serializes_event() {
        let event = WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id: "event:1".to_string(),
            command_id: "command:test:1".to_string(),
            world_id: WorldId("abutown-main".to_string()),
            tick: 0,
            version: 1,
            coord: ChunkCoordDto { x: 4, y: 4 },
            local_index: 11,
            kind: TileKindDto::Water,
        });
        let response = CommandResponseDto::Accepted(CommandAcceptedDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            command_id: "command:test:1".to_string(),
            event,
        });

        let json = serde_json::to_value(&response).expect("accepted response serializes");

        assert_eq!(json["status"], "accepted");
        assert_eq!(json["event"]["type"], "tile_kind_set");
        assert_eq!(json["event"]["event_id"], "event:1");
        assert_eq!(json["event"]["kind"], "water");
    }

    #[test]
    fn rejected_command_response_serializes_reason() {
        let response = CommandResponseDto::Rejected(CommandRejectedDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: Some(WorldId("abutown-main".to_string())),
            command_id: Some("command:test:2".to_string()),
            code: "chunk_not_loaded".to_string(),
            message: "chunk 9:9 is not loaded".to_string(),
        });

        let json = serde_json::to_value(&response).expect("rejected response serializes");

        assert_eq!(json["status"], "rejected");
        assert_eq!(json["world_id"], "abutown-main");
        assert_eq!(json["command_id"], "command:test:2");
        assert_eq!(json["code"], "chunk_not_loaded");
    }

    #[test]
    fn websocket_world_event_serializes_with_outer_type_tag() {
        let message = ServerMessageDto::WorldEvent {
            event: WorldEventDto::TileKindSet(TileKindSetEventDto {
                protocol_version: PROTOCOL_VERSION,
                event_id: "event:2".to_string(),
                command_id: "command:test:3".to_string(),
                world_id: WorldId("abutown-main".to_string()),
                tick: 4,
                version: 8,
                coord: ChunkCoordDto { x: 5, y: 4 },
                local_index: 23,
                kind: TileKindDto::Road,
            }),
        };

        let json = serde_json::to_value(&message).expect("world event message serializes");

        assert_eq!(json["type"], "world_event");
        assert_eq!(json["event"]["type"], "tile_kind_set");
        assert_eq!(json["event"]["version"], 8);
        assert_eq!(json["event"]["coord"]["x"], 5);
    }

    #[test]
    fn mobility_snapshot_serializes_agents_vehicles_and_stops() {
        let snapshot = MobilitySnapshotDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            tick: 3,
            agents: vec![AgentMobilityDto {
                id: EntityId("agent:pedestrian:0".to_string()),
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
                occupants: vec![EntityId("agent:pedestrian:0".to_string())],
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
        assert_eq!(json["agents"][0]["id"], "agent:pedestrian:0");
        assert_eq!(json["agents"][0]["state"]["type"], "in_vehicle");
        assert_eq!(json["agents"][0]["state"]["vehicle_id"], "vehicle:tram:0");
        assert_eq!(json["vehicles"][0]["occupants"][0], "agent:pedestrian:0");
        assert_eq!(json["stops"][0]["id"], "stop:old-town");
    }

    #[test]
    fn websocket_mobility_delta_serializes_with_type_tag() {
        let message = ServerMessageDto::MobilityDelta(MobilityDeltaDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            tick: 8,
            changed_agents: vec![AgentMobilityDto {
                id: EntityId("agent:pedestrian:0".to_string()),
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
