// scripts/geo/lib/corridorsnap.mjs
//
// Corridor-snap tile-encoder pass (Task 5d, spec §5 part B). The L2 tile
// heightfield samples the graded 2.5 m grid at ~12.5 m tile steps; on a steep
// hillside that resampling loses the flattened road bench and the bilinear
// tile height lands metres ABOVE the road profile — the rendered terrain
// pierces the road surface (metric v2 max 4.035 m before this pass).
//
// This wraps the graded DEM sampler that encodeTile reads (dem.heightAt) so
// that at every tile height-sample vertex inside a road/rail corridor the
// returned height is CLAMPED DOWN to <= profileY − CLEARANCE (terrain sits
// just under the road surface; the runtime road ribbon adds its own +0.10 m
// lift on top). Points in the blend band (corridor edge .. +BLEND_M) clamp
// against a smoothstep-relaxed bound so the terrain rejoins the raw hillside
// smoothly instead of stepping. Clamp-DOWN only — a point already below the
// profile (a graded embankment fill) is left exactly as graded.
//
// ── MIRROR of src/diorama/ksw/geo/groundSampler.ts ─────────────────────────
// That runtime module (makeCorridorGround) is the SAME geometric corridor,
// evaluated at render time so ribbons/cars/flow drape on the profile. This
// bake-time twin must use the identical corridor definition or the two
// disagree at corridor edges and piercing reappears where they differ. The
// spatial-hash query, projectToSegment, interpolateProfile and the smoothstep
// blend are ported from there. Two deliberate differences, both documented:
//   1. Frame: this operates in ABSOLUTE graded metres (report.profiles ys are
//      pre-shift, dem is graded-absolute — bake-world.mjs Step 2c), so no
//      anchor arithmetic; the runtime works in anchor-relative y≈0 space.
//   2. Direction: runtime REPLACES ground with the profile inside the
//      corridor; this only CLAMPS DOWN (min) — embankment fills stay.
// The corridor half-width here is the SAME `halfWidthM` grading actually
// flattened (bake-world.mjs's `ways`), which is `max(width, laneFloor)/2 +
// 1.5` for roads and `(width+2.2)/2 + 2` for rails — the runtime derives an
// equivalent half-width from correctRoadWidths; keep the two in sync if either
// width source changes. BLEND_M mirrors the runtime's 3 m edge band.
//
// Determinism: pure functions over the deterministic graded profiles; the
// query iterates hash buckets in a fixed order and the clamp is a pure
// min — a double encode is byte-identical (proven in the unit test and the
// bake's in-script check).

/** Terrain sits this far under the road surface (m). The runtime ribbon adds
 * its own ≈0.10 m lift on top, so the visible mesh never touches the road. */
export const CLEARANCE_M = 0.05;
/** Edge blend band (m) outside the hard corridor boundary — mirrors
 * groundSampler.ts's BLEND_M. */
const BLEND_M = 3;
/** Spatial-hash cell size (m) — mirrors groundSampler.ts's CELL. */
const CELL = 16;

/** Arc-length projection of (px,pz) onto segment (ax,az)-(bx,bz). Ported from
 * groundSampler.ts projectToSegment. */
export function projectToSegment(px, pz, ax, az, bx, bz) {
  const dx = bx - ax;
  const dz = bz - az;
  const l2 = dx * dx + dz * dz;
  let t = l2 > 0 ? ((px - ax) * dx + (pz - az) * dz) / l2 : 0;
  t = t < 0 ? 0 : t > 1 ? 1 : t;
  const cx = ax + t * dx;
  const cz = az + t * dz;
  const ex = px - cx;
  const ez = pz - cz;
  const dist = Math.hypot(ex, ez);
  const segLen = Math.sqrt(l2);
  return { dist, arc: t * segLen, t };
}

/** Interpolate a {stepM, ys} profile at arc-length `arc`. Ported from
 * groundSampler.ts interpolateProfile (and burial-metric.mjs's twin) — the
 * SAME 5b station convention (ys[k] at min(stepM*k, totalLen)). */
