// tests/traffic/carModels.test.ts
//
// FIX D2 — pure variant/colour selection for the CS-style cars. The geometry
// looks are verified visually (CDP screenshots); here we pin the stable-hash
// selection: a vehicle keeps its silhouette + colour across calls, the
// distribution spreads across all variants and the whole palette, and variant
// and colour are decorrelated.

import { describe, expect, it } from 'vitest';
import {
  carColorForId,
  carVariantForId,
  CAR_PALETTE,
  CAR_VARIANTS,
  hashId,
  wheelOffsets,
} from '../../src/diorama/traffic/carModels';
import { buildWheelGeometry, WHEEL_GEO_RADIUS } from '../../src/diorama/traffic/carModels';
import { boxGeo } from '../../src/diorama/ksw/geometryCache';
import { createCarLayer, CAR_CAPACITY, buildCarGeometry } from '../../src/diorama/traffic/carLayer';
import { buildLaneNet, type VehKinematics } from '../../src/diorama/traffic/deadReckon';

describe('carModels selection', () => {
  it('is stable per id (same colour + variant across calls)', () => {
    for (const id of [0, 1, 42, 1234, 0xfff, 0x123456]) {
      expect(carColorForId(id)).toBe(carColorForId(id));
      expect(carVariantForId(id)).toBe(carVariantForId(id));
    }
  });

  it('always returns a palette colour and a valid variant index', () => {
    for (let id = 0; id < 500; id++) {
      expect(CAR_PALETTE).toContain(carColorForId(id));
      const v = carVariantForId(id);
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThan(CAR_VARIANTS.length);
      expect(Number.isInteger(v)).toBe(true);
    }
  });

  it('spreads across every variant over a realistic fleet', () => {
    const seen = new Set<number>();
    for (let id = 0; id < 1500; id++) seen.add(carVariantForId(id));
    expect(seen.size).toBe(CAR_VARIANTS.length);
  });

  it('uses most of the palette over a realistic fleet', () => {
    const seen = new Set<number>();
    for (let id = 0; id < 1500; id++) seen.add(carColorForId(id));
    // at least most of the 14 colours should appear across 1500 vehicles
    expect(seen.size).toBeGreaterThanOrEqual(CAR_PALETTE.length - 2);
  });

  it('decorrelates variant from colour (different hash mixes)', () => {
    // For a fixed variant, colours should still span the palette — i.e. the two
    // selectors are not locked together.
    const byVariant = new Map<number, Set<number>>();
    for (let id = 0; id < 2000; id++) {
      const v = carVariantForId(id);
      let s = byVariant.get(v);
      if (!s) byVariant.set(v, (s = new Set()));
      s.add(carColorForId(id));
    }
    for (const s of byVariant.values()) {
      expect(s.size).toBeGreaterThan(CAR_PALETTE.length / 2);
    }
  });

  it('palette has the promised 10–14 distinct saturated colours', () => {
    expect(CAR_PALETTE.length).toBeGreaterThanOrEqual(10);
    expect(CAR_PALETTE.length).toBeLessThanOrEqual(14);
    expect(new Set(CAR_PALETTE).size).toBe(CAR_PALETTE.length); // all distinct
  });

  it('hashId is a stable well-spread 32-bit hash', () => {
    expect(hashId(7)).toBe(hashId(7));
    // adjacent ids must not collide into the same palette bucket trivially
    const buckets = new Set<number>();
    for (let id = 0; id < 14; id++) buckets.add(hashId(id) % CAR_PALETTE.length);
    expect(buckets.size).toBeGreaterThan(5);
  });
});

describe('CS variant table', () => {
  it('has the 6 spec variants in stable order', () => {
    expect(CAR_VARIANTS.map((v) => v.name)).toEqual([
      'sedan', 'hatchback', 'wagon', 'suv', 'van', 'pickup',
    ]);
  });

  it('variant lengths match the spec table', () => {
    const byName = Object.fromEntries(CAR_VARIANTS.map((v) => [v.name, v.length]));
    expect(byName).toEqual({
      sedan: 4.5, hatchback: 3.9, wagon: 4.6, suv: 4.6, van: 5.2, pickup: 5.0,
    });
  });

  it('every wheel layout is physically sane', () => {
    for (const v of CAR_VARIANTS) {
      expect(v.wheels.wheelbase).toBeGreaterThan(1.5);
      expect(v.wheels.wheelbase).toBeLessThan(v.length); // axles inside the body
      expect(v.wheels.track).toBeGreaterThan(1.0);
      expect(v.wheels.radius).toBeGreaterThanOrEqual(0.28);
      expect(v.wheels.radius).toBeLessThanOrEqual(0.42);
    }
  });

  it('wheelOffsets puts 4 wheels at ±track/2, ±wheelbase/2, y=radius', () => {
    const l = CAR_VARIANTS[0].wheels;
    const offs = wheelOffsets(l);
    expect(offs).toHaveLength(4);
    const xs = offs.map((o) => o[0]).sort((a, b) => a - b);
    const zs = offs.map((o) => o[2]).sort((a, b) => a - b);
    expect(xs[0]).toBeCloseTo(-l.track / 2);
    expect(xs[3]).toBeCloseTo(l.track / 2);
    expect(zs[0]).toBeCloseTo(-l.wheelbase / 2);
    expect(zs[3]).toBeCloseTo(l.wheelbase / 2);
    for (const o of offs) expect(o[1]).toBeCloseTo(l.radius);
    // front pair (+z) must be listed FIRST (indices 0,1) — carLayer steers them
    expect(offs[0][2]).toBeGreaterThan(0);
    expect(offs[1][2]).toBeGreaterThan(0);
  });
});

