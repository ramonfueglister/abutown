import { describe, expect, it } from 'vitest';
import { stacItemUrls } from '../../scripts/geo/lib/stac.mjs';

const page = {
  features: [
    { id: 'swissalti3d_2019_2696-1262', assets: {
      a: { href: 'https://x/swissalti3d_2019_2696-1262_0.5_2056_5728.tif' },
      b: { href: 'https://x/swissalti3d_2019_2696-1262_2_2056_5728.tif' } } },
    { id: 'swissalti3d_2024_2696-1262', assets: {
      b: { href: 'https://x/swissalti3d_2024_2696-1262_2_2056_5728.tif' } } },
  ],
};

describe('stacItemUrls', () => {
  it('picks the 2m asset and the newest vintage per tile', () => {
    const urls = stacItemUrls({ pageJsonList: [page], assetSuffix: '_2_2056_5728.tif' });
    expect(urls).toEqual(['https://x/swissalti3d_2024_2696-1262_2_2056_5728.tif']);
  });
});
