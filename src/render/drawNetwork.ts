// L0 network/terrain/buildings drawers for the schematic renderer.
// Spec: docs/superpowers/specs/2026-06-10-schematic-map-renderer-design.md §1
import type { WorldDetail } from '../city/worldTypes';
import type {
  RuntimeBuilding,
  RuntimeRailStation,
  RuntimeRailTile,
  RuntimeRoadTile,
} from './worldRuntimeTypes';
import { shouldRenderDetail } from './detailRenderPolicy';
import { minimalBuildingPlotOffset, minimalBuildingSize } from './minimalBuildingLayout';
import { screenStableWorldSize } from './minimalGlyphScale';
import { roundedRectPath } from './canvasPrimitives';
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
  EDGE_EXIT_TILES,
  TERRAIN_TILE_OVERLAP,
  isCoordVisible,
  iso,
  type Coord,
  type GridRect,
  type MinimalMapRendererState,
  type TileFillBatch,
  type TileFillStyle,
} from './minimalMapRenderer';
import {
  BUILDING_CIVIC,
  BUILDING_COMMERCIAL,
  BUILDING_INDUSTRIAL,
  BUILDING_RESIDENTIAL,
  DETAIL,
  GROUND,
  PARK,
  PLAZA,
  RAIL_CASING,
  RAIL_CORE,
  RIVERBANK,
  ROAD_CENTER_DASH,
  ROAD_INK,
  TREE,
  WATER,
} from './designTokens';

