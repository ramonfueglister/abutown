export type PedestrianPathCoord = { x: number; y: number };

export function buildPedestrianLoop(path: PedestrianPathCoord[]): PedestrianPathCoord[] {
  if (path.length <= 2) return path;
  return [...path, ...path.slice(1, -1).reverse()];
}

export function pedestrianWalkingSpeed(index: number): number {
  return 0.14 + (index % 6) * 0.014;
}
