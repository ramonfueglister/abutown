export type Coord = {
  x: number;
  y: number;
};

export type RoadHierarchy = 'primary' | 'secondary' | 'local';
export type RoadMode = 'car' | 'pedestrian' | 'service';

export type RoadEdge = {
  id: string;
  points: Coord[];
  hierarchy: RoadHierarchy;
  modes: RoadMode[];
};

export type District = {
  id: string;
  name: string;
  kind: 'old-town' | 'market' | 'residential' | 'civic' | 'industrial' | 'parkland';
  center: Coord;
  density: number;
  centrality: number;
};

export type Tile = {
  coord: Coord;
  terrain: 'water' | 'riverbank' | 'grass' | 'park' | 'plaza';
};

export type Building = {
  id: string;
  coord: Coord;
  width: number;
  height: number;
  kind: 'residential' | 'commercial' | 'civic' | 'industrial' | 'park';
};

export type City = {
  id: string;
  width: number;
  height: number;
  districts: District[];
  tiles: Tile[];
  roadEdges: RoadEdge[];
  buildings: Building[];
};
