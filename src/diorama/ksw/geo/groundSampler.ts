// src/diorama/ksw/geo/groundSampler.ts
//
// Corridor-aware ground sampler (spec §5 amendment, Task 5c): roads own their
// surface height. Task 5b baked a smoothed longitudinal profile into every
// road/rail way in data/winterthur/roads.json (`profile: {stepM, ys}`,
// heights already relative to the shared anchor — see
// .superpowers/sdd/task-5b-report.md). The L2 tile heightfield samples
// ~12.5 m and cannot represent a ~6 m carriageway bench, so the runtime
// builds ONE ground sampler that:
//   - inside a road/rail corridor, returns the profile height interpolated
//     along the way's arc length at the projected station;
//   - in a blend band at the corridor edge, smoothstep-mixes profile height
//     with the tile ground height;
//   - outside any corridor, falls through to the tile ground height.
// Every consumer (ribbons, cars, flow markers, lamps if ground-sampled)
// takes the SAME sampler, so they all agree on "ground under infrastructure"
// — this is data routing, not a fallback.
//
// Determinism: pure functions over the baked, already-deterministic profile
// data; construction is O(n) (one spatial-hash insert per corridor segment),
// query is O(1) expected (one cell bucket + its 3x3 neighborhood).
//
// ── MIRROR: scripts/geo/lib/corridorsnap.mjs ───────────────────────────────
// A bake-time twin (Task 5d) uses the SAME corridor geometry — spatial hash,
// projectToSegment, interpolateProfile, BLEND_M, CELL — to CLAMP the tile
// height field down to profileY − 0.05 m inside corridors so the rendered
// terrain never pierces the road profile this sampler serves. The two MUST
// stay geometrically identical (half-width, blend band, arc convention) or
// terrain piercing reappears where they disagree. If the corridor definition
// (half-width source, BLEND_M) changes here, change corridorsnap.mjs too.
import { corridorWidths, type TrafficNetDoc } from '../../traffic/corridorWidths';
import trafficNetJson from '../../../data/winterthur/trafficnet.json';
import type { RoadPath, RoadProfile } from './geoData';

export type GroundYAt = (x: number, z: number) => number;

/** Half-width margin added on top of the corridor/OSM road width to define
 * the hard corridor boundary (corridor width source: corridorWidths — the
 * lane-floor is terrain-corridor-only, never the render ribbon, per #134). */
const ROAD_HALFWIDTH_MARGIN_M = 1.5;
/** Rail bed halfWidth: (width + 2.2)/2 + 2, mirroring roads.ts's railBed
 * layer (`p.width + 2.2`) plus a margin so the sampler's corridor is a bit
 * wider than the rendered ballast bed. */
const RAIL_BED_PAD_M = 2.2;
const RAIL_HALFWIDTH_MARGIN_M = 2;
/** Edge blend band (m) outside the hard corridor boundary where the sampler
 * smoothstep-mixes profile height toward tile ground height. */
const BLEND_M = 3;
/** Spatial hash cell size (m), matching roadWidths.ts's coarse hash. */
const CELL = 16;

/** One indexed corridor segment: its two endpoints, precomputed length, and
 * a reference to the owning way's profile + cumulative arc length at the
 * segment start (so `profileAtArc` can resolve a global arc-length station
 * from a local projection distance along this segment). */
interface Seg {
  ax: number;
  az: number;
  bx: number;
  bz: number;
  len: number;
  arcStart: number;
  profile: RoadProfile;
  halfWidth: number;
}

/** Squared/plain distance + arc-length projection of point (px,pz) onto
 * segment (ax,az)-(bx,bz). `arc` is the distance along the segment from
 * (ax,az) to the projected (clamped) point; `dist` is the perpendicular (or
 * clamped-endpoint) distance from the point to that projection. */
