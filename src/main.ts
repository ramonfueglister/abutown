import './style.css';
import {
  defaultAppRuntimeDependencies,
  startAppRuntime,
  type AppRuntimeInitialState,
} from './app/appRuntime';
import { renderBackendRequired as renderBackendRequiredView } from './app/backendRequiredView';
import { createEntitySelection } from './app/entitySelection';
import { attachMapInteraction } from './app/interaction';
import { installRuntimeDiagnostics, type StaticRuntimeDiagnostics } from './app/runtimeDiagnostics';
import type {
  RuntimeBuilding,
  RuntimeRailTile,
  RuntimeRoadTile,
  RuntimeTerrain,
} from './render/worldRuntimeTypes';
import type { BaseWorldResponse, BaseWorldTerrainKind } from './backend/baseWorldClient';
import { resolveBackendBaseUrl, type BackendHealthDto } from './backend/backendGate';
import { type MobilityBackendBridge } from './backend/mobilityClient';
import { createMobilityOverlayState, type MobilityOverlayState } from './backend/mobilityState';
import {
  constrainCameraTargetToGrid,
  createCameraState,
  dampCamera,
} from './cameraController';
import { candidateVehicleSprites, type VehicleSprite } from './render/vehicleSprites';
import {
  candidateMinimalPedestrianSprites,
  type MinimalPedestrianSprite,
} from './render/minimalPedestrianSprites';
import {
  carsFromMobilityState,
  pedestriansFromMobilityState,
  type BackendCar,
  type BackendPedestrian,
} from './render/backendMobilityDrawables';
import { MINIMAL_MAP_TILE_SIZE, mapProject, mapUnproject } from './render/minimalMapProjection';
import {
  EDGE_EXIT_TILES,
  MAP_BACKGROUND,
  OUTSKIRTS_TILES,
  renderMinimalMap,
} from './render/minimalMapRenderer';
import {
  buildBackendCarInspector,
  buildBackendPedestrianInspector,
} from './render/entityInspector';
import { carVisualWorldPoint } from './render/entityRenderStyle';
import type { TerrainKind, WorldDetail } from './city/worldTypes';

type Coord = { x: number; y: number };

const VISUAL_STYLE_ID = 'minimal-motorways';
const TILE_W = MINIMAL_MAP_TILE_SIZE.width;
const TILE_H = MINIMAL_MAP_TILE_SIZE.height;
const tileSize = { width: TILE_W, height: TILE_H };
const CAMERA_EDGE_MARGIN = 8;
const CAMERA_EDGE_SOFTNESS = 4;
const CAMERA_MIN_SCALE = 0.18;
const CAMERA_MAX_SCALE = 2.8;

const backendBaseUrl = resolveBackendBaseUrl(import.meta.env.VITE_ABUTOWN_BACKEND_URL);
let worldId = 'abutopia';
let WIDTH = 16;
let HEIGHT = 8;
let chunkSize = 32;

const canvasElement = document.querySelector<HTMLCanvasElement>('#game');
if (!canvasElement) throw new Error('Missing game canvas');
const canvas: HTMLCanvasElement = canvasElement;

const canvasContext = canvas.getContext('2d');
if (!canvasContext) throw new Error('Missing canvas context');
const ctx: CanvasRenderingContext2D = canvasContext;
ctx.imageSmoothingEnabled = true;

const camera = createCameraState({ x: 0, y: 0, scale: 0.32 });
let cameraInitialized = false;

