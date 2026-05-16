#!/usr/bin/env node
// Run with: node --import tsx/esm scripts/generate-city-network.mjs
// tsx/esm is loaded via --import flag (required for Node >= 20.6 / 24).
import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');

const { buildZurichWorld } = await import(resolve(root, 'src/city/zurichWorld.ts'));
const { buildZurichTransport } = await import(resolve(root, 'src/city/zurichTransport.ts'));
const { buildPedestrianCorridors } = await import(resolve(root, 'src/city/pedestrianCorridors.ts'));

const world = buildZurichWorld({ seed: 1848 });
const transport = buildZurichTransport(world);
const corridors = buildPedestrianCorridors(transport.roads, { minLength: 5, maxCorridors: 260 });

const network = {
  version: 1,
  world_id: 'zurich-river-city-v1',
  chunk_size: 32,
  world_tiles: { width: world.width, height: world.height },
  arterial_paths: transport.arterialPaths.map((path) => path.map(({ x, y }) => ({ x, y }))),
  pedestrian_corridors: corridors.map((path) => path.map(({ x, y }) => ({ x, y }))),
};

const outPath = resolve(root, 'data/city/zurich-network.json');
mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, JSON.stringify(network, null, 2) + '\n');

console.log(
  `wrote ${outPath} — ${network.arterial_paths.length} arterial paths, ${network.pedestrian_corridors.length} pedestrian corridors`,
);
