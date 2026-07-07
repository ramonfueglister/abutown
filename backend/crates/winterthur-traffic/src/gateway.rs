//! The WebSocket gateway (Task 8): fan out per-AOI-cell traffic frames to
//! subscribing browser clients over `/traffic`.
//!
//! # Shape
//!
//! ```text
//!   sim thread                          gateway (tokio tasks)
//!   ──────────                          ─────────────────────
//!   publish_snapshot                    axum /traffic upgrade
//!     └─ SnapshotHook = Publisher         └─ per session:
//!          • every 2nd tick (5 Hz)             • reader task: decode
//!          • diff each cell's membership          TrafficClientMsg,
//!          • encode changed cells ONCE            mutate the session's
//!            as Arc<[u8]> (zero-copy)             subscription set,
//!          • fan out to subscribers via           enqueue keyframe reqs
//!            each session's bounded mpsc        • writer task: drain the
//!                                                 mpsc, send WS binary frames
//! ```
//!
//! ## Why a shared-registry fan-out (not `tokio::broadcast`)
//!
//! Each cell frame is encoded **once** into an `Arc<[u8]>` and the *same* buffer
//! is handed to every subscribing session (the #93 read-view zero-copy lesson).
//! A single `broadcast` channel would force every session to receive every
//! cell and filter client-side — wasteful when a session subscribes to a
//! handful of cells out of thousands. Instead the publisher holds a snapshot of
//! the session table (`RwLock<HashMap>`) and pushes each frame only to the
//! sessions that subscribe to that cell. The per-session channel is bounded
//! (capacity [`SESSION_CHANNEL_CAP`]); on overflow we **drop the oldest** frame
//! so a slow client can never block the sim/publish path.
//!
//! ## Determinism
//!
//! The publisher is a read-only [`SnapshotHook`] — it never mutates sim state,
//! and subscriptions live entirely gateway-side (not in the ECS `CommandQueue`).
//! The wire therefore cannot feed back into the simulation.

use crate::cells::CellGrid;
use crate::flow::{self, FLOW_EVERY_N_TICKS};
use crate::shell::{Snapshot, SnapshotHook};
use bytes::Bytes;
use prost::Message;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::Notify;

use abutown_protocol::traffic::{CellFrame, TrafficClientMsg, TrafficServerMsg, VehicleState};

/// Publish every 2nd sim tick → 5 Hz at the 10 Hz sim rate.
pub const PUBLISH_EVERY_N_TICKS: u64 = 2;

/// Force a keyframe for every cell this often (in publish ticks). 5 s at 5 Hz
/// = 25 publishes. A keyframe re-syncs any client that missed deltas.
pub const KEYFRAME_EVERY_N_PUBLISHES: u64 = 25;

/// Per-session outbound queue depth. Small: a healthy client drains at 5 Hz;
/// 64 frames of slack absorbs a scheduling hiccup, past which we drop-oldest.
pub const SESSION_CHANNEL_CAP: usize = 64;

/// Cap on the number of cells a single session may subscribe to. A browser AOI
/// is a handful of cells; anything past this is a malformed / hostile client,
/// so excess subscribe ids are dropped (logged once at debug).
pub const MAX_SUBSCRIPTIONS_PER_SESSION: usize = 256;

// --- Wire vehicle-id composition ------------------------------------------
//
// The on-wire vehicle id is NOT the raw fleet slot. A slot recycled after
// despawn would otherwise be indistinguishable from its former occupant, and a
// client dead-reckoning by id would ghost/teleport it if the departed delta was
// lost. We therefore pack the slot's reuse generation into the high bits:
//
//     wire_id = slot | (generation << SLOT_BITS)   (generation wraps)
//
// The fleet cap is `MAX_CONCURRENT = 30_000 < 32_768 = 2^15` (raised from
// v1's 1500 in Task 8, which widened this split 12 -> 15 bits), so 15 bits
// hold every slot — including the kernel's +64 slot headroom — and the
// remaining 17 bits carry the generation (which wraps by design).
// `assert_slot_cap_fits` enforces `cap <= 2^SLOT_BITS` at grid/publisher
// construction so this split can never silently truncate a slot id. Clients
// treat wire ids as opaque keys (map + hash-color), so widening the split is
// wire-compatible.
/// Low bits of the wire id that hold the fleet slot (`2^15 = 32768 > 30_064`).
pub const SLOT_BITS: u32 = 15;
/// Mask selecting the slot portion of a composed wire id.
pub const SLOT_MASK: u32 = (1 << SLOT_BITS) - 1;

/// Compose a wire-stable vehicle id from a fleet `slot` and its reuse
/// `generation`. The generation occupies the high `32 - SLOT_BITS` bits.
#[inline]
fn compose_wire_id(slot: u32, generation: u32) -> u32 {
    debug_assert!(slot <= SLOT_MASK, "slot {slot} exceeds SLOT_BITS");
    slot | (generation << SLOT_BITS)
}

/// Assert the fleet capacity fits in [`SLOT_BITS`]. Called at construction so a
/// future cap bump past `2^SLOT_BITS` fails loudly instead of truncating wire
/// ids (it caught the Task 8 1500 -> 30k raise and forced the 15-bit split).
fn assert_slot_cap_fits(cap: u32) {
    assert!(
        cap <= SLOT_MASK + 1,
        "fleet cap {cap} exceeds the {} slot ids representable in SLOT_BITS={SLOT_BITS}; \
         widen the wire-id split before raising the cap",
        SLOT_MASK + 1
    );
}