describe('CS geometry builders', () => {
  it('every variant builds non-empty body + glass with colour attributes', () => {
    for (const v of CAR_VARIANTS) {
      const body = v.buildBody(boxGeo);
      const glass = v.buildGlass();
      for (const g of [body, glass]) {
        expect(g.attributes.position.count).toBeGreaterThan(24); // more than one box
        expect(g.attributes.color).toBeDefined();
        expect(g.boundingSphere).not.toBeNull();
      }
      // body spans the declared length along z (±3% tolerance)
      body.computeBoundingBox();
      const bb = body.boundingBox!;
      expect(bb.max.z - bb.min.z).toBeGreaterThan(v.length * 0.97);
      expect(bb.max.z - bb.min.z).toBeLessThan(v.length * 1.03);
      // body underside clears the ground (wheels live below it): min y ≥ 0.25
      expect(bb.min.y).toBeGreaterThanOrEqual(0.25);
      // glass sits above the beltline, inside the body footprint
      glass.computeBoundingBox();
      expect(glass.boundingBox!.min.y).toBeGreaterThan(0.8);
    }
  });

  it('body bakes non-white detail zones (grille/lights/plates present)', () => {
    const body = CAR_VARIANTS[0].buildBody(boxGeo);
    const col = body.attributes.color;
    let nonWhite = 0;
    for (let i = 0; i < col.count; i++) {
      if (col.getX(i) < 0.95 || col.getY(i) < 0.95 || col.getZ(i) < 0.95) nonWhite++;
    }
    expect(nonWhite).toBeGreaterThan(20); // grille + 2 headlights + 2 taillights + 2 plates
    expect(nonWhite).toBeLessThan(col.count / 2); // …but the body is mostly tintable white
  });

  it('wheel geometry: cylinder about the x axis at the shared geo radius', () => {
    const wheel = buildWheelGeometry();
    wheel.computeBoundingBox();
    const bb = wheel.boundingBox!;
    expect(bb.max.y).toBeCloseTo(WHEEL_GEO_RADIUS, 2);
    expect(bb.min.y).toBeCloseTo(-WHEEL_GEO_RADIUS, 2);
    expect(bb.max.z).toBeCloseTo(WHEEL_GEO_RADIUS, 2);
    expect(bb.max.x).toBeLessThan(WHEEL_GEO_RADIUS); // width < diameter → axis is x
    expect(wheel.attributes.color).toBeDefined();
  });
});

describe('carLayer instancing', () => {
  const net = buildLaneNet([
    { id: 0, edge: 0, index: 0, lengthM: 100, pts: [[0, 0], [100, 0]] },
  ]);
  const vehicles = new Map<number, VehKinematics>([
    [1, { lane: 0, s: 10, v: 10, tickAt: 0 }],
    [2, { lane: 0, s: 30, v: 0, tickAt: 0 }],
  ]);

  it('draws one body+glass pair per vehicle and 4 wheels each', () => {
    const layer = createCarLayer();
    layer.update(net, vehicles, 0);
    expect(layer.debug.variantCounts().reduce((a, b) => a + b, 0)).toBe(2);
    expect(layer.debug.wheelCount()).toBe(8);
  });

  it('rotates wheels of a MOVING vehicle between frames, keeps parked wheels still', () => {
    const layer = createCarLayer();
    layer.update(net, vehicles, 0);
    const m0 = layer.debug.wheelMatrix(0); // belongs to id 1 (v=10) — insertion order
    const p0 = layer.debug.wheelMatrix(4); // id 2 (v=0)
    layer.update(net, vehicles, 10); // +1 s
    const m1 = layer.debug.wheelMatrix(0);
    const p1 = layer.debug.wheelMatrix(4);
    // rotation part must change for the mover…
    const rotDelta = (a: number[], b: number[]) =>
      Math.abs(a[5] - b[5]) + Math.abs(a[6] - b[6]) + Math.abs(a[9] - b[9]) + Math.abs(a[10] - b[10]);
    expect(rotDelta(m0, m1)).toBeGreaterThan(0.05);
    // …its position advances along the lane…
    expect(m1[14] - m0[14]).toBeCloseTo(0, 1); // z stays (lane runs along +x here)
    expect(m1[12] - m0[12]).toBeCloseTo(10, 0); // x advances ~10 m
    // …and the parked car's wheels do not spin
    expect(rotDelta(p0, p1)).toBeLessThan(1e-6);
  });

  it('exposes capacity for the wheel mesh at 4× CAR_CAPACITY', () => {
    expect(CAR_CAPACITY).toBe(4096);
  });
});

describe('far-LOD impostor geometry', () => {
  it('is a single merged geometry: sedan body + glass + 4 static wheels', () => {
    const g = buildCarGeometry();
    const sedan = CAR_VARIANTS[0];
    const bodyOnly = sedan.buildBody(boxGeo);
    expect(g.attributes.position.count).toBeGreaterThan(
      bodyOnly.attributes.position.count + 4 * 24, // strictly more than body + trivial wheels
    );
    g.computeBoundingBox();
    expect(g.boundingBox!.min.y).toBeLessThan(0.1); // wheels reach (near) the ground
    expect(g.attributes.color).toBeDefined();
  });
});
