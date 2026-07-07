// scripts/traffic/map-count-stations.mjs
//
// S2 calibration Task 2 (plan: docs/superpowers/plans/
// 2026-07-06-traffic-sota-s2-calibration.md): map each observed count
// station-direction (from fetch-count-profiles.mjs output) onto the directed
// trafficnet EDGE it measures, so the sim harness can count crossings at the
// same cross-sections.
//
// Direction resolution: a station-direction like "Richtung Seen" implies
// travel TOWARD a named locality. We author the localities' coordinates
// (stable geography, not data) and pick — among all lanes within
// SEARCH_RADIUS_M of the station — the directed edge whose local travel
// bearing best aligns with the bearing from the station to that locality
// (cosine similarity, hard-gated). Ambiguity or a miss is a LOUD error, not
// a guess: a mis-mapped cross-section would silently poison the whole
// calibration.
//
// Usage:
//   node scripts/traffic/map-count-stations.mjs \
//     [--profiles scratch/calibration/observed-profiles.json] \
//     [--net data/winterthur/trafficnet.json] \
//     [--output data/winterthur/count-stations.json]

import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { ANCHOR, makeProjector } from '../geo/lib/project.mjs';

/** Search radius around the station coordinate for candidate lanes (m). The
 * loop sits in the carriageway, so the true lane passes within metres; 40 m
 * bounds it while excluding parallel streets. */
const SEARCH_RADIUS_M = 40;

/** Minimum cosine similarity between lane travel bearing and the
 * station→destination bearing for an unambiguous match. */
const MIN_BEARING_COS = 0.5;

/** Authored destination localities (WGS84) for the `richtung_name` values of
 * the Winterthur count feed. Geography, not measurement — extend when the
 * Tiefbauamt brings more stations online. */
export const DESTINATIONS = {
  Seen: { lon: 8.7594, lat: 47.4812 },
  Oberwinterthur: { lon: 8.7551, lat: 47.5177 },
  'Grüze': { lon: 8.7519, lat: 47.4925 },
  'Winterthur Zentrum': { lon: 8.7295, lat: 47.4991 },
  'Autobahnanschluss A1': { lon: 8.7045, lat: 47.4935 }, // Anschluss Töss
  Bassersdorf: { lon: 8.6294, lat: 47.4436 },
};

/** Resolve `richtung_name` ("Richtung X") to an authored destination. */
export function destinationOf(richtungName) {
  const name = richtungName.replace(/^Richtung\s+/u, '');
  const dest = DESTINATIONS[name];
  if (!dest) throw new Error(`no authored destination for "${richtungName}" — extend DESTINATIONS`);
  return dest;
}

/** Squared distance from point p to segment ab, plus the segment's unit
 * travel direction. All in world (x, z). */
function segClosest(p, a, b) {
  const abx = b[0] - a[0];
  const abz = b[1] - a[1];
  const len2 = abx * abx + abz * abz;
  const t = len2 > 0 ? Math.max(0, Math.min(1, ((p[0] - a[0]) * abx + (p[1] - a[1]) * abz) / len2)) : 0;
  const cx = a[0] + t * abx;
  const cz = a[1] + t * abz;
  const dx = p[0] - cx;
  const dz = p[1] - cz;
  const len = Math.sqrt(len2);
  return { d2: dx * dx + dz * dz, dir: len > 0 ? [abx / len, abz / len] : [0, 0] };
}

/**
 * Map every station-direction in `profiles` onto its directed edge of `net`.
 * Returns `{ stations: [...] }`; throws on any unmappable entry.
 */
