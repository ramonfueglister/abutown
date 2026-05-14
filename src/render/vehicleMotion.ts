export type VehicleMotionCoord = { x: number; y: number };

export type VehicleRenderPose = {
  position: VehicleMotionCoord;
  headingDelta: VehicleMotionCoord;
};

export const CURVE_EASING_WINDOW = 0.42;
export const TURN_SPEED_FACTOR = 0.56;
export const TURN_SPEED_LOOKAHEAD = 0.58;
export const TURN_SPEED_RECOVERY = 0.62;
export const JUNCTION_SPEED_FACTOR = 0.78;
export const JUNCTION_SPEED_LOOKAHEAD = 0.62;
export const JUNCTION_SPEED_RECOVERY = 0.42;
export const MIN_VEHICLE_GAP_TILES = 0.72;
export const VEHICLE_FOLLOWING_SLOWDOWN_DISTANCE_TILES = 1.5;

const QUARTER_TURN_CONTROL_WEIGHT = Math.SQRT1_2;
const CURVE_TANGENT_SAMPLE = 0.012;

type VehicleRenderPoseInput = {
  path: readonly VehicleMotionCoord[];
  offset: number;
};

type VehicleSpeedFactorInput = VehicleRenderPoseInput & {
  cautionTileKeys?: ReadonlySet<string>;
};

type VehicleFollowingSpeedFactorInput = {
  offset: number;
  leaderOffset?: number;
  pathLength: number;
};

export function vehicleRenderPose(input: VehicleRenderPoseInput): VehicleRenderPose {
  const path = input.path;
  if (path.length === 0) return { position: { x: 0, y: 0 }, headingDelta: { x: 0, y: 0 } };
  if (path.length === 1) return { position: copyCoord(path[0]), headingDelta: { x: 0, y: 0 } };

  const base = positiveModulo(Math.floor(input.offset), path.length);
  const t = input.offset - Math.floor(input.offset);
  const current = path[base];
  const next = path[(base + 1) % path.length];
  const linear = {
    x: lerp(current.x, next.x, t),
    y: lerp(current.y, next.y, t),
  };
  const segmentDelta = delta(current, next);

  const eased = curvePoseNearSegmentBoundary(path, base, t);
  if (eased) return eased;

  return {
    position: linear,
    headingDelta: segmentDelta,
  };
}

export function vehicleSpeedFactor(input: VehicleSpeedFactorInput): number {
  const path = input.path;
  if (path.length <= 1) return 1;

  let factor = 1;
  for (let index = 0; index < path.length; index += 1) {
    if (isRightAngleTurnAt(path, index)) {
      factor = Math.min(factor, speedFactorAroundIndex(
        input.offset,
        index,
        path.length,
        TURN_SPEED_LOOKAHEAD,
        TURN_SPEED_RECOVERY,
        TURN_SPEED_FACTOR,
      ));
    }
    if (input.cautionTileKeys?.has(key(path[index]))) {
      factor = Math.min(factor, speedFactorAroundIndex(
        input.offset,
        index,
        path.length,
        JUNCTION_SPEED_LOOKAHEAD,
        JUNCTION_SPEED_RECOVERY,
        JUNCTION_SPEED_FACTOR,
      ));
    }
  }
  return Number(factor.toFixed(3));
}

export function vehicleFollowingSpeedFactor(input: VehicleFollowingSpeedFactorInput): number {
  if (input.leaderOffset === undefined || input.pathLength <= 1) return 1;
  const distance = forwardDistance(input.offset, input.leaderOffset, input.pathLength);
  if (distance <= MIN_VEHICLE_GAP_TILES) return 0;
  if (distance >= VEHICLE_FOLLOWING_SLOWDOWN_DISTANCE_TILES) return 1;
  const progress = (distance - MIN_VEHICLE_GAP_TILES) /
    (VEHICLE_FOLLOWING_SLOWDOWN_DISTANCE_TILES - MIN_VEHICLE_GAP_TILES);
  return Number(smoothstep(clamp01(progress)).toFixed(3));
}

export function vehicleFollowingAdvanceLimit(input: VehicleFollowingSpeedFactorInput): number {
  if (input.leaderOffset === undefined || input.pathLength <= 1) return Number.POSITIVE_INFINITY;
  const distance = forwardDistance(input.offset, input.leaderOffset, input.pathLength);
  return Math.max(0, distance - MIN_VEHICLE_GAP_TILES);
}

function curvePoseNearSegmentBoundary(
  path: readonly VehicleMotionCoord[],
  base: number,
  t: number,
): VehicleRenderPose | undefined {
  if (t > 1 - CURVE_EASING_WINDOW) {
    const cornerIndex = (base + 1) % path.length;
    const pose = poseAroundCorner(path, cornerIndex, ((t - (1 - CURVE_EASING_WINDOW)) / CURVE_EASING_WINDOW) * 0.5);
    if (pose) return pose;
  }
  if (t < CURVE_EASING_WINDOW) {
    const cornerIndex = base;
    const pose = poseAroundCorner(path, cornerIndex, 0.5 + (t / CURVE_EASING_WINDOW) * 0.5);
    if (pose) return pose;
  }
  return undefined;
}

