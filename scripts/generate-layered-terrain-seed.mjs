#!/usr/bin/env node
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { buildLayeredTerrainSeed, validateLayeredTerrainSeed } from '../src/city/layeredTerrainSeed.ts';
import { buildZurichPlacement } from '../src/city/zurichPlacement.ts';
import { buildZurichTransport } from '../src/city/zurichTransport.ts';
import { buildZurichWorld } from '../src/city/zurichWorld.ts';

const world = buildZurichWorld({ seed: 1848 });
const transport = buildZurichTransport(world);
const placement = buildZurichPlacement(world, transport);
const seed = buildLayeredTerrainSeed({ world, transport, placement });
const validationErrors = validateLayeredTerrainSeed(seed);

if (validationErrors.length > 0) {
  throw new Error(`layered terrain seed validation failed:\n${validationErrors.join('\n')}`);
}

const outPath = path.resolve('data/city/zurich-layered-terrain-seed.json');
await mkdir(path.dirname(outPath), { recursive: true });
await writeFile(outPath, `${JSON.stringify(seed, null, 2)}\n`);

console.log(`layered terrain seed complete -> ${outPath} (${seed.tiles.length} tiles)`);
