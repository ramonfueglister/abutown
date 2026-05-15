import { describe, expect, it } from 'vitest';
import {
  buildNorthboundTrainPath,
  trainFadeAlpha,
  trainPosition,
  trainWrappedOffset,
} from '../../src/render/trainMotion';

describe('train motion', () => {
  it('builds a northbound path that fades in below the south edge and out above the north edge', () => {
    const railPath = Array.from({ length: 4 }, (_, y) => ({ x: 82, y }));
    const path = buildNorthboundTrainPath(railPath, { fadeTiles: 2 });

    expect(path).toEqual([
      { x: 82, y: 5 },
      { x: 82, y: 4 },
      { x: 82, y: 3 },
      { x: 82, y: 2 },
      { x: 82, y: 1 },
      { x: 82, y: 0 },
      { x: 82, y: -1 },
      { x: 82, y: -2 },
    ]);
  });

  it('interpolates upward along the rail path', () => {
    const path = buildNorthboundTrainPath([
      { x: 82, y: 0 },
      { x: 82, y: 1 },
      { x: 82, y: 2 },
    ], { fadeTiles: 1 });

    expect(trainPosition(path, 1.5)).toEqual({ x: 82, y: 1.5 });
  });

  it('keeps the train invisible outside the fade bands and opaque on the map', () => {
    expect(trainFadeAlpha({ y: 267 }, { height: 256, fadeTiles: 12 })).toBe(0);
    expect(trainFadeAlpha({ y: 261 }, { height: 256, fadeTiles: 12 })).toBeCloseTo(0.5);
    expect(trainFadeAlpha({ y: 120 }, { height: 256, fadeTiles: 12 })).toBe(1);
    expect(trainFadeAlpha({ y: -6 }, { height: 256, fadeTiles: 12 })).toBeCloseTo(0.5);
    expect(trainFadeAlpha({ y: -12 }, { height: 256, fadeTiles: 12 })).toBe(0);
  });

  it('wraps only after the off-map fade-out segment', () => {
    const path = buildNorthboundTrainPath([
      { x: 82, y: 0 },
      { x: 82, y: 1 },
    ], { fadeTiles: 2 });

    expect(trainWrappedOffset(path.length + 1.2, path)).toBeCloseTo(1.2);
  });
});
