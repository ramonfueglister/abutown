// tests/geo/burial.test.ts
//
// Burial metric (spec §9): quantifies how far a planar road ribbon's edges
// sit off the actual terrain — the acceptance instrument for the terrain-
// grading pass (grading.mjs). At each 10 m station along a road's
// centreline, the cross-section deviation is |heightAt(edge) −
// heightAt(centre)| at both ±width/2 offsets; the ribbon is planar across
// its width (§5), so any real cross-slope under it reads as burial.
import { describe, expect, it } from 'vitest';
import { burialStats, burialStatsV2, burialStatsV3 } from '../../scripts/geo/burial-metric.mjs';
import { gradeDem } from '../../scripts/geo/lib/grading.mjs';

/** Bilinear sampler over a flat local-metre grid, same convention as grading.mjs. */
function makeSampler(n: number, cell: number, h: (x: number, z: number) => number) {
  const data = new Float64Array(n * n);
  for (let j = 0; j < n; j++) for (let i = 0; i < n; i++) data[j * n + i] = h(i * cell, j * cell);
  const grid = { ncols: n, nrows: n, cellsize: cell, xll: 0, yll: 0, data };
  const heightAt = (x: number, z: number) => {
    const col = (x - grid.xll) / grid.cellsize;
    const row = (z - grid.yll) / grid.cellsize;
    const c0 = Math.max(0, Math.min(grid.ncols - 2, Math.floor(col)));
    const r0 = Math.max(0, Math.min(grid.nrows - 2, Math.floor(row)));
    const fc = Math.max(0, Math.min(1, col - c0));
    const fr = Math.max(0, Math.min(1, row - r0));
    const at = (r: number, c: number) => grid.data[r * grid.ncols + c];
    return at(r0, c0) * (1 - fc) * (1 - fr) + at(r0, c0 + 1) * fc * (1 - fr) +
      at(r0 + 1, c0) * (1 - fc) * fr + at(r0 + 1, c0 + 1) * fc * fr;
  };
  return { grid, heightAt };
}

describe('burialStats', () => {
  it('reports the analytic deviation on a constant cross-slope grid', () => {
    // z-running road at x=30, constant cross-slope dh/dz = 0.5 (height = z*0.5).
    // At width=8 (halfWidth=4), the edges sit at z=30±4 relative to the
    // centreline offset direction — but the road runs ALONG z, so its
    // perpendicular is the x axis: edges are at x=30±4, z=const per station.
    // Use an x-running road instead so the perpendicular (z-offset) actually
    // crosses the slope.
    const { heightAt } = makeSampler(60, 1, (x, z) => z * 0.5);
    const road = { class: 'residential', width: 8, pts: [[5, 30], [55, 30]] };
    const stats = burialStats([road], [8], heightAt, 10);

    // Perpendicular to an x-running road is the z axis; edges at z=30±4.
    // deviation = |heightAt(centre_x, 30+4) - heightAt(centre_x, 30)|
    //           = |(34)*0.5 - 30*0.5| = 2.0 m on both sides.
    expect(stats.maxM).toBeCloseTo(2.0, 5);
    expect(stats.p99M).toBeCloseTo(2.0, 5);
    expect(stats.pctOver30cm).toBeCloseTo(100, 5);
    expect(stats.offenders.length).toBeGreaterThan(0);
    expect(stats.offenders[0].devM).toBeCloseTo(2.0, 5);
    expect(stats.offenders[0].class).toBe('residential');
  });

  it('reports zero deviation on flat terrain', () => {
    const { heightAt } = makeSampler(60, 1, () => 42);
    const road = { class: 'residential', width: 6, pts: [[5, 30], [55, 30]] };
    const stats = burialStats([road], [6], heightAt, 10);
    expect(stats.maxM).toBeCloseTo(0, 8);
    expect(stats.p99M).toBeCloseTo(0, 8);
    expect(stats.pctOver30cm).toBe(0);
    expect(stats.offenders.length).toBe(0);
  });

  it('folded-in longitudinal regression: after gradeDem, a bumpy 100 m road stays under the 0.3 m budget', () => {
    // Bumpy synthetic grid: sinusoidal terrain along both axes so the raw
    // DEM has real short-wavelength bumps a chord/ribbon would bury into.
    const n = 140;
    const cell = 1;
    const bumpy = (x: number, z: number) =>
      2.5 * Math.sin(x / 6) + 1.5 * Math.cos(z / 9) + 0.4 * Math.sin((x + z) / 3);
    const data = new Float64Array(n * n);
    for (let j = 0; j < n; j++) for (let i = 0; i < n; i++) data[j * n + i] = bumpy(i * cell, j * cell);
    const dem = { ncols: n, nrows: n, cellsize: cell, xll: 0, yll: 0, data };

    const way = {
      pts: [[10, 70], [110, 70]], // 100 m road along x
      halfWidthM: 4,
      blendM: 8,
      windowM: 40,
      maxGrade: 0.12,
      kind: 'road' as const,
    };
    gradeDem(dem, [way], { waterRings: [] });

    const heightAt = (x: number, z: number) => {
      const col = (x - dem.xll) / dem.cellsize;
      const row = (z - dem.yll) / dem.cellsize;
      const c0 = Math.max(0, Math.min(dem.ncols - 2, Math.floor(col)));
      const r0 = Math.max(0, Math.min(dem.nrows - 2, Math.floor(row)));
      const fc = Math.max(0, Math.min(1, col - c0));
      const fr = Math.max(0, Math.min(1, row - r0));
      const at = (r: number, c: number) => dem.data[r * dem.ncols + c];
      return at(r0, c0) * (1 - fc) * (1 - fr) + at(r0, c0 + 1) * fc * (1 - fr) +
        at(r0 + 1, c0) * (1 - fc) * fr + at(r0 + 1, c0 + 1) * fc * fr;
    };

    const road = { class: 'residential', width: 8, pts: [[10, 70], [110, 70]] };
    const stats = burialStats([road], [8], heightAt, 10);
    expect(stats.maxM).toBeLessThan(0.3);
  });

  it('is deterministic across repeated calls', () => {
    const { heightAt } = makeSampler(60, 1, (x, z) => Math.sin(x / 5) + z * 0.1);
    const road = { class: 'residential', width: 8, pts: [[5, 8], [55, 45]] };
    const a = burialStats([road], [8], heightAt, 10);
    const b = burialStats([road], [8], heightAt, 10);
    expect(a).toEqual(b);
  });

  it('offenders are sorted descending and capped at 10', () => {
    // Many stations across several roads with varying slope so more than
    // 10 stations exceed 30 cm; check the cap and the ordering.
    const { heightAt } = makeSampler(200, 1, (x, z) => z * 0.5);
    const roads = Array.from({ length: 5 }, (_, i) => ({
      class: 'residential',
      width: 8,
      pts: [[5, 20 + i * 20], [195, 20 + i * 20]],
    }));
    const widths = roads.map((r) => r.width);
    const stats = burialStats(roads, widths, heightAt, 10);
    expect(stats.offenders.length).toBeLessThanOrEqual(10);
    for (let i = 1; i < stats.offenders.length; i++) {
      expect(stats.offenders[i - 1].devM).toBeGreaterThanOrEqual(stats.offenders[i].devM);
    }
  });
});