export function projectToSegment(
  px: number,
  pz: number,
  ax: number,
  az: number,
  bx: number,
  bz: number,
): { dist: number; arc: number; t: number } {
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

/** Interpolate a road/rail profile's height at an arbitrary arc-length
 * position along the way. `ys[k]` is the height at station
 * `min(stepM * k, totalLen)` (Task 5b convention: the last two entries
 * coincide when totalLen is a multiple of stepM). Clamps outside the
 * profile's covered range to the nearest end station. */
export function interpolateProfile(profile: RoadProfile, arc: number): number {
  const { stepM, ys } = profile;
  if (ys.length === 0) throw new Error('interpolateProfile: profile.ys must not be empty');
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

function smoothstep(edge0: number, edge1: number, x: number): number {
  if (edge0 === edge1) return x < edge0 ? 0 : 1;
  const t = Math.min(1, Math.max(0, (x - edge0) / (edge1 - edge0)));
  return t * t * (3 - 2 * t);
}

function cellKey(cx: number, cz: number): string {
  return `${cx}_${cz}`;
}

function insertSegIntoHash(buckets: Map<string, Seg[]>, seg: Seg): void {
  const pad = seg.halfWidth + BLEND_M;
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
 * Build ONE corridor-aware ground sampler from the baked road/rail ways.
 * Every way MUST carry a `profile` (Task 5b's hard gate) — a way without one
 * is a construction-time hard error (no silent tileGround fallback; project
 * rule).
 */
export function makeCorridorGround(
  roads: RoadPath[],
  rails: RoadPath[],
  tileGround: GroundYAt,
): GroundYAt {
  const offenders: string[] = [];
  for (let i = 0; i < roads.length; i++) {
    if (!roads[i].profile) offenders.push(`road[${i}] class=${roads[i].class}`);
  }
  for (let i = 0; i < rails.length; i++) {
    if (!rails[i].profile) offenders.push(`rail[${i}] class=${rails[i].class}`);
  }
  if (offenders.length > 0) {
    throw new Error(
      `makeCorridorGround: ${offenders.length} way(s) missing a baked profile — run geo:bake-world → geo:bake ` +
        `to regenerate roads.json with profiles. Offenders (first 20): ${offenders.slice(0, 20).join(', ')}`,
    );
  }

  // Road halfWidth = max(OSM width, traffic-lane-floored corridor width)/2
  // + 1.5 — the SAME corridorWidths the grading bake used (gradewidths.mjs
  // twin), so the sampler's corridor matches the graded bench exactly.
  const correctedRoadWidths = corridorWidths(roads, trafficNetJson as unknown as TrafficNetDoc);

  const buckets = new Map<string, Seg[]>();

  const addWay = (path: RoadPath, halfWidth: number): void => {
    const profile = path.profile!;
    const pts = path.pts;
    if (pts.length < 2) return;
    let arc = 0;
    for (let i = 1; i < pts.length; i++) {
      const [ax, az] = pts[i - 1];
      const [bx, bz] = pts[i];
      const len = Math.hypot(bx - ax, bz - az);
      insertSegIntoHash(buckets, { ax, az, bx, bz, len, arcStart: arc, profile, halfWidth });
      arc += len;
    }
  };

  for (let i = 0; i < roads.length; i++) {
    const width = Math.max(roads[i].width, correctedRoadWidths[i] ?? roads[i].width);
    addWay(roads[i], width / 2 + ROAD_HALFWIDTH_MARGIN_M);
  }
  for (let i = 0; i < rails.length; i++) {
    addWay(rails[i], (rails[i].width + RAIL_BED_PAD_M) / 2 + RAIL_HALFWIDTH_MARGIN_M);
  }

  return (x: number, z: number): number => {
    const cx = Math.floor(x / CELL);
    const cz = Math.floor(z / CELL);
    let best: { dist: number; halfWidth: number; profileY: number } | null = null;

    for (let ncx = cx - 1; ncx <= cx + 1; ncx++) {
      for (let ncz = cz - 1; ncz <= cz + 1; ncz++) {
        const bucket = buckets.get(cellKey(ncx, ncz));
        if (!bucket) continue;
        for (const seg of bucket) {
          const proj = projectToSegment(x, z, seg.ax, seg.az, seg.bx, seg.bz);
          if (proj.dist > seg.halfWidth + BLEND_M) continue;
          if (best === null || proj.dist < best.dist) {
            const profileY = interpolateProfile(seg.profile, seg.arcStart + proj.arc);
            best = { dist: proj.dist, halfWidth: seg.halfWidth, profileY };
          }
        }
      }
    }

    if (best === null) return tileGround(x, z);
    if (best.dist <= best.halfWidth) return best.profileY;

    const tileY = tileGround(x, z);
    // Smoothstep mix: 0 at the hard corridor boundary (full profile) to 1 at
    // halfWidth + BLEND_M (full tile ground).
    const mix = smoothstep(best.halfWidth, best.halfWidth + BLEND_M, best.dist);
    return best.profileY + (tileY - best.profileY) * mix;
  };
}
