//! Input loading + coordinate plumbing: STATPOP hectares, the BFS commuter
//! matrix, commune centroids, OSM land-use polygons, and the authored
//! demand constants. All coordinates end up in the shared world frame
//! (x=east, z=south, meters, anchor lon 8.7285 / lat 47.5069 — the same
//! equirectangular projection as `scripts/geo/lib/project.mjs`).

use serde::Deserialize;
use std::fs;
use std::io;
use std::path::Path;

/// Shared projection anchor (`scripts/geo/lib/project.mjs` `ANCHOR`).
pub const ANCHOR_LON: f64 = 8.7285;
pub const ANCHOR_LAT: f64 = 47.5069;
/// Mean earth radius used by the projector.
pub const EARTH_R: f64 = 6_371_008.8;

/// LV95 → WGS84, swisstopo's published approximation formulas
/// ("Näherungslösungen für die direkte Transformation CH1903 ⇔ WGS84"),
/// accurate to ~1 m. Mirrors `scripts/geo/fetch-demand-data.mjs`.
/// Returns `(lon, lat)` in degrees.
pub fn lv95_to_wgs84(e: f64, n: f64) -> (f64, f64) {
    let y = (e - 2_600_000.0) / 1e6;
    let x = (n - 1_200_000.0) / 1e6;
    let lambda =
        2.6779094 + 4.728982 * y + 0.791484 * y * x + 0.1306 * y * x * x - 0.0436 * y * y * y;
    let phi = 16.9023892 + 3.238272 * x
        - 0.270978 * y * y
        - 0.002528 * x * x
        - 0.0447 * y * y * x
        - 0.014 * x * x * x;
    (lambda * 100.0 / 36.0, phi * 100.0 / 36.0)
}

/// WGS84 → world frame: equirectangular around the anchor,
/// `x = (lon-a.lon)·R·cos(a.lat)·π/180`, `z = -(lat-a.lat)·R·π/180`.
pub fn wgs84_to_world(lon: f64, lat: f64) -> (f64, f64) {
    let rad = std::f64::consts::PI / 180.0;
    let x = (lon - ANCHOR_LON) * EARTH_R * ANCHOR_LAT.to_radians().cos() * rad;
    let z = -(lat - ANCHOR_LAT) * EARTH_R * rad;
    (x, z)
}

/// One STATPOP hectare: world-frame center of the 100 m cell + residents.
#[derive(Debug, Clone, Copy)]
pub struct Hectare {
    pub x: f64,
    pub z: f64,
    pub residents: f64,
}

/// Parse `statpop.csv` (`E_KOORD,N_KOORD,BBTOT`, LV95 SW corners); the
/// returned points are the hectare CENTERS (+50 m each axis) in world coords.
pub fn load_statpop(path: &Path) -> io::Result<Vec<Hectare>> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines();
    expect_header(path, lines.next(), "E_KOORD,N_KOORD,BBTOT")?;
    lines
        .filter(|l| !l.is_empty())
        .map(|line| {
            let mut cols = line.split(',');
            let (e, n, pop) = (
                next_f64(path, line, &mut cols)?,
                next_f64(path, line, &mut cols)?,
                next_f64(path, line, &mut cols)?,
            );
            // SW corner → hectare center
            let (lon, lat) = lv95_to_wgs84(e + 50.0, n + 50.0);
            let (x, z) = wgs84_to_world(lon, lat);
            Ok(Hectare {
                x,
                z,
                residents: pop,
            })
        })
        .collect()
}

/// One commuter matrix row (`origin_bfs,dest_bfs,workers`).
#[derive(Debug, Clone, Copy)]
pub struct CommuterFlow {
    pub origin_bfs: u32,
    pub dest_bfs: u32,
    pub workers: f64,
}

/// Parse `pendlermatrix.csv`.
pub fn load_pendler(path: &Path) -> io::Result<Vec<CommuterFlow>> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines();
    expect_header(path, lines.next(), "origin_bfs,dest_bfs,workers")?;
    lines
        .filter(|l| !l.is_empty())
        .map(|line| {
            let mut cols = line.split(',');
            Ok(CommuterFlow {
                origin_bfs: next_f64(path, line, &mut cols)? as u32,
                dest_bfs: next_f64(path, line, &mut cols)? as u32,
                workers: next_f64(path, line, &mut cols)?,
            })
        })
        .collect()
}

/// One commune centroid (`bfs_nr,name,lon,lat`; name is quoted CSV).
#[derive(Debug, Clone)]
pub struct Commune {
    pub bfs: u32,
    pub lon: f64,
    pub lat: f64,
}