describe('burialStatsV2 (spec §9 metric v2 — terrain poke-through)', () => {
  it('reports zero poke-through when tile ground matches the profile exactly', () => {
    const { heightAt } = makeSampler(60, 1, (x) => x * 0.1); // tile ground rises with x
    const road = {
      class: 'residential',
      pts: [[0, 30], [50, 30]],
      profile: { stepM: 10, ys: [0, 1, 2, 3, 4, 5, 5] }, // matches x*0.1 at each 10 m station
    };
    const stats = burialStatsV2([road], heightAt, 10);
    expect(stats.maxM).toBeCloseTo(0, 6);
    expect(stats.p99M).toBeCloseTo(0, 6);
    expect(stats.offenders.length).toBe(0);
  });

  it('reports positive poke-through when tile ground sits above the profile (terrain pierces the road)', () => {
    const { heightAt } = makeSampler(60, 1, (x) => x * 0.1 + 0.2); // tile 0.2 m higher everywhere
    const road = {
      class: 'residential',
      pts: [[0, 30], [50, 30]],
      profile: { stepM: 10, ys: [0, 1, 2, 3, 4, 5, 5] },
    };
    const stats = burialStatsV2([road], heightAt, 10);
    expect(stats.maxM).toBeCloseTo(0.2, 6);
    expect(stats.p99M).toBeCloseTo(0.2, 6);
    expect(stats.offenders.length).toBeGreaterThan(0);
  });

  it('clamps negative poke-through (terrain below road, no piercing) toward the report but not the pass/fail max', () => {
    // tileY below profileY everywhere -> the terrain does not pierce the road.
    // maxM/p99M should reflect max(0, tileY - profileY) style poke-through only.
    const { heightAt } = makeSampler(60, 1, (x) => x * 0.1 - 5); // tile far below profile
    const road = {
      class: 'residential',
      pts: [[0, 30], [50, 30]],
      profile: { stepM: 10, ys: [0, 1, 2, 3, 4, 5, 5] },
    };
    const stats = burialStatsV2([road], heightAt, 10);
    expect(stats.maxM).toBeCloseTo(0, 6);
    expect(stats.p99M).toBeCloseTo(0, 6);
  });

  it('throws when a road lacks a baked profile', () => {
    const { heightAt } = makeSampler(60, 1, () => 0);
    const road = { class: 'residential', pts: [[0, 30], [50, 30]] };
    expect(() => burialStatsV2([road], heightAt, 10)).toThrow();
  });

  it('is deterministic across repeated calls', () => {
    const { heightAt } = makeSampler(60, 1, (x, z) => Math.sin(x / 5) + z * 0.1);
    const road = {
      class: 'residential',
      pts: [[5, 8], [55, 45]],
      profile: { stepM: 10, ys: [0, 0.5, 1, 1.5, 2, 2.5, 3, 3] },
    };
    const a = burialStatsV2([road], heightAt, 10);
    const b = burialStatsV2([road], heightAt, 10);
    expect(a).toEqual(b);
  });
});

