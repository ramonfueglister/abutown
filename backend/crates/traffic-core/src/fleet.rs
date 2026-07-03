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

/// Opaque handle to a vehicle's route: a `[start, end)` span into the fleet's
/// flat `route_lanes` buffer plus the index of the lane the vehicle is
/// currently on. Kept small and `Copy` so it lives inline in the SoA array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteHandle {
    /// Start offset into `Fleet::route_lanes`.
    pub start: u32,
    /// End offset (exclusive) into `Fleet::route_lanes`.
    pub end: u32,
    /// Index within `[start, end)` of the lane the vehicle currently occupies.
    pub cursor: u32,
}

impl RouteHandle {
    /// Number of lanes in the route.
    #[inline]
    pub fn len(&self) -> u32 {
        self.end - self.start
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
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
    /// Slot liveness.
    pub alive: Vec<bool>,

    /// Flat backing store for all routes; [`RouteHandle`] spans index into it.
    pub route_lanes: Vec<u32>,

    /// Free-list of despawned slots available for reuse (LIFO).
    free: Vec<VehId>,
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
            alive: Vec::with_capacity(cap),
            route_lanes: Vec::with_capacity(cap * 4),
            free: Vec::new(),
        }
    }

    /// Number of slots ever allocated (alive or free), i.e. the SoA length.
    #[inline]
    pub fn slots(&self) -> usize {
        self.alive.len()
    }

    /// Count of currently-alive vehicles.
    pub fn alive_count(&self) -> usize {
        self.alive.iter().filter(|a| **a).count()
    }

    /// Allocate a vehicle slot, reusing a free one if available. Returns the
    /// slot id. Internal: [`crate::Core::spawn`] wraps this with route setup.
    pub(crate) fn alloc(
        &mut self,
        lane: u32,
        s: f32,
        v: f32,
        len_m: f32,
        route: RouteHandle,
    ) -> VehId {
        if let Some(id) = self.free.pop() {
            let i = id as usize;
            self.lane[i] = lane;
            self.s[i] = s;
            self.v[i] = v;
            self.len_m[i] = len_m;
            self.route[i] = route;
            self.alive[i] = true;
            id
        } else {
            let id = self.alive.len() as VehId;
            self.lane.push(lane);
            self.s.push(s);
            self.v.push(v);
            self.len_m.push(len_m);
            self.route.push(route);
            self.alive.push(true);
            id
        }
    }

    /// Mark a slot free for reuse. The route span in `route_lanes` is left in
    /// place (not reclaimed) — cheap and keeps handles stable; acceptable
    /// because routes are bounded by `cap`.
    pub(crate) fn free(&mut self, id: VehId) {
        let i = id as usize;
        if self.alive[i] {
            self.alive[i] = false;
            self.free.push(id);
        }
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
