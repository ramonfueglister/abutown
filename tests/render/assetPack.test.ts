import { describe, expect, it } from 'vitest';
import { createAssetPack, missingAssetRoleError } from '../../src/assets/assetPack';

describe('asset pack lookup', () => {
  const pack = createAssetPack({
    id: 'test-pak128',
    tile: { width: 128, height: 64 },
    assets: [
      {
        role: 'terrain.grass',
        path: '/simutrans-assets/pak128/landscape/grounds/texture-climate.png',
        source: { x: 0, y: 0, width: 128, height: 64 },
        anchor: { x: 64, y: 32 },
        baseline: 32,
        scale: 1,
        cleanup: 'pak128',
        provenance: {
          sourcePath: 'landscape/grounds/texture-climate.png',
          datPath: 'landscape/grounds/texture-climate.dat',
          license: 'Artistic-2.0',
          revision: 'acdf2f0793a6beee5ea34ea85d308fbbeccf50c5',
        },
      },
    ],
  });

  it('resolves exact semantic roles', () => {
    expect(pack.require('terrain.grass')).toEqual(expect.objectContaining({
      role: 'terrain.grass',
      path: '/simutrans-assets/pak128/landscape/grounds/texture-climate.png',
    }));
  });

  it('returns undefined for missing optional lookup', () => {
    expect(pack.resolve('road.straight')).toBeUndefined();
  });

  it('throws a clear error for missing required roles', () => {
    expect(() => pack.require('road.straight')).toThrow(missingAssetRoleError('test-pak128', 'road.straight'));
  });
});
