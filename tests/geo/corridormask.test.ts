// tests/geo/corridormask.test.ts
// Task 5e: the bake-side corridor mask raster. A packed 1-bit-per-cell
// world-space grid marking every cell whose centre lies inside a road/rail
// corridor (the SAME halfWidth definition grading flattens). The terrain
// shader discards fragments where the mask reads 1; ribbon skirts close the
// resulting hole. These tests pin: rasterization matches the corridor
// predicate, encode↔decode round-trips byte-exactly, the reader agrees with
// the rasterizer, and a double-build is byte-identical (determinism).
import { describe, expect, it } from 'vitest';
import {
  buildCorridorMask,
  encodeCorridorMask,
  decodeCorridorMask,
  maskCovers,
} from '../../scripts/geo/lib/corridormask.mjs';

// One straight road along +x at z=0, halfWidth 3 m; covers a band |z| ≤ 3.
const WAYS = [
  { pts: [[0, 0], [40, 0]], halfWidthM: 3, kind: 'road' },
];
const BOUNDS = { minX: -10, minZ: -10, maxX: 50, maxZ: 10 };
const CELL = 2.5;

describe('buildCorridorMask', () => {
  it('marks cells whose centre is inside a corridor and clears the rest', () => {
    const mask = buildCorridorMask(WAYS, BOUNDS, CELL);
    // a point on the centreline is covered; a point 8 m away in z is not.
    expect(maskCovers(mask, 20, 0)).toBe(true);
    expect(maskCovers(mask, 20, 8)).toBe(false);
    // just inside / outside the 3 m halfWidth boundary (cell-quantized)
    expect(maskCovers(mask, 20, 2)).toBe(true);
    expect(maskCovers(mask, 20, 9)).toBe(false);
  });

  it('covers points off the ends only within the corridor cap', () => {
    const mask = buildCorridorMask(WAYS, BOUNDS, CELL);
    expect(maskCovers(mask, 0, 0)).toBe(true); // the start vertex
    expect(maskCovers(mask, 40, 0)).toBe(true); // the end vertex
    // far past the end, outside any cap
    expect(maskCovers(mask, 48, 0)).toBe(false);
  });

  it('has a header describing origin, cell size, and grid dimensions', () => {
    const mask = buildCorridorMask(WAYS, BOUNDS, CELL);
    expect(mask.originX).toBeCloseTo(BOUNDS.minX);
    expect(mask.originZ).toBeCloseTo(BOUNDS.minZ);
    expect(mask.cellSizeM).toBeCloseTo(CELL);
    expect(mask.cols).toBeGreaterThan(0);
    expect(mask.rows).toBeGreaterThan(0);
    // bitfield holds one bit per cell, packed to bytes
    expect(mask.bits.length).toBe(Math.ceil((mask.cols * mask.rows) / 8));
  });
});

describe('encode/decode round-trip', () => {
  it('re-decodes to a byte-identical mask', () => {
    const mask = buildCorridorMask(WAYS, BOUNDS, CELL);
    const bin = encodeCorridorMask(mask);
    const back = decodeCorridorMask(bin);
    expect(back.originX).toBeCloseTo(mask.originX);
    expect(back.originZ).toBeCloseTo(mask.originZ);
    expect(back.cellSizeM).toBeCloseTo(mask.cellSizeM);
    expect(back.cols).toBe(mask.cols);
    expect(back.rows).toBe(mask.rows);
    expect(Buffer.from(back.bits).equals(Buffer.from(mask.bits))).toBe(true);
    // reader agrees through the round-trip
    expect(maskCovers(back, 20, 0)).toBe(true);
    expect(maskCovers(back, 20, 8)).toBe(false);
  });

  it('is deterministic: two builds encode to identical bytes', () => {
    const a = encodeCorridorMask(buildCorridorMask(WAYS, BOUNDS, CELL));
    const b = encodeCorridorMask(buildCorridorMask(WAYS, BOUNDS, CELL));
    expect(Buffer.from(a).equals(Buffer.from(b))).toBe(true);
  });
});

describe('coverage guarantee', () => {
  it('covers 100% of densely-sampled centreline stations', () => {
    const mask = buildCorridorMask(WAYS, BOUNDS, CELL);
    for (let s = 0; s <= 40; s += 1) {
      expect(maskCovers(mask, s, 0)).toBe(true);
    }
  });
});
