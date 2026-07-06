//! World-clock census trip spawner (Plan 2, spec §5; re-clocked in Task 9).
//!
//! Replaces the v1 synthetic attractor spawner: trips are no longer sampled
//! from a two-peak curve over building clusters — they come from the offline
//! `demand-gen` census bake ([`TripSchedule`], `trips.bin`).
//!
//! # Clock binding (Task 9)
//!
//! The **second-of-day** driving trip release comes from the world clock
//! ([`WorldClock::s_of_world_day`], anchored at the boot wall time and
//! running 6× real time) — census demand lives on the same 4 h world day as
//! the citizens, so both traffic sources breathe together. The **day kind**
//! (workday/weekend block) stays on the real Europe/Zurich calendar via
//! [`WallClock::day_kind`] (spec: date/season stay real) — a *world*
//! midnight wrap therefore does NOT flip the block; only the real calendar
//! does.
//!
//! # Release windows
//!
//! Per tick the release window is `[last_world_s, s_of_world_day(now))` in
//! world seconds-of-day (the spawner tracks the last released second, so no
//! second is skipped or double-released). A window that wraps world midnight
//! is split into the old day's tail `[s, 86400)` and the new day's head
//! `[0, e)`; both halves use the real-calendar [`DayKind`] of the current
//! tick.
//!
//! # Thinning (`demand_scale`)
//!
//! Every trip gets one deterministic draw
//! `u01(seed ^ 0x5EED_DE44, day_kind as u64, trip.index as u64)` and spawns
//! iff the draw is `< demand_scale`. The draw is a pure function of the
//! trip's identity — `(day block, index)`, since record indices restart per
//! block — and is **tick-independent**, so the same subset of the census
//! spawns every day at any scale, and the warm start reuses the identical
//! draw (no double randomness). The day-kind discriminant rides in `u01`'s
//! `tick` parameter so a weekday and a weekend trip with equal indices get
//! independent draws.
//!
//! # Spawn kinematics
//!
//! A trip whose origin lane leaves a gateway node (Gemeinde-boundary stub,
//! [`TrafficNet::gateway_lanes_out`]) represents traffic *entering* the
//! modelled area at speed: it spawns at the lane entry rolling at
//! `0.8 × edge speed`, **capped** so the entry speed is dissipable within
//! the measured gap to the nearest downstream vehicle along the route
//! (`v ≤ √(2·b·gap)`, comfortable-braking kinematics) — a car merging into
//! a queue must not enter faster than it can stop. Internal origins spawn
//! standing (`v = 0`), as v1 did.
//!
//! The entry point is `s = ENTRY_S` (one car length into the lane) so the
//! spawned body lies fully inside the lane — a spawn at exactly `s = 0`
//! would hang its rear back through the junction, where an upstream vehicle
//! mid-crossing can already be (observed as a real collision on the
//! Gemeinde net). A spawn is dropped — counted in
//! [`SpawnCounters::blocked_entry`] — when any of these hold:
//!
//!  * the start lane is a micro connector shorter than
//!    [`MIN_SPAWN_LANE_M`];
//!  * the start lane holds a vehicle within [`SPAWN_CLEARANCE_M`] of the
//!    entry point;
//!  * a vehicle on a feeder lane (any turn into the start lane) is within
//!    its own braking-plus-headway distance of the junction, i.e. it may
//!    cross in before the fresh spawn could get moving.
//!
//! # Warm start
//!
//! Booting mid-day must not start with an empty world: at construction the
//! trips with `departure_s ∈ [boot_s − 900, boot_s)` that pass the *same*
//! thinning draw are queued and released uniformly over the first 600 ticks
//! (deterministic slot `trip.index % 600`). If the boot is less than 15 min
//! after midnight the lookback clamps at 00:00 rather than reaching into the
//! previous day's block (documented trade-off: a boot in that sliver warm
//! starts with slightly fewer trips).
//!
//! # Safety valve
//!
//! While `core.fleet.alive_count() >= MAX_CONCURRENT` every release is
//! dropped (not backlogged) and counted; one rate-limited log line fires per
//! closed window that saw suppression — same valve semantics as v1.

use crate::Router;
use crate::clock::WallClock;
use crate::demand::{DayKind, Trip, TripSchedule};
use traffic_core::{Core, u01};
use traffic_net::TrafficNet;
use world_core::WorldClock;

