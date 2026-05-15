import { describe, expect, it } from 'vitest';
import { shouldRenderDetail } from '../../src/render/detailRenderPolicy';

describe('detail render policy', () => {
  it('does not render imported farm-field details that read as red-blue rectangles', () => {
    expect(shouldRenderDetail({ category: 'field', assetCategory: 'farm-field' })).toBe(false);
  });

  it('does not render rail station roofs and depot details', () => {
    expect(shouldRenderDetail({ category: 'station', assetCategory: 'station-roof' })).toBe(false);
    expect(shouldRenderDetail({ category: 'yard', assetCategory: 'rail-depot' })).toBe(false);
    expect(shouldRenderDetail({ category: 'station', assetCategory: 'road-stop' })).toBe(false);
  });

  it('keeps ordinary city details renderable', () => {
    expect(shouldRenderDetail({ category: 'decor', assetCategory: 'decor' })).toBe(true);
  });
});
