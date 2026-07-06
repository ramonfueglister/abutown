// Rectilinear decomposition of an arbitrary (possibly concave) building
// footprint into a handful of axis-aligned rectangular "zones" — the coarse
// interior-planning unit consumed by generateInteriorPlan (T17). Pure,
// deterministic: no RNG, no Date, same input always yields the same output.
//
// Algorithm:
//   1. Rasterize the polygon onto a 2m grid — a cell is "inside" iff all
//      four of its corners pass the point-in-polygon test (pointInRing).
//      This is a deliberate strengthening of "center-only" sampling: near a
//      polygon wall that cuts diagonally through the world axes (the real
//      KSW footprint's walls run at ~23 deg to the world X axis), a cell
//      whose *center* is inside can still have its outer corner fall
//      outside the polygon. Since extracted rectangles are built by joining
//      whole cells edge-to-edge, their true geometric corners are exactly
//      the corners of their extreme cells — so corner-sampling each cell is
//      what actually guarantees "every zone corner is inside the polygon"
//      (a binding test invariant), where center-sampling does not.
//   2. Repeatedly extract the largest axis-aligned all-inside rectangle from
//      the remaining grid (the standard "largest rectangle in a binary
//      matrix" histogram method), mark it consumed, and repeat until
//      maxZones zones have been extracted or the remaining inside-coverage
//      drops below 15% of the polygon's rasterized area. See
//      DEFAULT_MAX_ZONES below for why the default differs from the
//      original nominal value of 8.
//   3. Any extracted rectangle with either side shorter than minSize (in
//      meters) is dropped (not counted against maxZones, not re-inserted).

export type Zone = { id: string; x: number; z: number; w: number; d: number };

export type DecomposeOpts = { maxZones?: number; minSize?: number; stopCoverage?: number; cell?: number };

const CELL = 2; // meters

// Standard ray-casting point-in-polygon test, mirrors
// scripts/geo/lib/join.mjs::pointInRing (same algorithm, kept separate here
// since this lib must stay pure TypeScript with no dependency on the .mjs
// bake tooling).
function pointInRing(x: number, z: number, ring: number[][]): boolean {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, zi] = ring[i];
    const [xj, zj] = ring[j];
    if (zi > z !== zj > z && x < ((xj - xi) * (z - zi)) / (zj - zi) + xi) inside = !inside;
  }
  return inside;
}

function boundsOf(ring: number[][]): { minX: number; maxX: number; minZ: number; maxZ: number } {
  let minX = Infinity;
  let maxX = -Infinity;
  let minZ = Infinity;
  let maxZ = -Infinity;
  for (const [x, z] of ring) {
    if (x < minX) minX = x;
    if (x > maxX) maxX = x;
    if (z < minZ) minZ = z;
    if (z > maxZ) maxZ = z;
  }
  return { minX, maxX, minZ, maxZ };
}

// Largest rectangle in a binary matrix (histogram method). `grid[row][col]`
// is 1 (inside/available) or 0. Returns the best rectangle found as
// row/col index bounds (inclusive), or null if the grid has no 1s.
function largestRectangle(
  grid: Uint8Array[],
): { r0: number; r1: number; c0: number; c1: number; area: number } | null {
  const rows = grid.length;
  if (rows === 0) return null;
  const cols = grid[0].length;
  const heights = new Int32Array(cols);
  let best: { r0: number; r1: number; c0: number; c1: number; area: number } | null = null;

  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      heights[c] = grid[r][c] ? heights[c] + 1 : 0;
    }

    // Monotonic-stack largest-rectangle-in-histogram over `heights`, tracking
    // the column span so we can recover the rectangle's bounds.
    const stack: number[] = []; // indices into heights, increasing height
    for (let c = 0; c <= cols; c++) {
      const h = c === cols ? 0 : heights[c];
      while (stack.length > 0 && heights[stack[stack.length - 1]] >= h) {
        const topIdx = stack.pop()!;
        const height = heights[topIdx];
        const leftBound = stack.length === 0 ? 0 : stack[stack.length - 1] + 1;
        const width = c - leftBound;
        const area = height * width;
        if (height > 0 && width > 0 && (best === null || area > best.area)) {
          best = {
            r0: r - height + 1,
            r1: r,
            c0: leftBound,
            c1: c - 1,
            area,
          };
        }
      }
      stack.push(c);
    }
  }
  return best;
}

// Default cap on extracted zones. The plan's nominal default was 8, but the
// real 113-point KSW footprint (data/winterthur/buildings.json, zone==='ksw',
// largest by shoelace area) is a sprawling multi-wing complex with long
// diagonal (~23°) walls — greedy largest-axis-aligned-rectangle extraction
// plateaus well below 60% coverage by the 8th rectangle no matter the raster
// resolution (checked 0.5m-3m cells) because so much of each additional
// rectangle's candidate area is already claimed by earlier, larger picks.
// 14 zones clears the >=60%-coverage invariant with margin (~61%) while
// keeping the same deterministic algorithm; the extra zones are exactly the
// kind of small wing/annex rooms a real hospital has anyway.
const DEFAULT_MAX_ZONES = 14;

