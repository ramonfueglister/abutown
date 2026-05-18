import { describe, it, expect } from 'vitest';
import { chunkOf, visibleChunks } from '../../src/render/viewportChunks';
import { createCameraState } from '../../src/cameraController';

describe('chunkOf', () => {
  it('maps positive world coords to chunks via floor', () => {
    expect(chunkOf(0, 0, 32)).toEqual({ x: 0, y: 0 });
    expect(chunkOf(31.9, 31.9, 32)).toEqual({ x: 0, y: 0 });
    expect(chunkOf(32, 32, 32)).toEqual({ x: 1, y: 1 });
    expect(chunkOf(65, 100, 32)).toEqual({ x: 2, y: 3 });
  });

  it('maps negative world coords via floor (matches backend div_euclid)', () => {
    expect(chunkOf(-1, -1, 32)).toEqual({ x: -1, y: -1 });
    expect(chunkOf(-32, -32, 32)).toEqual({ x: -1, y: -1 });
    expect(chunkOf(-33, -33, 32)).toEqual({ x: -2, y: -2 });
  });
});

describe('visibleChunks', () => {
  const world = { widthTiles: 256, heightTiles: 256 };
  const chunkSize = 32;
  const viewport = { width: 256, height: 256 };

  it('returns the chunk under a camera centred on world origin at scale 1, with 0 margin', () => {
    const camera = createCameraState({ x: 0, y: 0, scale: 1 });
    const result = visibleChunks(camera, viewport, world, chunkSize, 0);
    expect(result).toHaveLength(64);
    expect(result).toContainEqual({ x: 0, y: 0 });
    expect(result).toContainEqual({ x: 7, y: 7 });
  });

  it('clamps negative chunk indices to 0 when camera is past the world top-left', () => {
    const camera = createCameraState({ x: 64, y: 64, scale: 1 });
    const result = visibleChunks(camera, viewport, world, chunkSize, 0);
    for (const c of result) {
      expect(c.x).toBeGreaterThanOrEqual(0);
      expect(c.y).toBeGreaterThanOrEqual(0);
    }
  });

  it('clamps past-end chunk indices to worldChunks-1', () => {
    const camera = createCameraState({ x: -1000, y: -1000, scale: 1 });
    const result = visibleChunks(camera, viewport, world, chunkSize, 0);
    for (const c of result) {
      expect(c.x).toBeLessThanOrEqual(7);
      expect(c.y).toBeLessThanOrEqual(7);
    }
  });

  it('adds a 1-chunk ring when margin=1', () => {
    const camera = createCameraState({ x: 128 - 16 * 32, y: 128 - 16 * 32, scale: 32 });
    const zero = visibleChunks(camera, viewport, world, chunkSize, 0);
    const one = visibleChunks(camera, viewport, world, chunkSize, 1);
    expect(zero.length).toBeLessThan(one.length);
    expect(one.length).toBeLessThanOrEqual(9);
  });

  it('emits no duplicate chunk coords', () => {
    const camera = createCameraState({ x: 0, y: 0, scale: 1 });
    const result = visibleChunks(camera, viewport, world, chunkSize, 1);
    const seen = new Set(result.map((c) => `${c.x},${c.y}`));
    expect(seen.size).toBe(result.length);
  });
});
