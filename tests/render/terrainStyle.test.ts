import { describe, expect, it } from 'vitest';
import {
  outskirtsTileStyle,
  riverSurfaceFill,
  terrainBaseFill,
} from '../../src/render/terrainStyle';

describe('terrainStyle', () => {
  it('returns vector fills for park-like and plaza terrain bases', () => {
    expect(terrainBaseFill('Park')).toEqual({ color: '#cfe5bf', alpha: 0.82 });
    expect(terrainBaseFill('Forest')).toEqual({ color: '#cfe5bf', alpha: 0.82 });
    expect(terrainBaseFill('Reserve')).toEqual({ color: '#cfe5bf', alpha: 0.82 });
    expect(terrainBaseFill('Plaza')).toEqual({ color: '#eadbbd', alpha: 0.72 });
    expect(terrainBaseFill('Grass')).toBeNull();
  });

  it('chooses riverbank fill separately from open water fill', () => {
    expect(riverSurfaceFill('Riverbank')).toEqual({ color: '#bde8df', alpha: 0.96 });
    expect(riverSurfaceFill('Water')).toEqual({ color: '#92d8e9', alpha: 0.96 });
  });

  it('returns no outskirts style for playable or too-far coords', () => {
    const map = { width: 10, height: 10 };

    expect(outskirtsTileStyle({ x: 0, y: 0 }, map, 12)).toBeNull();
    expect(outskirtsTileStyle({ x: -13, y: 0 }, map, 12)).toBeNull();
  });

  it('returns faded outskirts fill and deterministic shadow alpha', () => {
    const style = outskirtsTileStyle({ x: -12, y: -1 }, { width: 10, height: 10 }, 12);

    expect(style?.fill.color).toBe('#eee7d7');
    expect(style?.fill.alpha).toBeCloseTo(0.0623076923);
    expect(style?.shadowAlpha).toBeCloseTo(0.0573076923);
  });

  it('omits shadow alpha when the deterministic hash does not select the coord', () => {
    const style = outskirtsTileStyle({ x: -1, y: 0 }, { width: 10, height: 10 }, 12);

    expect(style?.shadowAlpha).toBeNull();
    expect(style?.fill.alpha).toBeCloseTo(0.1976923077);
  });
});
