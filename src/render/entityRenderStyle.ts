import type { Coord } from '../cameraController';
import type { BackendCar } from './backendMobilityDrawables';
import { movementAngle, screenForwardOffset, stableHash } from './gridMath';
import { screenStableWorldSize } from './minimalGlyphScale';
import { MINIMAL_MAP_TILE_SIZE, mapProject } from './minimalMapProjection';
import { screenRightLaneOffset } from './vehicleSprites';

export type CarRenderStyle = {
  angle: number;
  selection: { x: number; y: number };
  capsule: { length: number; width: number };
};

export type PedestrianRenderStyle = {
  lane: Coord;
  selectedRadius: number;
  radius: number;
};

export function carRenderStyle(currentPoint: Coord, nextPoint: Coord, cameraScale: number): CarRenderStyle {
  return {
    angle: movementAngle(currentPoint, nextPoint),
    selection: {
      x: screenStableWorldSize(14, cameraScale, { minWorld: 8.5, maxWorld: 36 }),
      y: screenStableWorldSize(10, cameraScale, { minWorld: 6.5, maxWorld: 28 }),
    },
    capsule: {
      length: screenStableWorldSize(16, cameraScale, { minWorld: 12.5, maxWorld: 44 }),
      width: screenStableWorldSize(6.4, cameraScale, { minWorld: 5.2, maxWorld: 19 }),
    },
  };
}

export function carVisualWorldPoint(
  car: BackendCar,
  cameraScale: number,
  tileSize: { width: number; height: number } = MINIMAL_MAP_TILE_SIZE,
): Coord {
  const current = car.path[0];
  const next = car.path[1] ?? current;
  const currentPoint = mapProject(current, tileSize);
  const nextPoint = mapProject(next, tileSize);
  const lane = screenRightLaneOffset(currentPoint, nextPoint, screenStableWorldSize(6.8, cameraScale, { minWorld: 6.8, maxWorld: 20 }));
  const spreadIndex = (stableHash(car.id) % 9) - 4;
  const spreadMagnitude = screenStableWorldSize(Math.abs(spreadIndex) * 4.2, cameraScale, { minWorld: 0, maxWorld: 42 });
  const along = screenForwardOffset(currentPoint, nextPoint, Math.sign(spreadIndex) * spreadMagnitude);
  return {
    x: currentPoint.x + lane.x + along.x,
    y: currentPoint.y + lane.y + along.y,
  };
}

export function pedestrianRenderStyle(
  currentPoint: Coord,
  nextPoint: Coord,
  cameraScale: number,
  laneOffset: number,
): PedestrianRenderStyle {
  const lanePixels = laneOffset <= 0
    ? 0
    : screenStableWorldSize(laneOffset, cameraScale, { minWorld: 0, maxWorld: 14 });
  return {
    lane: lanePixels === 0 ? { x: 0, y: 0 } : screenRightLaneOffset(currentPoint, nextPoint, lanePixels),
    selectedRadius: screenStableWorldSize(4.5, cameraScale, { minWorld: 3.2, maxWorld: 10 }),
    radius: screenStableWorldSize(1.6, cameraScale, { minWorld: 1.2, maxWorld: 3.2 }),
  };
}
