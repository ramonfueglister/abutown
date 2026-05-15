import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { extname, join } from 'node:path';
import { describe, expect, it } from 'vitest';

const root = process.cwd();
const open = 'open';
const gfx = 'gfx';
const ttd = 'ttd';
const capitalizedTtd = 'Ttd';
const stalePedestrianRoot = ['pak128', 'pedestrians'].join('-');

const removedPaths = [
  ['public', `${open}${gfx}2`],
  ['public', `${open}${ttd}-fan-assets`],
  ['public', 'simutrans-assets', stalePedestrianRoot],
  ['scripts', `import-${open}${gfx}-assets.mjs`],
  ['scripts', `decode-${open}${ttd}-fan-grfs.mjs`],
  ['scripts', `import-${open}${ttd}-map.mjs`],
  ['scripts', `${open}${ttd}MapImportLib.mjs`],
  ['src', 'assets', `${open}${gfx}Catalog.ts`],
  ['src', 'assets', `${open}${gfx}Catalog.generated.ts`],
  ['src', 'city', `${open}${capitalizedTtd}Hamburg.generated.ts`],
  ['src', 'city', `${open}${capitalizedTtd}ImportedWorld.ts`],
  ['tests', 'render', `${open}${gfx}Catalog.test.ts`],
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
