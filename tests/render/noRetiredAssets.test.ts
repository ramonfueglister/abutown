import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { extname, join } from 'node:path';
import { describe, expect, it } from 'vitest';

const root = process.cwd();
const open = 'open';
const gfx = 'gfx';
const ttd = 'ttd';
const capitalizedTtd = 'Ttd';
const sim = ['simu', 'trans'].join('');
const oldPak = ['pak', '128'].join('');
const oldAssetPack = ['asset', 'Pack'].join('');
const stalePedestrianRoot = [oldPak, 'pedestrians'].join('-');

const removedPaths = [
  ['public', `${open}${gfx}2`],
  ['public', `${open}${ttd}-fan-assets`],
  ['public', `${sim}-assets`],
  ['public', `${sim}-assets`, stalePedestrianRoot],
  ['scripts', `import-${oldPak}-assets.mjs`],
  ['scripts', `import-${open}${gfx}-assets.mjs`],
  ['scripts', `decode-${open}${ttd}-fan-grfs.mjs`],
  ['scripts', `import-${open}${ttd}-map.mjs`],
  ['scripts', `${open}${ttd}MapImportLib.mjs`],
  ['src', 'assets', `${open}${gfx}Catalog.ts`],
  ['src', 'assets', `${open}${gfx}Catalog.generated.ts`],
  ['src', 'assets', `${oldPak}Catalog.ts`],
  ['src', 'assets', `${oldAssetPack}.ts`],
  ['src', 'city', `${open}${capitalizedTtd}Hamburg.generated.ts`],
  ['src', 'city', `${open}${capitalizedTtd}ImportedWorld.ts`],
  ['src', 'render', `${oldPak}RoadVehicleManifest.ts`],
  ['src', 'render', `${sim}PedestrianSprites.ts`],
  ['tests', 'render', `${open}${gfx}Catalog.test.ts`],
  ['tests', 'render', `${oldPak}Catalog.test.ts`],
  ['tests', 'render', `${sim}PedestrianSprites.test.ts`],
  ['tests', 'render', `${oldAssetPack}.test.ts`],
  ['tests', 'scripts', `${open}${ttd}MapImportLib.test.ts`],
  ['tests', 'render', 'assets.test 2.ts'],
];

const forbiddenPattern = new RegExp([
  `${open}${gfx}2`,
  `${open}${ttd}-fan-assets`,
  `assets:${open}${gfx}`,
  `${open}${gfx}Catalog`,
  `import-${open}${gfx}`,
  `decode-${open}${ttd}`,
  `import-${open}${ttd}-map`,
  `${open}${ttd}MapImportLib`,
  `${open}${capitalizedTtd}ImportedWorld`,
  `${open}${capitalizedTtd}Hamburg`,
  `${sim}-assets`,
  `${sim}PedestrianSprites`,
  `${oldPak}Catalog`,
  `${oldPak}RoadVehicleManifest`,
  `import-${oldPak}`,
  `assets:${oldPak}`,
  stalePedestrianRoot,
].map(escapeRegExp).join('|'), 'iu');
const textExtensions = new Set(['.css', '.dat', '.html', '.js', '.json', '.md', '.mjs', '.ts', '.txt']);

describe('retired raster asset removal', () => {
  it('removes old asset directories, import scripts, catalogs, and stray tests', () => {
    for (const pathParts of removedPaths) {
      const relativePath = pathParts.join('/');
      expect(existsSync(join(root, ...pathParts)), relativePath).toBe(false);
    }
  });

  it('keeps runtime and tests free of removed asset references', () => {
    for (const file of scanFiles(['package.json', 'src', 'tests', 'scripts', 'public'])) {
      expect(file, file).not.toMatch(forbiddenPattern);
      if (!textExtensions.has(extname(file))) continue;
      const contents = readFileSync(join(root, file), 'utf8');
      expect(contents, file).not.toMatch(forbiddenPattern);
    }
  });
});

function scanFiles(paths: string[]): string[] {
  const result: string[] = [];
  for (const path of paths) {
    const absolutePath = join(root, path);
    if (!existsSync(absolutePath)) continue;
    const stats = statSync(absolutePath);
    if (stats.isFile()) {
      result.push(path);
      continue;
    }
    for (const entry of readdirSync(absolutePath)) {
      if (entry === 'node_modules' || entry === '.git' || entry === 'dist') continue;
      result.push(...scanFiles([join(path, entry)]));
    }
  }
  return result;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&');
}
