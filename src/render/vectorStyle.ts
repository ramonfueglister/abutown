import type { Coord } from '../cameraController';
import { coordKey, stableHash } from './gridMath';

export const BUILDING_RESIDENTIAL = '#d8cfbf';
export const BUILDING_COMMERCIAL = '#c9d8dc';
export const BUILDING_CIVIC = '#dccb9a';
export const BUILDING_INDUSTRIAL = '#cabed6';
export const VEHICLE_COLORS = ['#e85d75', '#3f8fc7', '#49a879', '#e5a944', '#8c73c8', '#ef7f5a', '#28a6b0'] as const;

export type BuildingStyleInput = {
  sheet: string;
  district: string;
  coord?: Coord;
};

export type TreeRenderStyleInput = {
  coord: Coord;
  cameraScale: number;
  terrainBase: string;
};

export type TreeRenderStyle = {
  visible: boolean;
  jitter: Coord;
  alpha: number;
};

export function buildingVectorColor(building: BuildingStyleInput): string {
  if (building.sheet === 'church') return BUILDING_CIVIC;
  if (building.sheet === 'office' || building.sheet === 'modern' || building.sheet === 'tower') return BUILDING_COMMERCIAL;
  if (building.district === 'mill-yard') return BUILDING_INDUSTRIAL;
  return BUILDING_RESIDENTIAL;
}

export function buildingJitter(building: Required<Pick<BuildingStyleInput, 'district' | 'coord'>>): Coord {
  return {
    x: ((stableHash(`building-jitter-x:${building.district}:${coordKey(building.coord)}`) % 5) - 2) * 0.26,
    y: ((stableHash(`building-jitter-y:${building.district}:${coordKey(building.coord)}`) % 5) - 2) * 0.26,
  };
}

export function vehicleVectorColor(id: string): string {
  return VEHICLE_COLORS[stableHash(`vehicle-color:${id}`) % VEHICLE_COLORS.length];
}

export function treeRenderStyle(input: TreeRenderStyleInput): TreeRenderStyle {
  if (input.cameraScale < 0.32 && stableHash(`tree-lod:${coordKey(input.coord)}`) % 3 !== 0) {
    return { visible: false, jitter: { x: 0, y: 0 }, alpha: 0 };
  }

  return {
    visible: true,
    jitter: {
      x: ((stableHash(`tree-x:${coordKey(input.coord)}`) % 9) - 4) * 0.38,
      y: ((stableHash(`tree-y:${coordKey(input.coord)}`) % 9) - 4) * 0.38,
    },
    alpha: input.terrainBase === 'Forest' ? 0.72 : 0.54,
  };
}
