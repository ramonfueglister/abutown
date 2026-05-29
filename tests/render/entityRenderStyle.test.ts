import { describe, expect, it } from 'vitest';
import {
  carRenderStyle,
  pedestrianRenderStyle,
} from '../../src/render/entityRenderStyle';

describe('entityRenderStyle', () => {
  it('computes car lane, selection ring, angle, and capsule size from projected points', () => {
    expect(carRenderStyle({ x: 0, y: 0 }, { x: 10, y: 0 }, 0.5)).toEqual({
      lane: { x: 0, y: 13.6 },
      angle: 0,
      selection: { x: 28, y: 20 },
      capsule: { length: 32, width: 12.8 },
    });
  });

  it('clamps car metrics when the camera is zoomed far out', () => {
    expect(carRenderStyle({ x: 0, y: 0 }, { x: 0, y: 10 }, 0.2)).toEqual({
      lane: { x: -20, y: 0 },
      angle: Math.PI / 2,
      selection: { x: 36, y: 28 },
      capsule: { length: 44, width: 19 },
    });
  });

  it('computes pedestrian lane, selection radius, and body radius from projected points', () => {
    expect(pedestrianRenderStyle({ x: 0, y: 0 }, { x: 10, y: 0 }, 0.5, 2)).toEqual({
      lane: { x: 0, y: 12 },
      selectedRadius: 16,
      radius: 7.2,
    });
  });

  it('returns zero pedestrian lane for stationary agents', () => {
    expect(pedestrianRenderStyle({ x: 4, y: 4 }, { x: 4, y: 4 }, 0.5, 3).lane).toEqual({ x: 0, y: 0 });
  });
});
