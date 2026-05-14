import { describe, expect, test } from 'vitest';
import { getAssetFrame } from '../../src/render/assets';

describe('OpenGFX2 asset manifest lookup', () => {
  test('returns configured frame metadata for semantic asset keys', () => {
    expect(getAssetFrame('building.commercial')).toEqual({
      source: 'shopsandoffices_shape.png',
      frame: { x: 0, y: 0, w: 64, h: 96 },
      anchor: { x: 0.5, y: 0.78 },
    });
  });

  test('falls back to grass terrain for unknown asset keys', () => {
    expect(getAssetFrame('missing.asset')).toEqual({
      source: 'temperate_groundtiles_32bpp.png',
      frame: { x: 0, y: 0, w: 64, h: 42 },
      anchor: { x: 0.5, y: 0.38 },
    });
  });
});
