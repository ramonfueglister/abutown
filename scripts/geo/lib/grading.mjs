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

/**
 * Bbox-prefiltered water tester over a ring set. At municipality scale the
 * bake passes ~3.4k rings (515 polygons + ~2.9k buffered river centrelines);
 * the naive for-all-rings pointInRing per corridor cell was the second-biggest
 * hot spot after the accumulator itself. A per-ring bbox check rejects almost
 * every ring in O(1) before the raycast runs.
 */
function makeWaterTest(rings) {
  const items = rings.map((ring) => {
    let minX = Infinity, maxX = -Infinity, minZ = Infinity, maxZ = -Infinity;
    for (const [x, z] of ring) {
      if (x < minX) minX = x;
      if (x > maxX) maxX = x;
      if (z < minZ) minZ = z;
      if (z > maxZ) maxZ = z;
    }
    return { ring, minX, maxX, minZ, maxZ };
  });
  return (x, z) => {
    for (const it of items) {
      if (x < it.minX || x > it.maxX || z < it.minZ || z > it.maxZ) continue;
      if (pointInRing(x, z, it.ring)) return true;
    }
    return false;
  };
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
 * Precompute a way's densified centreline + graded profile, sampling
 * pre-grading heights straight from the (unmutated-so-far) dem grid.
 */
function prepareWay(dem, way) {
  const dense = densify(way.pts, 2);
  const arc = polylineArcLengths(dense);
  const totalLen = arc[arc.length - 1];
  const stepM = totalLen / Math.max(1, dense.length - 1);
  const rawHeights = dense.map(([x, z]) => sampleGrid(dem, x, z));
  const profile = smoothProfile(rawHeights, stepM > 0 ? stepM : 1, way.windowM, way.maxGrade);
  return { dense, arc, profile };
}

/** Linear-interpolate the grade-clamped profile at a target arc-length s. */
function profileAtArc(arc, profile, s) {
  const total = arc[arc.length - 1];
  if (s <= 0) return profile[0];
  if (s >= total) return profile[profile.length - 1];
  // arc is monotonically non-decreasing; find the bracketing dense segment.
  let lo = 0;
  let hi = arc.length - 1;
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1;
    if (arc[mid] <= s) lo = mid;
    else hi = mid;
  }
  const seg = arc[hi] - arc[lo];
  if (seg <= 0) return profile[lo];
  const t = (s - arc[lo]) / seg;
  return profile[lo] * (1 - t) + profile[hi] * t;
}

/**
 * Resample a way's grade-clamped longitudinal profile onto fixed 10 m
 * arc-length stations from the start, with the exact endpoint appended.
 * Convention: ys[k] is the profile height at arc-length min(10*k, totalLen)
 * for k in [0, floor(totalLen/10)], then one final entry at the exact
 * endpoint (arc = totalLen). Length = floor(totalLen/10) + 2. The last two
 * entries coincide when totalLen is a multiple of 10, keeping index math
 * uniform. These are the SAME grade-clamped values the rasterization stamps
 * (profileAtArc reads the identical `profile` array), so the road-owned
 * profile matches the terrain grading exactly.
 */
