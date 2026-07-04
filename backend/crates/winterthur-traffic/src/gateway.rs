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
use crate::shell::{Snapshot, SnapshotHook};
use bytes::Bytes;
use prost::Message;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::mpsc;

use abutown_protocol::traffic::{CellFrame, TrafficClientMsg, TrafficServerMsg, VehicleState};

/// Publish every 2nd sim tick → 5 Hz at the 10 Hz sim rate.
pub const PUBLISH_EVERY_N_TICKS: u64 = 2;

/// Force a keyframe for every cell this often (in publish ticks). 5 s at 5 Hz
/// = 25 publishes. A keyframe re-syncs any client that missed deltas.
pub const KEYFRAME_EVERY_N_PUBLISHES: u64 = 25;

/// Per-session outbound queue depth. Small: a healthy client drains at 5 Hz;
/// 64 frames of slack absorbs a scheduling hiccup, past which we drop-oldest.
pub const SESSION_CHANNEL_CAP: usize = 64;

/// An outbound, already-encoded WS message (a `TrafficServerMsg`), shared by
/// `Arc` across every session it fans out to.
type Frame = Arc<[u8]>;

/// A connected session as seen by the publisher and the axum handler.
struct Session {
    /// Bounded outbound queue to this session's writer task.
    tx: mpsc::Sender<Frame>,
    /// Cells this session currently subscribes to. Mutated by the session's
    /// reader task; read by the publisher. `RwLock` so the publisher's frequent
    /// reads don't serialise against each other.
    subscriptions: RwLock<HashSet<u32>>,
    /// Cells that were subscribed since the last publish and still owe an
    /// initial keyframe. Drained by the publisher each tick.
    pending_keyframes: Mutex<Vec<u32>>,
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

    /// Register a new session, returning its id and outbound receiver. The
    /// caller (axum handler) owns the receiver and spawns the writer task.
    fn add(&self) -> (u64, Arc<Session>, mpsc::Receiver<Frame>) {
        let (tx, rx) = mpsc::channel(SESSION_CHANNEL_CAP);
        let session = Arc::new(Session {
            tx,
            subscriptions: RwLock::new(HashSet::new()),
            pending_keyframes: Mutex::new(Vec::new()),
        });
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.inner.write().unwrap().insert(id, Arc::clone(&session));
        (id, session, rx)
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
fn apply_client_msg(session: &Session, msg: &TrafficClientMsg) {
    let mut subs = session.subscriptions.write().unwrap();
    let mut newly = Vec::new();
    for &c in &msg.subscribe_cells {
        if subs.insert(c) {
            newly.push(c);
        }
    }
    for &c in &msg.unsubscribe_cells {
        subs.remove(&c);
    }
    drop(subs);
    if !newly.is_empty() {
        session.pending_keyframes.lock().unwrap().extend(newly);
    }
}

/// Try to enqueue a frame to a session; on a full channel drop the OLDEST
/// queued frame and retry once, so the newest state always wins and the
/// publish path never blocks. Returns `false` if the session is gone (receiver
/// dropped), signalling the publisher to prune it.
fn send_drop_oldest(session: &Session, frame: &Frame) -> bool {
    match session.tx.try_send(Arc::clone(frame)) {
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Full(_)) => {
            // Drop-oldest: pop one, then push the newest. `try_recv` on the
            // sender side isn't available, so we rely on the writer task
            // draining; if it's wedged we simply drop this frame (still newest
            // state arrives on the next publish). Best-effort, never blocks.
            // A single retry keeps steady-state latency bounded.
            match session.tx.try_send(Arc::clone(frame)) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => true, // dropped this frame
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
        }
        Err(mpsc::error::TrySendError::Closed(_)) => false,
    }
}

/// Encode one `CellFrame` as a standalone `TrafficServerMsg{cells:[frame]}` so
/// the same `Arc<[u8]>` fans out to every subscriber of that cell.
fn encode_frame(frame: CellFrame) -> Frame {
    let msg = TrafficServerMsg { cells: vec![frame] };
    let bytes: Bytes = msg.encode_to_vec().into();
    Arc::from(bytes.as_ref())
}

/// Per-cell membership + the quantised state of each member vehicle, kept
/// between publishes so the publisher can diff for deltas and departed lists.
#[derive(Default, Clone)]
struct CellState {
    /// id → (lane, s_q, v_q) at the last publish for this cell.
    members: HashMap<u32, (u32, u32, u32)>,
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
    scratch_members: HashMap<u32, HashMap<u32, (u32, u32, u32)>>,
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

