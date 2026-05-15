export type Coord = { x: number; y: number };

export type RoadLike = {
  kind: string;
};

export type BuildingLike = {
  coord: Coord;
};

export function hasDirectStreetAdjacency(coord: Coord, roads: ReadonlyMap<string, RoadLike>): boolean {
  return cardinal(coord).some((neighbor) => roads.get(key(neighbor))?.kind === 'street');
}

export function hasVisibleStreetFrontage(coord: Coord, roads: ReadonlyMap<string, RoadLike>): boolean {
  return [
    { x: coord.x + 1, y: coord.y },
    { x: coord.x, y: coord.y + 1 },
  ].some((neighbor) => roads.get(key(neighbor))?.kind === 'street');
}

export function buildingStreetFrontageOffset(coord: Coord, roads: ReadonlyMap<string, RoadLike>): Coord {
  const frontageVectors = [
    { neighbor: { x: coord.x + 1, y: coord.y }, offset: { x: 7, y: 4 } },
    { neighbor: { x: coord.x, y: coord.y + 1 }, offset: { x: -7, y: 4 } },
  ].filter(({ neighbor }) => roads.get(key(neighbor))?.kind === 'street');

  if (frontageVectors.length === 0) return { x: 0, y: 0 };
  if (frontageVectors.length === 2) return { x: 0, y: 7 };
  return frontageVectors[0].offset;
}

export function countBuildingsWithoutDirectStreetAdjacency(
  buildings: readonly BuildingLike[],
  roads: ReadonlyMap<string, RoadLike>,
): number {
  return buildings.filter((building) => !hasDirectStreetAdjacency(building.coord, roads)).length;
}

function cardinal(coord: Coord): Coord[] {
  return [
    { x: coord.x, y: coord.y - 1 },
    { x: coord.x + 1, y: coord.y },
    { x: coord.x, y: coord.y + 1 },
    { x: coord.x - 1, y: coord.y },
  ];
}

function key(coord: Coord): string {
  return `${Math.round(coord.x)}:${Math.round(coord.y)}`;
}
