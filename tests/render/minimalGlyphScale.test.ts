import { describe, expect, it } from 'vitest';

import { screenStableWorldSize } from '../../src/render/minimalGlyphScale';

describe('screenStableWorldSize', () => {
  it('keeps tiny map glyphs readable at the default zoom', () => {
    expect(screenStableWorldSize(12, 0.32)).toBeCloseTo(37.5);
  });

  it('clamps glyphs so close zooms do not make vehicles too small in world space', () => {
    expect(screenStableWorldSize(12, 2.8)).toBe(10);
  });

  it('clamps glyphs so far zooms do not cover large road areas', () => {
    expect(screenStableWorldSize(12, 0.08)).toBe(48);
  });
});
