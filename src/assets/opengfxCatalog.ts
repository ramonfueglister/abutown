import { opengfxAssets, type OpenGfxAsset, type OpenGfxAssetCategory } from './opengfxCatalog.generated';

export function assetsByCategory(): Map<OpenGfxAssetCategory | string, OpenGfxAsset[]> {
  const result = new Map<OpenGfxAssetCategory | string, OpenGfxAsset[]>();
  for (const asset of opengfxAssets) {
    const assets = result.get(asset.category) ?? [];
    assets.push(asset);
    result.set(asset.category, assets);
  }
  return result;
}

export function getAssetsForCategory(category: OpenGfxAssetCategory | string): OpenGfxAsset[] {
  return assetsByCategory().get(category) ?? [];
}

export function firstAssetPath(category: OpenGfxAssetCategory | string, fallback: string): string {
  return getAssetsForCategory(category)[0]?.path ?? fallback;
}
