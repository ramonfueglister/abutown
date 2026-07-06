// src/diorama/traffic/cellGrid.ts
//
// The row-major 128 m AOI cell grid shared by the traffic channel AND the
// live (citizens) channel — extracted verbatim from trafficClient.ts so both
// WS clients derive IDENTICAL cell ids from the same trafficnet.json lanes
// (the server's live gateway reuses the traffic CellGrid, see
// backend/crates/winterthur-traffic/src/cells.rs — an off-by-one here
// silently produces empty subscriptions).
//
// A faithful port of the server's CellGrid so `cell = row*cols + col` ids
// line up on the wire, INCLUDING the vertex-keyed per-lane `(sEnd, cell)`
// breakpoints that `cell_of_lane_s` resolves against (cells.rs lines
// ~84-121/144-158). This is the ONE canonical cell-classification path —
// used by keyframe ghost-healing and stale-vehicle eviction alike (Task 9
// review finding 2: two independent approximations could disagree at a cell
// border and mis-evict).

import type { RawLane } from './deadReckon';

/** Must equal `CELL_SIZE_M` in backend/crates/winterthur-traffic/src/cells.rs. */
export const CELL_SIZE_M = 128;

/** One `(sEnd, cell)` breakpoint along a lane: for arc positions `s` in
 * `(prevSEnd, sEnd]` the vehicle is in `cell`. Mirrors Rust's `LaneSegment`
 * (cells.rs lines ~32-36). The last segment's `sEnd` is `+Infinity` so any `s`
 * past the declared lane length still resolves. */
interface LaneSegment {
  sEnd: number;
  cell: number;
}

export class CellGrid {
  readonly minX: number;
  readonly minZ: number;
  readonly cols: number;
  readonly rows: number;
  /** lane id -> ordered breakpoints (same construction as cells.rs). */
  private readonly laneSegments: Map<number, LaneSegment[]>;

  private constructor(
    minX: number,
    minZ: number,
    cols: number,
    rows: number,
    laneSegments: Map<number, LaneSegment[]>,
  ) {
    this.minX = minX;
    this.minZ = minZ;
    this.cols = cols;
    this.rows = rows;
    this.laneSegments = laneSegments;
  }

  /** Build from the baked lanes. Mirrors `plate_bbox` (bbox over every lane
   * vertex, [x, z]) then `CellGrid::build`'s cols/rows derivation, and walks
   * each lane's vertices in arc order emitting a new breakpoint whenever the
   * cell changes — byte-for-byte the same rule as cells.rs `CellGrid::build`. */
  static build(lanes: RawLane[]): CellGrid {
    let minX = Infinity;
    let minZ = Infinity;
    let maxX = -Infinity;
    let maxZ = -Infinity;
    for (const lane of lanes) {
      for (const p of lane.pts) {
        if (p[0] < minX) minX = p[0];
        if (p[1] < minZ) minZ = p[1];
        if (p[0] > maxX) maxX = p[0];
        if (p[1] > maxZ) maxZ = p[1];
      }
    }
    if (!Number.isFinite(minX)) {
      // Empty net fallback — matches the Rust unit-box fallback.
      minX = 0;
      minZ = 0;
      maxX = 1;
      maxZ = 1;
    }
    const cols = Math.floor((maxX - minX) / CELL_SIZE_M) + 1;
    const rows = Math.floor((maxZ - minZ) / CELL_SIZE_M) + 1;

    const cellOfXZ = (x: number, z: number): number => {
      const col = clamp(Math.floor((x - minX) / CELL_SIZE_M), 0, cols - 1);
      const row = clamp(Math.floor((z - minZ) / CELL_SIZE_M), 0, rows - 1);
      return row * cols + col;
    };

    const laneSegments = new Map<number, LaneSegment[]>();
    for (const lane of lanes) {
      const segs: LaneSegment[] = [];
      let acc = 0;
      let curCell = cellOfXZ(lane.pts[0][0], lane.pts[0][1]);
      for (let i = 1; i < lane.pts.length; i++) {
        const a = lane.pts[i - 1];
        const b = lane.pts[i];
        const dx = b[0] - a[0];
        const dz = b[1] - a[1];
        acc += Math.sqrt(dx * dx + dz * dz);
        const c = cellOfXZ(b[0], b[1]);
        if (c !== curCell) {
          segs.push({ sEnd: acc, cell: curCell });
          curCell = c;
        }
      }
      // Final run extends to +inf so any s past the declared length still
      // resolves to the lane's terminal cell.
      segs.push({ sEnd: Infinity, cell: curCell });
      laneSegments.set(lane.id, segs);
    }

    return new CellGrid(minX, minZ, cols, rows, laneSegments);
  }

  get cellCount(): number {
    return this.cols * this.rows;
  }

  /** Row-major cell id for a world (x, z). Clamps to the grid (matches Rust). */
  cellOf(x: number, z: number): number {
    const col = clamp(Math.floor((x - this.minX) / CELL_SIZE_M), 0, this.cols - 1);
    const row = clamp(Math.floor((z - this.minZ) / CELL_SIZE_M), 0, this.rows - 1);
    return row * this.cols + col;
  }

  /** The cell a vehicle at arc position `s` on `lane` (a lane id) occupies.
   * Direct port of `cells.rs::cell_of_lane_s`: a short linear scan over the
   * lane's precomputed breakpoints, no position interpolation. Returns -1 for
   * an unknown lane id (never happens for live vehicles). */
  cellOfLaneS(lane: number, s: number): number {
    const segs = this.laneSegments.get(lane);
    if (!segs) return -1;
    for (const seg of segs) {
      if (s <= seg.sEnd) return seg.cell;
    }
    // Unreachable — the last breakpoint's sEnd is +Infinity.
    return segs[segs.length - 1]?.cell ?? -1;
  }

  /** The (col, row) of a world position, clamped to the grid. */
  colRowOf(x: number, z: number): { col: number; row: number } {
    const col = clamp(Math.floor((x - this.minX) / CELL_SIZE_M), 0, this.cols - 1);
    const row = clamp(Math.floor((z - this.minZ) / CELL_SIZE_M), 0, this.rows - 1);
    return { col, row };
  }

  /** All valid cell ids within ±radius cells of the cell containing (x, z)
   * (a (2r+1)×(2r+1) block, clipped to the grid). radius 1 => 3×3, 2 => 5×5. */
  cellsAround(x: number, z: number, radius: number): Set<number> {
    const { col, row } = this.colRowOf(x, z);
    const out = new Set<number>();
    for (let dr = -radius; dr <= radius; dr++) {
      const r = row + dr;
      if (r < 0 || r >= this.rows) continue;
      for (let dc = -radius; dc <= radius; dc++) {
        const c = col + dc;
        if (c < 0 || c >= this.cols) continue;
        out.add(r * this.cols + c);
      }
    }
    return out;
  }
}

function clamp(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
}
