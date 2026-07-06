// scripts/geo/lib/grading.mjs
//
// Terrain grading kernel: levels road/rail corridors into a local-meter DEM
// grid (embankments/cuts), so the eventual mesh doesn't cross-slope across a
// carriageway. Pure math — no I/O, no bake wiring (that lands in a later
// task alongside encodeTile/extractPatch).
//
// Grid convention: this operates on a LOCAL-METER grid, the same row-major
// layout tiles.mjs uses for extracted DEM patches (see encodeTile /
// extractPatch in dem.mjs): `data[j*ncols+i]`, i steps with world x (east),
// j steps with world z (south), cell (0,0)'s centre sits at (xll, yll) in
// local metres, and cellsize is isotropic. This is NOT parseAAIGrid's raw
// geographic grid (lon/lat degrees, row 0 = north, celldx/celldy can
// differ) — that grid only ever gets read through the bilinear
// heightAt(x,z) sampler, never indexed directly. The local grid used here
// has no north/south flip: row index increases together with world z.
//
// No fallbacks: invalid `kind` or malformed way input throws rather than
// silently defaulting.

/** cell-centre world x for column i */
function worldXAt(grid, i) {
  return grid.xll + i * grid.cellsize;
}
/** cell-centre world z for row j */
function worldZAt(grid, j) {
  return grid.yll + j * grid.cellsize;
}
/** nearest column for a world x (not clamped) */
function colAt(grid, x) {
  return (x - grid.xll) / grid.cellsize;
}
/** nearest row for a world z (not clamped) */
function rowAt(grid, z) {
  return (z - grid.yll) / grid.cellsize;
}

function smoothstep(t) {
  const c = Math.max(0, Math.min(1, t));
  return c * c * (3 - 2 * c);
}

/**
 * Moving-average smoothing over `windowM`, then a two-pass grade limiter:
 * forward pass clamps the rise per step to `maxGrade * stepM`, backward pass
 * clamps the fall the same way. Deterministic, pure array math.
 */
export function smoothProfile(samples, stepM, windowM, maxGrade) {
  if (!Array.isArray(samples) || samples.length === 0) {
    throw new Error('smoothProfile: samples must be a non-empty array');
  }
  if (!(stepM > 0)) throw new Error('smoothProfile: stepM must be > 0');
  if (!(windowM > 0)) throw new Error('smoothProfile: windowM must be > 0');
  if (!(maxGrade > 0)) throw new Error('smoothProfile: maxGrade must be > 0');

  const n = samples.length;
  const halfWin = Math.max(1, Math.round(windowM / stepM / 2));
  const avg = new Array(n);
  for (let i = 0; i < n; i++) {
    let sum = 0;
    let count = 0;
    for (let k = -halfWin; k <= halfWin; k++) {
      const idx = i + k;
      if (idx < 0 || idx >= n) continue;
      sum += samples[idx];
      count++;
    }
    avg[i] = sum / count;
  }

  const maxStep = maxGrade * stepM;

  // Forward pass: clamp rise.
  const fwd = avg.slice();
  for (let i = 1; i < n; i++) {
    const delta = fwd[i] - fwd[i - 1];
    if (delta > maxStep) fwd[i] = fwd[i - 1] + maxStep;
    else if (delta < -maxStep) fwd[i] = fwd[i - 1] - maxStep;
  }
  // Backward pass: clamp fall, applied over the forward result.
  const out = fwd.slice();
  for (let i = n - 2; i >= 0; i--) {
    const delta = out[i] - out[i + 1];
    if (delta > maxStep) out[i] = out[i + 1] + maxStep;
    else if (delta < -maxStep) out[i] = out[i + 1] - maxStep;
  }
  return out;
}

/** Densify a polyline to ≤ 2 m steps. Returns [[x,z], ...] including all original vertices. */
function densify(pts, maxStepM = 2) {
  if (!Array.isArray(pts) || pts.length < 2) {
    throw new Error('densify: pts must have at least 2 points');
  }
  const out = [pts[0]];
  for (let i = 0; i < pts.length - 1; i++) {
    const [x0, z0] = pts[i];
    const [x1, z1] = pts[i + 1];
    const segLen = Math.hypot(x1 - x0, z1 - z0);
    const steps = Math.max(1, Math.ceil(segLen / maxStepM));
    for (let s = 1; s <= steps; s++) {
      const t = s / steps;
      out.push([x0 + (x1 - x0) * t, z0 + (z1 - z0) * t]);
    }
  }
  return out;
}

