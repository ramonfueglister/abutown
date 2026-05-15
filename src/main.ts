import './style.css';
import {
  buildingStreetFrontageOffset,
  countBuildingsWithoutDirectStreetAdjacency,
  hasDirectStreetAdjacency,
  hasVisibleStreetFrontage,
} from './city/buildingFrontage';
import { countAdjacentParallelRoadRuns, removeAdjacentParallelRoadRuns } from './city/roadParallelCleanup';
import { countInvalidRoadDeadEnds, pruneInvalidRoadDeadEnds } from './city/roadTopology';
import { buildPedestrianCorridors } from './city/pedestrianCorridors';
import { buildZurichPlacement } from './city/zurichPlacement';
import { buildZurichTransport } from './city/zurichTransport';
import { validateZurichCity } from './city/zurichValidation';
import { buildZurichWorld } from './city/zurichWorld';
import type { ZurichBuilding, ZurichDetail } from './city/worldTypes';
import { connectMobilityBackend, type MobilityBackendBridge } from './backend/mobilityClient';
import { createMobilityOverlayState, mobilityDiagnostics, type MobilityOverlayState } from './backend/mobilityState';
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
  vehicleFrameRect,
  type VehicleSheetName,
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
import { compareDrawableOrder } from './render/drawOrder';
import { buildPedestrianLoop, pedestrianWalkingSpeed } from './render/pedestrianMotion';
import {
  buildNorthboundTrainPath,
  trainFadeAlpha,
  trainPosition as movingTrainPosition,
  trainWrappedOffset,
} from './render/trainMotion';
import { drawMobilityOverlay } from './render/mobilityOverlay';

type Coord = { x: number; y: number };
type Terrain = 'grass' | 'water' | 'riverbank' | 'park';
type RoadKind = 'street' | 'bridge';
type RailTile = {
  coord: Coord;
  mask: number;
};

type RoadTile = {
  coord: Coord;
  kind: RoadKind;
  mask: number;
};

type Building = {
  coord: Coord;
  sheet: BuildingSheetName;
  frame: number;
  district: string;
};

type RailStation = {
  coord: Coord;
  frame: number;
};

type Car = {
  path: Coord[];
  offset: number;
  speed: number;
  sprite: VehicleSprite;
};

type Pedestrian = {
  path: Coord[];
  offset: number;
  speed: number;
  laneOffset: number;
  sprite: SimutransPedestrianSprite;
};

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
  | { type: 'detail'; coord: Coord; detail: ZurichDetail }
  | { type: 'tree'; coord: Coord }
  | { type: 'building'; coord: Coord; building: Building };

type CarDrawable = { type: 'car'; coord: Coord; car: Car };
type PedestrianDrawable = { type: 'pedestrian'; coord: Coord; pedestrian: Pedestrian };
type TrainDrawable = { type: 'train'; coord: Coord; train: Train };
type Drawable = StaticDrawable | TrainDrawable | CarDrawable | PedestrianDrawable;

type BuildingSheetName =
  | 'houses'
  | 'oldhouses'
  | 'cottages'
  | 'townhouses'
  | 'shops'
  | 'flats'
  | 'office'
  | 'modern'
  | 'tower'
  | 'church';

type BuildingSheet = {
  name: BuildingSheetName;
  file: string;
  cols: number;
  rows: number;
  scale: number;
};

type DistrictSeed = {
  id: string;
  center: Coord;
  radius: number;
  gridRadius: number;
  core: boolean;
  sheets: BuildingSheetName[];
};

const TILE_W = 64;
const TILE_H = 32;
const SPRITE_H = 42;
const CAMERA_EDGE_MARGIN = 8;
const CAMERA_EDGE_SOFTNESS = 4;
const CAMERA_MIN_SCALE = 0.48;
const CAMERA_MAX_SCALE = 3.2;
const VIEWPORT_GRID_PADDING = 9;
const OUTSKIRTS_TILES = 12;
const EDGE_EXIT_TILES = 7;
const NORTH = 1;
const EAST = 2;
const SOUTH = 4;
const WEST = 8;
const ROAD_SPRITE_STEP = 65;
const TRAIN_FADE_TILES = 12;
const TRAIN_SPEED = 8.5;

const zurichWorld = buildZurichWorld({ seed: 1848 });
const zurichTransport = buildZurichTransport(zurichWorld);
const zurichPlacement = buildZurichPlacement(zurichWorld, zurichTransport);
const zurichValidation = validateZurichCity(zurichWorld, zurichTransport, zurichPlacement);

const WIDTH = zurichWorld.width;
const HEIGHT = zurichWorld.height;

const roadFrameByMask = new Map<number, number>([
  [EAST | WEST, 0],
  [NORTH | SOUTH, 1],
  [NORTH | EAST | SOUTH | WEST, 2],
  [NORTH | EAST | WEST, 3],
  [NORTH | SOUTH | WEST, 4],
  [EAST | SOUTH | WEST, 5],
  [NORTH | EAST | SOUTH, 6],
  [SOUTH | WEST, 7],
  [NORTH | WEST, 8],
  [NORTH | EAST, 9],
  [EAST | SOUTH, 10],
]);

const canvasElement = document.querySelector<HTMLCanvasElement>('#game');
if (!canvasElement) throw new Error('Missing game canvas');
const canvas: HTMLCanvasElement = canvasElement;

const canvasContext = canvas.getContext('2d');
if (!canvasContext) throw new Error('Missing canvas context');
const ctx: CanvasRenderingContext2D = canvasContext;
ctx.imageSmoothingEnabled = false;

const camera = createCameraState({ x: 0, y: 0, scale: 0.56 });
let cameraInitialized = false;

const buildingSheets: BuildingSheet[] = [
  { name: 'houses', file: 'houses_shape.png', cols: 4, rows: 3, scale: 0.84 },
  { name: 'oldhouses', file: 'oldhouses_shape.png', cols: 4, rows: 3, scale: 0.84 },
  { name: 'cottages', file: 'cottages_shape.png', cols: 1, rows: 3, scale: 0.82 },
  { name: 'townhouses', file: 'townhouses_shape.png', cols: 2, rows: 3, scale: 0.86 },
  { name: 'shops', file: 'shopsandoffices_shape.png', cols: 6, rows: 3, scale: 0.82 },
  { name: 'flats', file: 'flats_shape.png', cols: 3, rows: 3, scale: 0.92 },
  { name: 'office', file: 'officeblocks_shape.png', cols: 4, rows: 3, scale: 0.92 },
  { name: 'modern', file: 'modernoffice_shape.png', cols: 2, rows: 4, scale: 0.84 },
  { name: 'tower', file: 'tallofficeblock_shape.png', cols: 4, rows: 3, scale: 0.86 },
  { name: 'church', file: 'churches_shape.png', cols: 1, rows: 3, scale: 0.84 },
];