/// An outbound, already-encoded WS message (a `TrafficServerMsg`), shared by
/// `Arc` across every session it fans out to.
type Frame = Arc<[u8]>;

/// A per-session outbound queue the publisher can *trim from either end*. A
/// tokio mpsc can't drop its own oldest entry (the receiver owns the head), so
/// we hold the queue explicitly: the publisher pushes to the back and, on
/// overflow, pops from the front (true drop-oldest — the newest state always
/// survives), then wakes the writer via [`Notify`]. Lock scopes are tiny and
/// the publisher never awaits while holding them, preserving the never-block-
/// the-sim guarantee.
struct OutQueue {
    /// FIFO of pending frames, capped at [`SESSION_CHANNEL_CAP`].
    deque: Mutex<VecDeque<Frame>>,
    /// Wakes the writer task when a frame is pushed (or the session closes).
    notify: Notify,
    /// Set once when the session is torn down so the writer can exit its wait.
    closed: std::sync::atomic::AtomicBool,
}

impl OutQueue {
    fn new() -> Self {
        OutQueue {
            deque: Mutex::new(VecDeque::with_capacity(SESSION_CHANNEL_CAP)),
            notify: Notify::new(),
            closed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Push a frame, dropping the OLDEST if the queue is over capacity, then
    /// wake the writer. Lock is held only for the push/pop; `notify` is fired
    /// after the guard drops. Never blocks — safe on the sim/publish path.
    fn push_drop_oldest(&self, frame: Frame) {
        {
            let mut q = self.deque.lock().unwrap();
            q.push_back(frame);
            if q.len() > SESSION_CHANNEL_CAP {
                q.pop_front(); // oldest dropped; newest state retained
            }
        }
        self.notify.notify_one();
    }

    /// Mark the queue closed and wake the writer so it can exit.
    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.notify.notify_one();
    }

    /// Drain all currently-queued frames into `out` (cleared first). Returns
    /// the number drained. Tiny lock scope, no await.
    fn drain_into(&self, out: &mut Vec<Frame>) -> usize {
        out.clear();
        let mut q = self.deque.lock().unwrap();
        out.extend(q.drain(..));
        out.len()
    }
}

/// A connected session as seen by the publisher and the axum handler.
struct Session {
    /// Bounded, trim-from-front outbound queue to this session's writer task.
    out: OutQueue,
    /// Cells this session currently subscribes to. Mutated by the session's
    /// reader task; read by the publisher. `RwLock` so the publisher's frequent
    /// reads don't serialise against each other.
    subscriptions: RwLock<HashSet<u32>>,
    /// Cells that were subscribed since the last publish and still owe an
    /// initial keyframe. Drained by the publisher each tick.
    pending_keyframes: Mutex<Vec<u32>>,
    /// Whether this session currently wants the channel's aggregate stream:
    /// on `/traffic` the flow channel (Task 11, `subscribe_flow`), on `/live`
    /// the economy vitals (Task 13, `subscribe_vitals`). Default off — most
    /// sessions only want per-cell frames. `AtomicBool` (not `RwLock`) since
    /// it's a single flag read every publish and written only by the reader
    /// task.
    aux_subscribed: AtomicBool,
}

/// The shared session table. Cloneable `Arc` handle shared between the axum
/// handler (inserts/removes sessions) and the publisher (reads + fans out).
#[derive(Clone, Default)]
pub struct Registry {
    inner: Arc<RwLock<HashMap<u64, Arc<Session>>>>,
    next_id: Arc<AtomicU64>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new session, returning its id and shared handle. The caller
    /// (axum handler) drives the writer off the session's [`OutQueue`].
    fn add(&self) -> (u64, Arc<Session>) {
        let session = Arc::new(Session {
            out: OutQueue::new(),
            subscriptions: RwLock::new(HashSet::new()),
            pending_keyframes: Mutex::new(Vec::new()),
            aux_subscribed: AtomicBool::new(false),
        });
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.inner.write().unwrap().insert(id, Arc::clone(&session));
        (id, session)
    }

    fn remove(&self, id: u64) {
        self.inner.write().unwrap().remove(&id);
    }

