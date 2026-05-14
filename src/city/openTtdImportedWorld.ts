import { openTtdImportedMap } from './openTtdHamburg.generated';
import { key, type Coord, type ZurichBuilding, type ZurichDetail, type ZurichTerrainKind, type ZurichTerrainTile, type ZurichWorld, type ZurichZone } from './worldTypes';
import type { ZurichPlacement } from './zurichPlacement';
import type { ZurichTransport } from './zurichTransport';

type TerrainRun = [kindIndex: number, length: number];
type RoadTuple = [x: number, y: number, mask: number, bridge: number];
type RailTuple = [x: number, y: number, mask: number];
type BuildingTuple = [x: number, y: number, sheetIndex: number, frame: number];
type DetailTuple = [x: number, y: number, categoryIndex: number, assetIndex: number];
type CoordTuple = [x: number, y: number];

type ImportedMapData = {
  id: string;
  source: string;
  sourceWidth: number;
  sourceHeight: number;
  width: number;
  height: number;
  terrainKinds: ZurichTerrainKind[];
  buildingSheets: ZurichBuilding['sheet'][];
  detailCategories: ZurichDetail['category'][];
  detailAssets: string[];
  terrainRle: TerrainRun[];
  roads: RoadTuple[];
  rails: RailTuple[];
  buildings: BuildingTuple[];
  trees: CoordTuple[];
  details: DetailTuple[];
};

const imported = openTtdImportedMap as ImportedMapData;

export function buildOpenTtdImportedWorld(): ZurichWorld {
  const terrain = new Map<string, ZurichTerrainTile>();
  const zones = buildImportedZones(imported.width, imported.height);
  const river: Coord[] = [];
  let tileIndex = 0;

  for (const [kindIndex, length] of imported.terrainRle) {
    const kind = imported.terrainKinds[kindIndex] ?? 'grass';
    for (let offset = 0; offset < length; offset += 1) {
      const x = tileIndex % imported.width;
      const y = Math.floor(tileIndex / imported.width);
      const coord = { x, y };
      const zone = zoneForKind(kind);
      terrain.set(key(coord), { coord, kind, elevation: 0, zoneId: zone });
      if (kind === 'water') river.push(coord);
      tileIndex += 1;
    }
  }

  if (tileIndex !== imported.width * imported.height) {
    throw new Error(`Imported OpenTTD terrain has ${tileIndex} tiles, expected ${imported.width * imported.height}`);
  }

  return {
    id: imported.id,
    seed: 0,
    width: imported.width,
    height: imported.height,
    chunkSize: 32,
    zones,
    terrain,
    river,
  };
}

export function buildOpenTtdImportedTransport(world: ZurichWorld): ZurichTransport {
  const roads = new Map<string, ZurichTransport['roads'] extends Map<string, infer T> ? T : never>();
  const rails = new Map<string, ZurichTransport['rails'] extends Map<string, infer T> ? T : never>();
  const bridges = new Set<string>();

  for (const [x, y, mask, bridge] of imported.roads) {
    const coord = { x, y };
    const tileKey = key(coord);
    const terrain = world.terrain.get(tileKey)?.kind;
    const isBridge = bridge === 1;
    roads.set(tileKey, { coord, mask, kind: isBridge ? 'bridge' : 'street' });
    if (isBridge && (terrain === 'water' || terrain === 'riverbank')) bridges.add(tileKey);
  }

  for (const [x, y, mask] of imported.rails) {
    const coord = { x, y };
    rails.set(key(coord), { coord, mask });
  }

  return {
    roads,
    rails,
    bridges,
    railCrossings: new Set<string>(),
    arterialPaths: buildRoadCorridors(roads, world),
    railPaths: buildRailCorridors(rails, world),
  };
}

export function buildOpenTtdImportedPlacement(): ZurichPlacement {
  const buildings: ZurichBuilding[] = imported.buildings.map(([x, y, sheetIndex, frame]) => ({
    coord: { x, y },
    sheet: imported.buildingSheets[sheetIndex] ?? 'houses',
    frame,
    zoneId: 'zone:imported-city',
  }));

  const trees = imported.trees.map(([x, y]) => ({ x, y }));

  const details: ZurichDetail[] = imported.details.map(([x, y, categoryIndex, assetIndex]) => ({
    coord: { x, y },
    category: imported.detailCategories[categoryIndex] ?? 'decor',
    assetCategory: imported.detailAssets[assetIndex] ?? 'decor',
  }));

  return {
    buildings,
    trees,
    details,
    reserveTiles: new Set<string>(),
  };
}

function buildImportedZones(width: number, height: number): ZurichZone[] {
  return [
    { id: 'zone:imported-water', kind: 'river', name: 'Imported OpenTTD Water', center: { x: width / 2, y: height / 2 }, radius: width / 3, density: 0 },
    { id: 'zone:imported-city', kind: 'residential', name: 'Imported OpenTTD City', center: { x: width / 2, y: height / 2 }, radius: width / 2, density: 0.8 },
    { id: 'zone:imported-forest', kind: 'forest', name: 'Imported OpenTTD Forest', center: { x: width / 2, y: height / 2 }, radius: width / 2, density: 0.25 },
  ];
}

function zoneForKind(kind: ZurichTerrainKind): string {
  if (kind === 'water' || kind === 'riverbank') return 'zone:imported-water';
  if (kind === 'forest') return 'zone:imported-forest';
  return 'zone:imported-city';
}

function buildRoadCorridors(roads: ZurichTransport['roads'], world: ZurichWorld): Coord[][] {
  const corridors: Coord[][] = [];
  const centerY = Math.floor(world.height / 2);
  const centerX = Math.floor(world.width / 2);
  corridors.push(...longRuns(roads, world.width, world.height, 'row', centerY));
  corridors.push(...longRuns(roads, world.width, world.height, 'column', centerX));
  return corridors.slice(0, 24);
}

function buildRailCorridors(rails: ZurichTransport['rails'], world: ZurichWorld): Coord[][] {
  return [
    ...longRuns(rails, world.width, world.height, 'row', Math.floor(world.height / 2)),
    ...longRuns(rails, world.width, world.height, 'column', Math.floor(world.width / 2)),
  ].slice(0, 12);
}

function longRuns(points: ReadonlyMap<string, unknown>, width: number, height: number, axis: 'row' | 'column', center: number): Coord[][] {
  const runs: Coord[][] = [];
  const outer = axis === 'row' ? height : width;
  const inner = axis === 'row' ? width : height;

  for (let major = 0; major < outer; major += 1) {
    let current: Coord[] = [];
    for (let minor = 0; minor < inner; minor += 1) {
      const coord = axis === 'row' ? { x: minor, y: major } : { x: major, y: minor };
      if (points.has(key(coord))) {
        current.push(coord);
      } else {
        if (current.length >= 10) runs.push(current);
        current = [];
      }
    }
    if (current.length >= 10) runs.push(current);
  }

  return runs.sort((a, b) => {
    const ac = a[Math.floor(a.length / 2)];
    const bc = b[Math.floor(b.length / 2)];
    const aDistance = axis === 'row' ? Math.abs(ac.y - center) : Math.abs(ac.x - center);
    const bDistance = axis === 'row' ? Math.abs(bc.y - center) : Math.abs(bc.x - center);
    return b.length - a.length || aDistance - bDistance;
  });
}
