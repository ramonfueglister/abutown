import type { CameraState } from '../cameraController';
import type { TerrainKind, WorldDetail } from '../city/worldTypes';
import { formatSimDate } from '../backend/simTime';
import type {
  RuntimeBuilding,
  RuntimeRailStation,
  RuntimeRailTile,
  RuntimeRoadTile,
  RuntimeTerrain,
} from './worldRuntimeTypes';
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
  EAST,
  NORTH,
  SOUTH,
  WEST,
  coordKey as key,
  maskSegments as gridMaskSegments,
  outwardExits as gridOutwardExits,
  stableHash as hash,
} from './gridMath';
import {
  carRenderStyle,
  carVisualWorldPoint,
  pedestrianRenderStyle,
} from './entityRenderStyle';

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
  | { type: 'detail'; coord: Coord; detail: WorldDetail }
  | { type: 'tree'; coord: Coord }
  | { type: 'building'; coord: Coord; building: RuntimeBuilding };

type CarDrawable = { type: 'car'; coord: Coord; car: BackendCar; vehicleId: string };
type PedestrianDrawable = { type: 'pedestrian'; coord: Coord; pedestrian: BackendPedestrian; agentId: string };
type Drawable = StaticDrawable | CarDrawable | PedestrianDrawable;
type TileFillStyle = { color: string; alpha: number };
type TileFillBatch = TileFillStyle & { coords: Coord[] };

export const MAP_BACKGROUND = '#182018';
const MAP_GRASS = '#91c86f';
const MAP_WATER = '#92d8e9';
const MAP_RIVERBANK = '#bde8df';
const MAP_PARK = '#cfe5bf';
const MAP_PLAZA = '#eadbbd';
const ROAD_SIDEWALK = '#d8d3c5';
const ROAD_CURB = '#aaa69c';
const ROAD_CASING = '#565d61';
const ROAD_CORE = '#71797d';
const ROAD_CENTER_LINE = '#f1c93a';
const RAIL_CASING = 'rgba(122, 131, 135, 0.32)';
const RAIL_CORE = 'rgba(122, 131, 135, 0.42)';
const TREE_COLOR = '#84ad78';
const DETAIL_COLOR = 'rgba(92, 97, 92, 0.34)';
const BUILDING_RESIDENTIAL = '#c9a16e';
const BUILDING_RESIDENTIAL_ROOF = '#8b5c3c';
const BUILDING_COMMERCIAL = '#c9d8dc';
const BUILDING_CIVIC = '#dccb9a';
const BUILDING_INDUSTRIAL = '#cabed6';
const AGENT_COLOR = '#343b43';
const VEHICLE_COLORS = ['#e85d75', '#3f8fc7', '#49a879', '#e5a944', '#8c73c8', '#ef7f5a', '#28a6b0'];
const VIEWPORT_GRID_PADDING = 9;
const TERRAIN_TILE_OVERLAP = 0.6;
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
  drawGrassBaseLayer(state);
  drawTerrainOverlayLayer(state, visibleTerrainTiles);
  drawRiverSurfaceLayer(state, visibleTerrainTiles);

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
  drawRoads(state, [...state.roads.values()].filter((road) => isCoordVisible(road.coord, visibleGrid)));
  for (const path of state.railPaths) drawRailPath(state, path);
  drawEdgeConnections(state, visibleGrid);
  for (const station of state.railStations) if (isCoordVisible(station.coord, visibleGrid)) drawRailStation(state, station);
  for (const detail of state.details) if (isCoordVisible(detail.coord, visibleGrid)) drawDetail(state, detail);
  for (const building of state.buildings) if (isCoordVisible(building.coord, visibleGrid)) drawBuilding(state, building);
  for (const coord of state.trees) if (isCoordVisible(coord, visibleGrid)) drawTree(state, coord);
  for (const item of carDrawables) drawCar(state, item.car, item.vehicleId === state.selectedVehicleId);
  for (const item of pedestrianDrawables) drawPedestrian(state, item.pedestrian, item.agentId === state.selectedAgentId);

  ctx.restore();
}

function drawGrassBaseLayer(state: MinimalMapRendererState): void {
  const { ctx, tileSize, world } = state;
  ctx.save();
  ctx.fillStyle = MAP_GRASS;
  ctx.fillRect(
    -TERRAIN_TILE_OVERLAP,
    -TERRAIN_TILE_OVERLAP,
    world.width * tileSize.width + TERRAIN_TILE_OVERLAP * 2,
    world.height * tileSize.height + TERRAIN_TILE_OVERLAP * 2,
  );
  ctx.restore();
}

function drawTerrainOverlayLayer(state: MinimalMapRendererState, coords: readonly Coord[]): void {
  const batches = new Map<string, TileFillBatch>();
  for (const coord of coords) {
    const style = terrainOverlayStyle(state, coord);
    if (style) appendTileFillBatch(batches, style, coord);
  }
  drawTileFillBatches(state, batches);
}

