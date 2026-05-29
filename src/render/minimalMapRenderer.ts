import type { CameraState } from '../cameraController';
import type { ZurichDetail, ZurichTerrainKind } from '../city/worldTypes';
import type {
  RuntimeBuilding,
  RuntimeRailStation,
  RuntimeRailTile,
  RuntimeRoadTile,
  RuntimeTerrain,
} from '../app/zurichRuntimeContext';
import type { MobilityOverlayState } from '../backend/mobilityState';
import { shouldRenderDetail } from './detailRenderPolicy';
import { compareDrawableOrder } from './drawOrder';
import {
  carsFromMobilityState,
  pedestriansFromMobilityState,
  type BackendCar,
  type BackendPedestrian,
} from './backendMobilityDrawables';
import { minimalBuildingPlotOffset, minimalBuildingSize } from './minimalBuildingLayout';
import { screenStableWorldSize } from './minimalGlyphScale';
import { MINIMAL_MAP_TILE_SIZE, mapProject, mapUnproject } from './minimalMapProjection';
import { screenRightLaneOffset, type VehicleSprite } from './vehicleSprites';
import type { MinimalPedestrianSprite } from './minimalPedestrianSprites';

export type Coord = { x: number; y: number };

export type EntityInspectorRow = { label: string; value: string };
export type EntityInspector = { title: string; rows: EntityInspectorRow[] } | null;

export type MinimalMapRendererState = {
  ctx: CanvasRenderingContext2D;
  viewport: { width: number; height: number; devicePixelRatio: number };
  camera: CameraState;
  world: { width: number; height: number };
  tileSize: { width: number; height: number };
  terrain: ReadonlyMap<string, RuntimeTerrain>;
  terrainKinds: ReadonlyMap<string, { kind: ZurichTerrainKind }>;
  roads: ReadonlyMap<string, RuntimeRoadTile>;
  rails: ReadonlyMap<string, RuntimeRailTile>;
  railPaths: readonly Coord[][];
  railStations: readonly RuntimeRailStation[];
  buildings: readonly RuntimeBuilding[];
  trees: readonly Coord[];
  details: readonly ZurichDetail[];
  mobilityState: MobilityOverlayState;
  mobilityTickPeriodMs: number;
  vehicleSprites: readonly VehicleSprite[];
  pedestrianSprites: readonly MinimalPedestrianSprite[];
  selectedAgentId: string | null;
  selectedVehicleId: string | null;
  now: () => number;
};

type GridRect = {
  minX: number;
  maxX: number;
  minY: number;
  maxY: number;
};

type StaticDrawable =
  | { type: 'rail'; coord: Coord; rail: RuntimeRailTile }
  | { type: 'road'; coord: Coord; road: RuntimeRoadTile }
  | { type: 'railStation'; coord: Coord; station: RuntimeRailStation }
  | { type: 'detail'; coord: Coord; detail: ZurichDetail }
  | { type: 'tree'; coord: Coord }
  | { type: 'building'; coord: Coord; building: RuntimeBuilding };

type CarDrawable = { type: 'car'; coord: Coord; car: BackendCar; vehicleId: string };
type PedestrianDrawable = { type: 'pedestrian'; coord: Coord; pedestrian: BackendPedestrian; agentId: string };
type Drawable = StaticDrawable | CarDrawable | PedestrianDrawable;

export const MAP_BACKGROUND = '#f6f0e3';
const MAP_OUTSKIRTS = '#eee7d7';
const MAP_WATER = '#92d8e9';
const MAP_RIVERBANK = '#bde8df';
const MAP_PARK = '#cfe5bf';
const MAP_PLAZA = '#eadbbd';
const ROAD_CASING = '#c7d1cf';
const ROAD_CORE = '#fffdf7';
const ROAD_BRIDGE_CASING = '#8fc9d7';
const ROAD_BRIDGE_CORE = '#fff9e9';
const RAIL_CASING = 'rgba(122, 131, 135, 0.32)';
const RAIL_CORE = 'rgba(122, 131, 135, 0.42)';
const TREE_COLOR = '#84ad78';
const DETAIL_COLOR = 'rgba(92, 97, 92, 0.34)';
const BUILDING_RESIDENTIAL = '#d8cfbf';
const BUILDING_COMMERCIAL = '#c9d8dc';
const BUILDING_CIVIC = '#dccb9a';
const BUILDING_INDUSTRIAL = '#cabed6';
const AGENT_COLOR = '#343b43';
const VEHICLE_COLORS = ['#e85d75', '#3f8fc7', '#49a879', '#e5a944', '#8c73c8', '#ef7f5a', '#28a6b0'];
const VIEWPORT_GRID_PADDING = 9;
export const OUTSKIRTS_TILES = 12;
export const EDGE_EXIT_TILES = 7;
const NORTH = 1;
const EAST = 2;
const SOUTH = 4;
const WEST = 8;