let terrain = new Map<string, RuntimeTerrain>();
let terrainKinds = new Map<string, { kind: TerrainKind }>();
let roads = new Map<string, RuntimeRoadTile>();
let rails = new Map<string, RuntimeRailTile>();
let railCrossings = new Set<string>();
let railReserved = new Set<string>();
let railPaths: Coord[][] = [];
const railYardPaths: Coord[][] = [];
const railStations: { coord: Coord; frame: number }[] = [];
let buildings: RuntimeBuilding[] = [];
let trees: Coord[] = [];
let details: WorldDetail[] = [];
let vehicleSprites: VehicleSprite[] = [];
let pedestrianSprites: MinimalPedestrianSprite[] = [];
let previousTime = performance.now();
let backendStatus: BackendHealthDto | null = null;
let mobilityState: MobilityOverlayState = createMobilityOverlayState();
let mobilityTickPeriodMs = 100;
let simTime = 0;
let mobilityBackendBridge: MobilityBackendBridge | null = null;
const entitySelection = createEntitySelection<BackendPedestrian, BackendCar>({
  getPedestrians: () => pedestriansFromMobilityState(mobilityState, pedestrianSprites, Date.now(), mobilityTickPeriodMs),
  getVehicles: () => carsFromMobilityState(mobilityState, vehicleSprites, Date.now(), mobilityTickPeriodMs),
  screenToWorld,
  projectPedestrian: (agent) => iso(agent.path[0]),
  projectVehicle: (car) => carVisualWorldPoint(car, camera.scale, tileSize),
  pedestrianRadius: () => Math.max(8, 20 / camera.scale),
  vehicleRadius: () => Math.max(10, 24 / camera.scale),
});

void startRuntime();

async function startRuntime(): Promise<void> {
  const handle = await startAppRuntime({
    backendBaseUrl,
    onInitialState: applyInitialRuntimeState,
    onMobilityState: (state) => {
      mobilityState = state;
    },
    viewport: {
      // Compose screen → render-world → tile so visibleChunks gets coords
      // in the same space the backend's `chunk_of` math operates on.
      getScreenToTile: () => (screen) => worldToGrid(screenToWorld(screen)),
      getViewport: () => ({ width: window.innerWidth, height: window.innerHeight }),
      getWorldDims: () => ({
        widthTiles: WIDTH,
        heightTiles: HEIGHT,
        chunkSize,
      }),
    },
    dependencies: defaultAppRuntimeDependencies(boot, renderBackendRequired),
  });
  mobilityBackendBridge = handle.mobilityBackendBridge;
}

function applyInitialRuntimeState(initial: AppRuntimeInitialState): void {
  backendStatus = initial.backendStatus;
  applyBaseWorld(initial.baseWorld);
  mobilityState = initial.mobilityState;
  mobilityTickPeriodMs = initial.mobilityTickPeriodMs;
  simTime = initial.simTime;
}

async function boot(): Promise<void> {
  vehicleSprites = candidateVehicleSprites();
  pedestrianSprites = candidateMinimalPedestrianSprites();
  resize();
  window.addEventListener('resize', resize);
  attachCamera();
  canvas.dataset.ready = 'true';
  requestAnimationFrame(frame);
}

function renderBackendRequired(error: unknown): void {
  renderBackendRequiredView({
    canvas,
    ctx,
    baseUrl: backendBaseUrl,
    background: MAP_BACKGROUND,
    error,
  });
}

function resize(): void {
  const ratio = window.devicePixelRatio || 1;
  canvas.width = Math.floor(window.innerWidth * ratio);
  canvas.height = Math.floor(window.innerHeight * ratio);
  canvas.style.width = `${window.innerWidth}px`;
  canvas.style.height = `${window.innerHeight}px`;
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  ctx.imageSmoothingEnabled = true;
  if (!cameraInitialized) {
    const focus = iso(initialCameraFocusCoord());
    camera.targetX = window.innerWidth / 2 - focus.x * camera.targetScale;
    camera.targetY = window.innerHeight * 0.52 - focus.y * camera.targetScale;
    camera.x = camera.targetX;
    camera.y = camera.targetY;
    camera.scale = camera.targetScale;
    cameraInitialized = true;
  }
  constrainCamera(false);
}

function attachCamera(): void {
  attachMapInteraction({
    canvas,
    camera,
    constrainCamera,
    selectAtScreenPoint: selectMobilityEntityAtScreenPoint,
    minScale: CAMERA_MIN_SCALE,
    maxScale: CAMERA_MAX_SCALE,
  });
}

