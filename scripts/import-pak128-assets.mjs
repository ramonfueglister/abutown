import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { basename, dirname, join } from 'node:path';
import { tmpdir } from 'node:os';

const revision = 'acdf2f0793a6beee5ea34ea85d308fbbeccf50c5';
const repoUrl = 'https://github.com/simutrans/pak128.git';
const root = process.cwd();
const temp = join(tmpdir(), 'abutown-pak128-import');
const outRoot = join(root, 'public', 'simutrans-assets', 'pak128');

const baseFiles = [
  'LICENSE.txt',
  'README.txt',
  'base/pedestrians/Pedestrians.dat',
  'base/pedestrians/privat-pedestrians-128.png',
  'base/pedestrians/privat-pedestrians_2-128.png',
  'base/pedestrians/privat-pedestrians_3-128.png',
  'cityhouses/com/com_09_18.dat',
  'cityhouses/com/com_09_18.png',
  'cityhouses/ind/ind_00_03.dat',
  'cityhouses/ind/ind_00_03.png',
  'cityhouses/res/res_08_47.dat',
  'cityhouses/res/res_08_47.png',
  'factories/modern_fuel_station.dat',
  'factories/modern_fuel_station.png',
  'infrastructure/rail_stations/station_03_02.dat',
  'infrastructure/rail_stations/station_03_02.png',
  'infrastructure/rail_tracks/rail_120_tracks.dat',
  'infrastructure/rail_tracks/rail_120_tracks.png',
  'infrastructure/road_bridges/road_040_bridge.dat',
  'infrastructure/road_bridges/road_040_bridge.png',
  'infrastructure/roads/road_090.dat',
  'infrastructure/roads/road_090.png',
  'infrastructure/water_all/bulk_dock.dat',
  'infrastructure/water_all/bulk_dock.png',
  'infrastructure/water_all/crate_goods_dock.dat',
  'infrastructure/water_all/crate_goods_dock.png',
  'landscape/grounds/texture-climate.dat',
  'landscape/grounds/texture-climate.png',
  'landscape/grounds/water_ani.dat',
  'landscape/grounds/water_ani.png',
  'landscape/rivers/rivers.dat',
  'landscape/rivers/rivers.png',
  'landscape/trees/tree020.dat',
  'landscape/trees/tree020.png',
  'special_buildings/city/french_park_1.dat',
  'special_buildings/city/french_park_1.png',
  'special_buildings/city/justice.dat',
  'special_buildings/city/justice.png',
  'vehicles/rail-engines/rvg-d41-tubus.dat',
  'vehicles/rail-engines/rvg-d41-tubus.png',
  'vehicles/rail-psg+mail/rvg_tigress_wagon.dat',
  'vehicles/rail-psg+mail/rvg_tigress_wagon.png',
];

const roadVehicleDirs = ['vehicles/road-cargo', 'vehicles/road-psg+mail'];

rmSync(temp, { recursive: true, force: true });
rmSync(outRoot, { recursive: true, force: true });
mkdirSync(outRoot, { recursive: true });

execFileSync('git', ['clone', '--filter=blob:none', '--sparse', '--no-checkout', repoUrl, temp], { stdio: 'inherit' });
execFileSync('git', [
  '-C',
  temp,
  'sparse-checkout',
  'set',
  '--no-cone',
  ...baseFiles.map((file) => `/${file}`),
  ...roadVehicleDirs.map((dir) => `/${dir}/*`),
], { stdio: 'inherit' });
execFileSync('git', ['-C', temp, 'checkout', revision], { stdio: 'inherit' });

const roadVehicleFiles = discoverPak128RoadVehicleFiles(temp);
const files = [...new Set([...baseFiles, ...roadVehicleFiles])].sort();

for (const file of files) {
  const source = join(temp, file);
  if (!existsSync(source)) throw new Error(`Missing pak128 source file: ${file}`);
  const destination = join(outRoot, file);
  mkdirSync(dirname(destination), { recursive: true });
  copyFileSync(source, destination);
}