function polylineArcLengths(pts) {
  const arc = [0];
  for (let i = 1; i < pts.length; i++) {
    const [x0, z0] = pts[i - 1];
    const [x1, z1] = pts[i];
    arc.push(arc[i - 1] + Math.hypot(x1 - x0, z1 - z0));
  }
  return arc;
}

/**
 * Nearest point on the densified centreline to (x,z). Returns
 * { d, h, index } — perpendicular-ish distance (nearest-vertex distance),
 * the graded profile height at that vertex, and its index.
 */
function nearestOnCenterline(pts, profile, x, z) {
  let bestD2 = Infinity;
  let bestIdx = 0;
  for (let i = 0; i < pts.length; i++) {
    const dx = pts[i][0] - x;
    const dz = pts[i][1] - z;
    const d2 = dx * dx + dz * dz;
    if (d2 < bestD2) {
      bestD2 = d2;
      bestIdx = i;
    }
  }
  return { d: Math.sqrt(bestD2), h: profile[bestIdx], index: bestIdx };
}

function bboxOfWay(pts, pad) {
  let minX = Infinity, maxX = -Infinity, minZ = Infinity, maxZ = -Infinity;
  for (const [x, z] of pts) {
    if (x < minX) minX = x;
    if (x > maxX) maxX = x;
    if (z < minZ) minZ = z;
    if (z > maxZ) maxZ = z;
  }
  return { minX: minX - pad, maxX: maxX + pad, minZ: minZ - pad, maxZ: maxZ + pad };
}

/** Ray-cast point-in-polygon (even-odd rule), ring = [[x,z], ...]. */
function pointInRing(x, z, ring) {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, zi] = ring[i];
    const [xj, zj] = ring[j];
    const intersects = (zi > z) !== (zj > z) &&
      x < ((xj - xi) * (z - zi)) / (zj - zi) + xi;
    if (intersects) inside = !inside;
  }
  return inside;
}

function pointInAnyRing(x, z, rings) {
  for (const ring of rings) if (pointInRing(x, z, ring)) return true;
  return false;
}

function validateWay(way) {
  if (!way || !Array.isArray(way.pts) || way.pts.length < 2) {
    throw new Error('gradeDem: way.pts must have at least 2 points');
  }
  if (!(way.halfWidthM > 0)) throw new Error('gradeDem: way.halfWidthM must be > 0');
  if (!(way.blendM >= 0)) throw new Error('gradeDem: way.blendM must be >= 0');
  if (!(way.windowM > 0)) throw new Error('gradeDem: way.windowM must be > 0');
  if (!(way.maxGrade > 0)) throw new Error('gradeDem: way.maxGrade must be > 0');
  if (way.kind !== 'road' && way.kind !== 'rail') {
    throw new Error(`gradeDem: way.kind must be 'road' or 'rail', got ${JSON.stringify(way.kind)}`);
  }
}

/**
 * Precompute a way's densified centreline + graded profile + bbox, sampling
 * pre-grading heights straight from the (unmutated-so-far) dem grid.
 */
function prepareWay(dem, way) {
  const dense = densify(way.pts, 2);
  const arc = polylineArcLengths(dense);
  const totalLen = arc[arc.length - 1];
  const stepM = totalLen / Math.max(1, dense.length - 1);
  const rawHeights = dense.map(([x, z]) => sampleGrid(dem, x, z));
  const profile = smoothProfile(rawHeights, stepM > 0 ? stepM : 1, way.windowM, way.maxGrade);
  const bbox = bboxOfWay(dense, way.halfWidthM + way.blendM);
  return { dense, profile, bbox };
}

/** Bilinear sample of the (possibly already-graded) grid at world (x,z). */
function sampleGrid(dem, x, z) {
  const col = colAt(dem, x);
  const row = rowAt(dem, z);
  const c0 = Math.max(0, Math.min(dem.ncols - 2, Math.floor(col)));
  const r0 = Math.max(0, Math.min(dem.nrows - 2, Math.floor(row)));
  const fc = Math.max(0, Math.min(1, col - c0));
  const fr = Math.max(0, Math.min(1, row - r0));
  const at = (r, c) => dem.data[r * dem.ncols + c];
  return at(r0, c0) * (1 - fc) * (1 - fr) + at(r0, c0 + 1) * fc * (1 - fr) +
    at(r0 + 1, c0) * (1 - fc) * fr + at(r0 + 1, c0 + 1) * fc * fr;
}

