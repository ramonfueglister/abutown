import { describe, expect, it, vi } from 'vitest';
import { createMobilityOverlayState } from '../../src/backend/mobilityState';
import {
  buildRuntimeDiagnosticsPayload,
  installRuntimeDiagnostics,
  type RuntimeDiagnosticsOptions,
} from '../../src/app/runtimeDiagnostics';

function baseOptions(): RuntimeDiagnosticsOptions {
  const mobilityState = createMobilityOverlayState();
  return {
    coordinateSystem: 'grid origin north-west, x east, y south, top-down minimal map projection',
    world: { id: 'test-world', width: 12, height: 9, chunkSize: 4 },
    visualStyle: { id: 'minimal-motorways', renderer: 'canvas-vector', spriteDrawing: 'disabled' },
    visualAssets: { id: 'minimal-vector', tile: { width: 18, height: 18 } },
    getBackend: () => ({ required: true, baseUrl: 'http://127.0.0.1:8080', status: null }),
    getMobilityState: () => mobilityState,
    getMobilityTickPeriodMs: () => 100,
    getSimTime: () => 4242,
    getPedestrianSprites: () => [],
    getVehicleSprites: () => [],
    getCamera: () => ({
      current: { x: 10, y: 20, scale: 2 },
      target: { x: 12, y: 22, scale: 2.5 },
      dragging: false,
      bounds: { minX: -8, maxX: 19, minY: -8, maxY: 16 },
      edgeTreatment: { outskirtsTiles: 12, exitTiles: 7 },
    }),
    getCounts: () => ({
      roadTiles: 3,
      railTiles: 2,
      bridges: 1,
      buildings: 4,
      trees: 5,
      railStations: 6,
      railYardTracks: 7,
      reserveTiles: 8,
    }),
    getDiagnostics: () => ({
      roadRailOverlap: 0,
      designedRailCrossings: 1,
      invalidBuildings: 2,
      buildingsOutsideStreetFrontageSet: 3,
      buildingsWithoutDirectStreetAdjacency: 4,
      buildingsWithoutAnyStreetAdjacency: 5,
      buildingsWithoutStreetFrontage: 6,
      buildingsTouchingRail: 7,
      buildingFramesOutsideFinishedRow: 8,
      railStationsOnRoad: 9,
      railStationsOnBuildings: 10,
      railStationsOnRails: 11,
      railStationsOnTrees: 12,
      adjacentParallelRoadRuns: 13,
      invalidRoadDeadEnds: 14,
      parallelRoadPairs: 15,
    }),
    getDetails: () => ({ total: 0 }),
    getValidation: () => ({
      validationErrors: 0,
      roadRailOverlap: 0,
      railCrossings: 1,
      invalidBuildings: 2,
      treeBuildingOverlap: 3,
    }),
    getSelected: () => ({
      agentId: null,
      vehicleId: null,
      agentInspector: null,
      vehicleInspector: null,
      selectedMarketCoord: null,
    }),
    getEconomyMarketCount: () => 0,
    getEconomyFlowCount: () => 0,
    getFrameTimeMs: () => 0,
    getEconomyMarkets: () => [],
    projectEntityScreen: (coord) => ({ x: coord.x + 100, y: coord.y + 200 }),
    carVisualWorldPoint: (vehicle) => vehicle.path[0],
    now: () => 1234,
    advanceTime: () => {},
  };
}

describe('buildRuntimeDiagnosticsPayload', () => {
  it('preserves the minimal renderer and asset contract', () => {
    const payload = buildRuntimeDiagnosticsPayload(baseOptions());

    expect(payload.city.visualStyle).toEqual({
      id: 'minimal-motorways',
      renderer: 'canvas-vector',
      spriteDrawing: 'disabled',
    });
    expect(payload.city.visualAssets).toEqual({
      id: 'minimal-vector',
      tile: { width: 18, height: 18 },
    });
    expect(payload.city.loadedRasterAssetPaths).toEqual([]);
  });

  it('exposes the world sim time for the clock display', () => {
    expect(buildRuntimeDiagnosticsPayload(baseOptions()).city.simTime).toBe(4242);
  });

  it('reports car traffic diagnostics', () => {
    const payload = buildRuntimeDiagnosticsPayload(baseOptions());

    expect(payload.city.traffic).toEqual({
      routes: 0,
      cars: 0,
      movingCars: 0,
      stuckCars: 0,
      invalidRouteCars: 0,
    });
  });
});

describe('installRuntimeDiagnostics', () => {
  it('installs render_game_to_text and advanceTime on a target object', () => {
    const target: { render_game_to_text?: () => string; advanceTime?: (ms: number) => void } = {};
    const advanceTime = vi.fn();

    installRuntimeDiagnostics(target, { ...baseOptions(), advanceTime });

    expect(target.render_game_to_text).toEqual(expect.any(Function));
    expect(target.advanceTime).toEqual(expect.any(Function));
    expect(JSON.parse(target.render_game_to_text!()).city.visualStyle.id).toBe('minimal-motorways');

    target.advanceTime!(2500);

    expect(advanceTime).toHaveBeenCalledExactlyOnceWith(2500);
  });
});
