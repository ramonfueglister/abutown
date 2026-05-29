import { describe, expect, it } from 'vitest';
import { trainRenderSegments } from '../../src/render/trainRenderStyle';

describe('trainRenderStyle', () => {
  it('builds projected render segments for the engine and cars', () => {
    const path = Array.from({ length: 20 }, (_, y) => ({ x: 0, y }));

    const segments = trainRenderSegments(
      {
        path,
        offset: 4,
        carSpacing: 1,
        fadeTiles: 2,
      },
      { height: 20, project: (coord) => ({ x: coord.x * 10, y: coord.y * 2 }) },
    );

    expect(segments).toHaveLength(5);
    expect(segments.map((segment) => segment.length)).toEqual([13.5, 10.5, 10.5, 10.5, 10.5]);
    expect(segments.map((segment) => segment.point)).toEqual([
      { x: 0, y: 8 },
      { x: 0, y: 6 },
      { x: 0, y: 4 },
      { x: 0, y: 2 },
      { x: 0, y: 0 },
    ]);
    expect(segments[0]).toMatchObject({
      alpha: 1,
      width: 4.8,
    });
    expect(segments[0].angle).toBeCloseTo(Math.PI / 2);
  });

  it('drops fully faded segments while preserving partial fade alpha', () => {
    const path = Array.from({ length: 6 }, (_, y) => ({ x: 0, y: y + 7 }));

    const segments = trainRenderSegments(
      {
        path,
        offset: 4,
        carSpacing: 1,
        fadeTiles: 2,
      },
      { height: 10, project: (coord) => coord },
    );

    expect(segments).toHaveLength(4);
    expect(segments[0]).toMatchObject({
      point: { x: 0, y: 10 },
      alpha: 0.5,
      length: 10.5,
      width: 4.8,
    });
    expect(segments.slice(1).map((segment) => segment.alpha)).toEqual([1, 1, 1]);
  });
});
