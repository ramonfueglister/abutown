import type { CameraState } from '../cameraController';
import type { TerrainKind, WorldDetail } from '../city/worldTypes';
import { formatSimDate } from '../backend/simTime';
import type { MarketLocationDto } from '../backend/mobilityProtocol';
import type {
  RuntimeBuilding,
  RuntimeRailStation,
  RuntimeRailTile,
  RuntimeRoadTile,
  RuntimeTerrain,
} from './worldRuntimeTypes';
import type { MobilityOverlayState } from '../backend/mobilityState';
import { compareDrawableOrder } from './drawOrder';
import {
  carsFromMobilityState,
  pedestriansFromMobilityState,
  type BackendCar,
  type BackendPedestrian,
} from './backendMobilityDrawables';
import { screenStableWorldSize } from './minimalGlyphScale';
import { mapProject, mapUnproject } from './minimalMapProjection';
import type { VehicleSprite } from './vehicleSprites';
import type { MinimalPedestrianSprite } from './minimalPedestrianSprites';
import {
  drawCapsule,
  roundedRectPath,
} from './canvasPrimitives';
import {
  buildBackendCarInspector,
  buildBackendPedestrianInspector,
  type EntityInspector,
} from './entityInspector';
import {
  AGENT_INSPECTOR_PANEL,
  VEHICLE_INSPECTOR_PANEL,
  drawInspectorPanel,
} from './inspectorPanelPainter';
import {
  coordKey as key,
  stableHash as hash,
} from './gridMath';
import {
  carRenderStyle,
  carVisualWorldPoint,
  pedestrianRenderStyle,
} from './entityRenderStyle';
import * as network from './drawNetwork';

export type Coord = { x: number; y: number };

export type MinimalMapRendererState = {
  ctx: CanvasRenderingContext2D;
  viewport: { width: number; height: number; devicePixelRatio: number };
  camera: CameraState;
  world: { width: number; height: number };
  tileSize: { width: number; height: number };
  terrain: ReadonlyMap<string, RuntimeTerrain>;
  terrainKinds: ReadonlyMap<string, { kind: TerrainKind }>;
  roads: ReadonlyMap<string, RuntimeRoadTile>;
  rails: ReadonlyMap<string, RuntimeRailTile>;
  railPaths: readonly Coord[][];
  railStations: readonly RuntimeRailStation[];
  buildings: readonly RuntimeBuilding[];
  trees: readonly Coord[];
  details: readonly WorldDetail[];
  mobilityState: MobilityOverlayState;
  mobilityTickPeriodMs: number;
  vehicleSprites: readonly VehicleSprite[];
  pedestrianSprites: readonly MinimalPedestrianSprite[];
  selectedAgentId: string | null;
  selectedVehicleId: string | null;
  now: () => number;
  simTime: number;
  markets?: readonly MarketLocationDto[];
};

export type GridRect = {
  minX: number;
  maxX: number;
  minY: number;
  maxY: number;
};

type StaticDrawable =
  | { type: 'rail'; coord: Coord; rail: RuntimeRailTile }
  | { type: 'road'; coord: Coord; road: RuntimeRoadTile }
  | { type: 'railStation'; coord: Coord; station: RuntimeRailStation }
  | { type: 'detail'; coord: Coord; detail: WorldDetail }
  | { type: 'tree'; coord: Coord }
  | { type: 'building'; coord: Coord; building: RuntimeBuilding };

type CarDrawable = { type: 'car'; coord: Coord; car: BackendCar; vehicleId: string };
type PedestrianDrawable = { type: 'pedestrian'; coord: Coord; pedestrian: BackendPedestrian; agentId: string };
type Drawable = StaticDrawable | CarDrawable | PedestrianDrawable;
export type TileFillStyle = { color: string; alpha: number };
export type TileFillBatch = TileFillStyle & { coords: Coord[] };

const MARKET_GLYPH_WORLD_SIZE = 10;
const MARKET_COLOR = '#d98c3a';

