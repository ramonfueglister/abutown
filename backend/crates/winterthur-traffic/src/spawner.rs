//! Trip spawner v1 (synthetic-structured; STATPOP OD is Plan 2).
//!
//! # Attractor set
//!
//! Origins/destinations are sampled from a weighted **attractor** set, where
//! each attractor is a network *edge* the trip enters/leaves the plate through:
//!
//!  * **Gateways** — plate-boundary edges: an edge is a gateway when either of
//!    its endpoint nodes has graph degree 1 (a genuine dead-end stub where
//!    traffic enters the modelled area) or lies within [`BORDER_M`] of the
//!    node-coordinate bounding box border. Gateways are weighted
//!    [`GATEWAY_WEIGHT`]× a cluster so through-traffic dominates, matching a
//!    real city plate where most trips cross the boundary.
//!  * **Clusters** — the [`N_CLUSTERS`] heaviest building clusters from
//!    `buildings.json`. Footprint centroids are averaged per spatial cell and
//!    the densest cells are kept; each cluster centre is snapped to the nearest
//!    lane, and that lane's edge becomes the attractor.
//!
//! # Trip rate
//!
//! A trip's spawn instants follow a two-peak daily demand curve (morning
//! 07–08 h and evening 17–18 h rush, each at 100 % of [`SpawnConfig::max_rate`],
//! `BASE_FRACTION` off-peak; piecewise linear between). Sim time-of-day is
//! `tick·dt` seconds after 06:00, wrapping every 24 h.
//!
//! ## Concurrency arithmetic (documenting the [`MAX_CONCURRENT`] target)
//!
//! By Little's law the steady-state concurrent fleet is `λ · E[trip time]`.
//! Mean trip length across the plate is ≈ 2 km at ≈ 11 m/s ⇒ `E[T] ≈ 180 s`.
//! With `max_rate = 18 veh/s`, unconstrained peak concurrency would be
//! `18 · 180 ≈ 3240` — so the hard [`MAX_CONCURRENT`] = 1500 cap binds at the
//! rush peaks (spawns are suppressed while `alive ≥ MAX_CONCURRENT`), holding
//! the peak fleet at ~1500. Off-peak (`0.2 · 18 = 3.6 veh/s`) gives
//! `3.6 · 180 ≈ 650` concurrent, comfortably under the cap.
//!
//! # Determinism
//!
//! All randomness is [`traffic_core::u01`]`(seed, tick, draw)` — a pure
//! function of a per-spawner draw counter, so a run is bit-reproducible for a
//! fixed seed regardless of wall-clock timing.

use crate::Router;
use traffic_core::{Core, u01};
use traffic_net::TrafficNet;

/// Timestep echoed from the kernel so spawn accumulation matches `Core::tick`.
const DT: f32 = traffic_core::DT;

/// Number of building clusters kept as attractors.
pub const N_CLUSTERS: usize = 30;

/// A gateway attractor's weight is this multiple of a cluster's.
pub const GATEWAY_WEIGHT: f32 = 2.0;

/// Off-peak demand as a fraction of `max_rate`.
pub const BASE_FRACTION: f32 = 0.2;

/// A node within this many metres of the bbox border marks its incident edges
/// as plate gateways. Sized to the real Winterthur plate: the modelled area is
/// ~1.6 km × 1.85 km with sparse peripheral nodes, so a 30 m band captures only
/// ~18 edges (too few distinct entrances → OD collisions + gateway-lane
/// gridlock). A 150 m band yields ~156 boundary edges, a realistic gateway ring.
pub const BORDER_M: f32 = 150.0;

/// Target peak concurrent fleet; spawns are suppressed at or above it. Also
/// the natural pre-size for the kernel's slot capacity.
pub const MAX_CONCURRENT: usize = 1500;

/// Spatial cell size (m) for clustering building centroids before ranking.
const CLUSTER_CELL_M: f32 = 120.0;

/// Tunable spawn parameters.
#[derive(Debug, Clone, Copy)]
pub struct SpawnConfig {
    /// Peak spawn rate (vehicles per second) at the rush-hour maxima.
    pub max_rate: f32,
}

impl Default for SpawnConfig {
    fn default() -> Self {
        // See the concurrency arithmetic in the module docs.
        SpawnConfig { max_rate: 30.0 }
    }
}

