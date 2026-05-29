import type { Coord } from '../cameraController';
import { movementAngle, screenForwardOffset } from './gridMath';

export type TrainRenderSegment = {
  point: Coord;
  angle: number;
  length: number;
  width: number;
};

const TRAIN_SEGMENT_WIDTH = 4.8;
const TRAIN_SEGMENT_LAYOUT = [
  { distance: 0, length: 13.5 },
  { distance: -9.5, length: 10.5 },
  { distance: -19, length: 10.5 },
  { distance: -28.5, length: 10.5 },
  { distance: -38, length: 10.5 },
] as const;

export function trainRenderSegments(head: Coord, next: Coord): TrainRenderSegment[] {
  const angle = movementAngle(head, next);
  return TRAIN_SEGMENT_LAYOUT.map((segment) => {
    const offset = screenForwardOffset(head, next, segment.distance);
    return {
      point: { x: head.x + offset.x, y: head.y + offset.y },
      angle,
      length: segment.length,
      width: TRAIN_SEGMENT_WIDTH,
    };
  });
}
