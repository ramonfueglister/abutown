//! Phase 7c — types for the mutation queue + Arc-Snapshot read view.
//!
//! `Mutation` is the variant flowing through the mpsc channel from any
//! HTTP/WS handler into the tick task. `RuntimeReadView` is the
//! immutable per-tick snapshot published via `ArcSwap`.

use abutown_protocol::{
    ChunkSnapshotDto, ClientCommandDto, HealthResponse, MobilityChunkDeltaDto,
    MobilityChunkSnapshotDto, MobilitySnapshotDto, WorldId, WorldSummaryDto,
};
use sim_core::ids::ChunkCoord;
use std::collections::HashMap;

use crate::commands::{AppliedCommand, CommandRejection};
use tokio::sync::oneshot;

/// All mutations to the runtime flow through one channel.
pub enum Mutation {
    ApplyCommand {
        command: ClientCommandDto,
        reply: oneshot::Sender<Result<AppliedCommand, CommandRejection>>,
    },
    SubscriptionDiff {
        added: Vec<ChunkCoord>,
        removed: Vec<ChunkCoord>,
        reply: oneshot::Sender<Vec<MobilityChunkSnapshotDto>>,
    },
    MarkChunkSnapshotsPersisted {
        coords: Vec<ChunkCoord>,
    },
}

/// Lock-free read view of the runtime, published once per tick.
/// Everything readers need is pre-materialized so readers never touch
/// the live World.
#[derive(Clone)]
pub struct RuntimeReadView {
    pub tick: u64,
    pub world_id: WorldId,
    pub mobility_tick: u64,
    pub health: HealthResponse,
    pub world_summary: WorldSummaryDto,
    pub chunk_snapshots: HashMap<ChunkCoord, ChunkSnapshotDto>,
    pub mobility_chunk_snapshots: HashMap<ChunkCoord, MobilityChunkSnapshotDto>,
    pub mobility_full_dto: MobilitySnapshotDto,
    pub per_chunk_deltas: Vec<MobilityChunkDeltaDto>,
    pub pulse_sequence: u64,
}
