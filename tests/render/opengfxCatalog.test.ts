import { describe, expect, it } from 'vitest';
import { OPEN_GFX_SOURCE_REVISION, opengfxAssets } from '../../src/assets/opengfxCatalog.generated';
import { assetsByCategory, firstAssetPath, getAssetsForCategory } from '../../src/assets/opengfxCatalog';

describe('OpenGFX catalog', () => {
  it('contains broad generated OpenGFX coverage', () => {
    expect(opengfxAssets.length).toBeGreaterThanOrEqual(600);
    expect(new Set(opengfxAssets.map((asset) => asset.category)).size).toBeGreaterThanOrEqual(10);
    expect(OPEN_GFX_SOURCE_REVISION).toBe('e922d2303d695e88965a70ea3158215f8c0be15b');
  });

  it('exposes semantic categories for city composition', () => {
    expect(getAssetsForCategory('terrain').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('water').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('road').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('rail').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('building').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('tree').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('vehicle').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('station').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('bridge').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('industry').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('decor').length).toBeGreaterThan(0);
  });

  it('categorizes top-level vehicle assets as vehicles', () => {
    const vehicleAssets = opengfxAssets.filter((asset) => asset.sourcePath.startsWith('graphics/vehicles/'));

    expect(vehicleAssets.length).toBeGreaterThan(0);
    expect(vehicleAssets.every((asset) => asset.category === 'vehicle')).toBe(true);
  });

  it('returns an empty list for unknown categories instead of throwing', () => {
    expect(assetsByCategory().get('missing-category')).toBeUndefined();
    expect(getAssetsForCategory('missing-category')).toEqual([]);
    expect(firstAssetPath('missing-category', '/fallback.png')).toBe('/fallback.png');
  });

  it('does not expose mutable category cache internals', () => {
    const terrainBefore = getAssetsForCategory('terrain').length;
    const vehiclesBefore = getAssetsForCategory('vehicle').length;

    assetsByCategory().delete('terrain');
    getAssetsForCategory('vehicle').length = 0;

    expect(getAssetsForCategory('terrain').length).toBe(terrainBefore);
    expect(getAssetsForCategory('vehicle').length).toBe(vehiclesBefore);
  });
});
