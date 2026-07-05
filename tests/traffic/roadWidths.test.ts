// tests/traffic/roadWidths.test.ts
//
// FIX D1 — carriage render width fits the traffic lane pairs. Pure geometric
// matching (correctRoadWidths), exercised on hand-built nets so the width rule
// and the parallel/distance match gates are pinned without the real asset.

import { describe, expect, it } from 'vitest';
import {
  correctRoadWidths,
  LANE_W,
  SHOULDER_M,
  MATCH_DIST_M,
} from '../../src/diorama/traffic/roadWidths';
import type { RoadPath } from '../../src/diorama/ksw/geo/geoData';

// A minimal net doc: one two-way street between nodes 1 and 2 running along +x
// at z=0, each direction carrying `laneCount` lanes. The lane polyline sits on
// the centreline (offsets don't matter for the match test).
function net(laneCount: number, opts: { z?: number } = {}) {
  const z = opts.z ?? 0;
  const pts = [
    [0, z],
    [50, z],
    [100, z],
  ];
  return {
    edges: [
      { id: 0, from: 1, to: 2, laneCount, lanes: [0] },
      { id: 1, from: 2, to: 1, laneCount, lanes: [1] },
    ],
    lanes: [
      { id: 0, edge: 0, pts },
      { id: 1, edge: 1, pts: [...pts].reverse() },
    ],
  };
}

function road(width: number, pts: number[][], cls = 'residential'): RoadPath {
  return { class: cls, width, pts };
}

describe('correctRoadWidths', () => {
  it('floors a two-way street to lanes_both_directions × LANE_W + shoulder', () => {
    // A narrow OSM residential ribbon lying on the matching traffic edge.
    const roads = [road(4, [[0, 0], [50, 0], [100, 0]])];
    const [w] = correctRoadWidths(roads, net(1));
    // both directions → 2 lanes total → 2×3.0 + 0.8 = 6.8
    expect(w).toBeCloseTo(2 * LANE_W + SHOULDER_M, 6);
  });

  it('sums laneCount across both directed edges (multi-lane arterials)', () => {
    const roads = [road(6, [[0, 0], [50, 0], [100, 0]])];
    const [w] = correctRoadWidths(roads, net(2)); // 2+2 = 4 lanes
    expect(w).toBeCloseTo(4 * LANE_W + SHOULDER_M, 6); // 12.8
  });

  it('never SHRINKS a ribbon already wider than the lane pairs', () => {
    const roads = [road(16, [[0, 0], [50, 0], [100, 0]])];
    const [w] = correctRoadWidths(roads, net(1)); // floor would be 6.8
    expect(w).toBe(16);
  });

  it('leaves roads with no nearby traffic edge at their OSM width', () => {
    // Far away from the net (net is along z=0; this road is at z=500).
    const roads = [road(3.2, [[0, 500], [50, 500], [100, 500]])];
    const [w] = correctRoadWidths(roads, net(1));
    expect(w).toBe(3.2);
  });

  it('rejects a perpendicular crossing street (parallel-tangent gate)', () => {
    // A street running along +z crossing the +x traffic edge at the origin.
    // Its midpoint is ON the edge but its tangent is perpendicular → no match.
    const roads = [road(3.2, [[50, -40], [50, 0], [50, 40]])];
    const [w] = correctRoadWidths(roads, net(1));
    expect(w).toBe(3.2);
  });

  it('does not match beyond MATCH_DIST_M', () => {
    // Parallel street offset laterally just past the match radius.
    const off = MATCH_DIST_M + 1;
    const roads = [road(3.2, [[0, off], [50, off], [100, off]])];
    const [w] = correctRoadWidths(roads, net(1));
    expect(w).toBe(3.2);
    // ...but a street just inside the radius DOES match.
    const inside = MATCH_DIST_M - 1;
    const roads2 = [road(3.2, [[0, inside], [50, inside], [100, inside]])];
    const [w2] = correctRoadWidths(roads2, net(1));
    expect(w2).toBeCloseTo(2 * LANE_W + SHOULDER_M, 6);
  });

  it('is deterministic — identical inputs give identical widths', () => {
    const roads = [road(4, [[0, 0], [50, 0], [100, 0]])];
    const a = correctRoadWidths(roads, net(1));
    const b = correctRoadWidths(roads, net(1));
    expect(a).toEqual(b);
  });

  it('returns one width per input road, aligned by index', () => {
    const roads = [
      road(4, [[0, 0], [100, 0]]),
      road(3.2, [[0, 500], [100, 500]]),
    ];
    const out = correctRoadWidths(roads, net(1));
    expect(out).toHaveLength(2);
    expect(out[0]).toBeCloseTo(2 * LANE_W + SHOULDER_M, 6); // matched
    expect(out[1]).toBe(3.2); // unmatched
  });
});
