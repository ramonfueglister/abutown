// tests/geo/grading.test.ts
//
// Grading operates on a *local-meter* grid — the same row-major convention
// used by tiles.mjs's extracted patches (see encodeTile/extractPatch):
// data[j*ncols+i], i steps with world x (east), j steps with world z
// (south), cell (0,0)'s centre sits at (xll, yll) in local metres. This is
// NOT the raw parseAAIGrid geographic grid (which is in lon/lat degrees
// with row 0 = north) — grading is wired to a local grid at bake time in a
// later task. Field names are kept aligned with parseAAIGrid's naming
// (ncols/nrows/xll/yll) but celldx/celldy collapse to a single isotropic
// `cellsize` here.
import { describe, expect, it } from 'vitest';
import { gradeDem, makeCorridorMask, smoothProfile } from '../../scripts/geo/lib/grading.mjs';

function flatDem(n: number, cell: number, h: (x: number, z: number) => number) {
  const data = new Float64Array(n * n);
  for (let j = 0; j < n; j++) for (let i = 0; i < n; i++) data[j * n + i] = h(i * cell, j * cell);
  return { ncols: n, nrows: n, cellsize: cell, xll: 0, yll: 0, data };
}

describe('smoothProfile', () => {
  it('clamps grade to the limit in both directions', () => {
    const raw = [0, 0, 10, 10]; // 10 m jump over one 2 m step = 500 %
    const out = smoothProfile(raw, 2, 4, 0.12);
    for (let i = 1; i < out.length; i++) expect(Math.abs(out[i] - out[i - 1])).toBeLessThanOrEqual(0.24 + 1e-9);
  });
  it('is deterministic', () => {
    const raw = [3, 1, 4, 1, 5, 9, 2, 6];
    expect(smoothProfile(raw, 2, 4, 0.12)).toEqual(smoothProfile(raw, 2, 4, 0.12));
  });
});

describe('grid <-> world mapping', () => {
  it('a graded cell lands where the way runs in world coords', () => {
    // Way runs along x at world z=30 across a cross-slope that varies with
    // z; grid row for z=30 (cellsize=1, yll=0) must be row index 30 — no
    // north/south flip for the local grid. A cell just off the centreline
    // (z=32, inside the corridor) should be pulled toward the levelled
    // profile, away from its raw z*0.5=16 height.
    const dem = flatDem(60, 1, (x, z) => z * 0.5);
    const way = { pts: [[5, 30], [55, 30]], halfWidthM: 4, blendM: 8, windowM: 40, maxGrade: 0.12, kind: 'road' as const };
    const before = dem.data[32 * 60 + 30];
    gradeDem(dem, [way], { waterRings: [] });
    const after = dem.data[32 * 60 + 30];
    expect(after).not.toBeCloseTo(before, 5); // the targeted row actually changed
    // A row far from the corridor (z=50) must stay at its original height.
    expect(dem.data[50 * 60 + 30]).toBeCloseTo(25, 5);
  });
});

