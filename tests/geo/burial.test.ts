// tests/geo/burial.test.ts
//
// Burial metric (spec §9): quantifies how far a planar road ribbon's edges
// sit off the actual terrain — the acceptance instrument for the terrain-
// grading pass (grading.mjs). At each 10 m station along a road's
// centreline, the cross-section deviation is |heightAt(edge) −
// heightAt(centre)| at both ±width/2 offsets; the ribbon is planar across
// its width (§5), so any real cross-slope under it reads as burial.
import { describe, expect, it } from 'vitest';
import { burialStats } from '../../scripts/geo/burial-metric.mjs';
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