const districtSeeds: DistrictSeed[] = [
  { id: 'old-town', center: { x: 32, y: 30 }, radius: 14, gridRadius: 7, core: true, sheets: ['oldhouses', 'townhouses', 'houses', 'shops'] },
  { id: 'market', center: { x: 50, y: 33 }, radius: 14, gridRadius: 7, core: true, sheets: ['shops', 'flats', 'office', 'townhouses'] },
  { id: 'rail-quarter', center: { x: 49, y: 48 }, radius: 13, gridRadius: 6, core: true, sheets: ['shops', 'flats', 'office', 'townhouses'] },
  { id: 'north-bank', center: { x: 29, y: 51 }, radius: 12, gridRadius: 7, core: false, sheets: ['houses', 'cottages', 'oldhouses', 'townhouses'] },
  { id: 'infill', center: { x: 40, y: 42 }, radius: 13, gridRadius: 7, core: true, sheets: ['oldhouses', 'shops', 'flats', 'townhouses'] },
  { id: 'civic', center: { x: 62, y: 51 }, radius: 13, gridRadius: 7, core: true, sheets: ['office', 'flats', 'shops', 'townhouses'] },
  { id: 'mill-yard', center: { x: 66, y: 30 }, radius: 12, gridRadius: 7, core: false, sheets: ['shops', 'office', 'flats', 'oldhouses'] },
  { id: 'west-garden', center: { x: 18, y: 24 }, radius: 11, gridRadius: 6, core: false, sheets: ['houses', 'cottages', 'oldhouses'] },
  { id: 'south-village', center: { x: 26, y: 64 }, radius: 11, gridRadius: 6, core: false, sheets: ['houses', 'cottages', 'townhouses'] },
  { id: 'east-suburb', center: { x: 76, y: 43 }, radius: 12, gridRadius: 7, core: false, sheets: ['houses', 'townhouses', 'shops'] },
  { id: 'south-east', center: { x: 76, y: 63 }, radius: 11, gridRadius: 6, core: false, sheets: ['houses', 'townhouses', 'shops'] },
  { id: 'north-east', center: { x: 78, y: 23 }, radius: 11, gridRadius: 6, core: false, sheets: ['houses', 'townhouses', 'shops'] },
];

const sheetByName = new Map(buildingSheets.map((sheet) => [sheet.name, sheet]));
const assetPaths = {
  grass: '/opengfx2/temperate_groundtiles_32bpp.png',
  water: '/opengfx2/universal_watertiles_32bpp.png',
  riverbank: '/opengfx2/universal_rivertiles_32bpp.png',
  bridge: '/opengfx2/general_bridgetiles_32bpp.png',
  road: '/opengfx2/road_town_overlayalpha.png',
  rail: '/opengfx2/rail_overlayalpha.png',
  railStation: '/opengfx2/railstations_shape.png',
  station64: '/opengfx2/all/stations__general__64__railstations_shape.png',
  railDepot: '/opengfx2/all/stations__general__64__raildepots_shape.png',
  roadDepot: '/opengfx2/all/stations__general__64__roaddepots_shape.png',
  roadStop: '/opengfx2/all/stations__general__64__roadstops_shape.png',
  dock: '/opengfx2/all/stations__general__64__docksandlocks_shape.png',
  ship: '/opengfx2/all/vehicles__64__water_32bpp.png',
  factory: '/opengfx2/all/industries__temperate__64__factory_shape.png',
  farm: '/opengfx2/all/industries__temperate__64__farm_shape.png',
  field: '/opengfx2/all/terrain__64__farm_groundtiles_32bpp.png',
  parkDetail: '/opengfx2/all/terrain__64__temperate_park_32bpp.png',
  tree: '/opengfx2/town_tree_32bpp.png',
  bus: '/opengfx2/road_buses_32bpp.png',
  lorry: '/opengfx2/road_lorries_firstgeneration_32bpp.png',
  trainEngine: '/opengfx2/all/vehicles__64__rail_engines_temperate_32bpp.png',
  trainWagon: '/opengfx2/all/vehicles__64__rail_wagons_32bpp.png',
};

const images = new Map<string, HTMLCanvasElement>();
const simutransSourceBounds = new Map<string, { x: number; y: number; width: number; height: number }>();
const terrain = new Map([...zurichWorld.terrain].map(([tileKey, tile]) => [tileKey, toRuntimeTerrain(tile.kind)]));
const roads = new Map<string, RoadTile>(
  [...zurichTransport.roads].map(([tileKey, road]) => [tileKey, { coord: road.coord, kind: road.kind, mask: road.mask }])
);
const rails = new Map<string, RailTile>(
  [...zurichTransport.rails].map(([tileKey, rail]) => [tileKey, { coord: rail.coord, mask: rail.mask }])
);
const railCrossings = zurichTransport.railCrossings;
const railReserved = new Set(rails.keys());
const railPaths = zurichTransport.railPaths;
const railYardPaths: Coord[][] = [];
const railStations = buildRailStations();
const buildings = zurichPlacement.buildings.map(toRuntimeBuilding);
const trees = zurichPlacement.trees;
const details = zurichPlacement.details;
const pedestrianDemandTiles = new Set<string>([
  ...buildings.map((building) => key(building.coord)),
  ...details.map((detail) => key(detail.coord)),
]);
const staticDrawables = buildStaticDrawables();
let vehicleSprites: VehicleSprite[] = [];
let pedestrianSprites: SimutransPedestrianSprite[] = [];
let cars: Car[] = [];
let pedestrians: Pedestrian[] = [];
let trains: Train[] = buildTrains();
let pedestrianCorridorCount = 0;
let mobilityState: MobilityOverlayState = createMobilityOverlayState();
let mobilityBridge: MobilityBackendBridge | null = null;
let previousTime = performance.now();

void boot();

async function boot(): Promise<void> {
  const imageEntries = [
    ...Object.values(assetPaths).map((path) => [path, path] as const),
    ...Object.values(SIMUTRANS_PEDESTRIAN_ASSET_PATHS).map((path) => [path, path] as const),
    ...buildingSheets.map((sheet) => [`/opengfx2/${sheet.file}`, `/opengfx2/${sheet.file}`] as const),
  ];
  await Promise.all(imageEntries.map(async ([key, path]) => images.set(key, await loadCleanImage(path))));
  vehicleSprites = usableVehicleSprites();
  pedestrianSprites = candidateSimutransPedestrianSprites();
  cars = buildCars(vehicleSprites);
  pedestrians = buildPedestrians(pedestrianSprites);
  const mobilityBackend = configuredMobilityBackend();
  if (mobilityBackend.enabled) {
    mobilityBridge = connectMobilityBackend({
      baseUrl: mobilityBackend.baseUrl,
      onState: (nextState) => {
        mobilityState = nextState;
      },
    });
    window.addEventListener('beforeunload', () => mobilityBridge?.stop(), { once: true });
  }
  resize();
  window.addEventListener('resize', resize);
  attachCamera();
  canvas.dataset.ready = 'true';
  requestAnimationFrame(frame);
}

function resize(): void {
  const ratio = window.devicePixelRatio || 1;
  canvas.width = Math.floor(window.innerWidth * ratio);
  canvas.height = Math.floor(window.innerHeight * ratio);
  canvas.style.width = `${window.innerWidth}px`;
  canvas.style.height = `${window.innerHeight}px`;
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  ctx.imageSmoothingEnabled = false;
  if (!cameraInitialized) {
    const focus = iso({ x: Math.floor(WIDTH / 2), y: Math.floor(HEIGHT / 2) });
    camera.targetX = window.innerWidth / 2 - focus.x * camera.targetScale;
    camera.targetY = Math.min(445, window.innerHeight * 0.58) - focus.y * camera.targetScale;
    camera.x = camera.targetX;
    camera.y = camera.targetY;
    camera.scale = camera.targetScale;
    cameraInitialized = true;
  }
  constrainCamera(false);
}