function weightAt(d, halfWidthM, blendM) {
  if (d <= halfWidthM) return 1;
  if (blendM <= 0) return 0;
  const t = (d - halfWidthM) / blendM;
  if (t >= 1) return 0;
  return 1 - smoothstep(t);
}

/**
 * Accumulate one kind-group's ways (all 'road', or all 'rail') into a single
 * shared (sumW, sumWH) layer covering the union of their bboxes, then blend
 * that shared layer into dem.data once. This makes overlapping same-kind
 * corridors (junction aprons, §4.3) order-independent: two ways contributing
 * to the same cell sum their weights/weighted-heights before either is
 * divided out, rather than each way blending into the DEM (and thus into
 * the next way's `orig` read) separately.
 *
 * Also tracks, per way, the world-space centres of skipped water cells so
 * the caller can decide bridge sites by total corridor crossing rather than
 * a per-raster-row consecutive-run heuristic (see FINDING 2).
 */
function accumulateKindGroup(dem, wayGroup, waterRings, sharedWaterSkipped) {
  const { ncols, nrows } = dem;
  if (wayGroup.length === 0) {
    return { cellsChanged: 0, waterSkippedByWay: [] };
  }

  const prepared = wayGroup.map((way) => ({ way, ...prepareWay(dem, way) }));

  // Union bbox across the whole kind-group's ways.
  let minX = Infinity, maxX = -Infinity, minZ = Infinity, maxZ = -Infinity;
  for (const { bbox } of prepared) {
    if (bbox.minX < minX) minX = bbox.minX;
    if (bbox.maxX > maxX) maxX = bbox.maxX;
    if (bbox.minZ < minZ) minZ = bbox.minZ;
    if (bbox.maxZ > maxZ) maxZ = bbox.maxZ;
  }
  const c0 = Math.max(0, Math.floor(colAt(dem, minX)));
  const c1 = Math.min(ncols - 1, Math.ceil(colAt(dem, maxX)));
  const r0 = Math.max(0, Math.floor(rowAt(dem, minZ)));
  const r1 = Math.min(nrows - 1, Math.ceil(rowAt(dem, maxZ)));
  const boxCols = Math.max(0, c1 - c0 + 1);
  const boxRows = Math.max(0, r1 - r0 + 1);

  const sumW = new Float64Array(boxCols * boxRows);
  const sumWH = new Float64Array(boxCols * boxRows);
  const waterCell = new Uint8Array(boxCols * boxRows);

  // Per-way water-skip tracking for orientation-independent bridge detection.
  const waterSkippedByWay = prepared.map(() => []);

  for (let r = r0; r <= r1; r++) {
    for (let c = c0; c <= c1; c++) {
      const x = worldXAt(dem, c);
      const z = worldZAt(dem, r);
      const boxIdx = (r - r0) * boxCols + (c - c0);
      const inWater = pointInAnyRing(x, z, waterRings);

      for (let wi = 0; wi < prepared.length; wi++) {
        const { way, dense, profile } = prepared[wi];
        const { d, h } = nearestOnCenterline(dense, profile, x, z);
        const corridorR = way.halfWidthM + way.blendM;
        if (d > corridorR) continue;

        if (inWater) {
          waterCell[boxIdx] = 1;
          sharedWaterSkipped.count++;
          waterSkippedByWay[wi].push({ x, z, d });
          continue;
        }

        const w = weightAt(d, way.halfWidthM, way.blendM);
        if (w > 0) {
          sumW[boxIdx] += w;
          sumWH[boxIdx] += w * h;
        }
      }
    }
  }

  let cellsChanged = 0;
  for (let r = r0; r <= r1; r++) {
    for (let c = c0; c <= c1; c++) {
      const boxIdx = (r - r0) * boxCols + (c - c0);
      if (waterCell[boxIdx]) continue;
      const w = sumW[boxIdx];
      if (w <= 0) continue;
      const gridIdx = r * ncols + c;
      const orig = dem.data[gridIdx];
      const t = Math.min(w, 1);
      const graded = t * (sumWH[boxIdx] / w) + (1 - t) * orig;
      if (graded !== dem.data[gridIdx]) cellsChanged++;
      dem.data[gridIdx] = graded;
    }
  }

  return { cellsChanged, waterSkippedByWay };
}