describe('gradeDem', () => {
  it('levels the corridor cross-slope and blends back over blendM', () => {
    const dem = flatDem(60, 1, (x, z) => z * 0.5); // 50 % cross-slope
    const way = { pts: [[5, 30], [55, 30]], halfWidthM: 4, blendM: 8, windowM: 40, maxGrade: 0.12, kind: 'road' as const };
    gradeDem(dem, [way], { waterRings: [] });
    const at = (x: number, z: number) => dem.data[Math.round(z) * 60 + Math.round(x)];
    expect(Math.abs(at(30, 28) - at(30, 32))).toBeLessThan(0.05); // level across corridor
    expect(at(30, 50)).toBeCloseTo(25, 5); // untouched far field
  });

  it('rail overrides road inside the rail corridor', () => {
    // Pre-shape terrain so road profile != rail profile at the crossing:
    // height varies only with x, so the (x-running) rail sees a real slope
    // to smooth while the (z-running) road sees none along its own line.
    function mk() {
      const dem = flatDem(60, 1, () => 0);
      for (let j = 0; j < 60; j++) for (let i = 0; i < 60; i++) dem.data[j * 60 + i] = i * 0.1;
      return dem;
    }
    const road = { pts: [[30, 5], [30, 55]], halfWidthM: 4, blendM: 8, windowM: 40, maxGrade: 0.12, kind: 'road' as const };
    const rail = { pts: [[5, 30], [55, 30]], halfWidthM: 3, blendM: 8, windowM: 200, maxGrade: 0.025, kind: 'rail' as const };

    const roadOnly = mk();
    gradeDem(roadOnly, [road], { waterRings: [] });
    const roadPlusRail = mk();
    gradeDem(roadPlusRail, [road, rail], { waterRings: [] });

    // At the crossing (row z=30), just past the road's own blend edge
    // (column x=35), road-only grading has already relaxed back toward
    // the raw slope — but the rail pass extends its own (much flatter)
    // profile over that same cell, so the combined result must differ
    // from the road-only result there. That's the observable override.
    const roadOnlyVal = roadOnly.data[30 * 60 + 35];
    const overriddenVal = roadPlusRail.data[30 * 60 + 35];
    expect(overriddenVal).not.toBeCloseTo(roadOnlyVal, 2);

    // Rail's own smoothing still respects its (tight) 2.5% grade limit
    // between two points inside its corridor along the crossing.
    const railAt30 = roadPlusRail.data[30 * 60 + 30];
    const railAt29 = roadPlusRail.data[30 * 60 + 29];
    expect(Math.abs(railAt30 - railAt29)).toBeLessThanOrEqual(0.025 * 2 + 1e-6);
  });

  it('never touches water cells and reports the bridge site', () => {
    const dem = flatDem(60, 1, () => 7);
    const way = { pts: [[5, 30], [55, 30]], halfWidthM: 4, blendM: 8, windowM: 40, maxGrade: 0.12, kind: 'road' as const };
    const water = [[[20, 20], [40, 20], [40, 40], [20, 40]]]; // square river
    const report = gradeDem(dem, [way], { waterRings: water });
    expect(dem.data[30 * 60 + 30]).toBe(7); // water cell untouched
    expect(report.waterSkippedCells).toBeGreaterThanOrEqual(3);
    expect(report.bridgeSites.length).toBe(1);
    expect(report.bridgeSites[0].kind).toBe('road');
  });

  it('reports cellsChanged and originDeltaM', () => {
    const dem = flatDem(60, 1, (x, z) => z * 0.5);
    const way = { pts: [[5, 30], [55, 30]], halfWidthM: 4, blendM: 8, windowM: 40, maxGrade: 0.12, kind: 'road' as const };
    const report = gradeDem(dem, [way], { waterRings: [] });
    expect(report.cellsChanged).toBeGreaterThan(0);
    expect(typeof report.originDeltaM).toBe('number');
    expect(report.originDeltaM).toBeGreaterThanOrEqual(0);
  });

  it('is deterministic across repeated runs on identical input', () => {
    const demA = flatDem(60, 1, (x, z) => z * 0.5);
    const demB = flatDem(60, 1, (x, z) => z * 0.5);
    const way = { pts: [[5, 30], [55, 30]], halfWidthM: 4, blendM: 8, windowM: 40, maxGrade: 0.12, kind: 'road' as const };
    gradeDem(demA, [way], { waterRings: [] });
    gradeDem(demB, [way], { waterRings: [] });
    expect(Array.from(demA.data)).toEqual(Array.from(demB.data));
  });

  it('throws on invalid way kind instead of silently defaulting', () => {
    const dem = flatDem(10, 1, () => 0);
    const bad = { pts: [[1, 1], [8, 8]], halfWidthM: 2, blendM: 4, windowM: 10, maxGrade: 0.1, kind: 'path' as unknown as 'road' };
    expect(() => gradeDem(dem, [bad], { waterRings: [] })).toThrow();
  });
});

describe('makeCorridorMask', () => {
  it('covers the carriageway, not the far field', () => {
    const mask = makeCorridorMask([{ pts: [[0, 0], [100, 0]], halfWidthM: 5 }]);
    expect(mask(50, 2)).toBe(true);
    expect(mask(50, 30)).toBe(false);
  });
});
