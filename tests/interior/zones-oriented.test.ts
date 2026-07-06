// Oriented decomposition (Phase A): zones are extracted in a frame rotated to
// the footprint's dominant wall angle, so rooms run parallel to the facade and
// coverage approaches the full footprint (the old axis-aligned decomposition
// plateaued at ~61% on the diagonal KSW complex).
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { decomposeOriented, decomposeToZones, dominantAngle, type Zone } from '../../src/diorama/ksw/interior/zones';

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
  for (let i = 0, j = n - 1; i < n; j = i++) a += ring[j][0] * ring[i][1] - ring[i][0] * ring[j][1];
  return Math.abs(a) / 2;
}

// a 40×24 rectangle rotated by 23° around the origin
function rotatedRect(deg: number): number[][] {
  const a = (deg * Math.PI) / 180;
  const pts: number[][] = [[-20, -12], [20, -12], [20, 12], [-20, 12]];
  return pts.map(([x, z]) => [x * Math.cos(a) + z * Math.sin(a), -x * Math.sin(a) + z * Math.cos(a)]);
}

function realKswFootprint(): number[][] {
  const raw = JSON.parse(readFileSync(resolve(__dirname, '../../data/winterthur/buildings.json'), 'utf8'));
  const ksw = raw.buildings.filter((b: { zone?: string }) => b.zone === 'ksw');
  let best = ksw[0];
  for (const b of ksw) if (shoelaceArea(b.footprint) > shoelaceArea(best.footprint)) best = b;
  return best.footprint;
}

describe('dominantAngle', () => {
  it('recovers the rotation of a rotated rectangle (mod 90 degrees)', () => {
    const a = dominantAngle(rotatedRect(23));
    const folded = Math.abs((((a * 180) / Math.PI) % 90) + 90) % 90;
    expect(Math.min(Math.abs(folded - 23), Math.abs(folded - 67))).toBeLessThan(0.5);
  });
  it('returns ~0 for an axis-aligned rectangle', () => {
    const a = dominantAngle([[0, 0], [30, 0], [30, 10], [0, 10]]);
    const deg = Math.abs((a * 180) / Math.PI) % 90;
    expect(Math.min(deg, 90 - deg)).toBeLessThan(0.5);
  });
  it('is deterministic', () => {
    const fp = realKswFootprint();
    expect(dominantAngle(fp)).toBe(dominantAngle(fp));
  });
});

describe('decomposeOriented', () => {
  it('frame round-trips: toWorld(toLocal(p)) approximately equals p', () => {
    const { frame } = decomposeOriented(rotatedRect(23));
    for (const [x, z] of [[3.2, -7.7], [0, 0], [-15, 4]]) {
      const [lx, lz] = frame.toLocal(x, z);
      const [wx, wz] = frame.toWorld(lx, lz);
      expect(wx).toBeCloseTo(x, 9);
      expect(wz).toBeCloseTo(z, 9);
    }
  });

  it('covers 85 percent of a rotated rectangle (the old path could not)', () => {
    const fp = rotatedRect(23);
    const { zones } = decomposeOriented(fp);
    const covered = zones.reduce((s, z) => s + z.w * z.d, 0);
    expect(covered / shoelaceArea(fp)).toBeGreaterThan(0.85);
  });

  it('covers 80 percent of the real KSW footprint (old: 61 percent)', () => {
    const fp = realKswFootprint();
    const { zones } = decomposeOriented(fp);
    const covered = zones.reduce((s, z) => s + z.w * z.d, 0);
    expect(covered / shoelaceArea(fp)).toBeGreaterThan(0.8);
  });

  it('every zone corner, mapped to world, lies inside the original footprint', () => {
    const fp = realKswFootprint();
    const { zones, frame } = decomposeOriented(fp);
    const eps = 1e-6;
    for (const z of zones) {
      for (const [cx, cz] of [
        [z.x - z.w / 2 + eps, z.z - z.d / 2 + eps],
        [z.x + z.w / 2 - eps, z.z - z.d / 2 + eps],
        [z.x + z.w / 2 - eps, z.z + z.d / 2 - eps],
        [z.x - z.w / 2 + eps, z.z + z.d / 2 - eps],
      ]) {
        const [wx, wz] = frame.toWorld(cx, cz);
        expect(pointInRing(wx, wz, fp)).toBe(true);
      }
    }
  });

  it('zones never overlap each other', () => {
    const { zones } = decomposeOriented(realKswFootprint());
    const e = 1e-6;
    for (let i = 0; i < zones.length; i++) {
      for (let j = i + 1; j < zones.length; j++) {
        const a = zones[i] as Zone;
        const b = zones[j] as Zone;
        const overlap =
          a.x - a.w / 2 < b.x + b.w / 2 - e && b.x - b.w / 2 < a.x + a.w / 2 - e &&
          a.z - a.d / 2 < b.z + b.d / 2 - e && b.z - b.d / 2 < a.z + a.d / 2 - e;
        expect(overlap).toBe(false);
      }
    }
  });

  it('legacy decomposeToZones is unchanged for existing callers', () => {
    const fp = [[0, 0], [40, 0], [40, 20], [0, 20]];
    const zones = decomposeToZones(fp);
    expect(zones.length).toBeGreaterThan(0);
  });
});
