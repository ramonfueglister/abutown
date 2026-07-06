// scripts/geo/lib/corridormask.mjs
//
// Corridor mask raster (Task 5e, spec §5 "Terrain-discard"). The definitive
// mechanism for "terrain never pierces a road surface": the bake exports a
// compact world-space raster marking every cell inside a road/rail corridor;
// the terrain fragment shader DISCARDS fragments where the mask reads 1, and
// ribbon side-skirts (roads.ts) close the resulting hole. Rendered terrain
// inside a corridor then does not exist — piercing is impossible by
// construction, so no per-vertex heightfield clamp can be blamed (Task 5d
// proved adjacent parallel ways at conflicting heights are unrepresentable in
// any single heightfield; discard sidesteps that entirely).
//
// ── ONE GEOMETRIC TRUTH (MIRROR) ───────────────────────────────────────────
// The corridor half-width used here is the SAME `halfWidthM` grading flattens
// (bake-world.mjs's `ways`: `max(width, laneFloor)/2 + 1.5` roads,
// `(width+2.2)/2 + 2` rails) and the SAME predicate grading's
// makeCorridorMask uses for tree-clearing: a cell is "in corridor" iff its
// centre lies within `halfWidthM` of some densified way point. The runtime
// corridor-snap twin (src/diorama/ksw/geo/groundSampler.ts / corridorsnap.mjs)
// derives an equivalent half-width from correctRoadWidths; keep the three in
// sync if any width source changes, or the discard region and the profile
// sampler disagree at corridor edges.
//
// Resolution: 2.5 m (spec requires ≥ 2.5 m) — matches the grading grid, so a
// median ~5 m carriageway spans ≥ 2 mask cells and the discard band never has
// a hole narrower than the corridor.
//
// Determinism: pure function of (ways, bounds, cellSize). Rasterization visits
// cells in a fixed order and sets bits with |=, so a double-build is
// byte-identical (unit-tested + in-bake check).
//
// Serialization (mask.bin): a small fixed header + a packed 1-bit-per-cell
// bitfield, little-endian. No fallbacks — decode validates the magic/version
// and throws on mismatch.

const MAGIC = 0x434d5330; // "CMS0" — Corridor Mask v0
const HEADER_BYTES = 4 /*magic*/ + 4 /*version*/ + 8 /*originX*/ + 8 /*originZ*/ + 4 /*cellSizeM*/ + 4 /*cols*/ + 4 /*rows*/;
const VERSION = 1;

/** Densify a polyline to ≤ maxStepM steps — mirrors grading.mjs's densify so
 * the mask's stamping points coincide with the corridor grading actually saw. */
