#!/usr/bin/env node
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { createZurichRuntimeContext } from '../src/app/zurichRuntimeContext.ts';
import { buildPedestrianCorridors } from '../src/city/pedestrianCorridors.ts';

const schemaVersion = 1;
const worldId = 'zurich-river-city-v1';
const root = resolve('data/worlds', worldId);
const context = createZurichRuntimeContext({ seed: 1848 });
const world = context.world;
const runtime = context.runtime;
const transport = context.transport;
const pedestrianCorridors = buildPedestrianCorridors(transport.roads, { minLength: 5, maxCorridors: 260 });

function pointKey(point) {
  return `${point.x}:${point.y}`;
}

function sortedCoords(points) {
  return [...points].sort((a, b) => a.y - b.y || a.x - b.x);
}

function assertPoint(point, label) {
  if (
    !Number.isInteger(point.x) ||
    !Number.isInteger(point.y) ||
    point.x < 0 ||
    point.y < 0 ||
    point.x >= world.width ||
    point.y >= world.height
  ) {
    throw new Error(`${label} point out of bounds: ${JSON.stringify(point)}`);
  }
}

function toPath(id, points) {
  if (points.length === 0) throw new Error(`${id} path is empty`);
  for (const point of points) assertPoint(point, id);
  return {
    id,
    points: points.map((point) => ({ x: point.x, y: point.y })),
  };
}

function toTerrainKind(kind) {
  if (kind === 'grass') return 'grass';
  if (kind === 'water') return 'water';
  if (kind === 'riverbank') return 'riverbank';
  if (kind === 'park') return 'park';
  if (kind === 'forest') return 'forest';
  if (kind === 'reserve') return 'reserve';
  if (kind === 'plaza') return 'plaza';
  throw new Error(`unknown terrain kind: ${kind}`);
}

function toRoadKind(kind) {
  if (kind === 'street' || kind === 'bridge') return kind;
  throw new Error(`unknown road kind: ${kind}`);
}

async function writeJson(relativePath, value) {
  const file = resolve(root, relativePath);
  await mkdir(dirname(file), { recursive: true });
  await writeFile(file, `${JSON.stringify(value, null, 2)}\n`);
}

const terrain = {
  schema_version: schemaVersion,
  world_id: worldId,
  tiles: sortedCoords([...world.terrain.values()].map((tile) => tile.coord))
    .map((coord) => world.terrain.get(pointKey(coord)))
    .filter((tile) => tile && tile.kind !== 'grass')
    .map((tile) => ({
      x: tile.coord.x,
      y: tile.coord.y,
      kind: toTerrainKind(tile.kind),
    })),
};

const roadTiles = sortedCoords([...runtime.roads.values()].map((road) => road.coord))
  .map((coord) => runtime.roads.get(pointKey(coord)))
  .filter(Boolean)
  .map((road) => ({
    x: road.coord.x,
    y: road.coord.y,
    kind: toRoadKind(road.kind),
    mask: road.mask,
  }));

const railTiles = sortedCoords([...runtime.rails.values()].map((rail) => rail.coord))
  .map((coord) => runtime.rails.get(pointKey(coord)))
  .filter(Boolean)
  .map((rail) => ({
    x: rail.coord.x,
    y: rail.coord.y,
    mask: rail.mask,
  }));

const arterialPaths = transport.arterialPaths.map((path, index) => toPath(`arterial:${index}`, path));
const railPaths = transport.railPaths.map((path, index) => toPath(`rail:${index}`, path));
const corridorPaths = pedestrianCorridors.map((path, index) => toPath(`pedestrian:${index}`, path));

const transportLayer = {
  schema_version: schemaVersion,
  world_id: worldId,
  roads: roadTiles,
  rails: railTiles,
  arterial_paths: arterialPaths,
  rail_paths: railPaths,
  pedestrian_corridors: corridorPaths,
};

const buildings = {
  schema_version: schemaVersion,
  world_id: worldId,
  footprints: runtime.buildings.map((building, index) => {
    assertPoint(building.coord, `building:${index}`);
    return {
      id: `building:${String(index).padStart(5, '0')}`,
      tiles: [{ x: building.coord.x, y: building.coord.y }],
      sheet: building.sheet,
      frame: building.frame,
      district: building.district,
    };
  }),
};

const decorations = {
  schema_version: schemaVersion,
  world_id: worldId,
  trees: runtime.trees.map((coord) => {
    assertPoint(coord, 'tree');
    return { x: coord.x, y: coord.y };
  }),
  details: runtime.details.map((detail, index) => {
    assertPoint(detail.coord, `detail:${index}`);
    return {
      x: detail.coord.x,
      y: detail.coord.y,
      category: detail.category,
      asset_category: detail.assetCategory,
    };
  }),
};

const spawns = {
  schema_version: schemaVersion,
  world_id: worldId,
  pedestrian_groups: corridorPaths.map((corridor) => ({
    id: `spawn:ped:${corridor.id}`,
    corridor_id: corridor.id,
    agents_per_corridor: 6,
  })),
  car_groups: arterialPaths.map((arterial) => ({
    id: `spawn:car:${arterial.id}`,
    arterial_id: arterial.id,
    cars_per_arterial: 17,
  })),
  tram_lines: railPaths.map((railPath) => ({
    id: `tram:${railPath.id}`,
    rail_path_ids: [railPath.id],
    trams: 4,
  })),
};

const manifest = {
  schema_version: schemaVersion,
  world_id: worldId,
  display_name: 'Zurich River City',
  chunk_size: world.chunkSize,
  world_tiles: { width: world.width, height: world.height },
  layers: {
    terrain: 'layers/terrain.json',
    transport: 'layers/transport.json',
    buildings: 'layers/buildings.json',
    decorations: 'layers/decorations.json',
    spawns: 'layers/spawns.json',
  },
};

await writeJson('manifest.json', manifest);
await writeJson('layers/terrain.json', terrain);
await writeJson('layers/transport.json', transportLayer);
await writeJson('layers/buildings.json', buildings);
await writeJson('layers/decorations.json', decorations);
await writeJson('layers/spawns.json', spawns);

console.log(JSON.stringify({
  worldId,
  terrain: terrain.tiles.length,
  roads: roadTiles.length,
  rails: railTiles.length,
  buildings: buildings.footprints.length,
  trees: decorations.trees.length,
  details: decorations.details.length,
  arterialPaths: arterialPaths.length,
  railPaths: railPaths.length,
  pedestrianCorridors: corridorPaths.length,
}, null, 2));
