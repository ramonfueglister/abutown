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

use crate::fleet::{Fleet, LaneIndex, VehId};
use crate::idm::{IdmParams, idm_accel};
use crate::junction::{self, APPROACH_ZONE_M, JunctionModel, MANDATORY_ZONE_M, NodeOccupancy};
use crate::mobil::{self, Follower, LaneNeighbourhood, MobilParams};
use rayon::prelude::*;
use traffic_net::TrafficNet;

/// Simulation timestep (s). Fixed; the integrator is ballistic-safe at this dt.
pub const DT: f32 = 0.1;

/// A read-only snapshot of one alive vehicle, for consumers outside the kernel
/// (the server's re-routing decision and per-tick publish seam).
#[derive(Debug, Clone, Copy)]
pub struct VehicleView {
    /// Lane id the vehicle currently occupies.
    pub lane: u32,
    /// Edge id of that lane.
    pub edge: u32,
    /// Arc position along the lane (m).
    pub s: f32,
    /// Speed (m/s).
    pub v: f32,
}

/// Default vehicle length (m) used for bumper-to-bumper gaps.
const VEHICLE_LEN: f32 = 4.5;

/// Small setback (m) from a lane end where a held vehicle waits — the "stop
/// line". Keeps a blocked vehicle strictly short of the boundary so
/// [`advance_route`] never advances its cursor while it waits.
const STOP_LINE_EPS: f32 = 0.05;

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
    /// If this tick's longitudinal motion would carry the vehicle across a node
    /// (advancing the route cursor onto the next lane), the turn id it would
    /// take. Phase 2 arbitrates conflict-point occupancy for these; a vehicle
    /// that loses is held at the stop line. `u32::MAX` = no crossing this tick.
    cross_turn: u32,
    /// The lane the vehicle *starts* the tick on (its current lane), needed by
    /// phase 2 to hold it in place (revert to the stop line) if it loses the
    /// conflict-point arbitration.
    from_lane: u32,
    /// Arc position clamped to the current lane's stop line, used when a hold
    /// reverts a would-be crossing.
    stop_s: f32,
    /// The route cursor before any crossing, restored on a hold.
    from_cursor: u32,
}