export function drawGrassBaseLayer(state: MinimalMapRendererState): void {
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

export function drawTerrainOverlayLayer(state: MinimalMapRendererState, coords: readonly Coord[]): void {
  const batches = new Map<string, TileFillBatch>();
  for (const coord of coords) {
    const style = terrainOverlayStyle(state, coord);
    if (style) appendTileFillBatch(batches, style, coord);
  }
  drawTileFillBatches(state, batches);
}

export function terrainOverlayStyle(state: MinimalMapRendererState, coord: Coord): TileFillStyle | null {
  const kind = state.terrainKinds.get(key(coord))?.kind;
  if (kind === 'park' || kind === 'forest' || kind === 'reserve') return { color: PARK, alpha: 0.82 };
  if (kind === 'plaza') return { color: PLAZA, alpha: 0.72 };
  return null;
}

export function drawRiverSurfaceLayer(state: MinimalMapRendererState, coords: readonly Coord[]): void {
  const batches = new Map<string, TileFillBatch>();
  for (const coord of coords) {
    const style = riverSurfaceStyle(state, coord);
    if (style) appendTileFillBatch(batches, style, coord);
  }
  drawTileFillBatches(state, batches);
}

export function riverSurfaceStyle(state: MinimalMapRendererState, coord: Coord): TileFillStyle | null {
  const terrain = state.terrain.get(key(coord));
  if (terrain === 'riverbank') return { color: RIVERBANK, alpha: 0.96 };
  if (terrain === 'water') return { color: WATER, alpha: 0.96 };
  return null;
}

export function drawRoad(state: MinimalMapRendererState, road: RuntimeRoadTile): void {
  drawRoadBand(state, road.coord, road.mask, ROAD_INK, screenStableWorldSize(14, state.camera.scale, { minWorld: 14, maxWorld: 24 }));
  drawRoadBand(state, road.coord, road.mask, ROAD_CENTER_DASH, screenStableWorldSize(2, state.camera.scale, { minWorld: 1.6, maxWorld: 3.4 }));
}

export function drawRoads(state: MinimalMapRendererState, roads: RuntimeRoadTile[]): void {
  if (roads.length === 0) return;
  const bands = [
    { color: ROAD_INK, width: screenStableWorldSize(14, state.camera.scale, { minWorld: 14, maxWorld: 24 }) },
    { color: ROAD_CENTER_DASH, width: screenStableWorldSize(2, state.camera.scale, { minWorld: 1.6, maxWorld: 3.4 }) },
  ];
  for (const band of bands) drawRoadRuns(state, roads, band.color, band.width);
}

export function drawRoadRuns(state: MinimalMapRendererState, roads: RuntimeRoadTile[], color: string, width: number): void {
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

export function drawRoadBand(state: MinimalMapRendererState, coord: Coord, mask: number, color: string, width: number): void {
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

export function appendGrouped(groups: Map<number, number[]>, key: number, value: number): void {
  const values = groups.get(key);
  if (values) values.push(value);
  else groups.set(key, [value]);
}

export function mergedRuns(values: number[]): { min: number; max: number }[] {
  const sorted = [...new Set(values)].sort((a, b) => a - b);
  const runs: { min: number; max: number }[] = [];
  for (const value of sorted) {
    const last = runs[runs.length - 1];
    if (last && value <= last.max + 1) last.max = value;
    else runs.push({ min: value, max: value });
  }
  return runs;
}

export function drawRail(state: MinimalMapRendererState, rail: RuntimeRailTile): void {
  drawMaskLine(state, rail.coord, rail.mask, {
    casing: RAIL_CASING,
    core: RAIL_CORE,
    casingWidth: screenStableWorldSize(2.8, state.camera.scale, { minWorld: 4.8, maxWorld: 9 }),
    coreWidth: screenStableWorldSize(1.2, state.camera.scale, { minWorld: 1.8, maxWorld: 4 }),
  });
}

export function drawRailPath(state: MinimalMapRendererState, path: readonly Coord[]): void {
  if (path.length < 2) return;
  const { ctx } = state;
  ctx.save();
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  drawPolyline(state, path, RAIL_CASING, screenStableWorldSize(2.8, state.camera.scale, { minWorld: 4.8, maxWorld: 9 }));
  drawPolyline(state, path, RAIL_CORE, screenStableWorldSize(1.2, state.camera.scale, { minWorld: 1.8, maxWorld: 4 }));
  ctx.restore();
}

export function drawPolyline(state: MinimalMapRendererState, path: readonly Coord[], color: string, width: number): void {
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

export function drawRailStation(state: MinimalMapRendererState, station: RuntimeRailStation): void {
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

export function drawDetail(state: MinimalMapRendererState, detail: WorldDetail): void {
  if (!shouldRenderDetail(detail)) return;
  if (detail.category !== 'industry' && detail.category !== 'dock' && detail.category !== 'station') return;
  const { ctx } = state;
  const point = iso(state, detail.coord);
  ctx.save();
  ctx.fillStyle = DETAIL;
  ctx.fillRect(point.x - 2, point.y - 2, 4, 4);
  ctx.restore();
}

export function drawBuilding(state: MinimalMapRendererState, building: RuntimeBuilding): void {
  const { ctx } = state;
  const point = iso(state, building.coord);
  const offset = minimalBuildingPlotOffset(building.coord, state.roads);
  const { width, height } = minimalBuildingSize(building);
  const jitter = buildingJitter(building);
  const x = point.x - width / 2 + offset.x + jitter.x;
  const y = point.y - height / 2 + offset.y + jitter.y;
  ctx.save();
  ctx.fillStyle = buildingVectorColor(building);
  roundedRectPath(ctx, x, y, width, height, 2.6);
  ctx.fill();
  ctx.restore();
}

export function buildingJitter(building: RuntimeBuilding): Coord {
  return {
    x: ((hash(`building-jitter-x:${building.district}:${key(building.coord)}`) % 5) - 2) * 0.26,
    y: ((hash(`building-jitter-y:${building.district}:${key(building.coord)}`) % 5) - 2) * 0.26,
  };
}

export function drawTree(state: MinimalMapRendererState, coord: Coord): void {
  if (state.camera.scale < 0.32 && hash(`tree-lod:${key(coord)}`) % 3 !== 0) return;
  const { ctx } = state;
  const point = iso(state, coord);
  const jitterX = ((hash(`tree-x:${key(coord)}`) % 9) - 4) * 0.38;
  const jitterY = ((hash(`tree-y:${key(coord)}`) % 9) - 4) * 0.38;
  ctx.save();
  ctx.fillStyle = TREE;
  ctx.globalAlpha = state.terrainKinds.get(key(coord))?.kind === 'forest' ? 0.72 : 0.54;
  ctx.beginPath();
  ctx.arc(point.x + jitterX, point.y + jitterY, 2.4, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

export function appendTileFillBatch(batches: Map<string, TileFillBatch>, style: TileFillStyle, coord: Coord): void {
  const batchKey = `${style.color}:${style.alpha}`;
  const batch = batches.get(batchKey);
  if (batch) {
    batch.coords.push(coord);
    return;
  }
  batches.set(batchKey, { ...style, coords: [coord] });
}

export function drawTileFillBatches(state: MinimalMapRendererState, batches: ReadonlyMap<string, TileFillBatch>): void {
  for (const batch of batches.values()) drawTileFillBatch(state, batch);
}

export function drawTileFillBatch(state: MinimalMapRendererState, batch: TileFillBatch): void {
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

export function drawMaskLine(
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

export function drawRoadPass(state: MinimalMapRendererState, point: Coord, segments: Coord[], color: string, width: number): void {
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

export function buildingVectorColor(building: RuntimeBuilding): string {
  if (building.sheet === 'church') return BUILDING_CIVIC;
  if (building.sheet === 'office' || building.sheet === 'modern' || building.sheet === 'tower') return BUILDING_COMMERCIAL;
  if (building.district === 'mill-yard') return BUILDING_INDUSTRIAL;
  return BUILDING_RESIDENTIAL;
}

export function drawEdgeConnections(state: MinimalMapRendererState, visibleGrid: GridRect): void {
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

export function drawFadingEdgeTile(state: MinimalMapRendererState, step: number, draw: () => void): void {
  const { ctx } = state;
  ctx.save();
  ctx.globalAlpha = 0.68 * (1 - step / (EDGE_EXIT_TILES + 1));
  draw();
  ctx.restore();
}
