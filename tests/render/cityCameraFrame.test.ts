import { describe, expect, it } from 'vitest';
import type { MarketLocationDto } from '../../src/backend/mobilityProtocol';
import {
  cameraScaleForProjectedBounds,
  cityCameraFrame,
  cityProjectedBounds,
  cityStartMinScale,
} from '../../src/render/cityCameraFrame';
import { ZOOM_CITY_MIN } from '../../src/render/designTokens';
import { MINIMAL_MAP_TILE_SIZE } from '../../src/render/minimalMapProjection';
import type { RuntimeBuilding, RuntimeRoadTile } from '../../src/render/worldRuntimeTypes';

function market(marketId: number, name: string, tileX: number, tileY: number): MarketLocationDto {
  return { marketId, name, tileX, tileY, wagePaidLastTick: 0 };
}

describe('cityCameraFrame', () => {
  it('frames the authored city instead of the full backend world', () => {
    const roads = new Map<string, RuntimeRoadTile>();
    for (let x = 13; x <= 66; x += 1) {
      roads.set(`${x}:24`, { coord: { x, y: 24 }, kind: 'street', mask: 10 });
    }
    const buildings: RuntimeBuilding[] = [
      { coord: { x: 31, y: 21 }, sheet: 'modern', frame: 0, district: 'Central Works' },
      { coord: { x: 49, y: 28 }, sheet: 'shops', frame: 0, district: 'Market Square' },
    ];

    const frame = cityCameraFrame({
      viewport: { width: 600, height: 520 },
      world: { width: 80, height: 48 },
      tileSize: MINIMAL_MAP_TILE_SIZE,
      markets: [
        market(9001, 'Central Works', 8, 8),
        market(9002, 'Market Square', 72, 8),
        market(9003, 'Harbor Depot', 8, 40),
        market(9004, 'Homes Quarter', 72, 40),
      ],
      buildings,
      roads,
      paddingPx: 52,
      minScale: 0.18,
      maxScale: 2.8,
    });

    const fullWorldScale = (600 - 52 * 2) / (80 * MINIMAL_MAP_TILE_SIZE.width);
    expect(frame.scale).toBeGreaterThan(fullWorldScale);
    expect(frame.center.x).toBeCloseTo(729, 0);
    expect(frame.center.y).toBeCloseTo(441, 0);
  });

  it('falls back to the world bounds when no city anchors exist', () => {
    const bounds = cityProjectedBounds({
      world: { width: 80, height: 48 },
      tileSize: MINIMAL_MAP_TILE_SIZE,
      markets: [],
      buildings: [],
      roads: new Map(),
    });

    expect(bounds).toEqual({
      minX: 0,
      minY: 0,
      maxX: 80 * MINIMAL_MAP_TILE_SIZE.width,
      maxY: 48 * MINIMAL_MAP_TILE_SIZE.height,
    });
  });

  it('clamps the computed scale to camera limits', () => {
    expect(
      cameraScaleForProjectedBounds(
        { minX: 0, minY: 0, maxX: 10, maxY: 10 },
        { width: 1200, height: 800 },
        40,
        0.18,
        2.8,
      ),
    ).toBe(2.8);
  });

  it('keeps the default city start out of the economy overlay band on narrow viewports', () => {
    expect(cityStartMinScale(0.53)).toBe(ZOOM_CITY_MIN);
    expect(cityStartMinScale(1.25)).toBe(1.25);
  });
});