const roadVehicleManifest = buildRoadVehicleManifest(roadVehicleFiles);
writeFileSync(join(root, 'src', 'render', 'pak128RoadVehicleManifest.ts'), `${roadVehicleManifestTs(roadVehicleManifest)}\n`);

writeFileSync(join(outRoot, 'README.md'), `# Simutrans pak128 Assets

Source: ${repoUrl}
Revision: \`${revision}\`

This directory contains the pak128 source PNG and DAT files used by Abutown's runtime renderer.

Imported files:

${files.map((file) => `- \`${file}\``).join('\n')}

License: Artistic License 2.0 unless an imported DAT file declares otherwise. See \`LICENSE.txt\` and individual DAT files.
`);

console.log(`Imported ${files.length} pak128 files into ${outRoot}`);

function discoverPak128RoadVehicleFiles(repoPath) {
  const available = new Set(roadVehicleDirs.flatMap((dir) => (
    readdirSync(join(repoPath, dir))
      .filter((file) => file.endsWith('.dat') || file.endsWith('.png'))
      .map((file) => `${dir}/${file}`)
  )));
  const datFiles = [...available].filter((file) => file.endsWith('.dat'));
  const selected = new Set();

  for (const datFile of datFiles) {
    const imageBase = imageBaseFromDatFile(repoPath, datFile);
    const pngFile = imageBase ? `${dirname(datFile)}/${imageBase}.png` : datFile.replace(/\.dat$/u, '.png');
    if (available.has(pngFile)) {
      selected.add(datFile);
      selected.add(pngFile);
    }
  }

  return [...selected].sort();
}

function imageBaseFromDatFile(repoPath, datFile) {
  const dat = readFileSync(join(repoPath, datFile), 'utf8');
  const match = dat.match(/^emptyimage\[w\]=([^.\n]+(?:\.[^.\n]+)*)\.(\d+)\.(\d+)/imu);
  return match?.[1] ?? datFile.replace(/^.*\/|\.dat$/gu, '');
}

function buildRoadVehicleManifest(roadFiles) {
  const pngFiles = new Set(roadFiles.filter((file) => file.endsWith('.png')));
  const entries = [];

  for (const datPath of roadFiles.filter((file) => file.endsWith('.dat'))) {
    const dat = readFileSync(join(outRoot, datPath), 'utf8');
    if (!/^obj=vehicle$/imu.test(dat)) continue;

    const image = dat.match(/^emptyimage\[w\]=([^.\n]+(?:\.[^.\n]+)*)\.(\d+)\.(\d+)/imu);
    if (!image) continue;

    const imageBase = image[1];
    const row = Number(image[2]);
    const pngPath = `${dirname(datPath)}/${imageBase}.png`;
    if (!pngFiles.has(pngPath) || !Number.isFinite(row)) continue;

    entries.push({
      id: basename(datPath, '.dat'),
      name: dat.match(/^name=(.+)$/imu)?.[1]?.trim() ?? basename(datPath, '.dat'),
      path: `/simutrans-assets/pak128/${pngPath}`,
      datPath,
      row,
      scale: roadVehicleScale(datPath),
    });
  }

  return entries.sort((a, b) => a.id.localeCompare(b.id));
}

function roadVehicleScale(datPath) {
  if (datPath.includes('road-psg+mail')) return 0.42;
  if (/trailer|transport|long|steel|log|container/iu.test(datPath)) return 0.4;
  return 0.42;
}

function roadVehicleManifestTs(entries) {
  return `export type Pak128RoadVehicleManifestEntry = {
  id: string;
  name: string;
  path: string;
  datPath: string;
  row: number;
  scale: number;
};

export const PAK128_ROAD_VEHICLES = ${JSON.stringify(entries, null, 2)} as const satisfies readonly Pak128RoadVehicleManifestEntry[];
`;
}
