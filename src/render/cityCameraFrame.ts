import type { MarketLocationDto } from '../backend/mobilityProtocol';
import { ZOOM_CITY_MIN } from './designTokens';
import { mapProject } from './minimalMapProjection';
import type { RuntimeBuilding, RuntimeRoadTile } from './worldRuntimeTypes';

type Coord = { x: number; y: number };

export type ProjectedBounds = {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
};

export type CityCameraFrameInput = {
  viewport: { width: number; height: number };
  world: { width: number; height: number };
  tileSize: { width: number; height: number };
  markets: readonly MarketLocationDto[];
  buildings: readonly RuntimeBuilding[];
  roads: ReadonlyMap<string, RuntimeRoadTile>;
  paddingPx: number;
  minScale: number;
  maxScale: number;
};

export function cityCameraFrame(input: CityCameraFrameInput): { center: Coord; scale: number } {
  const bounds = cityProjectedBounds(input);
  return {
    center: boundsCenter(bounds),
    scale: cameraScaleForProjectedBounds(bounds, input.viewport, input.paddingPx, input.minScale, input.maxScale),
  };
}

export function cityStartMinScale(viewportMinScale: number): number {
  return Math.max(viewportMinScale, ZOOM_CITY_MIN);
}

export function cityProjectedBounds(input: {
  world: { width: number; height: number };
  tileSize: { width: number; height: number };
  markets: readonly MarketLocationDto[];
  buildings: readonly RuntimeBuilding[];
  roads: ReadonlyMap<string, RuntimeRoadTile>;
}): ProjectedBounds {
  const bounds = emptyBounds();
  let points = 0;

  for (const market of input.markets) {
    addTileToBounds(bounds, { x: market.tileX, y: market.tileY }, input.tileSize);
    points += 1;
  }
  for (const building of input.buildings) {
    addTileToBounds(bounds, building.coord, input.tileSize);
    points += 1;
  }
  for (const road of input.roads.values()) {
    addTileToBounds(bounds, road.coord, input.tileSize);
    points += 1;
  }

  if (points === 0) {
    return {
      minX: 0,
      minY: 0,
      maxX: input.world.width * input.tileSize.width,
      maxY: input.world.height * input.tileSize.height,
    };
  }

  return bounds;
}

export function cameraScaleForProjectedBounds(
  bounds: ProjectedBounds,
  viewport: { width: number; height: number },
  paddingPx: number,
  minScale: number,
  maxScale: number,
): number {
  const width = Math.max(1, bounds.maxX - bounds.minX);
  const height = Math.max(1, bounds.maxY - bounds.minY);
  const availableWidth = Math.max(1, viewport.width - paddingPx * 2);
  const availableHeight = Math.max(1, viewport.height - paddingPx * 2);
  return clamp(Math.min(availableWidth / width, availableHeight / height), minScale, maxScale);
}

export function boundsCenter(bounds: ProjectedBounds): Coord {
  return {
    x: (bounds.minX + bounds.maxX) / 2,
    y: (bounds.minY + bounds.maxY) / 2,
  };
}

function addTileToBounds(
  bounds: ProjectedBounds,
  coord: Coord,
  tileSize: { width: number; height: number },
): void {
  const point = mapProject(coord, tileSize);
  addPoint(bounds, { x: point.x - tileSize.width / 2, y: point.y - tileSize.height / 2 });
  addPoint(bounds, { x: point.x + tileSize.width / 2, y: point.y + tileSize.height / 2 });
}

function emptyBounds(): ProjectedBounds {
  return {
    minX: Number.POSITIVE_INFINITY,
    minY: Number.POSITIVE_INFINITY,
    maxX: Number.NEGATIVE_INFINITY,
    maxY: Number.NEGATIVE_INFINITY,
  };
}

function addPoint(bounds: ProjectedBounds, point: Coord): void {
  bounds.minX = Math.min(bounds.minX, point.x);
  bounds.minY = Math.min(bounds.minY, point.y);
  bounds.maxX = Math.max(bounds.maxX, point.x);
  bounds.maxY = Math.max(bounds.maxY, point.y);
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}
