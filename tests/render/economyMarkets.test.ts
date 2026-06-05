import { expect, it } from 'vitest';
import { visibleMarketGlyphs } from '../../src/render/minimalMapRenderer';

it('keeps only markets whose tile is within the visible grid rect', () => {
  const markets = [
    { marketId: 1, name: 'A', tileX: 5, tileY: 5, wagePaidLastTick: 0 },
    { marketId: 2, name: 'B', tileX: 999, tileY: 999, wagePaidLastTick: 0 },
  ];
  const grid = { minX: 0, maxX: 32, minY: 0, maxY: 32 };
  expect(visibleMarketGlyphs(markets, grid).map((m) => m.marketId)).toEqual([1]);
});

it('returns empty array when markets is undefined', () => {
  const grid = { minX: 0, maxX: 32, minY: 0, maxY: 32 };
  expect(visibleMarketGlyphs(undefined, grid)).toEqual([]);
});

it('returns all markets when all are within the visible rect', () => {
  const markets = [
    { marketId: 10, name: 'X', tileX: 1, tileY: 1, wagePaidLastTick: 100 },
    { marketId: 11, name: 'Y', tileX: 10, tileY: 10, wagePaidLastTick: 50 },
  ];
  const grid = { minX: 0, maxX: 32, minY: 0, maxY: 32 };
  expect(visibleMarketGlyphs(markets, grid).map((m) => m.marketId)).toEqual([10, 11]);
});