/// Hard cap on the concurrent fleet; spawns are suppressed at or above it.
/// Also the natural pre-size for the kernel's slot capacity. Raised from
/// v1's 1500 to the spec §7 target (30 k) in Task 8: the measured morning
/// peak at `demand_scale = 1.0` is ~1.5–2 k alive with mean tick well under
/// the 50 ms budget, so the valve is a genuine safety valve again instead of
/// the binding constraint it was during Tasks 6–7.
pub const MAX_CONCURRENT: usize = 30_000;

/// Salt XOR-folded into the seed for the thinning draw so it can never alias
/// the kernel's per-vehicle noise stream or the shell's re-route stream.
const THINNING_SALT: u64 = 0x5EED_DE44;

/// Warm-start lookback: trips departed within this many seconds before boot
/// are considered "currently en route" and back-filled.
const WARM_LOOKBACK_S: u32 = 900;

/// Warm-start release horizon: queued trips are spread uniformly over the
/// first this-many ticks (60 s at 10 Hz) via `trip.index % 600`.
const WARM_RELEASE_TICKS: u64 = 600;

/// Minimum clear bumper distance (m) required around the entry point on the
/// start lane, so the first tick sees a sane leader gap (kept from v1).
const SPAWN_CLEARANCE_M: f32 = 12.0;

/// Arc position of the entry point: one kernel vehicle length (4.5 m) plus
/// margin into the lane, so the spawned body never pokes back through the
/// junction behind it.
const ENTRY_S: f32 = 5.0;

/// Minimum start-lane length (m) for a spawn: the body must fit fully
/// inside the lane at [`ENTRY_S`] with front margin. Trips whose route head
/// is a shorter micro connector lane are dropped ([`SpawnCounters::
/// blocked_entry`]) — a 4.5 m body on a ~2 m lane permanently straddles the
/// junction and collides with crossing traffic (observed on the real net).
const MIN_SPAWN_LANE_M: f32 = 2.0 * ENTRY_S;

/// Upstream guard time (s): a feeder-lane vehicle closer to the junction
/// than `headway·v + v²/(2b)` may cross in before a fresh standing spawn
/// can move — the spawn is blocked. Matches the kernel's IDM `T` (1.5 s).
const UPSTREAM_HEADWAY_S: f32 = 1.5;

/// Gateway origins enter rolling at this fraction of their edge's speed.
const GATEWAY_ENTRY_SPEED_FRACTION: f32 = 0.8;

/// How far down the route the entry-speed cap scans for a leader (m).
const ENTRY_LOOKAHEAD_M: f32 = 150.0;

/// Braking rate assumed by the entry-speed cap (m/s²). Deliberately below
/// the kernel's comfortable deceleration `b = 2.0` (Treiber et al. 2000) so
/// a rolling gateway entry always has braking headroom.
const ENTRY_BRAKE: f32 = 1.5;

/// Seconds per day.
const DAY_S: u32 = 86_400;

/// Runtime spawner tuning.
#[derive(Debug, Clone, Copy)]
pub struct SpawnerCfg {
    /// Demand thinning factor ∈ (0, 1]: the fraction of census trips that
    /// actually spawn. 1.0 until Task 8 measures the tick budget.
    pub demand_scale: f32,
}

impl Default for SpawnerCfg {
    fn default() -> Self {
        SpawnerCfg { demand_scale: 1.0 }
    }
}

/// One successful spawn, reported to the caller for destination tracking
/// (re-routing) and test introspection.
#[derive(Debug, Clone, Copy)]
pub struct SpawnRecord {
    /// Kernel slot id of the spawned vehicle.
    pub veh: u32,
    /// Destination edge (the trip's `dest_lane`'s edge).
    pub dest_edge: u32,
    /// The trip's block-local record index (deterministic identity).
    pub trip_index: u32,
}

/// Monotonic outcome counters over the spawner's lifetime.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpawnCounters {
    /// Vehicles actually placed into the kernel.
    pub spawned: u64,
    /// Trips dropped because the router found no origin→destination path.
    pub skipped_no_route: u64,
    /// Trips dropped by the [`MAX_CONCURRENT`] valve (incl. kernel-cap
    /// refusals, which only occur at the same boundary).
    pub suppressed: u64,
    /// Trips dropped because the entry point was physically occupied.
    pub blocked_entry: u64,
}