export function mapStations(profiles, net) {
  const proj = makeProjector(ANCHOR);
  const laneById = new Map(net.lanes.map((l) => [l.id, l]));
  const out = [];

  for (const st of profiles.stations) {
    const [sx, sz] = proj.toLocal(st.lon, st.lat);
    const dest = destinationOf(st.richtungName);
    const [dxw, dzw] = proj.toLocal(dest.lon, dest.lat);
    let want = [dxw - sx, dzw - sz];
    const wlen = Math.hypot(want[0], want[1]);
    want = [want[0] / wlen, want[1] / wlen];

    // Best candidate per EDGE (a multi-lane edge yields its closest lane).
    const byEdge = new Map(); // edge id → {d, cos, laneDir}
    for (const lane of net.lanes) {
      const pts = lane.pts;
      for (let i = 0; i + 1 < pts.length; i++) {
        const { d2, dir } = segClosest([sx, sz], pts[i], pts[i + 1]);
        if (d2 > SEARCH_RADIUS_M * SEARCH_RADIUS_M) continue;
        const cos = dir[0] * want[0] + dir[1] * want[1];
        const cur = byEdge.get(lane.edge);
        if (!cur || d2 < cur.d2) byEdge.set(lane.edge, { d2, cos });
      }
    }
    if (byEdge.size === 0) {
      throw new Error(
        `${st.anlageName} [${st.richtungName}]: no lane within ${SEARCH_RADIUS_M} m of station`,
      );
    }

    // Among nearby edges, the aligned ones; then the closest of those.
    const aligned = [...byEdge.entries()].filter(([, c]) => c.cos >= MIN_BEARING_COS);
    if (aligned.length === 0) {
      throw new Error(
        `${st.anlageName} [${st.richtungName}]: ${byEdge.size} nearby edges but none aligned ` +
          `(best cos ${Math.max(...[...byEdge.values()].map((c) => c.cos)).toFixed(2)}) — check DESTINATIONS`,
      );
    }
    // Proximity first, alignment as OVERRIDE: the loop lies IN the measured
    // carriageway (sub-metre), so the nearest aligned edge is almost always
    // right. But at a crossing (K501 sits on the Seenerstrasse×Rudolf-Diesel
    // corner) the nearest edge can belong to the OTHER street while still
    // clearing the cos gate — only then does a markedly better-aligned edge
    // (cos +0.15) farther out take over. A marginal cos edge must NOT beat a
    // 12× closer one (first calibration run: 5554 @ 11.8 m/cos .946 stole
    // Seenerstrasse from the true 5471 @ 1.0 m/cos .939 and measured a side
    // arm with zero flow).
    const COS_OVERRIDE = 0.15;
    aligned.sort((a, b) => a[1].d2 - b[1].d2);
    let [edgeId, best] = aligned[0];
    for (const [cand, c] of aligned.slice(1)) {
      if (c.cos >= best.cos + COS_OVERRIDE) {
        edgeId = cand;
        best = c;
      }
    }
    const edge = net.edges.find((e) => e.id === edgeId);

    out.push({
      anlageName: st.anlageName,
      richtung: st.richtung,
      richtungName: st.richtungName,
      edge: edgeId,
      lanes: edge.lanes,
      distM: Number(Math.sqrt(best.d2).toFixed(1)),
      bearingCos: Number(best.cos.toFixed(3)),
      worldX: Number(sx.toFixed(1)),
      worldZ: Number(sz.toFixed(1)),
    });
  }

  out.sort((a, b) =>
    `${a.anlageName}|${a.richtung}`.localeCompare(`${b.anlageName}|${b.richtung}`),
  );
  return { searchRadiusM: SEARCH_RADIUS_M, minBearingCos: MIN_BEARING_COS, stations: out };
}

const isMain = process.argv[1] && path.resolve(process.argv[1]) === new URL(import.meta.url).pathname;
if (isMain) {
  const arg = (name, dflt) => {
    const i = process.argv.indexOf(name);
    return i >= 0 ? process.argv[i + 1] : dflt;
  };
  const profiles = JSON.parse(
    readFileSync(arg('--profiles', 'scratch/calibration/observed-profiles.json'), 'utf8'),
  );
  const net = JSON.parse(readFileSync(arg('--net', 'data/winterthur/trafficnet.json'), 'utf8'));
  const mapped = mapStations(profiles, net);
  const output = arg('--output', 'data/winterthur/count-stations.json');
  writeFileSync(output, JSON.stringify(mapped, null, 2));
  for (const s of mapped.stations) {
    console.log(
      `${s.anlageName} [${s.richtungName}] → edge ${s.edge} (lanes ${s.lanes.join(',')}) ` +
        `dist=${s.distM}m cos=${s.bearingCos}`,
    );
  }
  console.log(`wrote ${output} (${mapped.stations.length} mappings)`);
}
