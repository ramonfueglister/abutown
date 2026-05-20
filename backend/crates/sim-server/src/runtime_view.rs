//! Phase 7c — types for the mutation queue + Arc-Snapshot read view.
//!
//! `Mutation` is the variant flowing through the mpsc channel from any
//! HTTP/WS handler into the tick task. `RuntimeReadView` is the
//! immutable per-tick snapshot published via `ArcSwap`.

use abutown_protocol::v1 as w;
use abutown_protocol::{ChunkSnapshotDto, WorldId};
use sim_core::ids::ChunkCoord;
use sim_core::mobility::MobilityWorld;
use std::collections::HashMap;

use crate::commands::{AppliedCommand, CommandRejection};
use tokio::sync::oneshot;

/// All mutations to the runtime flow through one channel.
pub enum Mutation {
    ApplyCommand {
        command: w::ClientCommand,
        reply: oneshot::Sender<Result<AppliedCommand, CommandRejection>>,
    },
    SubscriptionDiff {
        added: Vec<ChunkCoord>,
        removed: Vec<ChunkCoord>,
        reply: oneshot::Sender<Vec<w::MobilityChunkSnapshot>>,
    },
    MarkChunkSnapshotsPersisted {
        coords: Vec<ChunkCoord>,
    },
    /// Persist loop's request for everything it needs to write a snapshot
    /// cycle: a list of dirty chunk snapshots and a clone of the mobility
    /// world. Runs inside the tick task so no external lock is required.
    CollectPersistData {
        reply: oneshot::Sender<PersistPayload>,
    },
}

/// Everything the snapshot persist loop needs to issue DB writes without
/// touching the live runtime.
///
/// Persist payloads remain serde DTOs — the storage path keeps the legacy
/// schema until Task 6 / 7 revisit it. They never cross the WS wire.
pub struct PersistPayload {
    pub chunk_snapshots: Vec<ChunkSnapshotDto>,
    pub world_id: WorldId,
    pub mobility_tick: u64,
    pub mobility_world: MobilityWorld,
}

/// Lock-free read view of the runtime, published once per tick.
/// Everything readers need is pre-materialized so readers never touch
/// the live World.
#[derive(Clone)]
pub struct RuntimeReadView {
    pub tick: u64,
    pub world_id: WorldId,
    pub mobility_tick: u64,
    pub health: w::HealthResponse,
    pub world_summary: w::WorldSummary,
    pub chunk_snapshots: HashMap<ChunkCoord, w::ChunkSnapshot>,
    pub mobility_chunk_snapshots: HashMap<ChunkCoord, w::MobilityChunkSnapshot>,
    pub mobility_full_dto: w::MobilitySnapshot,
    pub per_chunk_deltas: Vec<w::MobilityChunkDelta>,
    pub pulse_sequence: u64,
    /// Subscriber-counts snapshot for chunk channels — moved into the view so
    /// WS chunk_unsubscribe can reap chunk channels without a runtime read-lock.
    pub chunk_subscriber_counts: HashMap<ChunkCoord, u8>,
}
