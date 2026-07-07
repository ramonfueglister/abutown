//! Structure-of-arrays vehicle fleet and the CSR lane occupancy index.
//!
//! The fleet stores every vehicle's mutable state in parallel `Vec`s indexed
//! by a stable slot ([`VehId`]). Slots are reused via a free-list so spawning
//! after despawn does not grow the arrays unboundedly. All hot per-tick state
//! lives here so the tick loop can iterate contiguous memory.
//!
//! [`LaneIndex`] is a compressed-sparse-row map `lane -> slots on that lane`,
//! with each lane's slots sorted by arc-position `s` **descending** so the
//! leader (furthest along) is first. It is rebuilt every tick's phase 2 from
//! the freshly-written positions; Task 4 (MOBIL) and Task 5 (intersections)
//! read the same structure to find neighbours.

/// Stable vehicle slot index into the [`Fleet`] SoA arrays.
pub type VehId = u32;

/// The vehicle's cursor within its route. The route itself is stored
/// per-slot in [`Fleet::route_lanes`] (indexed by [`VehId`]); this handle just
/// carries the index of the lane the vehicle currently occupies. Kept small and
/// `Copy` so it lives inline in the SoA array.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RouteHandle {
    /// Index into the slot's route (`route_lanes[slot]`) of the lane the vehicle
    /// currently occupies.
    pub cursor: u32,
}

/// SoA vehicle fleet. Index is the [`VehId`] slot; `alive[i] == false` marks a
/// free slot available for reuse.
#[derive(Debug, Default, Clone)]
pub struct Fleet {
    /// Lane id the vehicle currently occupies.
    pub lane: Vec<u32>,
    /// Arc position along the current lane (m from lane start).
    pub s: Vec<f32>,
    /// Speed (m/s).
    pub v: Vec<f32>,
    /// Route span + cursor.
    pub route: Vec<RouteHandle>,
    /// Vehicle length (m), used for bumper-to-bumper gaps.
    pub len_m: Vec<f32>,
    /// Vehicle class (see [`crate::idm::N_CLASSES`]): indexes the kernel's
    /// per-class IDM parameter table and rides the wire for silhouette
    /// selection. Immutable for a slot's occupant lifetime.
    pub class: Vec<u8>,
    /// Slot liveness.
    pub alive: Vec<bool>,

    /// Per-slot route storage: `route_lanes[slot]` is the lane-id sequence the
    /// vehicle in that slot follows. Reused across respawns — `spawn`/`reroute`
    /// `clear()` + `extend` the slot's `Vec`, which retains its capacity, so
    /// after warm-up there is no per-spawn allocation and total storage is
    /// bounded by `cap × max-route-len` (never the append-only unbounded growth
    /// of a single flat buffer). The [`RouteHandle::cursor`] indexes into this.
    pub route_lanes: Vec<Vec<u32>>,

    /// Free-list of despawned slots available for reuse (LIFO).
    free: Vec<VehId>,

    /// Per-slot reuse generation. Incremented every time a slot is freed, so a
    /// slot reused after despawn carries a distinct generation from its prior
    /// occupant. Pure gateway/wire bookkeeping: the kernel never reads this and
    /// [`crate::Core::state_hash`] deliberately excludes it (it has no bearing
    /// on simulation state — two runs that despawn different-but-equivalent
    /// slots must still hash equal). The gateway composes `slot | (gen << 12)`
    /// into a wire-stable vehicle id so a Task-9 client dead-reckoning by id
    /// cannot confuse a recycled slot for its former occupant.
    generation: Vec<u32>,
}

impl Fleet {
    /// A fleet pre-sized for `cap` vehicle slots. Arrays start empty (len 0)
    /// but with `cap` capacity reserved so spawning up to `cap` vehicles never
    /// reallocates.
    pub fn with_capacity(cap: usize) -> Self {
        Fleet {
            lane: Vec::with_capacity(cap),
            s: Vec::with_capacity(cap),
            v: Vec::with_capacity(cap),
            route: Vec::with_capacity(cap),
            len_m: Vec::with_capacity(cap),
            class: Vec::with_capacity(cap),
            alive: Vec::with_capacity(cap),
            route_lanes: Vec::with_capacity(cap),
            free: Vec::new(),
            generation: Vec::with_capacity(cap),
        }
    }

