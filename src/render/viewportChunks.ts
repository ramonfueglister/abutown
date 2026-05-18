import type { ChunkCoordDto } from '../backend/mobilityProtocol';

export function chunkOf(x: number, y: number, chunkSize: number): { x: number; y: number } {
  return {
    x: Math.floor(x / chunkSize),
    y: Math.floor(y / chunkSize),
  };
}

/// Compute the set of chunks intersecting the viewport, plus a `margin` ring,
/// clamped to the world. The caller supplies `screenToTile` — a projection
/// from a CSS screen pixel to mobility tile coords (the same coordinate
/// system the backend uses for `Position` / `chunk_of`).
///
/// The isometric frontend composes `worldToGrid(screenToWorld(camera, p))`
/// for this; a unit-camera test can pass the identity.
export function visibleChunks(
  screenToTile: (screen: { x: number; y: number }) => { x: number; y: number },
  viewport: { width: number; height: number },
  world: { widthTiles: number; heightTiles: number },
  chunkSize: number,
  margin: number,
): ChunkCoordDto[] {
  const screenCorners = [
    { x: 0, y: 0 },
    { x: viewport.width, y: 0 },
    { x: 0, y: viewport.height },
    { x: viewport.width, y: viewport.height },
  ];
  const cornerChunks = screenCorners
    .map(screenToTile)
    .map((tile) => chunkOf(tile.x, tile.y, chunkSize));

  const xs = cornerChunks.map((c) => c.x);
  const ys = cornerChunks.map((c) => c.y);
  let minX = Math.min(...xs) - margin;
  let maxX = Math.max(...xs) + margin;
  let minY = Math.min(...ys) - margin;
  let maxY = Math.max(...ys) + margin;

  const worldChunksX = Math.ceil(world.widthTiles / chunkSize);
  const worldChunksY = Math.ceil(world.heightTiles / chunkSize);
  minX = Math.max(0, minX);
  minY = Math.max(0, minY);
  maxX = Math.min(worldChunksX - 1, maxX);
  maxY = Math.min(worldChunksY - 1, maxY);

  const out: ChunkCoordDto[] = [];
  if (maxX < minX || maxY < minY) {
    return out;
  }
  for (let y = minY; y <= maxY; y++) {
    for (let x = minX; x <= maxX; x++) {
      out.push({ x, y });
    }
  }
  return out;
}
