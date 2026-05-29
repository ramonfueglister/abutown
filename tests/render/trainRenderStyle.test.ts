import { describe, expect, it } from 'vitest';
import { trainRenderSegments } from '../../src/render/trainRenderStyle';

describe('trainRenderStyle', () => {
  it('builds screen-space tram segments behind the head point', () => {
    expect(trainRenderSegments({ x: 10, y: 20 }, { x: 20, y: 20 })).toEqual([
      { point: { x: 10, y: 20 }, angle: 0, length: 13.5, width: 4.8 },
      { point: { x: 0.5, y: 20 }, angle: 0, length: 10.5, width: 4.8 },
      { point: { x: -9, y: 20 }, angle: 0, length: 10.5, width: 4.8 },
      { point: { x: -18.5, y: 20 }, angle: 0, length: 10.5, width: 4.8 },
      { point: { x: -28, y: 20 }, angle: 0, length: 10.5, width: 4.8 },
    ]);
  });

  it('uses the movement angle for all segments', () => {
    const segments = trainRenderSegments({ x: 0, y: 0 }, { x: 0, y: 10 });

    expect(segments).toHaveLength(5);
    expect(segments.map((segment) => segment.angle)).toEqual([
      Math.PI / 2,
      Math.PI / 2,
      Math.PI / 2,
      Math.PI / 2,
      Math.PI / 2,
    ]);
    expect(segments[1].point).toEqual({ x: 0, y: -9.5 });
  });
});
