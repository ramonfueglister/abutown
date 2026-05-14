export type VehiclePathCoord = { x: number; y: number };
export type VehiclePathKeyLookup = {
  readonly size?: number;
  has(tileKey: string): boolean;
};

type VehicleRoadSegmentOptions = {
  roadKeys: VehiclePathKeyLookup;
  railKeys?: VehiclePathKeyLookup;
  minLength?: number;
  minLoopLength?: number;
  allowMirroredDeadEndLoops?: boolean;
  allowReversingLoops?: boolean;
};

export function makeNonDespawningVehicleLoop(path: readonly VehiclePathCoord[]): VehiclePathCoord[] {
  if (path.length <= 2 || areAdjacent(path[0], path[path.length - 1])) return path.map(copyCoord);
  return [
    ...path.map(copyCoord),
    ...path.slice(1, -1).reverse().map(copyCoord),
  ];
}

export function hasTeleportingVehicleSegment(path: readonly VehiclePathCoord[]): boolean {
  if (path.length <= 1) return false;
  for (let i = 0; i < path.length; i += 1) {
    const current = path[i];
    const next = path[(i + 1) % path.length];
    if (!areAdjacent(current, next)) return true;
  }
  return false;
}

export function hasIllegalVehicleUTurn(
  path: readonly VehiclePathCoord[],
  options: VehicleRoadSegmentOptions,
): boolean {
  if (path.length === 2) {
    return !isDeadEndVehicleRoadTile(path[0], options) || !isDeadEndVehicleRoadTile(path[1], options);
  }
  if (path.length < 3) return false;

  for (let i = 0; i < path.length; i += 1) {
    const previous = path[(i - 1 + path.length) % path.length];
    const current = path[i];
    const next = path[(i + 1) % path.length];
    if (key(previous) === key(next) && !isDeadEndVehicleRoadTile(current, options)) return true;
  }
  return false;
}

export function hasVehicleRouteReversal(path: readonly VehiclePathCoord[]): boolean {
  if (path.length < 3) return false;

  for (let i = 0; i < path.length; i += 1) {
    const previous = path[(i - 1 + path.length) % path.length];
    const current = path[i];
    const next = path[(i + 1) % path.length];
    const inboundX = Math.sign(current.x - previous.x);
    const inboundY = Math.sign(current.y - previous.y);
    const outboundX = Math.sign(next.x - current.x);
    const outboundY = Math.sign(next.y - current.y);
    if (inboundX === -outboundX && inboundY === -outboundY) return true;
  }
  return false;
}

export function splitVehicleRoadSegments(
  path: readonly VehiclePathCoord[],
  options: VehicleRoadSegmentOptions,
): VehiclePathCoord[][] {
  const minLength = options.minLength ?? 2;
  const result: VehiclePathCoord[][] = [];
  let segment: VehiclePathCoord[] = [];
  const flush = () => {
    if (segment.length >= minLength) result.push(segment);
    segment = [];
  };

  for (const coord of path) {
    if (!isVehicleRoadTile(coord, options)) {
      flush();
      continue;
    }
    if (segment.length > 0 && !areAdjacent(segment[segment.length - 1], coord)) flush();
    segment.push(copyCoord(coord));
  }
  flush();

  return result;
}

export function buildVehicleRoadLoops(
  paths: readonly (readonly VehiclePathCoord[])[],
  options: VehicleRoadSegmentOptions,
): VehiclePathCoord[][] {
  return paths.flatMap((path) =>
    splitVehicleRoadSegments(path, options).flatMap((segment) => buildVehicleLoopCandidates(segment, options)),
  );
}

function buildVehicleLoopCandidates(
  segment: readonly VehiclePathCoord[],
  options: VehicleRoadSegmentOptions,
): VehiclePathCoord[][] {
  const graphLoop = traceVehicleRoadLoop(segment, options);
  if (graphLoop && isUsableVehicleLoop(graphLoop, options)) return [graphLoop];

  if (options.allowMirroredDeadEndLoops === false) return [];

  const mirroredLoop = makeNonDespawningVehicleLoop(segment);
  if (isUsableVehicleLoop(mirroredLoop, options)) return [mirroredLoop];
  return [];
}