    /// Snapshot the current sessions into `out` (reused buffer, cleared first)
    /// so the publisher iterates without holding the table lock while it
    /// encodes + sends.
    fn snapshot_into(&self, out: &mut Vec<Arc<Session>>) {
        out.clear();
        out.extend(self.inner.read().unwrap().values().cloned());
    }
}

/// Apply a client message to a session: update its subscription set and queue
/// initial keyframes for newly-subscribed cells.
///
/// Validation (finding 4): subscribe ids `>= cell_count` are silently ignored
/// (a client can only ask for cells that exist), and the total live
/// subscription set is capped at [`MAX_SUBSCRIPTIONS_PER_SESSION`] — excess ids
/// are dropped and logged once at debug so one misbehaving client can neither
/// index out of bounds nor balloon the fan-out cost.
fn apply_client_msg(session: &Session, msg: &TrafficClientMsg, cell_count: u32) {
    let mut subs = session.subscriptions.write().unwrap();
    let mut newly = Vec::new();
    let mut dropped_over_cap = 0usize;
    for &c in &msg.subscribe_cells {
        if c >= cell_count {
            continue; // invalid cell id — ignore
        }
        if subs.contains(&c) {
            continue;
        }
        if subs.len() >= MAX_SUBSCRIPTIONS_PER_SESSION {
            dropped_over_cap += 1;
            continue;
        }
        subs.insert(c);
        newly.push(c);
    }
    for &c in &msg.unsubscribe_cells {
        subs.remove(&c);
    }
    drop(subs);
    // `subscribe_flow`: true=on, false=off, absent (None)=no change (finding
    // per the brief — mirrors the additive-optional-field pattern already used
    // for `TrafficServerMsg.flow`).
    if let Some(on) = msg.subscribe_flow {
        session.aux_subscribed.store(on, Ordering::Relaxed);
    }
    if dropped_over_cap > 0 {
        tracing::debug!(
            dropped = dropped_over_cap,
            cap = MAX_SUBSCRIPTIONS_PER_SESSION,
            "session exceeded subscription cap; excess cells dropped"
        );
    }
    if !newly.is_empty() {
        session.pending_keyframes.lock().unwrap().extend(newly);
    }
}

/// Encode one `CellFrame` as a standalone `TrafficServerMsg{cells:[frame]}` so
/// the same `Arc<[u8]>` fans out to every subscriber of that cell.
fn encode_frame(frame: CellFrame) -> Frame {
    let msg = TrafficServerMsg {
        cells: vec![frame],
        flow: None,
    };
    let bytes: Bytes = msg.encode_to_vec().into();
    Arc::from(bytes.as_ref())
}

/// Encode one `FlowFrame` (Task 11) as a standalone
/// `TrafficServerMsg{flow: Some(frame)}` so the same `Arc<[u8]>` fans out to
/// every flow-subscribed session — mirrors [`encode_frame`]'s one-encode,
/// many-recipients shape.
fn encode_flow_frame(frame: abutown_protocol::traffic::FlowFrame) -> Frame {
    let msg = TrafficServerMsg {
        cells: Vec::new(),
        flow: Some(frame),
    };
    let bytes: Bytes = msg.encode_to_vec().into();
    Arc::from(bytes.as_ref())
}

/// One member vehicle's quantised wire state: `(lane, s_q, v_q, class)`.
type MemberState = (u32, u32, u32, u32);

/// Per-cell membership + the quantised state of each member vehicle, kept
/// between publishes so the publisher can diff for deltas and departed lists.
#[derive(Default, Clone)]
struct CellState {
    /// id → (lane, s_q, v_q, class) at the last publish for this cell.
    members: HashMap<u32, MemberState>,
}

/// The publish-side state: the grid, the session registry, and the rolling
/// per-cell membership. Lives behind a `Mutex` because [`SnapshotHook`] is
/// `Fn` (not `FnMut`) — only the single sim thread ever locks it, so there is
/// no contention.
struct PublisherState {
    grid: CellGrid,
    registry: Registry,
    /// Rolling membership, indexed by cell id.
    cells: Vec<CellState>,
    /// Publish counter (increments once per publish tick), for keyframe cadence.
    publish_seq: u64,
    /// Scratch: this-tick membership per touched cell (cell → members).
    scratch_members: HashMap<u32, HashMap<u32, MemberState>>,
    /// Scratch: session snapshot, reused across publishes.
    scratch_sessions: Vec<Arc<Session>>,
}

/// Quantise `(s, v)` to the wire units: `s_q = round(s*10)` (dm),
/// `v_q = round(v*4)` (0.25 m/s). Negative/NaN clamps to 0.
fn quantise(s: f32, v: f32) -> (u32, u32) {
    let s_q = (s * 10.0).round().max(0.0) as u32;
    let v_q = (v * 4.0).round().max(0.0) as u32;
    (s_q, v_q)
}

impl PublisherState {
    fn new(grid: CellGrid, registry: Registry) -> Self {
        // The wire-id split packs the fleet slot into SLOT_BITS; assert the cap
        // still fits so a future bump past 4096 fails loudly (finding 1).
        assert_slot_cap_fits(crate::spawner::MAX_CONCURRENT as u32);
        let n = grid.cell_count() as usize;
        PublisherState {
            grid,
            registry,
            cells: vec![CellState::default(); n],
            publish_seq: 0,
            scratch_members: HashMap::new(),
            scratch_sessions: Vec::new(),
        }
    }

    /// Whether `cell` is due its periodic re-sync keyframe on publish `seq`.
    /// Staggered by cell id (finding 3): `(seq + cell) % N == 0` spreads the
    /// keyframe burst across `N` publishes instead of firing every cell on the
    /// same tick, smoothing the fan-out cost and per-session queue pressure.
    #[inline]
    fn cell_due_keyframe(seq: u64, cell: u32) -> bool {
        (seq.wrapping_add(cell as u64)).is_multiple_of(KEYFRAME_EVERY_N_PUBLISHES)
    }

