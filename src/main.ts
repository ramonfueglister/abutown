import './style.css';
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
import {
  createCameraState,
  dampCamera,
  panCameraTarget,
  screenToWorld as cameraScreenToWorld,
  zoomCameraAt,
} from './cameraController';
import { shouldRenderDetail } from './render/detailRenderPolicy';
import {
  candidateVehicleSprites,
  type VehicleSprite,
} from './render/vehicleSprites';
import {
  candidateSimutransPedestrianSprites,
  type SimutransPedestrianSprite,
} from './render/simutransPedestrianSprites';
import {
  RIVERBANK_EAST,
  RIVERBANK_NORTH,
  RIVERBANK_SOUTH,
  RIVERBANK_WEST,
  riverSurfaceSourceFromMask,
} from './render/riverbankFrames';
import {
  carsFromMobilityState,
  pedestriansFromMobilityState,
  type BackendCar,
  type BackendPedestrian,
} from './render/backendMobilityDrawables';
import { MINIMAL_MAP_TILE_SIZE, mapProject, mapUnproject } from './render/minimalMapProjection';
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
} from './render/entityInspector';
import { selectMobilityEntityAtWorldPoint } from './render/mobilityEntitySelection';
import {
  chooseInitialCameraFocus,
  constrainCameraToMap,
  initializeCameraForGridFocus,
  isCoordVisibleInGridRect,
  visibleGridRectForCamera,
  type GridRect,
} from './render/cameraViewport';
import { createCityDiagnostics } from './render/cityDiagnostics';
import { buildRenderGameText } from './render/renderGameText';
import {
  buildVisibleCarDrawables,
  buildVisiblePedestrianDrawables,
  buildVisibleTrainDrawables,
  visibleTerrainCoords,
} from './render/sceneDrawables';
import {
  EAST,
  NORTH,
  SOUTH,
  WEST,
  coordKey as key,
  maskSegments as gridMaskSegments,
  outwardExits,
} from './render/gridMath';
import {
  buildingJitter,
  buildingVectorColor,
  treeRenderStyle,
  vehicleVectorColor,
} from './render/vectorStyle';
import {
  outskirtsTileStyle,
  riverSurfaceFill,
  terrainBaseFill,
} from './render/terrainStyle';
import {
  RAIL_CASING,
  RAIL_CORE,
  TRAIN_CORE,
  railLineStyle,
  roadLineStyle,
} from './render/transportStyle';
import {
  AGENT_INSPECTOR_PANEL,
  VEHICLE_INSPECTOR_PANEL,
  drawInspectorPanel,
} from './render/inspectorPanelPainter';
import {
  carRenderStyle,
  pedestrianRenderStyle,
} from './render/entityRenderStyle';
import { trainRenderSegments } from './render/trainRenderStyle';

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
const VIEWPORT_GRID_PADDING = 9;
const OUTSKIRTS_TILES = 12;
const EDGE_EXIT_TILES = 7;
const TRAIN_FADE_TILES = 12;
const TRAIN_SPEED = 8.5;
const MAP_BACKGROUND = '#f6f0e3';
const TREE_COLOR = '#84ad78';
const DETAIL_COLOR = 'rgba(92, 97, 92, 0.34)';
const AGENT_COLOR = '#343b43';

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
        getScreenToTile: () => (screen) => worldToGrid(cameraScreenToWorld(camera, screen)),
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
    initializeCameraForGridFocus(
      camera,
      initialCameraFocusCoord(),
      { width: window.innerWidth, height: window.innerHeight },
      iso,
      { verticalAnchor: 0.52 },
    );
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
  const pixelRatio = window.devicePixelRatio || 1;
  drawInspectorPanel(ctx, buildBackendPedestrianInspector(selectedBackendPedestrian()), AGENT_INSPECTOR_PANEL, pixelRatio);
  drawInspectorPanel(ctx, buildBackendCarInspector(selectedBackendCar()), VEHICLE_INSPECTOR_PANEL, pixelRatio);
}