const PROFILE_STEP_M = 10;
function resampleProfile(arc, profile) {
  const total = arc[arc.length - 1];
  const k = Math.floor(total / PROFILE_STEP_M);
  const ys = [];
  for (let i = 0; i <= k; i++) {
    ys.push(profileAtArc(arc, profile, i * PROFILE_STEP_M));
  }
  ys.push(profileAtArc(arc, profile, total)); // exact endpoint
  return { stepM: PROFILE_STEP_M, ys };
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
function accumulateKindGroup(dem, wayGroup, waterTest, sharedWaterSkipped) {
  const { ncols, nrows, cellsize } = dem;
  if (wayGroup.length === 0) {
    return { cellsChanged: 0, waterSkippedByWay: [], profiles: [] };
  }

  const prepared = wayGroup.map((way) => ({ way, ...prepareWay(dem, way) }));
  // Per-way 10 m-station profiles, index-aligned with wayGroup, read from the
  // SAME grade-clamped `profile` array the rasterization stamps below.
  const profiles = prepared.map((p) => resampleProfile(p.arc, p.profile));

  // Sparse accumulators keyed by grid cell index. The first version of this
  // kernel allocated dense (sumW, sumWH) arrays over the UNION bbox of the
  // whole kind-group and, per cell, scanned EVERY way's full densified
  // centreline — O(unionCells × ways × densePoints). Fine on the 60×60 test
  // grids; at municipality scale (3.7M cells × ~20k roads × ~10² points each)
  // that is >1e11 inner scans and never finishes. The stamping formulation
  // below computes the exact same quantities: for each way, walk its dense
  // points and visit only the cells within corridorR of that point; per
  // (way, cell), keep the minimum point distance (first argmin wins on exact
  // ties, matching the old ascending nearest-vertex scan). Cells farther than
  // corridorR from every dense point never mattered before either (`d >
  // corridorR` skipped them), so the visited set — and every d/h/weight — is
  // identical; only the representation is sparse. Work drops to
  // O(densePoints × (corridorR/cellsize)²) ≈ a few 1e7 at city scale.
  const sumW = new Map(); // gridIdx -> Σ weight
  const sumWH = new Map(); // gridIdx -> Σ weight·height
  const waterCells = new Set(); // gridIdx marked water inside some corridor

  // Per-way water-skip tracking for orientation-independent bridge detection.
  const waterSkippedByWay = prepared.map(() => []);
  const waterCache = new Map(); // gridIdx -> boolean (waterTest is per-cell)

  for (let wi = 0; wi < prepared.length; wi++) {
    const { way, dense, profile } = prepared[wi];
    const corridorR = way.halfWidthM + way.blendM;
    const corridorR2 = corridorR * corridorR;
    const reach = Math.ceil(corridorR / cellsize);

    // Stamp: gridIdx -> { d2, h } for this way's nearest dense point.
    const best = new Map();
    for (let pi = 0; pi < dense.length; pi++) {
      const px = dense[pi][0];
      const pz = dense[pi][1];
      const h = profile[pi];
      const pc = Math.round(colAt(dem, px));
      const pr = Math.round(rowAt(dem, pz));
      const rLo = Math.max(0, pr - reach);
      const rHi = Math.min(nrows - 1, pr + reach);
      const cLo = Math.max(0, pc - reach);
      const cHi = Math.min(ncols - 1, pc + reach);
      for (let r = rLo; r <= rHi; r++) {
        const dz = worldZAt(dem, r) - pz;
        for (let c = cLo; c <= cHi; c++) {
          const dx = worldXAt(dem, c) - px;
          const d2 = dx * dx + dz * dz;
          if (d2 > corridorR2) continue;
          const idx = r * ncols + c;
          const cur = best.get(idx);
          if (cur === undefined) best.set(idx, { d2, h });
          else if (d2 < cur.d2) {
            cur.d2 = d2;
            cur.h = h;
          }
        }
      }
    }

    // Fold this way into the shared kind-group layer in row-major cell order
    // (sorted numeric grid index), so water-skip lists and float accumulation
    // order are deterministic and independent of Map insertion order.
    const cellIdxs = [...best.keys()].sort((a, b) => a - b);
    for (const idx of cellIdxs) {
      const { d2, h } = best.get(idx);
      const r = (idx / ncols) | 0;
      const c = idx % ncols;
      const x = worldXAt(dem, c);
      const z = worldZAt(dem, r);
      let inWater = waterCache.get(idx);
      if (inWater === undefined) {
        inWater = waterTest(x, z);
        waterCache.set(idx, inWater);
      }
      if (inWater) {
        waterCells.add(idx);
        sharedWaterSkipped.count++;
        waterSkippedByWay[wi].push({ x, z, d: Math.sqrt(d2) });
        continue;
      }
      const w = weightAt(Math.sqrt(d2), way.halfWidthM, way.blendM);
      if (w > 0) {
        sumW.set(idx, (sumW.get(idx) ?? 0) + w);
        sumWH.set(idx, (sumWH.get(idx) ?? 0) + w * h);
      }
    }
  }

  // Blend the shared layer into dem.data once, in row-major cell order.
  let cellsChanged = 0;
  const idxs = [...sumW.keys()].sort((a, b) => a - b);
  for (const idx of idxs) {
    if (waterCells.has(idx)) continue;
    const w = sumW.get(idx);
    if (w <= 0) continue;
    const orig = dem.data[idx];
    const t = Math.min(w, 1);
    const graded = t * (sumWH.get(idx) / w) + (1 - t) * orig;
    if (graded !== dem.data[idx]) cellsChanged++;
    dem.data[idx] = graded;
  }

  return { cellsChanged, waterSkippedByWay, profiles };
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

  const waterTest = makeWaterTest(opts.waterRings);
  const origin00Before = sampleGrid(dem, 0, 0);

  // Keep each way's original index so per-kind-group profiles can be
  // reassembled into one array index-aligned with the caller's `ways`.
  const roads = [];
  const rails = [];
  for (let idx = 0; idx < ways.length; idx++) {
    const w = ways[idx];
    if (w.kind === 'road') roads.push({ w, idx });
    else rails.push({ w, idx });
  }

  let cellsChanged = 0;
  const sharedWaterSkipped = { count: 0 };
  const bridgeSites = [];
  const profiles = new Array(ways.length);

  for (const tagged of [roads, rails]) {
    if (tagged.length === 0) continue;
    const group = tagged.map((t) => t.w);
    const { cellsChanged: groupChanged, waterSkippedByWay, profiles: groupProfiles } = accumulateKindGroup(
      dem,
      group,
      waterTest,
      sharedWaterSkipped,
    );
    cellsChanged += groupChanged;
    bridgeSites.push(...bridgeSitesForGroup(group, waterSkippedByWay, dem.cellsize));
    for (let gi = 0; gi < tagged.length; gi++) profiles[tagged[gi].idx] = groupProfiles[gi];
  }

  const origin00After = sampleGrid(dem, 0, 0);
  const originDeltaM = Math.abs(origin00After - origin00Before);

  return { cellsChanged, waterSkippedCells: sharedWaterSkipped.count, bridgeSites, originDeltaM, profiles };
}

/**
 * Point-in-corridor predicate over the same distance math as gradeDem's
 * rasterization (no blend falloff — a hard halfWidthM boundary). Used for
 * tree clearing.
 */
export function makeCorridorMask(ways) {
  if (!Array.isArray(ways)) throw new Error('makeCorridorMask: ways must be an array');
  // Spatial hash over every way's densified points, each point carrying its
  // own way's halfWidthM². The predicate "the nearest dense point of some way
  // is within that way's halfWidthM" is exactly "∃ dense point p of way w with
  // dist(q,p) ≤ halfWidth_w", so a lookup over the 3×3 hash neighbourhood
  // (cell size ≥ max halfWidth) is exhaustive. The previous linear scan was
  // O(total dense points) per query — ~1e5 points × ~4e5 trees at municipality
  // scale; this is O(bucket occupancy).
  let maxHW = 0;
  for (const way of ways) {
    if (!way || !Array.isArray(way.pts) || way.pts.length < 2) {
      throw new Error('makeCorridorMask: way.pts must have at least 2 points');
    }
    if (!(way.halfWidthM > 0)) throw new Error('makeCorridorMask: way.halfWidthM must be > 0');
    if (way.halfWidthM > maxHW) maxHW = way.halfWidthM;
  }
  const CELL = Math.max(maxHW, 1);
  const buckets = new Map(); // "cx_cz" -> [{x, z, hw2}]
  for (const way of ways) {
    const hw2 = way.halfWidthM * way.halfWidthM;
    for (const [px, pz] of densify(way.pts, 2)) {
      const key = `${Math.floor(px / CELL)}_${Math.floor(pz / CELL)}`;
      let arr = buckets.get(key);
      if (!arr) buckets.set(key, (arr = []));
      arr.push({ x: px, z: pz, hw2 });
    }
  }
  return function corridorMask(x, z) {
    const cx = Math.floor(x / CELL);
    const cz = Math.floor(z / CELL);
    for (let gz = cz - 1; gz <= cz + 1; gz++) {
      for (let gx = cx - 1; gx <= cx + 1; gx++) {
        const arr = buckets.get(`${gx}_${gz}`);
        if (!arr) continue;
        for (const p of arr) {
          const dx = p.x - x;
          const dz = p.z - z;
          if (dx * dx + dz * dz <= p.hw2) return true;
        }
      }
    }
    return false;
  };
}
