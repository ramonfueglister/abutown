#!/usr/bin/env node
import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';

const worldId = 'abutopia';
const schemaVersion = 4;
const root = resolve('data/worlds', worldId);
const width = 80;
const height = 48;
const chunkSize = 32;
const roadY = Math.floor(height / 2);
const roadX0 = 14;
const roadX1 = 65;
const cornerLeftX = 8;
const cornerRightX = 72;
const cornerTopY = 8;
const cornerBottomY = 40;
const houseAX = roadX0 - 1;
const houseBX = roadX1 + 1;
const totalPedestrianAgents = 300;
const edgeCorridorCount = 4;
const pedestrianAgentsPerCorridor = totalPedestrianAgents / edgeCorridorCount;

const E = 2;
const W = 8;
const roads = [];
for (let x = roadX0; x <= roadX1; x += 1) {
  let mask = 0;
  if (x > roadX0) mask |= W;
  if (x < roadX1) mask |= E;
  roads.push({ x, y: roadY, kind: 'street', mask });
}

function horizontalPoints(y, x0 = houseAX, x1 = houseBX) {
  const points = [];
  for (let x = x0; x <= x1; x += 1) points.push({ x, y });
  return points;
}

function verticalPoints(x, y0, y1) {
  const points = [];
  for (let y = y0; y <= y1; y += 1) points.push({ x, y });
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
    markets: 'layers/markets.json',
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
    { id: 'corridor:edge:north', points: horizontalPoints(cornerTopY, cornerLeftX, cornerRightX) },
    { id: 'corridor:edge:east', points: verticalPoints(cornerRightX, cornerTopY, cornerBottomY) },
    { id: 'corridor:edge:south', points: horizontalPoints(cornerBottomY, cornerLeftX, cornerRightX) },
    { id: 'corridor:edge:west', points: verticalPoints(cornerLeftX, cornerTopY, cornerBottomY) },
  ],
};

const buildings = {
  schema_version: schemaVersion,
  world_id: worldId,
  footprints: [
    { id: 'landmark:central-works-a', tiles: [{ x: 31, y: 21 }], sheet: 'modern', frame: 0, district: 'Central Works' },
    { id: 'landmark:central-works-b', tiles: [{ x: 33, y: 21 }], sheet: 'office', frame: 0, district: 'Central Works' },
    { id: 'landmark:market-arcade-a', tiles: [{ x: 47, y: 28 }], sheet: 'shops', frame: 0, district: 'Market Square' },
    { id: 'landmark:market-arcade-b', tiles: [{ x: 49, y: 28 }], sheet: 'shops', frame: 1, district: 'Market Square' },
    { id: 'landmark:harbor-shed-a', tiles: [{ x: 15, y: 28 }], sheet: 'modern', frame: 1, district: 'Harbor Depot' },
    { id: 'landmark:harbor-shed-b', tiles: [{ x: 17, y: 25 }], sheet: 'office', frame: 1, district: 'Harbor Depot' },
    { id: 'landmark:homes-a', tiles: [{ x: 63, y: 21 }], sheet: 'houses', frame: 0, district: 'Homes Quarter' },
    { id: 'landmark:homes-b', tiles: [{ x: 66, y: 21 }], sheet: 'cottages', frame: 0, district: 'Homes Quarter' },
    { id: 'landmark:depot-yard-a', tiles: [{ x: 55, y: 28 }], sheet: 'tower', frame: 0, district: 'Depot Yard' },
    { id: 'landmark:depot-yard-b', tiles: [{ x: 57, y: 28 }], sheet: 'office', frame: 2, district: 'Depot Yard' },
  ],
};

const spawns = {
  schema_version: schemaVersion,
  world_id: worldId,
  pedestrian_groups: [
    { id: 'spawn:ped:north', corridor_id: 'corridor:edge:north', agents_per_corridor: pedestrianAgentsPerCorridor },
    { id: 'spawn:ped:east', corridor_id: 'corridor:edge:east', agents_per_corridor: pedestrianAgentsPerCorridor },
    { id: 'spawn:ped:south', corridor_id: 'corridor:edge:south', agents_per_corridor: pedestrianAgentsPerCorridor },
    { id: 'spawn:ped:west', corridor_id: 'corridor:edge:west', agents_per_corridor: pedestrianAgentsPerCorridor },
  ],
  car_groups: [],
};

const decorations = { schema_version: schemaVersion, world_id: worldId, trees: [], details: [] };

const markets = {
  schema_version: schemaVersion,
  world_id: worldId,
  markets: [
    { id: 9001, name: 'Central Works', anchor: [cornerLeftX, cornerTopY] },
    { id: 9002, name: 'Market Square', anchor: [cornerRightX, cornerTopY] },
    { id: 9003, name: 'Harbor Depot', anchor: [cornerLeftX, cornerBottomY] },
    { id: 9004, name: 'Homes Quarter', anchor: [cornerRightX, cornerBottomY] },
  ],
  distances: [{ from: 9001, to: 9002 }, { from: 9003, to: 9004 }, { from: 9001, to: 9003 }],
  supply: [
    { actor: 8001, market: 9001, good: 4, qty: 10, min_price: 500, opening_inventory: 1000000 },
    { actor: 8011, market: 9001, good: 1, qty: 10, min_price: 500, opening_inventory: 1000000 },
    { actor: 8021, market: 9003, good: 1, qty: 10, min_price: 500, opening_inventory: 1000000 },
  ],
  demand: [
    { actor: 8002, market: 9002, good: 4, qty: 10, max_price: 2000, mpc_bps: 8000, autonomous: 5000, opening_cash: 1000000 },
    { actor: 8012, market: 9002, good: 1, qty: 10, max_price: 2000, mpc_bps: 8000, autonomous: 5000, opening_cash: 1000000 },
    { actor: 8022, market: 9004, good: 1, qty: 10, max_price: 2000, mpc_bps: 8000, autonomous: 5000, opening_cash: 1000000 },
  ],
  extractors: [
    { actor: 8032, market: 9001, in_good: 5, out_good: 1, qty: 10, min_price: 500 },
    { actor: 8033, market: 9003, in_good: 5, out_good: 1, qty: 10, min_price: 500 },
    { actor: 8041, market: 9003, in_good: 5, out_good: 2, qty: 10, min_price: 50 },
  ],
  producers: [
    { actor: 8031, market: 9001, in_good: 2, in_qty: 10, out_good: 4, out_qty: 10, qty: 10, min_price: 500, theta_bps: 8000, batches_target: 2, opening_cash: 1000000 },
  ],
  household: { population: 1000000, capita_baseline: 10 },
  opening_prices: [
    { market: 9002, good: 4, price: 1000 },
    { market: 9002, good: 1, price: 1000 },
    { market: 9004, good: 1, price: 1000 },
    { market: 9001, good: 4, price: 1000 },
    { market: 9003, good: 2, price: 50 },
    { market: 9001, good: 2, price: 380 },
  ],
};

async function main() {
  await mkdir(resolve(root, 'layers'), { recursive: true });
  const write = (rel, obj) => writeFile(resolve(root, rel), `${JSON.stringify(obj, null, 2)}\n`);
  await write('manifest.json', manifest);
  await write('layers/terrain.json', terrain);
  await write('layers/transport.json', transport);
  await write('layers/buildings.json', buildings);
  await write('layers/spawns.json', spawns);
  await write('layers/decorations.json', decorations);
  await write('layers/markets.json', markets);
  console.log(`wrote ${root}`);
}

await main();