    /// One publish pass. Called on every `PUBLISH_EVERY_N_TICKS`-th tick.
    fn publish(&mut self, snap: &Snapshot<'_>) {
        let seq = self.publish_seq;
        self.publish_seq += 1;

        // 1) Recompute this tick's membership for every occupied cell. Vehicle
        //    ids are composed as `slot | (generation << SLOT_BITS)` so the wire
        //    id is stable across a slot's despawn+reuse — and, crucially, the
        //    rolling `prev` maps hold the SAME composed ids, so a `departed`
        //    entry always matches exactly what the client last saw for the cell.
        for m in self.scratch_members.values_mut() {
            m.clear();
        }
        // (Reuse the allocated inner maps but clear the outer set of keys by
        // rebuilding it — retain only keeps maps we might touch again. Simpler:
        // move to a fresh view and re-populate.)
        self.scratch_members.clear();

        let core = snap.core;
        let slots = core.fleet.slots();
        for veh in 0..slots as u32 {
            let Some(view) = core.vehicle_view(veh) else {
                continue;
            };
            let Some(cell) = self.grid.cell_of_lane_s(view.lane, view.s) else {
                continue;
            };
            let (s_q, v_q) = quantise(view.s, view.v);
            let wire_id = compose_wire_id(veh, core.fleet.generation(veh as usize));
            self.scratch_members.entry(cell).or_default().insert(
                wire_id,
                (
                    view.lane,
                    s_q,
                    v_q,
                    u32::from(core.fleet.class[veh as usize]),
                ),
            );
        }

        // 2) Determine which cells changed vs last publish. A cell is "dirty"
        //    if its membership map differs, or it's due a staggered keyframe
        //    while non-empty, or it just emptied.
        self.registry.snapshot_into(&mut self.scratch_sessions);
        let no_sessions = self.scratch_sessions.is_empty();

        // Collect the union of previously-occupied and now-occupied cells.
        let mut touched: HashSet<u32> = HashSet::new();
        for (&cell, members) in &self.scratch_members {
            let prev = &self.cells[cell as usize].members;
            if Self::cell_due_keyframe(seq, cell) || members != prev {
                touched.insert(cell);
            }
        }
        // Cells that had members and now don't.
        for (cell, state) in self.cells.iter().enumerate() {
            if !state.members.is_empty() && !self.scratch_members.contains_key(&(cell as u32)) {
                touched.insert(cell as u32);
            }
        }

        // 3) For each touched cell, build a shared frame and fan out. We skip
        //    the encode entirely if no session subscribes to it (saves work at
        //    idle) — but still update rolling state so a late subscriber gets a
        //    correct keyframe.
        //
        //    Ordering note (finding 5): a cell delta built here is pushed to a
        //    session's queue BEFORE that session's on-subscribe keyframe (step
        //    4). If both land in the same publish for the same cell, the client
        //    receives the delta first and the keyframe after — harmless, since
        //    the keyframe is a full-membership re-sync that supersedes whatever
        //    the delta did, and both ride the same ordered per-session queue.
        for &cell in &touched {
            let now = self.scratch_members.get(&cell).cloned().unwrap_or_default();
            let prev = std::mem::take(&mut self.cells[cell as usize].members);

            let any_subscriber = !no_sessions
                && self
                    .scratch_sessions
                    .iter()
                    .any(|s| s.subscriptions.read().unwrap().contains(&cell));

            if any_subscriber {
                let frame = if Self::cell_due_keyframe(seq, cell) {
                    build_keyframe(cell, snap.tick, &now)
                } else {
                    build_delta(cell, snap.tick, &prev, &now)
                };
                let encoded = encode_frame(frame);
                self.fan_out(cell, &encoded);
            }

            // Commit rolling state.
            self.cells[cell as usize].members = now;
        }

        // 4) Serve pending on-subscribe keyframes (per session, from committed
        //    rolling state). See the ordering note above.
        self.serve_pending_keyframes(snap.tick);

        // 4b) Aggregate flow channel (Task 11): every FLOW_EVERY_N_TICKS sim
        //     ticks, sample the fleet once into a per-edge FlowFrame and fan it
        //     out to flow-subscribed sessions only. Read-only (see flow.rs);
        //     gated independently of the cell-publish cadence above, and
        //     skipped entirely if nobody wants it (mirrors the `any_subscriber`
        //     early-out for cells).
        if snap.tick.is_multiple_of(FLOW_EVERY_N_TICKS) {
            let any_flow_subscriber = self
                .scratch_sessions
                .iter()
                .any(|s| s.aux_subscribed.load(Ordering::Relaxed));
            if any_flow_subscriber {
                let flow_frame = flow::sample_flow_frame(snap.core, snap.net, snap.tick);
                let encoded = encode_flow_frame(flow_frame);
                for session in &self.scratch_sessions {
                    if session.aux_subscribed.load(Ordering::Relaxed) {
                        session.out.push_drop_oldest(Arc::clone(&encoded));
                    }
                }
            }
        }

        // 5) Release the session snapshot. Sessions whose reader tore down are
        //    already gone from the registry, so next publish simply won't see
        //    them — no explicit prune needed.
        self.scratch_sessions.clear();
    }

    /// Push `frame` to every session subscribing `cell` (drop-oldest on
    /// overflow; never blocks the sim path).
    fn fan_out(&self, cell: u32, frame: &Frame) {
        for session in &self.scratch_sessions {
            if session.subscriptions.read().unwrap().contains(&cell) {
                session.out.push_drop_oldest(Arc::clone(frame));
            }
        }
    }