/// Sentinel in [`Intent::cross_turn`]: this vehicle is not crossing a node this
/// tick, so phase 2 skips conflict-point arbitration for it.
const NO_CROSS: u32 = u32::MAX;

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
            cross_turn: NO_CROSS,
            from_lane: 0,
            stop_s: 0.0,
            from_cursor: 0,
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

    /// The baked network. Cloned once at construction so phase-2 conflict-point
    /// arbitration can consult `turns_from` / `conflictsWith` without threading
    /// a borrow through the kernel. Not touched on the parallel hot path except
    /// through the immutable [`JunctionModel`].
    net: TrafficNet,

    /// Precomputed per-turn signal windows + gap headways (Task 5).
    junction: JunctionModel,

    /// Phase-2 conflict-point occupancy scratch, pre-sized in [`Core::new`].
    occupancy: NodeOccupancy,

    /// Reusable phase-1 output buffer, one slot per vehicle slot. Sized to
    /// `cap` at construction; never reallocated in the tick hot path.
    intents: Vec<Intent>,

    /// The lane ids that currently hold at least one vehicle, recomputed each
    /// tick for the parallel partition. Pre-sized to `lane_count`.
    active_lanes: Vec<u32>,

    /// Slots that finished their route this tick, collected in phase-2 and freed
    /// after the occupancy pass. Pre-sized to `cap`; reused each tick.
    despawn: Vec<VehId>,
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

        // Task 5 indexes `net.lanes` / `net.turns` directly by id in the tick
        // path (turn lookup, edge-of-lane, conflict lists). Assert id == index
        // for both so those O(1) accesses are sound — the real bake satisfies
        // this and a fixture that doesn't is a hard error, not a silent wrong
        // answer.
        for (i, l) in net.lanes.iter().enumerate() {
            assert!(
                l.id as usize == i,
                "lane id {} not equal to its index {i}; Core requires id == index",
                l.id
            );
        }
        for (i, tn) in net.turns.iter().enumerate() {
            assert!(
                tn.id as usize == i,
                "turn id {} not equal to its index {i}; Core requires id == index",
                tn.id
            );
        }

        let junction = JunctionModel::build(net);
        let occupancy = NodeOccupancy::new(net);

        Core {
            lane_len,
            lane_count,
            fleet: Fleet::with_capacity(cap),
            index: LaneIndex::new(lane_count, cap),
            params: IdmParams::default(),
            mobil: MobilParams::default(),
            lane_adj,
            seed,
            net: net.clone(),
            junction,
            occupancy,
            intents: vec![Intent::default(); cap],
            active_lanes: Vec::with_capacity(lane_count),
            despawn: Vec::with_capacity(cap),
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
        let id = self.fleet.alloc(lane, s, 0.0, VEHICLE_LEN, route);
        // Seed the lane index so the very first tick sees correct occupancy.
        self.index.rebuild(&self.fleet);
        Some(id)
    }

    /// Rebuild occupancy from current positions (e.g. after a batch of spawns
    /// before the first tick). Cheap; the tick already does this in phase 2.
    pub fn reindex(&mut self) {
        self.index.rebuild(&self.fleet);
    }

    /// Re-route an alive vehicle onto `new_route` in place, keeping its current
    /// lane and arc position. `new_route[0]` **must** equal the vehicle's
    /// current lane so the swap is a continuation, not a teleport; the cursor
    /// resets to 0 (the head of the new route). Returns `false` — leaving the
    /// old route untouched — if the vehicle is not alive, the route is empty,
    /// or its head is not the current lane. Used by the server's periodic
    /// congestion re-routing (Task 7); the kernel itself never re-routes.
    ///
    /// Like [`spawn`](Self::spawn), the new route is written into the slot's own
    /// route buffer (clear + refill, capacity retained), so route storage stays
    /// bounded by the live fleet — reroute churn does not grow memory.
    pub fn reroute(&mut self, veh: VehId, new_route: &[u32]) -> bool {
        let i = veh as usize;
        if i >= self.fleet.slots() || !self.fleet.alive[i] {
            return false;
        }
        if new_route.is_empty() || new_route[0] != self.fleet.lane[i] {
            return false;
        }
        self.fleet.set_route(veh, new_route);
        true
    }

    /// The edge id a lane belongs to, for route bookkeeping outside the kernel.
    pub fn edge_of_lane(&self, lane: u32) -> u32 {
        self.net.lanes[lane as usize].edge
    }

    /// Length (m) of `lane`, from the precomputed dense table. Read-only seam
    /// for consumers that need to detect "at the end of a lane" (e.g. the
    /// citizen-trip bridge's destination-edge arrival check).
    pub fn lane_len(&self, lane: u32) -> f32 {
        self.lane_len[lane as usize]
    }

    /// Remove an alive vehicle from the kernel outside the tick's own
    /// end-of-route path (external consumer despawn, e.g. the citizen-trip
    /// bridge freeing a car that reached its destination edge). Returns
    /// `false` for a free/out-of-range slot. Rebuilds the occupancy index so
    /// the next tick's phase 1 never sees the freed slot as a ghost leader.
    ///
    /// Deliberately NOT added to [`Core::despawned_last_tick`]: that buffer is
    /// the kernel's own end-of-route observation seam; external despawns are
    /// the caller's bookkeeping.
    pub fn despawn(&mut self, veh: VehId) -> bool {
        let i = veh as usize;
        if i >= self.fleet.slots() || !self.fleet.alive[i] {
            return false;
        }
        self.fleet.free(veh);
        self.index.rebuild(&self.fleet);
        true
    }

    /// Read-only view of an alive vehicle's `(lane, edge)` and remaining route
    /// as edge ids, for the server's re-routing / snapshot seam. Returns `None`
    /// for a free slot.
    pub fn vehicle_view(&self, veh: VehId) -> Option<VehicleView> {
        let i = veh as usize;
        if i >= self.fleet.slots() || !self.fleet.alive[i] {
            return None;
        }
        let lane = self.fleet.lane[i];
        Some(VehicleView {
            lane,
            edge: self.net.lanes[lane as usize].edge,
            s: self.fleet.s[i],
            v: self.fleet.v[i],
        })
    }

    /// The vehicle slots despawned by the most recent [`Core::tick`] call —
    /// every end-of-route removal, including gateway-sink arrivals (a route
    /// ending on a boundary stub's in-lane is a normal route end). Read-only
    /// observation seam for the shell's conservation audit; the buffer is
    /// cleared at the start of the next tick.
    pub fn despawned_last_tick(&self) -> &[VehId] {
        &self.despawn
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
        let net = &self.net;
        let junction = &self.junction;

        // Raw pointer into the intent buffer for disjoint parallel writes: each
        // vehicle slot is written by exactly one lane task, so the writes never
        // overlap. `IntentPtr` is a Send/Sync shim guaranteeing that.
        let intents_ptr = IntentPtr(self.intents.as_mut_ptr());

        self.active_lanes.par_iter().for_each(|&lane| {
            let ptr = intents_ptr; // Copy the wrapper into the closure.
            let occ = index.on_lane(lane);
            let this_lane_len = lane_len[lane as usize];
            // occ[0] is the leader (max s). occ[k] follows occ[k-1].
            for (k, &veh) in occ.iter().enumerate() {
                let i = veh as usize;
                let v = fleet.v[i];
                let s = fleet.s[i];

                // Find the leader's (gap, dv). Prefer the in-lane vehicle ahead
                // (occ[k-1], higher s). If none, look onto the next route lane.
                let (mut gap, mut dv) = if k > 0 {
                    let lead = occ[k - 1] as usize;
                    let raw_gap = fleet.s[lead] - s - fleet.len_m[lead];
                    (raw_gap, v - fleet.v[lead])
                } else {
                    leader_across_boundary(fleet, index, net, lane_len, lane, i, s, v)
                };

                // ---- Junction gate: is the next node crossable this tick? ----
                // Only the front-most-relevant vehicles near a lane end consult
                // it; deeper followers are governed by their in-lane leader. A
                // blocked crossing injects a v=0 phantom at the stop line (lane
                // end), composed with the leader gap via `min` so a red light
                // always beats a distant cross-boundary leader (carry-forward b).
                let dist_to_end = this_lane_len - s;
                // The vehicle's next boundary crossing (if it has a successor
                // lane on its route): `Some(turn)` when a turn connects, `None`
                // when the route ends here (open route → despawn), and a
                // "blocked" verdict when the pair has no turn (bake defect —
                // treat the lane end as a hard wall rather than teleport).
                let boundary = if dist_to_end <= APPROACH_ZONE_M {
                    route_boundary(fleet, net, i, lane)
                } else {
                    Boundary::NotYet
                };

                let mut blocked = false;
                let mut route_end = false;
                let next_turn = match boundary {
                    Boundary::Turn(turn) => {
                        if !junction_allows(fleet, index, lane_len, junction, net, turn, t) {
                            blocked = true;
                            let stop_gap = dist_to_end;
                            if stop_gap < gap {
                                gap = stop_gap;
                                dv = v;
                            }
                        }
                        turn
                    }
                    // Route ends here or the pair is a bake defect: in both cases
                    // do not cross. A genuine route end (RouteEnd) despawns in
                    // phase 2; a defect (NoTurn) holds at the wall so a later
                    // integration never teleports the vehicle.
                    Boundary::RouteEnd => {
                        route_end = true;
                        NO_CROSS
                    }
                    Boundary::NoTurn => {
                        blocked = true;
                        let stop_gap = dist_to_end;
                        if stop_gap < gap {
                            gap = stop_gap;
                            dv = v;
                        }
                        NO_CROSS
                    }
                    Boundary::NotYet => NO_CROSS,
                };

                let acc = idm_accel(params, v, dv, gap);

                // Ballistic-safe integration.
                let mut new_v = (v + acc * DT).max(0.0);
                let mut new_s = s + new_v * DT;

                // Blocked crossing or route end: clamp just short of the lane
                // end so `advance_route` keeps the cursor on the current lane
                // (no cross). Route-end vehicles sit at the stop line until
                // phase 2 despawns them.
                if (blocked || route_end) && new_s >= this_lane_len {
                    new_s = this_lane_len - STOP_LINE_EPS;
                }

                // Advance across lane boundaries along the route.
                let (new_lane, new_cursor, wrapped_s) = advance_route(fleet, lane_len, i, new_s);

                // Detect whether this longitudinal step crosses a node (cursor
                // advanced onto the next route lane). If so, record the turn so
                // phase 2 can arbitrate conflict-point occupancy sequentially.
                let cross_turn = if new_lane != lane && !blocked && !route_end {
                    next_turn
                } else {
                    NO_CROSS
                };
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
                // Turn-awareness (carry-forward a): within the mandatory zone of
                // a lane end, restrict changes to lanes that still serve the
                // route (see `evaluate_lane_change`).
                let lane_change = evaluate_lane_change(
                    fleet,
                    index,
                    mobil_params,
                    params,
                    lane_adj,
                    net,
                    lane,
                    s,
                    v,
                    dist_to_end,
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
                        cross_turn,
                        from_lane: lane,
                        stop_s: this_lane_len - STOP_LINE_EPS,
                        from_cursor: fleet.route[i].cursor,
                    };
                }
            }
        });

        // ---- Phase 2: sequential, fixed order -> apply + rebuild ------------
        // Pass A: apply the longitudinal intent (speed, arc position, route
        // progression) for every alive vehicle. Vehicles that cross a node this
        // tick must first win the conflict-point arbitration (fixed slot order,
        // so it is deterministic and thread-independent); a loser is held at the
        // stop line on its origin lane. This is the ONLY node bookkeeping and it
        // lives entirely here in the sequential apply.
        self.occupancy.begin_tick(t);
        self.despawn.clear();
        for i in 0..self.fleet.slots() {
            if !self.fleet.alive[i] {
                continue;
            }
            let it = self.intents[i];

            if it.cross_turn != NO_CROSS {
                // This vehicle would cross a node this tick. Two gates remain:
                // (1) route completion — an open route with no onward turn ends
                //     here → despawn; (2) conflict-point occupancy.
                let turn = it.cross_turn;
                let node = self.junction.node_of(turn);
                let conflicts = &self.net.turns[turn as usize].conflicts_with;
                if self.occupancy.try_claim(&self.net, node, turn, conflicts) {
                    // Won: commit the crossed state. The turn's `toLane` is the
                    // lane actually entered (authoritative over the authored
                    // route lane, which MOBIL may have desynced); rewrite the
                    // route lane at the new cursor so downstream lookups agree.
                    let mut to_lane = self.net.turns[turn as usize].to_lane;

                    // Keep-right at the junction (carry-forward a + European
                    // keep-right): MOBIL is turn-unaware and its free-road return
                    // incentive can sit exactly at threshold, so a vehicle that
                    // overtook into a left lane would otherwise never drift back.
                    // On crossing, re-seat it into the *rightmost* same-edge lane
                    // that still serves the route and is clear ahead at the entry
                    // point. It only shifts right (never left), so it never
                    // abandons an in-progress overtake, and the clear-ahead gate
                    // stops it cutting back in front of a slower vehicle.
                    to_lane = self.rightmost_clear_entry(i, to_lane);

                    self.fleet.v[i] = it.v;
                    self.fleet.s[i] = it.s;
                    self.fleet.lane[i] = to_lane;
                    self.fleet.route[i].cursor = it.cursor;
                    let cur = self.fleet.route[i].cursor as usize;
                    let route = &mut self.fleet.route_lanes[i];
                    if cur < route.len() {
                        route[cur] = to_lane;
                    }
                } else {
                    // Lost: hold at the stop line on the origin lane.
                    self.fleet.v[i] = 0.0;
                    self.fleet.s[i] = it.stop_s;
                    self.fleet.lane[i] = it.from_lane;
                    self.fleet.route[i].cursor = it.from_cursor;
                }
                continue;
            }

            // Non-crossing: apply longitudinal intent directly.
            self.fleet.v[i] = it.v;
            self.fleet.s[i] = it.s;
            self.fleet.lane[i] = it.lane;
            self.fleet.route[i].cursor = it.cursor;

            // Route completion for open (non-looping) routes: a vehicle sitting
            // at the end of its final route lane with no onward turn despawns.
            // Ring routes always have an onward turn back to route[0], so they
            // never trip this and keep looping.
            if route_completed(&self.fleet, &self.net, &self.lane_len, i) {
                self.despawn.push(i as VehId);
            }
        }

        // Free completed routes after the pass so slot reuse can't disturb the
        // fixed-order apply above.
        for &id in &self.despawn {
            self.fleet.free(id);
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
            // Skip MOBIL for a vehicle that crossed a node this tick: its phase-1
            // sideways decision referenced the *pre-crossing* lane/edge and is
            // now stale (the target lane belongs to the edge it just left). It
            // re-evaluates cleanly on its new edge next tick. Without this, a
            // stale target would move the vehicle onto a lane of the wrong edge,
            // which — near a node — is exactly where the keep-right return would
            // otherwise be applied to the wrong lane.
            if self.intents[i].cross_turn != NO_CROSS {
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
                let cur = self.fleet.route[i].cursor as usize;
                self.fleet.route_lanes[i][cur] = target;

                // Route reconciliation (carry-forward a): MOBIL is turn-unaware,
                // so after moving sideways the next planned route lane may no
                // longer be reachable from `target` (its turn departed the old
                // lane). Re-map the next hop to an equivalent lane on the *same
                // next edge* that `target` actually has a turn to. If none
                // exists the mandatory-lane-light gate would have suppressed the
                // change near the node; far from the node we leave it and the
                // junction gate will hold the vehicle (surfacing the mismatch)
                // rather than teleport it.
                let next_idx = cur + 1;
                if next_idx < self.fleet.route_lanes[i].len() {
                    let planned_next = self.fleet.route_lanes[i][next_idx];
                    let next_edge = self.net.lanes[planned_next as usize].edge;
                    // Already reachable? Then nothing to do.
                    if junction::turn_between(&self.net, target, planned_next).is_none() {
                        // Find a turn from `target` onto the same next edge.
                        if let Some(remapped) = self
                            .net
                            .turns_from(target)
                            .iter()
                            .map(|&tid| self.net.turns[tid as usize].to_lane)
                            .find(|&tl| self.net.lanes[tl as usize].edge == next_edge)
                        {
                            self.fleet.route_lanes[i][next_idx] = remapped;
                        }
                    }
                }
            }
        }

        self.index.rebuild(&self.fleet);
    }

    /// Choose the lane a crossing vehicle should enter: starting from the turn's
    /// `to_lane`, drift as far *right* (lower index) as possible while the
    /// candidate still serves the next route edge and its entry region is clear.
    /// Returns `to_lane` unchanged if no rightward shift is admissible.
    ///
    /// `slot`'s intent position (`intents[slot].s`) is the arc length at which
    /// it enters the new lane; we clear-check each candidate against the live
    /// occupancy at that `s`. Only rightward shifts are considered, so an
    /// in-progress overtake (which moved left) is never undone mid-manoeuvre.
    fn rightmost_clear_entry(&self, slot: usize, to_lane: u32) -> u32 {
        // Next route edge the entered lane must keep serving (if any).
        let cursor = self.fleet.route[slot].cursor as usize;
        let route = self.fleet.route_slice(slot);
        let next_edge = route
            .get(cursor + 1)
            .map(|&nl| self.net.lanes[nl as usize].edge);

        let entry_s = self.intents[slot].s;
        let veh_len = self.fleet.len_m[slot];

        let mut chosen = to_lane;
        // Walk right neighbours; accept the furthest-right admissible one.
        let mut cand = self.lane_adj[chosen as usize].1; // right of `chosen`
        while cand != u32::MAX {
            // Must still serve the route.
            let serves = match next_edge {
                Some(e) => turn_onto_edge(&self.net, cand, e).is_some(),
                None => true,
            };
            // Entry region clear: nearest vehicle ahead on `cand` leaves a gap.
            let clear = self.entry_gap_clear(cand, entry_s, veh_len);
            if serves && clear {
                chosen = cand;
                cand = self.lane_adj[chosen as usize].1;
            } else {
                break;
            }
        }
        chosen
    }

    /// Whether entering `lane` at arc `s` (vehicle length `len`) leaves a safe
    /// bumper gap to the nearest vehicle ahead on that lane (>= a few car
    /// lengths). Conservative: a tight lane is left alone so the vehicle stays
    /// where it crossed.
    fn entry_gap_clear(&self, lane: u32, s: f32, len: f32) -> bool {
        const MIN_ENTRY_GAP_M: f32 = 15.0;
        // Occupancy is leader-first (descending s). Find the nearest vehicle
        // ahead of `s` and behind it; require both gaps comfortable.
        let occ = self.index.on_lane(lane);
        for &veh in occ {
            let j = veh as usize;
            let sj = self.fleet.s[j];
            let gap = if sj >= s {
                sj - s - len
            } else {
                s - sj - self.fleet.len_m[j]
            };
            if gap < MIN_ENTRY_GAP_M {
                return false;
            }
        }
        true
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

        // Parallel lanes of one edge do NOT always share arc length (curved
        // edges bake different polyline lengths per lane). A change that
        // preserves `s` beyond the target lane's end would place the vehicle
        // past the junction ungated — the boundary-advance then carries it
        // onto the next lane on top of whatever queues there (observed as a
        // real collision on the Gemeinde net: 134.2 m → 131.1 m lanes).
        if s > self.lane_len[target as usize] {
            return false;
        }

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
    net: &TrafficNet,
    lane: u32,
    s: f32,
    v: f32,
    dist_to_end: f32,
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

    // Mandatory-lane-light (carry-forward a): within `MANDATORY_ZONE_M` of the
    // lane end, MOBIL is turn-unaware and would happily rewrite the route cursor
    // onto a lane with no turn for the vehicle's next edge — stranding it. So
    // near the node we only permit changes onto a lane that can still serve the
    // route: one with a turn whose `toLane` lies on the same edge as the route's
    // planned next lane. Away from the node (dist_to_end > zone) MOBIL is
    // unrestricted, as before.
    let restrict = dist_to_end <= MANDATORY_ZONE_M;
    let next_edge = if restrict {
        route_next_edge(fleet, net, veh as usize)
    } else {
        None
    };

    // Current-lane neighbourhood, excluding self (self occupies this lane).
    let cur = lane_neighbourhood(fleet, index, lane, s, v, veh);

    // Evaluate each candidate; keep the one with the greatest incentive that
    // both passes the criterion. `to_right` earns the keep-right bias.
    let mut best: Option<(u32, f32)> = None;
    for (target, to_right) in [(right, true), (left, false)] {
        if target == u32::MAX {
            continue;
        }
        // Turn-awareness gate: if restricted and the target lane can't serve the
        // route's next edge, skip it.
        if let Some(edge) = next_edge
            && !lane_serves_edge(net, target, edge)
        {
            continue;
        }
        // On the target lane the decider is absent, so exclude nothing real;
        // pass an impossible slot id.
        let tgt = lane_neighbourhood(fleet, index, target, s, v, VehId::MAX);
        let d = mobil::evaluate(mobil_params, idm, v, &cur, &tgt, to_right);
        if d.change {
            match best {
                Some((_, best_inc)) if best_inc >= d.incentive => {}
                _ => best = Some((target, d.incentive)),
            }
        }
    }

    best.map(|(target_lane, _)| LaneChange { target_lane })
}

