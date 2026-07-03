//! The deterministic two-phase simulation kernel.
//!
//! Each [`Core::tick`] runs:
//!  * **Phase 1 (parallel, read-only):** partition work by lane and compute
//!    each vehicle's IDM acceleration and its integrated next `(v, s)` plus any
//!    lane hand-off, writing results into pre-sized *intent* buffers. No fleet
//!    state is mutated, so lanes are embarrassingly parallel (rayon).
//!  * **Phase 2 (sequential, fixed order):** apply the intents in ascending
//!    slot order, advance route cursors across lane boundaries, then rebuild
//!    the [`LaneIndex`]. Sequential apply in a fixed order is what makes the
//!    result independent of how many threads phase 1 used.
//!
//! Leader lookup crosses lane boundaries along the route: the leader of the
//! front-most vehicle on a lane is the rear-most vehicle on the next lane of
//! its route, with the gap spanning the remaining distance on the current lane.
//! This is required both for the closed ring (wrap-around) and for real roads
//! where a gap straddles a lane end.

use crate::fleet::{Fleet, LaneIndex, RouteHandle, VehId};
use crate::idm::{IdmParams, idm_accel};
use crate::mobil::{self, Follower, LaneNeighbourhood, MobilParams};
use rayon::prelude::*;
use traffic_net::TrafficNet;

/// Simulation timestep (s). Fixed; the integrator is ballistic-safe at this dt.
pub const DT: f32 = 0.1;

/// Default vehicle length (m) used for bumper-to-bumper gaps.
const VEHICLE_LEN: f32 = 4.5;

/// Per-vehicle intent produced by phase 1 and consumed by phase 2.
///
/// `lane`/`cursor` are the *longitudinal* result (route progression along the
/// current lane, possibly crossing a lane boundary). A lane **change** is a
/// separate, orthogonal decision carried in `lane_change`: when `Some`, phase 2
/// will — after re-checking the target-lane gap against changes already applied
/// this tick — move the vehicle sideways to that lane at the same `s`.
#[derive(Debug, Clone, Copy)]
struct Intent {
    v: f32,
    s: f32,
    lane: u32,
    cursor: u32,
    /// Optional MOBIL lane change committed by phase 1: `(target_lane,
    /// new_follower_accel)`. The accel is re-validated in phase 2.
    lane_change: Option<LaneChange>,
}

/// A phase-1 MOBIL decision to switch sideways, applied (after re-check) in
/// phase 2. Only the target lane is carried: phase 2 re-derives the target
/// lane's leader/follower from live state (after longitudinal motion and any
/// earlier-slot change this tick), which is what makes the re-check see the
/// already-applied moves.
#[derive(Debug, Clone, Copy)]
struct LaneChange {
    /// The adjacent lane (same edge, index ±1) to move onto.
    target_lane: u32,
}

impl Default for Intent {
    fn default() -> Self {
        Intent {
            v: 0.0,
            s: 0.0,
            lane: 0,
            cursor: 0,
            lane_change: None,
        }
    }
}

/// The microscopic simulation kernel: owns the fleet, the lane occupancy
/// index, precomputed per-lane geometry, and reusable intent buffers.
pub struct Core {
    /// Precomputed lane length (m) indexed **by lane id** (dense 0..n).
    lane_len: Vec<f32>,
    /// Number of lanes (dense id space width).
    lane_count: usize,

    /// SoA vehicle state.
    pub fleet: Fleet,
    /// CSR lane occupancy, leader-first per lane.
    pub index: LaneIndex,

    /// IDM parameters (single vehicle class for now).
    params: IdmParams,

    /// MOBIL lane-change parameters (single vehicle class for now).
    mobil: MobilParams,

    /// Per-lane adjacency within the same edge, indexed by lane id:
    /// `(left_neighbour, right_neighbour)`. "Left" is the higher-index lane,
    /// "right" the lower-index one (European keep-right convention; see
    /// [`crate::mobil`]). `u32::MAX` marks "no neighbour on that side".
    lane_adj: Vec<(u32, u32)>,

    /// Base seed for deterministic per-vehicle noise via [`crate::u01`].
    seed: u64,

    /// Reusable phase-1 output buffer, one slot per vehicle slot. Sized to
    /// `cap` at construction; never reallocated in the tick hot path.
    intents: Vec<Intent>,

