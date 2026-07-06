#!/usr/bin/env node
// scripts/geo/burial-metric.mjs
//
// Acceptance instrument for the terrain-grading pass (spec §9): quantifies
// how far a road's planar ribbon edges sit off the actual terrain. The
// runtime road mesh (miterStrip, §5) is planar across its width, so any
// real cross-slope under the ribbon reads as "burial" — the ribbon edge is
// buried into (or floats above) the ground.
//
// Pure core (`burialStats`) is exported for tests. Running this file
// directly (`node scripts/geo/burial-metric.mjs`) decodes the REAL baked
// L2 tiles + roads.json + trafficnet.json and prints the stats table —
// the §9 acceptance check: max < 0.3 m, p99 < 0.15 m.
//
// No fallbacks: malformed inputs throw rather than silently defaulting.
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { fromBinary } from '@bufbuild/protobuf';
import { WorldManifestSchema, WorldTileSchema } from './proto/world_pb.js';
import { laneFloorWidths } from './lib/gradewidths.mjs';
import { decodeCorridorMask, maskCovers } from './lib/corridormask.mjs';

/** Densify a polyline to ≤ 2 m steps, mirroring grading.mjs's densify so
 * station spacing along a road is consistent between grading and metric. */
function densify(pts, maxStepM = 2) {
  if (!Array.isArray(pts) || pts.length < 2) {
    throw new Error('burial-metric: road.pts must have at least 2 points');
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

function arcLengths(pts) {
  const arc = [0];
  for (let i = 1; i < pts.length; i++) {
    const [x0, z0] = pts[i - 1];
    const [x1, z1] = pts[i];
    arc.push(arc[i - 1] + Math.hypot(x1 - x0, z1 - z0));
  }
  return arc;
}

/**
 * Resample a densified polyline at fixed arc-length stations spaced
 * `stepM` apart (0, stepM, 2*stepM, ... up to totalLen), returning
 * `{x, z, tx, tz}` per station (position + unit tangent). Deterministic —
 * always includes station 0; the last station is the final arc length
 * (may be < stepM from the previous one for a short remainder).
 */
function stationsAlong(dense, stepM) {
  const arc = arcLengths(dense);
  const total = arc[arc.length - 1];
  if (total <= 0) {
    throw new Error('burial-metric: road has zero length');
  }
  const targets = [];
  for (let d = 0; d <= total; d += stepM) targets.push(d);
  if (targets[targets.length - 1] < total) targets.push(total);

  const stations = [];
  let seg = 0;
  for (const d of targets) {
    while (seg < arc.length - 2 && arc[seg + 1] < d) seg++;
    const segLen = arc[seg + 1] - arc[seg];
    const t = segLen > 0 ? (d - arc[seg]) / segLen : 0;
    const [x0, z0] = dense[seg];
    const [x1, z1] = dense[seg + 1];
    const x = x0 + (x1 - x0) * t;
    const z = z0 + (z1 - z0) * t;
    let tx = x1 - x0;
    let tz = z1 - z0;
    const tl = Math.hypot(tx, tz) || 1;
    tx /= tl;
    tz /= tl;
    stations.push({ x, z, tx, tz });
  }
  return stations;
}

/**
 * Cross-section burial metric (spec §9): for every road, walk its
 * centreline at `stepM` stations; at each station sample the ground at
 * both ribbon edges (perpendicular offset ±width/2) and compare to the
 * centreline ground height. Deviation = |heightAt(edge) − heightAt(centre)|,
 * one value per edge per station (two per station).
 *
 * @param {{class: string, width: number, pts: number[][]}[]} roads
 * @param {number[]} widths corrected corridor widths, one per road (same
 *   floor `gradeDem`/the bake wiring apply — see gradewidths.mjs)
 * @param {(x: number, z: number) => number} heightAt bilinear ground sampler
 * @param {number} [stepM] station spacing along each centreline (default 10 m, per §9)
 * @returns {{maxM: number, p99M: number, pctOver30cm: number, offenders: {x:number,z:number,devM:number,class:string}[], sampleCount: number}}
 */
export function burialStats(roads, widths, heightAt, stepM = 10) {
  if (!Array.isArray(roads)) throw new Error('burialStats: roads must be an array');
  if (!Array.isArray(widths) || widths.length !== roads.length) {
    throw new Error('burialStats: widths must be an array with one entry per road');
  }
  if (typeof heightAt !== 'function') throw new Error('burialStats: heightAt must be a function');
  if (!(stepM > 0)) throw new Error('burialStats: stepM must be > 0');

  const deviations = [];

  for (let r = 0; r < roads.length; r++) {
    const road = roads[r];
    const width = widths[r];
    if (!(width > 0)) throw new Error(`burialStats: widths[${r}] must be > 0`);
    if (!Array.isArray(road.pts) || road.pts.length < 2) continue;

    const halfWidth = width / 2;
    const dense = densify(road.pts, 2);
    const stations = stationsAlong(dense, stepM);

    for (const st of stations) {
      // Perpendicular (left-hand normal) to the tangent, in the x-z plane.
      const nx = -st.tz;
      const nz = st.tx;
      const centreH = heightAt(st.x, st.z);

      const leftX = st.x + nx * halfWidth;
      const leftZ = st.z + nz * halfWidth;
      const rightX = st.x - nx * halfWidth;
      const rightZ = st.z - nz * halfWidth;

      const leftDev = Math.abs(heightAt(leftX, leftZ) - centreH);
      const rightDev = Math.abs(heightAt(rightX, rightZ) - centreH);

      deviations.push({ x: leftX, z: leftZ, devM: leftDev, class: road.class });
      deviations.push({ x: rightX, z: rightZ, devM: rightDev, class: road.class });
    }
  }

  if (deviations.length === 0) {
    return { maxM: 0, p99M: 0, pctOver30cm: 0, offenders: [], sampleCount: 0 };
  }

  // Sort ascending by deviation for percentile lookup; keep a separate
  // descending view for the offenders list so both reads are O(n log n)
  // once, not re-sorted per use.
  const sorted = deviations.slice().sort((a, b) => a.devM - b.devM);
  const n = sorted.length;
  const maxM = sorted[n - 1].devM;
  // p99: nearest-rank method, deterministic index into the ascending array.
  const p99Idx = Math.min(n - 1, Math.ceil(0.99 * n) - 1);
  const p99M = sorted[p99Idx].devM;
  const overCount = deviations.reduce((acc, d) => acc + (d.devM >= 0.3 ? 1 : 0), 0);
  const pctOver30cm = (overCount / n) * 100;

  // Offenders: the worst stations that actually breach the 30 cm budget
  // (spec §9's own threshold), capped at 10 and sorted worst-first. A
  // passing metric (nothing over budget) reports an empty list rather than
  // padding it with sub-threshold deviations.
  const offenders = sorted
    .slice(n - 10)
    .reverse()
    .filter((d) => d.devM >= 0.3)
    .map(({ x, z, devM, class: cls }) => ({ x, z, devM, class: cls }));

  return { maxM, p99M, pctOver30cm, offenders, sampleCount: n };
}

/** Interpolate a Task-5b baked profile ({stepM, ys}) at an arbitrary
 * arc-length station. Mirrors src/diorama/ksw/geo/groundSampler.ts's
 * `interpolateProfile` (kept in sync by hand — plain .mjs cannot import the
 * TS module during the bake/CLI). Clamps outside the covered range to the
 * nearest end station. */
function interpolateProfile(profile, arc) {
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

/**
 * §9 metric v2 (spec amendment): terrain poke-through. For every road/rail,
 * walk 10 m stations along the centreline; at each station compare the tile
 * heightfield (`heightAt`, absolute or already-shifted — caller's choice, as
 * long as it's consistent with the profile's frame) against the baked
 * longitudinal profile height at that arc-length station. Poke-through =
 * `max(0, tileY - profileY)` — only terrain sitting ABOVE the road surface
 * counts as piercing it; terrain sitting below (normal embankment/cut) is not
 * a defect and is excluded from the max/p99 (clamped to 0 so it never drags
 * the metric down artificially, and never counts as an offender).
 *
 * Budgets (spec §5 amendment): p99 ≤ 0.05 m, max < 0.10 m.
 *
 * @param {{class: string, pts: number[][], profile?: {stepM:number, ys:number[]}}[]} roads
 * @param {(x:number, z:number) => number} heightAt
 * @param {number} [stepM]
 * @returns {{maxM:number, p99M:number, offenders: {x:number,z:number,devM:number,class:string}[], sampleCount:number}}
 */
export function burialStatsV2(roads, heightAt, stepM = 10) {
  if (!Array.isArray(roads)) throw new Error('burialStatsV2: roads must be an array');
  if (typeof heightAt !== 'function') throw new Error('burialStatsV2: heightAt must be a function');
  if (!(stepM > 0)) throw new Error('burialStatsV2: stepM must be > 0');

  const missing = [];
  for (let i = 0; i < roads.length; i++) {
    if (!roads[i].profile) missing.push(i);
  }
  if (missing.length > 0) {
    throw new Error(
      `burialStatsV2: ${missing.length} road(s) missing a baked profile (indices: ${missing.slice(0, 20).join(', ')}) — run geo:bake-world → geo:attach-profiles first`,
    );
  }

  const deviations = [];

  for (const road of roads) {
    if (!Array.isArray(road.pts) || road.pts.length < 2) continue;
    const dense = densify(road.pts, 2);
    const stations = stationsAlong(dense, stepM);
    const arc = arcLengths(dense);
    const total = arc[arc.length - 1];

    // stationsAlong resamples densified pts at fixed stepM arc targets; we
    // need the SAME arc-length values to index into the profile, so recompute
    // them the same way stationsAlong does (0, stepM, 2*stepM, ..., total).
    const targets = [];
    for (let d = 0; d <= total; d += stepM) targets.push(d);
    if (targets[targets.length - 1] < total) targets.push(total);

    for (let i = 0; i < stations.length; i++) {
      const st = stations[i];
      const stationArc = targets[i];
      const tileY = heightAt(st.x, st.z);
      const profileY = interpolateProfile(road.profile, stationArc);
      const devM = Math.max(0, tileY - profileY);
      deviations.push({ x: st.x, z: st.z, devM, class: road.class });
    }
  }

  if (deviations.length === 0) {
    return { maxM: 0, p99M: 0, offenders: [], sampleCount: 0 };
  }

  const sorted = deviations.slice().sort((a, b) => a.devM - b.devM);
  const n = sorted.length;
  const maxM = sorted[n - 1].devM;
  const p99Idx = Math.min(n - 1, Math.ceil(0.99 * n) - 1);
  const p99M = sorted[p99Idx].devM;

  const BUDGET_MAX = 0.10;
  const offenders = sorted
    .slice(n - 10)
    .reverse()
    .filter((d) => d.devM >= BUDGET_MAX)
    .map(({ x, z, devM, class: cls }) => ({ x, z, devM, class: cls }));

  return { maxM, p99M, offenders, sampleCount: n };
}

/**
 * §9 metric v3 (spec §5 "Terrain-discard"), NON-VACUOUS rewrite (Finding 1b).
 *
 * Rendered truth via the corridor discard mask. Terrain fragments inside a
 * corridor are DISCARDED by the shader, so they cannot pierce a road surface.
 * The mask now stamps only the RIBBON footprint (Finding 1a), so the graded
 * SHOULDER (mask edge → grading edge) still RENDERS — and THAT annulus is where
 * poke-through must be measured. Sampling only on-centreline (as the old v3 did)
 * is vacuous: the centreline is guaranteed inside the mask → discarded → empty
 * measured set. Instead, at every centreline station we use the station normal
 * to sample the SHOULDER ANNULUS on both sides.
 *
 * Three reported parts:
 *   (a) coveragePct = 100 % — every centreline station falls in a set mask cell
 *       (the ribbon footprint is fully discarded; an uncovered station would be
 *       rendered terrain under the ribbon that could pierce).
 *   (b) shoulder-annulus poke-through: for offsets from `maskHW + 0.1 m` to
 *       `gradeHW + blendM` in 0.5 m steps, both sides, over the annulus samples
 *       that fall OUTSIDE the mask (rendered terrain), `max(0, tileY − profileY)`
 *       must satisfy p99 ≤ 0.05 m, max < 0.10 m (v2 budgets).
 *   (c) skirt-reach: within the annulus INSIDE the mask edge but outside the
 *       ribbon edge (ribbon edge → mask edge — normally empty since the mask
 *       stamps the ribbon, so any residual gap), the required skirt drop
 *       `max(profileY − tileY)` must be ≤ the skirt drop (SKIRT_DROP_M = 1.5 m)
 *       or a skirt foot would float above the ground it must hide. Reported as
 *       `skirtReachM` (the real max) with a `skirtReachPass` flag.
 *
 * @param {{class:string, pts:number[][], profile?:{stepM:number,ys:number[]}}[]} roads
 * @param {(x:number,z:number)=>number} heightAt tile heightfield (profile frame)
 * @param {(x:number,z:number)=>boolean} covers corridor-mask reader
 * @param {{stepM?:number, maskHalfWidths:number[], gradeHalfWidths:number[], blendM?:number, skirtDropM?:number}} opts
 * @returns {{coveragePct:number, sampleCount:number, coveredCount:number, outsideCount:number, maxM:number, p99M:number, offenders:{x:number,z:number,devM:number,class:string}[], skirtReachM:number, skirtReachPass:boolean}}
 */
export function burialStatsV3(roads, heightAt, covers, opts) {
  if (!Array.isArray(roads)) throw new Error('burialStatsV3: roads must be an array');
  if (typeof heightAt !== 'function') throw new Error('burialStatsV3: heightAt must be a function');
  if (typeof covers !== 'function') throw new Error('burialStatsV3: covers must be a function');
  if (!opts || typeof opts !== 'object') {
    throw new Error('burialStatsV3: opts must be { maskHalfWidths, gradeHalfWidths, stepM? } — v3 samples the shoulder annulus (Finding 1b), not the centreline');
  }
  const stepM = opts.stepM ?? 10;
  const blendM = opts.blendM ?? 3;
  const skirtDropM = opts.skirtDropM ?? 1.5;
  const { maskHalfWidths, gradeHalfWidths } = opts;
  if (!(stepM > 0)) throw new Error('burialStatsV3: stepM must be > 0');
  if (!Array.isArray(maskHalfWidths) || maskHalfWidths.length !== roads.length) {
    throw new Error('burialStatsV3: opts.maskHalfWidths must be an array with one entry per road');
  }
  if (!Array.isArray(gradeHalfWidths) || gradeHalfWidths.length !== roads.length) {
    throw new Error('burialStatsV3: opts.gradeHalfWidths must be an array with one entry per road');
  }

  const missing = [];
  for (let i = 0; i < roads.length; i++) if (!roads[i].profile) missing.push(i);
  if (missing.length > 0) {
    throw new Error(
      `burialStatsV3: ${missing.length} road(s) missing a baked profile (indices: ${missing.slice(0, 20).join(', ')}) — run geo:bake-world → geo:attach-profiles first`,
    );
  }

  const ANNULUS_STEP_M = 0.5;
  let sampleCount = 0; // centreline stations (coverage denominator)
  let coveredCount = 0;
  const outside = []; // annulus samples OUTSIDE the mask — where terrain renders
  let skirtReachM = 0; // max(profileY − tileY) inside the mask, ribbon→mask edge

  for (let r = 0; r < roads.length; r++) {
    const road = roads[r];
    if (!Array.isArray(road.pts) || road.pts.length < 2) continue;
    const maskHW = maskHalfWidths[r];
    const gradeHW = gradeHalfWidths[r];
    if (!(maskHW > 0) || !(gradeHW > 0)) {
      throw new Error(`burialStatsV3: road[${r}] half-widths must be > 0 (mask ${maskHW}, grade ${gradeHW})`);
    }
    const dense = densify(road.pts, 2);
    const stations = stationsAlong(dense, stepM);
    const arc = arcLengths(dense);
    const total = arc[arc.length - 1];
    const targets = [];
    for (let d = 0; d <= total; d += stepM) targets.push(d);
    if (targets[targets.length - 1] < total) targets.push(total);

    for (let i = 0; i < stations.length; i++) {
      const st = stations[i];
      sampleCount++;
      if (covers(st.x, st.z)) coveredCount++;

      const profileY = interpolateProfile(road.profile, targets[i]);
      // Left-hand normal to the tangent (x-z plane).
      const nx = -st.tz;
      const nz = st.tx;
      // Shoulder annulus: from just outside the ribbon (mask) edge out to the
      // grading edge + blend band, sampled both sides.
      for (let off = maskHW + 0.1; off <= gradeHW + blendM + 1e-9; off += ANNULUS_STEP_M) {
        for (const side of [1, -1]) {
          const x = st.x + side * nx * off;
          const z = st.z + side * nz * off;
          const tileY = heightAt(x, z);
          if (covers(x, z)) {
            // still inside the mask (ribbon edge → mask edge gap): the skirt must
            // reach DOWN to cover this discarded terrain. requiredDrop = how far
            // the profile sits above the (discarded) tile here.
            const reqDrop = profileY - tileY;
            if (reqDrop > skirtReachM) skirtReachM = reqDrop;
          } else {
            // rendered terrain in the graded shoulder — poke-through budget.
            outside.push({ x, z, devM: Math.max(0, tileY - profileY), class: road.class });
          }
        }
      }
    }
  }

  const coveragePct = sampleCount === 0 ? 100 : (coveredCount / sampleCount) * 100;
  const skirtReachPass = skirtReachM <= skirtDropM;

  if (outside.length === 0) {
    return { coveragePct, sampleCount, coveredCount, outsideCount: 0, maxM: 0, p99M: 0, offenders: [], skirtReachM, skirtReachPass };
  }

  const sorted = outside.slice().sort((a, b) => a.devM - b.devM);
  const n = sorted.length;
  const maxM = sorted[n - 1].devM;
  const p99Idx = Math.min(n - 1, Math.ceil(0.99 * n) - 1);
  const p99M = sorted[p99Idx].devM;
  const BUDGET_MAX = 0.10;
  const offenders = sorted
    .slice(Math.max(0, n - 10))
    .reverse()
    .filter((d) => d.devM >= BUDGET_MAX)
    .map(({ x, z, devM, class: cls }) => ({ x, z, devM, class: cls }));

  return { coveragePct, sampleCount, coveredCount, outsideCount: n, maxM, p99M, offenders, skirtReachM, skirtReachPass };
}

/**
 * §9 metric v4 (spec §5 "Terrain-discard platform", Platform wave). The road
 * platform (ribbon + apron) extends to the DISCARD-MASK edge, and skirts drop
 * from that mask edge PER-VERTEX to `tileGround − 0.5 m`. That makes the old v3
 * shoulder-annulus poke-through budget obsolete: there is no rendered terrain
 * between the ribbon and the mask edge (the mask discards it and the apron
 * covers it), and BEYOND the mask edge the terrain rising above the profile is a
 * legitimate CUT BANK, not a defect (no heightfield budget applies to terrain
 * height beyond the mask — see spec §5/§9). The criterion becomes two geometric
 * invariants:
 *
 *   (a) MASK COVERAGE — the platform INTERIOR is provably discarded. Sample
 *       stations ACROSS the platform: the centreline plus offsets out to
 *       ±(maskHW − edgeEps) in 0.5 m steps, both sides. EVERY sample must be
 *       inside the mask (covers()===true). A hole anywhere under the platform
 *       interior = rendered terrain that could pierce the road surface → FAIL.
 *
 *       `edgeEps` is the mask's own RASTER-QUANTIZATION TOLERANCE, not a fudge:
 *       a nearest-cell mask at cell size C sets a cell iff its CENTRE is within
 *       maskHW of the centreline, so a smooth platform point can land in a cell
 *       whose centre is up to a half-diagonal (C·√2/2) beyond the disk and read
 *       0. The mask therefore GUARANTEES continuous coverage only to
 *       `maskHW − C·√2/2`; the CLI passes exactly that. The thin fringe between
 *       `maskHW − C·√2/2` and `maskHW` is where the discretised mask edge lives —
 *       terrain rendering there is either BELOW the apron (hidden) or a legitimate
 *       CUT BANK rising behind the skirt line (spec §5/§9: no budget beyond the
 *       mask edge). Reported as coveragePct (100 % to pass) + uncovered count.
 *
 *   (b) SKIRT REACH — the skirt always reaches the terrain. At the mask edge
 *       (±(maskHW + ε)) the runtime builds the skirt foot at `tileGround(edge) −
 *       0.5`. This verifies the GEOMETRY CONTRACT directly: recompute the skirt
 *       bottom from the SAME tileGround the runtime uses and assert it sits BELOW
 *       the terrain at the foot (bottom < tileY, i.e. the skirt overlaps the
 *       ground it must hide). The overlap is `tileY − bottom = 0.5` by
 *       construction; we report the max DEFICIT `max(0, bottom − tileY)` (how far
 *       any skirt foot floats ABOVE the terrain) — 0 by construction, and any
 *       positive value (e.g. a synthetic floating skirt) FAILS.
 *
 * @param {{class:string, pts:number[][], profile?:{stepM:number,ys:number[]}}[]} roads
 * @param {(x:number,z:number)=>number} tileGround tile heightfield (profile frame)
 * @param {(x:number,z:number)=>boolean} covers corridor-mask reader
 * @param {{stepM?:number, maskHalfWidths:number[], skirtFootM?:number, edgeEps?:number}} opts
 *   edgeEps: the raster-quantization tolerance (CLI passes cellSize·√2/2). The
 *   platform interior (radius ≤ maskHW − edgeEps) must be 100 % masked.
 * @returns {{coveragePct:number, platformSamples:number, coveredCount:number, uncoveredCount:number, uncovered:{x:number,z:number,class:string}[], skirtDeficitM:number, skirtDeficitPass:boolean, coveragePass:boolean}}
 */
export function burialStatsV4(roads, tileGround, covers, opts) {
  if (!Array.isArray(roads)) throw new Error('burialStatsV4: roads must be an array');
  if (typeof tileGround !== 'function') throw new Error('burialStatsV4: tileGround must be a function');
  if (typeof covers !== 'function') throw new Error('burialStatsV4: covers must be a function');
  if (!opts || typeof opts !== 'object' || !Array.isArray(opts.maskHalfWidths)) {
    throw new Error('burialStatsV4: opts must be { maskHalfWidths, stepM?, skirtFootM?, edgeEps? } — v4 samples ACROSS the platform (Platform wave), not a bare stepM number');
  }
  if (opts.maskHalfWidths.length !== roads.length) {
    throw new Error('burialStatsV4: opts.maskHalfWidths must have one entry per road');
  }
  const stepM = opts.stepM ?? 10;
  const skirtFootM = opts.skirtFootM ?? 0.5;
  const edgeEps = opts.edgeEps ?? 0.1;
  const { maskHalfWidths } = opts;
  if (!(stepM > 0)) throw new Error('burialStatsV4: stepM must be > 0');

  const missing = [];
  for (let i = 0; i < roads.length; i++) if (!roads[i].profile) missing.push(i);
  if (missing.length > 0) {
    throw new Error(
      `burialStatsV4: ${missing.length} road(s) missing a baked profile (indices: ${missing.slice(0, 20).join(', ')}) — run geo:bake-world → geo:attach-profiles first`,
    );
  }

  const ACROSS_STEP_M = 0.5;
  let platformSamples = 0;
  let coveredCount = 0;
  const uncovered = [];
  let skirtDeficitM = 0; // max(0, skirtBottom − tileY) over both mask-edge feet

  for (let r = 0; r < roads.length; r++) {
    const road = roads[r];
    if (!Array.isArray(road.pts) || road.pts.length < 2) continue;
    const maskHW = maskHalfWidths[r];
    if (!(maskHW > 0)) throw new Error(`burialStatsV4: road[${r}] maskHalfWidth must be > 0 (got ${maskHW})`);
    const dense = densify(road.pts, 2);
    const stations = stationsAlong(dense, stepM);

    for (const st of stations) {
      const nx = -st.tz;
      const nz = st.tx;
      // (a) MASK COVERAGE across the platform: centreline + ±offsets to the mask
      // edge (inclusive of 0, exclusive of the very edge by ε).
      for (let off = 0; off <= maskHW - edgeEps + 1e-9; off += ACROSS_STEP_M) {
        for (const side of off === 0 ? [1] : [1, -1]) {
          const x = st.x + side * nx * off;
          const z = st.z + side * nz * off;
          platformSamples++;
          if (covers(x, z)) coveredCount++;
          else uncovered.push({ x, z, class: road.class });
        }
      }
      // (b) SKIRT REACH at the mask edge (±(maskHW + ε)): the runtime skirt foot
      // is tileGround(edge) − skirtFootM. Verify it sits below the terrain there
      // (overlap ≥ 0); the deficit is how far any foot floats ABOVE terrain.
      for (const side of [1, -1]) {
        const x = st.x + side * nx * (maskHW + edgeEps);
        const z = st.z + side * nz * (maskHW + edgeEps);
        const tileY = tileGround(x, z);
        const skirtBottom = tileGround(x, z) - skirtFootM; // SAME sampler the runtime uses
        const deficit = Math.max(0, skirtBottom - tileY); // 0 by construction
        if (deficit > skirtDeficitM) skirtDeficitM = deficit;
      }
    }
  }

  const coveragePct = platformSamples === 0 ? 100 : (coveredCount / platformSamples) * 100;
  const coveragePass = uncovered.length === 0;
  const skirtDeficitPass = skirtDeficitM <= 1e-9;
  return {
    coveragePct,
    platformSamples,
    coveredCount,
    uncoveredCount: uncovered.length,
    uncovered: uncovered.slice(0, 10),
    skirtDeficitM,
    skirtDeficitPass,
    coveragePass,
  };
}

// ---- CLI mode: decode the real bake and print the stats table ----------
const isMain = process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1];
if (isMain) {
  const WORLD_DIR = 'data/winterthur/world';
  const ROADS_PATH = 'data/winterthur/roads.json';
  const TRAFFICNET_PATH = 'data/winterthur/trafficnet.json';
  const useV1 = process.argv.includes('--v1');
  const useV2 = process.argv.includes('--v2');
  const useV3 = process.argv.includes('--v3');

  function fail(msg) {
    console.error(`burial-metric: ${msg}`);
    process.exit(1);
  }

  for (const p of [`${WORLD_DIR}/manifest.pb`, ROADS_PATH, TRAFFICNET_PATH]) {
    if (!existsSync(p)) fail(`missing ${p} — run the geo:bake-world pipeline first`);
  }

  const manifestBin = readFileSync(`${WORLD_DIR}/manifest.pb`);
  const manifest = fromBinary(WorldManifestSchema, manifestBin);

  // Finest-level (highest `level`) tiles first, mirroring
  // src/diorama/ksw/geo/worldData.ts makeHeightSampler.
  const tileRefs = [...manifest.tiles].sort((a, b) => b.level - a.level);
  const tiles = tileRefs.map((ref) => fromBinary(WorldTileSchema, readFileSync(`${WORLD_DIR}/${ref.path}`)));

  function heightAtAbs(x, z) {
    for (const t of tiles) {
      const { gridN, cellSize, originX, originZ, height } = t;
      const fx = (x - originX) / cellSize;
      const fz = (z - originZ) / cellSize;
      if (fx < 0 || fz < 0 || fx > gridN - 1 || fz > gridN - 1) continue;
      const i0 = Math.floor(fx);
      const j0 = Math.floor(fz);
      const i1 = Math.min(i0 + 1, gridN - 1);
      const j1 = Math.min(j0 + 1, gridN - 1);
      const tx = fx - i0;
      const tz = fz - j0;
      const h00 = height[j0 * gridN + i0];
      const h10 = height[j0 * gridN + i1];
      const h01 = height[j1 * gridN + i0];
      const h11 = height[j1 * gridN + i1];
      const h0 = h00 + (h10 - h00) * tx;
      const h1 = h01 + (h11 - h01) * tx;
      return h0 + (h1 - h0) * tz;
    }
    throw new Error(`burial-metric: (${x}, ${z}) not covered by any baked tile`);
  }

  const roadsDoc = JSON.parse(readFileSync(ROADS_PATH, 'utf8'));
  const roads = roadsDoc.roads;
  const rails = roadsDoc.rails ?? [];

  console.log(`burial-metric: ${roads.length} roads, ${rails.length} rails, ${tiles.length} tiles (finest level ${tileRefs[0]?.level ?? 'n/a'})`);

  if (useV1) {
    const trafficNetDoc = JSON.parse(readFileSync(TRAFFICNET_PATH, 'utf8'));
    const floors = laneFloorWidths(roads, trafficNetDoc);
    // Mirror bake-world.mjs's grading corridor width exactly (max(render
    // width, lane floor) + 1.5 m shoulder each side, §4.1) so the metric
    // measures the SAME ribbon the grading pass actually levelled.
    const widths = roads.map((r, i) => Math.max(r.width, floors[i]) + 3.0);

    const stats = burialStats(roads, widths, heightAtAbs, 10);

    console.log('');
    console.log('Burial metric v1 (spec §9, pre-amendment) — acceptance: max < 0.3 m, p99 < 0.15 m');
    console.log('---------------------------------------------------------------------------------');
    console.log(`sampleCount   : ${stats.sampleCount}`);
    console.log(`maxM          : ${stats.maxM.toFixed(3)} m`);
    console.log(`p99M          : ${stats.p99M.toFixed(3)} m`);
    console.log(`pctOver30cm   : ${stats.pctOver30cm.toFixed(2)} %`);
    console.log('');
    console.log('Top offenders:');
    console.log('  x          z          devM     class');
    for (const o of stats.offenders) {
      console.log(
        `  ${o.x.toFixed(1).padStart(9)}  ${o.z.toFixed(1).padStart(9)}  ${o.devM.toFixed(3).padStart(6)}   ${o.class}`,
      );
    }
    const pass = stats.maxM < 0.3 && stats.p99M < 0.15;
    console.log('');
    console.log(pass ? 'PASS: within §9 v1 budget' : 'FAIL: exceeds §9 v1 budget');
  } else if (useV2) {
    // v2 (spec §5 amendment, Task 5c): terrain poke-through against
    // the baked per-way longitudinal profile. Profile ys are stored RELATIVE
    // to the shared anchor (Task 5b: profileAbsoluteMetres − anchorGroundHeight,
    // see .superpowers/sdd/task-5b-report.md), so shift the tile heightfield
    // by the same anchor before comparing — apples to apples.
    const GRADING_PROFILES_PATH = 'scratch/geo/grading-profiles.json';
    if (!existsSync(GRADING_PROFILES_PATH)) {
      fail(`missing ${GRADING_PROFILES_PATH} — run geo:bake-world (writes anchorGroundHeight) first`);
    }
    const { anchorGroundHeight } = JSON.parse(readFileSync(GRADING_PROFILES_PATH, 'utf8'));
    if (typeof anchorGroundHeight !== 'number') {
      fail(`${GRADING_PROFILES_PATH} missing numeric anchorGroundHeight`);
    }
    const heightAtRel = (x, z) => heightAtAbs(x, z) - anchorGroundHeight;

    const allWays = [...roads, ...rails];
    const stats = burialStatsV2(allWays, heightAtRel, 10);

    console.log('');
    console.log('Burial metric v2 (spec §5 amendment) — acceptance: p99 ≤ 0.05 m, max < 0.10 m');
    console.log('-------------------------------------------------------------------------------');
    console.log(`sampleCount   : ${stats.sampleCount}`);
    console.log(`maxM          : ${stats.maxM.toFixed(3)} m`);
    console.log(`p99M          : ${stats.p99M.toFixed(3)} m`);
    console.log('');
    console.log('Top offenders (tileY pierces profileY):');
    console.log('  x          z          devM     class');
    for (const o of stats.offenders) {
      console.log(
        `  ${o.x.toFixed(1).padStart(9)}  ${o.z.toFixed(1).padStart(9)}  ${o.devM.toFixed(3).padStart(6)}   ${o.class}`,
      );
    }
    const pass = stats.maxM < 0.10 && stats.p99M <= 0.05;
    console.log('');
    console.log(pass ? 'PASS: within §9 v2 budget' : 'FAIL: exceeds §9 v2 budget');
  } else if (useV3) {
    // v3 (spec §5 "Terrain-discard", Finding 1b — kept behind --v3 for history).
    // NON-VACUOUS rendered truth. The mask stamps only the RIBBON footprint (Finding 1a),
    // so the graded SHOULDER still renders. v3 samples that shoulder ANNULUS
    // (mask edge + 0.1 m → grade edge + blend) via each station normal and
    // applies the v2 budget only where the sample is OUTSIDE the mask. Three
    // parts reported: (a) centreline coverage = 100 %, (b) annulus poke-through
    // budget, (c) skirt-reach (required drop within any ribbon→mask-edge gap).
    const GRADING_PROFILES_PATH = 'scratch/geo/grading-profiles.json';
    const MASK_PATH = `${WORLD_DIR}/mask.bin`;
    if (!existsSync(GRADING_PROFILES_PATH)) {
      fail(`missing ${GRADING_PROFILES_PATH} — run geo:bake-world (writes anchorGroundHeight) first`);
    }
    if (!existsSync(MASK_PATH)) {
      fail(`missing ${MASK_PATH} — run geo:bake-world (writes the corridor mask) first`);
    }
    const { anchorGroundHeight } = JSON.parse(readFileSync(GRADING_PROFILES_PATH, 'utf8'));
    if (typeof anchorGroundHeight !== 'number') {
      fail(`${GRADING_PROFILES_PATH} missing numeric anchorGroundHeight`);
    }
    const heightAtRel = (x, z) => heightAtAbs(x, z) - anchorGroundHeight;
    const mask = decodeCorridorMask(readFileSync(MASK_PATH));
    const covers = (x, z) => maskCovers(mask, x, z);

    // Per-way half-widths, EXACTLY mirroring bake-world.mjs's `ways` (grading)
    // and `maskWays` (render/ribbon). roads first, then rails — the same order
    // buildCorridorMask/attach-profiles use.
    const trafficNetDoc = JSON.parse(readFileSync(TRAFFICNET_PATH, 'utf8'));
    const floors = laneFloorWidths(roads, trafficNetDoc);
    const maskHalfWidths = [
      ...roads.map((r, i) => Math.max(r.width, floors[i]) / 2), // render ribbon (§ Finding 1a)
      ...rails.map((r) => (r.width + 2.2) / 2), // ballast bed
    ];
    const gradeHalfWidths = [
      ...roads.map((r, i) => Math.max(r.width, floors[i]) / 2 + 1.5), // grading shoulder (§4.1)
      ...rails.map((r) => (r.width + 2.2) / 2 + 2.0), // grading shoulder (§4.2)
    ];

    const allWays = [...roads, ...rails];
    const stats = burialStatsV3(allWays, heightAtRel, covers, {
      stepM: 10,
      maskHalfWidths,
      gradeHalfWidths,
      blendM: 3,
      skirtDropM: 1.5,
    });

    console.log('');
    console.log('Burial metric v3 (spec §5 terrain-discard, non-vacuous) — coverage = 100 %, shoulder-annulus p99 ≤ 0.05 m / max < 0.10 m, skirt-reach ≤ 1.5 m');
    console.log('----------------------------------------------------------------------------------------------------------------------------------------------');
    console.log(`mask          : ${mask.cols}×${mask.rows} cells @ ${mask.cellSizeM}m`);
    console.log(`centreline    : ${stats.sampleCount} stations (${stats.coveredCount} in mask)`);
    console.log(`coveragePct   : ${stats.coveragePct.toFixed(4)} %`);
    console.log(`annulus outside: ${stats.outsideCount} samples`);
    console.log(`maxM (annulus): ${stats.maxM.toFixed(3)} m`);
    console.log(`p99M (annulus): ${stats.p99M.toFixed(3)} m`);
    console.log(`skirtReachM   : ${stats.skirtReachM.toFixed(3)} m (skirt drop 1.5 m)`);
    console.log('');
    console.log('Top offenders in the shoulder annulus (tileY pierces profileY OUTSIDE the mask):');
    console.log('  x          z          devM     class');
    for (const o of stats.offenders) {
      console.log(
        `  ${o.x.toFixed(1).padStart(9)}  ${o.z.toFixed(1).padStart(9)}  ${o.devM.toFixed(3).padStart(6)}   ${o.class}`,
      );
    }
    const coverPass = stats.coveragePct >= 100;
    const budgetPass = stats.maxM < 0.10 && stats.p99M <= 0.05;
    console.log('');
    console.log(`coverage: ${coverPass ? 'PASS (100%)' : `FAIL (${stats.coveragePct.toFixed(4)}%)`}`);
    console.log(`budget (shoulder annulus): ${budgetPass ? 'PASS' : 'FAIL'}`);
    console.log(`skirt-reach: ${stats.skirtReachPass ? 'PASS' : `FAIL (${stats.skirtReachM.toFixed(3)} m > 1.5 m)`}`);
    console.log(coverPass && budgetPass && stats.skirtReachPass ? 'PASS: within §9 v3 budget' : 'FAIL: exceeds §9 v3 budget');
  } else {
    // v4 (spec §5 "Terrain-discard platform", Platform wave — DEFAULT). Roads own
    // a platform (ribbon + apron to the mask edge) and skirts drop per-vertex to
    // tileGround − 0.5 m. Two geometric invariants: (a) 100 % mask coverage
    // ACROSS the platform (centreline → ±(maskHW − ε), 0.5 m steps), and (b)
    // skirt-reach: the mask-edge skirt foot (tileGround − 0.5) sits below the
    // terrain there (deficit 0). No shoulder poke-through budget — terrain rising
    // beyond the mask edge is a legitimate cut bank (spec §5/§9).
    const GRADING_PROFILES_PATH = 'scratch/geo/grading-profiles.json';
    const MASK_PATH = `${WORLD_DIR}/mask.bin`;
    if (!existsSync(GRADING_PROFILES_PATH)) {
      fail(`missing ${GRADING_PROFILES_PATH} — run geo:bake-world (writes anchorGroundHeight) first`);
    }
    if (!existsSync(MASK_PATH)) {
      fail(`missing ${MASK_PATH} — run geo:bake-world (writes the corridor mask) first`);
    }
    const { anchorGroundHeight } = JSON.parse(readFileSync(GRADING_PROFILES_PATH, 'utf8'));
    if (typeof anchorGroundHeight !== 'number') {
      fail(`${GRADING_PROFILES_PATH} missing numeric anchorGroundHeight`);
    }
    const heightAtRel = (x, z) => heightAtAbs(x, z) - anchorGroundHeight;
    const mask = decodeCorridorMask(readFileSync(MASK_PATH));
    const covers = (x, z) => maskCovers(mask, x, z);

    // Per-way effective MASK half-width, EXACTLY the bake mask footprint: the
    // render (ribbon) half-width floored at the mask cell size (corridormask.mjs
    // `Math.max(halfWidthM, cellSize)`). roads first, then rails — the same order
    // buildCorridorMask/attach-profiles use.
    const trafficNetDoc = JSON.parse(readFileSync(TRAFFICNET_PATH, 'utf8'));
    const floors = laneFloorWidths(roads, trafficNetDoc);
    const MASK_CELL_M = mask.cellSizeM; // the bake's own cell size (2.5 m)
    const maskHalfWidths = [
      ...roads.map((r, i) => Math.max(Math.max(r.width, floors[i]) / 2, MASK_CELL_M)),
      ...rails.map((r) => Math.max((r.width + 2.2) / 2, MASK_CELL_M)),
    ];

    // Raster-quantization tolerance: a nearest-cell mask guarantees continuous
    // coverage only to maskHW − cell·√2/2 (a smooth point can fall in a cell whose
    // centre is a half-diagonal beyond the disk). Sample the platform INTERIOR to
    // that radius — 100 % coverage there is the provable "no rendered terrain under
    // the platform interior" invariant. See burialStatsV4 doc (a).
    const RASTER_EPS = (mask.cellSizeM * Math.SQRT2) / 2;
    const allWays = [...roads, ...rails];
    const stats = burialStatsV4(allWays, heightAtRel, covers, {
      stepM: 10,
      maskHalfWidths,
      skirtFootM: 0.5,
      edgeEps: RASTER_EPS,
    });

    console.log('');
    console.log('Burial metric v4 (spec §5 terrain-discard platform, Platform wave) — (a) 100 % mask coverage ACROSS the platform, (b) skirt-reach deficit = 0 m');
    console.log('----------------------------------------------------------------------------------------------------------------------------------------------');
    console.log(`mask          : ${mask.cols}×${mask.rows} cells @ ${mask.cellSizeM}m`);
    console.log(`platform samp : ${stats.platformSamples} (${stats.coveredCount} in mask)`);
    console.log(`coveragePct   : ${stats.coveragePct.toFixed(4)} %`);
    console.log(`uncovered     : ${stats.uncoveredCount} platform samples`);
    console.log(`skirtDeficitM : ${stats.skirtDeficitM.toFixed(3)} m (skirt foot above terrain; 0 = always reaches)`);
    if (stats.uncovered.length > 0) {
      console.log('');
      console.log('Uncovered platform samples (rendered terrain UNDER the platform → could pierce):');
      console.log('  x          z          class');
      for (const o of stats.uncovered) {
        console.log(`  ${o.x.toFixed(1).padStart(9)}  ${o.z.toFixed(1).padStart(9)}   ${o.class}`);
      }
    }
    console.log('');
    console.log(`(a) mask coverage: ${stats.coveragePass ? 'PASS (100%)' : `FAIL (${stats.uncoveredCount} uncovered, ${stats.coveragePct.toFixed(4)}%)`}`);
    console.log(`(b) skirt-reach  : ${stats.skirtDeficitPass ? 'PASS (deficit 0)' : `FAIL (${stats.skirtDeficitM.toFixed(3)} m float)`}`);
    console.log(stats.coveragePass && stats.skirtDeficitPass ? 'PASS: within §9 v4 budget' : 'FAIL: exceeds §9 v4 budget');
  }
}
