//! `demand-gen`: offline census-gravity demand bake for the Winterthur
//! traffic sim — STATPOP residents × OSM land-use destinations × BFS commuter
//! matrix × authored A1 through volumes → byte-stable `trips.bin`
//! (spec `docs/superpowers/specs/2026-07-05-winterthur-traffic-demand-plan2-design.md` §4).
//!
//! Everything is deterministic: fixed iteration orders, randomness only via
//! `traffic_core::u01(0xD3, index, salt)` with per-purpose salt streams, and
//! the writer sorts each day block by `(departure_s, origin_lane, dest_lane)`.

pub mod gateways;
pub mod gravity;
pub mod inputs;
pub mod output;
pub mod profiles;

use gateways::{GwInfo, Need};
use inputs::{Authored, wgs84_to_world};
use output::{
    SEGMENT_INBOUND, SEGMENT_INTERNAL, SEGMENT_OUTBOUND, SEGMENT_THROUGH, TripRecord, write_trips,
};
use profiles::Profile;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use traffic_core::u01;
use traffic_net::TrafficNet;

/// The single u01 seed for the whole bake (plan Task 4).
pub const SEED: u64 = 0xD3;
/// Zone grid pitch in meters.
pub const CELL_M: f64 = 200.0;
/// Winterthur's BFS commune number.
pub const WINTERTHUR_BFS: u32 = 230;
/// Furness convergence tolerance (0.1 %) and iteration cap.
pub const FURNESS_TOL: f64 = 1e-3;
pub const FURNESS_MAX_ITER: usize = 100;

// u01 salt streams (third argument), one per random purpose:
const SALT_INT_COUNT_WD: u64 = 0;
const SALT_INT_DEP_WD: u64 = 1;
const SALT_INT_COUNT_WE: u64 = 2;
const SALT_INT_DEP_WE: u64 = 3;
const SALT_IO_COUNT: u64 = 4;
const SALT_IN_DEST: u64 = 5;
const SALT_IN_DEP: u64 = 6;
const SALT_IN_RET_DEP: u64 = 7;
const SALT_OUT_ORIGIN: u64 = 8;
const SALT_OUT_DEP: u64 = 9;
const SALT_OUT_RET_DEP: u64 = 10;
const SALT_THR_COUNT_WD: u64 = 11;
const SALT_THR_DEP_WD: u64 = 12;
const SALT_THR_COUNT_WE: u64 = 13;
const SALT_THR_DEP_WE: u64 = 14;

/// Bake configuration (mirrors the CLI flags).
#[derive(Debug, Clone)]
pub struct BakeConfig {
    pub net_path: PathBuf,
    pub demand_dir: PathBuf,
    pub authored_path: PathBuf,
    pub landuse_path: PathBuf,
    pub out_path: PathBuf,
}

/// Summary statistics of a bake, for the report + sanity gates.
#[derive(Debug)]
pub struct BakeStats {
    pub weekday_total: usize,
    pub weekend_total: usize,
    /// Weekday trips per segment [internal, inbound, outbound, through].
    pub weekday_by_segment: [usize; 4],
    /// Weekday gateway volume (trips entering or leaving there), desc.
    pub gateway_volumes: Vec<(u32, u64)>,
    /// Communes in the matrix with no centroid (skipped, deterministic).
    pub skipped_communes: usize,
    pub furness_iters: usize,
    pub furness_max_rel_err: f64,
    pub net_hash_hex: String,
}