/**
 * Decide bridge sites for one kind-group from its per-way skipped-water-cell
 * lists: a way is flagged when it has >= 3 skipped water cells whose centres
 * lie within 2*cellsize of its densified centreline (i.e. the way's own
 * line actually crosses the water, not merely its blend zone brushing a
 * lake edge). Orientation-independent — unlike a per-raster-row consecutive
 * run, this counts total qualifying cells across the whole way regardless
 * of whether the water crossing is aligned with rows or columns.
 */
function bridgeSitesForGroup(wayGroup, waterSkippedByWay, cellsize) {
  const sites = [];
  const nearThreshold = 2 * cellsize;
  for (let wi = 0; wi < wayGroup.length; wi++) {
    const way = wayGroup[wi];
    const cells = waterSkippedByWay[wi];
    const near = cells.filter((cell) => cell.d <= nearThreshold);
    if (near.length >= 3) {
      const first = near[0];
      sites.push({ x: first.x, z: first.z, kind: way.kind });
    }
  }
  return sites;
}

/**
 * Grade a local-meter DEM grid in place for a set of road/rail ways.
 * Returns the report object { cellsChanged, waterSkippedCells, bridgeSites, originDeltaM }.
 *
 * Per spec §4.3: ALL roads accumulate into one shared (sumW, sumWH) layer
 * and are blended into the DEM together (so overlapping same-kind
 * corridors — junction aprons — come out order-independent), THEN all
 * rails accumulate into a second shared layer and blend on top, overriding
 * the road-graded value at crossings.
 */
export function gradeDem(dem, ways, opts) {
  if (!dem || !(dem.data instanceof Float64Array || dem.data instanceof Float32Array)) {
    throw new Error('gradeDem: dem.data must be a typed array');
  }
  if (!Array.isArray(ways)) throw new Error('gradeDem: ways must be an array');
  if (!opts || !Array.isArray(opts.waterRings)) {
    throw new Error('gradeDem: opts.waterRings must be an array');
  }
  for (const way of ways) validateWay(way);

  const waterRings = opts.waterRings;
  const origin00Before = sampleGrid(dem, 0, 0);

  const roads = ways.filter((w) => w.kind === 'road');
  const rails = ways.filter((w) => w.kind === 'rail');

  let cellsChanged = 0;
  const sharedWaterSkipped = { count: 0 };
  const bridgeSites = [];

  for (const group of [roads, rails]) {
    if (group.length === 0) continue;
    const { cellsChanged: groupChanged, waterSkippedByWay } = accumulateKindGroup(
      dem,
      group,
      waterRings,
      sharedWaterSkipped,
    );
    cellsChanged += groupChanged;
    bridgeSites.push(...bridgeSitesForGroup(group, waterSkippedByWay, dem.cellsize));
  }

  const origin00After = sampleGrid(dem, 0, 0);
  const originDeltaM = Math.abs(origin00After - origin00Before);

  return { cellsChanged, waterSkippedCells: sharedWaterSkipped.count, bridgeSites, originDeltaM };
}

/**
 * Point-in-corridor predicate over the same distance math as gradeDem's
 * rasterization (no blend falloff — a hard halfWidthM boundary). Used for
 * tree clearing.
 */
export function makeCorridorMask(ways) {
  if (!Array.isArray(ways)) throw new Error('makeCorridorMask: ways must be an array');
  const prepared = ways.map((way) => {
    if (!way || !Array.isArray(way.pts) || way.pts.length < 2) {
      throw new Error('makeCorridorMask: way.pts must have at least 2 points');
    }
    if (!(way.halfWidthM > 0)) throw new Error('makeCorridorMask: way.halfWidthM must be > 0');
    return { dense: densify(way.pts, 2), halfWidthM: way.halfWidthM };
  });
  return function corridorMask(x, z) {
    for (const { dense, halfWidthM } of prepared) {
      let bestD2 = Infinity;
      for (const [px, pz] of dense) {
        const dx = px - x;
        const dz = pz - z;
        const d2 = dx * dx + dz * dz;
        if (d2 < bestD2) bestD2 = d2;
      }
      if (Math.sqrt(bestD2) <= halfWidthM) return true;
    }
    return false;
  };
}
