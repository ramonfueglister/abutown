import type { BackendHealthDto } from '../backend/backendGate';
import type { TerrainBaseKind, TerrainState } from '../backend/terrainState';
import type { MobilityDiagnostics } from '../backend/mobilityState';
import type { Coord } from '../types';
import type {
  Building,
  Detail,
  RailStation,
  RailTile,
  RoadTile,
} from './backendTerrainRenderState';
import type {
  BackendCar,
  BackendPedestrian,
  SimutransPedestrianSpriteLike,
  VehicleSpriteLike,
} from './backendMobilityDrawables';
import type { EntityInspector } from './entityInspector';
import { buildBackendCarInspector, buildBackendPedestrianInspector } from './entityInspector';
import type { CityDiagnostics } from './cityDiagnostics';

export type RenderGameTextInput = {
  worldId: string;
  visualStyleId: string;
  tileSize: { width: number; height: number };
  nonPak128AssetPaths: string[];
  width: number;
  height: number;
  terrainState: TerrainState;
  roads: ReadonlyMap<string, RoadTile>;
  rails: ReadonlyMap<string, RailTile>;
  railCrossings: ReadonlySet<string>;
  railPaths: readonly Coord[][];
  railStations: readonly RailStation[];
  buildings: readonly Building[];
  trees: readonly Coord[];
  details: readonly Detail[];
  trains: readonly unknown[];
  projectedPedestrians: readonly BackendPedestrian[];
  projectedCars: readonly BackendCar[];
  pedestrianSprites: readonly SimutransPedestrianSpriteLike[];
  vehicleSprites: readonly VehicleSpriteLike[];
  selectedAgentId: string | null;
  selectedVehicleId: string | null;
  backendBaseUrl: string;
  backendStatus: BackendHealthDto | null;
  backendMobility: MobilityDiagnostics;
  diagnostics: CityDiagnostics;
  camera: {
    current: { x: number; y: number; scale: number };
    target: { x: number; y: number; scale: number };
    dragging: boolean;
    bounds: { minX: number; maxX: number; minY: number; maxY: number };
    edgeTreatment: { outskirtsTiles: number; exitTiles: number };
  };
  entityScreenPosition: (coord: Coord) => Coord;
  trainSummary: {
    position: Coord;
    alpha: number;
    speed: number;
    fadeTiles: number;
    direction: 'northbound';
  } | null;
};