/// Parse `communes.csv`, in file order.
pub fn load_communes(path: &Path) -> io::Result<Vec<Commune>> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines();
    expect_header(path, lines.next(), "bfs_nr,name,lon,lat")?;
    lines
        .filter(|l| !l.is_empty())
        .map(|line| {
            // bfs_nr,"name, possibly with commas",lon,lat — name is always
            // double-quoted by the fetch script; lon/lat never contain commas.
            let bad = || {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{path:?}: unparseable commune row: {line}"),
                )
            };
            let (bfs_str, rest) = line.split_once(',').ok_or_else(bad)?;
            let rest = rest.strip_prefix('"').ok_or_else(bad)?;
            // names contain "" for embedded quotes; find the closing `",`
            let close = rest.find("\",").ok_or_else(bad)?;
            let coords = &rest[close + 2..];
            let (lon_str, lat_str) = coords.split_once(',').ok_or_else(bad)?;
            Ok(Commune {
                bfs: bfs_str.parse().map_err(|_| bad())?,
                lon: lon_str.parse().map_err(|_| bad())?,
                lat: lat_str.parse().map_err(|_| bad())?,
            })
        })
        .collect()
}

/// A work-attraction site derived from an OSM land-use polygon: world-frame
/// centroid + weight (area m² × class factor).
#[derive(Debug, Clone, Copy)]
pub struct WorkSite {
    pub x: f64,
    pub z: f64,
    pub weight: f64,
}

/// Class factor for destination weights (spec/plan Task 4):
/// commercial/industrial/retail 1.0, residential 0.15, everything else 0.
pub fn landuse_factor(landuse: &str) -> f64 {
    match landuse {
        "commercial" | "industrial" | "retail" => 1.0,
        "residential" => 0.15,
        _ => 0.0,
    }
}

/// Parse `osm-landuse.json` (Overpass JSON with `geometry` on ways and on
/// relation members). Each way — and each `outer` member ring of a relation —
/// with a nonzero-factor `landuse` tag yields one [`WorkSite`]: shoelace
/// area/centroid over world coords. Zero-area rings are dropped.
pub fn load_landuse_worksites(path: &Path) -> io::Result<Vec<WorkSite>> {
    #[derive(Deserialize)]
    struct Overpass {
        elements: Vec<Element>,
    }
    #[derive(Deserialize)]
    struct Element {
        #[serde(rename = "type")]
        kind: String,
        #[serde(default)]
        tags: Option<Tags>,
        #[serde(default)]
        geometry: Option<Vec<LonLat>>,
        #[serde(default)]
        members: Option<Vec<Member>>,
    }
    #[derive(Deserialize)]
    struct Tags {
        #[serde(default)]
        landuse: Option<String>,
    }
    #[derive(Deserialize)]
    struct Member {
        role: String,
        #[serde(default)]
        geometry: Option<Vec<LonLat>>,
    }
    #[derive(Deserialize)]
    struct LonLat {
        lon: f64,
        lat: f64,
    }

    let text = fs::read_to_string(path)?;
    let doc: Overpass = serde_json::from_str(&text)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{path:?}: {e}")))?;

    let ring_world = |g: &[LonLat]| -> Vec<(f64, f64)> {
        g.iter().map(|p| wgs84_to_world(p.lon, p.lat)).collect()
    };
    let mut sites = Vec::new();
    for el in &doc.elements {
        let Some(landuse) = el.tags.as_ref().and_then(|t| t.landuse.as_deref()) else {
            continue;
        };
        let factor = landuse_factor(landuse);
        if factor <= 0.0 {
            continue;
        }
        let mut rings: Vec<Vec<(f64, f64)>> = Vec::new();
        match el.kind.as_str() {
            "way" => {
                if let Some(g) = &el.geometry {
                    rings.push(ring_world(g));
                }
            }
            "relation" => {
                for m in el.members.as_deref().unwrap_or(&[]) {
                    if m.role == "outer"
                        && let Some(g) = &m.geometry
                    {
                        rings.push(ring_world(g));
                    }
                }
            }
            _ => {}
        }
        for ring in rings {
            if let Some((area, (x, z))) = ring_area_centroid(&ring) {
                sites.push(WorkSite {
                    x,
                    z,
                    weight: area.abs() * factor,
                });
            }
        }
    }
    Ok(sites)
}

