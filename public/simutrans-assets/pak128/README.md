# Simutrans pak128 Assets

Source: https://github.com/simutrans/pak128.git
Revision: `acdf2f0793a6beee5ea34ea85d308fbbeccf50c5`

This directory contains the pak128 source PNG and DAT files used by Abutown's runtime renderer.

## Yellow Crossing HK road overlay

Surface road roles use `infrastructure/roads/yellow_crossing_hk_road_090.png`, generated from reachable Yellow Crossing Addon of Hong Kong source material and credited TKU road source.

Required local source files:

- `infrastructure/roads/yellow-crossing-hk/source/Source1.jpg`
- `infrastructure/roads/yellow-crossing-hk/source/Source2.gif`
- `infrastructure/roads/yellow-crossing-hk/source/tku_road.zip`

The importer refuses to use the existing Pak128 `road_090.png` as a fallback source.

Run `npm run assets:yellow-crossing-hk` after adding the source files. The script repacks the archived Yellow Crossing/TKU source cells into the existing runtime mask layout used by `roadSourceFromMask()` and writes `infrastructure/roads/yellow-crossing-hk/source/manifest.json` with the approved source list.

Imported files:

- `LICENSE.txt`
- `README.txt`
- `base/pedestrians/Pedestrians.dat`
- `base/pedestrians/privat-pedestrians-128.png`
- `base/pedestrians/privat-pedestrians_2-128.png`
- `base/pedestrians/privat-pedestrians_3-128.png`
- `cityhouses/com/com_09_18.dat`
- `cityhouses/com/com_09_18.png`
- `cityhouses/ind/ind_00_03.dat`
- `cityhouses/ind/ind_00_03.png`
- `cityhouses/res/res_08_47.dat`
- `cityhouses/res/res_08_47.png`
- `factories/modern_fuel_station.dat`
- `factories/modern_fuel_station.png`
- `infrastructure/rail_stations/station_03_02.dat`
- `infrastructure/rail_stations/station_03_02.png`
- `infrastructure/rail_tracks/rail_120_tracks.dat`
- `infrastructure/rail_tracks/rail_120_tracks.png`
- `infrastructure/road_bridges/road_040_bridge.dat`
- `infrastructure/road_bridges/road_040_bridge.png`
- `infrastructure/roads/road_090.dat`
- `infrastructure/roads/road_090.png`
- `infrastructure/roads/yellow_crossing_hk_road_090.dat`
- `infrastructure/roads/yellow_crossing_hk_road_090.png` (generated only when Yellow Crossing source files are available)
- `infrastructure/water_all/bulk_dock.dat`
- `infrastructure/water_all/bulk_dock.png`
- `infrastructure/water_all/crate_goods_dock.dat`
- `infrastructure/water_all/crate_goods_dock.png`
- `landscape/grounds/texture-climate.dat`
- `landscape/grounds/texture-climate.png`
- `landscape/grounds/water_ani.dat`
- `landscape/grounds/water_ani.png`
- `landscape/rivers/rivers.dat`
- `landscape/rivers/rivers.png`
- `landscape/trees/tree020.dat`
- `landscape/trees/tree020.png`
- `special_buildings/city/french_park_1.dat`
- `special_buildings/city/french_park_1.png`
- `special_buildings/city/justice.dat`
- `special_buildings/city/justice.png`
- `vehicles/rail-engines/rvg-d41-tubus.dat`
- `vehicles/rail-engines/rvg-d41-tubus.png`
- `vehicles/rail-psg+mail/rvg_tigress_wagon.dat`
- `vehicles/rail-psg+mail/rvg_tigress_wagon.png`
- `vehicles/road-cargo/goods_truck_0.dat`
- `vehicles/road-cargo/goods_truck_0.png`
- `vehicles/road-psg+mail/man_lions_city.dat`
- `vehicles/road-psg+mail/man_lions_city.png`

License: Artistic License 2.0 unless an imported DAT file declares otherwise. See `LICENSE.txt` and individual DAT files.
