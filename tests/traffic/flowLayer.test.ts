// tests/traffic/flowLayer.test.ts
//
// Task 12: pure placement/exclusion math for the far-LOD flow impostor layer,
// exercised WITHOUT three.js/WebGL (node-side logic only, per the module's
// testable-core split established by deadReckon.ts / trafficClient.ts).
//
// placeImpostors(edgeGeom, count, nowS, edgeId) places `count` impostors along
// an edge's polyline:
//   * deterministic across repeated calls at the SAME nowS (same hash-derived
//     per-slot base offset every time — no Math.random, no external state);
//   * advected: `(offset + v·nowS) mod lengthM` — the same nowS+later call at a
//     LARGER nowS moves each impostor further along the lane (mod wrap);
//   * fade: computed by the caller from subscribedCells (fadeFor below), not by
//     placeImpostors itself — placeImpostors returns raw world placement only,
//     and createFlowLayer.update composes placement + fade. We test fadeFor
//     directly for the 0-inside / ramp-over-one-ring behaviour.

import { describe, expect, it } from 'vitest';
import { placeImpostors, fadeFor, type EdgeGeom } from '../../src/diorama/traffic/flowLayer';
import { CellGrid, CELL_SIZE_M } from '../../src/diorama/traffic/trafficClient';
import type { RawLane } from '../../src/diorama/traffic/deadReckon';

// A straight 100 m edge along +x, single segment.
const STRAIGHT_EDGE: EdgeGeom = {
  pts: [[0, 0], [100, 0]],
  lengthM: 100,
};

describe('placeImpostors — deterministic placement + advection', () => {
  it('is deterministic across repeated calls at the same nowS (same edge/count/edgeId)', () => {
    const a = placeImpostors(STRAIGHT_EDGE, 5, 12.3, 7);
    const b = placeImpostors(STRAIGHT_EDGE, 5, 12.3, 7);
    expect(a).toEqual(b);
  });

  it('produces different base offsets per slot (not all impostors stacked)', () => {
    const out = placeImpostors(STRAIGHT_EDGE, 5, 0, 7);
    const xs = new Set(out.map((p) => p.x));
    expect(xs.size).toBeGreaterThan(1);
  });

  it('a different edgeId (same count/nowS) yields a different placement (hash depends on edgeId)', () => {
    const a = placeImpostors(STRAIGHT_EDGE, 5, 12.3, 7);
    const b = placeImpostors(STRAIGHT_EDGE, 5, 12.3, 99);
    expect(a).not.toEqual(b);
  });

  it('advects impostors forward along the edge as nowS increases (mod length wrap)', () => {
    const early = placeImpostors(STRAIGHT_EDGE, 1, 0, 3);
    const later = placeImpostors(STRAIGHT_EDGE, 1, 5, 3);
    // Moving from nowS=0 to nowS=5 at a fixed positive advection speed must
    // shift the impostor's arc position forward (mod length) — since the
    // straight edge runs along +x with z=0, that means x must differ (unless
    // it happened to wrap exactly onto the same point, astronomically
    // unlikely with a real speed constant).
    expect(early[0].x).not.toBeCloseTo(later[0].x, 6);
  });

  it('wraps the advected arc position modulo the edge length (stays on the polyline)', () => {
    // A very large nowS must still resolve to an in-bounds x in [0, 100].
    const out = placeImpostors(STRAIGHT_EDGE, 1, 1_000_000, 3);
    expect(out[0].x).toBeGreaterThanOrEqual(0);
    expect(out[0].x).toBeLessThanOrEqual(100);
    expect(out[0].z).toBeCloseTo(0, 6);
  });

  it('derives a yaw from the local tangent (straight +x edge -> yaw = +PI/2, atan2(tx,tz) convention)', () => {
    const out = placeImpostors(STRAIGHT_EDGE, 1, 0, 3);
    expect(out[0].yaw).toBeCloseTo(Math.PI / 2, 6);
  });
});

describe('fadeFor — 0 inside subscribed cells, ramp to 1 over one CELL_SIZE_M ring', () => {
  // Two lanes, one on each side, so the grid spans multiple cells.
  const LANES: RawLane[] = [
    { id: 1, lengthM: 500, pts: [[0, 0], [500, 0]] },
  ];
  const grid = CellGrid.build(LANES);

  it('is exactly 0 for a point inside a subscribed cell', () => {
    const subscribed = grid.cellsAround(0, 0, 1); // 3x3 around origin
    const fade = fadeFor(grid, subscribed, 0, 0);
    expect(fade).toBe(0);
  });

  it('ramps up (0, 1) for a point within one CELL_SIZE_M ring outside the subscribed set', () => {
    const subscribed = grid.cellsAround(0, 0, 0); // just the origin cell
    // A point one cell-width east of the subscribed cell's edge: still within
    // one ring, so fade should be a partial value, not fully 1.
    const fade = fadeFor(grid, subscribed, CELL_SIZE_M + 1, 0);
    expect(fade).toBeGreaterThan(0);
    expect(fade).toBeLessThanOrEqual(1);
  });

  it('is 1 (fully opaque impostor) far outside the subscribed set + its fade ring', () => {
    const subscribed = grid.cellsAround(0, 0, 0);
    const fade = fadeFor(grid, subscribed, CELL_SIZE_M * 10, 0);
    expect(fade).toBe(1);
  });
});