    /// Number of slots ever allocated (alive or free), i.e. the SoA length.
    #[inline]
    pub fn slots(&self) -> usize {
        self.alive.len()
    }

    /// The route (lane-id sequence) a slot currently follows.
    #[inline]
    pub fn route_slice(&self, slot: usize) -> &[u32] {
        &self.route_lanes[slot]
    }

    /// Total lane ids held across all per-slot route buffers (their `len`s), a
    /// probe for the bounded-storage regression test. This is the *used* count,
    /// not reserved capacity; see [`route_storage_capacity`](Self::route_storage_capacity).
    pub fn route_storage_len(&self) -> usize {
        self.route_lanes.iter().map(|r| r.len()).sum()
    }

    /// Total lane-id capacity reserved across all per-slot route buffers. Stays
    /// bounded (does not grow across respawn/reroute churn once warmed up),
    /// which is the leak-freedom the regression test asserts.
    pub fn route_storage_capacity(&self) -> usize {
        self.route_lanes.iter().map(|r| r.capacity()).sum()
    }

    /// Count of currently-alive vehicles.
    pub fn alive_count(&self) -> usize {
        self.alive.iter().filter(|a| **a).count()
    }

    /// Allocate a vehicle slot, reusing a free one if available. The slot's
    /// route buffer is `clear()`ed and re-filled from `route` (capacity retained
    /// on reuse → no per-spawn allocation after warm-up); the cursor resets to
    /// 0. Returns the slot id. Internal: [`crate::Core::spawn`] wraps this.
    pub(crate) fn alloc(
        &mut self,
        lane: u32,
        s: f32,
        v: f32,
        len_m: f32,
        class: u8,
        route: &[u32],
    ) -> VehId {
        let handle = RouteHandle { cursor: 0 };
        if let Some(id) = self.free.pop() {
            let i = id as usize;
            self.lane[i] = lane;
            self.s[i] = s;
            self.v[i] = v;
            self.len_m[i] = len_m;
            self.class[i] = class;
            self.route[i] = handle;
            // Reuse the slot's existing buffer: clear() keeps its capacity.
            let buf = &mut self.route_lanes[i];
            buf.clear();
            buf.extend_from_slice(route);
            self.alive[i] = true;
            id
        } else {
            let id = self.alive.len() as VehId;
            self.lane.push(lane);
            self.s.push(s);
            self.v.push(v);
            self.len_m.push(len_m);
            self.class.push(class);
            self.route.push(handle);
            self.route_lanes.push(route.to_vec());
            self.alive.push(true);
            self.generation.push(0);
            id
        }
    }

    /// Rewrite an alive slot's route in place (used by [`crate::Core::reroute`]).
    /// Clears + refills the slot's route buffer (capacity retained) and resets
    /// the cursor to 0.
    pub(crate) fn set_route(&mut self, id: VehId, route: &[u32]) {
        let i = id as usize;
        let buf = &mut self.route_lanes[i];
        buf.clear();
        buf.extend_from_slice(route);
        self.route[i].cursor = 0;
    }

    /// Mark a slot free for reuse. The slot's route buffer is left in place
    /// (capacity retained) so the next `alloc` reusing this slot pays no
    /// allocation; storage stays bounded by the live slot count.
    pub(crate) fn free(&mut self, id: VehId) {
        let i = id as usize;
        if self.alive[i] {
            self.alive[i] = false;
            // Bump the reuse generation so the next occupant of this slot gets a
            // distinct wire id. Wrapping is fine — the gateway packs only the
            // low 20 bits, and a collision needs 2^20 reuses of the same slot
            // between two frames a client dead-reckons across (never happens).
            self.generation[i] = self.generation[i].wrapping_add(1);
            self.free.push(id);
        }
    }

    /// The current reuse generation of a slot. See [`Fleet::generation`]. The
    /// gateway composes this with the slot id to produce a wire-stable vehicle
    /// id; the kernel itself never consults it.
    #[inline]
    pub fn generation(&self, slot: usize) -> u32 {
        self.generation[slot]
    }
}

