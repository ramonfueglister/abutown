// tests/diorama/corridorMask.test.ts
// Runtime decoder for the bake's mask.bin (Task 5e). Must round-trip the SAME
// bytes the bake writes (scripts/geo/lib/corridormask.mjs) — this test encodes
// with the bake lib and decodes with the runtime TS, proving the two agree.
import { describe, expect, it } from 'vitest';
import { decodeCorridorMask, corridorMaskDataTexture, maskShaderUv } from '../../src/diorama/ksw/geo/corridorMask';
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

describe('maskShaderUv (shader ↔ covers() cell parity, #144)', () => {
  const bin = encodeCorridorMask(buildCorridorMask(WAYS, BOUNDS, CELL));
  const mask = decodeCorridorMask(bin);

  it('NEAREST texel index from the shader UV equals the covers() round-to-nearest cell', () => {
    // The bake stamps cell (i,j) with CENTRE at (origin + i·cell) and covers()
    // reads round-to-nearest. A NEAREST texture sample resolves texel
    // floor(u·cols) — so the UV must be offset by half a cell or the shader
    // discards a footprint shifted +cell/2 in +x/+z (root cause of #144's
    // one-sided see-through strips). Sweep points across cells, cell borders
    // and half-cell offsets, including negative world coords.
    for (let k = 0; k < 2000; k++) {
      const x = mask.originX + (k * 0.37) % (mask.cols * mask.cellSizeM - 1);
      const z = mask.originZ + (k * 0.53) % (mask.rows * mask.cellSizeM - 1);
      const [u, v] = maskShaderUv(mask, x, z);
      const texelI = Math.min(mask.cols - 1, Math.max(0, Math.floor(u * mask.cols)));
      const texelJ = Math.min(mask.rows - 1, Math.max(0, Math.floor(v * mask.rows)));
      const cellI = Math.min(mask.cols - 1, Math.max(0, Math.round((x - mask.originX) / mask.cellSizeM)));
      const cellJ = Math.min(mask.rows - 1, Math.max(0, Math.round((z - mask.originZ) / mask.cellSizeM)));
      expect(texelI).toBe(cellI);
      expect(texelJ).toBe(cellJ);
    }
  });

  it('exact cell borders resolve like Math.round (half-up), matching covers()', () => {
    // x exactly between two cell centres: covers() rounds half-up.
    const x = mask.originX + 1.25; // between cell 0 (centre 0) and cell 1 (centre 2.5)
    const [u] = maskShaderUv(mask, x, mask.originZ);
    expect(Math.floor(u * mask.cols)).toBe(Math.round(1.25 / 2.5));
  });
});
