use serde::{Deserialize, Serialize};

use crate::ids::{AgentId, VehicleId};
use crate::routing::{ModeState, RoutingProfileKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VehicleKind {
    Car,
}

impl From<VehicleKind> for abutown_protocol::VehicleKindDto {
    fn from(value: VehicleKind) -> Self {
        match value {
            VehicleKind::Car => abutown_protocol::VehicleKindDto::Car,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AgentMobilityState {
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
        vehicle_id: VehicleId,
        stop_id: String,
    },
    InVehicle {
        vehicle_id: VehicleId,
        seat_index: u16,
    },
    Alighting {
        vehicle_id: VehicleId,
        stop_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanStage {
    WalkToStop {
        link_id: String,
        stop_id: String,
    },
    RideToStop {
        route_id: String,
        stop_id: String,
    },
    WalkToActivity {
        link_id: String,
        activity_id: String,
    },
    Activity {
        activity_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedActiveRoute {
    pub destination_node: u32,
    pub profile: RoutingProfileKey,
    pub cursor: usize,
    pub steps: Vec<PersistedRouteStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedRouteStep {
    pub edge_id: u32,
    pub mode: ModeState,
    pub canonical_edge_key: String,
    pub length: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRecord {
    pub id: AgentId,
    pub state: AgentMobilityState,
    pub plan: Vec<PlanStage>,
    pub plan_cursor: usize,
    pub walk_speed_per_tick: f32,
    #[serde(default)]
    pub birth_tick: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_route: Option<PersistedActiveRoute>,
    #[serde(default)]
    pub sex: crate::mobility::components::Sex,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<AgentId>,
    #[serde(default)]
    pub cyclic: bool,
}

impl AgentRecord {
    /// Construct an agent fresh — `plan_cursor` starts at 0 (the first plan
    /// stage). For agents being rehydrated from a snapshot, build the
    /// struct literal directly to preserve the persisted cursor.
    pub fn new(
        id: AgentId,
        state: AgentMobilityState,
        plan: Vec<PlanStage>,
        walk_speed_per_tick: f32,
    ) -> Self {
        Self::new_born_at(id, state, plan, walk_speed_per_tick, 0)
    }

    /// Construct an agent with an explicit birth tick for age derivation.
    pub fn new_born_at(
        id: AgentId,
        state: AgentMobilityState,
        plan: Vec<PlanStage>,
        walk_speed_per_tick: f32,
        birth_tick: i64,
    ) -> Self {
        Self {
            id,
            state,
            plan,
            plan_cursor: 0,
            walk_speed_per_tick,
            birth_tick,
            active_route: None,
            sex: crate::mobility::components::Sex::default(),
            parent_id: None,
            cyclic: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VehicleRecord {
    pub id: VehicleId,
    pub kind: VehicleKind,
    pub route_id: String,
    pub link_index: usize,
    pub progress: f32,
    pub speed_per_tick: f32,
    pub capacity: u16,
    pub occupants: Vec<AgentId>,
    pub dwell_ticks_remaining: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StopMobilityRecord {
    pub id: String,
    pub route_id: String,
    pub link_index: usize,
    pub progress: f32,
    pub waiting_agents: Vec<AgentId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MobilitySnapshot {
    pub agents: Vec<AgentRecord>,
    pub vehicles: Vec<VehicleRecord>,
    pub stops: Vec<StopMobilityRecord>,
}

/// The per-chunk delta produced by `tick_mobility`. Mirrors
/// `MobilityChunkDeltaDto` shape but uses sim-core record types directly.
#[derive(Debug, Clone, PartialEq)]
pub struct MobilityChunkDelta {
    pub chunk: crate::ids::ChunkCoord,
    pub changed_agents: Vec<AgentRecord>,
    pub changed_vehicles: Vec<VehicleRecord>,
    pub left_agents: Vec<crate::ids::AgentId>,
    pub left_vehicles: Vec<crate::ids::VehicleId>,
}

/// What `build_chunk_snapshot` returns: the current entities inside a chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct MobilityChunkSnapshot {
    pub chunk: crate::ids::ChunkCoord,
    pub agents: Vec<AgentRecord>,
    pub vehicles: Vec<VehicleRecord>,
}

#[cfg(test)]
mod route_execution_tests {
    use super::*;
    use crate::routing::{ModeState, RoutingProfileKey};

    #[test]
    fn agent_record_round_trips_active_route() {
        let mut record = AgentRecord::new(
            AgentId("agent:route".to_string()),
            AgentMobilityState::Walking {
                link_id: "link:start".to_string(),
                progress: 0.25,
            },
            vec![PlanStage::WalkToActivity {
                link_id: "link:start".to_string(),
                activity_id: "activity:work".to_string(),
            }],
            1.25,
        );
        record.active_route = Some(PersistedActiveRoute {
            destination_node: 42,
            profile: RoutingProfileKey::Walk,
            cursor: 1,
            steps: vec![
                PersistedRouteStep {
                    edge_id: 7,
                    mode: ModeState::Walking,
                    canonical_edge_key: "footway:7".to_string(),
                    length: 12.5,
                },
                PersistedRouteStep {
                    edge_id: 8,
                    mode: ModeState::Walking,
                    canonical_edge_key: "footway:8".to_string(),
                    length: 32.0,
                },
            ],
        });

        let json = serde_json::to_string(&record).expect("agent record serializes");
        let decoded: AgentRecord = serde_json::from_str(&json).expect("agent record deserializes");

        assert_eq!(decoded, record);
    }

    #[test]
    fn legacy_agent_record_defaults_active_route_to_none() {
        let json = r#"{
            "id":"agent:legacy",
            "state":{"AtActivity":{"activity_id":"home"}},
            "plan":[],
            "plan_cursor":0,
            "walk_speed_per_tick":1.0
        }"#;

        let decoded: AgentRecord =
            serde_json::from_str(json).expect("legacy agent record deserializes");

        assert_eq!(decoded.active_route, None);
    }
}
