import { describe, expect, it } from 'vitest';
import {
  RIVERBANK_EAST,
  RIVERBANK_NORTH,
  RIVERBANK_SOUTH,
  RIVERBANK_WEST,
  riverbankSourceFromMask,
  riverSurfaceSourceFromMask,
} from '../../src/render/riverbankFrames';

describe('riverbank frame mapping', () => {
  it('maps full water adjacency to the interior frame', () => {
    expect(riverbankSourceFromMask(RIVERBANK_NORTH | RIVERBANK_EAST | RIVERBANK_SOUTH | RIVERBANK_WEST)).toEqual({
      x: 384,
      y: 3200,
      width: 128,
      height: 128,
    });
  });

  it('keeps the full water adjacency frame renderable for river interiors', () => {
    expect(riverSurfaceSourceFromMask(RIVERBANK_NORTH | RIVERBANK_EAST | RIVERBANK_SOUTH | RIVERBANK_WEST)).toEqual({
      x: 384,
      y: 3200,
      width: 128,
      height: 128,
    });
  });

  it('maps exposed shore edges to side frames', () => {
    expect(riverbankSourceFromMask(RIVERBANK_NORTH | RIVERBANK_EAST | RIVERBANK_SOUTH)).toEqual({
      x: 0,
      y: 3328,
      width: 128,
      height: 128,
    });
    expect(riverbankSourceFromMask(RIVERBANK_EAST | RIVERBANK_SOUTH | RIVERBANK_WEST)).toEqual({
      x: 384,
      y: 3328,
      width: 128,
      height: 128,
    });
  });

  it('maps corner shore edges to corner frames', () => {
    expect(riverbankSourceFromMask(RIVERBANK_NORTH | RIVERBANK_EAST)).toEqual({
      x: 0,
      y: 3456,
      width: 128,
      height: 128,
    });
    expect(riverbankSourceFromMask(RIVERBANK_SOUTH | RIVERBANK_WEST)).toEqual({
      x: 384,
      y: 3456,
      width: 128,
      height: 128,
    });
  });
});
