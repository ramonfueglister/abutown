import { distance, inside, key, type Coord, type ZurichBuilding, type ZurichBuildingSheet, type ZurichDetail, type ZurichTerrainKind, type ZurichWorld, type ZurichZone } from './worldTypes';
import type { ZurichTransport } from './zurichTransport';

export type ZurichPlacement = {
  buildings: ZurichBuilding[];
  trees: Coord[];
  details: ZurichDetail[];
  reserveTiles: Set<string>;
};

const sheetPools: Record<ZurichZone['kind'], ZurichBuildingSheet[]> = {
  river: ['townhouses'],
  'old-town': ['oldhouses', 'townhouses', 'shops', 'church'],
  'rail-center': ['shops', 'flats', 'office', 'townhouses'],
  residential: ['houses', 'cottages', 'townhouses'],
  forest: ['cottages'],
  park: ['church', 'cottages', 'oldhouses'],
  industry: ['shops', 'office', 'flats'],
  reserve: ['cottages', 'houses'],
  civic: ['church', 'office', 'shops'],
  waterfront: ['townhouses', 'shops', 'flats'],
};

const PRIMARY_FRONTAGE_OFFSETS: Coord[] = [
  { x: -1, y: 0 },
  { x: 0, y: -1 },
];
const BACKFILL_FRONTAGE_OFFSETS: Coord[] = [
  { x: -1, y: 0 },
  { x: 0, y: -1 },
];
const MIN_BUILDING_FOOTPRINTS = 2260;

export function buildZurichPlacement(world: ZurichWorld, transport: ZurichTransport): ZurichPlacement {
  const blocked = new Set<string>([...transport.roads.keys(), ...transport.rails.keys()]);
  const buildings: ZurichBuilding[] = [];
  const trees: Coord[] = [];
  const details: ZurichDetail[] = [];
  const reserveTiles = new Set<string>();

  for (const tile of world.terrain.values()) {
    if (tile.kind === 'reserve') reserveTiles.add(key(tile.coord));
    const tileKey = key(tile.coord);
    if (tile.kind === 'forest' && isForestTreeTile(tile.coord) && !blocked.has(tileKey)) {
      trees.push(tile.coord);
      blocked.add(tileKey);
    }
    if (tile.kind === 'park' && hash(`park-tree:${tileKey}`) % 4 === 0 && !blocked.has(tileKey)) {
      trees.push(tile.coord);
      blocked.add(tileKey);
    }
  }

  for (const zone of world.zones) {
    if (zone.kind === 'forest' || zone.kind === 'river') continue;
    const candidates = frontageCandidates(world, transport, blocked, zone);
    for (const coord of candidates) {
      if (buildings.length >= 4200) break;
      if (blocked.has(key(coord))) continue;
      if (zone.kind === 'reserve') {
        if (hash(`reserve-building:${key(coord)}`) % 5 !== 0) continue;
      } else if (hash(`building-density:${zone.id}:${key(coord)}`) % 100 > Math.floor(effectiveBuildingDensity(zone) * 100)) continue;
      if (zone.kind === 'residential' && distance(coord, zone.center) > zone.radius * 0.72 && hash(`residential-edge:${zone.id}:${key(coord)}`) % 100 < 35) continue;
      pushBuilding(buildings, blocked, coord, zone);
    }
  }

  addDioramaDetails(world, transport, details, blocked);

  for (const zone of world.zones) {
    if (zone.kind === 'civic' || zone.kind === 'park' || zone.kind === 'industry') {
      const detailTarget = zone.kind === 'industry' ? 105 : zone.kind === 'civic' ? 72 : 48;
      let placed = 0;
      for (const coord of detailFrontageCandidates(world, transport, blocked, zone)) {
        if (placed >= detailTarget) break;
        pushDetail(
          world,
          details,
          blocked,
          coord,
          zone.kind === 'industry' ? 'industry' : zone.kind === 'civic' ? 'civic' : 'park',
          zone.kind === 'industry' ? 'industry' : 'decor',
          false,
        );
        placed += 1;
      }
    }
  }

  backfillBuildings(world, transport, buildings, blocked);

  return { buildings, trees, details, reserveTiles };
}

