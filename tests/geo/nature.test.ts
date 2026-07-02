// tests/geo/nature.test.ts
import { describe, expect, it } from 'vitest';
import { ANCHOR, makeProjector } from '../../scripts/geo/lib/project.mjs';
import { transformNature } from '../../scripts/geo/lib/transform.mjs';

const lonAt = (m: number) => ANCHOR.lon + m / (111320 * Math.cos((ANCHOR.lat * Math.PI) / 180));
const latAt = (m: number) => ANCHOR.lat + m / 111132;
const geomRect = (x0: number, x1: number, z0: number, z1: number) => [
  { lon: lonAt(x0), lat: latAt(-z0) }, { lon: lonAt(x1), lat: latAt(-z0) },
  { lon: lonAt(x1), lat: latAt(-z1) }, { lon: lonAt(x0), lat: latAt(-z1) },
  { lon: lonAt(x0), lat: latAt(-z0) },
];

describe('transformNature', () => {
  const osmNature = {
    elements: [
      { type: 'way', tags: { leisure: 'park' }, geometry: geomRect(0, 50, 0, 40) },
      { type: 'way', tags: { natural: 'wood' }, geometry: geomRect(100, 180, 0, 60) },
      { type: 'way', tags: { natural: 'water' }, geometry: geomRect(-40, -10, 0, 20) },
      { type: 'way', tags: { waterway: 'river', width: '8' }, geometry: geomRect(200, 300, 5, 5).slice(0, 2) },
      { type: 'node', tags: { natural: 'tree' }, lon: lonAt(25), lat: latAt(-20) },
      { type: 'way', tags: { highway: 'residential' }, geometry: geomRect(0, 10, 0, 10) }, // junk: ignored
    ],
  };
  const out = transformNature({ osmNature, projector: makeProjector(ANCHOR) });

  it('classifies greens with their kind', () => {
    expect(out.greens.length).toBe(2);
    const kinds = out.greens.map((g: { kind: string }) => g.kind).sort();
    expect(kinds).toEqual(['park', 'wood']);
    expect(out.greens[0].ring.length).toBeGreaterThanOrEqual(4);
  });

  it('separates water areas and river lines', () => {
    expect(out.waterAreas.length).toBe(1);
    expect(out.rivers.length).toBe(1);
    expect(out.rivers[0].width).toBe(8);
  });

  it('projects tree points to local meters', () => {
    const mapped = out.trees.filter((t: { x: number }) => t.x < 90); // exclude the wood's forest fill
    expect(mapped.length).toBe(1);
    expect(mapped[0].x).toBeCloseTo(25, 0);
    expect(mapped[0].z).toBeCloseTo(20, 0);
    expect(mapped[0].kind).toBe('broad');
  });

  it('fills the wood polygon with declared forest trees', () => {
    const filled = out.trees.filter((t: { x: number }) => t.x >= 90);
    expect(filled.length).toBeGreaterThan(20); // 4800 m² @ 1/60 ≈ 80
    for (const t of filled) expect(t.kind).toBe('broad');
  });
});
