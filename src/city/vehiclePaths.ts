export type VehiclePathCoord = { x: number; y: number };

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

function areAdjacent(a: VehiclePathCoord, b: VehiclePathCoord): boolean {
  return Math.abs(a.x - b.x) + Math.abs(a.y - b.y) <= 1;
}

function copyCoord(coord: VehiclePathCoord): VehiclePathCoord {
  return { x: coord.x, y: coord.y };
}
