import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { tmpdir } from 'node:os';

const revision = 'acdf2f0793a6beee5ea34ea85d308fbbeccf50c5';
const repoUrl = 'https://github.com/simutrans/pak128.git';
const root = process.cwd();
const temp = join(tmpdir(), 'abutown-pak128-import');
const outRoot = join(root, 'public', 'simutrans-assets', 'pak128');

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
  'vehicles/road-cargo/rvg_type_s_van.dat',
  'vehicles/road-cargo/rvg_type_s_van.png',
  'vehicles/road-cargo/cooling_truck_0.dat',
  'vehicles/road-cargo/cooling_truck_0.png',
  'vehicles/road-cargo/fluid_truck_0.dat',
  'vehicles/road-cargo/fluid_truck_0.png',
  'vehicles/road-cargo/concrete_truck_0.dat',
  'vehicles/road-cargo/concrete_truck_0.png',
  'vehicles/road-cargo/bulk_truck_0.dat',
  'vehicles/road-cargo/bulk_truck_0.png',
  'vehicles/road-cargo/car_transporter_0.dat',
  'vehicles/road-cargo/car_transporter_0.png',
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