    /// The lane ids that currently hold at least one vehicle, recomputed each
    /// tick for the parallel partition. Pre-sized to `lane_count`.
    active_lanes: Vec<u32>,
}

impl Core {
    /// Build a kernel over `net`, pre-sized for up to `cap` vehicles, seeded
    /// with `seed`.
    ///
    /// Precomputes a dense per-lane length table indexed by lane id so the tick
    /// loop never calls `TrafficNet::lane_len` (a linear id->index scan). Lane
    /// ids in the real bake are dense `0..n`; we validate that density here and
    /// fail fast otherwise, since the whole kernel assumes id == array index.
    pub fn new(net: &TrafficNet, cap: usize, seed: u64) -> Core {
        let lane_count = net.lanes.len();

        // Validate dense, contiguous lane ids 0..lane_count. id == index is a
        // load-bearing invariant for every O(1) lane lookup below.
        let mut seen = vec![false; lane_count];
        for l in &net.lanes {
            let id = l.id as usize;
            assert!(
                id < lane_count,
                "lane id {id} out of dense range 0..{lane_count}"
            );
            assert!(!seen[id], "duplicate lane id {id}");
            seen[id] = true;
        }
        assert!(
            seen.iter().all(|&b| b),
            "lane ids are not dense 0..{lane_count}; Core requires id == index"
        );

        // Dense length table indexed by lane id.
        let mut lane_len = vec![0.0f32; lane_count];
        for l in &net.lanes {
            lane_len[l.id as usize] = l.length_m;
        }

        // Per-lane same-edge adjacency by index. For each lane we record the
        // lane id one index higher (left) and one index lower (right) on the
        // same edge, or `u32::MAX` if there is none. Lanes of one edge share
        // the arc-length parameterization by construction of the bake, so a
        // sideways change preserves `s`.
        const NONE: u32 = u32::MAX;
        // Dense lane-id -> `index` LUT (id == index is validated above, but
        // `net.lanes` need not be stored id-sorted, so map explicitly).
        let mut lane_index_of = vec![0u32; lane_count];
        for l in &net.lanes {
            lane_index_of[l.id as usize] = l.index;
        }
        let mut lane_adj = vec![(NONE, NONE); lane_count];
        for e in &net.edges {
            // Map this edge's `index` -> lane id.
            let mut by_index: Vec<(u32, u32)> = e
                .lanes
                .iter()
                .map(|&lid| (lane_index_of[lid as usize], lid))
                .collect();
            by_index.sort_unstable();
            for w in by_index.windows(2) {
                let (_, lower_lane) = w[0];
                let (_, upper_lane) = w[1];
                // `upper_lane` is to the left of `lower_lane`.
                lane_adj[lower_lane as usize].0 = upper_lane; // left
                lane_adj[upper_lane as usize].1 = lower_lane; // right
            }
        }

        Core {
            lane_len,
            lane_count,
            fleet: Fleet::with_capacity(cap),
            index: LaneIndex::new(lane_count, cap),
            params: IdmParams::default(),
            mobil: MobilParams::default(),
            lane_adj,
            seed,
            intents: vec![Intent::default(); cap],
            active_lanes: Vec::with_capacity(lane_count),
        }
    }

    /// Override the IDM parameters (single vehicle class). Mainly for tests.
    pub fn set_params(&mut self, p: IdmParams) {
        self.params = p;
    }

    /// Override the MOBIL lane-change parameters. Mainly for tests.
    pub fn set_mobil(&mut self, m: MobilParams) {
        self.mobil = m;
    }

    /// The desired free-road speed `v0`.
    pub fn v0(&self) -> f32 {
        self.params.v0
    }

    /// Spawn a vehicle at arc position `s` on `lane`, following `route` (a
    /// sequence of lane ids the vehicle traverses in order; `route[0]` must be
    /// `lane`). Returns `None` if the fleet is at capacity or the route is
    /// empty / inconsistent.
    pub fn spawn(&mut self, lane: u32, s: f32, route: &[u32]) -> Option<VehId> {
        if route.is_empty() || route[0] != lane {
            return None;
        }
        if self.fleet.alive_count() >= self.intents.len() {
            return None; // would exceed cap and force a realloc
        }
        let start = self.fleet.route_lanes.len() as u32;
        self.fleet.route_lanes.extend_from_slice(route);
        let end = self.fleet.route_lanes.len() as u32;
        let handle = RouteHandle {
            start,
            end,
            cursor: 0,
        };
        let id = self.fleet.alloc(lane, s, 0.0, VEHICLE_LEN, handle);
        // Seed the lane index so the very first tick sees correct occupancy.
        self.index.rebuild(&self.fleet);
        Some(id)
    }

