//! Phase 7c — types for the mutation queue + Arc-Snapshot read view.
//!
//! `Mutation` is the variant flowing through the mpsc channel from any
//! HTTP/WS handler into the tick task. `RuntimeReadView` is the
//! immutable per-tick snapshot published via `ArcSwap`.

use abutown_protocol::v1 as w;
use abutown_protocol::{ChunkSnapshotDto, WorldId};
use sim_core::economy::{EconomyEvent, EconomyPersistSnapshot};
use sim_core::ids::ChunkCoord;
use sim_core::mobility::MobilityPersistSnapshot;
use std::collections::HashMap;
use std::sync::Arc;

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
    /// Acknowledge a successful audit append: advance the ledger audit cursor past
    /// the `count` events the persist loop durably appended and bound the live
    /// ledger. Fire-and-forget — a dropped commit just re-appends next cycle.
    CommitLedgerAudit {
        count: usize,
    },
    /// Persist loop's request for everything it needs to write a snapshot
    /// cycle: a list of dirty chunk snapshots and a clone of the mobility
    /// world. Runs inside the tick task so no external lock is required.
    CollectPersistData {
        reply: oneshot::Sender<PersistPayload>,
    },
    /// On-demand snapshot of the live economy for the debug endpoint.
    CollectEconomySnapshot {
        reply: oneshot::Sender<sim_core::economy::EconomyPersistSnapshot>,
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
    pub mobility_world: MobilityPersistSnapshot,
    pub economy_tick: u64,
    pub economy_world: EconomyPersistSnapshot,
    /// Tick stamped onto the audit events appended this cycle.
    pub economy_audit_tick: u64,
    /// The ledger tail not yet durably appended to the `EconomyEventStore`. The
    /// persist loop appends these, then sends `CommitLedgerAudit { count }`.
    pub economy_audit_pending: Vec<EconomyEvent>,
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
    /// Tile snapshots are `Arc`-cached across views: a chunk whose
    /// `ChunkVersion` is unchanged reuses the previous view's entry instead of
    /// re-reading ~1024 tiles + re-encoding a proto every 100 ms tick
    /// (2026-06-10 tick-cost design).
    pub chunk_snapshots: HashMap<ChunkCoord, Arc<w::ChunkSnapshot>>,
    pub mobility_chunk_snapshots: HashMap<ChunkCoord, w::MobilityChunkSnapshot>,
    pub mobility_full_dto: w::MobilitySnapshot,
    pub per_chunk_deltas: Vec<w::MobilityChunkDelta>,
    /// Subscriber-counts snapshot for chunk channels — moved into the view so
    /// WS chunk_unsubscribe can reap chunk channels without a runtime read-lock.
    pub chunk_subscriber_counts: HashMap<ChunkCoord, u8>,
    /// Pre-materialized economy snapshot for the current tick: all markets and
    /// per-(market,good) state, ready to send on connect and per-tick broadcast.
    pub economy: w::EconomySnapshot,
}