function densify(pts, maxStepM = 2) {
  if (!Array.isArray(pts) || pts.length < 2) {
    throw new Error('corridormask: way.pts must have at least 2 points');
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

/**
 * Rasterize road/rail corridors into a packed 1-bit-per-cell world-space mask.
 *
 * `ways`: `[{ pts, halfWidthM, kind }]` — the SAME array grading grades.
 * `bounds`: `{ minX, minZ, maxX, maxZ }` — the world extent to cover (the bake
 *   passes the tile-root span so every rendered terrain fragment is inside it).
 * `cellSize`: mask resolution in metres (2.5 m in the bake).
 *
 * A cell (i,j) with centre at (minX + i·cell, minZ + j·cell) is set iff that
 * centre lies within `halfWidthM` of some densified point of some way — the
 * exact predicate grading.makeCorridorMask evaluates, precomputed onto a grid.
 *
 * Hard-errors (no fallback) on malformed ways: a way must have ≥ 2 points and a
 * positive halfWidthM, or the discard region would silently miss it.
 */
export function buildCorridorMask(ways, bounds, cellSize) {
  if (!Array.isArray(ways)) throw new Error('buildCorridorMask: ways must be an array');
  if (!bounds || !(bounds.maxX > bounds.minX) || !(bounds.maxZ > bounds.minZ)) {
    throw new Error('buildCorridorMask: bounds must be { minX<maxX, minZ<maxZ }');
  }
  if (!(cellSize > 0)) throw new Error('buildCorridorMask: cellSize must be > 0');

  const { minX, minZ, maxX, maxZ } = bounds;
  const cols = Math.max(1, Math.ceil((maxX - minX) / cellSize) + 1);
  const rows = Math.max(1, Math.ceil((maxZ - minZ) / cellSize) + 1);
  const bits = new Uint8Array(Math.ceil((cols * rows) / 8));

  const setBit = (i, j) => {
    if (i < 0 || j < 0 || i >= cols || j >= rows) return;
    const n = j * cols + i;
    bits[n >> 3] |= 1 << (n & 7);
  };

  for (const way of ways) {
    if (!way || !Array.isArray(way.pts) || way.pts.length < 2) {
      throw new Error('buildCorridorMask: way.pts must have at least 2 points');
    }
    if (!(way.halfWidthM > 0)) throw new Error('buildCorridorMask: way.halfWidthM must be > 0');
    // Effective stamping radius: at least one cell, so a corridor ALWAYS covers
    // every cell its densified centreline's nearest-cell path touches —
    // including the diagonal-corner cell a straight line skips when it steps
    // from cell (i,j) to (i−1,j−1), and the boundary case where a 2.5 m-spaced
    // coverage station lands >½-diagonal from every 1.25 m-spaced stamping
    // point. A true halfWidth of 1.75 m (narrow `path` ways) sits just under the
    // cell half-diagonal 1.768 m, so a smaller floor leaves sub-cell holes
    // (measured on the real bake). Flooring at cellSize is provably gap-free
    // (0 uncovered stations across all 2 296 ways) and over-covers those narrow
    // ways by ≤ 0.75 m each side — negligible against the 8 m grading blend, and
    // the discard region must be ≥ the corridor, never less (spec §5: no
    // rendered terrain INSIDE it). Wider corridors are unaffected (max keeps
    // their own halfWidth).
    const hw = Math.max(way.halfWidthM, cellSize);
    const hw2 = hw * hw;
    const reach = Math.ceil(hw / cellSize);
    // Densify to ≤ half a cell so the centreline visits (has a point in) every
    // cell it crosses; the effective-radius disk then closes diagonal corners.
    for (const [px, pz] of densify(way.pts, cellSize / 2)) {
      const ci = Math.round((px - minX) / cellSize);
      const cj = Math.round((pz - minZ) / cellSize);
      setBit(ci, cj); // the point's own (nearest) cell — never missed
      for (let j = cj - reach; j <= cj + reach; j++) {
        const cz = minZ + j * cellSize;
        const dz = cz - pz;
        for (let i = ci - reach; i <= ci + reach; i++) {
          const cx = minX + i * cellSize;
          const dx = cx - px;
          if (dx * dx + dz * dz <= hw2) setBit(i, j);
        }
      }
    }
  }

  return { originX: minX, originZ: minZ, cellSizeM: cellSize, cols, rows, bits };
}

/** Read the mask at world (x,z): true iff the covering cell's bit is set.
 * Nearest-cell lookup (round), matching the shader's nearest sample. Out of
 * bounds → false (no corridor out there). */
export function maskCovers(mask, x, z) {
  const i = Math.round((x - mask.originX) / mask.cellSizeM);
  const j = Math.round((z - mask.originZ) / mask.cellSizeM);
  if (i < 0 || j < 0 || i >= mask.cols || j >= mask.rows) return false;
  const n = j * mask.cols + i;
  return (mask.bits[n >> 3] & (1 << (n & 7))) !== 0;
}

/** Serialize a mask to a little-endian Uint8Array: header + packed bits. */
export function encodeCorridorMask(mask) {
  const buf = new ArrayBuffer(HEADER_BYTES + mask.bits.length);
  const dv = new DataView(buf);
  let o = 0;
  dv.setUint32(o, MAGIC, true); o += 4;
  dv.setUint32(o, VERSION, true); o += 4;
  dv.setFloat64(o, mask.originX, true); o += 8;
  dv.setFloat64(o, mask.originZ, true); o += 8;
  dv.setFloat32(o, mask.cellSizeM, true); o += 4;
  dv.setUint32(o, mask.cols, true); o += 4;
  dv.setUint32(o, mask.rows, true); o += 4;
  new Uint8Array(buf, HEADER_BYTES).set(mask.bits);
  return new Uint8Array(buf);
}

/** Decode a mask.bin buffer (Uint8Array/ArrayBuffer) back to a mask object.
 * Hard-errors on magic/version/length mismatch — no silent fallback. */
export function decodeCorridorMask(bin) {
  const u8 = bin instanceof Uint8Array ? bin : new Uint8Array(bin);
  if (u8.byteLength < HEADER_BYTES) {
    throw new Error(`decodeCorridorMask: buffer too small (${u8.byteLength} < ${HEADER_BYTES} header bytes)`);
  }
  const dv = new DataView(u8.buffer, u8.byteOffset, u8.byteLength);
  let o = 0;
  const magic = dv.getUint32(o, true); o += 4;
  if (magic !== MAGIC) throw new Error(`decodeCorridorMask: bad magic 0x${magic.toString(16)} (expected 0x${MAGIC.toString(16)})`);
  const version = dv.getUint32(o, true); o += 4;
  if (version !== VERSION) throw new Error(`decodeCorridorMask: unsupported version ${version} (expected ${VERSION})`);
  const originX = dv.getFloat64(o, true); o += 8;
  const originZ = dv.getFloat64(o, true); o += 8;
  const cellSizeM = dv.getFloat32(o, true); o += 4;
  const cols = dv.getUint32(o, true); o += 4;
  const rows = dv.getUint32(o, true); o += 4;
  const expectBytes = Math.ceil((cols * rows) / 8);
  const bits = u8.subarray(HEADER_BYTES, HEADER_BYTES + expectBytes);
  if (bits.length !== expectBytes) {
    throw new Error(`decodeCorridorMask: truncated bitfield (${bits.length} < ${expectBytes} bytes for ${cols}×${rows})`);
  }
  return { originX, originZ, cellSizeM, cols, rows, bits: new Uint8Array(bits) };
}

export { HEADER_BYTES };