export function renderMinimalMap(state: MinimalMapRendererState): void {
  const { ctx, camera, viewport } = state;
  ctx.save();
  ctx.setTransform(viewport.devicePixelRatio, 0, 0, viewport.devicePixelRatio, 0, 0);
  ctx.imageSmoothingEnabled = true;
  ctx.fillStyle = MAP_BACKGROUND;
  ctx.fillRect(0, 0, viewport.width, viewport.height);
  ctx.translate(camera.x, camera.y);
  ctx.scale(camera.scale, camera.scale);

  drawScene(state, { x: 0, y: 0 });
  ctx.restore();
  drawAgentInspectorPanel(state, buildBackendPedestrianInspector(selectedBackendPedestrian(state)));
  drawCarInspectorPanel(state, buildBackendCarInspector(selectedBackendCar(state)));
}

function drawScene(state: MinimalMapRendererState, offset: Coord): void {
  const { ctx, world } = state;
  ctx.save();
  const sceneOffset = iso(state, offset);
  ctx.translate(sceneOffset.x, sceneOffset.y);
  const visibleGrid = visibleGridRect(state);

  drawOutskirtsTerrain(state, visibleGrid);
  const visibleTerrainTiles: Coord[] = [];
  for (let y = Math.max(0, visibleGrid.minY); y <= Math.min(world.height - 1, visibleGrid.maxY); y += 1) {
    for (let x = Math.max(0, visibleGrid.minX); x <= Math.min(world.width - 1, visibleGrid.maxX); x += 1) visibleTerrainTiles.push({ x, y });
  }
  visibleTerrainTiles.sort((a, b) => iso(state, a).y - iso(state, b).y || a.x - b.x);
  for (const coord of visibleTerrainTiles) drawTerrainBase(state, coord);
  for (const coord of visibleTerrainTiles) drawRiverSurface(state, coord);

  const pedestrians: BackendPedestrian[] = pedestriansFromMobilityState(
    state.mobilityState,
    state.pedestrianSprites,
    state.now(),
    state.mobilityTickPeriodMs,
  );
  const cars: BackendCar[] = carsFromMobilityState(
    state.mobilityState,
    state.vehicleSprites,
    state.now(),
    state.mobilityTickPeriodMs,
  );
  const carDrawables = cars
    .map((car) => ({ type: 'car' as const, coord: car.path[0], car, vehicleId: car.id }))
    .filter((item) => isCoordVisible(item.coord, visibleGrid))
    .sort((a, b) => compareDrawables(state, a, b));
  const pedestrianDrawables = pedestrians
    .map((pedestrian) => ({ type: 'pedestrian' as const, coord: pedestrian.path[0], pedestrian, agentId: pedestrian.id }))
    .filter((item) => isCoordVisible(item.coord, visibleGrid))
    .sort((a, b) => compareDrawables(state, a, b));
  for (const road of state.roads.values()) if (isCoordVisible(road.coord, visibleGrid)) drawRoad(state, road);
  for (const path of state.railPaths) drawRailPath(state, path);
  drawEdgeConnections(state, visibleGrid);
  for (const station of state.railStations) if (isCoordVisible(station.coord, visibleGrid)) drawRailStation(state, station);
  for (const detail of state.details) if (isCoordVisible(detail.coord, visibleGrid)) drawDetail(state, detail);
  for (const building of state.buildings) if (isCoordVisible(building.coord, visibleGrid)) drawBuilding(state, building);
  for (const coord of state.trees) if (isCoordVisible(coord, visibleGrid)) drawTree(state, coord);
  for (const item of carDrawables) drawCar(state, item.car, item.vehicleId === state.selectedVehicleId);
  for (const item of pedestrianDrawables) drawPedestrian(state, item.pedestrian, item.agentId === state.selectedAgentId);

  drawPerimeterMist(state);
  ctx.restore();
}