    /// Emit any owed on-subscribe keyframes. Each is per-session (built from the
    /// committed rolling membership), so not shared — but rare (subscribe only).
    fn serve_pending_keyframes(&self, tick: u64) {
        for session in &self.scratch_sessions {
            let pending: Vec<u32> = {
                let mut p = session.pending_keyframes.lock().unwrap();
                if p.is_empty() {
                    continue;
                }
                std::mem::take(&mut *p)
            };
            for cell in pending {
                // Only if still subscribed (unsubscribe may have raced).
                if !session.subscriptions.read().unwrap().contains(&cell) {
                    continue;
                }
                let members = self
                    .cells
                    .get(cell as usize)
                    .map(|s| &s.members)
                    .cloned()
                    .unwrap_or_default();
                let frame = build_keyframe(cell, tick, &members);
                let encoded = encode_frame(frame);
                session.out.push_drop_oldest(encoded);
            }
        }
    }
}

/// Build a keyframe: full membership, empty departed list.
fn build_keyframe(cell: u32, tick: u64, members: &HashMap<u32, MemberState>) -> CellFrame {
    let mut vehicles: Vec<VehicleState> = members
        .iter()
        .map(|(&id, &(lane, s_q, v_q, class))| VehicleState {
            id,
            lane,
            s_q,
            v_q,
            class,
        })
        .collect();
    // Deterministic ordering by id keeps frames stable for tests / debugging.
    vehicles.sort_unstable_by_key(|v| v.id);
    CellFrame {
        cell,
        tick,
        keyframe: true,
        vehicles,
        departed: Vec::new(),
    }
}

/// Build a delta: changed/entered vehicles + ids that left the cell.
fn build_delta(
    cell: u32,
    tick: u64,
    prev: &HashMap<u32, MemberState>,
    now: &HashMap<u32, MemberState>,
) -> CellFrame {
    let mut vehicles = Vec::new();
    for (&id, &(lane, s_q, v_q, class)) in now {
        if prev.get(&id) != Some(&(lane, s_q, v_q, class)) {
            vehicles.push(VehicleState {
                id,
                lane,
                s_q,
                v_q,
                class,
            });
        }
    }
    vehicles.sort_unstable_by_key(|v| v.id);

    let mut departed: Vec<u32> = prev
        .keys()
        .filter(|id| !now.contains_key(id))
        .copied()
        .collect();
    departed.sort_unstable();

    CellFrame {
        cell,
        tick,
        keyframe: false,
        vehicles,
        departed,
    }
}

/// Build the [`SnapshotHook`] closure that publishes at 5 Hz into `registry`.
/// Install it on the ECS world before the tick loop starts:
///
/// ```ignore
/// let registry = Registry::new();
/// world.insert_resource(make_publisher(CellGrid::build(&net), registry.clone()));
/// // ... spawn the axum /traffic server with `registry` ...
/// ```
pub fn make_publisher(grid: CellGrid, registry: Registry) -> SnapshotHook {
    let state = Mutex::new(PublisherState::new(grid, registry));
    SnapshotHook::new(move |snap: &Snapshot<'_>| {
        // Publish at 5 Hz: only every 2nd tick. Tick 0 (the pre-first-step
        // state) publishes too, so an immediate subscriber sees something.
        if !snap.tick.is_multiple_of(PUBLISH_EVERY_N_TICKS) {
            return;
        }
        state.lock().unwrap().publish(snap);
    })
}

// ---------------------------------------------------------------------------
// /live channel (Task 13): citizen AOI frames + vitals + building deltas
// ---------------------------------------------------------------------------

use crate::shell::{LiveHook, LiveSnapshot};
use abutown_protocol::live::{
    BuildingDelta, CitizenCellFrame, CitizenState as WireCitizenState, EconomyVitals,
    LiveClientMsg, LiveServerMsg, MarketPrice,
};
use std::collections::BTreeMap;
use world_core::{BuildingLifecycle, SimWorld};

/// Force a `/live` keyframe for every cell this often (in live publishes).
/// 5 s at the 1 Hz live cadence ([`crate::shell::LIVE_PUBLISH_EVERY_N_TICKS`]).
pub const LIVE_KEYFRAME_EVERY_N_PUBLISHES: u64 = 5;

/// `BuildingLifecycle` → wire code (`live.proto` `BuildingDelta.lifecycle`).
fn lifecycle_code(lifecycle: BuildingLifecycle) -> u32 {
    match lifecycle {
        BuildingLifecycle::Occupied => 0,
        BuildingLifecycle::Vacant => 1,
        BuildingLifecycle::Decaying => 2,
        BuildingLifecycle::Demolished => 3,
        BuildingLifecycle::UnderConstruction => 4,
    }
}

/// Quantised per-citizen wire state: `(x_dm, z_dm, activity)`.
type CitizenWire = (i32, i32, u32);

/// Encode a message with one citizen cell frame (shared `Arc` fan-out, same
/// one-encode-many-recipients shape as [`encode_frame`]).
fn encode_live_cell(frame: CitizenCellFrame) -> Frame {
    let msg = LiveServerMsg {
        cells: vec![frame],
        vitals: None,
        buildings: Vec::new(),
    };
    let bytes: Bytes = msg.encode_to_vec().into();
    Arc::from(bytes.as_ref())
}

/// Encode a vitals-only message.
fn encode_live_vitals(vitals: EconomyVitals) -> Frame {
    let msg = LiveServerMsg {
        cells: Vec::new(),
        vitals: Some(vitals),
        buildings: Vec::new(),
    };
    let bytes: Bytes = msg.encode_to_vec().into();
    Arc::from(bytes.as_ref())
}