/// Run the full bake: load inputs, build zones, gravity-balance internal
/// demand, expand in/out + through, and write `trips.bin`.
pub fn bake(cfg: &BakeConfig) -> Result<BakeStats, String> {
    let net_bytes = fs::read(&cfg.net_path).map_err(|e| format!("{:?}: {e}", cfg.net_path))?;
    let net_hash: [u8; 32] = *blake3::hash(&net_bytes).as_bytes();
    let net_json = String::from_utf8(net_bytes).map_err(|e| format!("net not UTF-8: {e}"))?;
    let net = traffic_net::load(&net_json).map_err(|e| format!("net load: {e}"))?;

    let authored = inputs::load_authored(&cfg.authored_path).map_err(|e| e.to_string())?;
    let statpop =
        inputs::load_statpop(&cfg.demand_dir.join("statpop.csv")).map_err(|e| e.to_string())?;
    let pendler = inputs::load_pendler(&cfg.demand_dir.join("pendlermatrix.csv"))
        .map_err(|e| e.to_string())?;
    let communes =
        inputs::load_communes(&cfg.demand_dir.join("communes.csv")).map_err(|e| e.to_string())?;
    let worksites = inputs::load_landuse_worksites(&cfg.landuse_path).map_err(|e| e.to_string())?;

    let model = Model::build(&net, &statpop, &worksites)?;
    let mut weekday: Vec<TripRecord> = Vec::new();
    let mut weekend: Vec<TripRecord> = Vec::new();
    let mut gateway_volumes: HashMap<u32, u64> = HashMap::new();

    // --- internal trips (gravity), weekday + weekend blocks ---------------
    let furness = model.internal_furness(&authored);
    let mut wd_dep_idx: u64 = 0;
    let mut we_dep_idx: u64 = 0;
    for (k, &t_ij) in furness.t.iter().enumerate() {
        let (i, j) = (k / model.dests.len(), k % model.dests.len());
        let o_lane = model.origins[i].lane;
        let d_lane = model.dests[j].lane;
        let expected = t_ij * authored.trips_scale;
        let n_wd = stochastic_round(expected, u01(SEED, k as u64, SALT_INT_COUNT_WD));
        for _ in 0..n_wd {
            let u = u01(SEED, wd_dep_idx, SALT_INT_DEP_WD);
            wd_dep_idx += 1;
            weekday.push(rec(Profile::Workday, u, o_lane, d_lane, SEGMENT_INTERNAL));
        }
        let n_we = stochastic_round(expected, u01(SEED, k as u64, SALT_INT_COUNT_WE));
        for _ in 0..n_we {
            let u = u01(SEED, we_dep_idx, SALT_INT_DEP_WE);
            we_dep_idx += 1;
            weekend.push(rec(Profile::Weekend, u, o_lane, d_lane, SEGMENT_INTERNAL));
        }
    }
    let weekday_internal = weekday.len();

    // --- in/out commuters (weekday only; weekends carry no commute) -------
    let commune_pos: HashMap<u32, (f64, f64)> = communes
        .iter()
        .map(|c| (c.bfs, wgs84_to_world(c.lon, c.lat)))
        .collect();
    let mut skipped_communes = 0usize;
    let mut in_idx: u64 = 0; // inbound trip counter (dest pick + departures)
    let mut out_idx: u64 = 0;
    for (row, flow) in pendler.iter().enumerate() {
        let external = if flow.origin_bfs == WINTERTHUR_BFS && flow.dest_bfs == WINTERTHUR_BFS {
            continue; // internal commutes are covered by the gravity model
        } else if flow.dest_bfs == WINTERTHUR_BFS {
            flow.origin_bfs
        } else {
            flow.dest_bfs
        };
        let Some(&(cx, cz)) = commune_pos.get(&external) else {
            skipped_communes += 1;
            continue;
        };
        let bearing = gateways::bearing_of(cx, cz);
        let dist_km = (cx * cx + cz * cz).sqrt() / 1000.0;
        let spawn_gw = gateways::pick(&model.gateways, bearing, dist_km, Need::Spawn)
            .ok_or("no gateway with a spawn lane exists")?;
        let sink_gw = gateways::pick(&model.gateways, bearing, dist_km, Need::Sink)
            .ok_or("no gateway with a sink lane exists")?;
        let expected = flow.workers * authored.car_share * authored.trips_scale;
        let n = stochastic_round(expected, u01(SEED, row as u64, SALT_IO_COUNT));
        let inbound = flow.dest_bfs == WINTERTHUR_BFS;
        for _ in 0..n {
            if inbound {
                let cell_lane = model.pick_dest_lane(u01(SEED, in_idx, SALT_IN_DEST));
                let morning = rec(
                    Profile::Morning,
                    u01(SEED, in_idx, SALT_IN_DEP),
                    spawn_gw
                        .spawn_lane
                        .expect("pick(Spawn) returned spawn lane"),
                    cell_lane,
                    SEGMENT_INBOUND,
                );
                let evening = rec(
                    Profile::Evening,
                    u01(SEED, in_idx, SALT_IN_RET_DEP),
                    cell_lane,
                    sink_gw.sink_lane.expect("pick(Sink) returned sink lane"),
                    SEGMENT_OUTBOUND,
                );
                weekday.push(morning);
                weekday.push(evening);
                in_idx += 1;
            } else {
                let cell_lane = model.pick_origin_lane(u01(SEED, out_idx, SALT_OUT_ORIGIN));
                let morning = rec(
                    Profile::Morning,
                    u01(SEED, out_idx, SALT_OUT_DEP),
                    cell_lane,
                    sink_gw.sink_lane.expect("pick(Sink) returned sink lane"),
                    SEGMENT_OUTBOUND,
                );
                let evening = rec(
                    Profile::Evening,
                    u01(SEED, out_idx, SALT_OUT_RET_DEP),
                    spawn_gw
                        .spawn_lane
                        .expect("pick(Spawn) returned spawn lane"),
                    cell_lane,
                    SEGMENT_INBOUND,
                );
                weekday.push(morning);
                weekday.push(evening);
                out_idx += 1;
            }
            *gateway_volumes.entry(spawn_gw.node).or_insert(0) += 1;
            *gateway_volumes.entry(sink_gw.node).or_insert(0) += 1;
        }
    }
    let weekday_io = weekday.len() - weekday_internal;

    // --- authored through traffic (both day kinds) -------------------------
    let mut thr_wd_idx: u64 = 0;
    let mut thr_we_idx: u64 = 0;
    for (e_idx, entry) in authored.through.iter().enumerate() {
        let from = model.gateway_by_node(entry.from_gateway).ok_or_else(|| {
            format!(
                "authored fromGateway {} is not a gateway",
                entry.from_gateway
            )
        })?;
        let to = model
            .gateway_by_node(entry.to_gateway)
            .ok_or_else(|| format!("authored toGateway {} is not a gateway", entry.to_gateway))?;
        let origin_lane = from.spawn_lane.ok_or_else(|| {
            format!(
                "authored fromGateway {} has no spawn lane",
                entry.from_gateway
            )
        })?;
        let dest_lane = to
            .sink_lane
            .ok_or_else(|| format!("authored toGateway {} has no sink lane", entry.to_gateway))?;
        let expected = entry.veh_per_day * authored.trips_scale;
        let n_wd = stochastic_round(expected, u01(SEED, e_idx as u64, SALT_THR_COUNT_WD));
        for _ in 0..n_wd {
            let u = u01(SEED, thr_wd_idx, SALT_THR_DEP_WD);
            thr_wd_idx += 1;
            weekday.push(rec(
                Profile::Through,
                u,
                origin_lane,
                dest_lane,
                SEGMENT_THROUGH,
            ));
        }
        let n_we = stochastic_round(expected, u01(SEED, e_idx as u64, SALT_THR_COUNT_WE));
        for _ in 0..n_we {
            let u = u01(SEED, thr_we_idx, SALT_THR_DEP_WE);
            thr_we_idx += 1;
            weekend.push(rec(
                Profile::Through,
                u,
                origin_lane,
                dest_lane,
                SEGMENT_THROUGH,
            ));
        }
        *gateway_volumes.entry(from.node).or_insert(0) += n_wd as u64;
        *gateway_volumes.entry(to.node).or_insert(0) += n_wd as u64;
    }
    let weekday_through = weekday.len() - weekday_internal - weekday_io;

    // --- write --------------------------------------------------------------
    let mut out = Vec::new();
    write_trips(&mut out, &net_hash, &weekday, &weekend).map_err(|e| e.to_string())?;
    fs::write(&cfg.out_path, &out).map_err(|e| format!("{:?}: {e}", cfg.out_path))?;

    let mut gateway_volumes: Vec<(u32, u64)> = gateway_volumes.into_iter().collect();
    gateway_volumes.sort_by_key(|&(node, vol)| (std::cmp::Reverse(vol), node));

    Ok(BakeStats {
        weekday_total: weekday.len(),
        weekend_total: weekend.len(),
        weekday_by_segment: [
            weekday_internal,
            weekday
                .iter()
                .filter(|r| r.segment == SEGMENT_INBOUND)
                .count(),
            weekday
                .iter()
                .filter(|r| r.segment == SEGMENT_OUTBOUND)
                .count(),
            weekday_through,
        ],
        gateway_volumes,
        skipped_communes,
        furness_iters: furness.iters,
        furness_max_rel_err: furness.max_rel_err,
        net_hash_hex: blake3::Hash::from_bytes(net_hash).to_hex().to_string(),
    })
}