function drawTerrainBase(state: MinimalMapRendererState, coord: Coord): void {
  const kind = state.terrainKinds.get(key(coord))?.kind;
  if (kind === 'park' || kind === 'forest' || kind === 'reserve') {
    drawTileFill(state, coord, MAP_PARK, 0.82);
  } else if (kind === 'plaza') {
    drawTileFill(state, coord, MAP_PLAZA, 0.72);
  }
}

function drawRiverSurface(state: MinimalMapRendererState, coord: Coord): void {
  if (!isWaterSurface(state, coord)) return;
  const kind = state.terrainKinds.get(key(coord))?.kind;
  drawTileFill(state, coord, kind === 'riverbank' ? MAP_RIVERBANK : MAP_WATER, 0.96);
}

function drawOutskirtsTerrain(state: MinimalMapRendererState, visibleGrid: GridRect): void {
  const { ctx, world } = state;
  for (let y = Math.max(-OUTSKIRTS_TILES, visibleGrid.minY); y <= Math.min(world.height - 1 + OUTSKIRTS_TILES, visibleGrid.maxY); y += 1) {
    for (let x = Math.max(-OUTSKIRTS_TILES, visibleGrid.minX); x <= Math.min(world.width - 1 + OUTSKIRTS_TILES, visibleGrid.maxX); x += 1) {
      const coord = { x, y };
      if (isInsidePlayableMap(state, coord)) continue;
      const edgeDistance = distanceOutsidePlayableMap(state, coord);
      if (edgeDistance > OUTSKIRTS_TILES) continue;

      const fade = 1 - edgeDistance / (OUTSKIRTS_TILES + 1);
      ctx.save();
      drawTileFill(state, coord, MAP_OUTSKIRTS, 0.05 + fade * 0.16);
      if (hash(`outskirts-shadow:${x}:${y}`) % 11 === 0) {
        const point = iso(state, coord);
        ctx.fillStyle = `rgba(151, 133, 103, ${0.025 + (1 - fade) * 0.035})`;
        drawIsoTile(state, point);
      }
      ctx.restore();
    }
  }
}

function drawRoad(state: MinimalMapRendererState, road: RuntimeRoadTile): void {
  drawMaskLine(state, road.coord, road.mask, {
    casing: road.kind === 'bridge' ? ROAD_BRIDGE_CASING : ROAD_CASING,
    core: road.kind === 'bridge' ? ROAD_BRIDGE_CORE : ROAD_CORE,
    casingWidth: road.kind === 'bridge'
      ? screenStableWorldSize(5.5, state.camera.scale, { minWorld: 10.5, maxWorld: 17 })
      : screenStableWorldSize(4.8, state.camera.scale, { minWorld: 9.2, maxWorld: 16 }),
    coreWidth: road.kind === 'bridge'
      ? screenStableWorldSize(3.8, state.camera.scale, { minWorld: 7, maxWorld: 12 })
      : screenStableWorldSize(3.4, state.camera.scale, { minWorld: 6.4, maxWorld: 10.5 }),
  });
}

function drawRail(state: MinimalMapRendererState, rail: RuntimeRailTile): void {
  drawMaskLine(state, rail.coord, rail.mask, {
    casing: RAIL_CASING,
    core: RAIL_CORE,
    casingWidth: screenStableWorldSize(2.8, state.camera.scale, { minWorld: 4.8, maxWorld: 9 }),
    coreWidth: screenStableWorldSize(1.2, state.camera.scale, { minWorld: 1.8, maxWorld: 4 }),
  });
}

function drawRailPath(state: MinimalMapRendererState, path: readonly Coord[]): void {
  if (path.length < 2) return;
  const { ctx } = state;
  ctx.save();
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  drawPolyline(state, path, RAIL_CASING, screenStableWorldSize(2.8, state.camera.scale, { minWorld: 4.8, maxWorld: 9 }));
  drawPolyline(state, path, RAIL_CORE, screenStableWorldSize(1.2, state.camera.scale, { minWorld: 1.8, maxWorld: 4 }));
  ctx.restore();
}

function drawPolyline(state: MinimalMapRendererState, path: readonly Coord[], color: string, width: number): void {
  const { ctx } = state;
  ctx.strokeStyle = color;
  ctx.lineWidth = width;
  ctx.beginPath();
  path.forEach((coord, index) => {
    const point = iso(state, coord);
    if (index === 0) ctx.moveTo(point.x, point.y);
    else ctx.lineTo(point.x, point.y);
  });
  ctx.stroke();
}

