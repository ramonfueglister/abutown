// tests/geo/transform.test.ts
import { describe, expect, it } from 'vitest';
import { makeProjector, ANCHOR } from '../../scripts/geo/lib/project.mjs';
import { transformBuildings, transformRoads } from '../../scripts/geo/lib/transform.mjs';

// one synthetic flat-roof building ~50 m east of the anchor, ground at 450 m
const lonAt = (m: number) => ANCHOR.lon + m / (111320 * Math.cos((ANCHOR.lat * Math.PI) / 180));
const latAt = (m: number) => ANCHOR.lat + m / 111132;
const ringLL = (x0: number, x1: number, z0: number, z1: number, y: number) => [
  [lonAt(x0), latAt(-z0), y], [lonAt(x1), latAt(-z0), y], [lonAt(x1), latAt(-z1), y], [lonAt(x0), latAt(-z1), y], [lonAt(x0), latAt(-z0), y],
];
const feat = (uuid: string, ring: number[][]) => ({
  type: 'Feature', properties: { UUID: uuid }, geometry: { type: 'MultiPolygon', coordinates: [[ring]] },
});
const fc = (...features: unknown[]) => ({ type: 'FeatureCollection', features });

const floors = fc(feat('b1', ringLL(50, 60, 10, 20, 450)));
const roofs = fc(feat('b1', ringLL(50, 60, 10, 20, 458)));
const walls = fc(
  feat('b1', [
    [lonAt(50), latAt(-10), 450], [lonAt(60), latAt(-10), 450],
    [lonAt(60), latAt(-10), 458], [lonAt(50), latAt(-10), 458], [lonAt(50), latAt(-10), 450],
  ]),
);
const osmBuildings = [{ ring: [[45, 5], [65, 5], [65, 25], [45, 25]], tags: { name: 'Testbau', building: 'hospital' } }];

describe('transformBuildings', () => {
  const out = transformBuildings({ floors, walls, roofs, osmBuildings, projector: makeProjector(ANCHOR) });

  it('produces one building with ground-normalized cm-integer geometry', () => {
    expect(out.length).toBe(1);
    const b = out[0];
    expect(b.height).toBeCloseTo(8, 1); // 458 − 450
    expect(Number.isInteger(b.roof.pos[0])).toBe(true);
    const ys = b.roof.pos.filter((_, i) => i % 3 === 1);
    expect(Math.max(...ys)).toBe(800); // roof at 8 m = 800 cm
    const wys = b.wall.pos.filter((_, i) => i % 3 === 1);
    expect(Math.min(...wys)).toBe(0); // ground normalized
  });

  it('joins the OSM name and flags the ksw zone (centroid 55 m < 170 m)', () => {
    expect(out[0].name).toBe('Testbau');
    expect(out[0].zone).toBe('ksw');
  });
});

describe('transformRoads', () => {
  it('projects way geometry and keeps the classification', () => {
    const osmRoads = {
      elements: [{ type: 'way', tags: { highway: 'residential' }, geometry: [
        { lon: lonAt(0), lat: latAt(0) }, { lon: lonAt(100), lat: latAt(0) },
      ] }],
    };
    const { roads } = transformRoads({ osmRoads, projector: makeProjector(ANCHOR) });
    expect(roads.length).toBe(1);
    expect(roads[0].width).toBe(5.5);
    expect(roads[0].pts[1][0]).toBeCloseTo(100, 0);
  });
});
