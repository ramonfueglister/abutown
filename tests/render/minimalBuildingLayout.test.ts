import { describe, expect, it } from 'vitest';
import { minimalBuildingPlotOffset, minimalBuildingSize } from '../../src/render/minimalBuildingLayout';

function key(x: number, y: number): string {
  return `${x}:${y}`;
}

function roads(coords: Array<[number, number]>): Map<string, { coord: { x: number; y: number }; kind: 'street' }> {
  return new Map(coords.map(([x, y]) => [key(x, y), { coord: { x, y }, kind: 'street' }]));
}

describe('minimal building layout', () => {
  it('pushes plots away from adjacent streets in top-down screen space', () => {
    expect(minimalBuildingPlotOffset({ x: 4, y: 4 }, roads([[5, 4]]))).toEqual({ x: -3.2, y: 0 });
    expect(minimalBuildingPlotOffset({ x: 4, y: 4 }, roads([[3, 4]]))).toEqual({ x: 3.2, y: 0 });
    expect(minimalBuildingPlotOffset({ x: 4, y: 4 }, roads([[4, 5]]))).toEqual({ x: 0, y: -3.2 });
    expect(minimalBuildingPlotOffset({ x: 4, y: 4 }, roads([[4, 3]]))).toEqual({ x: 0, y: 3.2 });
  });

  it('centres corner plots inside the block instead of pulling them onto frontage roads', () => {
    expect(minimalBuildingPlotOffset({ x: 4, y: 4 }, roads([[5, 4], [4, 5]]))).toEqual({ x: -2.4, y: -2.4 });
    expect(minimalBuildingPlotOffset({ x: 4, y: 4 }, roads([[3, 4], [4, 3]]))).toEqual({ x: 2.4, y: 2.4 });
  });

  it('uses restrained top-down building footprints', () => {
    expect(minimalBuildingSize({ sheet: 'houses', district: 'old-town' })).toEqual({ width: 5.4, height: 5.4 });
    expect(minimalBuildingSize({ sheet: 'office', district: 'market' })).toEqual({ width: 6.6, height: 6.2 });
    expect(minimalBuildingSize({ sheet: 'shops', district: 'mill-yard' })).toEqual({ width: 6.2, height: 5.8 });
  });
});
