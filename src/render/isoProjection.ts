// Shared isometric projection. The frontend places sprites at
// `iso({x, y})` in render-pixel space; `worldToGrid` reverses it for
// pointer hit-testing and viewport→chunk math.

export type Coord = { x: number; y: number };

export function iso(coord: Coord, tile: { width: number; height: number }): Coord {
  return {
    x: (coord.x - coord.y) * (tile.width / 2),
    y: (coord.x + coord.y) * (tile.height / 2),
  };
}

export function worldToGrid(point: Coord, tile: { width: number; height: number }): Coord {
  const projectedX = point.x / (tile.width / 2);
  const projectedY = point.y / (tile.height / 2);
  return {
    x: (projectedY + projectedX) / 2,
    y: (projectedY - projectedX) / 2,
  };
}
