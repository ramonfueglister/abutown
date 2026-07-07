// scripts/traffic/fetch-count-profiles.mjs
//
// S2 calibration Task 1 (plan: docs/superpowers/plans/
// 2026-07-06-traffic-sota-s2-calibration.md): turn the Stadt-Winterthur
// MIV count CSV (hourly induction-loop data, SWISS10 classes, per lane +
// direction) into observed WEEKDAY hourly profiles per station×direction and
// vehicle-class bucket, matching the sim's three classes:
//
//   car      = pw + pw_plus + mr + bus     (private/passenger movements)
//   delivery = lief + lief_plus + lief_aufl
//   truck    = lw + lw_plus + sattelzug
//
// Weekday filter mirrors the sim's DayKind: Tue–Thu only (classic "DTV
// Di–Do" engineering practice — Monday/Friday carry commute edge effects),
// minus the same fixed-date Swiss holidays the backend clock authors
// (clock.rs HOLIDAYS): 1.1., 2.1., 1.8., 25.12., 26.12.
//
// Usage:
//   node scripts/traffic/fetch-count-profiles.mjs \
//     [--input scratch/calibration/winterthur-miv.csv] \
//     [--output scratch/calibration/observed-profiles.json] \
//     [--download]   # fetch the newest CSV from the ZH OGD portal first
//
// Data source (opendata.swiss "Verkehrszähldaten motorisierter
// Individualverkehr in Winterthur", Tiefbauamt Winterthur, open use with
// attribution): https://daten.statistik.zh.ch/ogd/daten/ressourcen/
// KTZH_00003042_00006323.csv — raw sensor data, not plausibilized (doc TXT
// KTZH_00003042_00006324).

import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';

export const CSV_URL =
  'https://daten.statistik.zh.ch/ogd/daten/ressourcen/KTZH_00003042_00006323.csv';

/** Fixed-date Swiss holidays (month, day) — MUST mirror clock.rs HOLIDAYS. */
export const HOLIDAYS = [
  [1, 1],
  [1, 2],
  [8, 1],
  [12, 25],
  [12, 26],
];

/** SWISS10 → sim-class bucket mapping (column names of the CSV). */
export const CLASS_BUCKETS = {
  car: ['pw', 'pw_plus', 'mr', 'bus'],
  delivery: ['lief', 'lief_plus', 'lief_aufl'],
  truck: ['lw', 'lw_plus', 'sattelzug'],
};

/** Local (Europe/Zurich) parts of an ISO timestamp with numeric offset. */
const zurichParts = (() => {
  const fmt = new Intl.DateTimeFormat('en-CA', {
    timeZone: 'Europe/Zurich',
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    hourCycle: 'h23',
    weekday: 'short',
  });
  return (iso) => {
    // The portal emits short offsets like `+01` — JS Date needs `+01:00`.
    const norm = /[+-]\d{2}$/.test(iso) ? `${iso}:00` : iso;
    const d = new Date(norm);
    if (Number.isNaN(d.getTime())) throw new Error(`unparseable zeit_von: ${iso}`);
    const parts = Object.fromEntries(fmt.formatToParts(d).map((p) => [p.type, p.value]));
    return {
      month: Number(parts.month),
      day: Number(parts.day),
      hour: Number(parts.hour),
      weekday: parts.weekday, // 'Tue' etc.
    };
  };
})();

/** Whether a local date is one of the authored fixed-date holidays. */
export function isHoliday(month, day) {
  return HOLIDAYS.some(([m, d]) => m === month && d === day);
}

/**
 * Aggregate the CSV text into observed profiles.
 *
 * Returns `{ stations: [{ anlageName, richtung, richtungName, lat, lon,
 * nHourSamples, hours: { car: [24], delivery: [24], truck: [24] } }] }`
 * where each `hours[bucket][h]` is the MEAN vehicles/hour over all Tue–Thu
 * non-holiday samples of that local hour (lanes of one Anlage×Richtung are
 * summed per timestamp before averaging).
 */
