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
import { createZurichRuntimeContext } from './app/zurichRuntimeContext';
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
  buildBackendCarInspector,
  buildBackendPedestrianInspector,
  carVisualWorldPoint,
  EDGE_EXIT_TILES,
  MAP_BACKGROUND,
  OUTSKIRTS_TILES,
  renderMinimalMap,
} from './render/minimalMapRenderer';
import {
  buildNorthboundTrainPath,
  trainFadeAlpha,
  trainPosition as movingTrainPosition,
  trainWrappedOffset,
} from './render/trainMotion';

type Coord = { x: number; y: number };

type Train = {
  path: Coord[];
  offset: number;
  speed: number;
  fadeTiles: number;
  carSpacing: number;
};

const VISUAL_STYLE_ID = 'minimal-motorways';
const TILE_W = MINIMAL_MAP_TILE_SIZE.width;
const TILE_H = MINIMAL_MAP_TILE_SIZE.height;
const tileSize = { width: TILE_W, height: TILE_H };
const CAMERA_EDGE_MARGIN = 8;
const CAMERA_EDGE_SOFTNESS = 4;
const CAMERA_MIN_SCALE = 0.18;
const CAMERA_MAX_SCALE = 2.8;
const TRAIN_FADE_TILES = 12;
const TRAIN_SPEED = 8.5;

const backendBaseUrl = resolveBackendBaseUrl(import.meta.env.VITE_ABUTOWN_BACKEND_URL);
const zurichContext = createZurichRuntimeContext({ seed: 1848 });
const zurichWorld = zurichContext.world;
const zurichTransport = zurichContext.transport;
const zurichPlacement = zurichContext.placement;
const zurichValidation = zurichContext.validation;

const WIDTH = zurichWorld.width;
const HEIGHT = zurichWorld.height;

const canvasElement = document.querySelector<HTMLCanvasElement>('#game');
if (!canvasElement) throw new Error('Missing game canvas');
const canvas: HTMLCanvasElement = canvasElement;

const canvasContext = canvas.getContext('2d');
if (!canvasContext) throw new Error('Missing canvas context');
const ctx: CanvasRenderingContext2D = canvasContext;
ctx.imageSmoothingEnabled = true;

const camera = createCameraState({ x: 0, y: 0, scale: 0.32 });
let cameraInitialized = false;

const terrain = zurichContext.runtime.terrain;
const roads = zurichContext.runtime.roads;
const rails = zurichContext.runtime.rails;
const railCrossings = zurichContext.runtime.railCrossings;
const railReserved = zurichContext.runtime.railReserved;
const railPaths = zurichContext.runtime.railPaths;
const railYardPaths: Coord[][] = [];
const railStations = zurichContext.runtime.railStations;
const buildings = zurichContext.runtime.buildings;
const trees = zurichContext.runtime.trees;
const details = zurichContext.runtime.details;
let vehicleSprites: VehicleSprite[] = [];
let pedestrianSprites: MinimalPedestrianSprite[] = [];
let trains: Train[] = buildTrains();
let previousTime = performance.now();
let backendStatus: BackendHealthDto | null = null;
let mobilityState: MobilityOverlayState = createMobilityOverlayState();
let mobilityTickPeriodMs = 100;
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
        widthTiles: zurichWorld.width,
        heightTiles: zurichWorld.height,
        chunkSize: zurichWorld.chunkSize,
      }),
    },
    dependencies: defaultAppRuntimeDependencies(boot, renderBackendRequired),
  });
  mobilityBackendBridge = handle.mobilityBackendBridge;
}

