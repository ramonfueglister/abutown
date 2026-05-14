#!/usr/bin/env node
import { existsSync } from 'node:fs';
import { join, resolve } from 'node:path';
import {
  decodeOpenTtdSavegame,
  downloadBananasContent,
  normalizeOpenTtdMap,
  writeTypeScriptModule,
} from './openttdMapImportLib.mjs';

const args = parseArgs(process.argv.slice(2));
const contentId = Number(args.contentId ?? 11910279);
const targetSize = Number(args.targetSize ?? 512);
const id = String(args.id ?? 'openttd-hamburg-512');
const cacheDir = resolve(String(args.cacheDir ?? 'artifacts/openttd-content'));
const output = resolve(String(args.output ?? 'src/city/openTtdHamburg.generated.ts'));

if (!Number.isInteger(contentId) || contentId <= 0) throw new Error(`Invalid --content-id ${args.contentId}`);
if (!Number.isInteger(targetSize) || targetSize <= 0) throw new Error(`Invalid --target-size ${args.targetSize}`);

const downloadDir = join(cacheDir, String(contentId));
const cachedScenario = args.scenario ? resolve(String(args.scenario)) : undefined;
const scenarioPath = cachedScenario && existsSync(cachedScenario)
  ? cachedScenario
  : (await downloadBananasContent(contentId, downloadDir)).scenarioPath;

const decoded = decodeOpenTtdSavegame(scenarioPath);
const normalized = normalizeOpenTtdMap({
  id,
  source: scenarioPath,
  sourceWidth: decoded.sourceWidth,
  sourceHeight: decoded.sourceHeight,
  targetSize,
  tileTypes: decoded.tileTypes,
});

writeTypeScriptModule(normalized, output, 'openTtdImportedMap');

console.log(JSON.stringify({
  output,
  source: scenarioPath,
  sourceWidth: decoded.sourceWidth,
  sourceHeight: decoded.sourceHeight,
  targetSize,
  terrainRuns: normalized.terrainRle.length,
  roads: normalized.roads.length,
  rails: normalized.rails.length,
  buildings: normalized.buildings.length,
  trees: normalized.trees.length,
  details: normalized.details.length,
}, null, 2));

function parseArgs(argv) {
  const parsed = {};
  for (let index = 0; index < argv.length; index += 1) {
    const item = argv[index];
    if (!item.startsWith('--')) continue;
    const [rawKey, rawValue] = item.slice(2).split('=');
    const key = rawKey.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
    parsed[key] = rawValue ?? argv[index + 1];
    if (rawValue === undefined) index += 1;
  }
  return parsed;
}