/// Encode a building-deltas-only message.
fn encode_live_buildings(buildings: Vec<BuildingDelta>) -> Frame {
    let msg = LiveServerMsg {
        cells: Vec::new(),
        vitals: None,
        buildings,
    };
    let bytes: Bytes = msg.encode_to_vec().into();
    Arc::from(bytes.as_ref())
}

/// Build a `/live` keyframe: full cell membership, empty departed list.
fn build_live_keyframe(
    cell: u32,
    world_tick: u64,
    members: &HashMap<u32, CitizenWire>,
) -> CitizenCellFrame {
    let mut citizens: Vec<WireCitizenState> = members
        .iter()
        .map(|(&id, &(x_dm, z_dm, activity))| WireCitizenState {
            id,
            x_dm,
            z_dm,
            activity,
        })
        .collect();
    citizens.sort_unstable_by_key(|c| c.id);
    CitizenCellFrame {
        cell,
        world_tick,
        keyframe: true,
        citizens,
        departed: Vec::new(),
    }
}

/// Build a `/live` delta: changed/entered citizens + ids that left the cell.
fn build_live_delta(
    cell: u32,
    world_tick: u64,
    prev: &HashMap<u32, CitizenWire>,
    now: &HashMap<u32, CitizenWire>,
) -> CitizenCellFrame {
    let mut citizens = Vec::new();
    for (&id, &(x_dm, z_dm, activity)) in now {
        if prev.get(&id) != Some(&(x_dm, z_dm, activity)) {
            citizens.push(WireCitizenState {
                id,
                x_dm,
                z_dm,
                activity,
            });
        }
    }
    citizens.sort_unstable_by_key(|c| c.id);

    let mut departed: Vec<u32> = prev
        .keys()
        .filter(|id| !now.contains_key(id))
        .copied()
        .collect();
    departed.sort_unstable();

    CitizenCellFrame {
        cell,
        world_tick,
        keyframe: false,
        citizens,
        departed,
    }
}

/// Rolling per-cell citizen membership between live publishes.
#[derive(Default, Clone)]
struct LiveCellState {
    members: HashMap<u32, CitizenWire>,
}

/// Publish-side state of the `/live` channel — the [`Registry`] is its OWN
/// session table (never shared with `/traffic`), the grid is the SAME 128 m
/// [`CellGrid`] (identical cell ids on both wires).
struct LivePublisherState {
    grid: CellGrid,
    registry: Registry,
    sim: Arc<SimWorld>,
    cells: Vec<LiveCellState>,
    /// Building lifecycle deviations at the previous publish, diffed each
    /// publish into `BuildingDelta`s (M1: practically always empty — the
    /// channel exists, the transition systems come later).
    prev_building_states: BTreeMap<u32, BuildingLifecycle>,
    publish_seq: u64,
    scratch_members: HashMap<u32, HashMap<u32, CitizenWire>>,
    scratch_sessions: Vec<Arc<Session>>,
}

impl LivePublisherState {
    fn new(grid: CellGrid, registry: Registry, sim: Arc<SimWorld>) -> Self {
        let n = grid.cell_count() as usize;
        LivePublisherState {
            grid,
            registry,
            sim,
            cells: vec![LiveCellState::default(); n],
            prev_building_states: BTreeMap::new(),
            publish_seq: 0,
            scratch_members: HashMap::new(),
            scratch_sessions: Vec::new(),
        }
    }

    /// Staggered periodic keyframe, mirroring
    /// [`PublisherState::cell_due_keyframe`] at the live cadence.
    #[inline]
    fn cell_due_keyframe(seq: u64, cell: u32) -> bool {
        (seq.wrapping_add(cell as u64)).is_multiple_of(LIVE_KEYFRAME_EVERY_N_PUBLISHES)
    }