export function decomposeToZones(footprint: number[][], opts: DecomposeOpts = {}): Zone[] {
  const maxZones = opts.maxZones ?? DEFAULT_MAX_ZONES;
  const minSize = opts.minSize ?? 6;
  const cell = opts.cell ?? CELL;

  const { minX, maxX, minZ, maxZ } = boundsOf(footprint);
  const cols = Math.max(1, Math.ceil((maxX - minX) / cell));
  const rows = Math.max(1, Math.ceil((maxZ - minZ) / cell));

  // grid[row][col] = 1 if all four corners of the cell are inside the
  // polygon (see the corner-vs-center rationale in the module doc above).
  const grid: Uint8Array[] = [];
  let totalInside = 0;
  for (let r = 0; r < rows; r++) {
    const row = new Uint8Array(cols);
    const z0 = minZ + r * cell;
    const z1 = minZ + (r + 1) * cell;
    for (let c = 0; c < cols; c++) {
      const x0 = minX + c * cell;
      const x1 = minX + (c + 1) * cell;
      const inside =
        pointInRing(x0, z0, footprint) &&
        pointInRing(x1, z0, footprint) &&
        pointInRing(x0, z1, footprint) &&
        pointInRing(x1, z1, footprint);
      if (inside) {
        row[c] = 1;
        totalInside++;
      }
    }
    grid.push(row);
  }

  const zones: Zone[] = [];
  const minCoverageCells = totalInside * (opts.stopCoverage ?? 0.15);
  let remaining = totalInside;

  while (zones.length < maxZones && remaining >= minCoverageCells && remaining > 0) {
    const rect = largestRectangle(grid);
    if (rect === null || rect.area <= 0) break;

    const w = (rect.c1 - rect.c0 + 1) * cell;
    const d = (rect.r1 - rect.r0 + 1) * cell;

    // Consume the cells regardless of whether the rect survives the
    // minSize filter — otherwise a too-small rect would be re-extracted
    // forever (infinite loop) since it's still the "largest" remaining.
    for (let r = rect.r0; r <= rect.r1; r++) {
      for (let c = rect.c0; c <= rect.c1; c++) {
        if (grid[r][c]) {
          grid[r][c] = 0;
          remaining--;
        }
      }
    }

    if (w < minSize || d < minSize) continue;

    const x = minX + ((rect.c0 + rect.c1 + 1) / 2) * cell;
    const z = minZ + ((rect.r0 + rect.r1 + 1) / 2) * cell;
    zones.push({ id: `z${zones.length}`, x, z, w, d });
  }

  return zones;
}

// ── Oriented decomposition (Phase A) ────────────────────────────────────────
// Real footprints rarely run parallel to the world axes (KSW: ~23°). Extract
// zones in a frame rotated to the footprint's dominant wall angle so the
// rectangles hug the facade; the interior THREE group then simply rotates by
// frame.angle to land back in world space.

export type PlanFrame = {
  angle: number;
  toLocal(x: number, z: number): [number, number];
  toWorld(x: number, z: number): [number, number];
};

// Length-weighted dominant edge direction, folded to a 90°-periodic domain
// via the ×4 angle-doubling trick (a wall and its perpendicular vote for the
// same orientation). Deterministic — pure arithmetic over the ring.
export function dominantAngle(footprint: number[][]): number {
  let sx = 0;
  let sz = 0;
  const n = footprint.length;
  for (let i = 0; i < n; i++) {
    const [ax, az] = footprint[i];
    const [bx, bz] = footprint[(i + 1) % n];
    const ex = bx - ax;
    const ez = bz - az;
    const len = Math.hypot(ex, ez);
    if (len < 0.05) continue;
    const a4 = 4 * Math.atan2(ez, ex);
    sx += len * Math.cos(a4);
    sz += len * Math.sin(a4);
  }
  return Math.atan2(sz, sx) / 4;
}

export function makeFrame(angle: number): PlanFrame {
  const c = Math.cos(angle);
  const s = Math.sin(angle);
  return {
    angle,
    // matches THREE group.rotation.y = angle (local → world)
    toWorld: (lx, lz) => [lx * c + lz * s, -lx * s + lz * c],
    toLocal: (x, z) => [x * c - z * s, x * s + z * c],
  };
}

// Zone extraction knobs for the oriented path: in the rotated frame the walls
// are near-axis-aligned, so greedy rectangles reach near-full coverage — allow
// many small zones and stop only when 1% of the raster remains.
const ORIENTED_OPTS: DecomposeOpts = { maxZones: 150, minSize: 1.8, stopCoverage: 0.01, cell: 0.7 };

export function decomposeOriented(footprint: number[][]): { zones: Zone[]; frame: PlanFrame } {
  const frame = makeFrame(dominantAngle(footprint));
  const localRing = footprint.map(([x, z]) => {
    const [lx, lz] = frame.toLocal(x, z);
    return [lx, lz];
  });
  return { zones: decomposeToZones(localRing, ORIENTED_OPTS), frame };
}