function isUsableVehicleLoop(path: readonly VehiclePathCoord[], options: VehicleRoadSegmentOptions): boolean {
  const minLoopLength = options.minLoopLength ?? 2;
  return path.length >= minLoopLength &&
    !hasTeleportingVehicleSegment(path) &&
    !hasIllegalVehicleUTurn(path, options) &&
    (options.allowReversingLoops !== false || !hasVehicleRouteReversal(path));
}

function traceVehicleRoadLoop(
  segment: readonly VehiclePathCoord[],
  options: VehicleRoadSegmentOptions,
): VehiclePathCoord[] | undefined {
  if (segment.length < 2) return undefined;
  const route = segment.map(copyCoord);
  const visited = new Set(route.map(key));
  const maxSteps = Math.max(segment.length * 8, (options.roadKeys.size ?? 32) * 2);

  for (let steps = 0; steps < maxSteps; steps += 1) {
    const current = route[route.length - 1];
    if (route.length > 2 && areAdjacent(current, route[0])) return route;

    const previous = route[route.length - 2];
    const next = chooseNextVehicleRoadStep(route, visited, options);
    if (!next) return undefined;

    if (key(next) === key(route[0])) {
      return route.length > 2 && !areSameTile(previous, route[0]) ? route : undefined;
    }

    route.push(next);
    visited.add(key(next));
  }

  return undefined;
}

function chooseNextVehicleRoadStep(
  route: readonly VehiclePathCoord[],
  visited: ReadonlySet<string>,
  options: VehicleRoadSegmentOptions,
): VehiclePathCoord | undefined {
  const previous = route[route.length - 2];
  const current = route[route.length - 1];
  const neighbors = vehicleRoadNeighbors(current, options).filter((candidate) => !areSameTile(candidate, previous));

  if (neighbors.length === 0) {
    return isDeadEndVehicleRoadTile(current, options) ? copyCoord(previous) : undefined;
  }

  const startKey = key(route[0]);
  const unvisited = neighbors.filter((candidate) => key(candidate) === startKey || !visited.has(key(candidate)));
  const candidates = unvisited.length > 0 ? unvisited : neighbors;

  return [...candidates].sort((a, b) => turnPreference(previous, current, a) - turnPreference(previous, current, b))[0];
}

function turnPreference(previous: VehiclePathCoord, current: VehiclePathCoord, candidate: VehiclePathCoord): number {
  const inboundX = Math.sign(current.x - previous.x);
  const inboundY = Math.sign(current.y - previous.y);
  const outboundX = Math.sign(candidate.x - current.x);
  const outboundY = Math.sign(candidate.y - current.y);

  if (outboundX === inboundX && outboundY === inboundY) return 0;
  if (inboundX * outboundY - inboundY * outboundX > 0) return 1;
  return 2;
}

function isDeadEndVehicleRoadTile(coord: VehiclePathCoord, options: VehicleRoadSegmentOptions): boolean {
  return vehicleRoadNeighbors(coord, options).length === 1;
}

function vehicleRoadNeighbors(coord: VehiclePathCoord, options: VehicleRoadSegmentOptions): VehiclePathCoord[] {
  return [
    { x: coord.x + 1, y: coord.y },
    { x: coord.x, y: coord.y + 1 },
    { x: coord.x - 1, y: coord.y },
    { x: coord.x, y: coord.y - 1 },
  ].filter((candidate) => isVehicleRoadTile(candidate, options));
}

function isVehicleRoadTile(coord: VehiclePathCoord, options: VehicleRoadSegmentOptions): boolean {
  const tileKey = key(coord);
  return options.roadKeys.has(tileKey) && !options.railKeys?.has(tileKey);
}

function areSameTile(a: VehiclePathCoord, b: VehiclePathCoord): boolean {
  return key(a) === key(b);
}

function areAdjacent(a: VehiclePathCoord, b: VehiclePathCoord): boolean {
  return Math.abs(a.x - b.x) + Math.abs(a.y - b.y) <= 1;
}

function copyCoord(coord: VehiclePathCoord): VehiclePathCoord {
  return { x: coord.x, y: coord.y };
}

function key(coord: VehiclePathCoord): string {
  return `${Math.round(coord.x)}:${Math.round(coord.y)}`;
}
