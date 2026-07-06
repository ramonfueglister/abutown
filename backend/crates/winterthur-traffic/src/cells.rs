//! AOI (area-of-interest) cell mapping for the WS gateway (Task 8).
//!
//! The plate — the world-space bounding box of every lane polyline point — is
//! tiled by a fixed [`CELL_SIZE_M`]-metre grid. Each cell has a **row-major**
//! `u32` id: `cell = row * cols + col`, where `col`/`row` are the integer
//! offsets from the plate's minimum corner. The browser client subscribes to
//! the handful of cells its camera can see and receives only their vehicles.
//!
//! # Mapping a vehicle to a cell
//!
//! A vehicle's cell derives from its world position `pos_at(lane, s)`. Calling
//! `pos_at` (an O(log n) LUT binary search) once per vehicle per publish tick
//! (5 Hz) is cheap, but a lane rarely crosses a cell border — so we precompute,
//! once at boot, a per-lane list of `(s_end, cell)` breakpoints
//! ([`CellGrid::lane_segments`]). Resolving a vehicle's cell is then a short
//! linear scan over that lane's segments (typically 1–2 entries) with **no**
//! `pos_at` call and no allocation on the hot path.
//!
//! The grid is immutable after construction; the sim never reads it (the wire
//! must not feed back into the sim — determinism), it is a pure gateway concern.

use traffic_net::TrafficNet;

/// AOI cell edge length in metres. 128 m ≈ a city block; a typical camera sees
/// a small handful of cells.
pub const CELL_SIZE_M: f32 = 128.0;

/// One `(s_end, cell)` breakpoint along a lane: for arc positions `s` in
/// `(prev_s_end, s_end]` the vehicle is in `cell`. The first segment covers
/// `[0, s_end]`. The last segment's `s_end` is the lane length (clamped so any
/// `s` past the end still resolves).
#[derive(Debug, Clone, Copy, PartialEq)]
struct LaneSegment {
    s_end: f32,
    cell: u32,
}

/// The plate grid + per-lane cell breakpoints. Built once at boot from the net.
#[derive(Debug, Clone)]
pub struct CellGrid {
    /// Plate minimum corner (world x, z).
    min_x: f32,
    min_z: f32,
    /// Grid dimensions in cells.
    cols: u32,
    rows: u32,
    /// Per-lane breakpoints, indexed by lane **array position** (same order as
    /// `net.lanes`), *not* lane id. Resolve id→position via `lane_pos_of_id`.
    lane_segments: Vec<Vec<LaneSegment>>,
    /// lane id → array position, for the O(1) hot-path lookup.
    lane_pos_of_id: Vec<u32>,
}

impl CellGrid {
    /// Build the grid over `net`'s plate (the bbox of all lane points) and
    /// precompute each lane's `(s_end, cell)` breakpoints.
    ///
    /// A lane's breakpoints are found by walking its polyline vertices in arc
    /// order, emitting a new segment whenever the cell changes. Because a
    /// straight segment between two vertices could in principle skip across a
    /// cell it only clips a corner of, this is an approximation keyed on the
    /// vertices — acceptable at 128 m cells vs the ~10–40 m lane segments of
    /// the baked net (a lane vertex lands in every cell the lane meaningfully
    /// occupies). The client tolerates a vehicle appearing one cell early/late
    /// for one frame; a keyframe corrects membership within 5 s regardless.
    pub fn build(net: &TrafficNet) -> Self {
        let (min_x, min_z, max_x, max_z) = plate_bbox(net);
        // At least one cell in each axis even for a degenerate/empty net.
        let cols = (((max_x - min_x) / CELL_SIZE_M).floor() as u32) + 1;
        let rows = (((max_z - min_z) / CELL_SIZE_M).floor() as u32) + 1;

        let max_lane_id = net.lanes.iter().map(|l| l.id).max().unwrap_or(0);
        let mut lane_pos_of_id = vec![u32::MAX; (max_lane_id as usize) + 1];
        for (pos, lane) in net.lanes.iter().enumerate() {
            lane_pos_of_id[lane.id as usize] = pos as u32;
        }

        let cell_of = |x: f32, z: f32| -> u32 {
            let col = (((x - min_x) / CELL_SIZE_M).floor() as i64).clamp(0, cols as i64 - 1) as u32;
            let row = (((z - min_z) / CELL_SIZE_M).floor() as i64).clamp(0, rows as i64 - 1) as u32;
            row * cols + col
        };

        let mut lane_segments = Vec::with_capacity(net.lanes.len());
        for lane in &net.lanes {
            let mut segs: Vec<LaneSegment> = Vec::new();
            let mut acc = 0.0f32;
            // First vertex sets the initial cell.
            let mut cur_cell = cell_of(lane.pts[0][0], lane.pts[0][1]);
            for w in lane.pts.windows(2) {
                let dx = w[1][0] - w[0][0];
                let dz = w[1][1] - w[0][1];
                acc += (dx * dx + dz * dz).sqrt();
                let c = cell_of(w[1][0], w[1][1]);
                if c != cur_cell {
                    // Close the previous cell's run at this vertex's arc pos.
                    segs.push(LaneSegment {
                        s_end: acc,
                        cell: cur_cell,
                    });
                    cur_cell = c;
                }
            }
            // Final run extends to +inf so any s past the declared length still
            // resolves to the lane's terminal cell.
            segs.push(LaneSegment {
                s_end: f32::INFINITY,
                cell: cur_cell,
            });
            lane_segments.push(segs);
        }

        CellGrid {
            min_x,
            min_z,
            cols,
            rows,
            lane_segments,
            lane_pos_of_id,
        }
    }