function addDioramaDetails(world: ZurichWorld, transport: ZurichTransport, details: ZurichDetail[], blocked: Set<string>): void {
  addIndustrySetpiece(world, transport, details, blocked);
  addFieldSetpiece(world, details, blocked);
}

function addIndustrySetpiece(world: ZurichWorld, transport: ZurichTransport, details: ZurichDetail[], blocked: Set<string>): void {
  const industry = world.zones.find((zone) => zone.kind === 'industry');
  if (!industry) return;

  let placed = 0;
  for (const coord of detailFrontageCandidates(world, transport, blocked, industry)) {
    if (placed >= 90) break;
    const asset = placed % 2 === 0 ? 'factory' : 'road-depot';
    pushDetail(world, details, blocked, coord, 'industry', asset, false);
    placed += 1;
  }
}

function effectiveBuildingDensity(zone: ZurichZone): number {
  if (zone.kind === 'old-town') return 0.99;
  if (zone.kind === 'residential') return Math.min(0.99, zone.density + 0.38);
  if (zone.kind === 'industry' || zone.kind === 'civic') return Math.min(0.98, zone.density + 0.38);
  if (zone.kind === 'park') return Math.min(0.75, zone.density + 0.36);
  if (zone.kind === 'rail-center') return 1;
  return zone.density;
}

function addFieldSetpiece(world: ZurichWorld, details: ZurichDetail[], blocked: Set<string>): void {
  for (const zone of world.zones.filter((candidate) => candidate.kind === 'reserve')) {
    for (let y = zone.center.y - zone.radius + 2; y <= zone.center.y + zone.radius - 2; y += 3) {
      for (let x = zone.center.x - zone.radius + 2; x <= zone.center.x + zone.radius - 2; x += 3) {
        const coord = { x, y };
        if (distance(coord, zone.center) > zone.radius || hash(`field:${zone.id}:${key(coord)}`) % 100 > 58) continue;
        pushDetail(world, details, blocked, coord, 'field', 'farm-field', false);
      }
    }
  }
}

function pushDetail(
  world: ZurichWorld,
  details: ZurichDetail[],
  blocked: Set<string>,
  coord: Coord,
  category: ZurichDetail['category'],
  assetCategory: string,
  allowBlocked: boolean,
): void {
  const tileKey = key(coord);
  const terrain = world.terrain.get(tileKey)?.kind;
  if (!inside(coord, world.width, world.height) || !terrain) return;
  if (terrain === 'water' || terrain === 'riverbank') return;
  if (!allowBlocked && blocked.has(tileKey)) return;
  if (!allowBlocked && terrain === 'forest') return;
  details.push({ coord, category, assetCategory });
  if (!allowBlocked) blocked.add(tileKey);
}

function frontageCandidates(
  world: ZurichWorld,
  transport: ZurichTransport,
  blocked: ReadonlySet<string>,
  zone: ZurichZone,
  offsets: readonly Coord[] = PRIMARY_FRONTAGE_OFFSETS,
): Coord[] {
  const candidates: Coord[] = [];
  const seen = new Set<string>();

  for (const road of transport.roads.values()) {
    if (road.kind !== 'street') continue;
    for (const offset of offsets) {
      const coord = { x: road.coord.x + offset.x, y: road.coord.y + offset.y };
      const tileKey = key(coord);
      const terrain = world.terrain.get(tileKey)?.kind;
      if (!inside(coord, world.width, world.height) || blocked.has(tileKey) || seen.has(tileKey)) continue;
      if (!terrain || !isBuildingBaseTerrain(terrain)) continue;
      if (zone.kind !== 'old-town' && zone.kind !== 'waterfront' && distanceToWater(world, coord, 2) <= 2) continue;
      if (distance(coord, zone.center) <= zone.radius) {
        candidates.push(coord);
        seen.add(tileKey);
      }
    }
  }
  candidates.sort((a, b) => distance(a, zone.center) - distance(b, zone.center) || a.y - b.y || a.x - b.x);
  return candidates;
}

