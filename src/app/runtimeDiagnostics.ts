import { mobilityDiagnostics, trafficDiagnostics, type MobilityOverlayState } from '../backend/mobilityState';
import {
  carsFromMobilityState,
  pedestriansFromMobilityState,
  type BackendCar,
  type BackendPedestrian,
} from '../render/backendMobilityDrawables';
import type { MinimalPedestrianSprite } from '../render/minimalPedestrianSprites';
import type { VehicleSprite } from '../render/vehicleSprites';

export type Coord = { x: number; y: number };

export type RuntimeInspector = { title: string; rows: { label: string; value: string }[] } | null;

export type StaticRuntimeDiagnostics = {
  railStationsOnRoad: number;
  railStationsOnBuildings: number;
  railStationsOnRails: number;
  railStationsOnTrees: number;
  [key: string]: number;
};

type RuntimeBackendDiagnostics = {
  required: true;
  baseUrl: string;
  status: unknown;
};

type RuntimeCameraDiagnostics = {
  current: { x: number; y: number; scale: number };
  target: { x: number; y: number; scale: number };
  dragging: boolean;
  bounds: { minX: number; maxX: number; minY: number; maxY: number };
  edgeTreatment: { outskirtsTiles: number; exitTiles: number };
};

type RuntimeCounts = {
  roadTiles: number;
  railTiles: number;
  bridges: number;
  buildings: number;
  trees: number;
  railStations: number;
  railYardTracks: number;
  reserveTiles: number;
};

type RuntimeValidationDiagnostics = {
  validationErrors: number;
  roadRailOverlap: number;
  railCrossings: number;
  invalidBuildings: number;
  treeBuildingOverlap: number;
};

export type RuntimeDiagnosticsOptions = {
  coordinateSystem: string;
  world: { id: string; width: number; height: number; chunkSize: number };
  visualStyle: { id: string; renderer: 'canvas-vector'; spriteDrawing: 'disabled' };
  visualAssets: { id: string; tile: { width: number; height: number } };
  getBackend: () => RuntimeBackendDiagnostics;
  getMobilityState: () => MobilityOverlayState;
  getMobilityTickPeriodMs: () => number;
  getPedestrianSprites: () => readonly MinimalPedestrianSprite[];
  getVehicleSprites: () => readonly VehicleSprite[];
  getCamera: () => RuntimeCameraDiagnostics;
  getCounts: () => RuntimeCounts;
  getDiagnostics: () => StaticRuntimeDiagnostics;
  getDetails: () => Record<string, number>;
  getValidation: () => RuntimeValidationDiagnostics;
  getSelected: () => {
    agentId: string | null;
    vehicleId: string | null;
    agentInspector: RuntimeInspector;
    vehicleInspector: RuntimeInspector;
  };
  projectEntityScreen: (coord: Coord) => Coord;
  carVisualWorldPoint: (vehicle: BackendCar) => Coord;
  now: () => number;
  advanceTime: (ms: number) => void;
};

export type DiagnosticsWindow = {
  render_game_to_text?: () => string;
  advanceTime?: (ms: number) => void;
};

export function installRuntimeDiagnostics(target: DiagnosticsWindow, options: RuntimeDiagnosticsOptions): void {
  target.render_game_to_text = () => JSON.stringify(buildRuntimeDiagnosticsPayload(options));
  target.advanceTime = (ms) => options.advanceTime(ms);
}

