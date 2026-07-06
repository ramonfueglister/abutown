import { describe, expect, it } from 'vitest';
import { fromBinary } from '@bufbuild/protobuf';
import { WorldTileSchema } from '../../src/proto/world_pb.js';
import { assignToTiles, encodeTile, tileGridFor } from '../../scripts/geo/lib/tiles.mjs';
import { makeCorridorSnapSampler } from '../../scripts/geo/lib/corridorsnap.mjs';

// The corridor-snap sampler wraps the graded DEM sampler that encodeTile reads.
// Both frames are ABSOLUTE graded metres: report.profiles[i].ys (pre-shift) and
// the graded local grid share the same origin, so the clamp is a pure min with
// no anchor arithmetic. CLEARANCE = 0.05 m (terrain sits just under the road).

const CLEARANCE = 0.05;

// A synthetic steep hillside: height climbs 0.3 m per metre in +x (30 % grade),
// far steeper than any tile sample step can follow across a road bench.
function steepSampler(baseY: number, grade: number) {
  return { heightAt: (x: number, _z: number) => baseY + grade * x };
}

// One straight corridor way running along +z at x = X0, with a flat profile at
// height PY. In the graded world the bench is flat; the raw hillside pierces it.
function makeWay(X0: number, z0: number, z1: number, PY: number, halfWidthM: number) {
  const pts: [number, number][] = [];
  const ys: number[] = [];
  // 10 m stations from z0..z1
  for (let z = z0; z <= z1; z += 10) {
    pts.push([X0, z]);
    ys.push(PY);
  }
  return {
    way: { pts, kind: 'road' as const, halfWidthM },
    profile: { stepM: 10, ys },
  };
}

describe('makeCorridorSnapSampler', () => {
  it('clamps tile heights inside the corridor to <= profile - CLEARANCE', () => {
    const base = steepSampler(400, 0.3);
    const X0 = 50; // profile centreline; raw hillside is 400 + 0.3*50 = 415 here
    const w = makeWay(X0, 0, 200, 415, 4);
    const snap = makeCorridorSnapSampler(base, [w.way], [w.profile]);

    // At the centreline the raw hillside equals the profile — no clamp needed,
    // but a point a few metres uphill inside the corridor pierces the bench.
    const uphillX = X0 + 3; // still inside halfWidth 4; raw = 400 + 0.3*53 = 415.9
    const raw = base.heightAt(uphillX, 100);
    expect(raw).toBeGreaterThan(415); // it DOES pierce before the snap
    const snapped = snap.heightAt(uphillX, 100);
    expect(snapped).toBeLessThanOrEqual(415 - CLEARANCE + 1e-9);
  });

  it('leaves heights outside the corridor+blend unchanged', () => {
    const base = steepSampler(400, 0.3);
    const X0 = 50;
    const w = makeWay(X0, 0, 200, 415, 4);
    const snap = makeCorridorSnapSampler(base, [w.way], [w.profile]);

    // 10 m away in x is well outside halfWidth 4 + blend 3 = 7.
    const farX = X0 + 12;
    expect(snap.heightAt(farX, 100)).toBeCloseTo(base.heightAt(farX, 100), 9);
  });

  it('never raises a height (clamp-down only) below the profile', () => {
    // A raw terrain sitting BELOW the profile (an embankment fill) must be left
    // as graded — the snap only clamps down, never fills up.
    const base = { heightAt: () => 410 }; // flat, well below profile 415
    const X0 = 0;
    const w = makeWay(X0, 0, 100, 415, 4);
    const snap = makeCorridorSnapSampler(base, [w.way], [w.profile]);
    expect(snap.heightAt(0, 50)).toBeCloseTo(410, 9);
  });

  it('relaxes the clamp across the blend band via smoothstep', () => {
    const base = steepSampler(400, 0.3);
    const X0 = 50;
    const w = makeWay(X0, 0, 200, 415, 4);
    const snap = makeCorridorSnapSampler(base, [w.way], [w.profile]);

    // Just past the hard boundary (dist 5, halfWidth 4, blend 3): the bound is
    // relaxed between profile-CLEARANCE (at dist 4) and the raw tile height (at
    // dist 7). The snapped value must be >= the fully-clamped bound and
    // <= the raw height.
    const x = X0 + 5;
    const raw = base.heightAt(x, 100);
    const snapped = snap.heightAt(x, 100);
    expect(snapped).toBeLessThanOrEqual(raw + 1e-9);
    expect(snapped).toBeGreaterThanOrEqual(415 - CLEARANCE - 1e-9);
    // strictly between the two bounds → the blend is actually mixing
    expect(snapped).toBeGreaterThan(415 - CLEARANCE);
    expect(snapped).toBeLessThan(raw);
  });

  it('extends the hard clamp by snapMarginM (coarse-lattice fix)', () => {
    const base = steepSampler(400, 0.3);
    const X0 = 50;
    const w = makeWay(X0, 0, 200, 415, 4);
    const snap = makeCorridorSnapSampler(base, [w.way], [w.profile], 20);

    // A point at dist 10 from the centreline is OUTSIDE halfWidth 4 + blend 3,
    // so with no margin it is untouched...
    const x = X0 + 10;
    expect(snap.heightAt(x, 100, 0)).toBeCloseTo(base.heightAt(x, 100), 9);
    // ...but with a 15 m snap margin it falls inside the widened hard corridor
    // (4 + 15 = 19) and is clamped to profile - CLEARANCE.
    expect(snap.heightAt(x, 100, 15)).toBeLessThanOrEqual(415 - CLEARANCE + 1e-9);
  });

  it('caps the per-query margin at maxSnapMarginM', () => {
    const base = steepSampler(400, 0.3);
    const X0 = 50;
    const w = makeWay(X0, 0, 200, 415, 4);
    // maxSnapMarginM 5: a request for 100 m margin is capped, so a point 30 m
    // away stays outside the widened corridor and is untouched.
    const snap = makeCorridorSnapSampler(base, [w.way], [w.profile], 5);
    const x = X0 + 30;
    expect(snap.heightAt(x, 100, 100)).toBeCloseTo(base.heightAt(x, 100), 9);
  });

  it('produces byte-identical encoded tiles on a double encode (determinism)', () => {
    const boundary = [[-4000, -4000], [4000, -4000], [4000, 4000], [-4000, 4000]];
    const base = steepSampler(400, 0.3);
    const w = makeWay(50, -200, 200, 415, 4);
    const g = tileGridFor(boundary, 4000);
    const mk = () => {
      const snap = makeCorridorSnapSampler(base, [w.way], [w.profile]);
      const tiles = assignToTiles(g, { buildings: [], trees: [], landuse: [], graph: { edgeA: [], edgePtOffset: [], edgePtX: [], edgePtZ: [], edgePtY: [], edgeClass: [], edgeWidth: [] } });
      return encodeTile(tiles.get('L2/8_8'), snap);
    };
    const a = mk();
    const b = mk();
    expect(Buffer.from(a).equals(Buffer.from(b))).toBe(true);
    // sanity: the encoded tile decodes and its heights are all finite
    const t = fromBinary(WorldTileSchema, a);
    expect(t.height.every((h) => Number.isFinite(h))).toBe(true);
  });
});
