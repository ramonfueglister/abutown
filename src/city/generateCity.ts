import { coordKey } from '../projection';
import type { Building, City, Coord, District, RoadEdge, Tile } from '../types';

const districts: District[] = [
  { id: 'district:0', name: 'Old Town', kind: 'old-town', center: { x: 25, y: 24 }, density: 0.95, centrality: 0.95 },
  { id: 'district:1', name: 'Market Bend', kind: 'market', center: { x: 39, y: 30 }, density: 0.9, centrality: 0.92 },
  { id: 'district:2', name: 'North Bank', kind: 'residential', center: { x: 22, y: 42 }, density: 0.68, centrality: 0.58 },
  { id: 'district:3', name: 'Civic Hill', kind: 'civic', center: { x: 48, y: 42 }, density: 0.72, centrality: 0.74 },
  { id: 'district:4', name: 'Mill Yard', kind: 'industrial', center: { x: 52, y: 24 }, density: 0.58, centrality: 0.52 },
  { id: 'district:5', name: 'River Park', kind: 'parkland', center: { x: 45, y: 48 }, density: 0.28, centrality: 0.44 },
];

export function generateCity(): City {
  const width = 72;
  const height = 72;
  const roadEdges = makeRoadEdges();
  const roadCoords = new Set(roadEdges.flatMap((edge) => edge.points.map(coordKey)));

  return {
    id: 'abutown-river-polycentric',
    width,
    height,
    districts,
    tiles: makeTiles(width, height),
    roadEdges,
    buildings: makeBuildings(roadCoords),
  };
}

function makeTiles(width: number, height: number): Tile[] {
  const tiles: Tile[] = [];
  for (let y = 0; y < height; y += 1) {
    const riverX = riverCenterX(y);
    for (let x = 0; x < width; x += 1) {
      const nearest = nearestDistrict({ x, y });
      const riverDistance = Math.abs(x - riverX);
      tiles.push({
        coord: { x, y },
        terrain:
          riverDistance <= 2
            ? 'water'
            : riverDistance === 3
              ? 'riverbank'
              : nearest.kind === 'parkland' && distance({ x, y }, nearest.center) <= 10
                ? 'park'
                : ['market', 'old-town', 'civic'].includes(nearest.kind) && distance({ x, y }, nearest.center) <= 2
                  ? 'plaza'
                  : 'grass',
      });
    }
  }
  return tiles;
}

function makeRoadEdges(): RoadEdge[] {
  const routes: Array<{ hierarchy: RoadEdge['hierarchy']; points: Coord[] }> = [
    { hierarchy: 'primary', points: route([{ x: 0, y: 38 }, { x: 12, y: 38 }, { x: 22, y: 42 }, { x: 25, y: 24 }, { x: 30, y: 26 }, { x: 36, y: 26 }, { x: 39, y: 30 }, { x: 52, y: 24 }, { x: 71, y: 34 }]) },
    { hierarchy: 'primary', points: route([{ x: 28, y: 0 }, { x: 28, y: 14 }, { x: 25, y: 24 }, { x: 39, y: 30 }, { x: 48, y: 42 }, { x: 46, y: 71 }]) },
    { hierarchy: 'secondary', points: route([{ x: 22, y: 42 }, { x: 27, y: 42 }, { x: 33, y: 42 }, { x: 45, y: 48 }, { x: 48, y: 42 }]) },
    { hierarchy: 'secondary', points: route([{ x: 39, y: 30 }, { x: 43, y: 34 }, { x: 48, y: 42 }, { x: 52, y: 24 }]) },
    { hierarchy: 'secondary', points: route([{ x: 25, y: 24 }, { x: 18, y: 20 }, { x: 18, y: 36 }, { x: 22, y: 42 }]) },
  ];

  const localRoutes = districts.flatMap((district) => localGrid(district.center, district.kind === 'industrial' || district.kind === 'parkland' ? 5 : 4));
  return [...routes, ...localRoutes].map((route, index) => ({
    id: `roadEdge:${index}`,
    hierarchy: route.hierarchy,
    points: route.points,
    modes: route.hierarchy === 'local' ? ['car', 'pedestrian'] : ['car', 'pedestrian', 'service'],
  }));
}

function localGrid(center: Coord, radius: number): Array<{ hierarchy: RoadEdge['hierarchy']; points: Coord[] }> {
  const routes: Array<{ hierarchy: RoadEdge['hierarchy']; points: Coord[] }> = [];
  for (let offset = -radius; offset <= radius; offset += Math.max(2, Math.floor(radius / 2))) {
    routes.push({ hierarchy: 'local', points: route([{ x: center.x - radius, y: center.y + offset }, { x: center.x + radius, y: center.y + offset }]) });
    routes.push({ hierarchy: 'local', points: route([{ x: center.x + offset, y: center.y - radius }, { x: center.x + offset, y: center.y + radius }]) });
  }
  return routes;
}

function makeBuildings(roadCoords: Set<string>): Building[] {
  const buildings: Building[] = [];
  for (const district of districts) {
    const radius = district.kind === 'parkland' ? 6 : district.kind === 'industrial' ? 7 : 5;
    for (let y = district.center.y - radius; y <= district.center.y + radius; y += 2) {
      for (let x = district.center.x - radius; x <= district.center.x + radius; x += 2) {
        const coord = { x, y };
        if (roadCoords.has(coordKey(coord)) || Math.abs(x - riverCenterX(y)) <= 3 || distance(coord, district.center) > radius + 1) continue;
        if ((hash(`${district.id}:${x}:${y}`) % 100) / 100 > district.density) continue;
        buildings.push({
          id: `building:${buildings.length}`,
          coord,
          width: 1 + (hash(`w:${x}:${y}`) % 2),
          height: district.kind === 'market' || district.kind === 'civic' ? 3 : district.kind === 'industrial' ? 2 : 1 + (hash(`h:${x}:${y}`) % 2),
          kind: district.kind === 'old-town' || district.kind === 'residential' ? 'residential' : district.kind === 'parkland' ? 'park' : district.kind === 'market' ? 'commercial' : district.kind,
        });
      }
    }
  }
  return buildings;
}

function route(waypoints: Coord[]): Coord[] {
  const points: Coord[] = [];
  for (let index = 1; index < waypoints.length; index += 1) {
    const segment = line(waypoints[index - 1], waypoints[index]);
    points.push(...(index === 1 ? segment : segment.slice(1)));
  }
  return points;
}

function line(from: Coord, to: Coord): Coord[] {
  const points = [from];
  let current = from;
  while (current.x !== to.x) {
    current = { x: current.x + Math.sign(to.x - current.x), y: current.y };
    points.push(current);
  }
  while (current.y !== to.y) {
    current = { x: current.x, y: current.y + Math.sign(to.y - current.y) };
    points.push(current);
  }
  return points;
}

function riverCenterX(y: number): number {
  return 34 + Math.round(Math.sin(y / 8) * 5);
}

function nearestDistrict(coord: Coord): District {
  return districts.reduce((best, district) => (distance(coord, district.center) < distance(coord, best.center) ? district : best), districts[0]);
}

function distance(a: Coord, b: Coord): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

function hash(value: string): number {
  let result = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    result ^= value.charCodeAt(index);
    result = Math.imul(result, 16777619);
  }
  return result >>> 0;
}