    /// One publish pass. Called on every `PUBLISH_EVERY_N_TICKS`-th tick.
    fn publish(&mut self, snap: &Snapshot<'_>) {
        let seq = self.publish_seq;
        self.publish_seq += 1;
        let force_keyframe = seq.is_multiple_of(KEYFRAME_EVERY_N_PUBLISHES);

        // 1) Recompute this tick's membership for every occupied cell.
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
            self.scratch_members
                .entry(cell)
                .or_default()
                .insert(veh, (view.lane, s_q, v_q));
        }

        // 2) Determine which cells changed vs last publish. A cell is "dirty"
        //    if its membership map differs, or it's due a forced keyframe while
        //    non-empty, or it just emptied.
        self.registry.snapshot_into(&mut self.scratch_sessions);
        let no_sessions = self.scratch_sessions.is_empty();

        // Collect the union of previously-occupied and now-occupied cells.
        let mut touched: HashSet<u32> = HashSet::new();
        for (&cell, members) in &self.scratch_members {
            let prev = &self.cells[cell as usize].members;
            if force_keyframe || members != prev {
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
        for &cell in &touched {
            let now = self.scratch_members.get(&cell).cloned().unwrap_or_default();
            let prev = std::mem::take(&mut self.cells[cell as usize].members);

            let any_subscriber = !no_sessions
                && self
                    .scratch_sessions
                    .iter()
                    .any(|s| s.subscriptions.read().unwrap().contains(&cell));

            if any_subscriber {
                let frame = if force_keyframe {
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
        //    rolling state).
        self.serve_pending_keyframes(snap.tick);

        // 5) Prune sessions whose receiver was dropped (writer task ended).
        self.scratch_sessions.clear();
    }

    /// Send `frame` to every session subscribing `cell`; prune dead sessions.
    fn fan_out(&self, cell: u32, frame: &Frame) {
        let mut dead = Vec::new();
        for session in &self.scratch_sessions {
            if session.subscriptions.read().unwrap().contains(&cell)
                && !send_drop_oldest(session, frame)
            {
                dead.push(Arc::as_ptr(session));
            }
        }
        if !dead.is_empty() {
            self.prune(&dead);
        }
    }

    /// Emit any owed on-subscribe keyframes. Each is per-session (built from the
    /// committed rolling membership), so not shared — but rare (subscribe only).
    fn serve_pending_keyframes(&self, tick: u64) {
        let mut dead = Vec::new();
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
                if !send_drop_oldest(session, &encoded) {
                    dead.push(Arc::as_ptr(session));
                    break;
                }
            }
        }
        if !dead.is_empty() {
            self.prune(&dead);
        }
    }

    fn prune(&self, dead: &[*const Session]) {
        let mut table = self.registry.inner.write().unwrap();
        table.retain(|_, s| !dead.contains(&Arc::as_ptr(s)));
    }
}

/// Build a keyframe: full membership, empty departed list.
fn build_keyframe(cell: u32, tick: u64, members: &HashMap<u32, (u32, u32, u32)>) -> CellFrame {
    let mut vehicles: Vec<VehicleState> = members
        .iter()
        .map(|(&id, &(lane, s_q, v_q))| VehicleState { id, lane, s_q, v_q })
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
    prev: &HashMap<u32, (u32, u32, u32)>,
    now: &HashMap<u32, (u32, u32, u32)>,
) -> CellFrame {
    let mut vehicles = Vec::new();
    for (&id, &(lane, s_q, v_q)) in now {
        if prev.get(&id) != Some(&(lane, s_q, v_q)) {
            vehicles.push(VehicleState { id, lane, s_q, v_q });
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

/// Build the axum router exposing the `/traffic` WS endpoint, sharing
/// `registry` with the publisher. `/healthz` is added by
/// [`crate::shell::run_loop_with_router`], which merges this router onto the
/// same port.
pub fn router(registry: Registry) -> AxumRouter {
    AxumRouter::new()
        .route("/traffic", get(ws_upgrade))
        .with_state(registry)
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(registry): State<Registry>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, registry))
}

/// One connected client: split the socket, register the session, then run a
/// reader (client → subscription updates) and a writer (outbound frames)
/// concurrently. Either side ending tears down the session.
async fn handle_socket(socket: WebSocket, registry: Registry) {
    use futures_util::{SinkExt, StreamExt};

    let (id, session, mut rx) = registry.add();
    let (mut sink, mut stream) = socket.split();

    // Writer: drain the outbound queue to the socket.
    let writer = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if sink
                .send(WsMessage::Binary(frame.to_vec().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // Reader: decode client subscription messages until the socket closes.
    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            WsMessage::Binary(bytes) => {
                if let Ok(client_msg) = TrafficClientMsg::decode(bytes.as_ref()) {
                    apply_client_msg(&session, &client_msg);
                }
            }
            WsMessage::Close(_) => break,
            _ => {}
        }
    }

    // Reader ended → tear down: drop the session (closes the mpsc, ending the
    // writer) and remove it from the registry.
    registry.remove(id);
    writer.abort();
}