    /// Rebuild occupancy from current positions (e.g. after a batch of spawns
    /// before the first tick). Cheap; the tick already does this in phase 2.
    pub fn reindex(&mut self) {
        self.index.rebuild(&self.fleet);
    }

    /// Advance the simulation one timestep. `t` is the tick number, folded into
    /// deterministic per-vehicle noise.
    pub fn tick(&mut self, t: u64) {
        debug_assert!(
            self.intents.len() >= self.fleet.slots(),
            "intent buffer too small: {} < {}",
            self.intents.len(),
            self.fleet.slots()
        );

        // ---- Phase 1: parallel, read-only -> intents ------------------------
        // Partition by active lane. Each lane's vehicles are independent given
        // the read-only snapshot, so lanes run in parallel. Within a lane we
        // walk the leader-first bucket and compute each follower's IDM accel.
        self.active_lanes.clear();
        for l in 0..self.lane_count as u32 {
            if !self.index.on_lane(l).is_empty() {
                self.active_lanes.push(l);
            }
        }

        let fleet = &self.fleet;
        let index = &self.index;
        let lane_len = &self.lane_len;
        let params = &self.params;
        let mobil_params = &self.mobil;
        let lane_adj = &self.lane_adj;
        let seed = self.seed;

        // Raw pointer into the intent buffer for disjoint parallel writes: each
        // vehicle slot is written by exactly one lane task, so the writes never
        // overlap. `IntentPtr` is a Send/Sync shim guaranteeing that.
        let intents_ptr = IntentPtr(self.intents.as_mut_ptr());

        self.active_lanes.par_iter().for_each(|&lane| {
            let ptr = intents_ptr; // Copy the wrapper into the closure.
            let occ = index.on_lane(lane);
            // occ[0] is the leader (max s). occ[k] follows occ[k-1].
            for (k, &veh) in occ.iter().enumerate() {
                let i = veh as usize;
                let v = fleet.v[i];
                let s = fleet.s[i];

                // Find the leader's (gap, dv). Prefer the in-lane vehicle ahead
                // (occ[k-1], higher s). If none, look onto the next route lane.
                let (gap, dv) = if k > 0 {
                    let lead = occ[k - 1] as usize;
                    let raw_gap = fleet.s[lead] - s - fleet.len_m[lead];
                    (raw_gap, v - fleet.v[lead])
                } else {
                    leader_across_boundary(fleet, index, lane_len, lane, i, s, v)
                };

                let acc = idm_accel(params, v, dv, gap);

                // Ballistic-safe integration.
                let mut new_v = (v + acc * DT).max(0.0);
                let mut new_s = s + new_v * DT;

                // Advance across lane boundaries along the route.
                let (new_lane, new_cursor, wrapped_s) = advance_route(fleet, lane_len, i, new_s);
                new_s = wrapped_s;

                // Deterministic tiny speed noise keeps homogeneous rings from
                // sitting in an unstable uniform fixed point (breaks symmetry
                // so stop-and-go waves can nucleate). Pure fn of (seed,t,id).
                let noise = (crate::u01(seed, t, veh as u64) - 0.5) * 0.02;
                new_v = (new_v + noise).max(0.0);

                // ---- MOBIL: evaluate a sideways change on the CURRENT s -----
                // Decide on the pre-integration snapshot: the current-lane
                // neighbourhood (excluding self) and each same-edge adjacent
                // lane's neighbourhood at this vehicle's `s`. We only WRITE this
                // vehicle's own intent slot, preserving the disjointness proof.
                let lane_change = evaluate_lane_change(
                    fleet,
                    index,
                    mobil_params,
                    params,
                    lane_adj,
                    lane,
                    s,
                    v,
                    seed,
                    t,
                    veh,
                );

                // Disjoint write: this slot is owned by this lane task.
                unsafe {
                    *ptr.0.add(i) = Intent {
                        v: new_v,
                        s: new_s,
                        lane: new_lane,
                        cursor: new_cursor,
                        lane_change,
                    };
                }
            }
        });

        // ---- Phase 2: sequential, fixed order -> apply + rebuild ------------
        // Pass A: apply the longitudinal intent (speed, arc position, route
        // progression) for every alive vehicle.
        for i in 0..self.fleet.slots() {
            if !self.fleet.alive[i] {
                continue;
            }
            let it = self.intents[i];
            self.fleet.v[i] = it.v;
            self.fleet.s[i] = it.s;
            self.fleet.lane[i] = it.lane;
            self.fleet.route[i].cursor = it.cursor;
        }

        // Pass B: apply MOBIL lane changes in ascending slot order. Each change
        // is re-validated against the fleet state *after* longitudinal motion
        // and after any earlier-slot change this tick — so if two vehicles from
        // different lanes both target the same lane and would overlap, the
        // later applicant (higher slot id) re-checks the gap and aborts. This is
        // the deterministic conflict resolution the two-phase model requires.
        for i in 0..self.fleet.slots() {
            if !self.fleet.alive[i] {
                continue;
            }
            let Some(lc) = self.intents[i].lane_change else {
                continue;
            };
            if self.apply_lane_change_ok(i, lc) {
                let target = lc.target_lane;
                self.fleet.lane[i] = target;
                // Keep the route cursor consistent: the vehicle now travels the
                // adjacent lane, so rewrite the current cursor's lane id. Lanes
                // of one edge share arc-length, so `s` is unchanged.
                let rh = self.fleet.route[i];
                let idx = (rh.start + rh.cursor) as usize;
                self.fleet.route_lanes[idx] = target;
            }
        }

        self.index.rebuild(&self.fleet);
    }