    /// One `/live` publish pass (1 Hz — the shell's `publish_live` system
    /// already gates the cadence). Same diff/keyframe/fan-out shape as the
    /// `/traffic` publisher, over citizens instead of vehicles.
    fn publish(&mut self, snap: &LiveSnapshot<'_>) {
        let seq = self.publish_seq;
        self.publish_seq += 1;

        // 1) This publish's membership per cell.
        self.scratch_members.clear();
        for c in &snap.citizens {
            let cell = self
                .grid
                .cell_of_xz(c.x_dm as f32 * 0.1, c.z_dm as f32 * 0.1);
            self.scratch_members
                .entry(cell)
                .or_default()
                .insert(c.id, (c.x_dm, c.z_dm, c.activity));
        }

        self.registry.snapshot_into(&mut self.scratch_sessions);
        let no_sessions = self.scratch_sessions.is_empty();

        // 2) Touched cells: changed membership, due keyframe, or just emptied.
        let mut touched: HashSet<u32> = HashSet::new();
        for (&cell, members) in &self.scratch_members {
            let prev = &self.cells[cell as usize].members;
            if Self::cell_due_keyframe(seq, cell) || members != prev {
                touched.insert(cell);
            }
        }
        for (cell, state) in self.cells.iter().enumerate() {
            if !state.members.is_empty() && !self.scratch_members.contains_key(&(cell as u32)) {
                touched.insert(cell as u32);
            }
        }

        // 3) Encode + fan out per touched cell; commit rolling state.
        for &cell in &touched {
            let now = self.scratch_members.get(&cell).cloned().unwrap_or_default();
            let prev = std::mem::take(&mut self.cells[cell as usize].members);

            let any_subscriber = !no_sessions
                && self
                    .scratch_sessions
                    .iter()
                    .any(|s| s.subscriptions.read().unwrap().contains(&cell));

            if any_subscriber {
                let frame = if Self::cell_due_keyframe(seq, cell) {
                    build_live_keyframe(cell, snap.world_tick, &now)
                } else {
                    build_live_delta(cell, snap.world_tick, &prev, &now)
                };
                let encoded = encode_live_cell(frame);
                for session in &self.scratch_sessions {
                    if session.subscriptions.read().unwrap().contains(&cell) {
                        session.out.push_drop_oldest(Arc::clone(&encoded));
                    }
                }
            }

            self.cells[cell as usize].members = now;
        }

        // 4) On-subscribe keyframes owed to individual sessions.
        for session in &self.scratch_sessions {
            let pending: Vec<u32> = {
                let mut p = session.pending_keyframes.lock().unwrap();
                if p.is_empty() {
                    continue;
                }
                std::mem::take(&mut *p)
            };
            for cell in pending {
                if !session.subscriptions.read().unwrap().contains(&cell) {
                    continue;
                }
                let members = self
                    .cells
                    .get(cell as usize)
                    .map(|s| &s.members)
                    .cloned()
                    .unwrap_or_default();
                let frame = build_live_keyframe(cell, snap.world_tick, &members);
                session.out.push_drop_oldest(encode_live_cell(frame));
            }
        }

        // 5) Vitals (1 Hz) to vitals-subscribed sessions.
        let any_vitals = self
            .scratch_sessions
            .iter()
            .any(|s| s.aux_subscribed.load(Ordering::Relaxed));
        if any_vitals {
            let vitals = EconomyVitals {
                world_tick: snap.world_tick,
                s_of_world_day: snap.s_of_world_day,
                population: snap.population,
                total_money: snap.total_money,
                audit_ok: u32::from(snap.audit_ok),
                prices: snap
                    .prices
                    .iter()
                    .map(|p| MarketPrice {
                        market_id: p.market,
                        good_id: p.good,
                        ewma_price: p.ewma,
                        market_name: p.name.clone(),
                    })
                    .collect(),
                trips_active: snap.trips_active,
            };
            let encoded = encode_live_vitals(vitals);
            for session in &self.scratch_sessions {
                if session.aux_subscribed.load(Ordering::Relaxed) {
                    session.out.push_drop_oldest(Arc::clone(&encoded));
                }
            }
        }

        // 6) Building lifecycle deltas — to ALL live sessions. A key that
        //    vanished from the deviation map reverted to Occupied (code 0).
        if self.prev_building_states != *snap.building_states {
            let mut deltas: Vec<BuildingDelta> = Vec::new();
            for (&b, &lifecycle) in snap.building_states {
                if self.prev_building_states.get(&b) != Some(&lifecycle) {
                    deltas.push(BuildingDelta {
                        building_uuid: self.sim.buildings[b as usize].uuid.clone(),
                        lifecycle: lifecycle_code(lifecycle),
                        world_tick: snap.world_tick,
                    });
                }
            }
            for &b in self.prev_building_states.keys() {
                if !snap.building_states.contains_key(&b) {
                    deltas.push(BuildingDelta {
                        building_uuid: self.sim.buildings[b as usize].uuid.clone(),
                        lifecycle: lifecycle_code(BuildingLifecycle::Occupied),
                        world_tick: snap.world_tick,
                    });
                }
            }
            if !deltas.is_empty() && !no_sessions {
                let encoded = encode_live_buildings(deltas);
                for session in &self.scratch_sessions {
                    session.out.push_drop_oldest(Arc::clone(&encoded));
                }
            }
            self.prev_building_states = snap.building_states.clone();
        }

        self.scratch_sessions.clear();
    }
}

/// Build the [`LiveHook`] closure publishing the `/live` channel into
/// `registry` (its own session table, NOT the `/traffic` one). Install it on
/// the ECS world before the tick loop starts, alongside the `/traffic`
/// publisher:
///
/// ```ignore
/// let live_registry = Registry::new();
/// world.insert_resource(make_live_publisher(grid.clone(), live_registry.clone(), sim));
/// // ... merge `live_router(live_registry, grid.cell_count())` onto the port ...
/// ```
pub fn make_live_publisher(grid: CellGrid, registry: Registry, sim: Arc<SimWorld>) -> LiveHook {
    let state = Mutex::new(LivePublisherState::new(grid, registry, sim));
    LiveHook::new(move |snap: &LiveSnapshot<'_>| {
        state.lock().unwrap().publish(snap);
    })
}

// ---------------------------------------------------------------------------
// axum /traffic endpoint
// ---------------------------------------------------------------------------