export function interpolateProfile(profile, arc) {
  const { stepM, ys } = profile;
  if (!Array.isArray(ys) || ys.length === 0) {
    throw new Error('interpolateProfile: profile.ys must be a non-empty array');
  }
  if (ys.length === 1) return ys[0];
  if (arc <= 0) return ys[0];
  const maxIdx = ys.length - 1;
  const rawIdx = arc / stepM;
  if (rawIdx >= maxIdx) return ys[maxIdx];
  const i0 = Math.floor(rawIdx);
  const i1 = Math.min(i0 + 1, maxIdx);
  const frac = rawIdx - i0;
  return ys[i0] + (ys[i1] - ys[i0]) * frac;
}

/** smoothstep(edge0, edge1, x) — mirrors groundSampler.ts. */
function smoothstep(edge0, edge1, x) {
  if (edge0 === edge1) return x < edge0 ? 0 : 1;
  const t = Math.min(1, Math.max(0, (x - edge0) / (edge1 - edge0)));
  return t * t * (3 - 2 * t);
}

function cellKey(cx, cz) {
  return `${cx}_${cz}`;
}

function insertSegIntoHash(buckets, seg, extraPad) {
  const pad = seg.halfWidth + BLEND_M + extraPad;
  const minX = Math.min(seg.ax, seg.bx) - pad;
  const maxX = Math.max(seg.ax, seg.bx) + pad;
  const minZ = Math.min(seg.az, seg.bz) - pad;
  const maxZ = Math.max(seg.az, seg.bz) + pad;
  const cx0 = Math.floor(minX / CELL);
  const cx1 = Math.floor(maxX / CELL);
  const cz0 = Math.floor(minZ / CELL);
  const cz1 = Math.floor(maxZ / CELL);
  for (let cx = cx0; cx <= cx1; cx++) {
    for (let cz = cz0; cz <= cz1; cz++) {
      const k = cellKey(cx, cz);
      let arr = buckets.get(k);
      if (!arr) buckets.set(k, (arr = []));
      arr.push(seg);
    }
  }
}

/**
 * Build a corridor-snap height sampler that wraps `base` (the graded DEM
 * sampler encodeTile reads). `ways` are bake-world.mjs's grading ways (each
 * `{ pts, kind, halfWidthM }`), `profiles` is index-aligned (`report.profiles`,
 * ABSOLUTE graded metres, pre-shift). Returns
 * `{ heightAt(x, z, snapMarginM?) }`.
 *
 * `snapMarginM` (per query, default 0) WIDENS the hard-clamp region beyond the
 * physical corridor half-width. It exists because the encoder samples the tile
 * height field on a coarse vertex lattice (12.5 m at L2) and the rendered mesh
 * bilinearly interpolates BETWEEN those vertices: a 7 m-wide corridor centreline
 * usually passes between vertices, so clamping only the vertices physically
 * inside the corridor leaves the interpolated centreline height untouched (the
 * far, unclamped corners dominate the bilinear weight). Bilinear interpolation
 * is a convex combination, so if EVERY corner of the cell a corridor crosses is
 * clamped to ≤ profileY − CLEARANCE, the interpolated value there is too. The
 * caller passes `snapMarginM = √2 · vertexStep` (the cell diagonal) so every
 * vertex whose cell the corridor can cross is clamped. Those extra vertices
 * clamp to THEIR OWN projected profileY (a near-flat local profile → the same
 * bench height), so the terrain is lowered by ~one tile cell around the road on
 * steep slopes — the accepted cost of the coarse tile lattice (spec §5 part B).
 * At L0/L1 the vertex step is large (100 m / 1000 m), which would gouge huge
 * swaths, so the caller passes a SMALL margin there (see bake-world wiring):
 * coarse LODs are viewed from far away where sub-metre piercing is invisible.
 *
 * Every way MUST have a profile (bake-world's grading produces one per way) —
 * a missing profile is a hard error (no fallback), listing offenders.
 */
