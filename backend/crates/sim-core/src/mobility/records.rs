use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::ids::{AgentId, LinkId, RouteId, StopId, VehicleId};

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
        Self {
            id,
            state,
            plan,
            plan_cursor: 0,
            walk_speed_per_tick,
        }
    }
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

/// The new per-chunk delta produced by `tick_mobility`. Mirrors
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