function frame(now: number): void {
  const dt = Math.min(0.05, (now - previousTime) / 1000);
  previousTime = now;
  if (!camera.dragging) constrainCamera(false);
  dampCamera(camera, dt, 18);
  render();
  requestAnimationFrame(frame);
}

function render(): void {
  renderMinimalMap({
    ctx,
    viewport: {
      width: window.innerWidth,
      height: window.innerHeight,
      devicePixelRatio: window.devicePixelRatio || 1,
    },
    camera,
    world: { width: WIDTH, height: HEIGHT },
    tileSize,
    terrain,
    terrainKinds,
    roads,
    rails,
    railPaths,
    railStations,
    buildings,
    trees,
    details,
    mobilityState,
    mobilityTickPeriodMs,
    vehicleSprites,
    pedestrianSprites,
    selectedAgentId: entitySelection.selectedAgentId(),
    selectedVehicleId: entitySelection.selectedVehicleId(),
    now: Date.now,
    simTime,
  });
}

function applyBaseWorld(baseWorld: BaseWorldResponse): void {
  worldId = baseWorld.world_id;
  WIDTH = baseWorld.world_tiles.width;
  HEIGHT = baseWorld.world_tiles.height;
  chunkSize = baseWorld.chunk_size;

  terrain = new Map(baseWorld.terrain.tiles.map((tile) => [tileKey(tile), toRuntimeTerrain(tile.kind)]));
  terrainKinds = new Map(baseWorld.terrain.tiles.map((tile) => [tileKey(tile), { kind: tile.kind }]));
  roads = new Map(baseWorld.transport.roads.map((road) => [
    tileKey(road),
    { coord: toCoord(road), kind: road.kind, mask: road.mask },
  ]));
  rails = new Map(baseWorld.transport.rails.map((rail) => [
    tileKey(rail),
    { coord: toCoord(rail), mask: rail.mask },
  ]));
  railReserved = new Set(rails.keys());
  railCrossings = new Set([...roads.keys()].filter((tile) => railReserved.has(tile)));
  railPaths = baseWorld.transport.rail_paths.map((path) => path.points.map(toCoord));
  buildings = baseWorld.buildings.footprints.map((footprint) => {
    const coord = footprint.tiles[0];
    if (!coord) throw new Error(`Base world building ${footprint.id} has no tile`);
    if (!isRuntimeBuildingSheet(footprint.sheet)) {
      throw new Error(`Base world building ${footprint.id} has invalid sheet`);
    }
    if (typeof footprint.frame !== 'number') {
      throw new Error(`Base world building ${footprint.id} has invalid frame`);
    }
    return {
      coord: toCoord(coord),
      sheet: footprint.sheet,
      frame: footprint.frame,
      district: footprint.district ?? 'unknown',
    };
  });
  trees = baseWorld.decorations.trees.map(toCoord);
  details = baseWorld.decorations.details.map((detail) => ({
    coord: toCoord(detail),
    category: toDetailCategory(detail.category),
    assetCategory: detail.asset_category,
  }));
}

function tileKey(point: { readonly x: number; readonly y: number }): string {
  return `${point.x}:${point.y}`;
}

function toCoord(point: { readonly x: number; readonly y: number }): Coord {
  return { x: point.x, y: point.y };
}

function toRuntimeTerrain(kind: BaseWorldTerrainKind): RuntimeTerrain {
  if (kind === 'water') return 'water';
  if (kind === 'riverbank') return 'riverbank';
  if (kind === 'park' || kind === 'forest' || kind === 'reserve' || kind === 'plaza') return 'park';
  return 'grass';
}

function isRuntimeBuildingSheet(value: unknown): value is RuntimeBuilding['sheet'] {
  return (
    value === 'houses' ||
    value === 'oldhouses' ||
    value === 'cottages' ||
    value === 'townhouses' ||
    value === 'shops' ||
    value === 'flats' ||
    value === 'office' ||
    value === 'modern' ||
    value === 'tower' ||
    value === 'church'
  );
}

