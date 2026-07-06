// scripts/traffic/calibration-report.mjs
//
// S2 calibration Task 4: compare simulated vs observed hourly profiles per
// station-direction and class bucket via the GEH statistic
// (UK DMRB / Highways England practice: GEH = sqrt(2(M−C)²/(M+C)) with M =
// modelled flow, C = observed flow, both veh/h; GEH < 5 is a good fit,
// applied to volumes of meaningful size — tiny flows make GEH unstable, so
// hours with C < MIN_OBSERVED_VPH are reported but not gated).
//
// This is a REPORT, not a CI gate (swiss-roads lesson: prove a metric's
// discriminating power before gating on it).
//
// Usage:
//   node scripts/traffic/calibration-report.mjs \
//     [--observed scratch/calibration/observed-profiles.json] \
//     [--simulated scratch/calibration/simulated-profiles.json] \
//     [--output scratch/calibration/calibration-report.md]

import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';

/** Below this observed flow (veh/h) an hour is informational only. */
export const MIN_OBSERVED_VPH = 100;

/** GEH statistic for modelled `m` vs observed `c` hourly flows. */
export function geh(m, c) {
  if (m + c === 0) return 0;
  return Math.sqrt((2 * (m - c) * (m - c)) / (m + c));
}

/**
 * Join observed and simulated stations on (anlageName, richtungName) and
 * compute per-hour GEH for every class bucket. Throws on stations present
 * on one side only (a silent drop would fake coverage).
 */
export function compare(observed, simulated) {
  const key = (s) => `${s.anlageName}|${s.richtungName}`;
  const simBy = new Map(simulated.stations.map((s) => [key(s), s]));
  const stations = [];
  for (const obs of observed.stations) {
    const sim = simBy.get(key(obs));
    if (!sim) throw new Error(`no simulated profile for ${key(obs)}`);
    simBy.delete(key(obs));
    const buckets = {};
    for (const bucket of ['car', 'delivery', 'truck']) {
      buckets[bucket] = obs.hours[bucket].map((c, h) => {
        const m = sim.hours[bucket][h];
        return { hour: h, observed: c, simulated: m, geh: geh(m, c) };
      });
    }
    stations.push({ anlageName: obs.anlageName, richtungName: obs.richtungName, buckets });
  }
  if (simBy.size > 0) {
    throw new Error(`simulated-only stations: ${[...simBy.keys()].join(', ')}`);
  }

  // Headline: GEH<5 share over gated hours (observed car flow ≥ threshold).
  let gated = 0;
  let gatedOk = 0;
  for (const st of stations) {
    for (const e of st.buckets.car) {
      if (e.observed >= MIN_OBSERVED_VPH) {
        gated++;
        if (e.geh < 5) gatedOk++;
      }
    }
  }
  return { stations, gated, gatedOk };
}

/** Render the comparison as a Markdown report. */
export function renderReport(cmp, meta) {
  const lines = [];
  lines.push('# Traffic calibration report — simulated vs observed (S2)');
  lines.push('');
  lines.push(
    `Sim: seed=${meta.seed}, demand_scale=${meta.demandScale}, date=${meta.date}. ` +
      `Observed: Stadt Winterthur MIV counts, Tue–Thu means.`,
  );
  lines.push('');
  const pct = cmp.gated > 0 ? ((100 * cmp.gatedOk) / cmp.gated).toFixed(0) : 'n/a';
  lines.push(
    `**Headline: GEH < 5 in ${cmp.gatedOk}/${cmp.gated} gated station-hours (${pct}%)** ` +
      `(gated = observed car flow ≥ ${MIN_OBSERVED_VPH}/h; DMRB practice target ≥ 85%).`,
  );
  lines.push('');
  for (const st of cmp.stations) {
    lines.push(`## ${st.anlageName} — ${st.richtungName}`);
    lines.push('');
    lines.push('| h | obs car | sim car | GEH | obs lief | sim lief | obs LKW | sim LKW |');
    lines.push('|---|---|---|---|---|---|---|---|');
    for (let h = 0; h < 24; h++) {
      const c = st.buckets.car[h];
      const d = st.buckets.delivery[h];
      const t = st.buckets.truck[h];
      const mark = c.observed >= MIN_OBSERVED_VPH ? (c.geh < 5 ? ' ✓' : ' ✗') : '';
      lines.push(
        `| ${h} | ${c.observed.toFixed(0)} | ${c.simulated.toFixed(0)} | ${c.geh.toFixed(1)}${mark} ` +
          `| ${d.observed.toFixed(0)} | ${d.simulated.toFixed(0)} ` +
          `| ${t.observed.toFixed(0)} | ${t.simulated.toFixed(0)} |`,
      );
    }
    lines.push('');
  }
  return lines.join('\n');
}

const isMain = process.argv[1] && path.resolve(process.argv[1]) === new URL(import.meta.url).pathname;
if (isMain) {
  const arg = (name, dflt) => {
    const i = process.argv.indexOf(name);
    return i >= 0 ? process.argv[i + 1] : dflt;
  };
  const observed = JSON.parse(
    readFileSync(arg('--observed', 'scratch/calibration/observed-profiles.json'), 'utf8'),
  );
  const simulated = JSON.parse(
    readFileSync(arg('--simulated', 'scratch/calibration/simulated-profiles.json'), 'utf8'),
  );
  const cmp = compare(observed, simulated);
  const md = renderReport(cmp, simulated);
  const output = arg('--output', 'scratch/calibration/calibration-report.md');
  mkdirSync(path.dirname(output), { recursive: true });
  writeFileSync(output, md);
  const pct = cmp.gated > 0 ? ((100 * cmp.gatedOk) / cmp.gated).toFixed(0) : 'n/a';
  console.log(`GEH<5: ${cmp.gatedOk}/${cmp.gated} gated station-hours (${pct}%) → ${output}`);
}
