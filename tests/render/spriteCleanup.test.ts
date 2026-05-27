import { describe, expect, it } from 'vitest';
import { cleanupSpritePixels } from '../../src/render/spriteCleanup';

describe('sprite cleanup', () => {
  it('removes transparent source colors without path-specific cleanup', () => {
    const data = new Uint8ClampedArray([
      220, 10, 10, 255, 20, 20, 230, 255,
      80, 70, 60, 255, 255, 255, 255, 255,
    ]);

    cleanupSpritePixels({ data, width: 2, height: 2, path: '/test/vector-shape.png' });

    expect([...data.slice(0, 8)]).toEqual([220, 10, 10, 255, 0, 0, 0, 0]);
    expect([...data.slice(8, 16)]).toEqual([80, 70, 60, 255, 0, 0, 0, 0]);
  });

  it('does not clear ordinary opaque pixels', () => {
    const data = new Uint8ClampedArray([
      220, 10, 10, 255,
      80, 70, 60, 255,
    ]);

    cleanupSpritePixels({ data, width: 1, height: 2, path: '/test/vector-road.png' });

    expect([...data]).toEqual([220, 10, 10, 255, 80, 70, 60, 255]);
  });

  it('removes cyan sprite backgrounds', () => {
    const data = new Uint8ClampedArray([
      231, 255, 255, 255,
      40, 32, 24, 255,
    ]);

    cleanupSpritePixels({ data, width: 2, height: 1, path: '/test/legacy-cyan-background.png' });

    expect([...data]).toEqual([0, 0, 0, 0, 40, 32, 24, 255]);
  });
});