export const MAP_BACKGROUND = '#182018';
const AGENT_COLOR = '#343b43';
const TRADER_COLOR = '#c0392b';
const VEHICLE_COLORS = ['#e85d75', '#3f8fc7', '#49a879', '#e5a944', '#8c73c8', '#ef7f5a', '#28a6b0'];
const VIEWPORT_GRID_PADDING = 9;
export const TERRAIN_TILE_OVERLAP = 0.6;
export const OUTSKIRTS_TILES = 0;
export const EDGE_EXIT_TILES = 0;

export function renderMinimalMap(state: MinimalMapRendererState): void {
  const { ctx, camera, viewport } = state;
  ctx.save();
  ctx.setTransform(viewport.devicePixelRatio, 0, 0, viewport.devicePixelRatio, 0, 0);
  ctx.imageSmoothingEnabled = true;
  ctx.clearRect(0, 0, viewport.width, viewport.height);
  ctx.translate(camera.x, camera.y);
  ctx.scale(camera.scale, camera.scale);

  drawScene(state);
  ctx.restore();
  drawAgentInspectorPanel(state, buildBackendPedestrianInspector(selectedBackendPedestrian(state)));
  drawCarInspectorPanel(state, buildBackendCarInspector(selectedBackendCar(state)));
  drawWorldDateLabel(state);
}

function drawScene(state: MinimalMapRendererState): void {
  const { ctx, world } = state;
  ctx.save();
  const visibleGrid = visibleGridRect(state);

  const visibleTerrainTiles: Coord[] = [];
  for (let y = Math.max(0, visibleGrid.minY); y <= Math.min(world.height - 1, visibleGrid.maxY); y += 1) {
    for (let x = Math.max(0, visibleGrid.minX); x <= Math.min(world.width - 1, visibleGrid.maxX); x += 1) visibleTerrainTiles.push({ x, y });
  }
  visibleTerrainTiles.sort((a, b) => iso(state, a).y - iso(state, b).y || a.x - b.x);
  network.drawGrassBaseLayer(state);
  network.drawTerrainOverlayLayer(state, visibleTerrainTiles);
  network.drawRiverSurfaceLayer(state, visibleTerrainTiles);

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
  drawEconomyMarkets(state, visibleGrid);
  network.drawRoads(state, [...state.roads.values()].filter((road) => isCoordVisible(road.coord, visibleGrid)));
  for (const path of state.railPaths) network.drawRailPath(state, path);
  network.drawEdgeConnections(state, visibleGrid);
  for (const station of state.railStations) if (isCoordVisible(station.coord, visibleGrid)) network.drawRailStation(state, station);
  for (const detail of state.details) if (isCoordVisible(detail.coord, visibleGrid)) network.drawDetail(state, detail);
  for (const building of state.buildings) if (isCoordVisible(building.coord, visibleGrid)) network.drawBuilding(state, building);
  for (const coord of state.trees) if (isCoordVisible(coord, visibleGrid)) network.drawTree(state, coord);
  for (const item of carDrawables) drawCar(state, item.car, item.vehicleId === state.selectedVehicleId);
  for (const item of pedestrianDrawables) drawPedestrian(state, item.pedestrian, item.agentId === state.selectedAgentId);

  ctx.restore();
}

function drawCar(state: MinimalMapRendererState, car: BackendCar, selected: boolean): void {
  const { ctx, camera, tileSize } = state;
  const point = carVisualWorldPoint(car, camera.scale, tileSize);
  const currentPoint = iso(state, car.path[0]);
  const nextPoint = iso(state, car.path[1] ?? car.path[0]);
  const style = carRenderStyle(currentPoint, nextPoint, camera.scale);
  ctx.save();
  ctx.translate(point.x, point.y);
  if (selected) {
    ctx.globalAlpha = 0.94;
    ctx.strokeStyle = '#166c83';
    ctx.lineWidth = 2 / Math.max(0.75, camera.scale);
    ctx.beginPath();
    ctx.ellipse(0, 0, style.selection.x, style.selection.y, 0, 0, Math.PI * 2);
    ctx.stroke();
  }
  drawCapsule(ctx, { x: 0, y: 0 }, style.angle, style.capsule.length, style.capsule.width, vehicleVectorColor(car.id));
  ctx.restore();
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
  const style = pedestrianRenderStyle(currentPoint, nextPoint, camera.scale, pedestrian.laneOffset);
  ctx.save();
  ctx.translate(point.x + style.lane.x, point.y + style.lane.y);
  if (selected) {
    ctx.globalAlpha = 0.92;
    ctx.strokeStyle = '#a87309';
    ctx.lineWidth = 2 / Math.max(0.75, camera.scale);
    ctx.beginPath();
    ctx.ellipse(0, 0, style.selectedRadius, style.selectedRadius, 0, 0, Math.PI * 2);
    ctx.stroke();
  }
  if (pedestrian.kind === 'trader') {
    // Distinct trader marker: a larger, fully-opaque red dot.
    ctx.fillStyle = TRADER_COLOR;
    ctx.globalAlpha *= 0.95;
    ctx.beginPath();
    ctx.arc(0, 0, style.radius * 1.5, 0, Math.PI * 2);
    ctx.fill();
  } else {
    ctx.fillStyle = AGENT_COLOR;
    ctx.globalAlpha *= 0.78;
    ctx.beginPath();
    ctx.arc(0, 0, style.radius, 0, Math.PI * 2);
    ctx.fill();
  }
  ctx.restore();
}

