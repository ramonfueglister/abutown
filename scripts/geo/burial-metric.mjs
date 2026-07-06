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

// ---- CLI mode: decode the real bake and print the stats table ----------
const isMain = process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1];
if (isMain) {
  const WORLD_DIR = 'data/winterthur/world';
  const ROADS_PATH = 'data/winterthur/roads.json';
  const TRAFFICNET_PATH = 'data/winterthur/trafficnet.json';

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

  function heightAt(x, z) {
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
  const trafficNetDoc = JSON.parse(readFileSync(TRAFFICNET_PATH, 'utf8'));
  const roads = roadsDoc.roads;
  const floors = laneFloorWidths(roads, trafficNetDoc);
  // Mirror bake-world.mjs's grading corridor width exactly (max(render
  // width, lane floor) + 1.5 m shoulder each side, §4.1) so the metric
  // measures the SAME ribbon the grading pass actually levelled.
  const widths = roads.map((r, i) => Math.max(r.width, floors[i]) + 3.0);

  console.log(`burial-metric: ${roads.length} roads, ${tiles.length} tiles (finest level ${tileRefs[0]?.level ?? 'n/a'})`);
  const stats = burialStats(roads, widths, heightAt, 10);

  console.log('');
  console.log('Burial metric (spec §9) — acceptance: max < 0.3 m, p99 < 0.15 m');
  console.log('-------------------------------------------------------------');
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
  console.log(pass ? 'PASS: within §9 budget' : 'FAIL: exceeds §9 budget');
}
