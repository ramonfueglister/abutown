// tests/geo/transform.test.ts
import { describe, expect, it } from 'vitest';
import { makeProjector, ANCHOR } from '../../scripts/geo/lib/project.mjs';
import { transformBuildings, transformRoads, wallBasePointsMeters } from '../../scripts/geo/lib/transform.mjs';

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

describe('transformBuildings — roof part with NO wall facets at all (Task 12 completion fix)', () => {
  // Real swisstopo gap: some roof PARTS have zero corresponding wall facets in
  // the source (not "far away walls", but genuinely none). The old code left
  // those roofs floating with nothing under them. The fix: group roof rings
  // into connected components (rings sharing a vertex), and for any component
  // whose facet centroids have no nearby rendered wall-base point, build a
  // ground→eave prism from the convex hull of that component's own vertices —
  // geodetically honest because it derives strictly from that part's real
  // roof geometry, not from an unrelated part's footprint.
  const part2RoofRing = ringLL(100, 108, 100, 108, 405);
  const floors3 = fc(feat('b3', ringLL(0, 10, 0, 10, 400))); // part 1 base only
  const roofs3 = fc(
    feat('b3', ringLL(0, 10, 0, 10, 405)), // part 1 roof (has walls)
    feat('b3', part2RoofRing), // part 2 roof — NO wall facets in source
  );
  const wallBox = (uuid: string, x0: number, x1: number, z0: number, z1: number, y0: number, y1: number) => {
    const edge = (ax: number, az: number, bx: number, bz: number) => feat(uuid, [
      [lonAt(ax), latAt(-az), y0], [lonAt(bx), latAt(-bz), y0],
      [lonAt(bx), latAt(-bz), y1], [lonAt(ax), latAt(-az), y1], [lonAt(ax), latAt(-az), y0],
    ]);
    return [edge(x0, z0, x1, z0), edge(x1, z0, x1, z1), edge(x1, z1, x0, z1), edge(x0, z1, x0, z0)];
  };
  // only part 1 gets real wall facets — part 2 has none in this fixture
  const walls3 = fc(...wallBox('b3', 0, 10, 0, 10, 400, 405));

  const out3 = transformBuildings({ floors: floors3, walls: walls3, roofs: roofs3, osmBuildings: [], projector: makeProjector(ANCHOR) });

  it('closes the wall-less roof part with a per-part hull prism (coverage >= 0.9 on the fixture)', () => {
    expect(out3.length).toBe(1);
    const b = out3[0];
    const wallBaseXZ = wallBasePointsMeters(b.wall.pos);

    // Reuse the same coverage definition as the bake gate: facet centroid has
    // a wall-base point within 6 m. The hull-prism closure extrudes exactly
    // the part-2 roof ring's own footprint, so its wall base sits directly
    // under the roof — well within 6 m of the roof ring's own centroid,
    // regardless of the box's corner-to-center diagonal.
    let cx = 0, cz = 0, n = 0;
    for (const [lon, lat] of part2RoofRing) {
      const [lx, lz] = makeProjector(ANCHOR).toLocal(lon, lat);
      cx += lx; cz += lz; n += 1;
    }
    cx /= n; cz /= n;
    const covered = wallBaseXZ.some(([wx, wz]) => Math.hypot(wx - cx, wz - cz) < 6);
    expect(covered).toBe(true);
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

describe('wallBasePointsMeters — coverage gate must read the BAKED wall mesh (Task 12)', () => {
  // Task 12 finding: the coverage gate used to derive its wall point set from
  // the raw per-UUID `b.walls` facets, which exist even under the original
  // prism bug (extrudeWalls collapses every disjoint part onto ONE
  // footprint) — so it read ~100% on geometry that was actually broken. The
  // gate must instead measure the mesh that was really triangulated/rendered
  // (`wall.pos`), which for a single-footprint prism only has base points
  // under part 1.

  // A two-part building: part 1 at x=0..10,z=0..10; part 2 far away at
  // x=100..110,z=100..110. Real per-facet wall geometry covers BOTH parts.
  const realWallPos = [
    // part 1 base ring (y=0)
    0, 0, 0, 1000, 0, 0, 1000, 0, 1000, 0, 0, 1000,
    // part 2 base ring (y=0), far away
    10000, 0, 10000, 10800, 0, 10000, 10800, 0, 10800, 10000, 0, 10800,
  ];
  // A single-footprint PRISM (the pre-fix bug shape): extrudeWalls only ever
  // extrudes ONE traced footprint ring, so its base only covers part 1 —
  // part 2's roof floats with nothing nearby.
  const prismWallPos = [0, 0, 0, 1000, 0, 0, 1000, 0, 1000, 0, 0, 1000];
  const part2RoofCentroid = [104, 104]; // meters — centroid of the part-2 base square

  function coverageFor(wallPos: number[]) {
    const wallBaseXZ = wallBasePointsMeters(wallPos);
    return wallBaseXZ.some(([wx, wz]) => Math.hypot(wx - part2RoofCentroid[0], wz - part2RoofCentroid[1]) < 6);
  }

  it('reads LOW coverage for part 2 against a single-footprint prism mesh (the old bug shape)', () => {
    expect(coverageFor(prismWallPos)).toBe(false);
  });

  it('reads HIGH coverage for part 2 against the real multi-part wall mesh', () => {
    expect(coverageFor(realWallPos)).toBe(true);
  });

  it('extracts only ground-level (y ≤ 60 cm) points, in meters, dropping eave/ridge vertices', () => {
    const wallPos = [
      0, 0, 0, // base
      500, 55, 0, // still base (skirt tolerance)
      1000, 400, 0, // eave/top — must be excluded
    ];
    const pts = wallBasePointsMeters(wallPos);
    expect(pts).toEqual([[0, 0], [5, 0]]);
  });
});
