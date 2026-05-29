import type { Coord } from '../cameraController';
import { movementAngle } from './gridMath';
import {
  trainFadeAlpha,
  trainPosition,
} from './trainMotion';

export type TrainRenderState = {
  path: Coord[];
  offset: number;
  carSpacing: number;
  fadeTiles: number;
};

export type TrainRenderOptions = {
  height: number;
  project: (coord: Coord) => Coord;
};

export type TrainRenderSegment = {
  point: Coord;
  angle: number;
  alpha: number;
  length: number;
  width: number;
};

const TRAIN_SEGMENT_WIDTH = 4.8;
const TRAIN_ANGLE_LOOKAHEAD = 0.2;
const TRAIN_SEGMENT_LAYOUT = [
  { carIndex: 0, length: 13.5 },
  { carIndex: 1, length: 10.5 },
  { carIndex: 2, length: 10.5 },
  { carIndex: 3, length: 10.5 },
  { carIndex: 4, length: 10.5 },
] as const;

export function trainRenderSegments(
  train: TrainRenderState,
  options: TrainRenderOptions,
): TrainRenderSegment[] {
  const result: TrainRenderSegment[] = [];

  for (const segment of TRAIN_SEGMENT_LAYOUT) {
    const offset = train.offset - train.carSpacing * segment.carIndex;
    const position = trainPosition(train.path, offset);
    const alpha = trainFadeAlpha(position, { height: options.height, fadeTiles: train.fadeTiles });
    if (alpha <= 0) continue;

    const point = options.project(position);
    const nextPoint = options.project(trainPosition(train.path, offset + TRAIN_ANGLE_LOOKAHEAD));
    result.push({
      point,
      angle: movementAngle(point, nextPoint),
      alpha,
      length: segment.length,
      width: TRAIN_SEGMENT_WIDTH,
    });
  }

  return result;
}