function toDetailCategory(value: string): WorldDetail['category'] {
  if (
    value === 'tree' ||
    value === 'park' ||
    value === 'civic' ||
    value === 'industry' ||
    value === 'decor' ||
    value === 'station' ||
    value === 'dock' ||
    value === 'quai' ||
    value === 'field' ||
    value === 'yard'
  ) {
    return value;
  }
  throw new Error(`Base world detail category is invalid: ${value}`);
}

function initialCameraFocusCoord(): Coord {
  const pedestrians = pedestriansFromMobilityState(mobilityState, pedestrianSprites, Date.now(), mobilityTickPeriodMs);
  const cars = carsFromMobilityState(mobilityState, vehicleSprites, Date.now(), mobilityTickPeriodMs);
  const vehicleCoords = cars.map((car) => car.path[0]).filter(isInWorld);
  if (vehicleCoords.length > 0) return representativeCoord(vehicleCoords);
  const coords = pedestrians.map((agent) => agent.path[0]).filter(isInWorld);
  if (coords.length === 0) return { x: Math.floor(WIDTH / 2), y: Math.floor(HEIGHT / 2) };
  return representativeCoord(coords);
}

function representativeCoord(coords: Coord[]): Coord {
  const x = coords.reduce((sum, coord) => sum + coord.x, 0) / coords.length;
  const y = coords.reduce((sum, coord) => sum + coord.y, 0) / coords.length;
  const centroid = { x, y };
  return coords.reduce((best, coord) =>
    squaredDistance(coord, centroid) < squaredDistance(best, centroid) ? coord : best,
  );
}

function squaredDistance(a: Coord, b: Coord): number {
  const dx = a.x - b.x;
  const dy = a.y - b.y;
  return dx * dx + dy * dy;
}

function isInWorld(coord: Coord): boolean {
  return coord.x >= 0 && coord.y >= 0 && coord.x < WIDTH && coord.y < HEIGHT;
}

function selectedBackendPedestrian(): BackendPedestrian | null {
  return entitySelection.selectedPedestrian();
}

function selectedBackendCar(): BackendCar | null {
  return entitySelection.selectedVehicle();
}

function selectMobilityEntityAtScreenPoint(point: Coord): void {
  entitySelection.selectAtScreenPoint(point);
}

function screenToWorld(point: Coord): Coord {
  return {
    x: (point.x - camera.x) / camera.scale,
    y: (point.y - camera.y) / camera.scale,
  };
}

function iso(coord: Coord): Coord {
  return mapProject(coord, tileSize);
}

function worldToGrid(point: Coord): Coord {
  return mapUnproject(point, tileSize);
}

function constrainCamera(allowOverscroll: boolean): void {
  constrainCameraTargetToGrid(
    camera,
    { width: window.innerWidth, height: window.innerHeight },
    worldToGrid,
    iso,
    {
      minX: -CAMERA_EDGE_MARGIN,
      maxX: WIDTH - 1 + CAMERA_EDGE_MARGIN,
      minY: -CAMERA_EDGE_MARGIN,
      maxY: HEIGHT - 1 + CAMERA_EDGE_MARGIN,
      softness: CAMERA_EDGE_SOFTNESS,
      allowOverscroll,
    }
  );
}

declare global {
  interface Window {
    render_game_to_text?: () => string;
    advanceTime?: (ms: number) => void;
  }
}