/// CSR lane occupancy: for each lane id, the alive slots on it sorted by `s`
/// **descending** (leader first). Rebuilt each tick.
#[derive(Debug, Default, Clone)]
pub struct LaneIndex {
    /// `offsets[lane]..offsets[lane+1]` spans into `slots` for that lane.
    /// Length = `lane_count + 1`.
    offsets: Vec<u32>,
    /// Slot ids, grouped by lane, sorted by `s` descending within each lane.
    slots: Vec<VehId>,
    /// Scratch: per-lane running fill cursor during a rebuild (reused so the
    /// rebuild allocates nothing after construction).
    fill: Vec<u32>,
}

impl LaneIndex {
    /// An index sized for `lane_count` lanes and up to `cap` vehicles. All
    /// backing storage is reserved up front so [`rebuild`](Self::rebuild)
    /// never allocates.
    pub fn new(lane_count: usize, cap: usize) -> Self {
        LaneIndex {
            offsets: vec![0; lane_count + 1],
            slots: Vec::with_capacity(cap),
            fill: vec![0; lane_count],
        }
    }

    /// Slots on `lane`, leader first (descending `s`). Empty slice for an
    /// out-of-range or empty lane.
    #[inline]
    pub fn on_lane(&self, lane: u32) -> &[VehId] {
        let l = lane as usize;
        if l + 1 >= self.offsets.len() {
            return &[];
        }
        let start = self.offsets[l] as usize;
        let end = self.offsets[l + 1] as usize;
        &self.slots[start..end]
    }

    /// Rebuild the index from current fleet positions. Counting-sort into CSR
    /// buckets, then sort each bucket by `s` descending. Allocation-free after
    /// construction (all buffers are reused / truncated in place).
    ///
    /// Determinism: bucket order is lane-id ascending; within a lane, ties in
    /// `s` are broken by slot id ascending so the ordering is total and
    /// thread-independent.
    pub fn rebuild(&mut self, fleet: &Fleet) {
        let lane_count = self.fill.len();

        // Pass 1: count occupants per lane.
        for f in self.fill.iter_mut() {
            *f = 0;
        }
        for i in 0..fleet.slots() {
            if fleet.alive[i] {
                let l = fleet.lane[i] as usize;
                debug_assert!(l < lane_count, "vehicle on out-of-range lane {l}");
                self.fill[l] += 1;
            }
        }

        // Prefix-sum into offsets.
        self.offsets[0] = 0;
        for l in 0..lane_count {
            self.offsets[l + 1] = self.offsets[l] + self.fill[l];
        }

        // Reset fill to per-lane write cursors (= start offset).
        for l in 0..lane_count {
            self.fill[l] = self.offsets[l];
        }

        // Pass 2: scatter slot ids into their lane bucket.
        let total = *self.offsets.last().unwrap() as usize;
        self.slots.clear();
        self.slots.resize(total, 0);
        for i in 0..fleet.slots() {
            if fleet.alive[i] {
                let l = fleet.lane[i] as usize;
                let pos = self.fill[l] as usize;
                self.slots[pos] = i as VehId;
                self.fill[l] += 1;
            }
        }

        // Sort each lane bucket by s descending, tie-break slot id ascending.
        for l in 0..lane_count {
            let start = self.offsets[l] as usize;
            let end = self.offsets[l + 1] as usize;
            let bucket = &mut self.slots[start..end];
            bucket.sort_unstable_by(|&a, &b| {
                let sa = fleet.s[a as usize];
                let sb = fleet.s[b as usize];
                // descending s; NaN-free by construction
                sb.partial_cmp(&sa)
                    .unwrap_or(core::cmp::Ordering::Equal)
                    .then(a.cmp(&b))
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A slot reused via the free-list carries a distinct generation from its
    /// former occupant, so the gateway's composed wire id changes across reuse.
    #[test]
    fn reused_slot_bumps_generation() {
        let mut fleet = Fleet::with_capacity(4);
        let a = fleet.alloc(0, 0.0, 0.0, 4.0, 0, &[0]);
        assert_eq!(fleet.generation(a as usize), 0);

        fleet.free(a);
        // LIFO free-list: the next alloc reuses slot `a`.
        let b = fleet.alloc(0, 0.0, 0.0, 4.0, 0, &[0]);
        assert_eq!(b, a, "LIFO free-list must reuse the just-freed slot");
        assert_eq!(
            fleet.generation(b as usize),
            1,
            "reused slot must advance its generation"
        );
    }
}
