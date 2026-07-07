// tests/traffic/calibrationReport.test.ts
//
// S2 Task 4: GEH math + join semantics of the calibration report.

import { describe, expect, it } from 'vitest';
import {
  MIN_OBSERVED_VPH,
  compare,
  geh,
} from '../../scripts/traffic/calibration-report.mjs';

function station(name: string, carHours: number[]) {
  const zeros = new Array(24).fill(0);
  return {
    anlageName: name,
    richtungName: 'Richtung Test',
    hours: { car: carHours, delivery: zeros, truck: zeros },
  };
}

describe('geh', () => {
  it('matches hand-computed values and is 0 on perfect fit', () => {
    expect(geh(100, 100)).toBe(0);
    // GEH(120, 100) = sqrt(2·400/220) ≈ 1.907
    expect(geh(120, 100)).toBeCloseTo(1.907, 3);
    // A 2× miss at volume is clearly >5: GEH(200, 100) ≈ 8.16
    expect(geh(200, 100)).toBeGreaterThan(5);
    expect(geh(0, 0)).toBe(0);
  });
});

describe('compare', () => {
  it('joins by station+direction, gates on observed volume', () => {
    const hoursObs = new Array(24).fill(0);
    hoursObs[8] = 500; // gated hour (≥ MIN_OBSERVED_VPH)
    hoursObs[3] = 5; // tiny-volume hour: informational only
    const hoursSim = new Array(24).fill(0);
    hoursSim[8] = 520; // GEH ≈ 0.88 → ok
    hoursSim[3] = 50; // wildly off but NOT gated

    const cmp = compare(
      { stations: [station('K1', hoursObs)] },
      { stations: [station('K1', hoursSim)] },
    );
    expect(cmp.gated).toBe(1);
    expect(cmp.gatedOk).toBe(1);
    const buckets = cmp.stations[0].buckets as Record<
      string,
      Array<{ geh: number }>
    >;
    expect(buckets.car[8].geh).toBeLessThan(5);
    expect(hoursObs[3]).toBeLessThan(MIN_OBSERVED_VPH);
  });

  it('fails loud on one-sided stations (no silent coverage loss)', () => {
    const z = new Array(24).fill(0);
    expect(() =>
      compare({ stations: [station('K1', z)] }, { stations: [] }),
    ).toThrow(/no simulated profile/);
    expect(() =>
      compare(
        { stations: [station('K1', z)] },
        { stations: [station('K1', z), station('K2', z)] },
      ),
    ).toThrow(/simulated-only/);
  });
});