function poseAroundCorner(
  path: readonly VehicleMotionCoord[],
  cornerIndex: number,
  curveT: number,
): VehicleRenderPose | undefined {
  const previous = path[positiveModulo(cornerIndex - 1, path.length)];
  const corner = path[cornerIndex];
  const next = path[(cornerIndex + 1) % path.length];
  const incoming = delta(previous, corner);
  const outgoing = delta(corner, next);
  if (!isRightAngleTurn(incoming, outgoing)) return undefined;

  const start = {
    x: corner.x - incoming.x * CURVE_EASING_WINDOW,
    y: corner.y - incoming.y * CURVE_EASING_WINDOW,
  };
  const end = {
    x: corner.x + outgoing.x * CURVE_EASING_WINDOW,
    y: corner.y + outgoing.y * CURVE_EASING_WINDOW,
  };
  const position = rationalQuadraticPoint(start, corner, end, QUARTER_TURN_CONTROL_WEIGHT, curveT);
  const before = rationalQuadraticPoint(
    start,
    corner,
    end,
    QUARTER_TURN_CONTROL_WEIGHT,
    clamp01(curveT - CURVE_TANGENT_SAMPLE),
  );
  const after = rationalQuadraticPoint(
    start,
    corner,
    end,
    QUARTER_TURN_CONTROL_WEIGHT,
    clamp01(curveT + CURVE_TANGENT_SAMPLE),
  );
  const headingDelta = {
    x: after.x - before.x,
    y: after.y - before.y,
  };

  return { position, headingDelta };
}

function rationalQuadraticPoint(
  start: VehicleMotionCoord,
  control: VehicleMotionCoord,
  end: VehicleMotionCoord,
  controlWeight: number,
  t: number,
): VehicleMotionCoord {
  const oneMinusT = 1 - t;
  const startWeight = oneMinusT * oneMinusT;
  const middleWeight = 2 * oneMinusT * t * controlWeight;
  const endWeight = t * t;
  const denominator = startWeight + middleWeight + endWeight;
  return {
    x: (startWeight * start.x + middleWeight * control.x + endWeight * end.x) / denominator,
    y: (startWeight * start.y + middleWeight * control.y + endWeight * end.y) / denominator,
  };
}

function isRightAngleTurn(a: VehicleMotionCoord, b: VehicleMotionCoord): boolean {
  return manhattanLength(a) === 1 && manhattanLength(b) === 1 && dot(a, b) === 0;
}

function isRightAngleTurnAt(path: readonly VehicleMotionCoord[], cornerIndex: number): boolean {
  const previous = path[positiveModulo(cornerIndex - 1, path.length)];
  const corner = path[cornerIndex];
  const next = path[(cornerIndex + 1) % path.length];
  return isRightAngleTurn(delta(previous, corner), delta(corner, next));
}

function speedFactorAroundIndex(
  offset: number,
  targetIndex: number,
  pathLength: number,
  lookahead: number,
  recovery: number,
  minFactor: number,
): number {
  const ahead = forwardDistance(offset, targetIndex, pathLength);
  if (ahead <= lookahead) return interpolateSpeedFactor(ahead / lookahead, minFactor);
  const behind = forwardDistance(targetIndex, offset, pathLength);
  if (behind <= recovery) return interpolateSpeedFactor(behind / recovery, minFactor);
  return 1;
}

function interpolateSpeedFactor(progress: number, minFactor: number): number {
  const eased = smoothstep(clamp01(progress));
  return minFactor + (1 - minFactor) * eased;
}

function forwardDistance(from: number, to: number, pathLength: number): number {
  return positiveModuloFloat(to - from, pathLength);
}

function smoothstep(value: number): number {
  return value * value * (3 - 2 * value);
}

function clamp01(value: number): number {
  return Math.max(0, Math.min(1, value));
}

function delta(from: VehicleMotionCoord, to: VehicleMotionCoord): VehicleMotionCoord {
  return {
    x: Math.sign(to.x - from.x),
    y: Math.sign(to.y - from.y),
  };
}

function copyCoord(coord: VehicleMotionCoord): VehicleMotionCoord {
  return { x: coord.x, y: coord.y };
}

function positiveModulo(value: number, divisor: number): number {
  return ((value % divisor) + divisor) % divisor;
}

function positiveModuloFloat(value: number, divisor: number): number {
  return ((value % divisor) + divisor) % divisor;
}

function dot(a: VehicleMotionCoord, b: VehicleMotionCoord): number {
  return a.x * b.x + a.y * b.y;
}

function manhattanLength(coord: VehicleMotionCoord): number {
  return Math.abs(coord.x) + Math.abs(coord.y);
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

function key(coord: VehicleMotionCoord): string {
  return `${Math.round(coord.x)}:${Math.round(coord.y)}`;
}