/// The census trip spawner. Owns the loaded [`TripSchedule`] and the
/// boot-anchored [`WallClock`] (real-calendar `day_kind` only — release
/// seconds come from the [`WorldClock`] passed into [`TripSpawner::step`]);
/// driven once per tick by the shell's `spawn_trips` system.
pub struct TripSpawner {
    schedule: TripSchedule,
    clock: WallClock,
    cfg: SpawnerCfg,
    seed: u64,
    /// The world second-of-day up to which trips have been released
    /// (exclusive). Initialized at the boot world second.
    last_world_s: u32,
    /// Warm-start queue: `(release_tick, trip)`, sorted by release tick then
    /// trip index; `warm_next` is the cursor of the next unreleased entry.
    warm: Vec<(u64, Trip)>,
    warm_next: usize,
    counters: SpawnCounters,
    /// Suppressions accumulated since the last per-window log flush.
    window_suppressed: u64,
    /// Reusable buffer of thinned trips for the current window.
    pending: Vec<Trip>,
}

impl TripSpawner {
    /// Build the spawner and queue the warm start (see module docs). `seed`
    /// is the sim seed; the thinning stream is salted so it is disjoint from
    /// every other `u01` consumer. `world` is the boot-time [`WorldClock`]
    /// the release seconds are sourced from (warm-start lookback is 900
    /// *world* seconds before the boot world second).
    pub fn new(
        schedule: TripSchedule,
        clock: WallClock,
        cfg: SpawnerCfg,
        seed: u64,
        world: &WorldClock,
    ) -> Self {
        let boot_s = world.s_of_world_day();
        let boot_day = clock.day_kind(0);
        let lo = boot_s.saturating_sub(WARM_LOOKBACK_S);
        let mut warm: Vec<(u64, Trip)> = schedule
            .trips_in(boot_day, lo..boot_s)
            .iter()
            .filter(|t| thinning_passes(seed, cfg.demand_scale, boot_day, t.index))
            .map(|t| (u64::from(t.index) % WARM_RELEASE_TICKS, *t))
            .collect();
        warm.sort_by_key(|&(slot, t)| (slot, t.index));

        TripSpawner {
            schedule,
            clock,
            cfg,
            seed,
            last_world_s: boot_s,
            warm,
            warm_next: 0,
            counters: SpawnCounters::default(),
            window_suppressed: 0,
            pending: Vec::new(),
        }
    }

    /// Lifetime outcome counters.
    pub fn counters(&self) -> SpawnCounters {
        self.counters
    }

    /// Number of trips queued for warm-start release at construction.
    pub fn warm_queue_len(&self) -> usize {
        self.warm.len()
    }

    /// The boot-anchored wall clock (read-only; for boot logging).
    pub fn clock(&self) -> &WallClock {
        &self.clock
    }

    /// Advance one tick: release the warm-start slot for `t` (first 600
    /// ticks only) plus every scheduled trip in the world-second window
    /// `[last_world_s, world.s_of_world_day())`, thinned by `demand_scale`.
    /// Successful spawns are appended to `spawned`; returns the count placed
    /// this tick.
    pub fn step(
        &mut self,
        core: &mut Core,
        net: &TrafficNet,
        router: &Router,
        t: u64,
        world: &WorldClock,
        spawned: &mut Vec<SpawnRecord>,
    ) -> usize {
        let mut n = 0;

        // Warm start: release queued trips whose slot has come up. Already
        // thinned at construction — no second draw.
        if t < WARM_RELEASE_TICKS {
            while self.warm_next < self.warm.len() && self.warm[self.warm_next].0 <= t {
                let trip = self.warm[self.warm_next].1;
                self.warm_next += 1;
                if self.spawn_trip(core, net, router, trip, spawned) {
                    n += 1;
                }
            }
        }

        // Live window: [last released world second, current world second),
        // split at a world-midnight wrap. Both halves use the REAL-calendar
        // day kind of this tick (see module docs: only the real calendar
        // flips the demand block, never a world wrap).
        let start = self.last_world_s;
        let end = world.s_of_world_day();
        if start != end {
            let day = self.clock.day_kind(t);
            if end > start {
                n += self.release_window(core, net, router, day, start..end, spawned);
            } else {
                n += self.release_window(core, net, router, day, start..DAY_S, spawned);
                n += self.release_window(core, net, router, day, 0..end, spawned);
            }
            self.last_world_s = end;
            // The window closed with this tick: flush the suppression log
            // (at most one line per window — the v1 valve-log semantics).
            if self.window_suppressed > 0 {
                tracing::warn!(
                    suppressed = self.window_suppressed,
                    alive = core.fleet.alive_count(),
                    max_concurrent = MAX_CONCURRENT,
                    "spawn valve engaged: trips dropped this window"
                );
                self.window_suppressed = 0;
            }
        }

        n
    }