describe('burialStatsV3 (spec §9 metric v3 — non-vacuous shoulder-annulus truth)', () => {
  // A straight x-running road at z=30. maskHW=3 (ribbon), gradeHW=4.5 (ribbon +
  // 1.5 m shoulder); the annulus lives at |z−30| ∈ [3.1, 4.5+blend]. profile is
  // flat at 0 so poke-through = tileY there.
  const road = {
    class: 'residential',
    pts: [[0, 30], [60, 30]],
    profile: { stepM: 10, ys: [0, 0, 0, 0, 0, 0, 0] },
  };
  const opts = (extra: object = {}) => ({
    stepM: 10, maskHalfWidths: [3], gradeHalfWidths: [4.5], blendM: 3, skirtDropM: 1.5, ...extra,
  });

  it('samples the shoulder annulus (NOT the centreline) and reports 100% centreline coverage', () => {
    // Terrain flat AT the profile everywhere → no poke-through anywhere; but the
    // annulus IS sampled (outsideCount > 0), proving the measured set is not empty.
    const { heightAt } = makeSampler(80, 1, () => 0);
    const covers = (_x: number, z: number) => Math.abs(z - 30) <= 3; // ribbon footprint only
    const stats = burialStatsV3([road], heightAt, covers, opts());
    expect(stats.coveragePct).toBeCloseTo(100, 6); // every centreline station in mask
    expect(stats.outsideCount).toBeGreaterThan(0); // NON-VACUOUS: annulus measured
    expect(stats.maxM).toBeCloseTo(0, 6);
    expect(stats.skirtReachPass).toBe(true);
  });

  it('MUTATION: a shoulder-annulus breach FAILS the budget (proves non-vacuity)', () => {
    // The mask still covers only the ribbon (|z−30| ≤ 3), but the graded shoulder
    // terrain pokes 0.3 m above the profile. The OLD centreline-only v3 would see
    // this discarded and pass; the new annulus sampler must MEASURE it and exceed
    // the 0.10 m budget.
    const { heightAt } = makeSampler(80, 1, (_x, z) => (Math.abs(z - 30) > 3 ? 0.3 : 0));
    const covers = (_x: number, z: number) => Math.abs(z - 30) <= 3;
    const stats = burialStatsV3([road], heightAt, covers, opts());
    expect(stats.outsideCount).toBeGreaterThan(0);
    expect(stats.maxM).toBeCloseTo(0.3, 6);
    const budgetPass = stats.maxM < 0.10 && stats.p99M <= 0.05;
    expect(budgetPass).toBe(false); // the breach is caught
  });

  it('flags uncovered centreline stations and reports coverage < 100%', () => {
    const { heightAt } = makeSampler(80, 1, () => 0);
    // mask covers the ribbon only for x < 25 → the far half of the centreline is uncovered
    const covers = (x: number, z: number) => x < 25 && Math.abs(z - 30) <= 3;
    const stats = burialStatsV3([road], heightAt, covers, opts());
    expect(stats.coveragePct).toBeLessThan(100);
    expect(stats.coveragePct).toBeGreaterThan(0);
  });

  it('reports skirt-reach: profile above discarded tile inside a ribbon→mask gap', () => {
    // Mask over-covers past the ribbon (|z−30| ≤ 4, wider than maskHW=3), so the
    // annulus band z∈[3.1,4] is INSIDE the mask (discarded). profile sits 2 m
    // above the tile there → the skirt must drop 2 m > 1.5 m → skirt-reach FAILS.
    const { heightAt } = makeSampler(80, 1, () => -2); // tile 2 m below profile(0)
    const covers = (_x: number, z: number) => Math.abs(z - 30) <= 4;
    const stats = burialStatsV3([road], heightAt, covers, opts());
    expect(stats.skirtReachM).toBeCloseTo(2.0, 6);
    expect(stats.skirtReachPass).toBe(false);
  });

  it('is deterministic across repeated calls', () => {
    const { heightAt } = makeSampler(80, 1, (x, z) => Math.sin(x / 5) + z * 0.01);
    const covers = (_x: number, z: number) => Math.abs(z - 30) <= 3;
    const a = burialStatsV3([road], heightAt, covers, opts());
    const b = burialStatsV3([road], heightAt, covers, opts());
    expect(a).toEqual(b);
  });

  it('throws when a road lacks a baked profile', () => {
    const { heightAt } = makeSampler(80, 1, () => 0);
    expect(() => burialStatsV3([{ class: 'x', pts: [[0, 0], [60, 0]] }], heightAt, () => true, { maskHalfWidths: [3], gradeHalfWidths: [4.5] })).toThrow();
  });

  it('throws when half-width arrays are missing (guards vacuous centreline-only use)', () => {
    const { heightAt } = makeSampler(80, 1, () => 0);
    // @ts-expect-error — passing a bare stepM number is no longer allowed
    expect(() => burialStatsV3([road], heightAt, () => true, 10)).toThrow();
  });
});
