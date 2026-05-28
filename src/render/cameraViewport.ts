import {
  constrainCameraTargetToGrid,
  screenToWorld,
  type CameraState,
  type Coord,
  type ViewportSize,
} from '../cameraController';

export type GridRect = {
  minX: number;
  maxX: number;
  minY: number;
  maxY: number;
};

export type MapSize = {
  width: number;
  height: number;
};

export type CameraViewportBounds = {
  edgeMargin: number;
  edgeSoftness: number;
};

export type CameraFocusOptions = {
  verticalAnchor: number;
};

export function chooseInitialCameraFocus(coords: readonly Coord[], map: MapSize): Coord {
  const inside = coords.filter((coord) => coord.x >= 0 && coord.y >= 0 && coord.x < map.width && coord.y < map.height);
  if (inside.length === 0) return { x: Math.floor(map.width / 2), y: Math.floor(map.height / 2) };

  const x = inside.reduce((sum, coord) => sum + coord.x, 0) / inside.length;
  const y = inside.reduce((sum, coord) => sum + coord.y, 0) / inside.length;
  const center = { x: map.width / 2, y: map.height / 2 };
  return {
    x: center.x * 0.35 + x * 0.65,
    y: center.y * 0.35 + y * 0.65,
  };
}

export function initializeCameraForGridFocus(
  camera: CameraState,
  focusCoord: Coord,
  viewport: ViewportSize,
  gridToWorld: (coord: Coord) => Coord,
  options: CameraFocusOptions,
): void {
  const focus = gridToWorld(focusCoord);
  camera.targetX = viewport.width / 2 - focus.x * camera.targetScale;
  camera.targetY = viewport.height * options.verticalAnchor - focus.y * camera.targetScale;
  camera.x = camera.targetX;
  camera.y = camera.targetY;
  camera.scale = camera.targetScale;
}

export function constrainCameraToMap(
  camera: CameraState,
  viewport: ViewportSize,
  worldToGrid: (point: Coord) => Coord,
  gridToWorld: (coord: Coord) => Coord,
  map: MapSize,
  bounds: CameraViewportBounds & { allowOverscroll: boolean },
): void {
  constrainCameraTargetToGrid(camera, viewport, worldToGrid, gridToWorld, {
    minX: -bounds.edgeMargin,
    maxX: map.width - 1 + bounds.edgeMargin,
    minY: -bounds.edgeMargin,
    maxY: map.height - 1 + bounds.edgeMargin,
    softness: bounds.edgeSoftness,
    allowOverscroll: bounds.allowOverscroll,
  });
}

export function visibleGridRectForCamera(
  camera: CameraState,
  viewport: ViewportSize,
  worldToGrid: (point: Coord) => Coord,
  padding: number,
): GridRect {
  const corners = [
    worldToGrid(screenToWorld(camera, { x: 0, y: 0 })),
    worldToGrid(screenToWorld(camera, { x: viewport.width, y: 0 })),
    worldToGrid(screenToWorld(camera, { x: 0, y: viewport.height })),
    worldToGrid(screenToWorld(camera, { x: viewport.width, y: viewport.height })),
  ];
  return {
    minX: Math.floor(Math.min(...corners.map((coord) => coord.x))) - padding,
    maxX: Math.ceil(Math.max(...corners.map((coord) => coord.x))) + padding,
    minY: Math.floor(Math.min(...corners.map((coord) => coord.y))) - padding,
    maxY: Math.ceil(Math.max(...corners.map((coord) => coord.y))) + padding,
  };
}

export function isCoordVisibleInGridRect(coord: Coord, rect: GridRect): boolean {
  return coord.x >= rect.minX && coord.x <= rect.maxX && coord.y >= rect.minY && coord.y <= rect.maxY;
}