function drawAgentInspectorPanel(state: MinimalMapRendererState, inspector: EntityInspector): void {
  drawInspectorPanel(state.ctx, inspector, AGENT_INSPECTOR_PANEL, state.viewport.devicePixelRatio);
}

function drawCarInspectorPanel(state: MinimalMapRendererState, inspector: EntityInspector): void {
  drawInspectorPanel(state.ctx, inspector, VEHICLE_INSPECTOR_PANEL, state.viewport.devicePixelRatio);
}

export function visibleMarketGlyphs(
  markets: readonly MarketLocationDto[] | undefined,
  visibleGrid: GridRect,
): MarketLocationDto[] {
  if (!markets) return [];
  return markets.filter((m) => isCoordVisible({ x: m.tileX, y: m.tileY }, visibleGrid));
}

function drawEconomyMarkets(state: MinimalMapRendererState, visibleGrid: GridRect): void {
  for (const m of visibleMarketGlyphs(state.markets, visibleGrid)) {
    drawMarketGlyph(state, { x: m.tileX, y: m.tileY }, MARKET_COLOR);
  }
}

function drawMarketGlyph(state: MinimalMapRendererState, coord: Coord, color: string): void {
  const { ctx, camera } = state;
  const point = iso(state, coord);
  // A zoom-stable flat marker: slightly smaller than a small building, no roof.
  const size = screenStableWorldSize(MARKET_GLYPH_WORLD_SIZE, camera.scale, { minWorld: 7, maxWorld: 11 });
  const x = point.x - size / 2;
  const y = point.y - size / 2;
  ctx.save();
  ctx.fillStyle = color;
  ctx.globalAlpha *= 0.82;
  roundedRectPath(ctx, x, y, size, size, 1.4);
  ctx.fill();
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

export function isCoordVisible(coord: Coord, rect: GridRect): boolean {
  return coord.x >= rect.minX && coord.x <= rect.maxX && coord.y >= rect.minY && coord.y <= rect.maxY;
}

function isWaterSurface(state: MinimalMapRendererState, coord: Coord): boolean {
  const kind = state.terrain.get(key(coord));
  return kind === 'water' || kind === 'riverbank';
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

export function iso(state: MinimalMapRendererState, coord: Coord): Coord {
  return mapProject(coord, state.tileSize);
}

function worldToGrid(state: MinimalMapRendererState, point: Coord): Coord {
  return mapUnproject(point, state.tileSize);
}

function drawWorldDateLabel(state: MinimalMapRendererState): void {
  const { ctx, viewport } = state;
  const label = formatSimDate(state.simTime);
  ctx.save();
  ctx.setTransform(viewport.devicePixelRatio, 0, 0, viewport.devicePixelRatio, 0, 0);
  ctx.font = '11px system-ui, -apple-system, BlinkMacSystemFont, sans-serif';
  ctx.textBaseline = 'bottom';
  ctx.fillStyle = 'rgba(241, 238, 220, 0.72)';
  ctx.fillText(label, 12, viewport.height - 8);
  ctx.restore();
}