fn rec(profile: Profile, u: f32, origin_lane: u32, dest_lane: u32, segment: u8) -> TripRecord {
    TripRecord {
        departure_s: profiles::sample_departure_s(profile, u),
        origin_lane,
        dest_lane,
        segment,
        vehicle_class: 0,
    }
}

/// Deterministic integerization of an expected count: `floor + (u < frac)`.
pub fn stochastic_round(expected: f64, u: f32) -> u32 {
    let floor = expected.floor();
    let frac = expected - floor;
    floor as u32 + u32::from((u as f64) < frac)
}

/// One demand zone: a 200 m grid cell snapped to a lane.
#[derive(Debug, Clone, Copy)]
struct Zone {
    /// Cell center, world frame.
    x: f64,
    z: f64,
    weight: f64,
    /// Nearest drivable non-motorway lane id.
    lane: u32,
}

/// The zone system + gateway index built from the net and census inputs.
struct Model {
    origins: Vec<Zone>,
    dests: Vec<Zone>,
    /// Cumulative dest weights for weighted sampling (same order as `dests`).
    dest_cum: Vec<f64>,
    origin_cum: Vec<f64>,
    gateways: Vec<GwInfo>,
}

impl Model {
    fn build(
        net: &TrafficNet,
        statpop: &[inputs::Hectare],
        worksites: &[inputs::WorkSite],
    ) -> Result<Model, String> {
        // 200 m zone grid over the net extent
        let (mut min_x, mut min_z) = (f64::INFINITY, f64::INFINITY);
        let (mut max_x, mut max_z) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for n in &net.nodes {
            min_x = min_x.min(n.x as f64);
            max_x = max_x.max(n.x as f64);
            min_z = min_z.min(n.z as f64);
            max_z = max_z.max(n.z as f64);
        }
        if !min_x.is_finite() {
            return Err("net has no nodes".into());
        }
        let nx = (((max_x - min_x) / CELL_M).ceil() as usize).max(1);
        let nz = (((max_z - min_z) / CELL_M).ceil() as usize).max(1);
        let cell_of = |x: f64, z: f64| -> Option<usize> {
            let cx = ((x - min_x) / CELL_M).floor();
            let cz = ((z - min_z) / CELL_M).floor();
            (cx >= 0.0 && cz >= 0.0 && (cx as usize) < nx && (cz as usize) < nz)
                .then(|| cz as usize * nx + cx as usize)
        };

        let mut res = vec![0.0f64; nx * nz];
        let mut work = vec![0.0f64; nx * nz];
        for h in statpop {
            if let Some(c) = cell_of(h.x, h.z) {
                res[c] += h.residents;
            }
        }
        for w in worksites {
            if let Some(c) = cell_of(w.x, w.z) {
                work[c] += w.weight;
            }
        }

        // midpoints of drivable non-motorway lanes, ascending lane id
        let edge_speed: HashMap<u32, f32> = net.edges.iter().map(|e| (e.id, e.speed_ms)).collect();
        let mut lane_mid: Vec<(u32, f64, f64)> = net
            .lanes
            .iter()
            .filter(|l| edge_speed[&l.edge] < gateways::MOTORWAY_SPEED_MS)
            .map(|l| {
                let (p, _) = polyline_midpoint(&l.pts);
                (l.id, p[0] as f64, p[1] as f64)
            })
            .collect();
        lane_mid.sort_by_key(|&(id, _, _)| id);
        if lane_mid.is_empty() {
            return Err("net has no non-motorway lanes to snap zones onto".into());
        }
        let snap = |x: f64, z: f64| -> u32 {
            let mut best = lane_mid[0].0;
            let mut best_d = f64::INFINITY;
            for &(id, mx, mz) in &lane_mid {
                let d = (mx - x) * (mx - x) + (mz - z) * (mz - z);
                if d < best_d {
                    best_d = d;
                    best = id;
                }
            }
            best
        };

        // zones in ascending cell-index order (deterministic)
        let mut origins = Vec::new();
        let mut dests = Vec::new();
        for c in 0..nx * nz {
            if res[c] <= 0.0 && work[c] <= 0.0 {
                continue;
            }
            let x = min_x + ((c % nx) as f64 + 0.5) * CELL_M;
            let z = min_z + ((c / nx) as f64 + 0.5) * CELL_M;
            let lane = snap(x, z);
            if res[c] > 0.0 {
                origins.push(Zone {
                    x,
                    z,
                    weight: res[c],
                    lane,
                });
            }
            if work[c] > 0.0 {
                dests.push(Zone {
                    x,
                    z,
                    weight: work[c],
                    lane,
                });
            }
        }
        if origins.is_empty() {
            return Err("no residential zones (STATPOP empty or outside net extent)".into());
        }
        if dests.is_empty() {
            return Err("no work zones (land-use polygons empty or outside net extent)".into());
        }

        let cum = |zones: &[Zone]| -> Vec<f64> {
            let mut acc = 0.0;
            zones
                .iter()
                .map(|z| {
                    acc += z.weight;
                    acc
                })
                .collect()
        };
        Ok(Model {
            dest_cum: cum(&dests),
            origin_cum: cum(&origins),
            origins,
            dests,
            gateways: gateways::gateway_infos(net),
        })
    }

