import type { Coord } from '../cameraController';
import { movementAngle } from './gridMath';
import { screenStableWorldSize } from './minimalGlyphScale';
import { screenRightLaneOffset } from './vehicleSprites';

export type CarRenderStyle = {
  lane: Coord;
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
    lane: screenRightLaneOffset(currentPoint, nextPoint, screenStableWorldSize(6.8, cameraScale, { minWorld: 6.8, maxWorld: 20 })),
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

export function pedestrianRenderStyle(
  currentPoint: Coord,
  nextPoint: Coord,
  cameraScale: number,
  laneOffset: number,
): PedestrianRenderStyle {
  return {
    lane: screenRightLaneOffset(
      currentPoint,
      nextPoint,
      screenStableWorldSize(4 + laneOffset, cameraScale, { minWorld: 4, maxWorld: 14 }),
    ),
    selectedRadius: screenStableWorldSize(8, cameraScale, { minWorld: 6.2, maxWorld: 22 }),
    radius: screenStableWorldSize(3.6, cameraScale, { minWorld: 2.9, maxWorld: 10 }),
  };
}