/// A weighted trip endpoint: an edge id and its sampling weight.
#[derive(Debug, Clone, Copy)]
struct Attractor {
    edge: u32,
    weight: f32,
}

/// Deterministic weighted trip generator over a fixed attractor set.
pub struct Spawner {
    cfg: SpawnConfig,
    /// Cumulative-weight table over attractors for O(log n) sampling.
    attractors: Vec<Attractor>,
    cumulative: Vec<f32>,
    total_weight: f32,
    /// Fractional spawn carry so a sub-1 per-tick rate still spawns on average.
    accumulator: f32,
    /// Monotonic per-spawner draw counter feeding `u01`'s `id` argument, so
    /// every random decision uses a distinct, replayable stream position.
    draw: u64,
    seed: u64,
}

impl Spawner {
    /// Build the attractor set from the net + building clusters and seed the
    /// deterministic draw stream.
    pub fn new(net: &TrafficNet, buildings_json: &str, cfg: SpawnConfig, seed: u64) -> Self {
        let mut attractors = gateway_attractors(net);
        attractors.extend(cluster_attractors(net, buildings_json));

        // Guard against a degenerate net with no attractors (shouldn't happen
        // on the real bake) by falling back to every edge at unit weight.
        if attractors.is_empty() {
            attractors = net
                .edges
                .iter()
                .map(|e| Attractor {
                    edge: e.id,
                    weight: 1.0,
                })
                .collect();
        }

        let mut cumulative = Vec::with_capacity(attractors.len());
        let mut running = 0.0f32;
        for a in &attractors {
            running += a.weight;
            cumulative.push(running);
        }
        let total_weight = running;

        Spawner {
            cfg,
            attractors,
            cumulative,
            total_weight,
            accumulator: 0.0,
            draw: 0,
            seed,
        }
    }

    /// Number of attractors (gateways + clusters).
    pub fn attractor_count(&self) -> usize {
        self.attractors.len()
    }

    /// The demand fraction ∈ `[BASE_FRACTION, 1.0]` at sim tick `t`, following
    /// the two-peak daily curve. Public so the shell/tests can introspect it.
    pub fn demand_fraction(&self, t: u64) -> f32 {
        demand_fraction_at(t)
    }

    /// Advance the spawner one tick: accumulate the fractional spawn budget for
    /// the current demand level and emit that many trips into `core` (subject
    /// to the [`MAX_CONCURRENT`] cap). Each vehicle actually placed is appended
    /// to `spawned` as `(veh_id, dest_edge)` so the caller can track trip
    /// destinations for re-routing. Returns the count spawned this tick.
    pub fn step(
        &mut self,
        core: &mut Core,
        net: &TrafficNet,
        router: &Router,
        t: u64,
        spawned: &mut Vec<(u32, u32)>,
    ) -> usize {
        let fraction = demand_fraction_at(t);
        self.accumulator += self.cfg.max_rate * fraction * DT;

        let mut n = 0;
        // Budget is measured in *successful* spawns. A draw can be lost (same
        // endpoint, disconnected OD pair, router lane-head mismatch), so we
        // retry lost draws within a bounded attempt budget per unit so demand
        // is realised rather than silently thinned by routing topology.
        const MAX_ATTEMPTS_PER_SPAWN: u32 = 8;
        while self.accumulator >= 1.0 {
            if core.fleet.alive_count() >= MAX_CONCURRENT {
                // Cap reached: drop the budget (do not backlog) so the fleet
                // holds near the target instead of surging once cars clear.
                self.accumulator = 0.0;
                break;
            }
            let mut placed = false;
            for _ in 0..MAX_ATTEMPTS_PER_SPAWN {
                if let Some(rec) = self.try_spawn_one(core, net, router, t) {
                    spawned.push(rec);
                    placed = true;
                    break;
                }
            }
            // Consume one unit of budget whether or not a spawn landed, so a
            // pathological all-lost tick can't spin forever.
            self.accumulator -= 1.0;
            if placed {
                n += 1;
            }
        }
        n
    }

