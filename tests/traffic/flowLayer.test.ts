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
    { id: 1, edge: 1, index: 0, lengthM: 500, pts: [[0, 0], [500, 0]] },
  ];
  const grid = CellGrid.build(LANES);

  it('is exactly 0 for a point inside a subscribed cell', () => {
    const subscribed = grid.cellsAround(0, 0, 1); // 3x3 around origin
    const fade = fadeFor(grid, subscribed, 0, 0);
    expect(fade).toBe(0);
  });

  it('ramps CONTINUOUSLY with distance from the subscribed boundary (linear over one CELL_SIZE_M)', () => {
    const subscribed = grid.cellsAround(0, 0, 0); // just the origin cell (rect [0,CS]×[0,CS])
    // The subscribed cell's east boundary is at x = CELL_SIZE_M; walking east
    // from it, fade must equal distance/CELL_SIZE_M — a continuous linear
    // ramp, NOT a discrete per-ring step.
    expect(fadeFor(grid, subscribed, CELL_SIZE_M + CELL_SIZE_M * 0.25, 64)).toBeCloseTo(0.25, 6);
    expect(fadeFor(grid, subscribed, CELL_SIZE_M + CELL_SIZE_M * 0.5, 64)).toBeCloseTo(0.5, 6);
    expect(fadeFor(grid, subscribed, CELL_SIZE_M + CELL_SIZE_M * 0.75, 64)).toBeCloseTo(0.75, 6);
  });

  it('increases monotonically with distance outside the subscribed set', () => {
    const subscribed = grid.cellsAround(0, 0, 0);
    let prev = -1;
    for (let d = 0; d <= CELL_SIZE_M * 1.5; d += CELL_SIZE_M / 8) {
      const fade = fadeFor(grid, subscribed, CELL_SIZE_M + d, 64);
      expect(fade).toBeGreaterThanOrEqual(prev);
      prev = fade;
    }
  });

  it('reaches 1 at exactly one CELL_SIZE_M from the boundary and stays 1 beyond', () => {
    const subscribed = grid.cellsAround(0, 0, 0);
    expect(fadeFor(grid, subscribed, CELL_SIZE_M * 2, 64)).toBe(1);
    expect(fadeFor(grid, subscribed, CELL_SIZE_M * 10, 0)).toBe(1);
  });
});
