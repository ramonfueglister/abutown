// tests/traffic/countProfiles.test.ts
//
// S2 calibration Task 1: the observed-profile extractor over the Stadt-
// Winterthur MIV CSV. Pins the four load-bearing behaviours on a synthetic
// fixture: Tue–Thu weekday filter, fixed-holiday exclusion, per-timestamp
// lane summing before hour-of-day averaging, and the SWISS10 → 3-class
// bucket mapping.

import { describe, expect, it } from 'vitest';
import { buildProfiles, isHoliday } from '../../scripts/traffic/fetch-count-profiles.mjs';

const HEADER =
  'gemeinde_bfs_nr,gemeinde,anlage_nr,anlage_name,anlage_typ,zeit_von,zeit_bis,richtung,' +
  'richtung_name,spur_nr,lat,lon,bus,mr,pw,pw_plus,lief,lief_plus,lief_aufl,lw,lw_plus,sattelzug,total';

/** One CSV row for station/direction at an ISO time with given class counts. */
function row(
  zeitVon: string,
  spur: number,
  counts: Partial<Record<string, number>>,
  richtung = 'O',
): string {
  const c = (k: string) => counts[k] ?? 0;
  const total =
    c('bus') + c('mr') + c('pw') + c('pw_plus') + c('lief') + c('lief_plus') + c('lief_aufl') +
    c('lw') + c('lw_plus') + c('sattelzug');
  return (
    `230,Winterthur,abc123,K999 MIV Teststrasse,MIV,${zeitVon},${zeitVon},${richtung},` +
    `Richtung Test,${spur}.0,47.5,8.72,${c('bus')},${c('mr')},${c('pw')},${c('pw_plus')},` +
    `${c('lief')},${c('lief_plus')},${c('lief_aufl')},${c('lw')},${c('lw_plus')},${c('sattelzug')},${total}`
  );
}

describe('buildProfiles', () => {
  it('sums lanes per timestamp, averages per local hour, maps class buckets', () => {
    // Two Tuesdays, 08:00 local (CET winter, +01 short offset like the real
    // feed): lane 1 + lane 2 must sum per timestamp, then average across the
    // two days: car (10+20 and 30+40) → mean 50; delivery 2 & 4 → 3;
    // truck 1 & 1 → 1.
    const csv = [
      HEADER,
      row('2026-01-06T08:00:00+01', 1, { pw: 8, mr: 1, bus: 1, lief: 2, sattelzug: 1 }),
      row('2026-01-06T08:00:00+01', 2, { pw: 20 }),
      row('2026-01-13T08:00:00+01', 1, { pw: 30, lief_aufl: 4, lw: 1 }),
      row('2026-01-13T08:00:00+01', 2, { pw_plus: 40 }),
    ].join('\n');
    const p = buildProfiles(csv);
    expect(p.stations).toHaveLength(1);
    const st = p.stations[0];
    expect(st.anlageName).toBe('K999 MIV Teststrasse');
    expect(st.hours.car[8]).toBe(50);
    expect(st.hours.delivery[8]).toBe(3);
    expect(st.hours.truck[8]).toBe(1);
    // Hours without samples stay 0.
    expect(st.hours.car[3]).toBe(0);
  });

  it('keeps only Tue-Thu and drops fixed Swiss holidays', () => {
    const csv = [
      HEADER,
      row('2026-01-05T08:00:00+01', 1, { pw: 100 }), // Monday → dropped
      row('2026-01-09T08:00:00+01', 1, { pw: 100 }), // Friday → dropped
      row('2026-01-10T08:00:00+01', 1, { pw: 100 }), // Saturday → dropped
      row('2026-01-01T08:00:00+01', 1, { pw: 100 }), // Neujahr (a Thursday) → dropped
      row('2026-01-06T08:00:00+01', 1, { pw: 42 }), // Tuesday → kept
    ].join('\n');
    const p = buildProfiles(csv);
    expect(p.stations).toHaveLength(1);
    expect(p.stations[0].nHourSamples).toBe(1);
    expect(p.stations[0].hours.car[8]).toBe(42);
  });

  it('separates directions of the same Anlage and buckets by local Zurich hour', () => {
    // 2026-07-07 is a Tuesday; +00 offset at 06:00 UTC = 08:00 in Zurich (CEST).
    const csv = [
      HEADER,
      row('2026-07-07T06:00:00+00:00', 1, { pw: 10 }, 'O'),
      row('2026-07-07T06:00:00+00:00', 1, { pw: 20 }, 'W'),
    ].join('\n');
    const p = buildProfiles(csv);
    expect(p.stations).toHaveLength(2);
    for (const st of p.stations) expect(st.hours.car[8]).toBeGreaterThan(0);
  });

  it('mirrors the backend holiday list', () => {
    expect(isHoliday(1, 1)).toBe(true);
    expect(isHoliday(8, 1)).toBe(true);
    expect(isHoliday(12, 26)).toBe(true);
    expect(isHoliday(7, 7)).toBe(false);
  });
});
