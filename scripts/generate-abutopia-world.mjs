#!/usr/bin/env node
import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';

const worldId = 'abutopia';
const schemaVersion = 1;
const root = resolve('data/worlds', worldId);
const width = 16;
const height = 8;
const chunkSize = 32;
const roadY = 3;
const roadX0 = 3;
const roadX1 = 12;
const houseAX = 2;
const houseBX = 13;

const E = 2;
const W = 8;
const roads = [];
for (let x = roadX0; x <= roadX1; x += 1) {
  let mask = 0;
  if (x > roadX0) mask |= W;
  if (x < roadX1) mask |= E;
  roads.push({ x, y: roadY, kind: 'street', mask });
}

const sidewalkOffset = 0.51;
const sidewalkNorthY = Number((roadY - sidewalkOffset).toFixed(2));
const sidewalkSouthY = Number((roadY + sidewalkOffset).toFixed(2));

function corridorPointsFor(y) {
  const points = [];
  for (let x = houseAX; x <= houseBX; x += 1) points.push({ x, y });
  return points;
}

const manifest = {
  schema_version: schemaVersion,
  world_id: worldId,
  display_name: 'Abutopia',
  chunk_size: chunkSize,
  world_tiles: { width, height },
  layers: {
    terrain: 'layers/terrain.json',
    transport: 'layers/transport.json',
    buildings: 'layers/buildings.json',
    decorations: 'layers/decorations.json',
    spawns: 'layers/spawns.json',
  },
};

const terrain = { schema_version: schemaVersion, world_id: worldId, tiles: [] };

const transport = {
  schema_version: schemaVersion,
  world_id: worldId,
  roads,
  rails: [],
  arterial_paths: [],
  rail_paths: [],
  pedestrian_corridors: [
    { id: 'corridor:sidewalk:north', points: corridorPointsFor(sidewalkNorthY) },
    { id: 'corridor:sidewalk:south', points: corridorPointsFor(sidewalkSouthY) },
  ],
};

const buildings = {
  schema_version: schemaVersion,
  world_id: worldId,
  footprints: [
    { id: 'building:house-a', tiles: [{ x: houseAX, y: roadY }], sheet: 'oldhouses', frame: 0 },
    { id: 'building:house-b', tiles: [{ x: houseBX, y: roadY }], sheet: 'oldhouses', frame: 1 },
  ],
};

const spawns = {
  schema_version: schemaVersion,
  world_id: worldId,
  pedestrian_groups: [
    { id: 'spawn:ped:sidewalk-south', corridor_id: 'corridor:sidewalk:south', agents_per_corridor: 1 },
  ],
  car_groups: [],
  tram_lines: [],
};

const decorations = { schema_version: schemaVersion, world_id: worldId, trees: [], details: [] };

async function main() {
  await mkdir(resolve(root, 'layers'), { recursive: true });
  const write = (rel, obj) => writeFile(resolve(root, rel), `${JSON.stringify(obj, null, 2)}\n`);
  await write('manifest.json', manifest);
  await write('layers/terrain.json', terrain);
  await write('layers/transport.json', transport);
  await write('layers/buildings.json', buildings);
  await write('layers/spawns.json', spawns);
  await write('layers/decorations.json', decorations);
  console.log(`wrote ${root}`);
}

await main();
