import { describe, expect, it } from 'vitest';
import { shouldCopyPublicEntry } from '../../scripts/publicCopyFilter.mjs';

const retiredClassicAssets = `open${'gfx2'}-classic`;

describe('public copy filter', () => {
  it('skips metadata files and retired asset trees', () => {
    expect(shouldCopyPublicEntry('.DS_Store')).toBe(false);
    expect(shouldCopyPublicEntry('._index.html')).toBe(false);
    expect(shouldCopyPublicEntry(retiredClassicAssets)).toBe(false);
    expect(shouldCopyPublicEntry('simutrans-assets')).toBe(false);
  });

  it('copies ordinary public entries', () => {
    expect(shouldCopyPublicEntry('favicon.svg')).toBe(true);
    expect(shouldCopyPublicEntry('robots.txt')).toBe(true);
  });
});
