import type { Coord } from '../cameraController';
import type { TerrainBaseKind } from '../backend/terrainState';
import { distanceOutsideMap, isInsideMap, stableHash, type MapBounds } from './gridMath';

export const MAP_OUTSKIRTS = '#eee7d7';
export const MAP_WATER = '#92d8e9';
export const MAP_RIVERBANK = '#bde8df';
export const MAP_PARK = '#cfe5bf';
export const MAP_PLAZA = '#eadbbd';

export type TileFillStyle = {
  color: string;
  alpha: number;
};

export type OutskirtsTileStyle = {
  fill: TileFillStyle;
  shadowAlpha: number | null;
};

export function terrainBaseFill(base: TerrainBaseKind): TileFillStyle | null {
  if (base === 'Park' || base === 'Forest' || base === 'Reserve') return { color: MAP_PARK, alpha: 0.82 };
  if (base === 'Plaza') return { color: MAP_PLAZA, alpha: 0.72 };
  return null;
}

export function riverSurfaceFill(base: TerrainBaseKind): TileFillStyle {
  return { color: base === 'Riverbank' ? MAP_RIVERBANK : MAP_WATER, alpha: 0.96 };
}

export function outskirtsTileStyle(coord: Coord, map: MapBounds, outskirtsTiles: number): OutskirtsTileStyle | null {
  if (isInsideMap(coord, map)) return null;

  const edgeDistance = distanceOutsideMap(coord, map);
  if (edgeDistance > outskirtsTiles) return null;

  const fade = 1 - edgeDistance / (outskirtsTiles + 1);
  return {
    fill: { color: MAP_OUTSKIRTS, alpha: 0.05 + fade * 0.16 },
    shadowAlpha: stableHash(`outskirts-shadow:${coord.x}:${coord.y}`) % 11 === 0
      ? 0.025 + (1 - fade) * 0.035
      : null,
  };
}
