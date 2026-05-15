import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { tmpdir } from 'node:os';

const revision = 'acdf2f0793a6beee5ea34ea85d308fbbeccf50c5';
const repoUrl = 'https://github.com/simutrans/pak128.git';
const root = process.cwd();
const temp = join(tmpdir(), 'abutown-pak128-import');
const outRoot = join(root, 'public', 'simutrans-assets', 'pak128');

if (process.argv.includes('--yellow-crossing-hk')) {
  importYellowCrossingHk();
  process.exit(0);
}

const files = [
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
  'vehicles/road-cargo/goods_truck_0.dat',
  'vehicles/road-cargo/goods_truck_0.png',
  'vehicles/road-psg+mail/man_lions_city.dat',
  'vehicles/road-psg+mail/man_lions_city.png',
];

rmSync(temp, { recursive: true, force: true });
rmSync(outRoot, { recursive: true, force: true });
mkdirSync(outRoot, { recursive: true });

execFileSync('git', ['clone', '--filter=blob:none', '--sparse', '--no-checkout', repoUrl, temp], { stdio: 'inherit' });
execFileSync('git', ['-C', temp, 'sparse-checkout', 'set', '--no-cone', ...files.map((file) => `/${file}`)], { stdio: 'inherit' });
execFileSync('git', ['-C', temp, 'checkout', revision], { stdio: 'inherit' });

for (const file of files) {
  const source = join(temp, file);
  if (!existsSync(source)) throw new Error(`Missing pak128 source file: ${file}`);
  const destination = join(outRoot, file);
  mkdirSync(dirname(destination), { recursive: true });
  copyFileSync(source, destination);
}

writeFileSync(join(outRoot, 'README.md'), `# Simutrans pak128 Assets

Source: ${repoUrl}
Revision: \`${revision}\`

This directory contains the pak128 source PNG and DAT files used by Abutown's runtime renderer.

Imported files:

${files.map((file) => `- \`${file}\``).join('\n')}

License: Artistic License 2.0 unless an imported DAT file declares otherwise. See \`LICENSE.txt\` and individual DAT files.
`);

console.log(`Imported ${files.length} pak128 files into ${outRoot}`);