function drawRailStation(state: MinimalMapRendererState, station: RuntimeRailStation): void {
  const { ctx } = state;
  const point = iso(state, station.coord);
  ctx.save();
  ctx.fillStyle = 'rgba(255, 250, 240, 0.74)';
  ctx.strokeStyle = RAIL_CORE;
  ctx.lineWidth = 1.4;
  ctx.beginPath();
  ctx.arc(point.x, point.y, 5.5 + (station.frame % 2) * 0.5, 0, Math.PI * 2);
  ctx.fill();
  ctx.stroke();
  ctx.restore();
}

function drawDetail(state: MinimalMapRendererState, detail: ZurichDetail): void {
  if (!shouldRenderDetail(detail)) return;
  if (detail.category !== 'industry' && detail.category !== 'dock' && detail.category !== 'station') return;
  const { ctx } = state;
  const point = iso(state, detail.coord);
  ctx.save();
  ctx.fillStyle = DETAIL_COLOR;
  ctx.fillRect(point.x - 2, point.y - 2, 4, 4);
  ctx.restore();
}

function drawBuilding(state: MinimalMapRendererState, building: RuntimeBuilding): void {
  const { ctx } = state;
  const point = iso(state, building.coord);
  const offset = minimalBuildingPlotOffset(building.coord, state.roads);
  const { width, height } = minimalBuildingSize(building);
  const jitter = buildingJitter(building);
  const x = point.x - width / 2 + offset.x + jitter.x;
  const y = point.y - height / 2 + offset.y + jitter.y;
  ctx.save();
  ctx.fillStyle = 'rgba(108, 97, 77, 0.07)';
  roundedRect(state, x + 1.5, y + 1.5, width, height, 1.4);
  ctx.fill();
  ctx.globalAlpha = 0.66;
  ctx.fillStyle = buildingVectorColor(building);
  roundedRect(state, x, y, width, height, 1.4);
  ctx.fill();
  ctx.restore();
}

function buildingJitter(building: RuntimeBuilding): Coord {
  return {
    x: ((hash(`building-jitter-x:${building.district}:${key(building.coord)}`) % 5) - 2) * 0.26,
    y: ((hash(`building-jitter-y:${building.district}:${key(building.coord)}`) % 5) - 2) * 0.26,
  };
}