function terrainOverlayStyle(state: MinimalMapRendererState, coord: Coord): TileFillStyle | null {
  const kind = state.terrainKinds.get(key(coord))?.kind;
  if (kind === 'park' || kind === 'forest' || kind === 'reserve') return { color: MAP_PARK, alpha: 0.82 };
  if (kind === 'plaza') return { color: MAP_PLAZA, alpha: 0.72 };
  return null;
}

function drawRiverSurfaceLayer(state: MinimalMapRendererState, coords: readonly Coord[]): void {
  const batches = new Map<string, TileFillBatch>();
  for (const coord of coords) {
    const style = riverSurfaceStyle(state, coord);
    if (style) appendTileFillBatch(batches, style, coord);
  }
  drawTileFillBatches(state, batches);
}

function riverSurfaceStyle(state: MinimalMapRendererState, coord: Coord): TileFillStyle | null {
  const terrain = state.terrain.get(key(coord));
  if (terrain === 'riverbank') return { color: MAP_RIVERBANK, alpha: 0.96 };
  if (terrain === 'water') return { color: MAP_WATER, alpha: 0.96 };
  return null;
}

function drawRoad(state: MinimalMapRendererState, road: RuntimeRoadTile): void {
  drawRoadBand(state, road.coord, road.mask, ROAD_SIDEWALK, screenStableWorldSize(24, state.camera.scale, { minWorld: 24, maxWorld: 36 }));
  drawRoadBand(state, road.coord, road.mask, ROAD_CURB, screenStableWorldSize(18, state.camera.scale, { minWorld: 18, maxWorld: 29 }));
  drawRoadBand(state, road.coord, road.mask, ROAD_CASING, screenStableWorldSize(16, state.camera.scale, { minWorld: 16, maxWorld: 26 }));
  drawRoadBand(state, road.coord, road.mask, ROAD_CORE, screenStableWorldSize(13, state.camera.scale, { minWorld: 13, maxWorld: 22 }));
  drawRoadBand(state, road.coord, road.mask, ROAD_CENTER_LINE, screenStableWorldSize(2.4, state.camera.scale, { minWorld: 2, maxWorld: 4.2 }));
}

function drawRoads(state: MinimalMapRendererState, roads: RuntimeRoadTile[]): void {
  if (roads.length === 0) return;
  const bands = [
    { color: ROAD_SIDEWALK, width: screenStableWorldSize(24, state.camera.scale, { minWorld: 24, maxWorld: 36 }) },
    { color: ROAD_CURB, width: screenStableWorldSize(18, state.camera.scale, { minWorld: 18, maxWorld: 29 }) },
    { color: ROAD_CASING, width: screenStableWorldSize(16, state.camera.scale, { minWorld: 16, maxWorld: 26 }) },
    { color: ROAD_CORE, width: screenStableWorldSize(13, state.camera.scale, { minWorld: 13, maxWorld: 22 }) },
    { color: ROAD_CENTER_LINE, width: screenStableWorldSize(2.4, state.camera.scale, { minWorld: 2, maxWorld: 4.2 }) },
  ];
  for (const band of bands) drawRoadRuns(state, roads, band.color, band.width);
}

function drawRoadRuns(state: MinimalMapRendererState, roads: RuntimeRoadTile[], color: string, width: number): void {
  const horizontal = new Map<number, number[]>();
  const vertical = new Map<number, number[]>();
  for (const road of roads) {
    if ((road.mask & (EAST | WEST)) !== 0) appendGrouped(horizontal, road.coord.y, road.coord.x);
    if ((road.mask & (NORTH | SOUTH)) !== 0) appendGrouped(vertical, road.coord.x, road.coord.y);
  }

  const { ctx, tileSize } = state;
  ctx.save();
  ctx.fillStyle = color;
  for (const [y, xs] of horizontal) {
    for (const run of mergedRuns(xs)) {
      ctx.fillRect(
        run.min * tileSize.width,
        y * tileSize.height + tileSize.height / 2 - width / 2,
        (run.max - run.min + 1) * tileSize.width,
        width,
      );
    }
  }
  for (const [x, ys] of vertical) {
    for (const run of mergedRuns(ys)) {
      ctx.fillRect(
        x * tileSize.width + tileSize.width / 2 - width / 2,
        run.min * tileSize.height,
        width,
        (run.max - run.min + 1) * tileSize.height,
      );
    }
  }
  ctx.restore();
}