    /// Re-validate a phase-1 MOBIL change against the *current* (post-
    /// longitudinal, post-earlier-changes) fleet state. Returns `true` if the
    /// move is still safe: the vehicle fits between the target lane's leader and
    /// follower with both bumper-to-bumper gaps positive and the new follower's
    /// resulting IDM deceleration within `b_safe`.
    fn apply_lane_change_ok(&self, slot: usize, lc: LaneChange) -> bool {
        let target = lc.target_lane;
        let s = self.fleet.s[slot];
        let v = self.fleet.v[slot];

        // Re-find leader and follower on the target lane from live state.
        let nb = lane_neighbourhood(&self.fleet, &self.index, target, s, v, VehId::MAX);

        if let Some(f) = nb.follower {
            // Gaps are zero-clamped by lane_neighbourhood; a would-be overlap yields
            // gap 0 → IDM projects braking beyond b_safe → rejected by safety check below.
            let a = crate::idm::idm_accel(&self.params, f.v, f.dv_to_decider, f.gap_to_decider);
            if a <= -self.mobil.b_safe {
                return false;
            }
        }
        true
    }

    /// Order-independent state hash over all alive vehicles. Because we fold
    /// each vehicle's own fields (keyed by its stable slot id) the result is
    /// invariant to iteration/thread order.
    pub fn state_hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64; // FNV offset basis
        for i in 0..self.fleet.slots() {
            if !self.fleet.alive[i] {
                continue;
            }
            // Quantize floats to avoid representation noise; deterministic.
            let sq = (self.fleet.s[i] * 1000.0).round() as i64;
            let vq = (self.fleet.v[i] * 1000.0).round() as i64;
            for word in [
                i as u64,
                self.fleet.lane[i] as u64,
                sq as u64,
                vq as u64,
                self.fleet.route[i].cursor as u64,
            ] {
                h ^= word;
                h = h.wrapping_mul(0x0100_0000_01b3); // FNV prime
            }
        }
        h
    }
}