function drawScene(offset: Coord): void {
  ctx.save();
  const sceneOffset = iso(offset);
  ctx.translate(sceneOffset.x, sceneOffset.y);
  const visibleGrid = visibleGridRect();

  drawOutskirtsTerrain(visibleGrid);
  const visibleTerrainTiles = visibleTerrainCoords(visibleGrid, { width: WIDTH, height: HEIGHT }, iso);
  for (const coord of visibleTerrainTiles) drawTerrainBase(coord);
  for (const coord of visibleTerrainTiles) drawRiverSurface(coord);

  const pedestrians: BackendPedestrian[] = pedestriansFromMobilityState(mobilityState, pedestrianSprites, Date.now(), mobilityTickPeriodMs);
  const cars: BackendCar[] = carsFromMobilityState(mobilityState, vehicleSprites, Date.now(), mobilityTickPeriodMs);
  const carDrawables = buildVisibleCarDrawables(cars, visibleGrid, iso);
  const pedestrianDrawables = buildVisiblePedestrianDrawables(pedestrians, visibleGrid, iso);
  const trainDrawables = buildVisibleTrainDrawables(trains, trainPosition, visibleGrid, iso);

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

function drawTerrainBase(coord: Coord): void {
  const fill = terrainBaseFill(terrainBaseAt(coord));
  if (fill) drawTileFill(coord, fill.color, fill.alpha);
}

function drawRiverSurface(coord: Coord): void {
  if (!isWaterSurface(coord)) return;
  const fill = riverSurfaceFill(terrainBaseAt(coord));
  drawTileFill(coord, fill.color, fill.alpha);
}

function drawOutskirtsTerrain(visibleGrid: GridRect): void {
  for (let y = Math.max(-OUTSKIRTS_TILES, visibleGrid.minY); y <= Math.min(HEIGHT - 1 + OUTSKIRTS_TILES, visibleGrid.maxY); y += 1) {
    for (let x = Math.max(-OUTSKIRTS_TILES, visibleGrid.minX); x <= Math.min(WIDTH - 1 + OUTSKIRTS_TILES, visibleGrid.maxX); x += 1) {
      const coord = { x, y };
      const style = outskirtsTileStyle(coord, { width: WIDTH, height: HEIGHT }, OUTSKIRTS_TILES);
      if (!style) continue;

      ctx.save();
      drawTileFill(coord, style.fill.color, style.fill.alpha);
      if (style.shadowAlpha !== null) {
        const point = iso(coord);
        ctx.fillStyle = `rgba(151, 133, 103, ${style.shadowAlpha})`;
        drawIsoTile(point);
      }
      ctx.restore();
    }
  }
}

function drawRoad(road: RoadTile): void {
  drawMaskLine(road.coord, road.mask, roadLineStyle(road.kind, camera.scale));
}

function drawRail(_rail: RailTile): void {
  drawMaskLine(_rail.coord, _rail.mask, railLineStyle(camera.scale));
}

function drawRailPath(path: Coord[]): void {
  if (path.length < 2) return;
  ctx.save();
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  const style = railLineStyle(camera.scale);
  drawPolyline(path, style.casing, style.casingWidth);
  drawPolyline(path, style.core, style.coreWidth);
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

function drawTree(coord: Coord): void {
  const style = treeRenderStyle({ coord, cameraScale: camera.scale, terrainBase: terrainBaseAt(coord) });
  if (!style.visible) return;
  const point = iso(coord);
  ctx.save();
  ctx.fillStyle = TREE_COLOR;
  ctx.globalAlpha = style.alpha;
  ctx.beginPath();
  ctx.arc(point.x + style.jitter.x, point.y + style.jitter.y, 2.4, 0, Math.PI * 2);
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
  const segments = gridMaskSegments(mask, tileSize);
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

function drawCar(car: BackendCar, selected: boolean): void {
  const current = car.path[0];
  const next = car.path[1] ?? current;
  const pos = current;
  const point = iso(pos);
  const currentPoint = iso(current);
  const nextPoint = iso(next);
  const style = carRenderStyle(currentPoint, nextPoint, camera.scale);
  ctx.save();
  ctx.translate(point.x + style.lane.x, point.y + style.lane.y);
  if (selected) {
    ctx.globalAlpha = 0.94;
    ctx.strokeStyle = '#166c83';
    ctx.lineWidth = 2 / Math.max(0.75, camera.scale);
    ctx.beginPath();
    ctx.ellipse(0, 0, style.selection.x, style.selection.y, 0, 0, Math.PI * 2);
    ctx.stroke();
  }
  drawCapsule({ x: 0, y: 0 }, style.angle, style.capsule.length, style.capsule.width, vehicleVectorColor(car.id));
  ctx.restore();
}

function drawTrain(train: Train): void {
  for (const segment of trainRenderSegments(train, { height: HEIGHT, project: iso })) {
    ctx.save();
    ctx.globalAlpha *= segment.alpha;
    drawCapsule(segment.point, segment.angle, segment.length, segment.width, TRAIN_CORE, RAIL_CASING);
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

function drawPedestrian(pedestrian: BackendPedestrian, selected: boolean): void {
  const current = pedestrian.path[0];
  const next = pedestrian.path[1] ?? current;
  const pos = current;
  const point = iso(pos);
  const currentPoint = iso(current);
  const nextPoint = iso(next);
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
  ctx.fillStyle = AGENT_COLOR;
  ctx.globalAlpha *= 0.78;
  ctx.beginPath();
  ctx.arc(0, 0, style.radius, 0, Math.PI * 2);
  ctx.fill();
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
    for (const exit of outwardExits(road.coord, road.mask, { width: WIDTH, height: HEIGHT })) {
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
    for (const exit of outwardExits(rail.coord, rail.mask, { width: WIDTH, height: HEIGHT })) {
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
  const coords = [...pedestrians.map((agent) => agent.path[0]), ...cars.map((car) => car.path[0])];
  return chooseInitialCameraFocus(coords, { width: WIDTH, height: HEIGHT });
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
  const worldPoint = cameraScreenToWorld(camera, point);
  const pedestrians = pedestriansFromMobilityState(mobilityState, pedestrianSprites, Date.now(), mobilityTickPeriodMs);
  const cars = carsFromMobilityState(mobilityState, vehicleSprites, Date.now(), mobilityTickPeriodMs);
  const selection = selectMobilityEntityAtWorldPoint({
    pedestrians,
    cars,
    worldPoint,
    cameraScale: camera.scale,
    gridToWorld: iso,
  });
  selectedAgentId = selection.selectedAgentId;
  selectedVehicleId = selection.selectedVehicleId;
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
  constrainCameraToMap(
    camera,
    { width: window.innerWidth, height: window.innerHeight },
    worldToGrid,
    iso,
    { width: WIDTH, height: HEIGHT },
    { edgeMargin: CAMERA_EDGE_MARGIN, edgeSoftness: CAMERA_EDGE_SOFTNESS, allowOverscroll },
  );
}

function visibleGridRect(): GridRect {
  return visibleGridRectForCamera(
    camera,
    { width: window.innerWidth, height: window.innerHeight },
    worldToGrid,
    VIEWPORT_GRID_PADDING,
  );
}

function isCoordVisible(coord: Coord, rect: GridRect): boolean {
  return isCoordVisibleInGridRect(coord, rect);
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

function drawIsoTile(point: Coord): void {
  ctx.beginPath();
  ctx.rect(point.x - TILE_W / 2, point.y - TILE_H / 2, TILE_W, TILE_H);
  ctx.fill();
}

function drawFadingEdgeTile(step: number, draw: () => void): void {
  ctx.save();
  ctx.globalAlpha = 0.68 * (1 - step / (EDGE_EXIT_TILES + 1));
  draw();
  ctx.restore();
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

declare global {
  interface Window {
    render_game_to_text?: () => string;
    advanceTime?: (ms: number) => void;
  }
}

window.render_game_to_text = () => {
  const backendMobility = mobilityDiagnostics(mobilityState);
  const projectedPedestrians = pedestriansFromMobilityState(mobilityState, pedestrianSprites, Date.now(), mobilityTickPeriodMs);
  const projectedCars = carsFromMobilityState(mobilityState, vehicleSprites, Date.now(), mobilityTickPeriodMs);
  const diagnostics = createCityDiagnostics({
    width: WIDTH,
    height: HEIGHT,
    terrainAt,
    roads,
    rails,
    railCrossings,
    railReserved,
    railStations,
    buildings,
    trees,
  });
  const train = trains[0] ?? null;
  const trainPositionSnapshot = train ? trainPosition(train) : null;
  const entityScreenPosition = (coord: Coord): Coord => {
    const projected = iso(coord);
    return {
      x: camera.x + projected.x * camera.scale,
      y: camera.y + projected.y * camera.scale,
    };
  };
  return buildRenderGameText({
    worldId: TERRAIN_SEED_WORLD_ID,
    visualStyleId: VISUAL_STYLE_ID,
    tileSize,
    nonPak128AssetPaths: [],
    width: WIDTH,
    height: HEIGHT,
    terrainState,
    roads,
    rails,
    railCrossings,
    railPaths,
    railStations,
    buildings,
    trees,
    details,
    trains,
    projectedPedestrians,
    projectedCars,
    pedestrianSprites,
    vehicleSprites,
    selectedAgentId,
    selectedVehicleId,
    backendBaseUrl,
    backendStatus,
    backendMobility,
    diagnostics,
    camera: {
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
    entityScreenPosition,
    trainSummary: train && trainPositionSnapshot
      ? {
          position: trainPositionSnapshot,
          alpha: trainFadeAlpha(trainPositionSnapshot, { height: HEIGHT, fadeTiles: train.fadeTiles }),
          speed: train.speed,
          fadeTiles: train.fadeTiles,
          direction: 'northbound',
        }
      : null,
  });
};

window.advanceTime = (ms: number) => {
  for (const train of trains) train.offset = trainWrappedOffset(train.offset + train.speed * (ms / 1000), train.path);
  render();
};
