//! Intersection behaviour: signal phase gating, gap acceptance, and
//! conflict-point occupancy at network nodes (Task 5).
//!
//! A vehicle traverses the network as a sequence of lanes ([`crate::fleet`]).
//! Where two consecutive route lanes belong to different edges, the vehicle
//! must cross a **node** — governed by a **turn** (`fromLane -> toLane`) whose
//! `node`, `conflictsWith`, and `yieldsTo` fields carry the junction rules.
//! This module answers, for a vehicle approaching a lane end: *may I cross?*
//!
//! # Determinism
//! * **Signal state** is a pure function of the tick: the cycle position is
//!   `(t·dt) mod cycleS`, so no per-signal mutable clock is stored or advanced.
//!   Per-turn green windows are precomputed once in [`JunctionModel::build`].
//! * **Gap acceptance** is read-only over the phase-1 snapshot.
//! * **Conflict-point occupancy** is the only bookkeeping and it lives entirely
//!   in phase-2 sequential apply (fixed slot order); see [`NodeOccupancy`].
//! * No `HashMap` on the sim path; the turn lookup uses the net's `turns_from`
//!   CSR (≤ a handful of turns per lane) and everything else is a dense `Vec`.

use traffic_net::{NodeKind, TrafficNet};

/// Gap-acceptance critical headway (s), keyed by the controlling node kind.
///
/// The brief specifies three canonical values: 4 s roundabout entry, 6 s left
/// across oncoming, 5 s right-before-left. We cannot robustly recover the
/// left/right geometry of a turn in v1, so we map by node kind — the simplest
/// correct classification that honours the three values:
///
/// * `Roundabout` → 4 s (circulating-gap entry),
/// * `Priority`   → 5 s (right-before-left / priority give-way),
/// * everything else (uncontrolled) → 6 s (conservative: worst-case left across
///   oncoming).
///
/// A vehicle taking a *yielding* turn accepts iff every conflicting approaching
/// vehicle is farther than `t_gap·v_conflict + margin` from the conflict point.
const T_GAP_ROUNDABOUT: f32 = 4.0;
const T_GAP_PRIORITY: f32 = 5.0;
const T_GAP_UNCONTROLLED: f32 = 6.0;

/// Spatial safety margin (m) added to the time-gap distance so a just-barely
/// gap is rejected rather than accepted into a hairline miss.
const GAP_MARGIN_M: f32 = 4.0;

/// A conflicting vehicle slower than this (m/s) is not an *approaching*
/// priority stream and is skipped by the phase-1 gap check. Gap-acceptance
/// theory defines the critical gap against approaching priority traffic; a
/// vehicle standing at its own stop line is itself waiting, and vetoing on it
/// deadlocks mutual-yield nodes (right-before-left is cyclic by construction
/// — the S2 calibration run gridlocked to 26.6k of 26.7k vehicles stopped by
/// world midnight). Physical exclusion at the node stays guaranteed by the
/// phase-2 conflict-point occupancy (claims + clearance interval), which is
/// the crossing authority for slow/standing traffic.
pub const APPROACHING_MIN_V: f32 = 0.5;

/// How close (m) to the lane end a vehicle begins consulting the junction gate.
/// Beyond this it is treated as free road (its next turn is irrelevant yet).
pub const APPROACH_ZONE_M: f32 = 40.0;

/// Within this distance (m) of a lane end, MOBIL lane changes are restricted to
/// target lanes that can still serve the route ("mandatory-lane-light"; see
/// [`crate::tick`]). Keeps a turn-unaware MOBIL from stranding a vehicle in a
/// lane with no turn for its next route edge.
pub const MANDATORY_ZONE_M: f32 = 50.0;

/// Precomputed per-turn junction data, indexed by turn id (dense `0..n`).
///
/// Built once from the [`TrafficNet`]; read-only during the tick. Holds the
/// signal green window (if the turn's node is signalised) and the gap-acceptance
/// critical headway for the turn's node kind.
pub struct JunctionModel {
    /// Per turn id: signal gating window, or `None` if the node is unsignalised.
    signal: Vec<Option<GreenWindow>>,
    /// Per turn id: gap-acceptance critical headway (s). `0.0` means "no yield
    /// required" (the turn has an empty `yieldsTo`).
    t_gap: Vec<f32>,
    /// Per turn id: the node it crosses (for conflict-point occupancy).
    node: Vec<u32>,
}

/// A signalised turn's green interval within its cycle, in seconds.
///
/// The turn is green iff `cycle_pos ∈ [start, start+green)` where
/// `cycle_pos = (t·dt) mod cycle_s`. Cycle positions covered by no phase's
/// green are all-red (every turn at the node reads red), which is exactly the
/// bake's all-red time between phases.
#[derive(Clone, Copy)]
struct GreenWindow {
    cycle_s: f32,
    start: f32,
    green: f32,
}

