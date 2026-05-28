import './style.css';
import { pak128AssetPack } from './assets/pak128Catalog';
import { backendErrorMessage, requireBackend, resolveBackendBaseUrl, type BackendHealthDto } from './backend/backendGate';
import { loadBackendTerrainState } from './backend/backendTerrain';
import { connectMobilityBackend, requireMobilitySnapshot, type MobilityBackendBridge } from './backend/mobilityClient';
import { createMobilityOverlayState, mobilityDiagnostics, type MobilityOverlayState } from './backend/mobilityState';
import {
  createTerrainState,
  terrainTileAt,
  type TerrainBaseKind,
  type TerrainState,
} from './backend/terrainState';
import { mountCardHandView } from './cardHand/cardHandView';
import type { AssetFrame, AssetRole } from './assets/assetPack';
import {
  countBuildingsWithoutDirectStreetAdjacency,
  hasDirectStreetAdjacency,
  hasVisibleStreetFrontage,
} from './city/buildingFrontage';
import { countAdjacentParallelRoadRuns } from './city/roadParallelCleanup';
import { countInvalidRoadDeadEnds } from './city/roadTopology';
import {
  constrainCameraTargetToGrid,
  createCameraState,
  dampCamera,
  panCameraTarget,
  zoomCameraAt,
} from './cameraController';
import { cleanupSpritePixels } from './render/spriteCleanup';
import { shouldRenderDetail } from './render/detailRenderPolicy';
import {
  candidateVehicleSprites,
  screenRightLaneOffset,
  vehicleFrameForGridDelta,
  type VehicleSprite,
} from './render/vehicleSprites';
import {
  candidateSimutransPedestrianSprites,
  SIMUTRANS_PEDESTRIAN_ASSET_PATHS,
  simutransPedestrianDisplayScale,
  simutransPedestrianFrameForGridDelta,
  simutransPedestrianFrameRect,
  type SimutransDirection,
  type SimutransPedestrianSprite,
} from './render/simutransPedestrianSprites';
import {
  RIVERBANK_EAST,
  RIVERBANK_NORTH,
  RIVERBANK_SOUTH,
  RIVERBANK_WEST,
  riverSurfaceSourceFromMask,
} from './render/riverbankFrames';
import { compareDrawableOrder } from './render/drawOrder';
import {
  carsFromMobilityState,
  pedestriansFromMobilityState,
  type BackendCar,
  type BackendPedestrian,
} from './render/backendMobilityDrawables';
import { MINIMAL_MAP_TILE_SIZE, mapProject, mapUnproject } from './render/minimalMapProjection';
import { screenStableWorldSize } from './render/minimalGlyphScale';
import { minimalBuildingPlotOffset, minimalBuildingSize } from './render/minimalBuildingLayout';
import {
  buildNorthboundTrainPath,
  trainFadeAlpha,
  trainPosition as movingTrainPosition,
  trainWrappedOffset,
} from './render/trainMotion';
import {
  createBackendTerrainRenderState,
  type Building,
  type Detail,
  type RailStation,
  type RailTile,
  type RoadTile,
  type Terrain,
} from './render/backendTerrainRenderState';
import {
  buildBackendCarInspector,
  buildBackendPedestrianInspector,
  type EntityInspector,
} from './render/entityInspector';

type Coord = { x: number; y: number };

type Train = {
  path: Coord[];
  offset: number;
  speed: number;
  fadeTiles: number;
  carSpacing: number;
};

type GridRect = {
  minX: number;
  maxX: number;
  minY: number;
  maxY: number;
};

type StaticDrawable =
  | { type: 'rail'; coord: Coord; rail: RailTile }
  | { type: 'road'; coord: Coord; road: RoadTile }
  | { type: 'railStation'; coord: Coord; station: RailStation }
  | { type: 'detail'; coord: Coord; detail: Detail }
  | { type: 'tree'; coord: Coord }
  | { type: 'building'; coord: Coord; building: Building };

type CarDrawable = { type: 'car'; coord: Coord; car: BackendCar; vehicleId: string };
type PedestrianDrawable = { type: 'pedestrian'; coord: Coord; pedestrian: BackendPedestrian; agentId: string };
type TrainDrawable = { type: 'train'; coord: Coord; train: Train };
type Drawable = StaticDrawable | TrainDrawable | CarDrawable | PedestrianDrawable;

const activeAssetPack = pak128AssetPack;
const VISUAL_STYLE_ID = 'minimal-motorways';
const TILE_W = MINIMAL_MAP_TILE_SIZE.width;
const TILE_H = MINIMAL_MAP_TILE_SIZE.height;
const tileSize = { width: TILE_W, height: TILE_H };
const CAMERA_EDGE_MARGIN = 8;
const CAMERA_EDGE_SOFTNESS = 4;
const CAMERA_MIN_SCALE = 0.18;
const CAMERA_MAX_SCALE = 2.8;
const VIEWPORT_GRID_PADDING = 9;
const OUTSKIRTS_TILES = 12;
const EDGE_EXIT_TILES = 7;
const NORTH = 1;
const EAST = 2;
const SOUTH = 4;
const WEST = 8;
const TRAIN_FADE_TILES = 12;
const TRAIN_SPEED = 8.5;
const MAP_BACKGROUND = '#f6f0e3';
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
const TRAIN_CORE = '#5f6f75';
const TREE_COLOR = '#84ad78';
const DETAIL_COLOR = 'rgba(92, 97, 92, 0.34)';
const BUILDING_RESIDENTIAL = '#d8cfbf';
const BUILDING_COMMERCIAL = '#c9d8dc';
const BUILDING_CIVIC = '#dccb9a';
const BUILDING_INDUSTRIAL = '#cabed6';
const AGENT_COLOR = '#343b43';
const VEHICLE_COLORS = ['#e85d75', '#3f8fc7', '#49a879', '#e5a944', '#8c73c8', '#ef7f5a', '#28a6b0'];

const backendBaseUrl = resolveBackendBaseUrl(import.meta.env.VITE_ABUTOWN_BACKEND_URL);
const TERRAIN_SEED_WORLD_ID = 'zurich-river-city-v1';

let WIDTH = 256;
let HEIGHT = 256;
let terrainState: TerrainState = createTerrainState({ width: WIDTH, height: HEIGHT, chunkSize: 32 });

const canvasElement = document.querySelector<HTMLCanvasElement>('#game');
if (!canvasElement) throw new Error('Missing game canvas');
const canvas: HTMLCanvasElement = canvasElement;

const canvasContext = canvas.getContext('2d');
if (!canvasContext) throw new Error('Missing canvas context');
const ctx: CanvasRenderingContext2D = canvasContext;
ctx.imageSmoothingEnabled = true;

const camera = createCameraState({ x: 0, y: 0, scale: 0.32 });
let cameraInitialized = false;

const BUILDING_FRAME_VARIANTS = 4;