/// The edge id of the route's *next* lane (the lane after the cursor), or `None`
/// if the vehicle is on its final route lane. Used by the mandatory-lane gate.
fn route_next_edge(fleet: &Fleet, net: &TrafficNet, veh: usize) -> Option<u32> {
    let cursor = fleet.route[veh].cursor as usize;
    let route = fleet.route_slice(veh);
    let next_lane = *route.get(cursor + 1)?;
    Some(net.lanes[next_lane as usize].edge)
}

/// Whether `lane` has any turn leading onto a lane of `edge` — i.e. it can serve
/// a route whose next hop is on `edge`. Scans the lane's `turns_from` CSR.
fn lane_serves_edge(net: &TrafficNet, lane: u32, edge: u32) -> bool {
    net.turns_from(lane).iter().any(|&tid| {
        let to_lane = net.turns[tid as usize].to_lane;
        net.lanes[to_lane as usize].edge == edge
    })
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
#[allow(clippy::too_many_arguments)]
fn leader_across_boundary(
    fleet: &Fleet,
    index: &LaneIndex,
    net: &TrafficNet,
    lane_len: &[f32],
    lane: u32,
    follower: usize,
    s: f32,
    v: f32,
) -> (f32, f32) {
    let cursor = fleet.route[follower].cursor as usize;
    // Distance from follower to the end of its current lane.
    let mut ahead = lane_len[lane as usize] - s;

    // Walk forward along the route looking for the rear-most vehicle on each
    // subsequent lane. Bound the walk to the route length to avoid infinite
    // loops on degenerate data; the ring's route is short so this is cheap.
    let route = fleet.route_slice(follower);
    let n = route.len();
    if n == 0 {
        return (f32::INFINITY, 0.0);
    }
    // Scan up to `n` successor lanes along the route. Each empty lane
    // contributes its full length to the running gap; the first lane with an
    // occupant yields the leader (its rear-most vehicle, smallest `s`). After
    // `n` steps every lane of the route has been traversed (a full loop, for a
    // *closed* ring route) without finding anyone -> free road.
    //
    // Wrapping past the route's last lane is only valid when the route
    // genuinely loops (a turn connects the final lane back onto the first
    // route lane's edge) — this is what makes ring fixtures work. For an open
    // route (e.g. a dead end with no such turn) there is nothing beyond the
    // final lane: treat it as free road rather than wrapping the cursor back
    // to index 0, which would otherwise phantom-leader this vehicle off an
    // unrelated queue sitting on the route's FIRST lane (e.g. traffic queued
    // behind a signal far behind this vehicle, on a completely different part
    // of the network). See `route_completed`, which makes the same
    // loop-vs-open distinction for despawn.
    let last_lane = route[n - 1];
    let is_loop = junction::turn_between(net, last_lane, route[0]).is_some();
    if !is_loop && cursor + 1 >= n {
        // Already on (or beyond) the final lane of an open route: nothing
        // ahead but free road until the route completes.
        return (f32::INFINITY, 0.0);
    }

    let mut cur = cursor;
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
    let start_cursor = fleet.route[veh].cursor;
    let route = fleet.route_slice(veh);
    let n = route.len();
    if n == 0 {
        return (fleet.lane[veh], start_cursor, new_s);
    }
    let mut cursor = start_cursor as usize;
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

/// What lies at the end of a vehicle's current route lane.
enum Boundary {
    /// Too far from the lane end to matter yet.
    NotYet,
    /// The next route lane is reached via this turn id.
    Turn(u32),
    /// The route ends on this lane (no successor) — an open route completing.
    RouteEnd,
    /// The route has a successor lane but the net has no turn connecting them
    /// (a bake defect). Treated as an impassable wall.
    NoTurn,
}

/// Classify the boundary at the end of vehicle `veh`'s current route lane.
///
/// The route is a lane-id sequence, but MOBIL changes which lane the vehicle
/// actually occupies (rewriting the cursor lane and re-mapping the immediate
/// next hop). To stay robust to that, the boundary is matched at **edge**
/// granularity: the transition is carried by any turn from the current `lane`
/// onto the *same edge* as the route's next planned lane. The turn's real
/// `toLane` becomes the lane the vehicle enters (see [`advance_route`], which is
/// kept consistent by rewriting the route lane on crossing). A route with no
/// successor edge (cursor at the end and no loop turn back to the first edge)
/// is an *open* route completing.
fn route_boundary(fleet: &Fleet, net: &TrafficNet, veh: usize, lane: u32) -> Boundary {
    let cursor = fleet.route[veh].cursor as usize;
    let route = fleet.route_slice(veh);
    let n = route.len();
    if n == 0 {
        return Boundary::RouteEnd;
    }
    if cursor + 1 < n {
        let next_edge = net.lanes[route[cursor + 1] as usize].edge;
        match turn_onto_edge(net, lane, next_edge) {
            Some(turn) => Boundary::Turn(turn),
            None => Boundary::NoTurn,
        }
    } else {
        // Last lane: a loop turn back onto the first route lane's edge continues.
        let first_edge = net.lanes[route[0] as usize].edge;
        match turn_onto_edge(net, lane, first_edge) {
            Some(turn) => Boundary::Turn(turn),
            None => Boundary::RouteEnd,
        }
    }
}

/// The first turn from `lane` whose `toLane` lies on `edge`, or `None`. Matches
/// at edge granularity so a MOBIL-shifted vehicle still finds its onward turn.
fn turn_onto_edge(net: &TrafficNet, lane: u32, edge: u32) -> Option<u32> {
    net.turns_from(lane)
        .iter()
        .copied()
        .find(|&tid| net.lanes[net.turns[tid as usize].to_lane as usize].edge == edge)
}

/// Whether a vehicle has completed its (open) route: it sits at the end of its
/// final route lane and there is no onward turn back to the route start.
fn route_completed(fleet: &Fleet, net: &TrafficNet, lane_len: &[f32], veh: usize) -> bool {
    let cursor = fleet.route[veh].cursor as usize;
    let route = fleet.route_slice(veh);
    let n = route.len();
    if n == 0 {
        return true;
    }
    if cursor + 1 < n {
        return false; // still lanes ahead
    }
    // Final lane: a loop continues, so it is not "completed".
    let lane = fleet.lane[veh];
    if junction::turn_between(net, lane, route[0]).is_some() {
        return false;
    }
    // Open route on its final lane: completed once it reaches the lane end.
    fleet.s[veh] >= lane_len[lane as usize] - STOP_LINE_EPS - 1e-3
}

/// Whether the next turn may be crossed this tick: signal green *and* every
/// conflicting approaching vehicle leaves an acceptable gap.
///
/// The gap check scans, for each turn this one yields to, the vehicles on that
/// turn's `fromLane` (the conflicting approach) and rejects if any is closer
/// than the critical time-gap distance to the shared node. Read-only over the
/// phase-1 snapshot; the final crossing authority is phase-2 occupancy.
fn junction_allows(
    fleet: &Fleet,
    index: &LaneIndex,
    lane_len: &[f32],
    junction: &JunctionModel,
    net: &TrafficNet,
    turn: u32,
    t: u64,
) -> bool {
    // Signal gating first (cheap, stateless).
    if !junction.signal_green(turn, t, DT) {
        return false;
    }
    // Gap acceptance: only if this turn yields to something.
    if junction.t_gap(turn) == 0.0 {
        return true;
    }
    let yields_to = &net.turns[turn as usize].yields_to;
    for &other in yields_to {
        let from_lane = net.turns[other as usize].from_lane;
        // The conflicting approach: vehicles on `from_lane`, ranked leader-first
        // (nearest the node = smallest remaining distance). Check the nearest.
        let occ = index.on_lane(from_lane);
        let llen = lane_len[from_lane as usize];
        // occ[0] is the furthest-along (max s) = nearest the node.
        if let Some(&lead) = occ.first() {
            let j = lead as usize;
            let dist_to_conflict = (llen - fleet.s[j]).max(0.0);
            let v_conflict = fleet.v[j];
            if !junction.gap_ok(turn, dist_to_conflict, v_conflict) {
                return false;
            }
        }
    }
    true
}