    /// Sample an (origin, destination) attractor pair, route between them, and
    /// spawn a vehicle at the route head. Returns `(veh_id, dest_edge)` on
    /// success, or `None` when the draw is lost (same endpoint, disconnected
    /// pair, or the fleet is full) — fine for a synthetic generator.
    fn try_spawn_one(
        &mut self,
        core: &mut Core,
        net: &TrafficNet,
        router: &Router,
        t: u64,
    ) -> Option<(u32, u32)> {
        let from = self.sample_attractor(t);
        let to = self.sample_attractor(t);
        if from == to {
            return None;
        }
        let route = router.route(net, from, to)?;
        if route.len() < 2 {
            return None;
        }
        let start_lane = route[0];
        // Enter a short way onto the first lane so the very first tick has a
        // sane leader gap; clamp to a small positive offset within the lane.
        let s0 = (net.lane_len(start_lane) * 0.1).clamp(1.0, 5.0);
        // Refuse to spawn on top of an existing vehicle: if any car on the
        // start lane sits within a spawn clearance of `s0`, drop this draw
        // (a real vehicle can't materialise inside another). Without this two
        // trips sharing a gateway lane in one tick would overlap at identical
        // `s`, which the kernel reads as a collision.
        if !start_lane_clear(core, start_lane, s0) {
            return None;
        }
        let veh = core.spawn(start_lane, s0, &route)?;
        Some((veh, to))
    }

    /// Draw one attractor edge ∝ weight via the cumulative table.
    fn sample_attractor(&mut self, t: u64) -> u32 {
        let r = u01(self.seed, t, self.draw) * self.total_weight;
        self.draw = self.draw.wrapping_add(1);
        // First cumulative bucket strictly above `r`.
        let idx = self
            .cumulative
            .partition_point(|&c| c <= r)
            .min(self.attractors.len() - 1);
        self.attractors[idx].edge
    }
}

/// Minimum clear bumper distance (m) required around a spawn point on the
/// start lane. A few car lengths so the first tick sees a comfortable gap.
const SPAWN_CLEARANCE_M: f32 = 12.0;

/// Whether `start_lane` has no existing vehicle within [`SPAWN_CLEARANCE_M`] of
/// arc position `s0`. Scans the lane's live occupancy (small — a single lane).
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

/// The two-peak demand fraction at tick `t`: morning (07–08 h) and evening
/// (17–18 h) rush at 1.0, `BASE_FRACTION` elsewhere, piecewise-linear ramps
/// into each peak over the hour on either side. Sim time starts at 06:00.
fn demand_fraction_at(t: u64) -> f32 {
    const START_H: f32 = 6.0;
    let sim_seconds = t as f32 * DT;
    let hour = (START_H + sim_seconds / 3600.0).rem_euclid(24.0);

    // Triangular peaks: ramp up over the hour before the maximum, ramp down
    // over the hour after. Morning maximum at 07:30, evening at 17:30 keep the
    // 07–08 / 17–18 windows near full.
    let peak = |centre: f32| -> f32 {
        let d = (hour - centre).abs();
        if d >= 1.0 {
            0.0
        } else {
            1.0 - d // linear falloff to 0 at ±1 h
        }
    };
    let intensity = peak(7.5).max(peak(17.5));
    BASE_FRACTION + (1.0 - BASE_FRACTION) * intensity
}

/// Gateway attractors: edges incident to a degree-1 node or a node within
/// [`BORDER_M`] of the node-coordinate bounding box.
fn gateway_attractors(net: &TrafficNet) -> Vec<Attractor> {
    // Node degree over the (undirected) edge graph.
    let mut degree = vec![0u32; net.nodes.len()];
    // Nodes are id==index on the real bake, but don't assume it: build an
    // id->index map to stay robust.
    let idx_of: std::collections::HashMap<u32, usize> = net
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id, i))
        .collect();
    for e in &net.edges {
        if let Some(&i) = idx_of.get(&e.from) {
            degree[i] += 1;
        }
        if let Some(&i) = idx_of.get(&e.to) {
            degree[i] += 1;
        }
    }

    // Bounding box over node coords.
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for n in &net.nodes {
        min_x = min_x.min(n.x);
        max_x = max_x.max(n.x);
        min_z = min_z.min(n.z);
        max_z = max_z.max(n.z);
    }

    let is_border = |n: &traffic_net::Node| -> bool {
        (n.x - min_x).abs() <= BORDER_M
            || (max_x - n.x).abs() <= BORDER_M
            || (n.z - min_z).abs() <= BORDER_M
            || (max_z - n.z).abs() <= BORDER_M
    };

    let node_is_gateway = |id: u32| -> bool {
        match idx_of.get(&id) {
            Some(&i) => degree[i] <= 1 || is_border(&net.nodes[i]),
            None => false,
        }
    };

    net.edges
        .iter()
        .filter(|e| node_is_gateway(e.from) || node_is_gateway(e.to))
        .map(|e| Attractor {
            edge: e.id,
            weight: GATEWAY_WEIGHT,
        })
        .collect()
}

