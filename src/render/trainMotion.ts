export type TrainCoord = { x: number; y: number };

export type TrainPathOptions = {
  fadeTiles: number;
};

export function buildNorthboundTrainPath(railPath: TrainCoord[], options: TrainPathOptions): TrainCoord[] {
  if (railPath.length === 0) return [];
  const railX = railPath[0].x;
  const sortedY = railPath.map((coord) => coord.y).sort((a, b) => a - b);
  const northY = sortedY[0];
  const southY = sortedY[sortedY.length - 1];
  const path: TrainCoord[] = [];

  for (let y = southY + options.fadeTiles; y >= northY - options.fadeTiles; y -= 1) {
    path.push({ x: railX, y });
  }
  return path;
}

export function trainPosition(path: TrainCoord[], offset: number): TrainCoord {
  if (path.length === 0) return { x: 0, y: 0 };
  if (path.length === 1) return path[0];
  const wrapped = trainWrappedOffset(offset, path);
  const base = Math.floor(wrapped);
  const next = (base + 1) % path.length;
  const t = wrapped - base;

  return {
    x: path[base].x + (path[next].x - path[base].x) * t,
    y: path[base].y + (path[next].y - path[base].y) * t,
  };
}

export function trainFadeAlpha(position: { y: number }, options: { height: number; fadeTiles: number }): number {
  if (position.y > options.height - 1) {
    return clamp(1 - (position.y - (options.height - 1)) / options.fadeTiles, 0, 1);
  }
  if (position.y < 0) {
    return clamp(1 + position.y / options.fadeTiles, 0, 1);
  }
  return 1;
}

export function trainWrappedOffset(offset: number, path: TrainCoord[]): number {
  if (path.length === 0) return 0;
  return ((offset % path.length) + path.length) % path.length;
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}
