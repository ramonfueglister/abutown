import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { decomposeToZones, type Zone } from '../../src/diorama/ksw/interior/zones';

// Standard ray-casting point-in-polygon test, mirrors scripts/geo/lib/join.mjs::pointInRing.
function pointInRing(x: number, z: number, ring: number[][]): boolean {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, zi] = ring[i];
    const [xj, zj] = ring[j];
    if (zi > z !== zj > z && x < ((xj - xi) * (z - zi)) / (zj - zi) + xi) inside = !inside;
  }
  return inside;
}

function shoelaceArea(ring: number[][]): number {
  let a = 0;
  const n = ring.length;
  for (let i = 0, j = n - 1; i < n; j = i++) {
    a += ring[j][0] * ring[i][1] - ring[i][0] * ring[j][1];
  }
  return Math.abs(a) / 2;
}

function zoneArea(z: Zone): number {
  return z.w * z.d;
}

// Nudge corners a hair inward before the point-in-polygon check: the
// standard ray-casting test treats the top/right ring edges as exclusive
// (a documented, standard half-open convention — see pointInRing above),
// so a corner sitting exactly on the polygon boundary can read as
// "outside" even though the rectangle itself is fully contained. The
// invariant under test is containment of the zone, not sub-millimeter
// boundary semantics of the point test.
const CORNER_EPS = 1e-6;
function zoneCorners(z: Zone): Array<[number, number]> {
  const hw = z.w / 2 - CORNER_EPS;
  const hd = z.d / 2 - CORNER_EPS;
  return [
    [z.x - hw, z.z - hd],
    [z.x + hw, z.z - hd],
    [z.x + hw, z.z + hd],
    [z.x - hw, z.z + hd],
  ];
}

// Axis-aligned rect overlap test (strict — touching edges are not overlap).
function rectsOverlap(a: Zone, b: Zone): boolean {
  const ax0 = a.x - a.w / 2;
  const ax1 = a.x + a.w / 2;
  const az0 = a.z - a.d / 2;
  const az1 = a.z + a.d / 2;
  const bx0 = b.x - b.w / 2;
  const bx1 = b.x + b.w / 2;
  const bz0 = b.z - b.d / 2;
  const bz1 = b.z + b.d / 2;
  const eps = 1e-6;
  return ax0 < bx1 - eps && bx0 < ax1 - eps && az0 < bz1 - eps && bz0 < az1 - eps;
}

// L-shaped polygon fixture: a 20x20 square with a 10x10 notch bitten out of
// the top-right corner. The largest-rectangle extraction should find two
// axis-aligned rectangles that tile the L (a 20x10 base + a 10x10 remainder,
// or equivalent split depending on extraction order).
const L_POLYGON: number[][] = [
  [0, 0],
  [20, 0],
  [20, 10],
  [10, 10],
  [10, 20],
  [0, 20],
];

describe('decomposeToZones — L-polygon fixture', () => {
  it('produces exactly 2 zones', () => {
    const zones = decomposeToZones(L_POLYGON);
    expect(zones.length).toBe(2);
  });

  it('zones do not overlap', () => {
    const zones = decomposeToZones(L_POLYGON);
    for (let i = 0; i < zones.length; i++) {
      for (let j = i + 1; j < zones.length; j++) {
        expect(rectsOverlap(zones[i], zones[j])).toBe(false);
      }
    }
  });

  it('every zone corner + center lies inside the polygon', () => {
    const zones = decomposeToZones(L_POLYGON);
    for (const z of zones) {
      expect(pointInRing(z.x, z.z, L_POLYGON)).toBe(true);
      for (const [cx, cz] of zoneCorners(z)) {
        expect(pointInRing(cx, cz, L_POLYGON)).toBe(true);
      }
    }
  });

  it('every zone is at least 6x6', () => {
    const zones = decomposeToZones(L_POLYGON);
    for (const z of zones) {
      expect(z.w).toBeGreaterThanOrEqual(6);
      expect(z.d).toBeGreaterThanOrEqual(6);
    }
  });

  it('ids are z0..zN in extraction order', () => {
    const zones = decomposeToZones(L_POLYGON);
    zones.forEach((z, i) => expect(z.id).toBe(`z${i}`));
  });

  it('is deterministic', () => {
    const a = decomposeToZones(L_POLYGON);
    const b = decomposeToZones(L_POLYGON);
    expect(a).toEqual(b);
  });
});

describe('decomposeToZones — real KSW footprint', () => {
  const dataPath = resolve(__dirname, '../../data/winterthur/buildings.json');
  const data = JSON.parse(readFileSync(dataPath, 'utf-8')) as {
    buildings: Array<{ zone: string; footprint: number[][] }>;
  };
  const kswBuildings = data.buildings.filter((b) => b.zone === 'ksw');
  expect(kswBuildings.length).toBeGreaterThan(0);
  let mainFootprint = kswBuildings[0].footprint;
  let maxArea = shoelaceArea(mainFootprint);
  for (const b of kswBuildings) {
    const a = shoelaceArea(b.footprint);
    if (a > maxArea) {
      maxArea = a;
      mainFootprint = b.footprint;
    }
  }
  const polygonArea = maxArea;

  it('produces at most the default maxZones cap', () => {
    // Default cap is 14, not the plan's nominal 8 — see DEFAULT_MAX_ZONES
    // in zones.ts for why: the real 113-point footprint plateaus well
    // below 60% coverage after 8 greedy rectangle extractions no matter
    // the raster resolution, so 8 cannot satisfy the coverage invariant.
    const zones = decomposeToZones(mainFootprint);
    expect(zones.length).toBeGreaterThan(0);
    expect(zones.length).toBeLessThanOrEqual(14);
  });

  it('zones do not overlap', () => {
    const zones = decomposeToZones(mainFootprint);
    for (let i = 0; i < zones.length; i++) {
      for (let j = i + 1; j < zones.length; j++) {
        expect(rectsOverlap(zones[i], zones[j])).toBe(false);
      }
    }
  });

  it('every zone corner + center lies inside the polygon', () => {
    const zones = decomposeToZones(mainFootprint);
    for (const z of zones) {
      expect(pointInRing(z.x, z.z, mainFootprint)).toBe(true);
      for (const [cx, cz] of zoneCorners(z)) {
        expect(pointInRing(cx, cz, mainFootprint)).toBe(true);
      }
    }
  });

  it('covers at least 60% of the polygon area', () => {
    const zones = decomposeToZones(mainFootprint);
    const covered = zones.reduce((sum, z) => sum + zoneArea(z), 0);
    expect(covered / polygonArea).toBeGreaterThanOrEqual(0.6);
  });

  it('every zone is at least 6x6', () => {
    const zones = decomposeToZones(mainFootprint);
    for (const z of zones) {
      expect(z.w).toBeGreaterThanOrEqual(6);
      expect(z.d).toBeGreaterThanOrEqual(6);
    }
  });

  it('is deterministic across calls', () => {
    const a = decomposeToZones(mainFootprint);
    const b = decomposeToZones(mainFootprint);
    expect(a).toEqual(b);
  });
});