function attachCamera(): void {
  canvas.addEventListener('pointerdown', (event) => {
    camera.dragging = true;
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
  canvas.addEventListener('pointerup', () => {
    camera.dragging = false;
    constrainCamera(false);
  });
  canvas.addEventListener('pointercancel', () => {
    camera.dragging = false;
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
  for (const car of cars) car.offset = (car.offset + car.speed * dt) % car.path.length;
  for (const pedestrian of pedestrians) pedestrian.offset = (pedestrian.offset + pedestrian.speed * dt) % pedestrian.path.length;
  for (const train of trains) train.offset = trainWrappedOffset(train.offset + train.speed * dt, train.path);
  if (!camera.dragging) constrainCamera(false);
  dampCamera(camera, dt, 18);
  render();
  requestAnimationFrame(frame);
}

function render(): void {
  ctx.save();
  ctx.setTransform(window.devicePixelRatio || 1, 0, 0, window.devicePixelRatio || 1, 0, 0);
  ctx.imageSmoothingEnabled = false;
  ctx.fillStyle = '#050705';
  ctx.fillRect(0, 0, window.innerWidth, window.innerHeight);
  ctx.translate(camera.x, camera.y);
  ctx.scale(camera.scale, camera.scale);

  drawScene({ x: 0, y: 0 });
  ctx.restore();
}

function drawScene(offset: Coord): void {
  ctx.save();
  const sceneOffset = iso(offset);
  ctx.translate(sceneOffset.x, sceneOffset.y);
  const visibleGrid = visibleGridRect();

  drawOutskirtsTerrain(visibleGrid);
  for (let y = Math.max(0, visibleGrid.minY); y <= Math.min(HEIGHT - 1, visibleGrid.maxY); y += 1) {
    for (let x = Math.max(0, visibleGrid.minX); x <= Math.min(WIDTH - 1, visibleGrid.maxX); x += 1) drawTerrain({ x, y });
  }
  drawEdgeConnections(visibleGrid);

  const visibleStaticDrawables = staticDrawables.filter((item) => isCoordVisible(item.coord, visibleGrid));
  const carDrawables = cars
    .map((car) => ({ type: 'car' as const, coord: carPosition(car), car }))
    .filter((item) => isCoordVisible(item.coord, visibleGrid))
    .sort(compareDrawables);
  const pedestrianDrawables = pedestrians
    .map((pedestrian) => ({ type: 'pedestrian' as const, coord: pedestrianPosition(pedestrian), pedestrian }))
    .filter((item) => isCoordVisible(item.coord, visibleGrid))
    .sort(compareDrawables);
  const trainDrawables = trains
    .map((train) => ({ type: 'train' as const, coord: trainPosition(train), train }))
    .filter((item) => isCoordVisible(item.coord, visibleGrid))
    .sort(compareDrawables);

  for (const item of mergeSortedDrawables(visibleStaticDrawables, [...trainDrawables, ...carDrawables, ...pedestrianDrawables].sort(compareDrawables))) {
    if (item.type === 'rail') drawRail(item.rail);
    if (item.type === 'road') drawRoad(item.road);
    if (item.type === 'railStation') drawRailStation(item.station);
    if (item.type === 'detail') drawDetail(item.detail);
    if (item.type === 'tree') drawTree(item.coord);
    if (item.type === 'building') drawBuilding(item.building);
    if (item.type === 'train') drawTrain(item.train);
    if (item.type === 'car') drawCar(item.car);
    if (item.type === 'pedestrian') drawPedestrian(item.pedestrian);
  }

  drawPerimeterMist();
  drawMobilityOverlay(ctx, mobilityState, {
    project: iso,
    isVisible: (coord) => isCoordVisible(coord, visibleGrid),
  });
  ctx.restore();
}

function drawTerrain(coord: Coord): void {
  const kind = terrain.get(key(coord)) ?? 'grass';
  const point = iso(coord);
  const image = images.get(kind === 'water' ? assetPaths.water : kind === 'riverbank' ? assetPaths.riverbank : assetPaths.grass);
  if (!image) return;
  const sx = kind === 'water' ? 0 : kind === 'riverbank' ? 0 : 0;
  ctx.drawImage(image, sx, 0, 64, SPRITE_H, point.x - TILE_W / 2, point.y - 12, TILE_W, SPRITE_H);
}

function drawOutskirtsTerrain(visibleGrid: GridRect): void {
  const image = images.get(assetPaths.grass);
  if (!image) return;

  for (let y = Math.max(-OUTSKIRTS_TILES, visibleGrid.minY); y <= Math.min(HEIGHT - 1 + OUTSKIRTS_TILES, visibleGrid.maxY); y += 1) {
    for (let x = Math.max(-OUTSKIRTS_TILES, visibleGrid.minX); x <= Math.min(WIDTH - 1 + OUTSKIRTS_TILES, visibleGrid.maxX); x += 1) {
      const coord = { x, y };
      if (isInsidePlayableMap(coord)) continue;
      const edgeDistance = distanceOutsidePlayableMap(coord);
      if (edgeDistance > OUTSKIRTS_TILES) continue;

      const point = iso(coord);
      const fade = 1 - edgeDistance / (OUTSKIRTS_TILES + 1);
      ctx.save();
      ctx.globalAlpha = 0.05 + fade * 0.22;
      ctx.drawImage(image, 0, 0, 64, SPRITE_H, point.x - TILE_W / 2, point.y - 12, TILE_W, SPRITE_H);
      if (hash(`outskirts-shadow:${x}:${y}`) % 11 === 0) {
        ctx.fillStyle = `rgba(5, 7, 5, ${0.035 + (1 - fade) * 0.055})`;
        drawIsoTile(point);
      }
      ctx.restore();
    }
  }
}

function drawRoad(road: RoadTile): void {
  const point = iso(road.coord);
  const roadImage = images.get(assetPaths.road);
  ctx.save();
  if (road.kind === 'bridge') {
    const bridge = images.get(assetPaths.bridge);
    if (bridge) {
      const bridgeFrameW = spriteSheetCellSize(bridge.width, 19);
      const bridgeFrame = road.mask & (NORTH | SOUTH) ? 1 : 0;
      ctx.drawImage(bridge, bridgeFrame * bridgeFrameW, 0, bridgeFrameW, bridge.height, point.x - 32, point.y - 14, 64, 48);
    }
  }

  if (roadImage) {
    const frame = roadSpriteFrame(road.mask);
    ctx.drawImage(roadImage, frame * ROAD_SPRITE_STEP, 0, 64, SPRITE_H, point.x - 32, point.y - 12, 64, SPRITE_H);
  }
  ctx.restore();
}

function drawRail(rail: RailTile): void {
  const point = iso(rail.coord);
  const railImage = images.get(assetPaths.rail);
  if (!railImage) return;
  const frame = roadSpriteFrame(rail.mask);
  ctx.drawImage(railImage, frame * ROAD_SPRITE_STEP, 0, 64, SPRITE_H, point.x - 32, point.y - 12, 64, SPRITE_H);
}

function drawRailStation(station: RailStation): void {
  const image = images.get(assetPaths.railStation);
  if (!image) return;
  const cols = 5;
  const rows = 4;
  const col = station.frame % cols;
  const row = Math.floor(station.frame / cols) % rows;
  const cellW = spriteSheetCellSize(image.width, cols);
  const cellH = spriteSheetCellSize(image.height, rows);
  const point = iso(station.coord);
  ctx.drawImage(image, col * cellW, row * cellH, cellW, cellH, point.x - 33, point.y - 32, 66, 66);
}

function drawDetail(detail: ZurichDetail): void {
  if (!shouldRenderDetail(detail)) return;
  if (detail.category === 'field') {
    drawGroundDetail(assetPaths.field, detail.coord, 19, 9);
    return;
  }
  if (detail.category === 'park' || detail.category === 'civic' || detail.category === 'decor') {
    drawGroundDetail(assetPaths.parkDetail, detail.coord, 2, 1);
    return;
  }
  if (detail.assetCategory === 'station-roof') {
    drawGridSprite(assetPaths.station64, detail.coord, 5, 4, [0, 5, 10, 15][hash(`station:${key(detail.coord)}`) % 4], 0.98, -22);
    return;
  }
  if (detail.assetCategory === 'road-stop') {
    drawGridSprite(assetPaths.roadStop, detail.coord, 10, 4, hash(`road-stop:${key(detail.coord)}`) % 20, 0.92, -28);
    return;
  }
  if (detail.assetCategory === 'rail-depot') {
    drawGridSprite(assetPaths.railDepot, detail.coord, 1, 4, hash(`rail-depot:${key(detail.coord)}`) % 4, 0.96, -42);
    return;
  }
  if (detail.assetCategory === 'road-depot') {
    drawGridSprite(assetPaths.roadDepot, detail.coord, 1, 4, hash(`road-depot:${key(detail.coord)}`) % 4, 0.92, -42);
    return;
  }
  if (detail.assetCategory === 'ship') {
    drawGridSprite(assetPaths.ship, detail.coord, 8, 4, 1 + (hash(`ship:${key(detail.coord)}`) % 22), 0.72, -36);
    return;
  }
  if (detail.assetCategory === 'dock' || detail.assetCategory === 'quay') {
    drawGridSprite(assetPaths.dock, detail.coord, 8, 4, hash(`dock:${key(detail.coord)}`) % 28, 0.98, -30);
    return;
  }
  if (detail.assetCategory === 'factory') {
    drawGridSprite(assetPaths.factory, detail.coord, 2, 3, hash(`factory:${key(detail.coord)}`) % 2, 0.9, -76);
    return;
  }
  drawGridSprite(assetPaths.farm, detail.coord, 5, 1, hash(`farm:${key(detail.coord)}`) % 5, 0.86, -38);
}

function drawGroundDetail(path: string, coord: Coord, cols: number, rows: number): void {
  const image = images.get(path);
  if (!image) return;
  const frame = hash(`ground-detail:${path}:${key(coord)}`) % (cols * rows);
  const cellW = spriteSheetCellSize(image.width, cols);
  const cellH = spriteSheetCellSize(image.height, rows);
  const col = frame % cols;
  const row = Math.floor(frame / cols) % rows;
  const point = iso(coord);
  ctx.drawImage(image, col * cellW, row * cellH, Math.min(64, cellW), Math.min(SPRITE_H, cellH), point.x - 32, point.y - 12, 64, SPRITE_H);
}

function drawGridSprite(path: string, coord: Coord, cols: number, rows: number, frame: number, scale: number, yOffset: number): void {
  const image = images.get(path);
  if (!image) return;
  const col = frame % cols;
  const row = Math.floor(frame / cols) % rows;
  const cellW = spriteSheetCellSize(image.width, cols);
  const cellH = spriteSheetCellSize(image.height, rows);
  const point = iso(coord);
  const w = cellW * scale;
  const h = cellH * scale;
  ctx.drawImage(image, col * cellW, row * cellH, cellW, cellH, point.x - w / 2, point.y + yOffset, w, h);
}

function drawBuilding(building: Building): void {
  const sheet = sheetByName.get(building.sheet);
  const image = sheet ? images.get(`/opengfx2/${sheet.file}`) : undefined;
  if (!sheet || !image) return;
  const col = building.frame % sheet.cols;
  const row = Math.floor(building.frame / sheet.cols) % sheet.rows;
  const cellW = spriteSheetCellSize(image.width, sheet.cols);
  const cellH = spriteSheetCellSize(image.height, sheet.rows);
  const point = iso(building.coord);
  const offset = buildingStreetFrontageOffset(building.coord, roads);
  const w = cellW * sheet.scale;
  const h = cellH * sheet.scale;
  ctx.drawImage(image, col * cellW, row * cellH, cellW, cellH, point.x - w / 2 + offset.x, point.y + 4 - h + offset.y, w, h);
}

function drawTree(coord: Coord): void {
  const image = images.get(assetPaths.tree);
  if (!image) return;
  const point = iso(coord);
  const isForest = zurichWorld.terrain.get(key(coord))?.kind === 'forest';
  const forestFrames = [
    { sx: 0, sy: 0, w: 16, h: 43 },
    { sx: 16, sy: 0, w: 17, h: 43 },
    { sx: 33, sy: 0, w: 28, h: 43 },
  ];
  const frame = isForest ? forestFrames[hash(`tree-frame:${key(coord)}`) % forestFrames.length] : forestFrames[0];
  const jitterX = (hash(`tree-x:${key(coord)}`) % 13) - 6;
  const jitterY = (hash(`tree-y:${key(coord)}`) % 9) - 4;
  const scale = (isForest ? 0.98 : 0.82) + (hash(`tree-scale:${key(coord)}`) % 23) / 100;
  const w = frame.w * scale;
  const h = frame.h * scale;
  ctx.drawImage(image, frame.sx, frame.sy, frame.w, frame.h, point.x - w / 2 + jitterX, point.y - h + 7 + jitterY, w, h);
}

function drawCar(car: Car): void {
  const image = images.get(vehicleAssetPath(car.sprite.sheet));
  if (!image) return;
  const base = Math.floor(car.offset);
  const current = car.path[base];
  const next = car.path[(base + 1) % car.path.length];
  const pos = carPosition(car);
  const point = iso(pos);
  const currentPoint = iso(current);
  const nextPoint = iso(next);
  const lane = screenRightLaneOffset(currentPoint, nextPoint, 5.5);
  const frame = vehicleFrameForGridDelta({ x: next.x - current.x, y: next.y - current.y });
  const rect = vehicleFrameRect(car.sprite, frame);
  if (rect.x >= image.width || rect.y >= image.height) return;
  const sourceWidth = Math.min(rect.width, image.width - rect.x);
  const sourceHeight = Math.min(rect.height, image.height - rect.y);
  const scale = car.sprite.scale;
  const width = sourceWidth * scale;
  const height = sourceHeight * scale;
  ctx.save();
  ctx.translate(point.x + lane.x, point.y + lane.y + 7);
  ctx.drawImage(image, rect.x, rect.y, sourceWidth, sourceHeight, -width / 2, -height, width, height);
  ctx.restore();
}

function drawTrain(train: Train): void {
  const engineImage = images.get(assetPaths.trainEngine);
  const wagonImage = images.get(assetPaths.trainWagon);
  if (!engineImage || !wagonImage) return;

  const segments = [
    { image: engineImage, source: { x: 153, y: 5, width: 20, height: 15 }, offset: train.offset, scale: 4.0 },
    { image: wagonImage, source: { x: 153, y: 4, width: 20, height: 16 }, offset: train.offset - train.carSpacing, scale: 3.85 },
    { image: wagonImage, source: { x: 153, y: 4, width: 20, height: 16 }, offset: train.offset - train.carSpacing * 2, scale: 3.85 },
    { image: wagonImage, source: { x: 153, y: 4, width: 20, height: 16 }, offset: train.offset - train.carSpacing * 3, scale: 3.85 },
    { image: wagonImage, source: { x: 153, y: 4, width: 20, height: 16 }, offset: train.offset - train.carSpacing * 4, scale: 3.85 },
  ].sort((a, b) => iso(movingTrainPosition(train.path, a.offset)).y - iso(movingTrainPosition(train.path, b.offset)).y);

  for (const segment of segments) {
    const pos = movingTrainPosition(train.path, segment.offset);
    const alpha = trainFadeAlpha(pos, { height: HEIGHT, fadeTiles: train.fadeTiles });
    if (alpha <= 0) continue;
    const point = iso(pos);
    const width = segment.source.width * segment.scale;
    const height = segment.source.height * segment.scale;
    ctx.save();
    ctx.globalAlpha *= alpha;
    ctx.translate(point.x, point.y + 8);
    ctx.fillStyle = 'rgba(0, 0, 0, 0.28)';
    ctx.beginPath();
    ctx.ellipse(0, -4, width * 0.42, 5.5, -Math.PI / 7, 0, Math.PI * 2);
    ctx.fill();
    ctx.drawImage(
      segment.image,
      segment.source.x,
      segment.source.y,
      segment.source.width,
      segment.source.height,
      -width / 2,
      -height,
      width,
      height,
    );
    ctx.restore();
  }
}

function drawPedestrian(pedestrian: Pedestrian): void {
  const image = images.get(SIMUTRANS_PEDESTRIAN_ASSET_PATHS[pedestrian.sprite.sheet]);
  if (!image) return;
  const base = Math.floor(pedestrian.offset);
  const current = pedestrian.path[base];
  const next = pedestrian.path[(base + 1) % pedestrian.path.length];
  const pos = pedestrianPosition(pedestrian);
  const point = iso(pos);
  const currentPoint = iso(current);
  const nextPoint = iso(next);
  const lane = screenRightLaneOffset(currentPoint, nextPoint, 4 + pedestrian.laneOffset);
  const direction = simutransPedestrianFrameForGridDelta({ x: next.x - current.x, y: next.y - current.y });
  const rect = simutransPedestrianFrameRect(pedestrian.sprite, direction);
  const visible = visibleSourceBounds(image, pedestrian.sprite, direction, rect);
  const scale = simutransPedestrianDisplayScale(pedestrian.sprite.scale, camera.scale);
  const width = visible.width * scale;
  const height = visible.height * scale;
  ctx.save();
  ctx.translate(point.x + lane.x, point.y + lane.y + 5);
  ctx.fillStyle = 'rgba(0, 0, 0, 0.28)';
  ctx.fillRect(-Math.max(2, width * 0.18), -Math.max(1, height * 0.08), Math.max(4, width * 0.36), Math.max(2, height * 0.1));
  ctx.drawImage(image, visible.x, visible.y, visible.width, visible.height, -width / 2, -height, width, height);
  ctx.restore();
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
  const top = iso({ x: 0, y: 0 });
  const east = iso({ x: WIDTH - 1, y: 0 });
  const bottom = iso({ x: WIDTH - 1, y: HEIGHT - 1 });
  const west = iso({ x: 0, y: HEIGHT - 1 });
  const padX = OUTSKIRTS_TILES * TILE_W;
  const padY = OUTSKIRTS_TILES * TILE_H;
  const minX = west.x - padX;
  const maxX = east.x + padX;
  const minY = top.y - padY;
  const maxY = bottom.y + padY * 1.5;

  ctx.save();
  ctx.fillStyle = 'rgba(180, 196, 170, 0.07)';
  ctx.beginPath();
  ctx.rect(minX, minY, maxX - minX, maxY - minY);
  ctx.moveTo(top.x, top.y - 22);
  ctx.lineTo(east.x + 38, east.y);
  ctx.lineTo(bottom.x, bottom.y + 34);
  ctx.lineTo(west.x - 38, west.y);
  ctx.closePath();
  ctx.fill('evenodd');

  ctx.strokeStyle = 'rgba(225, 232, 212, 0.08)';
  ctx.lineWidth = 92;
  ctx.lineJoin = 'round';
  ctx.beginPath();
  ctx.moveTo(top.x, top.y - 8);
  ctx.lineTo(east.x + 16, east.y);
  ctx.lineTo(bottom.x, bottom.y + 16);
  ctx.lineTo(west.x - 16, west.y);
  ctx.closePath();
  ctx.stroke();
  ctx.restore();
}

function toRuntimeTerrain(kind: string): Terrain {
  if (kind === 'water') return 'water';
  if (kind === 'riverbank') return 'riverbank';
  if (kind === 'park' || kind === 'forest' || kind === 'reserve' || kind === 'plaza') return 'park';
  return 'grass';
}

function toRuntimeBuilding(building: ZurichBuilding): Building {
  return {
    coord: building.coord,
    sheet: building.sheet,
    frame: building.frame,
    district: building.zoneId,
  };
}

function buildTerrain(): Map<string, Terrain> {
  const result = new Map<string, Terrain>();
  for (let y = 0; y < HEIGHT; y += 1) {
    for (let x = 0; x < WIDTH; x += 1) {
      let kind: Terrain = 'grass';
      if (distance({ x, y }, { x: 44, y: 58 }) < 6 || distance({ x, y }, { x: 68, y: 57 }) < 5) kind = 'park';
      result.set(`${x}:${y}`, kind);
    }
  }
  return result;
}

function buildRoadNetwork(): Map<string, RoadTile> {
  const points = new Map<string, RoadKind>();
  const maxX = WIDTH - 1;
  const maxY = HEIGHT - 1;
  const addPath = (path: Coord[]) => {
    for (const point of path) addRoadPoint(points, point);
  };
  const arterialPaths = [
    linePath({ x: 0, y: 42 }, { x: maxX, y: 42 }),
    linePath({ x: 52, y: 0 }, { x: 52, y: maxY }),
    roadRoute([{ x: 0, y: 58 }, { x: 18, y: 54 }, { x: 36, y: 52 }, { x: 56, y: 50 }, { x: 78, y: 55 }, { x: maxX, y: 58 }]),
    roadRoute([{ x: 6, y: 21 }, { x: 22, y: 23 }, { x: 38, y: 26 }, { x: 54, y: 23 }, { x: 75, y: 20 }, { x: maxX, y: 18 }]),
    roadRoute([{ x: 14, y: 42 }, { x: 24, y: 48 }, { x: 32, y: 42 }, { x: 42, y: 42 }]),
    roadRoute([{ x: 47, y: 42 }, { x: 50, y: 34 }, { x: 66, y: 30 }, { x: maxX, y: 30 }]),
    roadRoute([{ x: 47, y: 42 }, { x: 60, y: 48 }, { x: 65, y: 64 }, { x: 52, y: maxY }]),
    roadRoute([{ x: 12, y: maxY }, { x: 28, y: 68 }, { x: 46, y: 66 }, { x: 68, y: 68 }, { x: maxX, y: 70 }]),
  ];

  for (const path of arterialPaths) addPath(path);

  for (const district of districtSeeds) addDistrictStreets(points, district.center, district.gridRadius, district.core);

  const protectedRoads = new Set(arterialPaths.flatMap((path) => path.map(key)));
  removeAdjacentParallelRoadRuns(points, protectedRoads);
  removeStraightParallelRoads(points, protectedRoads);
  pruneDeadEnds(points, protectedRoads);
  pruneInvalidRoadDeadEnds(points, { width: WIDTH, height: HEIGHT });

  const roads = new Map<string, RoadTile>();
  for (const [tileKey, kind] of points) {
    const coord = parseKey(tileKey);
    const mask =
      roadMask(points, coord);
    roads.set(tileKey, { coord, kind, mask });
  }
  return roads;
}

function removeStraightParallelRoads(points: Map<string, RoadKind>, protectedPoints: ReadonlySet<string>): void {
  const removable = new Set<string>();
  for (const tileKey of points.keys()) {
    if (protectedPoints.has(tileKey)) continue;
    const coord = parseKey(tileKey);
    const mask = roadMask(points, coord);
    const south = { x: coord.x, y: coord.y + 1 };
    const east = { x: coord.x + 1, y: coord.y };
    if (isStraightEastWest(mask) && points.has(key(south)) && isStraightEastWest(roadMask(points, south))) {
      removable.add(tileKey);
    }
    if (isStraightNorthSouth(mask) && points.has(key(east)) && isStraightNorthSouth(roadMask(points, east))) {
      removable.add(tileKey);
    }
  }
  for (const tileKey of removable) points.delete(tileKey);
}

function pruneDeadEnds(points: Map<string, RoadKind>, protectedPoints: ReadonlySet<string>): void {
  for (let pass = 0; pass < 5; pass += 1) {
    const removable: string[] = [];
    for (const tileKey of points.keys()) {
      if (protectedPoints.has(tileKey)) continue;
      const coord = parseKey(tileKey);
      const degree = cardinal(coord).filter((neighbor) => points.has(key(neighbor))).length;
      if (degree <= 1) removable.push(tileKey);
    }
    if (removable.length === 0) return;
    for (const tileKey of removable) points.delete(tileKey);
  }
}

function buildRailPaths(): Coord[][] {
  const maxX = WIDTH - 1;
  return [
    railRoute([{ x: 0, y: 64 }, { x: maxX, y: 64 }]),
  ];
}

function buildRailReserved(paths: Coord[][]): Set<string> {
  const result = new Set<string>();
  for (const path of paths) {
    for (const point of path) {
      if (inside(point) && terrain.get(key(point)) !== 'water') result.add(key(point));
    }
  }
  return result;
}

function buildRailCrossings(): Set<string> {
  return new Set(['52:64']);
}

function buildRailNetwork(paths: Coord[][]): Map<string, RailTile> {
  const points = new Set<string>();
  for (const path of paths) {
    for (const point of path) {
      if (terrain.get(key(point)) !== 'water') points.add(key(point));
    }
  }

  const result = new Map<string, RailTile>();
  for (const tileKey of points) {
    const coord = parseKey(tileKey);
    const mask =
      (points.has(key({ x: coord.x, y: coord.y - 1 })) ? NORTH : 0) |
      (points.has(key({ x: coord.x + 1, y: coord.y })) ? EAST : 0) |
      (points.has(key({ x: coord.x, y: coord.y + 1 })) ? SOUTH : 0) |
      (points.has(key({ x: coord.x - 1, y: coord.y })) ? WEST : 0);
    result.set(tileKey, { coord, mask });
  }
  return result;
}

function buildRailStations(): RailStation[] {
  return [];
}

function addDistrictStreets(points: Map<string, RoadKind>, center: Coord, radius: number, dense: boolean): void {
  const arm = dense ? radius : Math.max(4, radius - 2);
  const half = Math.max(3, Math.floor(radius / 2));
  if (dense) {
    addStreetSegment(points, { x: center.x - arm, y: center.y }, { x: center.x + arm, y: center.y });
    if (hash(`district-axis:${key(center)}`) % 2 === 0) {
      addStreetSegment(points, { x: center.x, y: center.y - half }, { x: center.x, y: center.y + half });
    }
    addUrbanBlock(points, center, Math.max(5, radius - 1), Math.max(4, Math.floor(radius * 0.65)));
  } else if (hash(`district-axis:${key(center)}`) % 2 === 0) {
    addStreetSegment(points, { x: center.x - arm, y: center.y }, { x: center.x + arm, y: center.y });
    addStreetSegment(points, { x: center.x, y: center.y - half }, { x: center.x, y: center.y + half });
  } else {
    addStreetSegment(points, { x: center.x, y: center.y - arm }, { x: center.x, y: center.y + arm });
    addStreetSegment(points, { x: center.x - half, y: center.y }, { x: center.x + half, y: center.y });
  }
}

function addStreetSegment(points: Map<string, RoadKind>, from: Coord, to: Coord): void {
  for (const coord of cardinalLinePath(from, to)) addRoadPoint(points, coord);
}

function addUrbanBlock(points: Map<string, RoadKind>, center: Coord, halfWidth: number, halfHeight: number): void {
  const west = center.x - halfWidth;
  const east = center.x + halfWidth;
  const north = center.y - halfHeight;
  const south = center.y + halfHeight;
  addStreetSegment(points, { x: west, y: north }, { x: east, y: north });
  addStreetSegment(points, { x: east, y: north }, { x: east, y: south });
  addStreetSegment(points, { x: east, y: south }, { x: west, y: south });
  addStreetSegment(points, { x: west, y: south }, { x: west, y: north });
}

function addRoadPoint(points: Map<string, RoadKind>, coord: Coord): void {
  if (!inside(coord)) return;
  if (railReserved.has(key(coord)) && !railCrossings.has(key(coord))) return;
  points.set(key(coord), terrain.get(key(coord)) === 'water' ? 'bridge' : 'street');
}

function buildBuildings(): Building[] {
  const result: Building[] = [];
  const streetFrontages = buildStreetFrontages();
  const occupied = new Set<string>([
    ...roads.keys(),
    ...rails.keys(),
    ...railStations.map((station) => key(station.coord)),
  ]);
  const placeBuilding = (coord: Coord, district: DistrictSeed): boolean => {
    if (result.length >= 1800 || occupied.has(key(coord)) || !isBuildable(coord)) return false;
    if (!streetFrontages.has(key(coord))) return false;
    if (!hasDirectStreetAdjacency(coord, roads)) return false;
    if (!hasVisibleStreetFrontage(coord, roads)) return false;
    if (touchesRail(coord)) return false;
    if (!district.core && hash(key(coord)) % 12 === 0) return false;
    const sheet = district.sheets[hash(`${district.id}:${key(coord)}`) % district.sheets.length];
    const meta = sheetByName.get(sheet);
    if (!meta) return false;
    const frameCount = meta.cols * usableRows(meta);
    result.push({ coord, sheet, frame: hash(`${key(coord)}:${sheet}`) % frameCount, district: district.id });
    occupied.add(key(coord));
    return true;
  };

  for (const district of districtSeeds) {
    const candidates: Coord[] = [];
    for (let y = district.center.y - district.radius; y <= district.center.y + district.radius; y += 1) {
      for (let x = district.center.x - district.radius; x <= district.center.x + district.radius; x += 1) {
        const coord = { x, y };
        if (!inside(coord) || occupied.has(key(coord)) || !isBuildable(coord)) continue;
        if (!streetFrontages.has(key(coord))) continue;
        if (!hasVisibleStreetFrontage(coord, roads)) continue;
        candidates.push(coord);
      }
    }
    candidates.sort((a, b) => distance(a, district.center) - distance(b, district.center) || a.y - b.y || a.x - b.x);
    for (const coord of candidates) {
      placeBuilding(coord, district);
    }
  }

  const frontages: Coord[] = [...streetFrontages].map(parseKey).filter((coord) =>
    inside(coord) && !occupied.has(key(coord)) && isBuildable(coord)
  );
  frontages.sort((a, b) => hash(`frontage:${key(a)}`) - hash(`frontage:${key(b)}`));
  for (const coord of frontages) {
    const district = nearestDistrict(coord);
    if (distance(coord, district.center) > district.radius + 4) continue;
    placeBuilding(coord, district);
  }
  return result;
}

function buildStreetFrontages(): Set<string> {
  const result = new Set<string>();
  for (const road of roads.values()) {
    if (road.kind !== 'street') continue;
    for (const coord of cardinal(road.coord)) {
      if (inside(coord) && isBuildable(coord)) result.add(key(coord));
    }
  }
  return result;
}

function touchesRail(coord: Coord): boolean {
  return [coord, ...cardinal(coord)].some((neighbor) => railReserved.has(key(neighbor)));
}

function buildTrees(): Coord[] {
  const result: Coord[] = [];
  const blocked = new Set<string>([
    ...roads.keys(),
    ...rails.keys(),
    ...buildings.map((building) => key(building.coord)),
  ]);
  for (let y = 0; y < HEIGHT; y += 1) {
    for (let x = 0; x < WIDTH; x += 1) {
      const coord = { x, y };
      if (!isBuildable(coord) || blocked.has(key(coord))) continue;
      const outsideUrban = Math.min(
        ...districtSeeds.map((district) => distance(coord, district.center)),
      ) > 15;
      if (
        (outsideUrban && hash(`forest:${key(coord)}`) % 9 === 0) ||
        (terrain.get(key(coord)) === 'park' && hash(key(coord)) % 5 === 0) ||
        hash(`tree:${key(coord)}`) % 97 === 0
      ) {
        result.push(coord);
      }
    }
  }
  return result;
}

function buildCars(sprites: VehicleSprite[]): Car[] {
  if (sprites.length === 0) return [];
  const baseCorridors = zurichTransport.arterialPaths.filter((path) => path.length >= 2);
  if (baseCorridors.length === 0) return [];
  const corridors = baseCorridors.flatMap((path) => [path, [...path].reverse()]);
  return Array.from({ length: 156 }, (_, index) => {
    const path = corridors[index % corridors.length];
    return {
      path,
      offset: (index * 7 + Math.floor(index / corridors.length) * 3) % path.length,
      speed: 1.15 + (index % 9) * 0.13,
      sprite: sprites[index % sprites.length],
    };
  });
}

function buildPedestrians(sprites: SimutransPedestrianSprite[]): Pedestrian[] {
  if (sprites.length === 0) return [];
  const roadCorridors = buildPedestrianCorridors(roads, { minLength: 5, maxCorridors: 260 });
  const urbanRoadCorridors = roadCorridors.flatMap(urbanPedestrianSegments);
  const urbanArterials = zurichTransport.arterialPaths.flatMap(urbanPedestrianSegments);
  const baseCorridors = [
    ...(urbanRoadCorridors.length > 0 ? urbanRoadCorridors : roadCorridors.slice(0, 72)),
    ...urbanArterials,
  ];
  if (baseCorridors.length === 0) return [];
  const corridors = baseCorridors.flatMap((path) => [path, [...path].reverse()]).map(buildPedestrianLoop);
  pedestrianCorridorCount = corridors.length;
  const totalPathTiles = baseCorridors.reduce((sum, path) => sum + path.length, 0);
  const pedestrianCount = Math.min(420, Math.max(220, Math.floor(totalPathTiles / 3)));
  return Array.from({ length: pedestrianCount }, (_, index) => {
    const path = corridors[index % corridors.length];
    const sprite = sprites[index % sprites.length];
    return {
      path,
      offset: (index * 11 + Math.floor(index / corridors.length) * 5) % path.length,
      speed: pedestrianWalkingSpeed(index),
      laneOffset: ((index % 5) - 2) * 0.45,
      sprite,
    };
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

function urbanPedestrianSegments(path: Coord[]): Coord[][] {
  const segments: Coord[][] = [];
  let current: Coord[] = [];

  for (const coord of path) {
    if (hasPedestrianDemandNearby(coord)) {
      current.push(coord);
      continue;
    }
    if (current.length >= 3) segments.push(current);
    current = [];
  }
  if (current.length >= 3) segments.push(current);
  return segments;
}

function hasPedestrianDemandNearby(coord: Coord): boolean {
  for (let y = coord.y - 4; y <= coord.y + 4; y += 1) {
    for (let x = coord.x - 4; x <= coord.x + 4; x += 1) {
      if (pedestrianDemandTiles.has(key({ x, y }))) return true;
    }
  }
  return false;
}

function usableVehicleSprites(): VehicleSprite[] {
  return candidateVehicleSprites().filter((sprite) => spriteHasVisiblePixels(sprite));
}

function spriteHasVisiblePixels(sprite: VehicleSprite): boolean {
  const image = images.get(vehicleAssetPath(sprite.sheet));
  if (!image) return false;
  const probe = document.createElement('canvas');
  probe.width = image.width;
  probe.height = image.height;
  const context = probe.getContext('2d', { willReadFrequently: true });
  if (!context) return false;
  context.drawImage(image, 0, 0);
  for (let frame = 0; frame < 8; frame += 1) {
    const rect = vehicleFrameRect(sprite, frame);
    if (rect.x + rect.width > image.width || rect.y + rect.height > image.height) continue;
    const data = context.getImageData(rect.x, rect.y, rect.width, rect.height).data;
    for (let i = 3; i < data.length; i += 4) {
      if (data[i] !== 0) return true;
    }
  }
  return false;
}

function vehicleAssetPath(sheet: VehicleSheetName): string {
  return sheet === 'bus' ? assetPaths.bus : assetPaths.lorry;
}

function carPosition(car: Car): Coord {
  const base = Math.floor(car.offset);
  const next = (base + 1) % car.path.length;
  const t = car.offset - base;
  return {
    x: lerp(car.path[base].x, car.path[next].x, t),
    y: lerp(car.path[base].y, car.path[next].y, t),
  };
}

function pedestrianPosition(pedestrian: Pedestrian): Coord {
  const base = Math.floor(pedestrian.offset);
  const next = (base + 1) % pedestrian.path.length;
  const t = pedestrian.offset - base;
  return {
    x: lerp(pedestrian.path[base].x, pedestrian.path[next].x, t),
    y: lerp(pedestrian.path[base].y, pedestrian.path[next].y, t),
  };
}

function trainPosition(train: Train): Coord {
  return movingTrainPosition(train.path, train.offset);
}

function route(points: Coord[]): Coord[] {
  const result: Coord[] = [];
  for (let i = 1; i < points.length; i += 1) {
    const segment = linePath(points[i - 1], points[i]);
    result.push(...(i === 1 ? segment : segment.slice(1)));
  }
  return result;
}

function roadRoute(points: Coord[]): Coord[] {
  const result: Coord[] = [];
  for (let i = 1; i < points.length; i += 1) {
    const segment = cardinalLinePath(points[i - 1], points[i]);
    result.push(...(i === 1 ? segment : segment.slice(1)));
  }
  return result;
}

function railRoute(points: Coord[]): Coord[] {
  return roadRoute(points);
}

function cardinalLinePath(from: Coord, to: Coord): Coord[] {
  const result: Coord[] = [];
  let x = from.x;
  let y = from.y;
  result.push({ x, y });
  const xFirst = Math.abs(to.x - from.x) >= Math.abs(to.y - from.y);
  const stepX = () => {
    while (x !== to.x) {
      x += Math.sign(to.x - x);
      result.push({ x, y });
    }
  };
  const stepY = () => {
    while (y !== to.y) {
      y += Math.sign(to.y - y);
      result.push({ x, y });
    }
  };
  if (xFirst) {
    stepX();
    stepY();
  } else {
    stepY();
    stepX();
  }
  return result.filter(inside);
}

function linePath(from: Coord, to: Coord): Coord[] {
  const result: Coord[] = [];
  let x = from.x;
  let y = from.y;
  const dx = Math.abs(to.x - from.x);
  const dy = Math.abs(to.y - from.y);
  const sx = Math.sign(to.x - from.x);
  const sy = Math.sign(to.y - from.y);
  let err = dx - dy;
  result.push({ x, y });
  while (x !== to.x || y !== to.y) {
    const twiceErr = err * 2;
    if (twiceErr > -dy && x !== to.x) {
      err -= dy;
      x += sx;
      result.push({ x, y });
    }
    if (twiceErr < dx && y !== to.y) {
      err += dx;
      y += sy;
      result.push({ x, y });
    }
  }
  return result.filter(inside);
}

function isBuildable(coord: Coord): boolean {
  const kind = terrain.get(key(coord));
  return kind === 'grass' || kind === 'park';
}

function roadSpriteFrame(mask: number): number {
  const normalized = mask & (NORTH | EAST | SOUTH | WEST);
  if (normalized === NORTH || normalized === SOUTH) return 1;
  if (normalized === EAST || normalized === WEST) return 0;
  return roadFrameByMask.get(normalized) ?? 2;
}

function roadMask(points: Map<string, RoadKind>, coord: Coord): number {
  return (
    (points.has(key({ x: coord.x, y: coord.y - 1 })) ? NORTH : 0) |
    (points.has(key({ x: coord.x + 1, y: coord.y })) ? EAST : 0) |
    (points.has(key({ x: coord.x, y: coord.y + 1 })) ? SOUTH : 0) |
    (points.has(key({ x: coord.x - 1, y: coord.y })) ? WEST : 0)
  );
}

function isStraightEastWest(mask: number): boolean {
  return (mask & (EAST | WEST)) === (EAST | WEST) && (mask & (NORTH | SOUTH)) === 0;
}

function isStraightNorthSouth(mask: number): boolean {
  return (mask & (NORTH | SOUTH)) === (NORTH | SOUTH) && (mask & (EAST | WEST)) === 0;
}

function iso(coord: Coord): Coord {
  return {
    x: (coord.x - coord.y) * (TILE_W / 2),
    y: (coord.x + coord.y) * (TILE_H / 2),
  };
}

function worldToGrid(point: Coord): Coord {
  const projectedX = point.x / (TILE_W / 2);
  const projectedY = point.y / (TILE_H / 2);
  return {
    x: (projectedY + projectedX) / 2,
    y: (projectedY - projectedX) / 2,
  };
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

function distanceOutsidePlayableMap(coord: Coord): number {
  return Math.max(0, -coord.x, coord.x - (WIDTH - 1), -coord.y, coord.y - (HEIGHT - 1));
}

function drawIsoTile(point: Coord): void {
  ctx.beginPath();
  ctx.moveTo(point.x, point.y - TILE_H / 2);
  ctx.lineTo(point.x + TILE_W / 2, point.y);
  ctx.lineTo(point.x, point.y + TILE_H / 2);
  ctx.lineTo(point.x - TILE_W / 2, point.y);
  ctx.closePath();
  ctx.fill();
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

function inside(coord: Coord): boolean {
  return coord.x >= 0 && coord.y >= 0 && coord.x < WIDTH && coord.y < HEIGHT;
}

function distance(a: Coord, b: Coord): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

function nearestDistrict(coord: Coord): DistrictSeed {
  return districtSeeds.reduce((best, district) =>
    distance(coord, district.center) < distance(coord, best.center) ? district : best,
  );
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
    const terrainKind = terrain.get(tileKey);
    const sheet = sheetByName.get(building.sheet);
    if (roads.has(tileKey) || rails.has(tileKey) || !(terrainKind === 'grass' || terrainKind === 'park')) invalidBuildings += 1;
    if (!streetFrontages.has(tileKey)) buildingsOutsideStreetFrontageSet += 1;
    if (!hasDirectStreetAdjacency(building.coord, roads)) buildingsWithoutAnyStreetAdjacency += 1;
    if (!hasVisibleStreetFrontage(building.coord, roads)) buildingsWithoutStreetFrontage += 1;
    if (touchesRail(building.coord)) buildingsTouchingRail += 1;
    if (sheet && Math.floor(building.frame / sheet.cols) > 0) buildingFramesOutsideFinishedRow += 1;
  }

  const buildingTiles = new Set(buildings.map((building) => key(building.coord)));
  const treeTiles = new Set(trees.map(key));
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

function parseKey(value: string): Coord {
  const [x, y] = value.split(':').map(Number);
  return { x, y };
}

function hash(value: string): number {
  let result = 2166136261;
  for (let i = 0; i < value.length; i += 1) {
    result ^= value.charCodeAt(i);
    result = Math.imul(result, 16777619);
  }
  return result >>> 0;
}

function usableRows(sheet: BuildingSheet): number {
  return Math.min(1, sheet.rows);
}

function spriteSheetCellSize(totalPixels: number, frameCount: number): number {
  if (totalPixels % frameCount === 0) return totalPixels / frameCount;
  if ((totalPixels - 1) % frameCount === 0) return (totalPixels - 1) / frameCount;
  return Math.floor(totalPixels / frameCount);
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
  const legacyDiagnostics = cityDiagnostics();
  const detailCounts = detailCountsByCategory();
  return JSON.stringify({
    coordinateSystem: 'grid origin north-west, x east, y south, isometric projection',
    city: {
      worldId: zurichWorld.id,
      width: WIDTH,
      height: HEIGHT,
      roadTiles: roads.size,
      railTiles: rails.size,
      bridges: [...roads.values()].filter((road) => road.kind === 'bridge').length,
      buildings: buildings.length,
      trees: trees.length,
      cars: cars.length,
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
      pedestrians: pedestrians.length,
      pedestrianCorridors: pedestrianCorridorCount,
      pedestrianSprites: pedestrianSprites.length,
      pedestrianSpriteSheets: [...new Set(pedestrianSprites.map((sprite) => sprite.sheet))],
      vehicleSprites: vehicleSprites.length,
      vehicleSheets: [...new Set(vehicleSprites.map((sprite) => sprite.sheet))],
      mobility: mobilityDiagnostics(mobilityState),
      railStations: railStations.length,
      railYardTracks: Math.max(0, railPaths.length - 2),
      details: detailCounts,
      reserveTiles: zurichPlacement.reserveTiles.size,
      validationErrors: zurichValidation.errors.length,
      roadRailOverlap: zurichValidation.stats.roadRailOverlap,
      railCrossings: zurichValidation.stats.railCrossings,
      invalidBuildings: zurichValidation.stats.invalidBuildings,
      treeBuildingOverlap: zurichValidation.stats.treeBuildingOverlap,
      railStationsOnRoad: legacyDiagnostics.railStationsOnRoad,
      railStationsOnBuildings: legacyDiagnostics.railStationsOnBuildings,
      railStationsOnRails: legacyDiagnostics.railStationsOnRails,
      railStationsOnTrees: legacyDiagnostics.railStationsOnTrees,
      legacyDiagnostics,
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

window.advanceTime = (ms: number) => {
  for (const car of cars) car.offset = (car.offset + car.speed * (ms / 1000)) % car.path.length;
  for (const pedestrian of pedestrians) pedestrian.offset = (pedestrian.offset + pedestrian.speed * (ms / 1000)) % pedestrian.path.length;
  for (const train of trains) train.offset = trainWrappedOffset(train.offset + train.speed * (ms / 1000), train.path);
  render();
};

function configuredMobilityBackend(): { enabled: boolean; baseUrl?: string } {
  const params = new URLSearchParams(window.location.search);
  const explicitBaseUrl = params.get('mobilityBackend');
  if (explicitBaseUrl) return { enabled: true, baseUrl: explicitBaseUrl };
  if (params.get('mobility') === '1') return { enabled: true };
  try {
    if (window.localStorage.getItem('abutown:mobility') === '1') return { enabled: true };
  } catch {
    return { enabled: false };
  }
  return { enabled: false };
}