    /// Release every trip of `day` departing within `window`, thinned by the
    /// per-trip draw. Returns the number actually spawned.
    fn release_window(
        &mut self,
        core: &mut Core,
        net: &TrafficNet,
        router: &Router,
        day: DayKind,
        window: core::ops::Range<u32>,
        spawned: &mut Vec<SpawnRecord>,
    ) -> usize {
        // Copy the thinned window into a reusable buffer so the schedule
        // borrow ends before the &mut self spawn calls below.
        let mut pending = std::mem::take(&mut self.pending);
        pending.clear();
        pending.extend(
            self.schedule
                .trips_in(day, window)
                .iter()
                .filter(|t| thinning_passes(self.seed, self.cfg.demand_scale, day, t.index))
                .copied(),
        );

        let mut n = 0;
        for &trip in &pending {
            if self.spawn_trip(core, net, router, trip, spawned) {
                n += 1;
            }
        }
        self.pending = pending;
        n
    }

    /// Route and place one trip. Returns `true` iff a vehicle was spawned;
    /// every failure path increments exactly one counter.
    fn spawn_trip(
        &mut self,
        core: &mut Core,
        net: &TrafficNet,
        router: &Router,
        trip: Trip,
        spawned: &mut Vec<SpawnRecord>,
    ) -> bool {
        if core.fleet.alive_count() >= MAX_CONCURRENT {
            self.counters.suppressed += 1;
            self.window_suppressed += 1;
            return false;
        }

        let origin_edge = lane_edge(net, trip.origin_lane);
        let dest_edge = lane_edge(net, trip.dest_lane);
        let Some(route) = router.route(net, origin_edge, dest_edge) else {
            self.counters.skipped_no_route += 1;
            // Rate-limited: log only at power-of-two counts (1, 2, 4, 8, …).
            if self.counters.skipped_no_route.is_power_of_two() {
                tracing::warn!(
                    origin_edge,
                    dest_edge,
                    total = self.counters.skipped_no_route,
                    "trip skipped: no route (rate-limited log)"
                );
            }
            return false;
        };

        // The router picks the concrete lane on the origin edge (lowest lane
        // with a turn towards the next hop) — spawn on that lane at s = 0.
        let start_lane = route[0];
        let s0 = ENTRY_S;
        if net.lanes[start_lane as usize].length_m < MIN_SPAWN_LANE_M
            || !start_lane_clear(core, start_lane, s0)
            || !upstream_clear(core, net, start_lane)
        {
            self.counters.blocked_entry += 1;
            return false;
        }

        // Gateway origins enter the plate rolling (capped by the braking
        // kinematics against the nearest downstream vehicle); internal
        // origins start standing. Computed BEFORE the spawn so the scan
        // never sees the entering vehicle itself.
        let v0 = if net.gateway_lanes_out().binary_search(&start_lane).is_ok() {
            let target = GATEWAY_ENTRY_SPEED_FRACTION * net.edges[origin_edge as usize].speed_ms;
            entry_speed_cap(core, net, &route, s0, target)
        } else {
            0.0
        };

        let Some(veh) = core.spawn(start_lane, s0, trip.vehicle_class, &route) else {
            // Kernel slot-cap refusal: same valve, same counter.
            self.counters.suppressed += 1;
            self.window_suppressed += 1;
            return false;
        };
        core.fleet.v[veh as usize] = v0;

        self.counters.spawned += 1;
        spawned.push(SpawnRecord {
            veh,
            dest_edge,
            trip_index: trip.index,
        });
        true
    }
}

/// The deterministic thinning gate: pure in `(seed, day block, trip index)`,
/// independent of the tick the trip is released on (see module docs).
fn thinning_passes(seed: u64, demand_scale: f32, day: DayKind, index: u32) -> bool {
    u01(seed ^ THINNING_SALT, day as u64, u64::from(index)) < demand_scale
}

