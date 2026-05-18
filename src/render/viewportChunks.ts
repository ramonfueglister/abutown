import { screenToWorld, type CameraState } from '../cameraController';
import type { ChunkCoordDto } from '../backend/mobilityProtocol';

export function chunkOf(x: number, y: number, chunkSize: number): { x: number; y: number } {
  return {
    x: Math.floor(x / chunkSize),
    y: Math.floor(y / chunkSize),
  };
}

export function visibleChunks(
  camera: CameraState,
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
    .map((p) => screenToWorld(camera, p))
    .map((w) => chunkOf(w.x, w.y, chunkSize));

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
