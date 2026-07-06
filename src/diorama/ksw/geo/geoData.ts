// src/diorama/ksw/geo/geoData.ts
// Typed access to the baked Winterthur artifacts (data/winterthur/*.json,
// produced by scripts/geo/bake-winterthur.mjs). Static imports — the data
// ships with the bundle, no fetch, no fallback. The ksw zone is split out:
// those footprints belong to the hero diorama, the city renders the rest.
import buildingsJson from '../../../../data/winterthur/buildings.json';
import metaJson from '../../../../data/winterthur/meta.json';
import natureJson from '../../../../data/winterthur/nature.json';
import roadsJson from '../../../../data/winterthur/roads.json';

export type BakedMesh = { pos: number[]; idx: number[] };
// Wall mesh carries a facade-UV attribute (Task 13): fuv = 2 dm-ints per vertex
// (u = horizontal metres·10 along the facet, v = height·10), aligned to pos.
export type BakedWallMesh = { pos: number[]; idx: number[]; fuv: number[] };
export type BakedDoor = { x: number; z: number; yaw: number };
export type BakedBuilding = {
  id: string;
  name?: string;
  usage?: string;
  zone: 'ksw' | 'city';
  // Enrichment (geo:bake-attributes): ÖREB Grundnutzung + GWR. Null = no join
  // hit (e.g. shed without EGID, footprint outside a Bauzone) — shown as such,
  // never guessed.
  egid?: number | null;
  gwrCategory?: string | null;
  bauzone?: string | null;
  bauzoneCode?: string | null;
  footprint: number[][];
  height: number;
  // Real eave height (m, 1 decimal) — the facade shader clamps windows below it.
  eaveH: number;
  wall: BakedWallMesh;
  roof: BakedMesh;
  door?: BakedDoor;
};
export type RoadPath = { class: string; width: number; pts: number[][] };
export type CityMeta = {
  plate: { cx: number; cz: number; w: number; d: number };
  landmarks: Record<string, number[]>;
  counts: Record<string, number>;
  attribution: string[];
};

export type GreenArea = { kind: string; ring: number[][] };
export type WaterArea = { ring: number[][] };
export type RiverPath = { width: number; pts: number[][] };
export type TreeFamily = 'spreading' | 'oval' | 'tall' | 'conic' | 'slender';
export type TreeSpec = { x: number; z: number; h: number; r: number; kind: 'broad' | 'conifer'; family?: TreeFamily };
export type CityNature = { greens: GreenArea[]; waterAreas: WaterArea[]; rivers: RiverPath[]; trees: TreeSpec[] };

const all = (buildingsJson as { buildings: BakedBuilding[] }).buildings;

export const cityMeta = metaJson as unknown as CityMeta;
export const cityBuildings: BakedBuilding[] = all.filter((b) => b.zone === 'city');
export const kswBuildings: BakedBuilding[] = all.filter((b) => b.zone === 'ksw');
export const cityRoads: RoadPath[] = (roadsJson as { roads: RoadPath[] }).roads;
export const cityRails: RoadPath[] = (roadsJson as { rails: RoadPath[] }).rails;
export const cityNature = natureJson as CityNature;
