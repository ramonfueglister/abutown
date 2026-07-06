// Width-aware lane offsets (root-cause fix for FIX D1): the traffic kernel
// must fit its lanes into the REAL OSM carriage width instead of assuming
// 3.0 m lanes everywhere and then widening the rendered world to match.
import { describe, expect, it } from 'vitest';
// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore — plain-ESM bake lib without type declarations
import { LANE_WIDTH, laneOffsets } from '../../scripts/geo/lib/trafficnet.mjs';

describe('laneOffsets', () => {
  it('wide two-way street keeps the classic 3.0 m lanes (offset +1.5)', () => {
    expect(laneOffsets({ laneCount: 1, reverseLaneCount: 1, widthM: 6.8 })).toEqual([1.5]);
  });

  it('narrow 5 m two-way street narrows lanes to 2.5 m (offset +1.25)', () => {
    expect(laneOffsets({ laneCount: 1, reverseLaneCount: 1, widthM: 5 })).toEqual([1.25]);
  });

  it('4 m service road: lanes floor at 1.8 m so cars never sit on the centreline', () => {
    expect(laneOffsets({ laneCount: 1, reverseLaneCount: 1, widthM: 3.2 })).toEqual([0.9]);
  });

  it('one-way single lane drives ON the centreline', () => {
    expect(laneOffsets({ laneCount: 1, reverseLaneCount: 0, widthM: 4 })).toEqual([0]);
  });

  it('one-way double lane straddles the centreline symmetrically', () => {
    const [a, b] = laneOffsets({ laneCount: 2, reverseLaneCount: 0, widthM: 6 });
    expect(a).toBeCloseTo(-1.5, 5);
    expect(b).toBeCloseTo(1.5, 5);
  });

  it('two-way multi-lane: forward lanes stack to the right of the centreline', () => {
    const offs = laneOffsets({ laneCount: 2, reverseLaneCount: 2, widthM: 12 });
    expect(offs).toEqual([1.5, 4.5]);
  });

  it('missing width falls back to full-width lanes (classic behaviour)', () => {
    expect(laneOffsets({ laneCount: 1, reverseLaneCount: 1, widthM: undefined })).toEqual([
      LANE_WIDTH / 2,
    ]);
  });

  it('lanes never exceed LANE_WIDTH even on very wide roads', () => {
    expect(laneOffsets({ laneCount: 1, reverseLaneCount: 1, widthM: 20 })).toEqual([
      LANE_WIDTH / 2,
    ]);
  });
});
