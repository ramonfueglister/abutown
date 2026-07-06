import { describe, expect, it } from 'vitest';
import {
  GKAT_LABELS, centroid, joinBauzone, joinGwr, lv95ToWgs84, pointInPolygon,
} from '../../scripts/geo/lib/enrich.mjs';

const square = (cx: number, cz: number, r: number): [number, number][] => [
  [cx - r, cz - r], [cx + r, cz - r], [cx + r, cz + r], [cx - r, cz + r], [cx - r, cz - r],
];

describe('lv95ToWgs84', () => {
  it('projection origin (Bern) is exact', () => {
    // E=2600000/N=1200000 is the LV95 projection centre: the polynomial
    // reduces to its constant terms → 7°26'19.076'' / 46°57'3.89''.
    const { lon, lat } = lv95ToWgs84(2600000, 1200000);
    expect(lon).toBeCloseTo(7.43864, 4);
    expect(lat).toBeCloseTo(46.95108, 4);
  });
  it('Rigi reference point (swisstopo worked example) within approximation error', () => {
    // Canonical vector from swisstopo "Formeln und Konstanten für die
    // Kartenprojektion" (Rigi): LV95 2679520.05/1212273.44
    // Formula yields: lon=8.485306671781535, lat=47.05671263483547 (~1m accuracy).
    const { lon, lat } = lv95ToWgs84(2679520.05, 1212273.44);
    expect(lon).toBeCloseTo(8.4853, 4);
    expect(lat).toBeCloseTo(47.0567, 4);
  });
});

describe('pointInPolygon', () => {
  const ring = square(0, 0, 10);
  it('inside / outside / concave-safe', () => {
    expect(pointInPolygon([0, 0], ring)).toBe(true);
    expect(pointInPolygon([11, 0], ring)).toBe(false);
    const lShape: [number, number][] = [[0,0],[10,0],[10,4],[4,4],[4,10],[0,10],[0,0]];
    expect(pointInPolygon([2, 8], lShape)).toBe(true);   // in the vertical arm
    expect(pointInPolygon([8, 8], lShape)).toBe(false);  // in the notch
  });
});

describe('centroid', () => {
  it('area centroid of an offset square', () => {
    expect(centroid(square(5, -3, 2))).toEqual([5, -3]);
  });
});

describe('joinBauzone', () => {
  const zones = [
    { ring: square(0, 0, 50), bauzone: 'Wohnzone W3', bauzoneCode: 'W3', zhCode: 'C1103' },
    { ring: square(200, 0, 50), bauzone: 'Gewerbezone 5.0', bauzoneCode: 'G5', zhCode: 'C1202' },
  ];
  it('centroid picks the containing zone', () => {
    expect(joinBauzone(square(10, 10, 5), zones)?.bauzoneCode).toBe('W3');
    expect(joinBauzone(square(210, 0, 5), zones)?.bauzoneCode).toBe('G5');
  });
  it('no containing zone → null', () => {
    expect(joinBauzone(square(1000, 1000, 5), zones)).toBeNull();
  });
});

describe('joinGwr', () => {
  const fp = square(0, 0, 10);
  it('single point inside', () => {
    const r = joinGwr(fp, [{ x: 1, z: 1, egid: 42, gkat: '1020', gklas: '1110' }]);
    expect(r).toEqual({ egid: 42, gwrCategory: GKAT_LABELS['1020'], gwrClass: '1110', egids: [42] });
  });
  it('dominant GKAT wins; tie → lowest EGID (deterministic)', () => {
    const r = joinGwr(fp, [
      { x: -1, z: 0, egid: 7, gkat: '1020', gklas: null },
      { x: 1, z: 0, egid: 5, gkat: '1020', gklas: null },
      { x: 0, z: 1, egid: 9, gkat: '1060', gklas: null },
    ]);
    expect(r?.gwrCategory).toBe(GKAT_LABELS['1020']); // 2× 1020 beats 1× 1060
    expect(r?.egid).toBe(5);                          // lowest EGID of the dominant class
    expect(r?.egids).toEqual([5, 7, 9]);              // sorted, all matches kept
  });
  it('no point inside → null', () => {
    expect(joinGwr(fp, [{ x: 100, z: 100, egid: 1, gkat: '1020', gklas: null }])).toBeNull();
  });
});
