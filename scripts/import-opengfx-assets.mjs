import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, rmSync, copyFileSync, writeFileSync } from 'node:fs';
import { basename, dirname, extname, join, relative } from 'node:path';
import { tmpdir } from 'node:os';

const repoUrl = 'https://github.com/OpenTTD/OpenGFX2.git';
const root = process.cwd();
const temp = join(tmpdir(), 'abutown-opengfx2-import');
const publicRoot = join(root, 'public', 'opengfx2', 'all');
const licenseRoot = join(root, 'public', 'opengfx2', 'licenses');
const generatedPath = join(root, 'src', 'assets', 'opengfxCatalog.generated.ts');

rmSync(temp, { recursive: true, force: true });
mkdirSync(dirname(generatedPath), { recursive: true });
mkdirSync(publicRoot, { recursive: true });
mkdirSync(licenseRoot, { recursive: true });

execFileSync('git', ['clone', '--depth', '1', '--filter=blob:none', '--sparse', repoUrl, temp], { stdio: 'inherit' });
execFileSync('git', ['-C', temp, 'sparse-checkout', 'set', '--no-cone', '/LICENSE', '/credits.md', 'graphics'], { stdio: 'inherit' });

for (const file of ['LICENSE', 'credits.md']) {
  const source = join(temp, file);
  if (existsSync(source)) copyFileSync(source, join(licenseRoot, file));
}

const pngs = [];
function walk(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) walk(full);
    if (entry.isFile() && extname(entry.name).toLowerCase() === '.png') pngs.push(full);
  }
}
walk(join(temp, 'graphics'));

const usefulPngs = pngs.filter((file) => {
  const normalized = file.replaceAll('\\', '/');
  return normalized.includes('/64/') || normalized.includes('/32bpp') || normalized.includes('_shape.png') || normalized.includes('overlayalpha');
});

const assets = [];
for (const source of usefulPngs) {
  const rel = relative(join(temp, 'graphics'), source).replaceAll('\\', '/');
  const outputName = rel.replaceAll('/', '__');
  const destination = join(publicRoot, outputName);
  copyFileSync(source, destination);
  assets.push({
    key: outputName.replace(/\.png$/u, ''),
    path: `/opengfx2/all/${outputName}`,
    sourcePath: `graphics/${rel}`,
    fileName: basename(source),
    category: categorize(rel),
  });
}

assets.sort((a, b) => a.key.localeCompare(b.key));

writeFileSync(generatedPath, `export type OpenGfxAssetCategory =
  | 'terrain'
  | 'water'
  | 'road'
  | 'rail'
  | 'bridge'
  | 'building'
  | 'tree'
  | 'vehicle'
  | 'industry'
  | 'station'
  | 'decor'
  | 'unknown';

export type OpenGfxAsset = {
  key: string;
  path: string;
  sourcePath: string;
  fileName: string;
  category: OpenGfxAssetCategory;
};

export const opengfxAssets: OpenGfxAsset[] = ${JSON.stringify(assets, null, 2)} as const satisfies OpenGfxAsset[];
`);

console.log(`Imported ${assets.length} OpenGFX assets into ${publicRoot}`);

function categorize(rel) {
  const value = rel.toLowerCase();
  if (value.includes('terrain') || value.includes('ground') || value.includes('landscape')) return 'terrain';
  if (value.includes('water') || value.includes('river') || value.includes('canal')) return 'water';
  if (value.includes('bridge')) return 'bridge';
  if (value.includes('road') || value.includes('street')) return 'road';
  if (value.includes('rail') || value.includes('train') || value.includes('track')) return 'rail';
  if (value.includes('station')) return 'station';
  if (value.includes('town') || value.includes('house') || value.includes('office') || value.includes('church')) return 'building';
  if (value.includes('tree') || value.includes('forest')) return 'tree';
  if (value.includes('vehicle') || value.includes('bus') || value.includes('lorry') || value.includes('truck')) return 'vehicle';
  if (value.includes('industry') || value.includes('industrial')) return 'industry';
  if (value.includes('object') || value.includes('furniture') || value.includes('fence')) return 'decor';
  return 'unknown';
}