/// The edge a lane belongs to. Lane ids are array indices on every validated
/// bake (the baker emits dense id == index arrays); the debug assert guards
/// the convention in tests.
fn lane_edge(net: &TrafficNet, lane: u32) -> u32 {
    let l = &net.lanes[lane as usize];
    debug_assert_eq!(l.id, lane, "lane ids must be dense array indices");
    l.edge
}

/// The largest safe entry speed at `s0` on the head of `route`, at most
/// `target`: `v ≤ √(2·b·gap)` against the nearest vehicle rear within
/// [`ENTRY_LOOKAHEAD_M`] along the route (minus [`SPAWN_CLEARANCE_M`] of
/// margin). An empty road ahead returns `target` unchanged.
fn entry_speed_cap(core: &Core, net: &TrafficNet, route: &[u32], s0: f32, target: f32) -> f32 {
    let mut offset = -s0; // entry point → start of the current route lane
    let mut min_gap = f32::INFINITY;
    for &lane in route {
        if offset > ENTRY_LOOKAHEAD_M {
            break;
        }
        for &veh in core.index.on_lane(lane) {
            let j = veh as usize;
            let rear = offset + core.fleet.s[j] - core.fleet.len_m[j];
            if rear >= 0.0 {
                min_gap = min_gap.min(rear);
            }
        }
        offset += net.lanes[lane as usize].length_m;
    }
    if min_gap == f32::INFINITY {
        return target;
    }
    let usable = (min_gap - SPAWN_CLEARANCE_M).max(0.0);
    target.min((2.0 * ENTRY_BRAKE * usable).sqrt())
}

/// Whether no vehicle on a feeder lane (any turn into `start_lane`) is close
/// enough to the junction to cross in on top of a fresh spawn: blocked when
/// `dist_to_lane_end < SPAWN_CLEARANCE_M + headway·v + v²/(2b)`. Gateway
/// origin lanes have no feeder turns, so this never blocks a gateway entry.
fn upstream_clear(core: &Core, net: &TrafficNet, start_lane: u32) -> bool {
    for turn in &net.turns {
        if turn.to_lane != start_lane {
            continue;
        }
        let feeder = turn.from_lane;
        let feeder_len = net.lanes[feeder as usize].length_m;
        for &veh in core.index.on_lane(feeder) {
            let j = veh as usize;
            let v = core.fleet.v[j];
            let reach = SPAWN_CLEARANCE_M + UPSTREAM_HEADWAY_S * v + v * v / (2.0 * ENTRY_BRAKE);
            if feeder_len - core.fleet.s[j] < reach {
                return false;
            }
        }
    }
    true
}

