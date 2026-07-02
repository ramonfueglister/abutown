import type { CameraState } from '../cameraController';
import type { TerrainKind, WorldDetail } from '../city/worldTypes';
import { formatSimDate } from '../backend/simTime';
import type { EconomyFlowDto, MarketGoodDto, MarketLocationDto } from '../backend/mobilityProtocol';
import type {
  RuntimeBuilding,
  RuntimeRailStation,
  RuntimeRailTile,
  RuntimeRoadTile,
  RuntimeTerrain,
} from './worldRuntimeTypes';
import type { MobilityOverlayState } from '../backend/mobilityState';
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
import { AGENT_INSPECTOR_PANEL, VEHICLE_INSPECTOR_PANEL, drawInspectorPanel } from './inspectorPanelPainter';
import { GROUND } from './designTokens';
import { layerBlend } from './layerBlend';
import { drawMarketNodes } from './drawMarkets';
import { drawPedestrian } from './drawAgents';

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

export type TileFillStyle = { color: string; alpha: number };
export type TileFillBatch = TileFillStyle & { coords: Coord[] };

export const MAP_BACKGROUND = GROUND;
const VIEWPORT_GRID_PADDING = 9;
export const TERRAIN_TILE_OVERLAP = 0.6;
export const OUTSKIRTS_TILES = 0;
export const EDGE_EXIT_TILES = 0;

let lastFlowsDrawn = 0;
export function flowsDrawnLastFrame(): number {
  return lastFlowsDrawn;
}

let lastMarketGuideEdgesDrawn = 0;
export function marketGuideEdgesDrawnLastFrame(): number {
  return lastMarketGuideEdgesDrawn;
}

export function defaultVisiblePedestrians(
  pedestrians: readonly BackendPedestrian[],
): BackendPedestrian[] {
  return pedestrians.filter((pedestrian) => pedestrian.kind !== 'trader');
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
  const { ctx } = state;
  ctx.save();
  const visibleGrid = visibleGridRect(state);

  drawPaperWorld(state);

  lastMarketGuideEdgesDrawn = 0;
  lastFlowsDrawn = 0;

  const pedestrians = pedestriansFromMobilityState(
    state.mobilityState,
    state.pedestrianSprites,
    state.now(),
    state.mobilityTickPeriodMs,
  );
  const agentsBlend = layerBlend('agents', state.camera.scale);
  const visiblePedestrians = defaultVisiblePedestrians(pedestrians)
    .filter((pedestrian) => isCoordVisible(pedestrian.path[0], visibleGrid))
    .sort((a, b) => iso(state, a.path[0]).y - iso(state, b.path[0]).y || a.id.localeCompare(b.id));
  for (const pedestrian of visiblePedestrians) {
    drawPedestrian(state, pedestrian, pedestrian.id === state.selectedAgentId, agentsBlend);
  }

  const visibleMarkets = visibleMarketGlyphs(state.markets, visibleGrid);

  const visibleMarketMap = new Map(visibleMarkets.map((market) => [market.marketId, market]));
  drawMarketNodes(
    state.ctx,
    (coord) => iso(state, coord),
    state.camera.scale,
    visibleMarketMap,
    () => [],
    0,
  );

  ctx.restore();
}

function drawPaperWorld(state: MinimalMapRendererState): void {
  const { ctx, tileSize, world } = state;
  ctx.save();
  ctx.fillStyle = GROUND;
  ctx.fillRect(
    -TERRAIN_TILE_OVERLAP,
    -TERRAIN_TILE_OVERLAP,
    world.width * tileSize.width + TERRAIN_TILE_OVERLAP * 2,
    world.height * tileSize.height + TERRAIN_TILE_OVERLAP * 2,
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
  return markets.filter((market) => isCoordVisible({ x: market.tileX, y: market.tileY }, visibleGrid));
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
  ctx.fillStyle = 'rgba(46, 52, 64, 0.78)';
  ctx.fillText(label, viewport.width - 12, viewport.height - 10);
  ctx.restore();
}