function importYellowCrossingHk() {
  const sourceDir = join(outRoot, 'infrastructure', 'roads', 'yellow-crossing-hk', 'source');
  const roadsDir = join(outRoot, 'infrastructure', 'roads');
  const runtimePng = join(roadsDir, 'yellow_crossing_hk_road_090.png');
  const runtimeDat = join(roadsDir, 'yellow_crossing_hk_road_090.dat');
  const manifestPath = join(sourceDir, 'manifest.json');
  const archiveName = 'tku_road.zip';
  const sourceSheetName = join('tku_road', 'tku_style_road_a02.png');
  const sourceDatName = join('tku_road', 'tku_style_road.dat');
  const required = ['Source1.jpg', 'Source2.gif', archiveName];

  if (!required.every((file) => existsSync(join(sourceDir, file)))) {
    throw new Error([
      'Missing Yellow Crossing HK source files.',
      `Expected ${required.join(', ')} under:`,
      `- ${sourceDir}`,
      'Add the original Yellow Crossing Addon/source files before running this importer.',
      'This importer will not use the existing Pak128 road sheet as a fallback.',
    ].join('\n'));
  }

  const oldManifest = join(sourceDir, 'manifest.json');
  if (existsSync(oldManifest)) {
    const parsed = JSON.parse(readFileSync(oldManifest, 'utf8'));
    if (parsed.userApprovedReuse === false) throw new Error('Yellow Crossing HK source manifest explicitly denies reuse.');
  }

  const yellowTemp = mkdtempSync(join(tmpdir(), 'abutown-yellow-crossing-'));
  execFileSync('unzip', ['-q', join(sourceDir, archiveName), '-d', yellowTemp]);
  const sourceSheet = join(yellowTemp, sourceSheetName);
  const sourceDat = join(yellowTemp, sourceDatName);
  if (!existsSync(sourceSheet)) {
    throw new Error(`Yellow Crossing archive is missing ${sourceSheetName}. Available source files: ${readdirSync(sourceDir).sort().join(', ')}`);
  }

  const baseRuntime = join(yellowTemp, 'yellow_crossing_hk_road_090_base.png');
  const markedRuntime = join(yellowTemp, 'yellow_crossing_hk_road_090_marked.png');
  const cellMap = [
    [[3, 2], [1, 0], 'dead-end'],
    [[1, 1], [1, 1], 'north'],
    [[1, 3], [1, 2], 'south'],
    [[1, 2], [1, 3], 'east'],
    [[1, 0], [1, 4], 'west'],
    [[3, 0], [1, 5], 'north-south'],
    [[3, 1], [1, 6], 'east-west'],
    [[7, 0], [1, 7], 'north-south-east'],
    [[7, 1], [2, 0], 'north-south-west'],
    [[7, 3], [2, 1], 'north-east-west'],
    [[7, 2], [2, 2], 'south-east-west'],
    [[3, 3], [2, 3], 'four-way'],
    [[6, 1], [2, 4], 'north-east'],
    [[6, 3], [2, 5], 'south-east'],
    [[6, 0], [2, 6], 'north-west'],
    [[6, 2], [2, 7], 'south-west'],
  ];
  const compositeArgs = ['-size', '1024x384', 'canvas:none'];
  for (const [[sourceRow, sourceCol], [targetRow, targetCol], name] of cellMap) {
    const cellPng = join(yellowTemp, `${name}.png`);
    execFileSync('magick', [sourceSheet, '-crop', `128x128+${sourceCol * 128}+${sourceRow * 128}`, '+repage', cellPng]);
    compositeArgs.push(cellPng, '-geometry', `+${targetCol * 128}+${targetRow * 128}`, '-composite');
  }
  compositeArgs.push(baseRuntime);
  execFileSync('magick', compositeArgs);
  drawYellowCrossingBars(baseRuntime, markedRuntime);

  copyFileSync(markedRuntime, runtimePng);
  writeFileSync(manifestPath, `${JSON.stringify({
    userApprovedReuse: true,
    sourceForum: 'https://forum.simutrans.com/index.php/topic,20304.0.html',
    sourceFiles: ['Source1.jpg', 'Source2.gif', ...(existsSync(join(sourceDir, 'Demo.png')) ? ['Demo.png'] : []), archiveName],
    runtimePng: 'yellow_crossing_hk_road_090.png',
    runtimeDerivedFrom: `${sourceSheetName} + Source2.gif yellow crossing markings`,
    noFallbackSource: 'public/simutrans-assets/pak128/infrastructure/roads/road_090.png',
  }, null, 2)}\n`);
  writeFileSync(runtimeDat, `# Yellow Crossing Addon of Hong Kong road runtime sheet

name=yellow_crossing_hk_road_090
object=way
copyright=OrangeSkin325 / Yellow Crossing Addon of Hong Kong
license=See original forum/source material; verify redistribution before publishing.
source_forum=https://forum.simutrans.com/index.php/topic,20304.0.html
source_reference=Source1.jpg
source_reference=Source2.gif
source_archive=${archiveName}
source_dat=${existsSync(sourceDat) ? sourceDatName : ''}
runtime_source=${sourceSheetName} + Source2.gif yellow crossing markings
runtime_png=yellow_crossing_hk_road_090.png
source_manifest=yellow-crossing-hk/source/manifest.json

# Generated by scripts/import-pak128-assets.mjs --yellow-crossing-hk from local source files.
`);
  console.log(`Imported Yellow Crossing HK road sheet into ${runtimePng}`);
}

function drawYellowCrossingBars(inputPng, outputPng) {
  const line = (row, col, x1, y1, x2, y2) => `line ${col * 128 + x1},${row * 128 + y1} ${col * 128 + x2},${row * 128 + y2}`;
  const bars = (row, col) => [
    line(row, col, 55, 38, 73, 47),
    line(row, col, 49, 45, 67, 54),
    line(row, col, 55, 80, 73, 89),
    line(row, col, 49, 87, 67, 96),
    line(row, col, 30, 62, 48, 53),
    line(row, col, 38, 68, 56, 59),
    line(row, col, 80, 53, 98, 62),
    line(row, col, 72, 59, 90, 68),
  ];
  const commands = [...bars(2, 3), ...bars(1, 7), ...bars(2, 0), ...bars(2, 1), ...bars(2, 2)];
  execFileSync('magick', [
    inputPng,
    '-stroke', '#ffd400',
    '-strokewidth', '3',
    '-fill', 'none',
    ...commands.flatMap((command) => ['-draw', command]),
    outputPng,
  ]);
}
