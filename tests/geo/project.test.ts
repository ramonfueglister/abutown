// tests/geo/project.test.ts
import { describe, expect, it } from 'vitest';
import { ANCHOR, BBOX, makeProjector } from '../../scripts/geo/lib/project.mjs';

function haversine(lon1: number, lat1: number, lon2: number, lat2: number): number {
  const R = 6371008.8;
  const p = Math.PI / 180;
  const a =
    Math.sin(((lat2 - lat1) * p) / 2) ** 2 +
    Math.cos(lat1 * p) * Math.cos(lat2 * p) * Math.sin(((lon2 - lon1) * p) / 2) ** 2;
  return 2 * R * Math.asin(Math.sqrt(a));
}

describe('makeProjector', () => {
  const proj = makeProjector(ANCHOR);

  it('maps the anchor itself to the origin', () => {
    const [x, z] = proj.toLocal(ANCHOR.lon, ANCHOR.lat);
    expect(Math.abs(x)).toBeLessThan(1e-9);
    expect(Math.abs(z)).toBeLessThan(1e-9);
  });

  it('matches haversine distance to <1 m across the whole bbox diagonal', () => {
    const [x, z] = proj.toLocal(BBOX.lonMin, BBOX.latMin);
    const d = Math.hypot(x, z);
    const ref = haversine(ANCHOR.lon, ANCHOR.lat, BBOX.lonMin, BBOX.latMin);
    expect(Math.abs(d - ref)).toBeLessThan(1.0);
  });

  it('Bahnhof lies south-west of the KSW anchor: x<0, z>0 (south positive)', () => {
    const [x, z] = proj.toLocal(8.724, 47.5003);
    expect(x).toBeLessThan(-300);
    expect(z).toBeGreaterThan(600);
  });

  it('exposes anchorLon/anchorLat for inverse projection (DEM sampler)', () => {
    expect(proj.anchorLon).toBe(ANCHOR.lon);
    expect(proj.anchorLat).toBe(ANCHOR.lat);
  });
});