function drawRoadBand(state: MinimalMapRendererState, coord: Coord, mask: number, color: string, width: number): void {
  const { ctx, tileSize } = state;
  const point = iso(state, coord);
  const horizontal = (mask & (EAST | WEST)) !== 0;
  const vertical = (mask & (NORTH | SOUTH)) !== 0;
  const overlap = 0.8;
  ctx.save();
  ctx.fillStyle = color;
  if (horizontal) {
    ctx.fillRect(point.x - tileSize.width / 2 - overlap, point.y - width / 2, tileSize.width + overlap * 2, width);
  }
  if (vertical) {
    ctx.fillRect(point.x - width / 2, point.y - tileSize.height / 2 - overlap, width, tileSize.height + overlap * 2);
  }
  if (!horizontal && !vertical) {
    ctx.fillRect(point.x - width / 2, point.y - width / 2, width, width);
  }
  ctx.restore();
}

function appendGrouped(groups: Map<number, number[]>, key: number, value: number): void {
  const values = groups.get(key);
  if (values) values.push(value);
  else groups.set(key, [value]);
}

function mergedRuns(values: number[]): { min: number; max: number }[] {
  const sorted = [...new Set(values)].sort((a, b) => a - b);
  const runs: { min: number; max: number }[] = [];
  for (const value of sorted) {
    const last = runs[runs.length - 1];
    if (last && value <= last.max + 1) last.max = value;
    else runs.push({ min: value, max: value });
  }
  return runs;
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

function drawDetail(state: MinimalMapRendererState, detail: WorldDetail): void {
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
  ctx.fillStyle = buildingVectorColor(building);
  roundedRectPath(ctx, x, y, width, height, 1.4);
  ctx.fill();
  if (building.sheet === 'oldhouses' || building.sheet === 'houses') {
    ctx.fillStyle = BUILDING_RESIDENTIAL_ROOF;
    roundedRectPath(ctx, x + 1.6, y + 1.5, width - 3.2, height * 0.44, 1.2);
    ctx.fill();
    ctx.fillStyle = 'rgba(255, 246, 214, 0.76)';
    ctx.fillRect(x + width * 0.68, y + height * 0.62, width * 0.16, height * 0.22);
  }
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

function appendTileFillBatch(batches: Map<string, TileFillBatch>, style: TileFillStyle, coord: Coord): void {
  const batchKey = `${style.color}:${style.alpha}`;
  const batch = batches.get(batchKey);
  if (batch) {
    batch.coords.push(coord);
    return;
  }
  batches.set(batchKey, { ...style, coords: [coord] });
}

function drawTileFillBatches(state: MinimalMapRendererState, batches: ReadonlyMap<string, TileFillBatch>): void {
  for (const batch of batches.values()) drawTileFillBatch(state, batch);
}

function drawTileFillBatch(state: MinimalMapRendererState, batch: TileFillBatch): void {
  const { ctx, tileSize } = state;
  ctx.save();
  ctx.globalAlpha *= batch.alpha;
  ctx.fillStyle = batch.color;
  ctx.beginPath();
  for (const coord of batch.coords) {
    const point = iso(state, coord);
    ctx.rect(
      point.x - tileSize.width / 2 - TERRAIN_TILE_OVERLAP,
      point.y - tileSize.height / 2 - TERRAIN_TILE_OVERLAP,
      tileSize.width + TERRAIN_TILE_OVERLAP * 2,
      tileSize.height + TERRAIN_TILE_OVERLAP * 2,
    );
  }
  ctx.fill();
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
  const segments = gridMaskSegments(mask, state.tileSize);
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
  ctx.fillStyle = AGENT_COLOR;
  ctx.globalAlpha *= 0.78;
  ctx.beginPath();
  ctx.arc(0, 0, style.radius, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

function drawAgentInspectorPanel(state: MinimalMapRendererState, inspector: EntityInspector): void {
  drawInspectorPanel(state.ctx, inspector, AGENT_INSPECTOR_PANEL, state.viewport.devicePixelRatio);
}

function drawCarInspectorPanel(state: MinimalMapRendererState, inspector: EntityInspector): void {
  drawInspectorPanel(state.ctx, inspector, VEHICLE_INSPECTOR_PANEL, state.viewport.devicePixelRatio);
}

function drawEdgeConnections(state: MinimalMapRendererState, visibleGrid: GridRect): void {
  for (const road of state.roads.values()) {
    for (const exit of gridOutwardExits(road.coord, road.mask, state.world)) {
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
    for (const exit of gridOutwardExits(rail.coord, rail.mask, state.world)) {
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

function isWaterSurface(state: MinimalMapRendererState, coord: Coord): boolean {
  const kind = state.terrain.get(key(coord));
  return kind === 'water' || kind === 'riverbank';
}

function drawFadingEdgeTile(state: MinimalMapRendererState, step: number, draw: () => void): void {
  const { ctx } = state;
  ctx.save();
  ctx.globalAlpha = 0.68 * (1 - step / (EDGE_EXIT_TILES + 1));
  draw();
  ctx.restore();
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