const images = new Map<string, HTMLCanvasElement>();
const simutransSourceBounds = new Map<string, { x: number; y: number; width: number; height: number }>();
let terrain = new Map<string, Terrain>();
let roads = new Map<string, RoadTile>();
let rails = new Map<string, RailTile>();
let railCrossings = new Set<string>();
let railReserved = new Set<string>();
let railPaths: Coord[][] = [];
let railYardPaths: Coord[][] = [];
let railStations: RailStation[] = [];
let buildings: Building[] = [];
let trees: Coord[] = [];
let details: Detail[] = [];
let staticDrawables: StaticDrawable[] = [];
let vehicleSprites: VehicleSprite[] = [];
let pedestrianSprites: SimutransPedestrianSprite[] = [];
let trains: Train[] = [];
let selectedAgentId: string | null = null;
let selectedVehicleId: string | null = null;
let previousTime = performance.now();
let backendStatus: BackendHealthDto | null = null;
let mobilityState: MobilityOverlayState = createMobilityOverlayState();
let mobilityTickPeriodMs = 100;
let mobilityBackendBridge: MobilityBackendBridge | null = null;

void startRuntime();

async function startRuntime(): Promise<void> {
  try {
    backendStatus = await requireBackend({ baseUrl: backendBaseUrl });
    await loadBackendTerrain(backendBaseUrl);
    const required = await requireMobilitySnapshot({ baseUrl: backendBaseUrl });
    mobilityState = required.state;
    mobilityTickPeriodMs = required.tickPeriodMs;
    mountCardHandView({ baseUrl: backendBaseUrl });
    await boot();
    mobilityBackendBridge = connectMobilityBackend({
      baseUrl: backendBaseUrl,
      initialState: mobilityState,
      onState: (state) => {
        mobilityState = state;
      },
      viewport: {
        // Compose screen → render-world → tile so visibleChunks gets coords
        // in the same space the backend's `chunk_of` math operates on.
        getScreenToTile: () => (screen) => worldToGrid(screenToWorld(screen)),
        getViewport: () => ({ width: window.innerWidth, height: window.innerHeight }),
        getWorldDims: () => ({
          widthTiles: terrainState.width,
          heightTiles: terrainState.height,
          chunkSize: terrainState.chunkSize,
        }),
      },
    });
    window.addEventListener('beforeunload', () => mobilityBackendBridge?.stop(), { once: true });
  } catch (error) {
    renderBackendRequired(error);
  }
}

async function loadBackendTerrain(baseUrl: string): Promise<void> {
  const loaded = await loadBackendTerrainState({ baseUrl });
  WIDTH = loaded.width;
  HEIGHT = loaded.height;
  terrainState = loaded.state;
  rebuildRenderStateFromTerrain();
}

function rebuildRenderStateFromTerrain(): void {
  const renderState = createBackendTerrainRenderState(terrainState, { buildingFrameVariants: BUILDING_FRAME_VARIANTS });
  terrain = renderState.terrain;
  roads = renderState.roads;
  rails = renderState.rails;
  railCrossings = renderState.railCrossings;
  railReserved = renderState.railReserved;
  railPaths = renderState.railPaths;
  railYardPaths = renderState.railYardPaths;
  railStations = renderState.railStations;
  buildings = renderState.buildings;
  trees = renderState.trees;
  details = renderState.details;
  staticDrawables = buildStaticDrawables();
  trains = buildTrains();
}

async function boot(): Promise<void> {
  vehicleSprites = candidateVehicleSprites();
  pedestrianSprites = candidateSimutransPedestrianSprites();
  resize();
  window.addEventListener('resize', resize);
  attachCamera();
  canvas.dataset.ready = 'true';
  requestAnimationFrame(frame);
}

function renderBackendRequired(error: unknown): void {
  const message = backendErrorMessage(error);
  canvas.dataset.ready = 'false';
  canvas.dataset.backendRequired = 'true';
  ctx.save();
  ctx.setTransform(window.devicePixelRatio || 1, 0, 0, window.devicePixelRatio || 1, 0, 0);
  ctx.fillStyle = MAP_BACKGROUND;
  ctx.fillRect(0, 0, window.innerWidth, window.innerHeight);
  ctx.restore();

  document.querySelector<HTMLElement>('[data-backend-required]')?.remove();
  const panel = document.createElement('section');
  panel.className = 'backend-required-panel';
  panel.dataset.backendRequired = 'true';
  panel.innerHTML = `
    <h1>Backend required</h1>
    <p>Start Abutown backend at ${escapeHtml(backendBaseUrl)} and reload.</p>
    <pre>cargo run --manifest-path backend/Cargo.toml -p sim-server</pre>
    <small>${escapeHtml(message)}</small>
  `;
  document.body.appendChild(panel);
  console.error(`Abutown backend required: ${message}`);
}

function escapeHtml(value: unknown): string {
  return String(value ?? '').replace(/[&<>"']/g, (char) => ({
    '&': '&amp;',
    '<': '&lt;',
    '>': '&gt;',
    '"': '&quot;',
    "'": '&#39;',
  })[char] ?? char);
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
  let pointerDown: Coord | null = null;
  canvas.addEventListener('pointerdown', (event) => {
    camera.dragging = true;
    pointerDown = { x: event.clientX, y: event.clientY };
    camera.lastX = event.clientX;
    camera.lastY = event.clientY;
    canvas.setPointerCapture(event.pointerId);
  });
  canvas.addEventListener('pointermove', (event) => {
    if (!camera.dragging) return;
    panCameraTarget(camera, event.clientX - camera.lastX, event.clientY - camera.lastY);
    constrainCamera(true);
    camera.lastX = event.clientX;
    camera.lastY = event.clientY;
  });
  canvas.addEventListener('pointerup', (event) => {
    const clickDistance = pointerDown ? Math.hypot(event.clientX - pointerDown.x, event.clientY - pointerDown.y) : Infinity;
    camera.dragging = false;
    if (clickDistance < 4) selectMobilityEntityAtScreenPoint({ x: event.clientX, y: event.clientY });
    pointerDown = null;
    constrainCamera(false);
  });
  canvas.addEventListener('pointercancel', () => {
    camera.dragging = false;
    pointerDown = null;
    constrainCamera(false);
  });
  canvas.addEventListener('wheel', (event) => {
    event.preventDefault();
    zoomCameraAt(camera, { x: event.clientX, y: event.clientY }, event.deltaY, event.deltaMode, {
      minScale: CAMERA_MIN_SCALE,
      maxScale: CAMERA_MAX_SCALE,
    });
    constrainCamera(false);
  }, { passive: false });
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
  ctx.save();
  ctx.setTransform(window.devicePixelRatio || 1, 0, 0, window.devicePixelRatio || 1, 0, 0);
  ctx.imageSmoothingEnabled = true;
  ctx.fillStyle = MAP_BACKGROUND;
  ctx.fillRect(0, 0, window.innerWidth, window.innerHeight);
  ctx.translate(camera.x, camera.y);
  ctx.scale(camera.scale, camera.scale);

  drawScene({ x: 0, y: 0 });
  ctx.restore();
  drawAgentInspectorPanel(buildBackendPedestrianInspector(selectedBackendPedestrian()));
  drawCarInspectorPanel(buildBackendCarInspector(selectedBackendCar()));
}

