export type Coord = { x: number; y: number };

export type CameraState = {
  x: number;
  y: number;
  scale: number;
  targetX: number;
  targetY: number;
  targetScale: number;
  dragging: boolean;
  lastX: number;
  lastY: number;
};

export type ViewportSize = {
  width: number;
  height: number;
};

export type GridBounds = {
  minX: number;
  maxX: number;
  minY: number;
  maxY: number;
  softness: number;
  allowOverscroll: boolean;
};

export type ZoomBounds = {
  minScale: number;
  maxScale: number;
};

export type CameraSpace = 'current' | 'target';

export function createCameraState(initial: { x: number; y: number; scale: number }): CameraState {
  return {
    x: initial.x,
    y: initial.y,
    scale: initial.scale,
    targetX: initial.x,
    targetY: initial.y,
    targetScale: initial.scale,
    dragging: false,
    lastX: 0,
    lastY: 0,
  };
}

export function screenToWorld(camera: CameraState, point: Coord, space: CameraSpace = 'current'): Coord {
  const x = space === 'target' ? camera.targetX : camera.x;
  const y = space === 'target' ? camera.targetY : camera.y;
  const scale = space === 'target' ? camera.targetScale : camera.scale;
  return { x: (point.x - x) / scale, y: (point.y - y) / scale };
}

export function focusCameraTargetOnGrid(
  camera: CameraState,
  coord: Coord,
  viewport: ViewportSize,
  gridToWorld: (coord: Coord) => Coord
): void {
  const point = gridToWorld(coord);
  camera.targetX = viewport.width / 2 - point.x * camera.targetScale;
  camera.targetY = viewport.height / 2 - point.y * camera.targetScale;
}

export function panCameraTarget(camera: CameraState, dx: number, dy: number): void {
  camera.targetX += dx;
  camera.targetY += dy;
}

export function zoomCameraAt(camera: CameraState, pointer: Coord, deltaY: number, deltaMode: number, bounds: ZoomBounds): void {
  const before = screenToWorld(camera, pointer, 'target');
  const lineHeight = deltaMode === 1 ? 18 : deltaMode === 2 ? 360 : 1;
  const normalizedDelta = deltaY * lineHeight;
  const nextScale = clamp(camera.targetScale * Math.exp(-normalizedDelta * 0.0015), bounds.minScale, bounds.maxScale);

  camera.targetScale = nextScale;
  camera.targetX = pointer.x - before.x * nextScale;
  camera.targetY = pointer.y - before.y * nextScale;
}

export function constrainCameraTargetToGrid(
  camera: CameraState,
  viewport: ViewportSize,
  worldToGrid: (point: Coord) => Coord,
  gridToWorld: (coord: Coord) => Coord,
  bounds: GridBounds
): void {
  const centerScreen = { x: viewport.width / 2, y: viewport.height / 2 };
  const centerGrid = worldToGrid(screenToWorld(camera, centerScreen, 'target'));
  const constrainedCenter = {
    x: constrainAxis(centerGrid.x, bounds.minX, bounds.maxX, bounds.softness, bounds.allowOverscroll),
    y: constrainAxis(centerGrid.y, bounds.minY, bounds.maxY, bounds.softness, bounds.allowOverscroll),
  };

  if (constrainedCenter.x === centerGrid.x && constrainedCenter.y === centerGrid.y) return;
  const point = gridToWorld(constrainedCenter);
  camera.targetX = centerScreen.x - point.x * camera.targetScale;
  camera.targetY = centerScreen.y - point.y * camera.targetScale;
}

export function dampCamera(camera: CameraState, dt: number, stiffness: number): void {
  const t = 1 - Math.exp(-stiffness * dt);
  camera.x = lerp(camera.x, camera.targetX, t);
  camera.y = lerp(camera.y, camera.targetY, t);
  camera.scale = lerp(camera.scale, camera.targetScale, t);

  if (Math.abs(camera.x - camera.targetX) < 0.01) camera.x = camera.targetX;
  if (Math.abs(camera.y - camera.targetY) < 0.01) camera.y = camera.targetY;
  if (Math.abs(camera.scale - camera.targetScale) < 0.0001) camera.scale = camera.targetScale;
}

function constrainAxis(value: number, min: number, max: number, softness: number, allowOverscroll: boolean): number {
  if (!allowOverscroll) return clamp(value, min, max);
  if (value < min) return min - softness * (1 - Math.exp((value - min) / softness));
  if (value > max) return max + softness * (1 - Math.exp((max - value) / softness));
  return value;
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}
