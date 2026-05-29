import { describe, expect, it } from 'vitest';
import {
  RAIL_CASING,
  RAIL_CORE,
  TRAIN_CORE,
  railLineStyle,
  roadLineStyle,
} from '../../src/render/transportStyle';

describe('transportStyle', () => {
  it('returns street road colors and screen-stable widths', () => {
    expect(roadLineStyle('street', 0.5)).toEqual({
      casing: '#c7d1cf',
      core: '#fffdf7',
      casingWidth: 9.6,
      coreWidth: 6.8,
    });
  });

  it('returns bridge road colors and wider screen-stable widths', () => {
    expect(roadLineStyle('bridge', 0.5)).toEqual({
      casing: '#8fc9d7',
      core: '#fff9e9',
      casingWidth: 11,
      coreWidth: 7.6,
    });
  });

  it('clamps rail widths through the same screen-stable sizing helper', () => {
    expect(railLineStyle(0.25)).toEqual({
      casing: RAIL_CASING,
      core: RAIL_CORE,
      casingWidth: 9,
      coreWidth: 4,
    });
  });

  it('keeps transport palette constants centralized', () => {
    expect(RAIL_CASING).toBe('rgba(122, 131, 135, 0.32)');
    expect(RAIL_CORE).toBe('rgba(122, 131, 135, 0.42)');
    expect(TRAIN_CORE).toBe('#5f6f75');
  });
});
