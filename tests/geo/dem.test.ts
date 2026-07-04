// tests/geo/dem.test.ts
import { describe, expect, it } from 'vitest';
import { extractPatch, makeDemSampler, parseAAIGrid } from '../../scripts/geo/lib/dem.mjs';
import { ANCHOR, makeProjector } from '../../scripts/geo/lib/project.mjs';

// 3×3-Grid um den Anker, Zelle ≈ 0.0001° — Werte 100..108 zeilenweise (Nord→Süd)
const asc = [
  'ncols 3', 'nrows 3',
  `xllcorner ${ANCHOR.lon - 0.00015}`, `yllcorner ${ANCHOR.lat - 0.00015}`,
  'cellsize 0.0001', 'NODATA_value -9999',
  '100 101 102', '103 104 105', '106 107 108',
].join('\n');

describe('dem', () => {
  it('parses AAIGrid headers and data', () => {
    const g = parseAAIGrid(asc);
    expect(g.ncols).toBe(3);
    expect(g.data[4]).toBe(104); // Mitte
  });
  it('samples bilinearly at the anchor (grid center)', () => {
    const g = parseAAIGrid(asc);
    const s = makeDemSampler(g, makeProjector(ANCHOR));
    expect(s.heightAt(0, 0)).toBeCloseTo(104, 0);
  });
  it('extracts a row-major patch', () => {
    const g = parseAAIGrid(asc);
    const s = makeDemSampler(g, makeProjector(ANCHOR));
    const p = extractPatch(s, { originX: -10, originZ: -10, gridN: 2, cellSize: 20 });
    expect(p.length).toBe(4);
    expect(p[0]).toBeGreaterThan(99);
  });
  it('distinguishes north/south samples asymmetrically', () => {
    const g = parseAAIGrid(asc);
    const s = makeDemSampler(g, makeProjector(ANCHOR));
    // z=-10: ~10 m NORTH of anchor → samples north row (values ~100-102)
    // z=+10: ~10 m SOUTH of anchor → samples south row (values ~106-108)
    const northSample = s.heightAt(0, -10);
    const southSample = s.heightAt(0, +10);
    expect(northSample).toBeLessThan(southSample);
    expect(northSample).toBeCloseTo(101, 0); // north row center ≈ 101
    expect(southSample).toBeCloseTo(107, 0); // south row center ≈ 107
  });
});