export function buildRuntimeDiagnosticsPayload(options: RuntimeDiagnosticsOptions) {
  const diagnostics = options.getDiagnostics();
  const detailCounts = options.getDetails();
  const mobilityState = options.getMobilityState();
  const backendMobility = mobilityDiagnostics(mobilityState);
  const traffic = trafficDiagnostics(mobilityState);
  const now = options.now();
  const tickPeriodMs = options.getMobilityTickPeriodMs();
  const pedestrianSprites = options.getPedestrianSprites();
  const vehicleSprites = options.getVehicleSprites();
  const projectedPedestrians = pedestriansFromMobilityState(
    mobilityState,
    pedestrianSprites,
    now,
    tickPeriodMs,
  );
  const projectedCars = carsFromMobilityState(
    mobilityState,
    vehicleSprites,
    now,
    tickPeriodMs,
  );
  const selected = options.getSelected();
  const mobilityAgentEntries = projectedPedestrians.map((agent) => mobilityAgentEntry(agent, options));
  const mobilityVehicleEntries = projectedCars.map((vehicle) => mobilityVehicleEntry(vehicle, options));
  const selectedMobilityAgentEntry = selected.agentId === null
    ? null
    : mobilityAgentEntries.find((entry) => entry.id === selected.agentId) ?? null;
  const selectedMobilityVehicleEntry = selected.vehicleId === null
    ? null
    : mobilityVehicleEntries.find((entry) => entry.id === selected.vehicleId) ?? null;
  const counts = options.getCounts();
  const validation = options.getValidation();
  const camera = options.getCamera();

  return {
    coordinateSystem: options.coordinateSystem,
    city: {
      worldId: options.world.id,
      visualStyle: options.visualStyle,
      visualAssets: options.visualAssets,
      loadedRasterAssetPaths: [],
      width: options.world.width,
      height: options.world.height,
      roadTiles: counts.roadTiles,
      railTiles: counts.railTiles,
      bridges: counts.bridges,
      buildings: counts.buildings,
      trees: counts.trees,
      cars: projectedCars.length,
      pedestrians: projectedPedestrians.length,
      pedestrianSprites: pedestrianSprites.length,
      pedestrianSpriteSheets: [...new Set(pedestrianSprites.map((sprite) => sprite.sheet))],
      vehicleSprites: vehicleSprites.length,
      vehicleSheets: [...new Set(vehicleSprites.map((sprite) => sprite.sheet))],
      backend: options.getBackend(),
      mobility: {
        source: 'backend',
        status: backendMobility.status,
        tick: backendMobility.tick,
        agents: backendMobility.agents,
        vehicles: backendMobility.vehicles,
        stops: backendMobility.stops,
        invalidMessages: backendMobility.invalidMessages,
        lastError: backendMobility.lastError,
      },
      traffic,
      mobilityAgents: {
        count: mobilityAgentEntries.length,
        selectedId: selected.agentId,
        selected: selectedMobilityAgentEntry,
        agents: mobilityAgentEntries,
      },
      mobilityVehicles: {
        count: mobilityVehicleEntries.length,
        selectedId: selected.vehicleId,
        selected: selectedMobilityVehicleEntry,
        vehicles: mobilityVehicleEntries,
      },
      agentInspector: selected.agentInspector,
      vehicleInspector: selected.vehicleInspector,
      railStations: counts.railStations,
      railYardTracks: counts.railYardTracks,
      details: detailCounts,
      reserveTiles: counts.reserveTiles,
      validationErrors: validation.validationErrors,
      roadRailOverlap: validation.roadRailOverlap,
      railCrossings: validation.railCrossings,
      invalidBuildings: validation.invalidBuildings,
      treeBuildingOverlap: validation.treeBuildingOverlap,
      railStationsOnRoad: diagnostics.railStationsOnRoad,
      railStationsOnBuildings: diagnostics.railStationsOnBuildings,
      railStationsOnRails: diagnostics.railStationsOnRails,
      railStationsOnTrees: diagnostics.railStationsOnTrees,
      diagnostics,
      camera: {
        mode: 'bounded-fixed-map',
        current: camera.current,
        target: camera.target,
        dragging: camera.dragging,
        bounds: camera.bounds,
        edgeTreatment: camera.edgeTreatment,
      },
    },
  };
}

function mobilityAgentEntry(agent: BackendPedestrian, options: RuntimeDiagnosticsOptions) {
  return {
    id: agent.id,
    kind: 'pedestrian' as const,
    state: 'walking' as const,
    coord: agent.path[0],
    screen: options.projectEntityScreen(agent.path[0]),
    direction: agent.direction,
    spriteSheet: agent.sprite.sheet,
  };
}

function mobilityVehicleEntry(vehicle: BackendCar, options: RuntimeDiagnosticsOptions) {
  return {
    id: vehicle.id,
    kind: 'car' as const,
    state: 'driving' as const,
    coord: vehicle.path[0],
    screen: options.projectEntityScreen(options.carVisualWorldPoint(vehicle)),
    direction: vehicle.direction,
    spriteSheet: vehicle.sprite.sheet,
  };
}
