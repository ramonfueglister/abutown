import { distance, inside, key, type Coord, type ZurichBuilding, type ZurichBuildingSheet, type ZurichDetail, type ZurichWorld, type ZurichZone } from './worldTypes';
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
      if (zone.kind === 'reserve' && hash(`reserve-building:${key(coord)}`) % 9 !== 0) continue;
      if (hash(`building-density:${zone.id}:${key(coord)}`) % 100 > Math.floor(zone.density * 100)) continue;
      if (zone.kind === 'residential' && distance(coord, zone.center) > zone.radius * 0.72 && hash(`residential-edge:${zone.id}:${key(coord)}`) % 100 < 55) continue;
      const sheets = sheetPools[zone.kind];
      const sheet = sheets[hash(`sheet:${zone.id}:${key(coord)}`) % sheets.length];
      buildings.push({ coord, sheet, frame: hash(`frame:${sheet}:${key(coord)}`) % frameCount(sheet), zoneId: zone.id });
      blocked.add(key(coord));
    }
  }

  for (const zone of world.zones) {
    if (zone.kind === 'civic' || zone.kind === 'park' || zone.kind === 'industry') {
      for (let index = 0; index < 80; index += 1) {
        const coord = {
          x: zone.center.x + ((hash(`detail-x:${zone.id}:${index}`) % (zone.radius * 2)) - zone.radius),
          y: zone.center.y + ((hash(`detail-y:${zone.id}:${index}`) % (zone.radius * 2)) - zone.radius),
        };
        const tileKey = key(coord);
        if (!inside(coord, world.width, world.height) || blocked.has(tileKey)) continue;
        details.push({
          coord,
          category: zone.kind === 'industry' ? 'industry' : zone.kind === 'civic' ? 'civic' : 'park',
          assetCategory: zone.kind === 'industry' ? 'industry' : 'decor',
        });
      }
    }
  }

  return { buildings, trees, details, reserveTiles };
}

function frontageCandidates(world: ZurichWorld, transport: ZurichTransport, blocked: ReadonlySet<string>, zone: ZurichZone): Coord[] {
  const candidates: Coord[] = [];
  const seen = new Set<string>();
  const offsets = [
    { x: 1, y: 0 },
    { x: -1, y: 0 },
    { x: 0, y: 1 },
    { x: 0, y: -1 },
    { x: 2, y: 0 },
    { x: -2, y: 0 },
    { x: 0, y: 2 },
    { x: 0, y: -2 },
  ];

  for (const road of transport.roads.values()) {
    if (road.kind !== 'street') continue;
    for (const offset of offsets) {
      const coord = { x: road.coord.x + offset.x, y: road.coord.y + offset.y };
      const tileKey = key(coord);
      const terrain = world.terrain.get(tileKey)?.kind;
      if (!inside(coord, world.width, world.height) || blocked.has(tileKey) || seen.has(tileKey)) continue;
      if (terrain === 'water' || terrain === 'riverbank' || terrain === 'forest') continue;
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

function isForestTreeTile(coord: Coord): boolean {
  const tileKey = key(coord);
  const local = hash(`forest-local:${tileKey}`) % 100;
  const pocketA = hash(`forest-pocket-a:${Math.floor(coord.x / 8)}:${Math.floor(coord.y / 8)}`) % 100;
  const pocketB = hash(`forest-pocket-b:${Math.floor((coord.x + 5) / 13)}:${Math.floor((coord.y + 3) / 13)}`) % 100;
  if (pocketA < 16) return local < 20;
  if (pocketB > 70) return local < 92;
  return local < 62;
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
