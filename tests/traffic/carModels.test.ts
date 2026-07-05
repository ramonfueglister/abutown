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
