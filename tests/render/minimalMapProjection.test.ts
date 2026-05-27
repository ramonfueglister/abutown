import { describe, expect, it } from 'vitest';
import { MINIMAL_MAP_TILE_SIZE, mapProject, mapUnproject } from '../../src/render/minimalMapProjection';

describe('minimal map projection', () => {
  it('projects tile coordinates to centered top-down map pixels', () => {
    expect(MINIMAL_MAP_TILE_SIZE).toEqual({ width: 18, height: 18 });
    expect(mapProject({ x: 0, y: 0 })).toEqual({ x: 9, y: 9 });
    expect(mapProject({ x: 10, y: 4 })).toEqual({ x: 189, y: 81 });
  });

  it('round-trips projected points back to backend tile coordinates', () => {
    const coord = { x: 42.25, y: 58.75 };
    const projected = mapProject(coord);

    expect(mapUnproject(projected).x).toBeCloseTo(coord.x, 6);
    expect(mapUnproject(projected).y).toBeCloseTo(coord.y, 6);
  });
});
