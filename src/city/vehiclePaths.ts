export type VehiclePathCoord = { x: number; y: number };
export type VehiclePathKeyLookup = { has(tileKey: string): boolean };

type VehicleRoadSegmentOptions = {
  roadKeys: VehiclePathKeyLookup;
  railKeys?: VehiclePathKeyLookup;
  minLength?: number;
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
    splitVehicleRoadSegments(path, options).map((segment) => makeNonDespawningVehicleLoop(segment)),
  );
}

function isVehicleRoadTile(coord: VehiclePathCoord, options: VehicleRoadSegmentOptions): boolean {
  const tileKey = key(coord);
  return options.roadKeys.has(tileKey) && !options.railKeys?.has(tileKey);
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
