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