/// Signed shoelace area (m²) and area-weighted centroid of a world-space
/// ring (closing edge implied). Returns `None` for degenerate (<1 m²) rings.
pub fn ring_area_centroid(pts: &[(f64, f64)]) -> Option<(f64, (f64, f64))> {
    if pts.len() < 3 {
        return None;
    }
    let mut area2 = 0.0f64; // 2 × signed area
    let mut cx = 0.0f64;
    let mut cz = 0.0f64;
    for i in 0..pts.len() {
        let (x0, z0) = pts[i];
        let (x1, z1) = pts[(i + 1) % pts.len()];
        let cross = x0 * z1 - x1 * z0;
        area2 += cross;
        cx += (x0 + x1) * cross;
        cz += (z0 + z1) * cross;
    }
    let area = area2 * 0.5;
    if area.abs() < 1.0 {
        return None; // degenerate (< 1 m²)
    }
    Some((area, (cx / (3.0 * area2), cz / (3.0 * area2))))
}

fn expect_header(path: &Path, line: Option<&str>, want: &str) -> io::Result<()> {
    if line != Some(want) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{path:?}: expected header {want:?}, got {line:?}"),
        ));
    }
    Ok(())
}

fn next_f64<'a>(
    path: &Path,
    line: &str,
    cols: &mut impl Iterator<Item = &'a str>,
) -> io::Result<f64> {
    cols.next().and_then(|c| c.parse().ok()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{path:?}: malformed row: {line}"),
        )
    })
}

/// The authored constants + through-traffic table
/// (`data/winterthur/demand-authored.json`).
#[derive(Debug, Clone, Deserialize)]
pub struct Authored {
    #[serde(default)]
    pub notes: Option<String>,
    pub trips_scale: f64,
    pub workers_per_resident: f64,
    pub car_share: f64,
    pub lambda_km: f64,
    pub through: Vec<ThroughEntry>,
}

/// One authored through-traffic flow (gateway → gateway).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThroughEntry {
    pub from_gateway: u32,
    pub to_gateway: u32,
    pub veh_per_day: f64,
    pub profile: String,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Load + validate the authored JSON (every through profile must be
/// `"through"` — the only profile authored entries may use in v1).
pub fn load_authored(path: &Path) -> io::Result<Authored> {
    let text = fs::read_to_string(path)?;
    let authored: Authored = serde_json::from_str(&text)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{path:?}: {e}")))?;
    for t in &authored.through {
        if t.profile != "through" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "authored through entry {}->{} has profile {:?}, only \"through\" is supported",
                    t.from_gateway, t.to_gateway, t.profile
                ),
            ));
        }
    }
    Ok(authored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lv95_to_wgs84_matches_swisstopo_anchor() {
        // same self-check as scripts/geo/fetch-demand-data.mjs
        let (lon, lat) = lv95_to_wgs84(2_696_000.0, 1_261_500.0);
        assert!((lon - 8.7127).abs() < 0.002, "lon {lon}");
        assert!((lat - 47.4972).abs() < 0.002, "lat {lat}");
    }

    #[test]
    fn wgs84_to_world_anchor_is_origin() {
        let (x, z) = wgs84_to_world(ANCHOR_LON, ANCHOR_LAT);
        assert!(x.abs() < 1e-9 && z.abs() < 1e-9);
        // one degree north ≈ 111.2 km, negative z (z = south)
        let (_, z) = wgs84_to_world(ANCHOR_LON, ANCHOR_LAT + 1.0);
        assert!((z + 111_194.9).abs() < 100.0, "z {z}");
    }

    #[test]
    fn shoelace_square() {
        // 100 m × 200 m axis-aligned rectangle
        let pts = [(0.0, 0.0), (100.0, 0.0), (100.0, 200.0), (0.0, 200.0)];
        let (area, (cx, cz)) = ring_area_centroid(&pts).unwrap();
        assert!((area.abs() - 20_000.0).abs() < 1e-6, "area {area}");
        assert!((cx - 50.0).abs() < 1e-6 && (cz - 100.0).abs() < 1e-6);
        // closed ring (repeated first point) gives the same result
        let closed = [
            (0.0, 0.0),
            (100.0, 0.0),
            (100.0, 200.0),
            (0.0, 200.0),
            (0.0, 0.0),
        ];
        let (area2, c2) = ring_area_centroid(&closed).unwrap();
        assert_eq!(area.abs(), area2.abs());
        assert_eq!((cx, cz), c2);
    }

    #[test]
    fn degenerate_ring_is_none() {
        assert!(ring_area_centroid(&[(0.0, 0.0), (1.0, 1.0)]).is_none());
    }
}