    /// Total number of cells in the grid.
    pub fn cell_count(&self) -> u32 {
        self.cols * self.rows
    }

    /// Grid dimensions `(cols, rows)`.
    pub fn dims(&self) -> (u32, u32) {
        (self.cols, self.rows)
    }

    /// Plate minimum corner `(min_x, min_z)` in world metres. The client needs
    /// this plus [`CELL_SIZE_M`] and [`dims`](Self::dims) to map its camera
    /// rectangle to cell ids.
    pub fn origin(&self) -> (f32, f32) {
        (self.min_x, self.min_z)
    }

    /// The cell containing world position `(x, z)` in metres, clamped onto
    /// the plate (a building slightly outside the lane bbox maps to the
    /// nearest border cell). Used by the `/live` citizen publisher, which
    /// shares this exact grid with `/traffic` (same ids on the wire).
    pub fn cell_of_xz(&self, x: f32, z: f32) -> u32 {
        let col =
            (((x - self.min_x) / CELL_SIZE_M).floor() as i64).clamp(0, self.cols as i64 - 1) as u32;
        let row =
            (((z - self.min_z) / CELL_SIZE_M).floor() as i64).clamp(0, self.rows as i64 - 1) as u32;
        row * self.cols + col
    }

    /// The cell a vehicle at arc position `s` on `lane` occupies. `lane` is a
    /// lane **id**. Returns `None` for an unknown lane id (never happens for
    /// live vehicles, whose lane came from this same net). Hot path: one array
    /// index + a short linear scan, no allocation, no `pos_at`.
    pub fn cell_of_lane_s(&self, lane: u32, s: f32) -> Option<u32> {
        let pos = *self.lane_pos_of_id.get(lane as usize)?;
        if pos == u32::MAX {
            return None;
        }
        let segs = &self.lane_segments[pos as usize];
        for seg in segs {
            if s <= seg.s_end {
                return Some(seg.cell);
            }
        }
        // `s_end` of the last segment is +inf, so the loop always returns; this
        // is unreachable but keeps the type total.
        segs.last().map(|s| s.cell)
    }
}

/// World-space bounding box `(min_x, min_z, max_x, max_z)` over every lane
/// polyline vertex. The net's `Meta` carries no bbox, so we derive the plate
/// from the geometry itself. Falls back to a unit box for an empty net.
fn plate_bbox(net: &TrafficNet) -> (f32, f32, f32, f32) {
    let mut min_x = f32::INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for lane in &net.lanes {
        for p in &lane.pts {
            min_x = min_x.min(p[0]);
            min_z = min_z.min(p[1]);
            max_x = max_x.max(p[0]);
            max_z = max_z.max(p[1]);
        }
    }
    if !min_x.is_finite() {
        return (0.0, 0.0, 1.0, 1.0);
    }
    (min_x, min_z, max_x, max_z)
}
