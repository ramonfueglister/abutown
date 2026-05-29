import type { Coord } from '../cameraController';
import { compareDrawableOrder, type DrawableType } from './drawOrder';
import { isCoordVisibleInGridRect, type GridRect, type MapSize } from './cameraViewport';

export type GridDrawable = {
  type: DrawableType;
  coord: Coord;
};

export type PathEntity = {
  id: string;
  path: readonly Coord[];
};

export type CarDrawable<TCar extends PathEntity> = {
  type: 'car';
  coord: Coord;
  car: TCar;
  vehicleId: string;
};

export type PedestrianDrawable<TPedestrian extends PathEntity> = {
  type: 'pedestrian';
  coord: Coord;
  pedestrian: TPedestrian;
  agentId: string;
};

export type TrainDrawable<TTrain> = {
  type: 'train';
  coord: Coord;
  train: TTrain;
};

export function visibleTerrainCoords(
  rect: GridRect,
  map: MapSize,
  gridToWorld: (coord: Coord) => Coord,
): Coord[] {
  const coords: Coord[] = [];
  for (let y = Math.max(0, rect.minY); y <= Math.min(map.height - 1, rect.maxY); y += 1) {
    for (let x = Math.max(0, rect.minX); x <= Math.min(map.width - 1, rect.maxX); x += 1) {
      coords.push({ x, y });
    }
  }
  return coords.sort((a, b) => gridToWorld(a).y - gridToWorld(b).y || a.x - b.x);
}

export function buildVisibleCarDrawables<TCar extends PathEntity>(
  cars: readonly TCar[],
  rect: GridRect,
  gridToWorld: (coord: Coord) => Coord,
): Array<CarDrawable<TCar>> {
  return cars
    .map((car) => ({ type: 'car' as const, coord: car.path[0], car, vehicleId: car.id }))
    .filter((item) => isCoordVisibleInGridRect(item.coord, rect))
    .sort((a, b) => compareGridDrawables(a, b, gridToWorld));
}

export function buildVisiblePedestrianDrawables<TPedestrian extends PathEntity>(
  pedestrians: readonly TPedestrian[],
  rect: GridRect,
  gridToWorld: (coord: Coord) => Coord,
): Array<PedestrianDrawable<TPedestrian>> {
  return pedestrians
    .map((pedestrian) => ({ type: 'pedestrian' as const, coord: pedestrian.path[0], pedestrian, agentId: pedestrian.id }))
    .filter((item) => isCoordVisibleInGridRect(item.coord, rect))
    .sort((a, b) => compareGridDrawables(a, b, gridToWorld));
}

export function buildVisibleTrainDrawables<TTrain>(
  trains: readonly TTrain[],
  trainPosition: (train: TTrain) => Coord,
  rect: GridRect,
  gridToWorld: (coord: Coord) => Coord,
): Array<TrainDrawable<TTrain>> {
  return trains
    .map((train) => ({ type: 'train' as const, coord: trainPosition(train), train }))
    .filter((item) => isCoordVisibleInGridRect(item.coord, rect))
    .sort((a, b) => compareGridDrawables(a, b, gridToWorld));
}

export function compareGridDrawables(
  a: GridDrawable,
  b: GridDrawable,
  gridToWorld: (coord: Coord) => Coord,
): number {
  return compareDrawableOrder(
    { type: a.type, isoY: gridToWorld(a.coord).y, x: a.coord.x },
    { type: b.type, isoY: gridToWorld(b.coord).y, x: b.coord.x },
  );
}