impl JunctionModel {
    /// Precompute per-turn signal windows and gap headways from `net`.
    ///
    /// Signal windows: for each signal node, walk its phases accumulating
    /// `green_s` to get each phase's `[start, start+green)` window, and record
    /// it against every turn the phase gates. Turns at unsignalised nodes get
    /// `None` (governed by gap acceptance only).
    pub fn build(net: &TrafficNet) -> JunctionModel {
        let turn_count = net.turns.len();
        // Turn ids in the bake are dense 0..n (validated upstream); index by id.
        let mut signal = vec![None; turn_count];
        let mut t_gap = vec![0.0f32; turn_count];
        let mut node = vec![0u32; turn_count];

        // Map node id -> kind for gap-headway classification. Nodes are not
        // guaranteed id==index, so build a small dense LUT by max id.
        let max_node = net.nodes.iter().map(|n| n.id).max().unwrap_or(0) as usize;
        let mut node_kind = vec![NodeKind::Uncontrolled; max_node + 1];
        for n in &net.nodes {
            node_kind[n.id as usize] = n.kind;
        }

        for t in &net.turns {
            let id = t.id as usize;
            node[id] = t.node;
            if !t.yields_to.is_empty() {
                t_gap[id] = match node_kind[t.node as usize] {
                    NodeKind::Roundabout => T_GAP_ROUNDABOUT,
                    NodeKind::Priority => T_GAP_PRIORITY,
                    _ => T_GAP_UNCONTROLLED,
                };
            }
        }

        // Signal windows from each signal node's phase table.
        for n in &net.nodes {
            let Some(sig) = &n.signal else { continue };
            let mut start = 0.0f32;
            for phase in &sig.phases {
                for &turn in &phase.turns {
                    signal[turn as usize] = Some(GreenWindow {
                        cycle_s: sig.cycle_s,
                        start,
                        green: phase.green_s,
                    });
                }
                start += phase.green_s;
            }
        }

        JunctionModel {
            signal,
            t_gap,
            node,
        }
    }

    /// The node this turn crosses.
    #[inline]
    pub fn node_of(&self, turn: u32) -> u32 {
        self.node[turn as usize]
    }

    /// Whether this turn's signal is green at tick `t`. Turns at unsignalised
    /// nodes are always "green" (their safety comes from gap acceptance).
    ///
    /// Stateless: derived purely from `t` and the precomputed window.
    #[inline]
    pub fn signal_green(&self, turn: u32, t: u64, dt: f32) -> bool {
        match self.signal[turn as usize] {
            None => true,
            Some(w) => {
                // Cycle position in [0, cycle_s). f64 for phase-stable modulo.
                let elapsed = t as f64 * dt as f64;
                let pos = elapsed.rem_euclid(w.cycle_s as f64) as f32;
                pos >= w.start && pos < w.start + w.green
            }
        }
    }

    /// The gap-acceptance critical headway (s) for this turn; `0.0` if the turn
    /// yields to nobody (no gap check needed).
    #[inline]
    pub fn t_gap(&self, turn: u32) -> f32 {
        self.t_gap[turn as usize]
    }

    /// Does the *approaching conflicting* vehicle leave an acceptable gap?
    ///
    /// `dist_to_conflict` is the conflicting vehicle's remaining distance to the
    /// shared node (m); `v_conflict` its speed (m/s). Accept iff it is farther
    /// than `t_gap·v_conflict + margin`. With `t_gap == 0` (no yield) this is
    /// always accepted.
    #[inline]
    pub fn gap_ok(&self, turn: u32, dist_to_conflict: f32, v_conflict: f32) -> bool {
        let tg = self.t_gap[turn as usize];
        if tg == 0.0 {
            return true;
        }
        dist_to_conflict > tg * v_conflict + GAP_MARGIN_M
    }
}

/// Sentinel turn id meaning "no turn" in dense turn-id space.
pub const NONE_TURN: u32 = u32::MAX;

/// Find the turn (id) that carries a vehicle from `from_lane` onto `to_lane`,
/// or `None` if no such turn exists (route/net inconsistency).
///
/// Scans the net's `turns_from(from_lane)` CSR — a handful of entries — for the
/// one whose `toLane` matches. No allocation, no `HashMap`.
#[inline]
pub fn turn_between(net: &TrafficNet, from_lane: u32, to_lane: u32) -> Option<u32> {
    net.turns_from(from_lane)
        .iter()
        .copied()
        .find(|&tid| net.turns[tid as usize].to_lane == to_lane)
}