use axum::{
    Router as AxumRouter,
    extract::{
        State,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};

/// Which wire protocol a WS endpoint speaks. `/traffic` and `/live` share the
/// session machinery (registry, queues, cell subscriptions); only the client
/// message decoding differs.
#[derive(Clone, Copy)]
enum ChannelKind {
    Traffic,
    Live,
}

/// Axum state for a WS endpoint: the channel's session registry plus the
/// grid's cell count, used to validate client subscribe ids (finding 4).
#[derive(Clone)]
struct GatewayState {
    registry: Registry,
    cell_count: u32,
    kind: ChannelKind,
}

/// Build the axum router exposing the `/traffic` WS endpoint, sharing
/// `registry` with the publisher. `cell_count` is the AOI grid size, used to
/// clamp client-requested cell ids. `/healthz` is added by
/// [`crate::shell::run_loop_with_router`], which merges this router onto the
/// same port.
pub fn router(registry: Registry, cell_count: u32) -> AxumRouter {
    AxumRouter::new()
        .route("/traffic", get(ws_upgrade))
        .with_state(GatewayState {
            registry,
            cell_count,
            kind: ChannelKind::Traffic,
        })
}

/// Build the axum router exposing the `/live` WS endpoint (Task 13) over its
/// OWN session registry (shared with [`make_live_publisher`], never with
/// `/traffic`). Same grid ⇒ same `cell_count`.
pub fn live_router(registry: Registry, cell_count: u32) -> AxumRouter {
    AxumRouter::new()
        .route("/live", get(ws_upgrade))
        .with_state(GatewayState {
            registry,
            cell_count,
            kind: ChannelKind::Live,
        })
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<GatewayState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// One connected client: split the socket, register the session, then run a
/// reader (client → subscription updates) and a writer (outbound frames)
/// concurrently. Either side ending tears down the session.
async fn handle_socket(socket: WebSocket, state: GatewayState) {
    use futures_util::{SinkExt, StreamExt};

    let GatewayState {
        registry,
        cell_count,
        kind,
    } = state;
    let (id, session) = registry.add();
    let (mut sink, mut stream) = socket.split();

    // Writer: wait on the session's Notify, drain its trim-from-front queue,
    // and flush each frame to the socket. The publisher never blocks on us: it
    // pushes (dropping the oldest on overflow) and wakes us. We exit when the
    // queue is closed (session torn down) or the socket errors.
    let writer_session = Arc::clone(&session);
    let writer = tokio::spawn(async move {
        let mut batch: Vec<Frame> = Vec::new();
        loop {
            // Drain anything already queued before parking on the notify, so a
            // frame pushed between drains is never missed.
            writer_session.out.drain_into(&mut batch);
            for frame in batch.drain(..) {
                if sink
                    .send(WsMessage::Binary(frame.to_vec().into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            if writer_session.out.closed.load(Ordering::Acquire) {
                return;
            }
            writer_session.out.notify.notified().await;
        }
    });

    // Reader: decode client subscription messages until the socket closes.
    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            WsMessage::Binary(bytes) => match kind {
                ChannelKind::Traffic => {
                    if let Ok(client_msg) = TrafficClientMsg::decode(bytes.as_ref()) {
                        apply_client_msg(&session, &client_msg, cell_count);
                    }
                }
                ChannelKind::Live => {
                    // `LiveClientMsg` is field-for-field the same shape
                    // (`subscribe_vitals` ↔ `subscribe_flow`), so it reuses
                    // the one validated apply path via the traffic shape.
                    if let Ok(live_msg) = LiveClientMsg::decode(bytes.as_ref()) {
                        let as_traffic = TrafficClientMsg {
                            subscribe_cells: live_msg.subscribe_cells,
                            unsubscribe_cells: live_msg.unsubscribe_cells,
                            subscribe_flow: live_msg.subscribe_vitals,
                        };
                        apply_client_msg(&session, &as_traffic, cell_count);
                    }
                }
            },
            WsMessage::Close(_) => break,
            _ => {}
        }
    }

    // Reader ended → tear down: remove the session from the registry (the
    // publisher stops seeing it next tick) and close its queue so the writer
    // task wakes from its notify and exits cleanly, flushing any last frames.
    registry.remove(id);
    session.out.close();
    let _ = writer.await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(byte: u8) -> Frame {
        Arc::from([byte].as_ref())
    }

    /// True drop-oldest (finding 2): pushing past the cap evicts the FRONT
    /// (oldest) entries; a drain sees the most recent `SESSION_CHANNEL_CAP`
    /// frames, and the newest push is always present.
    #[test]
    fn out_queue_drops_oldest_keeps_newest() {
        let q = OutQueue::new();
        let total = SESSION_CHANNEL_CAP + 10;
        for i in 0..total {
            q.push_drop_oldest(frame(i as u8));
        }
        let mut out = Vec::new();
        let n = q.drain_into(&mut out);
        assert_eq!(n, SESSION_CHANNEL_CAP, "queue must be capped at capacity");

        // The retained frames are the newest `cap`: bytes [total-cap .. total).
        let expected_first = (total - SESSION_CHANNEL_CAP) as u8;
        assert_eq!(
            out.first().map(|f| f[0]),
            Some(expected_first),
            "oldest surviving frame must be the (total-cap)-th push, not frame 0"
        );
        assert_eq!(
            out.last().map(|f| f[0]),
            Some((total - 1) as u8),
            "newest push must always survive"
        );
        // Frame 0 (the very oldest) must have been dropped.
        assert!(
            !out.iter().any(|f| f[0] == 0),
            "the oldest frames must have been evicted"
        );
    }

    /// The wire-id split round-trips: the slot occupies the low bits and the
    /// generation the high bits, and two generations of the same slot differ.
    #[test]
    fn wire_id_composition_separates_generations() {
        let a = compose_wire_id(1500 & SLOT_MASK, 0);
        let b = compose_wire_id(1500 & SLOT_MASK, 1);
        assert_ne!(a, b, "same slot, different generation -> different wire id");
        assert_eq!(a & SLOT_MASK, b & SLOT_MASK, "slot bits preserved");
    }
}
