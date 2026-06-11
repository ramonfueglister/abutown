import type { CameraState } from '../cameraController';
import type { TerrainKind, WorldDetail } from '../city/worldTypes';
import { formatSimDate } from '../backend/simTime';
import type {
  EconomyFlowDto,
  MarketGoodDto,
  MarketLocationDto,
} from '../backend/mobilityProtocol';
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
import { mapProject, mapUnproject } from './minimalMapProjection';
import type { VehicleSprite } from './vehicleSprites';
import type { MinimalPedestrianSprite } from './minimalPedestrianSprites';
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
import { coordKey as key } from './gridMath';
import * as network from './drawNetwork';
import { OUT_OF_WORLD } from './designTokens';
import { layerBlend } from './layerBlend';
import { drawFlows } from './drawFlows';
import { drawMarketNodes } from './drawMarkets';
import { drawCar, drawPedestrian } from './drawAgents';

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
  goods?: readonly MarketGoodDto[];
  flows?: readonly EconomyFlowDto[];
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

export const MAP_BACKGROUND = OUT_OF_WORLD;
const VIEWPORT_GRID_PADDING = 9;
export const TERRAIN_TILE_OVERLAP = 0.6;
export const OUTSKIRTS_TILES = 0;
export const EDGE_EXIT_TILES = 0;

let lastFlowsDrawn = 0;
export function flowsDrawnLastFrame(): number {
  return lastFlowsDrawn;
}

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
  const agentsBlend = layerBlend('agents', state.camera.scale);
  const flowsBlend = layerBlend('flows', state.camera.scale);

  network.drawRoads(state, [...state.roads.values()].filter((road) => isCoordVisible(road.coord, visibleGrid)));
  for (const path of state.railPaths) network.drawRailPath(state, path);
  network.drawEdgeConnections(state, visibleGrid);
  for (const station of state.railStations) if (isCoordVisible(station.coord, visibleGrid)) network.drawRailStation(state, station);
  for (const detail of state.details) if (isCoordVisible(detail.coord, visibleGrid)) network.drawDetail(state, detail);
  for (const building of state.buildings) if (isCoordVisible(building.coord, visibleGrid)) network.drawBuilding(state, building);
  for (const coord of state.trees) if (isCoordVisible(coord, visibleGrid)) network.drawTree(state, coord);

  const marketsById = new Map((state.markets ?? []).map((m) => [m.marketId, m]));
  lastFlowsDrawn = drawFlows(
    state.ctx,
    (c) => iso(state, c),
    marketsById,
    state.flows ?? [],
    flowsBlend,
    state.camera.scale,
    state.now(),
  );

  for (const item of carDrawables) drawCar(state, item.car, item.vehicleId === state.selectedVehicleId);
  for (const item of pedestrianDrawables) drawPedestrian(state, item.pedestrian, item.agentId === state.selectedAgentId, agentsBlend);

  const visibleMarkets = new Map(
    visibleMarketGlyphs(state.markets, visibleGrid).map((m) => [m.marketId, m]),
  );
  drawMarketNodes(
    state.ctx,
    (c) => iso(state, c),
    state.camera.scale,
    visibleMarkets,
    (marketId) => (state.goods ?? []).filter((g) => g.marketId === marketId),
    state.now(),
  );

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
  ctx.font = '600 11px system-ui, -apple-system, BlinkMacSystemFont, sans-serif';
  ctx.textBaseline = 'bottom';
  ctx.textAlign = 'right';
  // ink on paper — the old light-on-light label was unreadable over GROUND
  ctx.fillStyle = 'rgba(46, 52, 64, 0.78)';
  ctx.fillText(label, viewport.width - 12, viewport.height - 10);
  ctx.restore();
}