function applyInitialRuntimeState(initial: AppRuntimeInitialState): void {
  backendStatus = initial.backendStatus;
  mobilityState = initial.mobilityState;
  mobilityTickPeriodMs = initial.mobilityTickPeriodMs;
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
  for (const train of trains) train.offset = trainWrappedOffset(train.offset + train.speed * dt, train.path);
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
    terrainKinds: zurichWorld.terrain,
    roads,
    rails,
    railPaths,
    railStations,
    buildings,
    trees,
    details,
    trains,
    mobilityState,
    mobilityTickPeriodMs,
    vehicleSprites,
    pedestrianSprites,
    selectedAgentId: entitySelection.selectedAgentId(),
    selectedVehicleId: entitySelection.selectedVehicleId(),
    now: Date.now,
  });
}

function buildTrains(): Train[] {
  const path = buildNorthboundTrainPath(railPaths[0] ?? [], { fadeTiles: TRAIN_FADE_TILES });
  if (path.length === 0) return [];
  return [{
    path,
    offset: 0,
    speed: TRAIN_SPEED,
    fadeTiles: TRAIN_FADE_TILES,
    carSpacing: 1.45,
  }];
}

function initialCameraFocusCoord(): Coord {
  const pedestrians = pedestriansFromMobilityState(mobilityState, pedestrianSprites, Date.now(), mobilityTickPeriodMs);
  const cars = carsFromMobilityState(mobilityState, vehicleSprites, Date.now(), mobilityTickPeriodMs);
  const vehicleCoords = cars.map((car) => car.path[0]).filter(isInWorld);
  const coords = vehicleCoords.length > 0
    ? vehicleCoords
    : pedestrians.map((agent) => agent.path[0]).filter(isInWorld);
  if (coords.length === 0) return { x: Math.floor(WIDTH / 2), y: Math.floor(HEIGHT / 2) };
  const x = coords.reduce((sum, coord) => sum + coord.x, 0) / coords.length;
  const y = coords.reduce((sum, coord) => sum + coord.y, 0) / coords.length;
  const center = { x: WIDTH / 2, y: HEIGHT / 2 };
  return {
    x: center.x * 0.35 + x * 0.65,
    y: center.y * 0.35 + y * 0.65,
  };
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

function trainPosition(train: Train): Coord {
  return movingTrainPosition(train.path, train.offset);
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

function cityDiagnostics(): StaticRuntimeDiagnostics {
  return zurichContext.staticDiagnostics();
}

declare global {
  interface Window {
    render_game_to_text?: () => string;
    advanceTime?: (ms: number) => void;
  }
}

installRuntimeDiagnostics(window, {
  coordinateSystem: 'grid origin north-west, x east, y south, top-down minimal map projection',
  world: { id: zurichWorld.id, width: WIDTH, height: HEIGHT, chunkSize: zurichWorld.chunkSize },
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
    trains: trains.length,
    railStations: railStations.length,
    railYardTracks: Math.max(0, railPaths.length - 2),
    reserveTiles: zurichPlacement.reserveTiles.size,
  }),
  getDiagnostics: () => cityDiagnostics(),
  getDetails: () => detailCountsByCategory(),
  getValidation: () => ({
    validationErrors: zurichValidation.errors.length,
    roadRailOverlap: zurichValidation.stats.roadRailOverlap,
    railCrossings: zurichValidation.stats.railCrossings,
    invalidBuildings: zurichValidation.stats.invalidBuildings,
    treeBuildingOverlap: zurichValidation.stats.treeBuildingOverlap,
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
  getTrain: () => trains[0]
    ? {
        position: trainPosition(trains[0]),
        alpha: trainFadeAlpha(trainPosition(trains[0]), { height: HEIGHT, fadeTiles: trains[0].fadeTiles }),
        speed: trains[0].speed,
        fadeTiles: trains[0].fadeTiles,
        direction: 'northbound',
      }
    : null,
  now: Date.now,
  advanceTime: (ms) => {
    for (const train of trains) train.offset = trainWrappedOffset(train.offset + train.speed * (ms / 1000), train.path);
    render();
  },
});

function detailCountsByCategory(): Record<string, number> {
  const result: Record<string, number> = { total: details.length };
  for (const detail of details) {
    result[detail.category] = (result[detail.category] ?? 0) + 1;
  }
  return result;
}