/// How many ticks a crossing vehicle keeps its node's conflict point claimed.
///
/// The node interior is not a lane in v1 (a crossing advances the route cursor
/// in a single tick), but a vehicle physically occupies the junction for a short
/// clearance time as it passes through. Holding the claim for a few ticks after
/// the crossing stops a *conflicting* turn from entering while the first vehicle
/// is still clearing the node — the standard intersection clearance interval.
/// At `DT = 0.1 s`, 20 ticks ≈ 2 s, enough for a vehicle to clear a small node
/// at urban speed. Non-conflicting turns are never blocked, so throughput on
/// compatible movements is unaffected.
pub const CLEARANCE_TICKS: u64 = 20;

/// Per-node conflict-point occupancy, resolved in phase-2 fixed slot order. A
/// vehicle crossing a node "claims" its turn for [`CLEARANCE_TICKS`]; a later
/// vehicle whose turn conflicts with a still-active claim is held at the stop
/// line. Because phase-2 applies vehicles in ascending slot order and the claim
/// bookkeeping lives entirely there, the arbitration is total and
/// thread-independent.
///
/// Allocation-free after construction: each node keeps a small fixed-capacity
/// list of `(turn, expiry_tick)` claims. A claim is live iff `now < expiry`;
/// expired entries are lazily overwritten, so no per-tick sweep is needed.
pub struct NodeOccupancy {
    /// Per node id: recent claims as `(turn_id, expiry_tick)`. A claim blocks
    /// conflicting turns until `now >= expiry_tick`.
    claims: Vec<Vec<(u32, u64)>>,
    /// Current tick, set at the start of each phase-2 occupancy pass.
    now: u64,
}

impl NodeOccupancy {
    /// Sized for the net's node id space. Pre-reserves each node's claim buffer
    /// at that node's actual turn count (the real Winterthur network peaks at
    /// ~18 turns on its busiest node), so a fully-loaded node never reallocates
    /// on the hot path. Nodes with no turns still get a capacity-1 buffer (the
    /// `Vec` itself is cheap; this only avoids a first-claim realloc).
    pub fn new(net: &TrafficNet) -> NodeOccupancy {
        let max_node = net.nodes.iter().map(|n| n.id).max().unwrap_or(0) as usize;
        let n = max_node + 1;
        let mut turns_per_node = vec![0usize; n];
        for t in &net.turns {
            turns_per_node[t.node as usize] += 1;
        }
        NodeOccupancy {
            claims: turns_per_node
                .into_iter()
                .map(|count| Vec::with_capacity(count.max(1)))
                .collect(),
            now: 0,
        }
    }

    /// Begin a new phase-2 occupancy pass for tick `t`.
    #[inline]
    pub fn begin_tick(&mut self, t: u64) {
        self.now = t;
    }

    /// Try to claim `node` for crossing turn `turn`, where `conflicts` is the
    /// turn's `conflictsWith` list. Succeeds (records the claim for the clearance
    /// window, returns `true`) iff no *live* claim at this node conflicts with
    /// `turn` — a symmetric check, robust to one-sided `conflictsWith` data. On
    /// failure the vehicle holds at the stop line.
    pub fn try_claim(&mut self, net: &TrafficNet, node: u32, turn: u32, conflicts: &[u32]) -> bool {
        let n = node as usize;
        let now = self.now;
        // The lane this turn merges onto: two distinct turns that feed the SAME
        // `toLane` share a physical merge point, so only one may cross per
        // clearance window regardless of whether the bake marked them
        // `conflictsWith` (real bakes frequently omit merge conflicts). Treating
        // a shared `toLane` as an implicit conflict closes that gap — two
        // vehicles can no longer land on top of each other at s≈0 on the merged
        // lane. Grounded in the same conflict-point principle the explicit list
        // encodes; purely additive (never *removes* a conflict).
        let turn_to_lane = net.turns[turn as usize].to_lane;
        // Reject if any live claim conflicts.
        for &(other, expiry) in &self.claims[n] {
            if now >= expiry || other == turn {
                continue; // expired, or the same movement (compatible with self)
            }
            let a_conflicts_b = conflicts.contains(&other);
            let b_conflicts_a = net.turns[other as usize].conflicts_with.contains(&turn);
            let shared_merge = net.turns[other as usize].to_lane == turn_to_lane;
            if a_conflicts_b || b_conflicts_a || shared_merge {
                return false;
            }
        }
        // Record the claim, reusing an expired slot to bound the buffer.
        let expiry = now + CLEARANCE_TICKS;
        let list = &mut self.claims[n];
        if let Some(slot) = list.iter_mut().find(|(_, e)| now >= *e) {
            *slot = (turn, expiry);
        } else {
            list.push((turn, expiry));
        }
        true
    }
}
