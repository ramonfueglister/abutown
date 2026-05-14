import { describe, expect, it } from 'vitest';
import { opengfxAssets } from '../../src/assets/opengfxCatalog.generated';
import { assetsByCategory, getAssetsForCategory } from '../../src/assets/opengfxCatalog';

describe('OpenGFX catalog', () => {
  it('contains broad generated OpenGFX coverage', () => {
    expect(opengfxAssets.length).toBeGreaterThan(40);
    expect(new Set(opengfxAssets.map((asset) => asset.category)).size).toBeGreaterThanOrEqual(8);
  });

  it('exposes semantic categories for city composition', () => {
    expect(getAssetsForCategory('terrain').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('water').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('road').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('rail').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('building').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('tree').length).toBeGreaterThan(0);
  });

  it('returns an empty list for unknown categories instead of throwing', () => {
    expect(assetsByCategory().get('missing-category')).toBeUndefined();
    expect(getAssetsForCategory('missing-category')).toEqual([]);
  });
});
