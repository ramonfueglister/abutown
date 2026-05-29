import type { BuildingSheet, Coord } from '../city/worldTypes';

export type RuntimeTerrain = 'grass' | 'water' | 'riverbank' | 'park';
export type RuntimeRoadKind = 'street' | 'bridge';
export type RuntimeRoadTile = { coord: Coord; kind: RuntimeRoadKind; mask: number };
export type RuntimeRailTile = { coord: Coord; mask: number };
export type RuntimeRailStation = { coord: Coord; frame: number };
export type RuntimeBuilding = { coord: Coord; sheet: BuildingSheet; frame: number; district: string };