    fn internal_furness(&self, authored: &Authored) -> gravity::FurnessResult {
        let o: Vec<f64> = self
            .origins
            .iter()
            .map(|z| z.weight * authored.workers_per_resident * authored.car_share)
            .collect();
        let sum_o: f64 = o.iter().sum();
        let sum_work: f64 = self.dests.iter().map(|z| z.weight).sum();
        // D_j ∝ work weight, scaled so ΣD = ΣO
        let d: Vec<f64> = self
            .dests
            .iter()
            .map(|z| z.weight * sum_o / sum_work)
            .collect();
        // deterrence f(c) = exp(-c/λ), c = euclidean km between cell centers
        let mut f = Vec::with_capacity(o.len() * d.len());
        for oi in &self.origins {
            for dj in &self.dests {
                let dx = oi.x - dj.x;
                let dz = oi.z - dj.z;
                let c_km = (dx * dx + dz * dz).sqrt() / 1000.0;
                f.push((-c_km / authored.lambda_km).exp());
            }
        }
        gravity::furness(&o, &d, &f, FURNESS_TOL, FURNESS_MAX_ITER)
    }

    fn gateway_by_node(&self, node: u32) -> Option<&GwInfo> {
        self.gateways.iter().find(|g| g.node == node)
    }

    fn pick_dest_lane(&self, u: f32) -> u32 {
        pick_weighted(&self.dest_cum, u, |i| self.dests[i].lane)
    }