function drawTree(state: MinimalMapRendererState, coord: Coord): void {
  if (state.camera.scale < 0.32 && hash(`tree-lod:${key(coord)}`) % 3 !== 0) return;
  const { ctx } = state;
  const point = iso(state, coord);
  const jitterX = ((hash(`tree-x:${key(coord)}`) % 9) - 4) * 0.38;
  const jitterY = ((hash(`tree-y:${key(coord)}`) % 9) - 4) * 0.38;
  ctx.save();
  ctx.fillStyle = TREE_COLOR;
  ctx.globalAlpha = state.terrainKinds.get(key(coord))?.kind === 'forest' ? 0.72 : 0.54;
  ctx.beginPath();
  ctx.arc(point.x + jitterX, point.y + jitterY, 2.4, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

function drawTileFill(state: MinimalMapRendererState, coord: Coord, color: string, alpha = 1): void {
  const { ctx, tileSize } = state;
  const point = iso(state, coord);
  ctx.save();
  ctx.globalAlpha *= alpha;
  ctx.fillStyle = color;
  ctx.fillRect(point.x - tileSize.width / 2 - 0.6, point.y - tileSize.height / 2 - 0.6, tileSize.width + 1.2, tileSize.height + 1.2);
  ctx.restore();
}

function drawMaskLine(
  state: MinimalMapRendererState,
  coord: Coord,
  mask: number,
  style: { casing: string; core: string; casingWidth: number; coreWidth: number },
): void {
  const { ctx } = state;
  const point = iso(state, coord);
  const segments = maskSegments(state, mask);
  ctx.save();
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  drawRoadPass(state, point, segments, style.casing, style.casingWidth);
  drawRoadPass(state, point, segments, style.core, style.coreWidth);
  ctx.restore();
}

function drawRoadPass(state: MinimalMapRendererState, point: Coord, segments: Coord[], color: string, width: number): void {
  const { ctx } = state;
  ctx.strokeStyle = color;
  ctx.lineWidth = width;
  ctx.beginPath();
  if (segments.length === 0) {
    ctx.arc(point.x, point.y, width / 2, 0, Math.PI * 2);
  } else {
    for (const segment of segments) {
      ctx.moveTo(point.x, point.y);
      ctx.lineTo(point.x + segment.x, point.y + segment.y);
    }
  }
  ctx.stroke();
}

function maskSegments(state: MinimalMapRendererState, mask: number): Coord[] {
  const { tileSize } = state;
  const result: Coord[] = [];
  if ((mask & NORTH) !== 0) result.push({ x: 0, y: -tileSize.height / 2 });
  if ((mask & EAST) !== 0) result.push({ x: tileSize.width / 2, y: 0 });
  if ((mask & SOUTH) !== 0) result.push({ x: 0, y: tileSize.height / 2 });
  if ((mask & WEST) !== 0) result.push({ x: -tileSize.width / 2, y: 0 });
  return result;
}

function buildingVectorColor(building: RuntimeBuilding): string {
  if (building.sheet === 'church') return BUILDING_CIVIC;
  if (building.sheet === 'office' || building.sheet === 'modern' || building.sheet === 'tower') return BUILDING_COMMERCIAL;
  if (building.district === 'mill-yard') return BUILDING_INDUSTRIAL;
  return BUILDING_RESIDENTIAL;
}

function drawCar(state: MinimalMapRendererState, car: BackendCar, selected: boolean): void {
  const { ctx, camera, tileSize } = state;
  const point = carVisualWorldPoint(car, camera.scale, tileSize);
  const currentPoint = iso(state, car.path[0]);
  const nextPoint = iso(state, car.path[1] ?? car.path[0]);
  const angle = movementAngle(currentPoint, nextPoint);
  const selectX = screenStableWorldSize(14, camera.scale, { minWorld: 8.5, maxWorld: 36 });
  const selectY = screenStableWorldSize(10, camera.scale, { minWorld: 6.5, maxWorld: 28 });
  const length = screenStableWorldSize(16, camera.scale, { minWorld: 12.5, maxWorld: 44 });
  const width = screenStableWorldSize(6.4, camera.scale, { minWorld: 5.2, maxWorld: 19 });
  ctx.save();
  ctx.translate(point.x, point.y);
  if (selected) {
    ctx.globalAlpha = 0.94;
    ctx.strokeStyle = '#166c83';
    ctx.lineWidth = 2 / Math.max(0.75, camera.scale);
    ctx.beginPath();
    ctx.ellipse(0, 0, selectX, selectY, 0, 0, Math.PI * 2);
    ctx.stroke();
  }
  drawCapsule(state, { x: 0, y: 0 }, angle, length, width, vehicleVectorColor(car.id));
  ctx.restore();
}

export function carVisualWorldPoint(
  car: BackendCar,
  cameraScale: number,
  tileSize: { width: number; height: number } = MINIMAL_MAP_TILE_SIZE,
): Coord {
  const current = car.path[0];
  const next = car.path[1] ?? current;
  const currentPoint = mapProject(current, tileSize);
  const nextPoint = mapProject(next, tileSize);
  const lane = screenRightLaneOffset(currentPoint, nextPoint, screenStableWorldSize(6.8, cameraScale, { minWorld: 6.8, maxWorld: 20 }));
  const spreadIndex = (hash(car.id) % 9) - 4;
  const spreadMagnitude = screenStableWorldSize(Math.abs(spreadIndex) * 4.2, cameraScale, { minWorld: 0, maxWorld: 42 });
  const along = screenForwardOffset(currentPoint, nextPoint, Math.sign(spreadIndex) * spreadMagnitude);
  return {
    x: currentPoint.x + lane.x + along.x,
    y: currentPoint.y + lane.y + along.y,
  };
}

function screenForwardOffset(from: Coord, to: Coord, distance: number): Coord {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const length = Math.hypot(dx, dy);
  if (length === 0 || distance === 0) return { x: 0, y: 0 };
  return {
    x: (dx / length) * distance,
    y: (dy / length) * distance,
  };
}

function drawCapsule(
  state: MinimalMapRendererState,
  point: Coord,
  angle: number,
  length: number,
  width: number,
  color: string,
  casing?: string,
): void {
  const { ctx } = state;
  ctx.save();
  ctx.translate(point.x, point.y);
  ctx.rotate(angle);
  ctx.lineCap = 'round';
  if (casing) {
    ctx.strokeStyle = casing;
    ctx.lineWidth = width + 2.6;
    ctx.beginPath();
    ctx.moveTo(-length / 2, 0);
    ctx.lineTo(length / 2, 0);
    ctx.stroke();
  }
  ctx.strokeStyle = color;
  ctx.lineWidth = width;
  ctx.beginPath();
  ctx.moveTo(-length / 2, 0);
  ctx.lineTo(length / 2, 0);
  ctx.stroke();
  ctx.restore();
}

function movementAngle(currentPoint: Coord, nextPoint: Coord): number {
  const dx = nextPoint.x - currentPoint.x;
  const dy = nextPoint.y - currentPoint.y;
  if (Math.abs(dx) + Math.abs(dy) < 0.001) return 0;
  return Math.atan2(dy, dx);
}

function vehicleVectorColor(id: string): string {
  return VEHICLE_COLORS[hash(`vehicle-color:${id}`) % VEHICLE_COLORS.length];
}

function drawPedestrian(state: MinimalMapRendererState, pedestrian: BackendPedestrian, selected: boolean): void {
  const { ctx, camera } = state;
  const current = pedestrian.path[0];
  const next = pedestrian.path[1] ?? current;
  const pos = current;
  const point = iso(state, pos);
  const currentPoint = iso(state, current);
  const nextPoint = iso(state, next);
  const lane = screenRightLaneOffset(currentPoint, nextPoint, screenStableWorldSize(4 + pedestrian.laneOffset, camera.scale, { minWorld: 4, maxWorld: 14 }));
  const selectedRadius = screenStableWorldSize(8, camera.scale, { minWorld: 6.2, maxWorld: 22 });
  const radius = screenStableWorldSize(3.6, camera.scale, { minWorld: 2.9, maxWorld: 10 });
  ctx.save();
  ctx.translate(point.x + lane.x, point.y + lane.y);
  if (selected) {
    ctx.globalAlpha = 0.92;
    ctx.strokeStyle = '#a87309';
    ctx.lineWidth = 2 / Math.max(0.75, camera.scale);
    ctx.beginPath();
    ctx.ellipse(0, 0, selectedRadius, selectedRadius, 0, 0, Math.PI * 2);
    ctx.stroke();
  }
  ctx.fillStyle = AGENT_COLOR;
  ctx.globalAlpha *= 0.78;
  ctx.beginPath();
  ctx.arc(0, 0, radius, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

function formatBackendCoord(coord: { x: number; y: number }): string {
  return `${coord.x.toFixed(1)}, ${coord.y.toFixed(1)}`;
}

export function buildBackendPedestrianInspector(agent: BackendPedestrian | null): EntityInspector {
  if (!agent) return null;
  return {
    title: agent.id,
    rows: [
      { label: 'State', value: 'walking' },
      { label: 'Tile', value: formatBackendCoord(agent.path[0]) },
      { label: 'Next', value: formatBackendCoord(agent.path[1] ?? agent.path[0]) },
      { label: 'Direction', value: agent.direction },
      { label: 'Sprite', value: agent.sprite.sheet },
    ],
  };
}

export function buildBackendCarInspector(vehicle: BackendCar | null): EntityInspector {
  if (!vehicle) return null;
  return {
    title: vehicle.id,
    rows: [
      { label: 'State', value: 'driving' },
      { label: 'Tile', value: formatBackendCoord(vehicle.path[0]) },
      { label: 'Next', value: formatBackendCoord(vehicle.path[1] ?? vehicle.path[0]) },
      { label: 'Direction', value: vehicle.direction },
      { label: 'Sprite', value: vehicle.sprite.role },
    ],
  };
}

function drawAgentInspectorPanel(state: MinimalMapRendererState, inspector: EntityInspector): void {
  if (!inspector) return;
  drawInspectorPanel(state, inspector, { x: 12, y: 12, accent: '#f7d76a', stroke: 'rgba(247, 215, 106, 0.8)' });
}

function drawCarInspectorPanel(state: MinimalMapRendererState, inspector: EntityInspector): void {
  if (!inspector) return;
  drawInspectorPanel(state, inspector, { x: 12, y: 128, accent: '#75d7ff', stroke: 'rgba(117, 215, 255, 0.8)' });
}

function drawInspectorPanel(
  state: MinimalMapRendererState,
  inspector: { title: string; rows: { label: string; value: string }[] },
  options: { x: number; y: number; accent: string; stroke: string },
): void {
  const { ctx, viewport } = state;
  const ratio = viewport.devicePixelRatio;
  const { x, y } = options;
  const width = 232;
  const padding = 10;
  const rowHeight = 17;
  const titleHeight = 20;
  const height = padding * 2 + titleHeight + inspector.rows.length * rowHeight;

  ctx.save();
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  ctx.fillStyle = 'rgba(7, 10, 9, 0.82)';
  ctx.strokeStyle = options.stroke;
  ctx.lineWidth = 1;
  roundedRect(state, x, y, width, height, 6);
  ctx.fill();
  ctx.stroke();

  ctx.font = '600 12px system-ui, -apple-system, BlinkMacSystemFont, sans-serif';
  ctx.fillStyle = options.accent;
  ctx.textBaseline = 'top';
  ctx.fillText(inspector.title, x + padding, y + padding);

  ctx.font = '11px system-ui, -apple-system, BlinkMacSystemFont, sans-serif';
  inspector.rows.forEach((row, index) => {
    const rowY = y + padding + titleHeight + index * rowHeight;
    ctx.fillStyle = 'rgba(231, 236, 224, 0.72)';
    ctx.fillText(row.label, x + padding, rowY);
    ctx.fillStyle = '#f7f7e8';
    ctx.fillText(row.value, x + 70, rowY);
  });
  ctx.restore();
}

function roundedRect(state: MinimalMapRendererState, x: number, y: number, width: number, height: number, radius: number): void {
  const { ctx } = state;
  const r = Math.min(radius, width / 2, height / 2);
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.lineTo(x + width - r, y);
  ctx.quadraticCurveTo(x + width, y, x + width, y + r);
  ctx.lineTo(x + width, y + height - r);
  ctx.quadraticCurveTo(x + width, y + height, x + width - r, y + height);
  ctx.lineTo(x + r, y + height);
  ctx.quadraticCurveTo(x, y + height, x, y + height - r);
  ctx.lineTo(x, y + r);
  ctx.quadraticCurveTo(x, y, x + r, y);
  ctx.closePath();
}

function drawEdgeConnections(state: MinimalMapRendererState, visibleGrid: GridRect): void {
  for (const road of state.roads.values()) {
    for (const exit of outwardExits(state, road.coord, road.mask)) {
      for (let step = 1; step <= EDGE_EXIT_TILES; step += 1) {
        const coord = { x: road.coord.x + exit.dx * step, y: road.coord.y + exit.dy * step };
        if (!isCoordVisible(coord, visibleGrid)) continue;
        drawFadingEdgeTile(state, step, () => drawRoad(state, {
          coord,
          kind: 'street',
          mask: exit.mask,
        }));
      }
    }
  }

  for (const rail of state.rails.values()) {
    for (const exit of outwardExits(state, rail.coord, rail.mask)) {
      for (let step = 1; step <= EDGE_EXIT_TILES; step += 1) {
        const coord = { x: rail.coord.x + exit.dx * step, y: rail.coord.y + exit.dy * step };
        if (!isCoordVisible(coord, visibleGrid)) continue;
        drawFadingEdgeTile(state, step, () => drawRail(state, {
          coord,
          mask: exit.mask,
        }));
      }
    }
  }
}

function drawPerimeterMist(state: MinimalMapRendererState): void {
  const { ctx, world, tileSize } = state;
  const minX = 0;
  const minY = 0;
  const maxX = world.width * tileSize.width;
  const maxY = world.height * tileSize.height;
  ctx.save();
  ctx.strokeStyle = 'rgba(139, 129, 108, 0.18)';
  ctx.lineWidth = 1.4;
  ctx.strokeRect(minX, minY, maxX, maxY);
  ctx.restore();
}

function visibleGridRect(state: MinimalMapRendererState): GridRect {
  const { camera, viewport } = state;
  const inverseScale = 1 / camera.scale;
  const corners = [
    worldToGrid(state, { x: -camera.x * inverseScale, y: -camera.y * inverseScale }),
    worldToGrid(state, { x: (viewport.width - camera.x) * inverseScale, y: -camera.y * inverseScale }),
    worldToGrid(state, { x: -camera.x * inverseScale, y: (viewport.height - camera.y) * inverseScale }),
    worldToGrid(state, { x: (viewport.width - camera.x) * inverseScale, y: (viewport.height - camera.y) * inverseScale }),
  ];
  return {
    minX: Math.floor(Math.min(...corners.map((coord) => coord.x))) - VIEWPORT_GRID_PADDING,
    maxX: Math.ceil(Math.max(...corners.map((coord) => coord.x))) + VIEWPORT_GRID_PADDING,
    minY: Math.floor(Math.min(...corners.map((coord) => coord.y))) - VIEWPORT_GRID_PADDING,
    maxY: Math.ceil(Math.max(...corners.map((coord) => coord.y))) + VIEWPORT_GRID_PADDING,
  };
}

function isCoordVisible(coord: Coord, rect: GridRect): boolean {
  return coord.x >= rect.minX && coord.x <= rect.maxX && coord.y >= rect.minY && coord.y <= rect.maxY;
}

function isInsidePlayableMap(state: MinimalMapRendererState, coord: Coord): boolean {
  return coord.x >= 0 && coord.y >= 0 && coord.x < state.world.width && coord.y < state.world.height;
}

function isWaterSurface(state: MinimalMapRendererState, coord: Coord): boolean {
  const kind = state.terrain.get(key(coord));
  return kind === 'water' || kind === 'riverbank';
}

function distanceOutsidePlayableMap(state: MinimalMapRendererState, coord: Coord): number {
  const { world } = state;
  return Math.max(0, -coord.x, coord.x - (world.width - 1), -coord.y, coord.y - (world.height - 1));
}

function drawIsoTile(state: MinimalMapRendererState, point: Coord): void {
  const { ctx, tileSize } = state;
  ctx.beginPath();
  ctx.rect(point.x - tileSize.width / 2, point.y - tileSize.height / 2, tileSize.width, tileSize.height);
  ctx.fill();
}

function drawFadingEdgeTile(state: MinimalMapRendererState, step: number, draw: () => void): void {
  const { ctx } = state;
  ctx.save();
  ctx.globalAlpha = 0.68 * (1 - step / (EDGE_EXIT_TILES + 1));
  draw();
  ctx.restore();
}

function outwardExits(state: MinimalMapRendererState, coord: Coord, mask: number): { dx: number; dy: number; mask: number }[] {
  const { world } = state;
  const exits: { dx: number; dy: number; mask: number }[] = [];
  if (coord.y === 0 && (mask & NORTH) !== 0) exits.push({ dx: 0, dy: -1, mask: NORTH | SOUTH });
  if (coord.x === world.width - 1 && (mask & EAST) !== 0) exits.push({ dx: 1, dy: 0, mask: EAST | WEST });
  if (coord.y === world.height - 1 && (mask & SOUTH) !== 0) exits.push({ dx: 0, dy: 1, mask: NORTH | SOUTH });
  if (coord.x === 0 && (mask & WEST) !== 0) exits.push({ dx: -1, dy: 0, mask: EAST | WEST });
  return exits;
}

function compareDrawables(state: MinimalMapRendererState, a: Drawable, b: Drawable): number {
  return compareDrawableOrder(
    { type: a.type, isoY: iso(state, a.coord).y, x: a.coord.x },
    { type: b.type, isoY: iso(state, b.coord).y, x: b.coord.x },
  );
}

function selectedBackendPedestrian(state: MinimalMapRendererState): BackendPedestrian | null {
  return pedestriansFromMobilityState(
    state.mobilityState,
    state.pedestrianSprites,
    state.now(),
    state.mobilityTickPeriodMs,
  ).find((agent) => agent.id === state.selectedAgentId) ?? null;
}

function selectedBackendCar(state: MinimalMapRendererState): BackendCar | null {
  return carsFromMobilityState(
    state.mobilityState,
    state.vehicleSprites,
    state.now(),
    state.mobilityTickPeriodMs,
  ).find((vehicle) => vehicle.id === state.selectedVehicleId) ?? null;
}

function iso(state: MinimalMapRendererState, coord: Coord): Coord {
  return mapProject(coord, state.tileSize);
}

function worldToGrid(state: MinimalMapRendererState, point: Coord): Coord {
  return mapUnproject(point, state.tileSize);
}

function key(coord: Coord): string {
  return `${Math.round(coord.x)}:${Math.round(coord.y)}`;
}

function hash(value: string): number {
  let result = 2166136261;
  for (let i = 0; i < value.length; i += 1) {
    result ^= value.charCodeAt(i);
    result = Math.imul(result, 16777619);
  }
  return result >>> 0;
}