export function makeCorridorSnapSampler(base, ways, profiles, maxSnapMarginM = 0) {
  if (!base || typeof base.heightAt !== 'function') {
    throw new Error('makeCorridorSnapSampler: base must expose heightAt(x,z)');
  }
  if (!Array.isArray(ways) || !Array.isArray(profiles) || ways.length !== profiles.length) {
    throw new Error('makeCorridorSnapSampler: ways and profiles must be index-aligned arrays');
  }
  const offenders = [];
  for (let i = 0; i < ways.length; i++) {
    if (!profiles[i] || !Array.isArray(profiles[i].ys) || profiles[i].ys.length === 0) {
      offenders.push(`way[${i}] kind=${ways[i]?.kind}`);
    }
    if (!(ways[i]?.halfWidthM > 0)) offenders.push(`way[${i}] halfWidthM missing`);
  }
  if (offenders.length > 0) {
    throw new Error(
      `makeCorridorSnapSampler: ${offenders.length} way(s) missing a graded profile/halfWidth — ` +
        `bake wiring bug (profiles must be index-aligned with grading ways). ` +
        `Offenders (first 20): ${offenders.slice(0, 20).join(', ')}`,
    );
  }

  const buckets = new Map();
  for (let i = 0; i < ways.length; i++) {
    const pts = ways[i].pts;
    const profile = profiles[i];
    const halfWidth = ways[i].halfWidthM;
    if (!Array.isArray(pts) || pts.length < 2) continue;
    let arc = 0;
    for (let s = 1; s < pts.length; s++) {
      const [ax, az] = pts[s - 1];
      const [bx, bz] = pts[s];
      const len = Math.hypot(bx - ax, bz - az);
      insertSegIntoHash(buckets, { ax, az, bx, bz, arcStart: arc, profile, halfWidth }, maxSnapMarginM);
      arc += len;
    }
  }

  const heightAt = (x, z, snapMarginM = 0) => {
    const margin = snapMarginM > maxSnapMarginM ? maxSnapMarginM : snapMarginM;
    const raw = base.heightAt(x, z);
    const cx = Math.floor(x / CELL);
    const cz = Math.floor(z / CELL);
    let best = null;
    for (let ncx = cx - 1; ncx <= cx + 1; ncx++) {
      for (let ncz = cz - 1; ncz <= cz + 1; ncz++) {
        const bucket = buckets.get(cellKey(ncx, ncz));
        if (!bucket) continue;
        for (const seg of bucket) {
          // Hard-clamp radius = physical halfWidth + the per-query snap margin
          // (cell-diagonal at this tile level). Beyond that, the BLEND_M band
          // relaxes toward raw terrain.
          const hard = seg.halfWidth + margin;
          const proj = projectToSegment(x, z, seg.ax, seg.az, seg.bx, seg.bz);
          if (proj.dist > hard + BLEND_M) continue;
          if (best === null || proj.dist < best.dist) {
            const profileY = interpolateProfile(seg.profile, seg.arcStart + proj.arc);
            best = { dist: proj.dist, hard, profileY };
          }
        }
      }
    }
    if (best === null) return raw;

    // Bound the terrain to sit just under the road surface. Inside the hard
    // (margin-widened) corridor the bound is profileY − CLEARANCE; across the
    // blend band it relaxes (smoothstep) toward the raw hillside so the terrain
    // rejoins the slope without a step. Clamp-DOWN only: a point already below
    // the bound (embankment fill) is untouched.
    const hardBound = best.profileY - CLEARANCE_M;
    let bound;
    if (best.dist <= best.hard) {
      bound = hardBound;
    } else {
      const mix = smoothstep(best.hard, best.hard + BLEND_M, best.dist);
      // mix 0 at the hard edge (full hardBound) → 1 at hard+BLEND (raw).
      bound = hardBound + (raw - hardBound) * mix;
    }
    return raw < bound ? raw : bound;
  };

  return { heightAt };
}
