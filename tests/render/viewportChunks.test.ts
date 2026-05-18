import { describe, it, expect } from 'vitest';
import { chunkOf, visibleChunks } from '../../src/render/viewportChunks';

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

  // Identity projection: 1 screen pixel = 1 tile. Simulates a unit camera and
  // matches the assumption used throughout these unit tests.
  const identity = (screen: { x: number; y: number }) => ({ x: screen.x, y: screen.y });

  it('returns all 64 chunks when the entire world fits the viewport at identity projection, margin 0', () => {
    const result = visibleChunks(identity, viewport, world, chunkSize, 0);
    expect(result).toHaveLength(64);
    expect(result).toContainEqual({ x: 0, y: 0 });
    expect(result).toContainEqual({ x: 7, y: 7 });
  });

  it('clamps negative chunk indices to 0 when the projection puts the view past the world top-left', () => {
    const offsetNegative = (screen: { x: number; y: number }) => ({ x: screen.x - 64, y: screen.y - 64 });
    const result = visibleChunks(offsetNegative, viewport, world, chunkSize, 0);
    for (const c of result) {
      expect(c.x).toBeGreaterThanOrEqual(0);
      expect(c.y).toBeGreaterThanOrEqual(0);
    }
  });

  it('clamps past-end chunk indices to worldChunks-1', () => {
    const offsetFarPast = (screen: { x: number; y: number }) => ({ x: screen.x + 1000, y: screen.y + 1000 });
    const result = visibleChunks(offsetFarPast, viewport, world, chunkSize, 0);
    for (const c of result) {
      expect(c.x).toBeLessThanOrEqual(7);
      expect(c.y).toBeLessThanOrEqual(7);
    }
  });

  it('adds a 1-chunk ring when margin=1', () => {
    // Project the entire screen into a single chunk: divide screen px by 256
    // so every pixel lands at tile (0..1) → chunk (0,0).
    const oneChunk = (screen: { x: number; y: number }) => ({ x: screen.x / 256, y: screen.y / 256 });
    const zero = visibleChunks(oneChunk, viewport, world, chunkSize, 0);
    const one = visibleChunks(oneChunk, viewport, world, chunkSize, 1);
    expect(zero.length).toBeLessThan(one.length);
    const visibleChunksPerAxis = 1;
    const maxWithRing = (visibleChunksPerAxis + 1 * 2) * (visibleChunksPerAxis + 1 * 2); // (1 + 2*margin)^2 per axis
    expect(one.length).toBeLessThanOrEqual(maxWithRing);
  });

  it('emits no duplicate chunk coords', () => {
    const result = visibleChunks(identity, viewport, world, chunkSize, 1);
    const seen = new Set(result.map((c) => `${c.x},${c.y}`));
    expect(seen.size).toBe(result.length);
  });

  it('returns the projected chunks even when the projection is non-identity (regression: render-pixel-vs-tile-coord mismatch)', () => {
    // Composition: input screen pixel → render world pixel → tile coord.
    // Mimics the production isometric pipeline at small scale.
    const camera = { x: 100, y: 100, scale: 0.5 };
    const screenToTile = (screen: { x: number; y: number }) => {
      const worldX = (screen.x - camera.x) / camera.scale;
      const worldY = (screen.y - camera.y) / camera.scale;
      // 1 render-world pixel == 1 tile in this synthetic projection.
      return { x: worldX, y: worldY };
    };
    const result = visibleChunks(screenToTile, viewport, world, chunkSize, 0);
    // Non-empty: the camera sits at world(-200,-200) — the visible area at
    // scale 0.5 spans world(-200, 312), which intersects the [0..256) world.
    expect(result.length).toBeGreaterThan(0);
  });
});
