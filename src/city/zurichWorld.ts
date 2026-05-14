import { distance, key, type Coord, type ZurichTerrainKind, type ZurichTerrainTile, type ZurichWorld, type ZurichZone } from './worldTypes';

export type ZurichWorldOptions = {
  seed: number;
};

const WIDTH = 256;
const HEIGHT = 256;
const CHUNK_SIZE = 32;

const layoutZones: ZurichZone[] = [
  { id: 'zone:limmat-river', kind: 'river', name: 'Limmat River', center: { x: 128, y: 128 }, radius: 36, density: 0 },
  { id: 'zone:old-town-west', kind: 'old-town', name: 'Old Town West', center: { x: 112, y: 112 }, radius: 22, density: 0.95 },
  { id: 'zone:old-town-east', kind: 'old-town', name: 'Old Town East', center: { x: 139, y: 112 }, radius: 20, density: 0.92 },
  { id: 'zone:main-station', kind: 'rail-center', name: 'Main Station Quarter', center: { x: 118, y: 145 }, radius: 26, density: 0.9 },
  { id: 'zone:civic', kind: 'civic', name: 'Civic Garden', center: { x: 151, y: 143 }, radius: 18, density: 0.66 },
  { id: 'zone:west-residential', kind: 'residential', name: 'West Residential', center: { x: 74, y: 125 }, radius: 31, density: 0.62 },
  { id: 'zone:north-residential', kind: 'residential', name: 'North Residential', center: { x: 129, y: 72 }, radius: 30, density: 0.58 },
  { id: 'zone:east-residential', kind: 'residential', name: 'East Residential', center: { x: 182, y: 119 }, radius: 34, density: 0.6 },
  { id: 'zone:south-village', kind: 'residential', name: 'South Village', center: { x: 100, y: 196 }, radius: 30, density: 0.48 },
  { id: 'zone:industry', kind: 'industry', name: 'Rail Industry Edge', center: { x: 175, y: 184 }, radius: 28, density: 0.54 },
  { id: 'zone:north-forest', kind: 'forest', name: 'North Forest', center: { x: 58, y: 48 }, radius: 45, density: 0.18 },
  { id: 'zone:east-forest', kind: 'forest', name: 'East Forest', center: { x: 220, y: 72 }, radius: 38, density: 0.18 },
  { id: 'zone:south-forest', kind: 'forest', name: 'South Forest', center: { x: 205, y: 222 }, radius: 42, density: 0.18 },
  { id: 'zone:river-park', kind: 'park', name: 'River Park', center: { x: 144, y: 160 }, radius: 22, density: 0.24 },
  { id: 'zone:west-reserve', kind: 'reserve', name: 'West Expansion Reserve', center: { x: 45, y: 184 }, radius: 28, density: 0.12 },
  { id: 'zone:south-reserve', kind: 'reserve', name: 'South Expansion Reserve', center: { x: 142, y: 226 }, radius: 24, density: 0.12 },
];

export function buildZurichWorld(options: ZurichWorldOptions): ZurichWorld {
  const river = buildRiver();
  const riverKeys = new Set(river.map(key));
  const zones = cloneZones(layoutZones);
  const riverZone = zones.find((zone) => zone.kind === 'river');
  const terrain = new Map<string, ZurichTerrainTile>();

  for (let y = 0; y < HEIGHT; y += 1) {
    const riverX = riverCenterX(y);
    for (let x = 0; x < WIDTH; x += 1) {
      const coord = { x, y };
      const zone = riverKeys.has(key(coord)) ? riverZone : nearestZone(coord, zones);
      const riverDistance = Math.abs(x - riverX);
      const kind = terrainKind(coord, riverDistance, riverKeys, zone);
      terrain.set(key(coord), { coord, kind, elevation: 0, zoneId: zone?.id });
    }
  }

  return {
    id: 'zurich-river-city-v1',
    // The seed identifies this world for now; the fixed Zurich layout stays seed-independent until placement tasks use it.
    seed: options.seed,
    width: WIDTH,
    height: HEIGHT,
    chunkSize: CHUNK_SIZE,
    zones,
    terrain,
    river,
  };
}

function cloneZones(source: ZurichZone[]): ZurichZone[] {
  return source.map((zone) => ({ ...zone, center: { ...zone.center } }));
}

function buildRiver(): Coord[] {
  const river: Coord[] = [];
  for (let y = 0; y < HEIGHT; y += 1) {
    const center = riverCenterX(y);
    for (let dx = -4; dx <= 4; dx += 1) river.push({ x: center + dx, y });
  }
  return river;
}

function riverCenterX(y: number): number {
  return 128 + Math.round(Math.sin(y / 23) * 12 + Math.sin(y / 61) * 7);
}

function terrainKind(coord: Coord, riverDistance: number, riverKeys: ReadonlySet<string>, zone?: ZurichZone): ZurichTerrainKind {
  if (riverKeys.has(key(coord))) return 'water';
  if (riverDistance <= 5) return 'riverbank';
  if (zone?.kind === 'forest' && distance(coord, zone.center) <= zone.radius) return 'forest';
  if (zone?.kind === 'reserve' && distance(coord, zone.center) <= zone.radius) return 'reserve';
  if (zone?.kind === 'park' && distance(coord, zone.center) <= zone.radius) return 'park';
  if ((zone?.kind === 'old-town' || zone?.kind === 'civic') && distance(coord, zone.center) < 4) return 'plaza';
  return 'grass';
}

function nearestZone(coord: Coord, zones: ZurichZone[]): ZurichZone | undefined {
  return zones.filter((zone) => zone.kind !== 'river').reduce<ZurichZone | undefined>((best, zone) => {
    if (!best) return zone;
    return distance(coord, zone.center) / zone.radius < distance(coord, best.center) / best.radius ? zone : best;
  }, undefined);
}