function drawScene(offset: Coord): void {
  ctx.save();
  const sceneOffset = iso(offset);
  ctx.translate(sceneOffset.x, sceneOffset.y);
  const visibleGrid = visibleGridRect();

  drawOutskirtsTerrain(visibleGrid);
  const visibleTerrainTiles: Coord[] = [];
  for (let y = Math.max(0, visibleGrid.minY); y <= Math.min(HEIGHT - 1, visibleGrid.maxY); y += 1) {
    for (let x = Math.max(0, visibleGrid.minX); x <= Math.min(WIDTH - 1, visibleGrid.maxX); x += 1) visibleTerrainTiles.push({ x, y });
  }
  visibleTerrainTiles.sort((a, b) => iso(a).y - iso(b).y || a.x - b.x);
  for (const coord of visibleTerrainTiles) drawTerrainBase(coord);
  for (const coord of visibleTerrainTiles) drawRiverSurface(coord);

  const pedestrians: BackendPedestrian[] = pedestriansFromMobilityState(mobilityState, pedestrianSprites, Date.now(), mobilityTickPeriodMs);
  const cars: BackendCar[] = carsFromMobilityState(mobilityState, vehicleSprites, Date.now(), mobilityTickPeriodMs);
  const carDrawables = cars
    .map((car) => ({ type: 'car' as const, coord: car.path[0], car, vehicleId: car.id }))
    .filter((item) => isCoordVisible(item.coord, visibleGrid))
    .sort(compareDrawables);
  const pedestrianDrawables = pedestrians
    .map((pedestrian) => ({ type: 'pedestrian' as const, coord: pedestrian.path[0], pedestrian, agentId: pedestrian.id }))
    .filter((item) => isCoordVisible(item.coord, visibleGrid))
    .sort(compareDrawables);
  const trainDrawables = trains
    .map((train) => ({ type: 'train' as const, coord: trainPosition(train), train }))
    .filter((item) => isCoordVisible(item.coord, visibleGrid))
    .sort(compareDrawables);

  for (const road of roads.values()) if (isCoordVisible(road.coord, visibleGrid)) drawRoad(road);
  for (const path of railPaths) drawRailPath(path);
  drawEdgeConnections(visibleGrid);
  for (const station of railStations) if (isCoordVisible(station.coord, visibleGrid)) drawRailStation(station);
  for (const detail of details) if (isCoordVisible(detail.coord, visibleGrid)) drawDetail(detail);
  for (const building of buildings) if (isCoordVisible(building.coord, visibleGrid)) drawBuilding(building);
  for (const coord of trees) if (isCoordVisible(coord, visibleGrid)) drawTree(coord);
  for (const item of trainDrawables) drawTrain(item.train);
  for (const item of carDrawables) drawCar(item.car, item.vehicleId === selectedVehicleId);
  for (const item of pedestrianDrawables) drawPedestrian(item.pedestrian, item.agentId === selectedAgentId);

  drawPerimeterMist();
  ctx.restore();
}

type AssetDrawOptions = {
  offsetX?: number;
  offsetY?: number;
  scale?: number;
  alpha?: number;
};

function drawTerrainBase(coord: Coord): void {
  const base = terrainBaseAt(coord);
  if (base === 'Park' || base === 'Forest' || base === 'Reserve') {
    drawTileFill(coord, MAP_PARK, 0.82);
  } else if (base === 'Plaza') {
    drawTileFill(coord, MAP_PLAZA, 0.72);
  }
}

function drawRiverSurface(coord: Coord): void {
  if (!isWaterSurface(coord)) return;
  const base = terrainBaseAt(coord);
  drawTileFill(coord, base === 'Riverbank' ? MAP_RIVERBANK : MAP_WATER, 0.96);
}

function drawOutskirtsTerrain(visibleGrid: GridRect): void {
  for (let y = Math.max(-OUTSKIRTS_TILES, visibleGrid.minY); y <= Math.min(HEIGHT - 1 + OUTSKIRTS_TILES, visibleGrid.maxY); y += 1) {
    for (let x = Math.max(-OUTSKIRTS_TILES, visibleGrid.minX); x <= Math.min(WIDTH - 1 + OUTSKIRTS_TILES, visibleGrid.maxX); x += 1) {
      const coord = { x, y };
      if (isInsidePlayableMap(coord)) continue;
      const edgeDistance = distanceOutsidePlayableMap(coord);
      if (edgeDistance > OUTSKIRTS_TILES) continue;

      const fade = 1 - edgeDistance / (OUTSKIRTS_TILES + 1);
      ctx.save();
      drawTileFill(coord, MAP_OUTSKIRTS, 0.05 + fade * 0.16);
      if (hash(`outskirts-shadow:${x}:${y}`) % 11 === 0) {
        const point = iso(coord);
        ctx.fillStyle = `rgba(151, 133, 103, ${0.025 + (1 - fade) * 0.035})`;
        drawIsoTile(point);
      }
      ctx.restore();
    }
  }
}

function drawRoad(road: RoadTile): void {
  drawMaskLine(road.coord, road.mask, {
    casing: road.kind === 'bridge' ? ROAD_BRIDGE_CASING : ROAD_CASING,
    core: road.kind === 'bridge' ? ROAD_BRIDGE_CORE : ROAD_CORE,
    casingWidth: road.kind === 'bridge'
      ? screenStableWorldSize(5.5, camera.scale, { minWorld: 10.5, maxWorld: 17 })
      : screenStableWorldSize(4.8, camera.scale, { minWorld: 9.2, maxWorld: 16 }),
    coreWidth: road.kind === 'bridge'
      ? screenStableWorldSize(3.8, camera.scale, { minWorld: 7, maxWorld: 12 })
      : screenStableWorldSize(3.4, camera.scale, { minWorld: 6.4, maxWorld: 10.5 }),
  });
}

function drawRail(_rail: RailTile): void {
  drawMaskLine(_rail.coord, _rail.mask, {
    casing: RAIL_CASING,
    core: RAIL_CORE,
    casingWidth: screenStableWorldSize(2.8, camera.scale, { minWorld: 4.8, maxWorld: 9 }),
    coreWidth: screenStableWorldSize(1.2, camera.scale, { minWorld: 1.8, maxWorld: 4 }),
  });
}

function drawRailPath(path: Coord[]): void {
  if (path.length < 2) return;
  ctx.save();
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  drawPolyline(path, RAIL_CASING, screenStableWorldSize(2.8, camera.scale, { minWorld: 4.8, maxWorld: 9 }));
  drawPolyline(path, RAIL_CORE, screenStableWorldSize(1.2, camera.scale, { minWorld: 1.8, maxWorld: 4 }));
  ctx.restore();
}

function drawPolyline(path: Coord[], color: string, width: number): void {
  ctx.strokeStyle = color;
  ctx.lineWidth = width;
  ctx.beginPath();
  path.forEach((coord, index) => {
    const point = iso(coord);
    if (index === 0) ctx.moveTo(point.x, point.y);
    else ctx.lineTo(point.x, point.y);
  });
  ctx.stroke();
}