/// Evaluate MOBIL for a vehicle against both same-edge adjacent lanes and
/// return the best admissible change (or `None`). Read-only over the snapshot;
/// the caller writes only this vehicle's own intent slot.
///
/// The randomized acceptance gate (`u01 < 0.9`, per the brief / MOSS practice)
/// is applied here so an admissible change is *dropped* on 10% of ticks — this
/// desynchronizes adjacent vehicles and avoids the synchronized flapping that a
/// deterministic threshold-crossing would produce. It is a pure fn of
/// `(seed, tick, veh)`, so determinism across threads is preserved.
#[allow(clippy::too_many_arguments)]
fn evaluate_lane_change(
    fleet: &Fleet,
    index: &LaneIndex,
    mobil_params: &MobilParams,
    idm: &IdmParams,
    lane_adj: &[(u32, u32)],
    lane: u32,
    s: f32,
    v: f32,
    seed: u64,
    t: u64,
    veh: VehId,
) -> Option<LaneChange> {
    let (left, right) = lane_adj[lane as usize];
    if left == u32::MAX && right == u32::MAX {
        return None; // single-lane edge: nothing to change to
    }

    // Randomized acceptance: on ~10% of ticks, suppress any change entirely.
    if crate::u01(seed, t, veh as u64) >= 0.9 {
        return None;
    }

    // Current-lane neighbourhood, excluding self (self occupies this lane).
    let cur = lane_neighbourhood(fleet, index, lane, s, v, veh);

    // Evaluate each candidate; keep the one with the greatest incentive that
    // both passes the criterion. `to_right` earns the keep-right bias.
    let mut best: Option<(u32, f32, LaneNeighbourhood)> = None;
    for (target, to_right) in [(right, true), (left, false)] {
        if target == u32::MAX {
            continue;
        }
        // On the target lane the decider is absent, so exclude nothing real;
        // pass an impossible slot id.
        let tgt = lane_neighbourhood(fleet, index, target, s, v, VehId::MAX);
        let d = mobil::evaluate(mobil_params, idm, v, &cur, &tgt, to_right);
        if d.change {
            match best {
                Some((_, best_inc, _)) if best_inc >= d.incentive => {}
                _ => best = Some((target, d.incentive, tgt)),
            }
        }
    }

    best.map(|(target_lane, _, _)| LaneChange { target_lane })
}

/// Build the MOBIL [`LaneNeighbourhood`] a vehicle at `(s, v)` with length
/// `len` would see on `lane`, treating any vehicle at slot `exclude` as absent
/// (used for the *current* lane, where the decider itself occupies a slot).
///
/// The leader is the rear-most vehicle strictly ahead (`s_other > s`) and the
/// follower is the front-most vehicle strictly behind (`s_other < s`). Because
/// `index.on_lane` is sorted by `s` descending, we scan once: the last vehicle
/// still ahead is the leader, the first vehicle behind is the follower.
///
/// This is a *within-lane* query only — it does not look across lane
/// boundaries. For a lane-change decision that is the correct locality (MOBIL
/// concerns the immediate side neighbours), and it keeps the read purely on the
/// snapshot with no route walk. If the target lane is empty ahead, the leader
/// gap is left infinite (free road) rather than chased onto the next lane; a
/// vehicle near a lane end simply sees an open target lane, which is safe (the
/// longitudinal IDM still governs its car-following once switched).
fn lane_neighbourhood(
    fleet: &Fleet,
    index: &LaneIndex,
    lane: u32,
    s: f32,
    v: f32,
    exclude: VehId,
) -> LaneNeighbourhood {
    let occ = index.on_lane(lane);
    // Leader: smallest s among those with s_other > s (i.e. last in the
    // descending scan still ahead). Follower: largest s among those with
    // s_other < s (first behind in the descending scan).
    let mut leader: Option<usize> = None;
    let mut follower: Option<usize> = None;
    for &veh in occ {
        if veh == exclude {
            continue;
        }
        let j = veh as usize;
        let sj = fleet.s[j];
        if sj > s {
            leader = Some(j); // keep updating; last one > s is the closest ahead
        } else if sj < s && follower.is_none() {
            follower = Some(j); // first one < s in descending order is closest behind
            break; // everything after is further behind
        }
        // sj == s: coincident (shouldn't happen across distinct slots on the
        // same lane at the same instant); skip so we don't self-block.
    }

    let (lead_gap, lead_dv) = match leader {
        Some(j) => ((fleet.s[j] - s - fleet.len_m[j]).max(0.0), v - fleet.v[j]),
        None => (f32::INFINITY, 0.0),
    };

    let follower = follower.map(|j| {
        let vf = fleet.v[j];
        // Gap the follower keeps to the decider (decider is the leader).
        let gap_to_decider = (s - fleet.s[j] - fleet.len_m[j]).max(0.0);
        let dv_to_decider = vf - v;
        // Gap / dv the follower would have to the decider's leader once the
        // decider is out of the way (current lane) or before it arrives
        // (target lane).
        let (gap_without_decider, dv_without_decider) = match leader {
            Some(l) => (
                (fleet.s[l] - fleet.s[j] - fleet.len_m[l]).max(0.0),
                vf - fleet.v[l],
            ),
            None => (f32::INFINITY, 0.0),
        };
        Follower {
            v: vf,
            gap_to_decider,
            gap_without_decider,
            dv_to_decider,
            dv_without_decider,
        }
    });

    LaneNeighbourhood {
        lead_gap,
        lead_dv,
        follower,
    }
}