export function buildRenderGameText(input: RenderGameTextInput): string {
  const selectedAgent = input.projectedPedestrians.find((agent) => agent.id === input.selectedAgentId) ?? null;
  const selectedVehicle = input.projectedCars.find((vehicle) => vehicle.id === input.selectedVehicleId) ?? null;
  const mobilityAgentEntries = input.projectedPedestrians.map((agent) => ({
    id: agent.id,
    kind: 'pedestrian' as const,
    state: 'walking' as const,
    coord: agent.path[0],
    screen: input.entityScreenPosition(agent.path[0]),
    direction: agent.direction,
    spriteSheet: agent.sprite.sheet,
  }));
  const mobilityVehicleEntries = input.projectedCars.map((vehicle) => ({
    id: vehicle.id,
    kind: 'car' as const,
    state: 'driving' as const,
    coord: vehicle.path[0],
    screen: input.entityScreenPosition(vehicle.path[0]),
    direction: vehicle.direction,
    spriteSheet: vehicle.sprite.sheet,
  }));
  const selectedMobilityAgentEntry = selectedAgent
    ? mobilityAgentEntries.find((entry) => entry.id === selectedAgent.id) ?? null
    : null;
  const selectedMobilityVehicleEntry = selectedVehicle
    ? mobilityVehicleEntries.find((entry) => entry.id === selectedVehicle.id) ?? null
    : null;

  return JSON.stringify({
    coordinateSystem: 'grid origin north-west, x east, y south, top-down minimal map projection',
    city: {
      worldId: input.worldId,
      terrainSource: 'backend-layered',
      layeredTerrain: {
        loadedTiles: input.terrainState.tiles.size,
        loadedChunks: input.terrainState.loadedChunks.size,
      },
      visualStyle: {
        id: input.visualStyleId,
        renderer: 'canvas-vector',
        spriteDrawing: 'disabled',
      },
      assetPack: {
        id: 'minimal-vector',
        tile: input.tileSize,
      },
      nonPak128AssetPaths: input.nonPak128AssetPaths,
      width: input.width,
      height: input.height,
      roadTiles: input.roads.size,
      railTiles: input.rails.size,
      bridges: [...input.roads.values()].filter((road) => road.kind === 'bridge').length,
      buildings: input.buildings.length,
      trees: input.trees.length,
      cars: input.projectedCars.length,
      trains: input.trains.length,
      train: input.trainSummary,
      pedestrians: input.projectedPedestrians.length,
      pedestrianSprites: input.pedestrianSprites.length,
      pedestrianSpriteSheets: [...new Set(input.pedestrianSprites.map((sprite) => sprite.sheet))],
      vehicleSprites: input.vehicleSprites.length,
      vehicleSheets: [...new Set(input.vehicleSprites.map((sprite) => sprite.sheet))],
      backend: {
        required: true,
        baseUrl: input.backendBaseUrl,
        status: input.backendStatus,
      },
      mobility: {
        source: 'backend',
        status: input.backendMobility.status,
        tick: input.backendMobility.tick,
        agents: input.backendMobility.agents,
        vehicles: input.backendMobility.vehicles,
        stops: input.backendMobility.stops,
        invalidMessages: input.backendMobility.invalidMessages,
        lastError: input.backendMobility.lastError,
      },
      mobilityAgents: {
        count: mobilityAgentEntries.length,
        selectedId: input.selectedAgentId,
        selected: selectedMobilityAgentEntry,
        agents: mobilityAgentEntries,
      },
      mobilityVehicles: {
        count: mobilityVehicleEntries.length,
        selectedId: input.selectedVehicleId,
        selected: selectedMobilityVehicleEntry,
        vehicles: mobilityVehicleEntries,
      },
      agentInspector: buildBackendPedestrianInspector(selectedAgent) satisfies EntityInspector,
      vehicleInspector: buildBackendCarInspector(selectedVehicle) satisfies EntityInspector,
      railStations: input.railStations.length,
      railYardTracks: Math.max(0, input.railPaths.length - 2),
      details: detailCountsByCategory(input.details),
      reserveTiles: countTerrainBase(input.terrainState, 'Reserve'),
      validationErrors: 0,
      roadRailOverlap: input.diagnostics.roadRailOverlap,
      railCrossings: input.railCrossings.size,
      invalidBuildings: input.diagnostics.invalidBuildings,
      treeBuildingOverlap: input.diagnostics.treeBuildingOverlap,
      railStationsOnRoad: input.diagnostics.railStationsOnRoad,
      railStationsOnBuildings: input.diagnostics.railStationsOnBuildings,
      railStationsOnRails: input.diagnostics.railStationsOnRails,
      railStationsOnTrees: input.diagnostics.railStationsOnTrees,
      diagnostics: input.diagnostics,
      camera: {
        mode: 'bounded-fixed-map',
        current: input.camera.current,
        target: input.camera.target,
        dragging: input.camera.dragging,
        bounds: input.camera.bounds,
        edgeTreatment: input.camera.edgeTreatment,
      },
    },
  });
}

export function detailCountsByCategory(details: readonly Detail[]): Record<string, number> {
  const result: Record<string, number> = { total: details.length };
  for (const detail of details) {
    result[detail.category] = (result[detail.category] ?? 0) + 1;
  }
  return result;
}

export function countTerrainBase(terrainState: TerrainState, base: TerrainBaseKind): number {
  let count = 0;
  for (const tile of terrainState.tiles.values()) {
    if (tile.base === base) count += 1;
  }
  return count;
}

export function nonPak128AssetPaths(paths: Iterable<string>): string[] {
  return [...paths].filter((path) => !path.startsWith('/simutrans-assets/pak128/')).sort();
}
