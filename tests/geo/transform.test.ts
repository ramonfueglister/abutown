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

describe('transformBuildings — multi-part UUID (real LoD2 wall facets)', () => {
  // one swisstopo UUID with TWO disjoint building parts (real-world case:
  // an annex/wing far from the main volume) — the roof for part 2 must sit
  // on part 2's own walls, not float above the footprint of part 1 only.
  const floors2 = fc(
    feat('b2', ringLL(0, 10, 0, 10, 400)), // part 1 base
    feat('b2', ringLL(100, 110, 100, 110, 400)), // part 2 base, far away
  );
  const roofs2 = fc(
    feat('b2', ringLL(0, 10, 0, 10, 405)), // part 1 roof
    feat('b2', ringLL(100, 110, 100, 110, 405)), // part 2 roof
  );
  // one wall facet per edge of an x0..x1 × z0..z1 box, y0..y1 tall — a real
  // building has 4 (or more) wall facets, not one flat plane
  const wallBox = (uuid: string, x0: number, x1: number, z0: number, z1: number, y0: number, y1: number) => {
    const edge = (ax: number, az: number, bx: number, bz: number) => feat(uuid, [
      [lonAt(ax), latAt(-az), y0], [lonAt(bx), latAt(-bz), y0],
      [lonAt(bx), latAt(-bz), y1], [lonAt(ax), latAt(-az), y1], [lonAt(ax), latAt(-az), y0],
    ]);
    return [edge(x0, z0, x1, z0), edge(x1, z0, x1, z1), edge(x1, z1, x0, z1), edge(x0, z1, x0, z0)];
  };
  const walls2 = fc(
    ...wallBox('b2', 0, 10, 0, 10, 400, 405), // part 1 walls (4 facets)
    ...wallBox('b2', 100, 110, 100, 110, 400, 405), // part 2 walls — far from part 1
  );

  const out2 = transformBuildings({ floors: floors2, walls: walls2, roofs: roofs2, osmBuildings: [], projector: makeProjector(ANCHOR) });

  it('carries every roof part on its own real wall facets (no floating roof)', () => {
    expect(out2.length).toBe(1);
    const b = out2[0];
    // gather wall base (y=0 after ground-normalize) XZ points
    const wallBaseXZ: number[][] = [];
    for (let i = 0; i < b.wall.pos.length; i += 3) {
      if (b.wall.pos[i + 1] === 0) wallBaseXZ.push([b.wall.pos[i] / 100, b.wall.pos[i + 2] / 100]);
    }
    // every roof vertex (in XZ) must have SOME wall-base point within 2 m —
    // this fails for part 2 (near x=100..110,z=-100..-110) under the old
    // single-footprint prism, which only extrudes part 1's footprint.
    const within2m = (x: number, z: number) => wallBaseXZ.some(([wx, wz]) => Math.hypot(wx - x, wz - z) < 2);
    let covered = 0;
    let total = 0;
    for (let i = 0; i < b.roof.pos.length; i += 3) {
      total += 1;
      if (within2m(b.roof.pos[i] / 100, b.roof.pos[i + 2] / 100)) covered += 1;
    }
    expect(total).toBeGreaterThan(0);
    expect(covered / total).toBeGreaterThan(0.95);
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

  it('prefers an explicit width tag over the class fallback', () => {
    const osmRoads = {
      elements: [{ type: 'way', tags: { highway: 'residential', width: '7.5' }, geometry: [
        { lon: lonAt(0), lat: latAt(0) }, { lon: lonAt(100), lat: latAt(0) },
      ] }],
    };
    const { roads } = transformRoads({ osmRoads, projector: makeProjector(ANCHOR) });
    expect(roads[0].width).toBe(7.5);
  });
});
