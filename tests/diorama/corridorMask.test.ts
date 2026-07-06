// tests/diorama/corridorMask.test.ts
// Runtime decoder for the bake's mask.bin (Task 5e). Must round-trip the SAME
// bytes the bake writes (scripts/geo/lib/corridormask.mjs) — this test encodes
// with the bake lib and decodes with the runtime TS, proving the two agree.
import { describe, expect, it } from 'vitest';
import { decodeCorridorMask, corridorMaskDataTexture } from '../../src/diorama/ksw/geo/corridorMask';
import { buildCorridorMask, encodeCorridorMask } from '../../scripts/geo/lib/corridormask.mjs';

const WAYS = [{ pts: [[0, 0], [40, 0]], halfWidthM: 3, kind: 'road' }];
const BOUNDS = { minX: -10, minZ: -10, maxX: 50, maxZ: 10 };
const CELL = 2.5;

describe('decodeCorridorMask (runtime ↔ bake parity)', () => {
  const bin = encodeCorridorMask(buildCorridorMask(WAYS, BOUNDS, CELL));
  const mask = decodeCorridorMask(bin);

  it('decodes the bake header verbatim', () => {
    expect(mask.originX).toBeCloseTo(-10);
    expect(mask.originZ).toBeCloseTo(-10);
    expect(mask.cellSizeM).toBeCloseTo(2.5);
    expect(mask.cols).toBeGreaterThan(0);
    expect(mask.rows).toBeGreaterThan(0);
  });

  it('reads covered/uncovered cells consistent with the corridor', () => {
    expect(mask.covers(20, 0)).toBe(true); // on the centreline
    expect(mask.covers(20, 8)).toBe(false); // 8 m off
    expect(mask.covers(48, 0)).toBe(false); // past the end
  });

  it('hard-errors on a bad magic (no silent fallback)', () => {
    const bad = new Uint8Array(bin);
    bad[0] ^= 0xff;
    expect(() => decodeCorridorMask(bad)).toThrow(/magic/i);
  });

  it('builds a nearest-filtered single-channel data texture sized cols×rows', () => {
    const tex = corridorMaskDataTexture(mask);
    expect(tex.image.width).toBe(mask.cols);
    expect(tex.image.height).toBe(mask.rows);
    // covered cell → 255, uncovered → 0 in the expanded R8 texel buffer
    const data = tex.image.data as Uint8Array;
    const at = (x: number, z: number) => {
      const i = Math.round((x - mask.originX) / mask.cellSizeM);
      const j = Math.round((z - mask.originZ) / mask.cellSizeM);
      return data[j * mask.cols + i];
    };
    expect(at(20, 0)).toBe(255);
    expect(at(20, 8)).toBe(0);
  });
});