/// Cluster attractors: parse building footprint centroids, bucket them into
/// [`CLUSTER_CELL_M`] cells, keep the [`N_CLUSTERS`] densest cells, and snap
/// each cell centroid to the nearest lane's edge.
fn cluster_attractors(net: &TrafficNet, buildings_json: &str) -> Vec<Attractor> {
    let centroids = building_centroids(buildings_json);
    if centroids.is_empty() {
        return Vec::new();
    }

    // Bucket centroids into a coarse grid; track per-cell sum + count so we can
    // rank by density and recover each cell's mean position deterministically.
    use std::collections::BTreeMap;
    // Per-cell accumulator: (sum_x, sum_z, count).
    type CellAgg = (f32, f32, u32);
    let mut cells: BTreeMap<(i32, i32), CellAgg> = BTreeMap::new();
    for (x, z) in centroids {
        let key = (
            (x / CLUSTER_CELL_M).floor() as i32,
            (z / CLUSTER_CELL_M).floor() as i32,
        );
        let e = cells.entry(key).or_insert((0.0, 0.0, 0));
        e.0 += x;
        e.1 += z;
        e.2 += 1;
    }

    // Rank cells by count descending (tie-break by key via the stable sort over
    // the BTreeMap's ascending-key iteration) and keep the top N.
    let mut ranked: Vec<((i32, i32), CellAgg)> = cells.into_iter().collect();
    ranked.sort_by(|a, b| b.1.2.cmp(&a.1.2).then(a.0.cmp(&b.0)));
    ranked.truncate(N_CLUSTERS);

    ranked
        .into_iter()
        .filter_map(|(_key, (sx, sz, count))| {
            let cx = sx / count as f32;
            let cz = sz / count as f32;
            nearest_lane_edge(net, cx, cz).map(|edge| Attractor { edge, weight: 1.0 })
        })
        .collect()
}

/// Parse building footprint centroids from `buildings.json`. Each building has
/// a `footprint: [[x, z], ...]`; the centroid is the mean of its vertices.
fn building_centroids(buildings_json: &str) -> Vec<(f32, f32)> {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(buildings_json) else {
        return Vec::new();
    };
    let Some(buildings) = doc.get("buildings").and_then(|b| b.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(buildings.len());
    for b in buildings {
        let Some(fp) = b.get("footprint").and_then(|f| f.as_array()) else {
            continue;
        };
        let mut sx = 0.0f64;
        let mut sz = 0.0f64;
        let mut n = 0u32;
        for pt in fp {
            let Some(arr) = pt.as_array() else { continue };
            if arr.len() >= 2
                && let (Some(x), Some(z)) = (arr[0].as_f64(), arr[1].as_f64())
            {
                sx += x;
                sz += z;
                n += 1;
            }
        }
        if n > 0 {
            out.push(((sx / n as f64) as f32, (sz / n as f64) as f32));
        }
    }
    out
}

/// The edge id of the lane whose polyline passes closest to `(x, z)`. Scans
/// every lane's vertices — O(total polyline vertices), run once at startup.
fn nearest_lane_edge(net: &TrafficNet, x: f32, z: f32) -> Option<u32> {
    let mut best: Option<(f32, u32)> = None;
    for lane in &net.lanes {
        for p in &lane.pts {
            let dx = p[0] - x;
            let dz = p[1] - z;
            let d2 = dx * dx + dz * dz;
            match best {
                Some((bd, _)) if bd <= d2 => {}
                _ => best = Some((d2, lane.edge)),
            }
        }
    }
    best.map(|(_, edge)| edge)
}
