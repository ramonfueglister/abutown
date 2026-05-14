import { describe, expect, it } from 'vitest';
import { cleanupSpritePixels } from '../../src/render/spriteCleanup';

describe('sprite cleanup', () => {
  it('removes transparent source colors and OpenGFX2 shape metadata rows', () => {
    const data = new Uint8ClampedArray([
      220, 10, 10, 255, 20, 20, 230, 255,
      80, 70, 60, 255, 255, 255, 255, 255,
    ]);

    cleanupSpritePixels({ data, width: 2, height: 2, path: '/opengfx2/houses_shape.png' });

    expect([...data.slice(0, 8)]).toEqual([0, 0, 0, 0, 0, 0, 0, 0]);
    expect([...data.slice(8, 16)]).toEqual([80, 70, 60, 255, 0, 0, 0, 0]);
  });

  it('does not clear the first row of non-shape atlases', () => {
    const data = new Uint8ClampedArray([
      220, 10, 10, 255,
      80, 70, 60, 255,
    ]);

    cleanupSpritePixels({ data, width: 1, height: 2, path: '/opengfx2/road_town_overlayalpha.png' });

    expect([...data]).toEqual([220, 10, 10, 255, 80, 70, 60, 255]);
  });
});