function detailFrontageCandidates(world: ZurichWorld, transport: ZurichTransport, blocked: ReadonlySet<string>, zone: ZurichZone): Coord[] {
  return frontageCandidates(world, transport, blocked, zone)
    .sort((a, b) => hash(`detail-order:${zone.id}:${key(a)}`) - hash(`detail-order:${zone.id}:${key(b)}`));
}

function backfillBuildings(
  world: ZurichWorld,
  transport: ZurichTransport,
  buildings: ZurichBuilding[],
  blocked: Set<string>,
): void {
  if (buildings.length >= MIN_BUILDING_FOOTPRINTS) return;

  const candidates: Array<{ coord: Coord; zone: ZurichZone }> = [];
  for (const zone of world.zones) {
    if (zone.kind === 'forest' || zone.kind === 'river' || zone.kind === 'reserve') continue;
    for (const coord of frontageCandidates(world, transport, blocked, zone, BACKFILL_FRONTAGE_OFFSETS)) {
      candidates.push({ coord, zone });
    }
  }

  candidates.sort((a, b) =>
    hash(`building-backfill:${a.zone.id}:${key(a.coord)}`) -
      hash(`building-backfill:${b.zone.id}:${key(b.coord)}`) ||
    distance(a.coord, a.zone.center) - distance(b.coord, b.zone.center) ||
    a.coord.y - b.coord.y ||
    a.coord.x - b.coord.x
  );

  for (const { coord, zone } of candidates) {
    if (buildings.length >= MIN_BUILDING_FOOTPRINTS) break;
    if (blocked.has(key(coord))) continue;
    pushBuilding(buildings, blocked, coord, zone);
  }
}

function pushBuilding(
  buildings: ZurichBuilding[],
  blocked: Set<string>,
  coord: Coord,
  zone: ZurichZone,
): void {
  const sheets = sheetPools[zone.kind];
  const sheet = sheets[hash(`sheet:${zone.id}:${key(coord)}`) % sheets.length];
  buildings.push({ coord, sheet, frame: hash(`frame:${sheet}:${key(coord)}`) % frameCount(sheet), zoneId: zone.id });
  blocked.add(key(coord));
}

function isBuildingBaseTerrain(kind: ZurichTerrainKind): boolean {
  return kind === 'grass' || kind === 'park' || kind === 'reserve';
}

function isForestTreeTile(coord: Coord): boolean {
  const tileKey = key(coord);
  const local = hash(`forest-local:${tileKey}`) % 100;
  const pocketA = hash(`forest-pocket-a:${Math.floor(coord.x / 8)}:${Math.floor(coord.y / 8)}`) % 100;
  const pocketB = hash(`forest-pocket-b:${Math.floor((coord.x + 5) / 13)}:${Math.floor((coord.y + 3) / 13)}`) % 100;
  if (pocketA < 12) return local < 34;
  if (pocketB > 78) return local < 55;
  return local < 18;
}

function distanceToWater(world: ZurichWorld, coord: Coord, maxDistance: number): number {
  for (let radius = 1; radius <= maxDistance; radius += 1) {
    for (let dy = -radius; dy <= radius; dy += 1) {
      const dx = radius - Math.abs(dy);
      if (world.terrain.get(key({ x: coord.x + dx, y: coord.y + dy }))?.kind === 'water') return radius;
      if (dx !== 0 && world.terrain.get(key({ x: coord.x - dx, y: coord.y + dy }))?.kind === 'water') return radius;
    }
  }
  return maxDistance + 1;
}

function frameCount(sheet: ZurichBuildingSheet): number {
  if (sheet === 'church' || sheet === 'cottages') return 1;
  if (sheet === 'townhouses' || sheet === 'modern') return 2;
  if (sheet === 'flats') return 3;
  if (sheet === 'houses' || sheet === 'oldhouses' || sheet === 'office' || sheet === 'tower') return 4;
  return 6;
}

function hash(value: string): number {
  let result = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    result ^= value.charCodeAt(index);
    result = Math.imul(result, 16777619);
  }
  return result >>> 0;
}