/// Whether `start_lane` has no existing vehicle within [`SPAWN_CLEARANCE_M`]
/// of arc position `s0`. Scans the lane's live occupancy (kept from v1).
fn start_lane_clear(core: &Core, start_lane: u32, s0: f32) -> bool {
    let fleet = &core.fleet;
    for &veh in core.index.on_lane(start_lane) {
        let j = veh as usize;
        if (fleet.s[j] - s0).abs() < SPAWN_CLEARANCE_M {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveTime, TimeZone, Utc};
    use demand_gen::output::{self, TripRecord};
    use std::io::Write as _;
    use std::path::PathBuf;

    /// The diamond fixture with gateway endpoints: node 0 (lane 0 out) and
    /// node 5 (lane 5 in) are `kind: "gateway"`.
    fn fixture_json() -> String {
        let p = format!(
            "{}/tests/fixtures/diamond-gateway.json",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p}: {e}"))
    }

    fn fixture_net(json: &str) -> TrafficNet {
        traffic_net::load(json).expect("diamond-gateway fixture must validate")
    }

    fn rec(dep: u32, o: u32, d: u32, seg: u8) -> TripRecord {
        TripRecord {
            departure_s: dep,
            origin_lane: o,
            dest_lane: d,
            segment: seg,
            vehicle_class: 0,
        }
    }

    /// Write a trips.bin for `net_json` to a unique temp file and load it.
    fn make_schedule(
        name: &str,
        net_json: &str,
        weekday: &[TripRecord],
        weekend: &[TripRecord],
    ) -> TripSchedule {
        let net_hash = *blake3::hash(net_json.as_bytes()).as_bytes();
        let mut bytes = Vec::new();
        output::write_trips(&mut bytes, &net_hash, weekday, weekend).unwrap();
        let path: PathBuf = std::env::temp_dir().join(format!(
            "winterthur-traffic-spawner-test-{}-{name}.bin",
            std::process::id()
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&bytes).unwrap();
        TripSchedule::load(&path, net_json.as_bytes()).unwrap()
    }

    /// A clock anchored to a fixed WORKDAY (Friday 2026-07-03) at `hh:mm`.
    fn clock_at(hh: u32, mm: u32) -> WallClock {
        let now = Utc.with_ymd_and_hms(2026, 7, 3, 12, 0, 0).unwrap();
        WallClock::new(now, Some(NaiveTime::from_hms_opt(hh, mm, 0).unwrap()))
    }

    /// The boot [`WorldClock`] anchored at the wall clock's boot second (the
    /// shell's `world_clock_anchored`): world time = wall time-of-day at
    /// boot, then 6× real time.
    fn world_clock_for(clock: &WallClock) -> WorldClock {
        WorldClock {
            world_tick: u64::from(clock.s_of_day(0)) * 10 / 6,
        }
    }

    /// Drive spawner + kernel for `ticks` in the shell's order (advance the
    /// world clock, spawn, then tick). Returns `(core, spawner, spawned trip
    /// indices in order)`.
    fn run(
        json: &str,
        weekday: &[TripRecord],
        clock: WallClock,
        cfg: SpawnerCfg,
        seed: u64,
        ticks: u64,
        name: &str,
    ) -> (Core, TripSpawner, Vec<u32>) {
        let net = fixture_net(json);
        let router = Router::new(&net);
        let mut core = Core::new(&net, MAX_CONCURRENT + 64, seed);
        let schedule = make_schedule(name, json, weekday, &[]);
        let mut wc = world_clock_for(&clock);
        let mut spawner = TripSpawner::new(schedule, clock, cfg, seed, &wc);
        let mut indices = Vec::new();
        let mut recs = Vec::new();
        for t in 0..ticks {
            wc.advance();
            recs.clear();
            spawner.step(&mut core, &net, &router, t, &wc, &mut recs);
            indices.extend(recs.iter().map(|r| r.trip_index));
            core.tick(t);
        }
        (core, spawner, indices)
    }

    /// Morning-peak trips file: 30 trips every 10 s in [27000, 27300) and two
    /// stragglers at 03:00 — the ratio fixture for the peak test. (10 s world
    /// spacing ≈ 1.7 real seconds at the 6× world clock, comfortably past the
    /// spawn-clearance window on the single origin lane.)
    fn peak_trips() -> Vec<TripRecord> {
        let mut trips: Vec<TripRecord> = vec![
            rec(10_800, 0, 5, output::SEGMENT_INBOUND),
            rec(10_830, 0, 5, output::SEGMENT_INBOUND),
        ];
        for k in 0..30 {
            trips.push(rec(27_000 + 10 * k, 0, 5, output::SEGMENT_INBOUND));
        }
        trips
    }

    #[test]
    fn morning_peak_spawns_far_more_than_night() {
        let json = fixture_json();
        let trips = peak_trips();
        let cfg = SpawnerCfg::default();

        let (_, sp_peak, _) = run(&json, &trips, clock_at(7, 30), cfg, 7, 600, "peak-0730");
        let (_, sp_night, _) = run(&json, &trips, clock_at(3, 0), cfg, 7, 600, "peak-0300");

        let peak = sp_peak.counters().spawned;
        let night = sp_night.counters().spawned;
        assert!(night >= 1, "03:00 must still spawn its scheduled trips");
        assert!(
            peak > 5 * night,
            "07:30 must out-spawn 03:00 by > 5x over 600 ticks: peak={peak} night={night}"
        );
    }

    #[test]
    fn demand_scale_half_spawns_same_subset_across_runs() {
        let json = fixture_json();
        let trips = peak_trips();
        let cfg = SpawnerCfg { demand_scale: 0.5 };

        let (core_a, sp_a, idx_a) = run(&json, &trips, clock_at(7, 30), cfg, 42, 700, "half-a");
        let (core_b, sp_b, idx_b) = run(&json, &trips, clock_at(7, 30), cfg, 42, 700, "half-b");

        assert_eq!(idx_a, idx_b, "the thinned subset must be run-invariant");
        assert_eq!(sp_a.counters(), sp_b.counters());
        assert_eq!(core_a.state_hash(), core_b.state_hash());
        // Thinning actually thinned: strictly between none and all 30 peak
        // trips (2 + 30 total in the file; the 03:00 ones never release).
        let n = sp_a.counters().spawned;
        assert!(
            (1..30).contains(&n),
            "demand_scale=0.5 should spawn a strict subset, got {n}"
        );
    }

    #[test]
    fn warm_start_populates_world_after_midday_boot() {
        let json = fixture_json();
        // 40 trips departed in the 15 min before a 12:00 boot.
        let trips: Vec<TripRecord> = (0..40)
            .map(|k| rec(42_300 + 20 * k, 0, 5, output::SEGMENT_INBOUND))
            .collect();

        let (core, sp, _) = run(
            &json,
            &trips,
            clock_at(12, 0),
            SpawnerCfg::default(),
            9,
            300,
            "warm",
        );
        assert!(
            sp.warm_queue_len() > 0,
            "boot at 12:00 must queue warm-start trips from [11:45, 12:00)"
        );
        assert!(
            core.fleet.alive_count() > 0,
            "warm start must populate the world by tick 300"
        );
        assert!(sp.counters().spawned > 0);
    }

    #[test]
    fn deterministic_same_anchor_same_hash_different_anchor_diverges() {
        let json = fixture_json();
        let trips = peak_trips();
        let cfg = SpawnerCfg::default();

        let (core_a, _, idx_a) = run(&json, &trips, clock_at(7, 30), cfg, 5, 650, "det-a");
        let (core_b, _, idx_b) = run(&json, &trips, clock_at(7, 30), cfg, 5, 650, "det-b");
        assert_eq!(idx_a, idx_b);
        assert_eq!(
            core_a.state_hash(),
            core_b.state_hash(),
            "same (seed, trips, boot anchor) must reproduce the state"
        );

        let (core_c, _, idx_c) = run(&json, &trips, clock_at(3, 0), cfg, 5, 650, "det-c");
        assert_ne!(
            idx_a, idx_c,
            "a different boot anchor must release a different spawn pattern"
        );
        assert_ne!(core_a.state_hash(), core_c.state_hash());
    }

    /// Spec §5 gateway sinks: a route ending on a gateway in-lane despawns
    /// via the kernel's normal end-of-route path — no traffic-core change.
    #[test]
    fn gateway_arrival_despawns_via_end_of_route() {
        let json = fixture_json();
        let net = fixture_net(&json);
        // Sanity: the fixture really has gateway stubs on both ends.
        assert_eq!(net.gateways(), &[0, 5]);
        assert_eq!(net.gateway_lanes_out(), &[0]);
        assert_eq!(net.gateway_lanes_in(), &[5]);

        // One trip, departing one second after a 07:30 boot, gateway→gateway.
        let trips = vec![rec(27_001, 0, 5, output::SEGMENT_THROUGH)];
        let (core, sp, _) = run(
            &json,
            &trips,
            clock_at(7, 30),
            SpawnerCfg::default(),
            1,
            3_000,
            "gateway-sink",
        );
        assert_eq!(sp.counters().spawned, 1, "the trip must have spawned");
        assert_eq!(
            core.fleet.alive_count(),
            0,
            "the vehicle must despawn on arrival at the gateway in-lane"
        );
    }

    /// World-midnight wrap: the release window splits into the old world
    /// day's tail and the new world day's head — but the demand block stays
    /// on the REAL calendar (Task 9: a world wrap mid-real-Friday keeps
    /// workday demand; only real midnight flips the block).
    #[test]
    fn world_midnight_wrap_splits_window_but_keeps_real_day_block() {
        let json = fixture_json();
        let net = fixture_net(&json);
        let router = Router::new(&net);
        let seed = 3u64;
        let mut core = Core::new(&net, MAX_CONCURRENT + 64, seed);

        // Friday 23:59:30 wall boot ⇒ world clock anchored at 86 370 s; the
        // WORLD midnight wraps 30 world seconds (= 50 ticks) later, while the
        // real calendar is still Friday for another 30 real seconds.
        let now = Utc.with_ymd_and_hms(2026, 7, 3, 12, 0, 0).unwrap();
        let clock = WallClock::new(now, Some(NaiveTime::from_hms_opt(23, 59, 30).unwrap()));
        // Two weekday-block trips straddling the world wrap — both must
        // spawn; a weekend-block trip after the wrap must NOT (real calendar
        // still Friday).
        let weekday = vec![
            rec(86_395, 0, 5, output::SEGMENT_INBOUND),
            rec(2, 3, 5, output::SEGMENT_INTERNAL),
        ];
        let weekend = vec![rec(2, 0, 5, output::SEGMENT_INBOUND)];
        let schedule = make_schedule("midnight", &json, &weekday, &weekend);
        let mut wc = world_clock_for(&clock);
        let mut spawner = TripSpawner::new(schedule, clock, SpawnerCfg::default(), seed, &wc);

        let mut recs = Vec::new();
        for t in 0..600 {
            wc.advance();
            spawner.step(&mut core, &net, &router, t, &wc, &mut recs);
            core.tick(t);
        }
        let indices: Vec<u32> = recs.iter().map(|r| r.trip_index).collect();
        assert_eq!(
            recs.len(),
            2,
            "both weekday trips (tail + post-wrap head) must spawn, got {indices:?}"
        );
        assert_eq!(spawner.counters().spawned, 2);
    }

    /// Gateway entries roll in at 0.8 × edge speed on a free road, but the
    /// entry speed is capped by braking kinematics when a vehicle already
    /// occupies the road ahead — never faster than the gap can dissipate.
    #[test]
    fn gateway_entry_speed_is_capped_by_downstream_gap() {
        let json = fixture_json();
        let net = fixture_net(&json);
        let router = Router::new(&net);
        let seed = 11u64;

        // One gateway-origin trip departing one second after a 07:30 boot.
        let trips = vec![rec(27_001, 0, 5, output::SEGMENT_INBOUND)];
        let spawn_one = |core: &mut Core, name: &str| -> u32 {
            let schedule = make_schedule(name, &json, &trips, &[]);
            let clock = clock_at(7, 30);
            let mut wc = world_clock_for(&clock);
            let mut sp = TripSpawner::new(schedule, clock, SpawnerCfg::default(), seed, &wc);
            let mut recs = Vec::new();
            for t in 0..20 {
                wc.advance();
                // No core.tick: the pre-placed leader must stay put.
                sp.step(core, &net, &router, t, &wc, &mut recs);
            }
            assert_eq!(recs.len(), 1, "the single trip must spawn");
            recs[0].veh
        };

        // Free road: the full 0.8 × edge speed (edge 0: 10 m/s → 8 m/s).
        let mut free = Core::new(&net, 64, seed);
        let veh = spawn_one(&mut free, "entry-free");
        assert!(
            (free.fleet.v[veh as usize] - 8.0).abs() < 1e-5,
            "free-road gateway entry must roll at 0.8 x edge speed, got {}",
            free.fleet.v[veh as usize]
        );

        // A standing leader 30 m in: entry must slow to what the remaining
        // gap can absorb at ENTRY_BRAKE (strictly below the free-road 8).
        let mut jammed = Core::new(&net, 64, seed);
        jammed.spawn(0, 30.0, 0, &[0]).expect("leader spawn");
        let veh = spawn_one(&mut jammed, "entry-jammed");
        let v = jammed.fleet.v[veh as usize];
        assert!(
            v > 0.0 && v < 6.5,
            "jammed gateway entry must be gap-capped (expected ~6.4), got {v}"
        );
    }

    /// Thinning is a pure function of (seed, day block, index): weekday and
    /// weekend trips with equal indices draw independently.
    #[test]
    fn thinning_is_day_block_scoped_and_tick_independent() {
        let seed = 0xF00D;
        for index in 0..64u32 {
            let wd = thinning_passes(seed, 0.5, DayKind::Workday, index);
            // Re-evaluating must be stable (tick plays no part).
            assert_eq!(wd, thinning_passes(seed, 0.5, DayKind::Workday, index));
        }
        // Independence: over enough indices the two blocks must disagree at
        // least once (they'd be identical if day_kind were ignored).
        let disagree = (0..256u32).any(|i| {
            thinning_passes(seed, 0.5, DayKind::Workday, i)
                != thinning_passes(seed, 0.5, DayKind::Weekend, i)
        });
        assert!(disagree, "weekday/weekend draws must be independent");
    }
}