function drawRailStation(station: RailStation): void {
  const point = iso(station.coord);
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

function drawDetail(detail: Detail): void {
  if (!shouldRenderDetail(detail)) return;
  if (detail.category !== 'industry' && detail.category !== 'dock' && detail.category !== 'station') return;
  const point = iso(detail.coord);
  ctx.save();
  ctx.fillStyle = DETAIL_COLOR;
  ctx.fillRect(point.x - 2, point.y - 2, 4, 4);
  ctx.restore();
}

function drawBuilding(building: Building): void {
  const point = iso(building.coord);
  const offset = minimalBuildingPlotOffset(building.coord, roads);
  const { width, height } = minimalBuildingSize(building);
  const jitter = buildingJitter(building);
  const x = point.x - width / 2 + offset.x + jitter.x;
  const y = point.y - height / 2 + offset.y + jitter.y;
  ctx.save();
  ctx.fillStyle = 'rgba(108, 97, 77, 0.07)';
  roundedRect(x + 1.5, y + 1.5, width, height, 1.4);
  ctx.fill();
  ctx.globalAlpha = 0.66;
  ctx.fillStyle = buildingVectorColor(building);
  roundedRect(x, y, width, height, 1.4);
  ctx.fill();
  ctx.restore();
}

function buildingJitter(building: Building): Coord {
  return {
    x: ((hash(`building-jitter-x:${building.district}:${key(building.coord)}`) % 5) - 2) * 0.26,
    y: ((hash(`building-jitter-y:${building.district}:${key(building.coord)}`) % 5) - 2) * 0.26,
  };
}

function drawTree(coord: Coord): void {
  if (camera.scale < 0.32 && hash(`tree-lod:${key(coord)}`) % 3 !== 0) return;
  const point = iso(coord);
  const jitterX = ((hash(`tree-x:${key(coord)}`) % 9) - 4) * 0.38;
  const jitterY = ((hash(`tree-y:${key(coord)}`) % 9) - 4) * 0.38;
  ctx.save();
  ctx.fillStyle = TREE_COLOR;
  ctx.globalAlpha = terrainBaseAt(coord) === 'Forest' ? 0.72 : 0.54;
  ctx.beginPath();
  ctx.arc(point.x + jitterX, point.y + jitterY, 2.4, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

function drawTileFill(coord: Coord, color: string, alpha = 1): void {
  const point = iso(coord);
  ctx.save();
  ctx.globalAlpha *= alpha;
  ctx.fillStyle = color;
  ctx.fillRect(point.x - TILE_W / 2 - 0.6, point.y - TILE_H / 2 - 0.6, TILE_W + 1.2, TILE_H + 1.2);
  ctx.restore();
}

function drawMaskLine(
  coord: Coord,
  mask: number,
  style: { casing: string; core: string; casingWidth: number; coreWidth: number },
): void {
  const point = iso(coord);
  const segments = maskSegments(mask);
  ctx.save();
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  drawRoadPass(point, segments, style.casing, style.casingWidth);
  drawRoadPass(point, segments, style.core, style.coreWidth);
  ctx.restore();
}

function drawRoadPass(point: Coord, segments: Coord[], color: string, width: number): void {
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

function maskSegments(mask: number): Coord[] {
  const result: Coord[] = [];
  if ((mask & NORTH) !== 0) result.push({ x: 0, y: -TILE_H / 2 });
  if ((mask & EAST) !== 0) result.push({ x: TILE_W / 2, y: 0 });
  if ((mask & SOUTH) !== 0) result.push({ x: 0, y: TILE_H / 2 });
  if ((mask & WEST) !== 0) result.push({ x: -TILE_W / 2, y: 0 });
  return result;
}

function buildingVectorColor(building: Building): string {
  if (building.sheet === 'church') return BUILDING_CIVIC;
  if (building.sheet === 'office' || building.sheet === 'modern' || building.sheet === 'tower') return BUILDING_COMMERCIAL;
  if (building.district === 'mill-yard') return BUILDING_INDUSTRIAL;
  return BUILDING_RESIDENTIAL;
}

function drawCar(car: BackendCar, selected: boolean): void {
  const current = car.path[0];
  const next = car.path[1] ?? current;
  const pos = current;
  const point = iso(pos);
  const currentPoint = iso(current);
  const nextPoint = iso(next);
  const lane = screenRightLaneOffset(currentPoint, nextPoint, screenStableWorldSize(6.8, camera.scale, { minWorld: 6.8, maxWorld: 20 }));
  const angle = movementAngle(currentPoint, nextPoint);
  const selectX = screenStableWorldSize(14, camera.scale, { minWorld: 8.5, maxWorld: 36 });
  const selectY = screenStableWorldSize(10, camera.scale, { minWorld: 6.5, maxWorld: 28 });
  const length = screenStableWorldSize(16, camera.scale, { minWorld: 12.5, maxWorld: 44 });
  const width = screenStableWorldSize(6.4, camera.scale, { minWorld: 5.2, maxWorld: 19 });
  ctx.save();
  ctx.translate(point.x + lane.x, point.y + lane.y);
  if (selected) {
    ctx.globalAlpha = 0.94;
    ctx.strokeStyle = '#166c83';
    ctx.lineWidth = 2 / Math.max(0.75, camera.scale);
    ctx.beginPath();
    ctx.ellipse(0, 0, selectX, selectY, 0, 0, Math.PI * 2);
    ctx.stroke();
  }
  drawCapsule({ x: 0, y: 0 }, angle, length, width, vehicleVectorColor(car.id));
  ctx.restore();
}

function drawTrain(train: Train): void {
  const segments = [
    { offset: train.offset, length: 13.5 },
    { offset: train.offset - train.carSpacing, length: 10.5 },
    { offset: train.offset - train.carSpacing * 2, length: 10.5 },
    { offset: train.offset - train.carSpacing * 3, length: 10.5 },
    { offset: train.offset - train.carSpacing * 4, length: 10.5 },
  ];

  for (const segment of segments) {
    const pos = movingTrainPosition(train.path, segment.offset);
    const alpha = trainFadeAlpha(pos, { height: HEIGHT, fadeTiles: train.fadeTiles });
    if (alpha <= 0) continue;
    const point = iso(pos);
    const nextPoint = iso(movingTrainPosition(train.path, segment.offset + 0.2));
    ctx.save();
    ctx.globalAlpha *= alpha;
    drawCapsule(point, movementAngle(point, nextPoint), segment.length, 4.8, TRAIN_CORE, RAIL_CASING);
    ctx.restore();
  }
}

function drawCapsule(point: Coord, angle: number, length: number, width: number, color: string, casing?: string): void {
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

function drawAssetRole(role: AssetRole, coord: Coord, options: AssetDrawOptions = {}): void {
  drawAssetFrame(activeAssetPack.require(role), coord, options);
}

function drawAssetFrame(asset: AssetFrame, coord: Coord, options: AssetDrawOptions = {}): void {
  drawAssetAt(asset, iso(coord), options);
}

function drawAssetAt(asset: AssetFrame, point: Coord, options: AssetDrawOptions = {}): void {
  const image = images.get(asset.path);
  if (!image) return;
  const sourceWidth = Math.min(asset.source.width, image.width - asset.source.x);
  const sourceHeight = Math.min(asset.source.height, image.height - asset.source.y);
  if (sourceWidth <= 0 || sourceHeight <= 0) return;

  const scale = asset.scale * (options.scale ?? 1);
  const width = sourceWidth * scale;
  const height = sourceHeight * scale;
  ctx.save();
  ctx.globalAlpha *= options.alpha ?? 1;
  ctx.drawImage(
    image,
    asset.source.x,
    asset.source.y,
    sourceWidth,
    sourceHeight,
    point.x - asset.anchor.x * scale + (options.offsetX ?? 0),
    point.y - asset.anchor.y * scale + (options.offsetY ?? 0),
    width,
    height,
  );
  ctx.restore();
}

function terrainRole(kind: Terrain): Extract<AssetRole, 'terrain.grass' | 'terrain.water' | 'terrain.riverbank'> {
  if (kind === 'water') return 'terrain.water';
  if (kind === 'riverbank') return 'terrain.riverbank';
  return 'terrain.grass';
}

function roadRole(road: RoadTile): Extract<AssetRole, 'road.straight' | 'road.curve' | 'road.intersection' | 'road.bridge'> {
  if (road.kind === 'bridge') return 'road.bridge';
  const normalized = road.mask & (NORTH | EAST | SOUTH | WEST);
  const degree = [NORTH, EAST, SOUTH, WEST].filter((direction) => (normalized & direction) !== 0).length;
  if (degree >= 3) return 'road.intersection';
  if (degree <= 1 || isStraightEastWest(normalized) || isStraightNorthSouth(normalized)) return 'road.straight';
  return 'road.curve';
}

function buildingRole(building: Building): Extract<AssetRole, 'building.residential.low' | 'building.commercial.mid' | 'building.civic' | 'building.industrial'> {
  if (building.sheet === 'church') return 'building.civic';
  if (building.district === 'mill-yard' && hash(`industrial:${key(building.coord)}`) % 3 === 0) return 'building.industrial';
  if (building.sheet === 'houses' || building.sheet === 'oldhouses' || building.sheet === 'cottages' || building.sheet === 'townhouses') {
    return 'building.residential.low';
  }
  return 'building.commercial.mid';
}

function detailRole(detail: Detail): Extract<AssetRole, 'detail.park' | 'detail.industry' | 'detail.dock' | 'detail.quay'> {
  if (detail.assetCategory === 'dock') return 'detail.dock';
  if (detail.assetCategory === 'quay' || detail.assetCategory === 'ship') return 'detail.quay';
  if (detail.category === 'field' || detail.category === 'park' || detail.category === 'civic' || detail.category === 'decor') return 'detail.park';
  return 'detail.industry';
}

function drawPedestrian(pedestrian: BackendPedestrian, selected: boolean): void {
  const current = pedestrian.path[0];
  const next = pedestrian.path[1] ?? current;
  const pos = current;
  const point = iso(pos);
  const currentPoint = iso(current);
  const nextPoint = iso(next);
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

function drawAgentInspectorPanel(inspector: EntityInspector): void {
  if (!inspector) return;
  drawInspectorPanel(inspector, { x: 12, y: 12, accent: '#f7d76a', stroke: 'rgba(247, 215, 106, 0.8)' });
}

function drawCarInspectorPanel(inspector: EntityInspector): void {
  if (!inspector) return;
  drawInspectorPanel(inspector, { x: 12, y: 128, accent: '#75d7ff', stroke: 'rgba(117, 215, 255, 0.8)' });
}

function drawInspectorPanel(
  inspector: { title: string; rows: { label: string; value: string }[] },
  options: { x: number; y: number; accent: string; stroke: string },
): void {
  const ratio = window.devicePixelRatio || 1;
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
  roundedRect(x, y, width, height, 6);
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

function roundedRect(x: number, y: number, width: number, height: number, radius: number): void {
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

function drawEdgeConnections(visibleGrid: GridRect): void {
  for (const road of roads.values()) {
    for (const exit of outwardExits(road.coord, road.mask)) {
      for (let step = 1; step <= EDGE_EXIT_TILES; step += 1) {
        const coord = { x: road.coord.x + exit.dx * step, y: road.coord.y + exit.dy * step };
        if (!isCoordVisible(coord, visibleGrid)) continue;
        drawFadingEdgeTile(step, () => drawRoad({
          coord,
          kind: 'street',
          mask: exit.mask,
        }));
      }
    }
  }

  for (const rail of rails.values()) {
    for (const exit of outwardExits(rail.coord, rail.mask)) {
      for (let step = 1; step <= EDGE_EXIT_TILES; step += 1) {
        const coord = { x: rail.coord.x + exit.dx * step, y: rail.coord.y + exit.dy * step };
        if (!isCoordVisible(coord, visibleGrid)) continue;
        drawFadingEdgeTile(step, () => drawRail({
          coord,
          mask: exit.mask,
        }));
      }
    }
  }
}

function drawPerimeterMist(): void {
  const minX = 0;
  const minY = 0;
  const maxX = WIDTH * TILE_W;
  const maxY = HEIGHT * TILE_H;
  ctx.save();
  ctx.strokeStyle = 'rgba(139, 129, 108, 0.18)';
  ctx.lineWidth = 1.4;
  ctx.strokeRect(minX, minY, maxX, maxY);
  ctx.restore();
}

function terrainAt(coord: Coord): Terrain {
  return terrain.get(key(coord)) ?? 'grass';
}

function terrainBaseAt(coord: Coord): TerrainBaseKind {
  return terrainTileAt(terrainState, coord)?.base ?? 'Grass';
}

function buildStreetFrontages(): Set<string> {
  const result = new Set<string>();
  for (const road of roads.values()) {
    if (road.kind !== 'street') continue;
    for (const coord of cardinal(road.coord)) {
      const terrainKind = terrainAt(coord);
      if (isInsidePlayableMap(coord) && (terrainKind === 'grass' || terrainKind === 'park')) result.add(key(coord));
    }
  }
  return result;
}

function touchesRail(coord: Coord): boolean {
  return [coord, ...cardinal(coord)].some((neighbor) => railReserved.has(key(neighbor)));
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
  const coords = [...pedestrians.map((agent) => agent.path[0]), ...cars.map((car) => car.path[0])]
    .filter((coord) => coord.x >= 0 && coord.y >= 0 && coord.x < WIDTH && coord.y < HEIGHT);
  if (coords.length === 0) return { x: Math.floor(WIDTH / 2), y: Math.floor(HEIGHT / 2) };
  const x = coords.reduce((sum, coord) => sum + coord.x, 0) / coords.length;
  const y = coords.reduce((sum, coord) => sum + coord.y, 0) / coords.length;
  const center = { x: WIDTH / 2, y: HEIGHT / 2 };
  return {
    x: center.x * 0.35 + x * 0.65,
    y: center.y * 0.35 + y * 0.65,
  };
}

function selectedBackendPedestrian(): BackendPedestrian | null {
  if (!selectedAgentId) return null;
  const pedestrians = pedestriansFromMobilityState(mobilityState, pedestrianSprites, Date.now(), mobilityTickPeriodMs);
  return pedestrians.find((agent) => agent.id === selectedAgentId) ?? null;
}

function selectedBackendCar(): BackendCar | null {
  if (!selectedVehicleId) return null;
  const cars = carsFromMobilityState(mobilityState, vehicleSprites, Date.now(), mobilityTickPeriodMs);
  return cars.find((vehicle) => vehicle.id === selectedVehicleId) ?? null;
}

function selectMobilityEntityAtScreenPoint(point: Coord): void {
  const worldPoint = screenToWorld(point);
  const pedestrians = pedestriansFromMobilityState(mobilityState, pedestrianSprites, Date.now(), mobilityTickPeriodMs);
  const cars = carsFromMobilityState(mobilityState, vehicleSprites, Date.now(), mobilityTickPeriodMs);
  const vehicleHit = findNearestProjectedEntity(cars, worldPoint, Math.max(10, 24 / camera.scale));
  if (vehicleHit) {
    selectedVehicleId = vehicleHit.id;
    selectedAgentId = null;
    return;
  }
  const agentHit = findNearestProjectedEntity(pedestrians, worldPoint, Math.max(8, 20 / camera.scale));
  selectedAgentId = agentHit?.id ?? null;
  selectedVehicleId = null;
}

function findNearestProjectedEntity<T extends { id: string; path: Coord[] }>(
  entities: readonly T[],
  worldPoint: Coord,
  radius: number,
): T | null {
  let nearest: { entity: T; distance: number } | null = null;
  for (const entity of entities) {
    const projected = iso(entity.path[0]);
    const distance = Math.hypot(projected.x - worldPoint.x, projected.y - worldPoint.y);
    if (distance > radius) continue;
    if (!nearest || distance < nearest.distance) nearest = { entity, distance };
  }
  return nearest?.entity ?? null;
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

function isStraightEastWest(mask: number): boolean {
  return (mask & (EAST | WEST)) === (EAST | WEST) && (mask & (NORTH | SOUTH)) === 0;
}

function isStraightNorthSouth(mask: number): boolean {
  return (mask & (NORTH | SOUTH)) === (NORTH | SOUTH) && (mask & (EAST | WEST)) === 0;
}

function iso(coord: Coord): Coord {
  return mapProject(coord, tileSize);
}

function worldToGrid(point: Coord): Coord {
  return mapUnproject(point, tileSize);
}

function buildStaticDrawables(): StaticDrawable[] {
  return [
    ...[...rails.values()].map((rail) => ({ type: 'rail' as const, coord: rail.coord, rail })),
    ...[...roads.values()].map((road) => ({ type: 'road' as const, coord: road.coord, road })),
    ...railStations.map((station) => ({ type: 'railStation' as const, coord: station.coord, station })),
    ...details.map((detail) => ({ type: 'detail' as const, coord: detail.coord, detail })),
    ...trees.map((coord) => ({ type: 'tree' as const, coord })),
    ...buildings.map((building) => ({ type: 'building' as const, coord: building.coord, building })),
  ].sort(compareDrawables);
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

function visibleGridRect(): GridRect {
  const inverseScale = 1 / camera.scale;
  const corners = [
    worldToGrid({ x: -camera.x * inverseScale, y: -camera.y * inverseScale }),
    worldToGrid({ x: (window.innerWidth - camera.x) * inverseScale, y: -camera.y * inverseScale }),
    worldToGrid({ x: -camera.x * inverseScale, y: (window.innerHeight - camera.y) * inverseScale }),
    worldToGrid({ x: (window.innerWidth - camera.x) * inverseScale, y: (window.innerHeight - camera.y) * inverseScale }),
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

function isInsidePlayableMap(coord: Coord): boolean {
  return coord.x >= 0 && coord.y >= 0 && coord.x < WIDTH && coord.y < HEIGHT;
}

function waterSurfaceMask(coord: Coord): number {
  return (
    (isWaterSurface({ x: coord.x, y: coord.y - 1 }) ? RIVERBANK_NORTH : 0) |
    (isWaterSurface({ x: coord.x + 1, y: coord.y }) ? RIVERBANK_EAST : 0) |
    (isWaterSurface({ x: coord.x, y: coord.y + 1 }) ? RIVERBANK_SOUTH : 0) |
    (isWaterSurface({ x: coord.x - 1, y: coord.y }) ? RIVERBANK_WEST : 0)
  );
}

function isWaterSurface(coord: Coord): boolean {
  const kind = terrainAt(coord);
  return kind === 'water' || kind === 'riverbank';
}

function distanceOutsidePlayableMap(coord: Coord): number {
  return Math.max(0, -coord.x, coord.x - (WIDTH - 1), -coord.y, coord.y - (HEIGHT - 1));
}

function drawIsoTile(point: Coord): void {
  ctx.beginPath();
  ctx.rect(point.x - TILE_W / 2, point.y - TILE_H / 2, TILE_W, TILE_H);
  ctx.fill();
}

function streetRoadAssetFrame(road: RoadTile): AssetFrame {
  const asset = activeAssetPack.require(roadRole(road));
  return { ...asset, source: roadSourceFromMask(road.mask) };
}

function bridgeRoadAssetFrames(road: RoadTile): AssetFrame[] {
  const asset = activeAssetPack.require('road.bridge');
  const eastWest = isStraightEastWest(road.mask) || (!isStraightNorthSouth(road.mask) && (road.mask & (EAST | WEST)) !== 0);
  const row = 1;
  const backCol = eastWest ? 4 : 0;
  return [
    { ...asset, source: pak128Cell(row, backCol), anchor: { x: 64, y: 96 }, baseline: 96 },
    { ...asset, source: pak128Cell(row, backCol + 1), anchor: { x: 64, y: 96 }, baseline: 96 },
  ];
}

function roadSourceFromMask(mask: number): AssetFrame['source'] {
  const normalized = mask & (NORTH | EAST | SOUTH | WEST);
  if (normalized === 0) return pak128Cell(1, 0);
  if (normalized === NORTH) return pak128Cell(1, 1);
  if (normalized === SOUTH) return pak128Cell(1, 2);
  if (normalized === EAST) return pak128Cell(1, 3);
  if (normalized === WEST) return pak128Cell(1, 4);
  if (normalized === (NORTH | SOUTH)) return pak128Cell(1, 5);
  if (normalized === (EAST | WEST)) return pak128Cell(1, 6);
  if (normalized === (NORTH | SOUTH | EAST)) return pak128Cell(1, 7);
  if (normalized === (NORTH | SOUTH | WEST)) return pak128Cell(2, 0);
  if (normalized === (NORTH | EAST | WEST)) return pak128Cell(2, 1);
  if (normalized === (SOUTH | EAST | WEST)) return pak128Cell(2, 2);
  if (normalized === (NORTH | SOUTH | EAST | WEST)) return pak128Cell(2, 3);
  if (normalized === (NORTH | EAST)) return pak128Cell(2, 4);
  if (normalized === (SOUTH | EAST)) return pak128Cell(2, 5);
  if (normalized === (NORTH | WEST)) return pak128Cell(2, 6);
  return pak128Cell(2, 7);
}

function pak128Cell(row: number, col: number, height = 128): AssetFrame['source'] {
  return { x: col * 128, y: row * 128, width: 128, height };
}

function drawFadingEdgeTile(step: number, draw: () => void): void {
  ctx.save();
  ctx.globalAlpha = 0.68 * (1 - step / (EDGE_EXIT_TILES + 1));
  draw();
  ctx.restore();
}

function outwardExits(coord: Coord, mask: number): { dx: number; dy: number; mask: number }[] {
  const exits: { dx: number; dy: number; mask: number }[] = [];
  if (coord.y === 0 && (mask & NORTH) !== 0) exits.push({ dx: 0, dy: -1, mask: NORTH | SOUTH });
  if (coord.x === WIDTH - 1 && (mask & EAST) !== 0) exits.push({ dx: 1, dy: 0, mask: EAST | WEST });
  if (coord.y === HEIGHT - 1 && (mask & SOUTH) !== 0) exits.push({ dx: 0, dy: 1, mask: NORTH | SOUTH });
  if (coord.x === 0 && (mask & WEST) !== 0) exits.push({ dx: -1, dy: 0, mask: EAST | WEST });
  return exits;
}

function depthSort(a: { coord: Coord }, b: { coord: Coord }): number {
  return iso(a.coord).y - iso(b.coord).y || a.coord.x - b.coord.x;
}

function compareDrawables(a: Drawable, b: Drawable): number {
  return compareDrawableOrder(
    { type: a.type, isoY: iso(a.coord).y, x: a.coord.x },
    { type: b.type, isoY: iso(b.coord).y, x: b.coord.x },
  );
}

function mergeSortedDrawables(staticItems: StaticDrawable[], dynamicItems: Array<TrainDrawable | CarDrawable | PedestrianDrawable>): Drawable[] {
  const result: Drawable[] = [];
  let staticIndex = 0;
  let dynamicIndex = 0;
  while (staticIndex < staticItems.length || dynamicIndex < dynamicItems.length) {
    const staticItem = staticItems[staticIndex];
    const dynamicItem = dynamicItems[dynamicIndex];
    if (!staticItem) {
      result.push(dynamicItem);
      dynamicIndex += 1;
    } else if (!dynamicItem || compareDrawables(staticItem, dynamicItem) <= 0) {
      result.push(staticItem);
      staticIndex += 1;
    } else {
      result.push(dynamicItem);
      dynamicIndex += 1;
    }
  }
  return result;
}

function cardinal(coord: Coord): Coord[] {
  return [
    { x: coord.x, y: coord.y - 1 },
    { x: coord.x + 1, y: coord.y },
    { x: coord.x, y: coord.y + 1 },
    { x: coord.x - 1, y: coord.y },
  ];
}

function cityDiagnostics(): Record<string, number> {
  let roadRailOverlap = 0;
  let designedRailCrossings = 0;
  for (const tileKey of roads.keys()) {
    if (!rails.has(tileKey)) continue;
    if (railCrossings.has(tileKey)) designedRailCrossings += 1;
    else roadRailOverlap += 1;
  }

  let invalidBuildings = 0;
  let buildingsOutsideStreetFrontageSet = 0;
  let buildingsWithoutAnyStreetAdjacency = 0;
  let buildingsWithoutStreetFrontage = 0;
  let buildingsTouchingRail = 0;
  let buildingFramesOutsideFinishedRow = 0;
  const streetFrontages = buildStreetFrontages();
  for (const building of buildings) {
    const tileKey = key(building.coord);
    const terrainKind = terrainAt(building.coord);
    if (roads.has(tileKey) || rails.has(tileKey) || !(terrainKind === 'grass' || terrainKind === 'park')) invalidBuildings += 1;
    if (!streetFrontages.has(tileKey)) buildingsOutsideStreetFrontageSet += 1;
    if (!hasDirectStreetAdjacency(building.coord, roads)) buildingsWithoutAnyStreetAdjacency += 1;
    if (!hasVisibleStreetFrontage(building.coord, roads)) buildingsWithoutStreetFrontage += 1;
    if (touchesRail(building.coord)) buildingsTouchingRail += 1;
  }

  const buildingTiles = new Set(buildings.map((building) => key(building.coord)));
  const treeTiles = new Set(trees.map(key));
  let treeBuildingOverlap = 0;
  for (const tileKey of treeTiles) if (buildingTiles.has(tileKey)) treeBuildingOverlap += 1;
  let railStationsOnRoad = 0;
  let railStationsOnBuildings = 0;
  let railStationsOnRails = 0;
  let railStationsOnTrees = 0;
  for (const station of railStations) {
    const stationKey = key(station.coord);
    if (roads.has(stationKey)) railStationsOnRoad += 1;
    if (buildingTiles.has(stationKey)) railStationsOnBuildings += 1;
    if (rails.has(stationKey)) railStationsOnRails += 1;
    if (treeTiles.has(stationKey)) railStationsOnTrees += 1;
  }

  let parallelRoadPairs = 0;
  for (const road of roads.values()) {
    const mask = road.mask;
    if (isStraightEastWest(mask)) {
      const south = roads.get(key({ x: road.coord.x, y: road.coord.y + 1 }));
      if (south && isStraightEastWest(south.mask)) parallelRoadPairs += 1;
    }
    if (isStraightNorthSouth(mask)) {
      const east = roads.get(key({ x: road.coord.x + 1, y: road.coord.y }));
      if (east && isStraightNorthSouth(east.mask)) parallelRoadPairs += 1;
    }
  }

  return {
    roadRailOverlap,
    designedRailCrossings,
    invalidBuildings,
    buildingsOutsideStreetFrontageSet,
    buildingsWithoutDirectStreetAdjacency: countBuildingsWithoutDirectStreetAdjacency(buildings, roads),
    buildingsWithoutAnyStreetAdjacency,
    buildingsWithoutStreetFrontage,
    buildingsTouchingRail,
    buildingFramesOutsideFinishedRow,
    treeBuildingOverlap,
    railStationsOnRoad,
    railStationsOnBuildings,
    railStationsOnRails,
    railStationsOnTrees,
    adjacentParallelRoadRuns: countAdjacentParallelRoadRuns(roads),
    invalidRoadDeadEnds: countInvalidRoadDeadEnds(roads, { width: WIDTH, height: HEIGHT }),
    parallelRoadPairs,
  };
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

function visibleSourceBounds(
  image: HTMLCanvasElement,
  sprite: SimutransPedestrianSprite,
  direction: SimutransDirection,
  rect: { x: number; y: number; width: number; height: number },
): { x: number; y: number; width: number; height: number } {
  const cacheKey = `${sprite.sheet}:${sprite.row}:${direction}`;
  const cached = simutransSourceBounds.get(cacheKey);
  if (cached) return cached;

  const context = image.getContext('2d', { willReadFrequently: true });
  if (!context) return rect;
  const pixels = context.getImageData(rect.x, rect.y, rect.width, rect.height).data;
  let minX = rect.width;
  let minY = rect.height;
  let maxX = -1;
  let maxY = -1;

  for (let y = 0; y < rect.height; y += 1) {
    for (let x = 0; x < rect.width; x += 1) {
      const alpha = pixels[(y * rect.width + x) * 4 + 3];
      if (alpha === 0) continue;
      minX = Math.min(minX, x);
      minY = Math.min(minY, y);
      maxX = Math.max(maxX, x);
      maxY = Math.max(maxY, y);
    }
  }

  const bounds = maxX < minX
    ? rect
    : {
        x: rect.x + minX,
        y: rect.y + minY,
        width: maxX - minX + 1,
        height: maxY - minY + 1,
      };
  simutransSourceBounds.set(cacheKey, bounds);
  return bounds;
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

async function loadCleanImage(path: string): Promise<HTMLCanvasElement> {
  const image = await new Promise<HTMLImageElement>((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error(`Unable to load ${path}`));
    img.src = path;
  });
  const buffer = document.createElement('canvas');
  buffer.width = image.naturalWidth;
  buffer.height = image.naturalHeight;
  const context = buffer.getContext('2d', { willReadFrequently: true });
  if (!context) throw new Error(`Unable to clean ${path}`);
  context.drawImage(image, 0, 0);
  const data = context.getImageData(0, 0, buffer.width, buffer.height);
  cleanupSpritePixels({ data: data.data, width: buffer.width, height: buffer.height, path });
  context.putImageData(data, 0, 0);
  return buffer;
}

declare global {
  interface Window {
    render_game_to_text?: () => string;
    advanceTime?: (ms: number) => void;
  }
}

window.render_game_to_text = () => {
  const diagnostics = cityDiagnostics();
  const detailCounts = detailCountsByCategory();
  const backendMobility = mobilityDiagnostics(mobilityState);
  const projectedPedestrians = pedestriansFromMobilityState(mobilityState, pedestrianSprites, Date.now(), mobilityTickPeriodMs);
  const projectedCars = carsFromMobilityState(mobilityState, vehicleSprites, Date.now(), mobilityTickPeriodMs);
  const selectedAgent = projectedPedestrians.find((agent) => agent.id === selectedAgentId) ?? null;
  const selectedVehicle = projectedCars.find((vehicle) => vehicle.id === selectedVehicleId) ?? null;
  const entityScreenPosition = (coord: Coord): Coord => {
    const projected = iso(coord);
    return {
      x: camera.x + projected.x * camera.scale,
      y: camera.y + projected.y * camera.scale,
    };
  };
  const mobilityAgentEntries = projectedPedestrians.map((agent) => ({
    id: agent.id,
    kind: 'pedestrian' as const,
    state: 'walking' as const,
    coord: agent.path[0],
    screen: entityScreenPosition(agent.path[0]),
    direction: agent.direction,
    spriteSheet: agent.sprite.sheet,
  }));
  const mobilityVehicleEntries = projectedCars.map((vehicle) => ({
    id: vehicle.id,
    kind: 'car' as const,
    state: 'driving' as const,
    coord: vehicle.path[0],
    screen: entityScreenPosition(vehicle.path[0]),
    direction: vehicle.direction,
    spriteSheet: vehicle.sprite.sheet,
  }));
  const selectedMobilityAgentEntry = selectedAgent
    ? mobilityAgentEntries.find((entry) => entry.id === selectedAgent.id) ?? null
    : null;
  const selectedMobilityVehicleEntry = selectedVehicle
    ? mobilityVehicleEntries.find((entry) => entry.id === selectedVehicle.id) ?? null
    : null;
  return JSON.stringify({
    coordinateSystem: 'grid origin north-west, x east, y south, top-down minimal map projection',
    city: {
      worldId: TERRAIN_SEED_WORLD_ID,
      terrainSource: 'backend-layered',
      layeredTerrain: {
        loadedTiles: terrainState.tiles.size,
        loadedChunks: terrainState.loadedChunks.size,
      },
      visualStyle: {
        id: VISUAL_STYLE_ID,
        renderer: 'canvas-vector',
        spriteDrawing: 'disabled',
      },
      assetPack: {
        id: 'minimal-vector',
        tile: tileSize,
      },
      nonPak128AssetPaths: nonPak128AssetPaths(),
      width: WIDTH,
      height: HEIGHT,
      roadTiles: roads.size,
      railTiles: rails.size,
      bridges: [...roads.values()].filter((road) => road.kind === 'bridge').length,
      buildings: buildings.length,
      trees: trees.length,
      cars: projectedCars.length,
      trains: trains.length,
      train: trains[0]
        ? {
            position: trainPosition(trains[0]),
            alpha: trainFadeAlpha(trainPosition(trains[0]), { height: HEIGHT, fadeTiles: trains[0].fadeTiles }),
            speed: trains[0].speed,
            fadeTiles: trains[0].fadeTiles,
            direction: 'northbound',
          }
        : null,
      pedestrians: projectedPedestrians.length,
      pedestrianSprites: pedestrianSprites.length,
      pedestrianSpriteSheets: [...new Set(pedestrianSprites.map((sprite) => sprite.sheet))],
      vehicleSprites: vehicleSprites.length,
      vehicleSheets: [...new Set(vehicleSprites.map((sprite) => sprite.sheet))],
      backend: {
        required: true,
        baseUrl: backendBaseUrl,
        status: backendStatus,
      },
      mobility: {
        source: 'backend',
        status: backendMobility.status,
        tick: backendMobility.tick,
        agents: backendMobility.agents,
        vehicles: backendMobility.vehicles,
        stops: backendMobility.stops,
        invalidMessages: backendMobility.invalidMessages,
        lastError: backendMobility.lastError,
      },
      mobilityAgents: {
        count: mobilityAgentEntries.length,
        selectedId: selectedAgentId,
        selected: selectedMobilityAgentEntry,
        agents: mobilityAgentEntries,
      },
      mobilityVehicles: {
        count: mobilityVehicleEntries.length,
        selectedId: selectedVehicleId,
        selected: selectedMobilityVehicleEntry,
        vehicles: mobilityVehicleEntries,
      },
      agentInspector: buildBackendPedestrianInspector(selectedAgent),
      vehicleInspector: buildBackendCarInspector(selectedVehicle),
      railStations: railStations.length,
      railYardTracks: Math.max(0, railPaths.length - 2),
      details: detailCounts,
      reserveTiles: countTerrainBase('Reserve'),
      validationErrors: 0,
      roadRailOverlap: diagnostics.roadRailOverlap,
      railCrossings: railCrossings.size,
      invalidBuildings: diagnostics.invalidBuildings,
      treeBuildingOverlap: diagnostics.treeBuildingOverlap,
      railStationsOnRoad: diagnostics.railStationsOnRoad,
      railStationsOnBuildings: diagnostics.railStationsOnBuildings,
      railStationsOnRails: diagnostics.railStationsOnRails,
      railStationsOnTrees: diagnostics.railStationsOnTrees,
      diagnostics,
      camera: {
        mode: 'bounded-fixed-map',
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
      },
    },
  });
};

function detailCountsByCategory(): Record<string, number> {
  const result: Record<string, number> = { total: details.length };
  for (const detail of details) {
    result[detail.category] = (result[detail.category] ?? 0) + 1;
  }
  return result;
}

function countTerrainBase(base: TerrainBaseKind): number {
  let count = 0;
  for (const tile of terrainState.tiles.values()) {
    if (tile.base === base) count += 1;
  }
  return count;
}

function nonPak128AssetPaths(): string[] {
  return [...images.keys()].filter((path) => !path.startsWith('/simutrans-assets/pak128/')).sort();
}

window.advanceTime = (ms: number) => {
  for (const train of trains) train.offset = trainWrappedOffset(train.offset + train.speed * (ms / 1000), train.path);
  render();
};