export function buildProfiles(csvText) {
  const lines = csvText.split('\n').filter((l) => l.length > 0);
  const header = lines[0].split(',');
  if (lines[0].includes('"')) {
    throw new Error('quoted CSV fields not supported — format changed upstream, adapt the parser');
  }
  const col = Object.fromEntries(header.map((name, i) => [name, i]));
  for (const need of [
    'anlage_name',
    'zeit_von',
    'richtung',
    'richtung_name',
    'spur_nr',
    'lat',
    'lon',
    'total',
  ]) {
    if (!(need in col)) throw new Error(`CSV missing column ${need}`);
  }

  // key = anlage_name|richtung → station accumulator
  // per station: hourKey = zeit_von → per-bucket lane-summed counts, then
  // those per-timestamp sums feed hour-of-day means.
  const stations = new Map();

  for (let i = 1; i < lines.length; i++) {
    const f = lines[i].split(',');
    const t = zurichParts(f[col.zeit_von]);
    if (!['Tue', 'Wed', 'Thu'].includes(t.weekday)) continue;
    if (isHoliday(t.month, t.day)) continue;

    const key = `${f[col.anlage_name]}|${f[col.richtung]}`;
    let st = stations.get(key);
    if (!st) {
      st = {
        anlageName: f[col.anlage_name],
        richtung: f[col.richtung],
        richtungName: f[col.richtung_name],
        lat: Number(f[col.lat]),
        lon: Number(f[col.lon]),
        byTimestamp: new Map(), // zeit_von → {car, delivery, truck, hour}
      };
      stations.set(key, st);
    }
    let ts = st.byTimestamp.get(f[col.zeit_von]);
    if (!ts) {
      ts = { hour: t.hour, car: 0, delivery: 0, truck: 0 };
      st.byTimestamp.set(f[col.zeit_von], ts);
    }
    for (const [bucket, cols] of Object.entries(CLASS_BUCKETS)) {
      for (const c of cols) ts[bucket] += Number(f[col[c]] ?? 0);
    }
  }

  const out = [];
  for (const st of stations.values()) {
    const sums = {
      car: new Array(24).fill(0),
      delivery: new Array(24).fill(0),
      truck: new Array(24).fill(0),
    };
    const n = new Array(24).fill(0);
    for (const ts of st.byTimestamp.values()) {
      n[ts.hour] += 1;
      sums.car[ts.hour] += ts.car;
      sums.delivery[ts.hour] += ts.delivery;
      sums.truck[ts.hour] += ts.truck;
    }
    const mean = (arr) => arr.map((v, h) => (n[h] > 0 ? v / n[h] : 0));
    out.push({
      anlageName: st.anlageName,
      richtung: st.richtung,
      richtungName: st.richtungName,
      lat: st.lat,
      lon: st.lon,
      nHourSamples: st.byTimestamp.size,
      hours: { car: mean(sums.car), delivery: mean(sums.delivery), truck: mean(sums.truck) },
    });
  }
  // Deterministic output order.
  out.sort((a, b) =>
    `${a.anlageName}|${a.richtung}`.localeCompare(`${b.anlageName}|${b.richtung}`),
  );
  return { source: CSV_URL, weekdayFilter: 'Tue-Thu minus fixed Swiss holidays', stations: out };
}

const isMain = process.argv[1] && path.resolve(process.argv[1]) === new URL(import.meta.url).pathname;
if (isMain) {
  const arg = (name, dflt) => {
    const i = process.argv.indexOf(name);
    return i >= 0 ? process.argv[i + 1] : dflt;
  };
  const input = arg('--input', 'scratch/calibration/winterthur-miv.csv');
  const output = arg('--output', 'scratch/calibration/observed-profiles.json');

  if (process.argv.includes('--download')) {
    const res = await fetch(CSV_URL);
    if (!res.ok) throw new Error(`download failed: ${res.status}`);
    mkdirSync(path.dirname(input), { recursive: true });
    writeFileSync(input, Buffer.from(await res.arrayBuffer()));
    console.log(`downloaded ${input}`);
  }

  const profiles = buildProfiles(readFileSync(input, 'utf8'));
  mkdirSync(path.dirname(output), { recursive: true });
  writeFileSync(output, JSON.stringify(profiles, null, 2));
  for (const s of profiles.stations) {
    const peak = Math.max(...s.hours.car);
    const peakH = s.hours.car.indexOf(peak);
    console.log(
      `${s.anlageName} [${s.richtungName}] samples=${s.nHourSamples} ` +
        `car-peak=${peak.toFixed(0)}/h @ ${peakH}:00 ` +
        `truck-day=${s.hours.truck.reduce((a, b) => a + b, 0).toFixed(0)}/d`,
    );
  }
  console.log(`wrote ${output} (${profiles.stations.length} station-directions)`);
}