    fn pick_origin_lane(&self, u: f32) -> u32 {
        pick_weighted(&self.origin_cum, u, |i| self.origins[i].lane)
    }
}

/// Midpoint (at half the polyline arc length) of a lane polyline.
fn polyline_midpoint(pts: &[[f32; 2]]) -> ([f32; 2], f32) {
    let mut total = 0.0f32;
    for w in pts.windows(2) {
        total += ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
    }
    let half = total * 0.5;
    let mut acc = 0.0f32;
    for w in pts.windows(2) {
        let seg = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
        if acc + seg >= half && seg > 0.0 {
            let t = (half - acc) / seg;
            return (
                [
                    w[0][0] + (w[1][0] - w[0][0]) * t,
                    w[0][1] + (w[1][1] - w[0][1]) * t,
                ],
                total,
            );
        }
        acc += seg;
    }
    (*pts.last().expect("nonempty polyline"), total)
}

/// Weighted pick via a cumulative-sum table: index of the first entry whose
/// cumulative weight exceeds `u * total`.
fn pick_weighted(cum: &[f64], u: f32, lane_of: impl Fn(usize) -> u32) -> u32 {
    let total = *cum.last().expect("nonempty cumulative table");
    let target = (u as f64) * total;
    let idx = cum.partition_point(|&c| c <= target).min(cum.len() - 1);
    lane_of(idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stochastic_round_floor_plus_bernoulli() {
        assert_eq!(stochastic_round(2.0, 0.99), 2);
        assert_eq!(stochastic_round(2.3, 0.29), 3);
        assert_eq!(stochastic_round(2.3, 0.31), 2);
        assert_eq!(stochastic_round(0.0, 0.0), 0);
    }

    /// Golden byte-stability gate: the REAL bake, run twice, must produce
    /// byte-identical trips.bin. Needs the gitignored `scratch/demand/` +
    /// `scratch/geo/osm-landuse.json` artifacts → `#[ignore]`, run locally:
    /// `cargo test -p demand-gen --release -- --ignored golden`
    #[test]
    #[ignore = "needs local scratch/demand + scratch/geo data artifacts"]
    fn golden_real_bake_is_byte_stable() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let tmp = std::env::temp_dir().join("demand-gen-golden");
        std::fs::create_dir_all(&tmp).unwrap();
        let out = |name: &str| tmp.join(name);
        let cfg = |out_path: std::path::PathBuf| BakeConfig {
            net_path: root.join("data/winterthur/trafficnet.json"),
            demand_dir: root.join("scratch/demand"),
            authored_path: root.join("data/winterthur/demand-authored.json"),
            landuse_path: root.join("scratch/geo/osm-landuse.json"),
            out_path,
        };
        bake(&cfg(out("a.bin"))).unwrap();
        bake(&cfg(out("b.bin"))).unwrap();
        let a = std::fs::read(out("a.bin")).unwrap();
        let b = std::fs::read(out("b.bin")).unwrap();
        assert_eq!(blake3::hash(&a), blake3::hash(&b));
        assert_eq!(a, b);
    }

    #[test]
    fn pick_weighted_hits_all_buckets() {
        let cum = [1.0, 1.0, 3.0]; // weights 1, 0, 2
        assert_eq!(pick_weighted(&cum, 0.0, |i| i as u32), 0);
        assert_eq!(pick_weighted(&cum, 0.5, |i| i as u32), 2);
        assert_eq!(pick_weighted(&cum, 0.99, |i| i as u32), 2);
    }
}