installRuntimeDiagnostics(window, {
  coordinateSystem: 'grid origin north-west, x east, y south, top-down minimal map projection',
  world: { id: worldId, width: WIDTH, height: HEIGHT, chunkSize },
  visualStyle: { id: VISUAL_STYLE_ID, renderer: 'canvas-vector', spriteDrawing: 'disabled' },
  visualAssets: { id: 'minimal-vector', tile: tileSize },
  getBackend: () => ({ required: true, baseUrl: backendBaseUrl, status: backendStatus }),
  getMobilityState: () => mobilityState,
  getMobilityTickPeriodMs: () => mobilityTickPeriodMs,
  getPedestrianSprites: () => pedestrianSprites,
  getVehicleSprites: () => vehicleSprites,
  getCamera: () => ({
    current: { x: camera.x, y: camera.y, scale: camera.scale },
    target: { x: camera.targetX, y: camera.targetY, scale: camera.targetScale },
    dragging: camera.dragging,
    bounds: {
      minX: -CAMERA_EDGE_MARGIN,
      maxX: WIDTH - 1 + CAMERA_EDGE_MARGIN,
      minY: -CAMERA_EDGE_MARGIN,
      maxY: HEIGHT - 1 + CAMERA_EDGE_MARGIN,
    },
    edgeTreatment: {
      outskirtsTiles: OUTSKIRTS_TILES,
      exitTiles: EDGE_EXIT_TILES,
    },
  }),
  getCounts: () => ({
    roadTiles: roads.size,
    railTiles: rails.size,
    bridges: [...roads.values()].filter((road) => road.kind === 'bridge').length,
    buildings: buildings.length,
    trees: trees.length,
    railStations: railStations.length,
    railYardTracks: Math.max(0, railPaths.length - 2),
    reserveTiles: countTerrainKind('reserve'),
  }),
  getDiagnostics: () => baseWorldDiagnostics(),
  getDetails: () => detailCountsByCategory(),
  getValidation: () => ({
    validationErrors: invalidBuildingCount(),
    roadRailOverlap: 0,
    railCrossings: railCrossings.size,
    invalidBuildings: invalidBuildingCount(),
    treeBuildingOverlap: treeBuildingOverlapCount(),
  }),
  getSelected: () => ({
    agentId: entitySelection.selectedAgentId(),
    vehicleId: entitySelection.selectedVehicleId(),
    agentInspector: buildBackendPedestrianInspector(selectedBackendPedestrian()),
    vehicleInspector: buildBackendCarInspector(selectedBackendCar()),
  }),
  projectEntityScreen: (coord) => ({
    x: camera.x + iso(coord).x * camera.scale,
    y: camera.y + iso(coord).y * camera.scale,
  }),
  carVisualWorldPoint: (vehicle) => worldToGrid(carVisualWorldPoint(vehicle, camera.scale, tileSize)),
  now: Date.now,
  advanceTime: (ms) => {
    void ms;
    render();
  },
});

function baseWorldDiagnostics(): StaticRuntimeDiagnostics {
  const invalidBuildings = invalidBuildingCount();
  return {
    roadRailOverlap: 0,
    designedRailCrossings: railCrossings.size,
    invalidBuildings,
    buildingsOutsideStreetFrontageSet: 0,
    buildingsWithoutDirectStreetAdjacency: 0,
    buildingsWithoutAnyStreetAdjacency: 0,
    buildingsWithoutStreetFrontage: 0,
    buildingsTouchingRail: 0,
    buildingFramesOutsideFinishedRow: 0,
    railStationsOnRoad: 0,
    railStationsOnBuildings: 0,
    railStationsOnRails: 0,
    railStationsOnTrees: 0,
    adjacentParallelRoadRuns: 0,
    invalidRoadDeadEnds: 0,
    parallelRoadPairs: 0,
  };
}

function countTerrainKind(kind: TerrainKind): number {
  let count = 0;
  for (const terrain of terrainKinds.values()) if (terrain.kind === kind) count += 1;
  return count;
}

function invalidBuildingCount(): number {
  let count = 0;
  for (const building of buildings) {
    const key = tileKey(building.coord);
    const terrainKind = terrain.get(key);
    if (roads.has(key) || rails.has(key) || terrainKind === 'water' || terrainKind === 'riverbank') count += 1;
  }
  return count;
}

function treeBuildingOverlapCount(): number {
  const buildingTiles = new Set(buildings.map((building) => tileKey(building.coord)));
  return trees.filter((tree) => buildingTiles.has(tileKey(tree))).length;
}

function detailCountsByCategory(): Record<string, number> {
  const result: Record<string, number> = { total: details.length };
  for (const detail of details) {
    result[detail.category] = (result[detail.category] ?? 0) + 1;
  }
  return result;
}
