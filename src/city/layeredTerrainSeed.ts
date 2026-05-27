import { key, type ZurichBuilding, type ZurichDetail, type ZurichTerrainKind, type ZurichWorld } from './worldTypes';
import type { ZurichPlacement } from './zurichPlacement';
import type { ZurichTransport } from './zurichTransport';

export type LayeredBaseKind = 'Grass' | 'Water' | 'Riverbank' | 'Forest' | 'Park' | 'Reserve' | 'Plaza';
export type LayeredSurfaceKind = 'None' | 'Street' | 'Bridge' | 'Rail' | 'RailCrossing';
export type LayeredCoverKind = 'None' | 'Building' | 'Tree' | 'Detail';

export type LayeredTerrainTile = {
  x: number;
  y: number;
  base: LayeredBaseKind;
  surface: LayeredSurfaceKind;
  cover: LayeredCoverKind;
  display: string | null;
  zone_id: string | null;
  road_mask: number | null;
  rail_mask: number | null;
  version: number;
};

export type LayeredTerrainSeed = {
  version: 1;
  world_id: string;
  width: number;
  height: number;
  chunk_size: number;
  tiles: LayeredTerrainTile[];
};

export function buildLayeredTerrainSeed(input: {
  world: ZurichWorld;
  transport: ZurichTransport;
  placement: ZurichPlacement;
}): LayeredTerrainSeed {
  const buildingsByKey = new Map(input.placement.buildings.map((building) => [key(building.coord), building]));
  const treesByKey = new Set(input.placement.trees.map(key));
  const detailsByKey = new Map(input.placement.details.map((detail) => [key(detail.coord), detail]));
  const tiles: LayeredTerrainTile[] = [];

  for (let y = 0; y < input.world.height; y += 1) {
    for (let x = 0; x < input.world.width; x += 1) {
      const coord = { x, y };
      const tileKey = key(coord);
      const terrain = input.world.terrain.get(tileKey);
      if (!terrain) throw new Error(`missing terrain tile ${tileKey}`);
      const road = input.transport.roads.get(tileKey);
      const rail = input.transport.rails.get(tileKey);
      const building = buildingsByKey.get(tileKey);
      const detail = detailsByKey.get(tileKey);
      const surface = surfaceFor({ roadKind: road?.kind, hasRail: Boolean(rail), isRailCrossing: input.transport.railCrossings.has(tileKey) });
      const cover = coverFor({ building, hasTree: treesByKey.has(tileKey), detail, surface });

      tiles.push({
        x,
        y,
        base: baseFor(terrain.kind),
        surface,
        cover,
        display: displayFor({ building, detail, cover }),
        zone_id: terrain.zoneId ?? null,
        road_mask: road ? road.mask : null,
        rail_mask: rail ? rail.mask : null,
        version: 0,
      });
    }
  }

  return {
    version: 1,
    world_id: input.world.id,
    width: input.world.width,
    height: input.world.height,
    chunk_size: input.world.chunkSize,
    tiles,
  };
}

export function validateLayeredTerrainSeed(seed: LayeredTerrainSeed): string[] {
  const errors: string[] = [];
  const seen = new Set<string>();
  if (seed.tiles.length !== seed.width * seed.height) errors.push(`tile_count:${seed.tiles.length}`);
  if (seed.width % seed.chunk_size !== 0 || seed.height % seed.chunk_size !== 0) errors.push('chunk_size:does_not_partition_world');

  for (const tile of seed.tiles) {
    const tileKey = `${tile.x}:${tile.y}`;
    if (tile.x < 0 || tile.y < 0 || tile.x >= seed.width || tile.y >= seed.height) errors.push(`tile:${tileKey}:out_of_bounds`);
    if (seen.has(tileKey)) errors.push(`tile:${tileKey}:duplicate`);
    seen.add(tileKey);
    if (tile.surface === 'Bridge' && tile.base !== 'Water' && tile.base !== 'Riverbank') errors.push(`tile:${tileKey}:bridge_without_water`);
    if (tile.cover === 'Building' && tile.base === 'Water') errors.push(`tile:${tileKey}:building_on_water`);
    if ((tile.cover === 'Building' || tile.cover === 'Tree') && tile.surface !== 'None') errors.push(`tile:${tileKey}:cover_on_transport_surface`);
    if (tile.road_mask !== null && tile.surface !== 'Street' && tile.surface !== 'Bridge' && tile.surface !== 'RailCrossing') errors.push(`tile:${tileKey}:road_mask_without_road_surface`);
    if (tile.rail_mask !== null && tile.surface !== 'Rail' && tile.surface !== 'RailCrossing') errors.push(`tile:${tileKey}:rail_mask_without_rail_surface`);
    if ((tile.surface === 'Street' || tile.surface === 'Bridge' || tile.surface === 'RailCrossing') && tile.road_mask === null) errors.push(`tile:${tileKey}:road_surface_without_road_mask`);
    if ((tile.surface === 'Rail' || tile.surface === 'RailCrossing') && tile.rail_mask === null) errors.push(`tile:${tileKey}:rail_surface_without_rail_mask`);
  }

  return errors;
}

function baseFor(kind: ZurichTerrainKind): LayeredBaseKind {
  const mapping: Record<ZurichTerrainKind, LayeredBaseKind> = {
    grass: 'Grass',
    water: 'Water',
    riverbank: 'Riverbank',
    forest: 'Forest',
    park: 'Park',
    reserve: 'Reserve',
    plaza: 'Plaza',
  };
  return mapping[kind];
}

function surfaceFor(input: { roadKind?: 'street' | 'bridge'; hasRail: boolean; isRailCrossing: boolean }): LayeredSurfaceKind {
  if (input.isRailCrossing) return 'RailCrossing';
  if (input.roadKind === 'bridge') return 'Bridge';
  if (input.roadKind === 'street') return 'Street';
  if (input.hasRail) return 'Rail';
  return 'None';
}

function coverFor(input: {
  building?: ZurichBuilding;
  hasTree: boolean;
  detail?: ZurichDetail;
  surface: LayeredSurfaceKind;
}): LayeredCoverKind {
  if (input.surface !== 'None') return 'None';
  if (input.building) return 'Building';
  if (input.hasTree) return 'Tree';
  if (input.detail) return 'Detail';
  return 'None';
}

function displayFor(input: { building?: ZurichBuilding; detail?: ZurichDetail; cover: LayeredCoverKind }): string | null {
  if (input.cover === 'Building' && input.building) return input.building.sheet;
  if (input.cover === 'Detail' && input.detail) return input.detail.assetCategory;
  return null;
}
