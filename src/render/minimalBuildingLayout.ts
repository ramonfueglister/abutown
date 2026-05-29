export type Coord = { x: number; y: number };

export type RoadLike = {
  kind: string;
};

export type MinimalBuildingLike = {
  sheet: string;
  district: string;
};

const STREET_PULL = 3.0;
const CORNER_PULL = 2.0;

export function minimalBuildingPlotOffset(coord: Coord, roads: ReadonlyMap<string, RoadLike>): Coord {
  const adjacent = [
    { neighbor: { x: coord.x + 1, y: coord.y }, toward: { x: 1, y: 0 } },
    { neighbor: { x: coord.x - 1, y: coord.y }, toward: { x: -1, y: 0 } },
    { neighbor: { x: coord.x, y: coord.y + 1 }, toward: { x: 0, y: 1 } },
    { neighbor: { x: coord.x, y: coord.y - 1 }, toward: { x: 0, y: -1 } },
  ].filter(({ neighbor }) => roads.get(key(neighbor))?.kind === 'street');

  if (adjacent.length === 0) return { x: 0, y: 0 };

  const vector = adjacent.reduce(
    (sum, item) => ({ x: sum.x + item.toward.x, y: sum.y + item.toward.y }),
    { x: 0, y: 0 },
  );

  const magnitude = Math.hypot(vector.x, vector.y);
  if (magnitude < 0.001) return { x: 0, y: 0 };

  const distance = Math.abs(vector.x) > 0 && Math.abs(vector.y) > 0 ? CORNER_PULL : STREET_PULL;
  return {
    x: roundOne((vector.x / magnitude) * distance * (Math.abs(vector.x) > 0 && Math.abs(vector.y) > 0 ? Math.SQRT2 : 1)),
    y: roundOne((vector.y / magnitude) * distance * (Math.abs(vector.x) > 0 && Math.abs(vector.y) > 0 ? Math.SQRT2 : 1)),
  };
}

export function minimalBuildingSize(building: MinimalBuildingLike): { width: number; height: number } {
  if (building.district === 'mill-yard') return { width: 6.2, height: 5.8 };
  if (building.sheet === 'tower' || building.sheet === 'office') return { width: 6.6, height: 6.2 };
  if (building.sheet === 'modern' || building.sheet === 'flats' || building.sheet === 'shops') return { width: 6.0, height: 5.8 };
  if (building.sheet === 'oldhouses' || building.sheet === 'houses') return { width: 12.0, height: 10.8 };
  return { width: 5.4, height: 5.4 };
}

function key(coord: Coord): string {
  return `${Math.round(coord.x)}:${Math.round(coord.y)}`;
}

function roundOne(value: number): number {
  return Math.round(value * 10) / 10;
}