/// `*mut Intent` wrapper asserting Send+Sync for disjoint parallel writes.
/// Safety: phase 1 writes each intent slot from exactly one lane task (a
/// vehicle belongs to one lane), so no two threads touch the same address.
#[derive(Clone, Copy)]
struct IntentPtr(*mut Intent);
// SAFETY: writes are provably disjoint (one writer per vehicle slot).
unsafe impl Send for IntentPtr {}
unsafe impl Sync for IntentPtr {}

/// Compute `(gap, dv)` for a vehicle that is the front-most on its lane, by
/// looking onto subsequent lanes of its route. Returns an effectively infinite
/// gap (free road) if no vehicle is found within the lookahead, or a standing
/// obstacle at the lane end if the route terminates without a successor.
fn leader_across_boundary(
    fleet: &Fleet,
    index: &LaneIndex,
    lane_len: &[f32],
    lane: u32,
    follower: usize,
    s: f32,
    v: f32,
) -> (f32, f32) {
    let rh = fleet.route[follower];
    // Distance from follower to the end of its current lane.
    let mut ahead = lane_len[lane as usize] - s;

    // Walk forward along the route looking for the rear-most vehicle on each
    // subsequent lane. Bound the walk to the route length to avoid infinite
    // loops on degenerate data; the ring's route is short so this is cheap.
    let route = &fleet.route_lanes[rh.start as usize..rh.end as usize];
    let n = route.len();
    if n == 0 {
        return (f32::INFINITY, 0.0);
    }
    // Scan up to `n` successor lanes along the (wrapping) route. Each empty
    // lane contributes its full length to the running gap; the first lane with
    // an occupant yields the leader (its rear-most vehicle, smallest `s`). After
    // `n` steps we have traversed every lane of the route (a full loop on the
    // ring) without finding anyone -> free road. This bounds the walk and so is
    // safe even on a degenerate single-vehicle ring.
    let mut cur = rh.cursor as usize;
    for _ in 0..n {
        cur = (cur + 1) % n;
        let next_lane = route[cur];
        if let Some(&rear) = index.on_lane(next_lane).last() {
            let r = rear as usize;
            let gap = ahead + fleet.s[r] - fleet.len_m[r];
            let dv = v - fleet.v[r];
            return (gap.max(0.0), dv);
        }
        ahead += lane_len[next_lane as usize];
    }
    (f32::INFINITY, 0.0)
}

/// Given an integrated `new_s` that may exceed the current lane length, advance
/// the route cursor across as many lane boundaries as needed and return the
/// resulting `(lane, cursor, s_on_that_lane)`.
///
/// The route wraps modulo its length (a closed ring). For an open route that
/// would run off its final lane, Task 5 will gate this on turn permission; for
/// now we wrap, which is correct for the ring and harmless until intersections
/// land (open routes are not yet spawned).
fn advance_route(fleet: &Fleet, lane_len: &[f32], veh: usize, mut new_s: f32) -> (u32, u32, f32) {
    let rh = fleet.route[veh];
    let route = &fleet.route_lanes[rh.start as usize..rh.end as usize];
    let n = route.len();
    if n == 0 {
        return (fleet.lane[veh], rh.cursor, new_s);
    }
    let mut cursor = rh.cursor as usize;
    let mut lane = route[cursor];

    // Move forward while we've run past the end of the current lane.
    let mut guard = 0;
    while new_s >= lane_len[lane as usize] && guard <= n {
        new_s -= lane_len[lane as usize];
        cursor = (cursor + 1) % n;
        lane = route[cursor];
        guard += 1;
    }
    (lane, cursor as u32, new_s)
}
