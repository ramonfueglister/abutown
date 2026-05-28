import { describe, expect, it } from 'vitest';
import { createTerrainState } from '../../src/backend/terrainState';
import { createCityDiagnostics } from '../../src/render/cityDiagnostics';
import { buildRenderGameText, detailCountsByCategory, nonPak128AssetPaths } from '../../src/render/renderGameText';

describe('render game text diagnostics', () => {
  it('counts city consistency diagnostics from render state', () => {
    const diagnostics = createCityDiagnostics({
      width: 8,
      height: 8,
      terrainAt: (coord) => coord.x === 1 && coord.y === 1 ? 'water' : 'grass',
      roads: new Map([
        ['1:1', { coord: { x: 1, y: 1 }, kind: 'street', mask: 15 }],
        ['2:2', { coord: { x: 2, y: 2 }, kind: 'street', mask: 15 }],
      ]),
      rails: new Map([
        ['1:1', { coord: { x: 1, y: 1 }, mask: 5 }],
        ['2:2', { coord: { x: 2, y: 2 }, mask: 5 }],
      ]),
      railCrossings: new Set(['2:2']),
      railReserved: new Set(['1:1']),
      railStations: [
        { coord: { x: 1, y: 1 }, frame: 0 },
      ],
      buildings: [
        { coord: { x: 1, y: 1 }, sheet: 'houses', frame: 0, district: 'zone:test' },
      ],
      trees: [{ x: 1, y: 1 }],
    });

    expect(diagnostics).toEqual(expect.objectContaining({
      roadRailOverlap: 1,
      designedRailCrossings: 1,
      invalidBuildings: 1,
      treeBuildingOverlap: 1,
      railStationsOnRoad: 1,
      railStationsOnBuildings: 1,
      railStationsOnRails: 1,
      railStationsOnTrees: 1,
    }));
  });

  it('builds the render_game_to_text JSON snapshot', () => {
    const terrainState = createTerrainState({ width: 8, height: 8, chunkSize: 4 });
    terrainState.loadedChunks.add('0:0');
    terrainState.tiles.set('0:0', {
      base: 'Reserve',
      surface: 'None',
      cover: 'None',
      display: null,
      zoneId: null,
      roadMask: null,
      railMask: null,
      version: 1,
    });

    const diagnostics = {
      roadRailOverlap: 0,
      designedRailCrossings: 1,
      invalidBuildings: 0,
      buildingsOutsideStreetFrontageSet: 0,
      buildingsWithoutDirectStreetAdjacency: 0,
      buildingsWithoutAnyStreetAdjacency: 0,
      buildingsWithoutStreetFrontage: 0,
      buildingsTouchingRail: 0,
      buildingFramesOutsideFinishedRow: 0,
      treeBuildingOverlap: 0,
      railStationsOnRoad: 0,
      railStationsOnBuildings: 0,
      railStationsOnRails: 0,
      railStationsOnTrees: 0,
      adjacentParallelRoadRuns: 0,
      invalidRoadDeadEnds: 0,
      parallelRoadPairs: 0,
    };

    const json = buildRenderGameText({
      worldId: 'test-world',
      visualStyleId: 'minimal-motorways',
      tileSize: { width: 18, height: 18 },
      nonPak128AssetPaths: [],
      width: 8,
      height: 8,
      terrainState,
      roads: new Map([['0:0', { coord: { x: 0, y: 0 }, kind: 'bridge', mask: 10 }]]),
      rails: new Map([['0:1', { coord: { x: 0, y: 1 }, mask: 5 }]]),
      railCrossings: new Set(['0:1']),
      railPaths: [[{ x: 0, y: 1 }]],
      railStations: [],
      buildings: [{ coord: { x: 2, y: 2 }, sheet: 'office', frame: 1, district: 'zone:test' }],
      trees: [{ x: 3, y: 3 }],
      details: [{ coord: { x: 4, y: 4 }, category: 'industry', assetCategory: 'factory' }],
      trains: [],
      projectedPedestrians: [{
        id: 'agent:1',
        path: [{ x: 1, y: 1 }, { x: 2, y: 1 }],
        offset: 0,
        speed: 0,
        laneOffset: 0,
        direction: 'e',
        sprite: { sheet: 'ped-sheet' },
      }],
      projectedCars: [{
        id: 'vehicle:car:1',
        path: [{ x: 2, y: 2 }, { x: 3, y: 2 }],
        offset: 0,
        speed: 0,
        direction: 'e',
        sprite: { sheet: 'car-sheet', role: 'vehicle.0' },
      }],
      pedestrianSprites: [{ sheet: 'ped-sheet' }],
      vehicleSprites: [{ sheet: 'car-sheet', role: 'vehicle.0' }],
      selectedAgentId: 'agent:1',
      selectedVehicleId: null,
      backendBaseUrl: 'http://backend.test',
      backendStatus: { service: 'abutown-sim', world_id: 'abutown-main', ok: true, protocol_version: 1 },
      backendMobility: { status: 'connected', tick: 3, agents: 1, vehicles: 1, stops: 0, invalidMessages: 0, lastError: null },
      diagnostics,
      camera: {
        current: { x: 1, y: 2, scale: 0.5 },
        target: { x: 3, y: 4, scale: 0.75 },
        dragging: false,
        bounds: { minX: -8, maxX: 15, minY: -8, maxY: 15 },
        edgeTreatment: { outskirtsTiles: 12, exitTiles: 7 },
      },
      entityScreenPosition: (coord) => ({ x: coord.x * 10, y: coord.y * 10 }),
      trainSummary: null,
    });

    const state = JSON.parse(json);
    expect(state.city.worldId).toBe('test-world');
    expect(state.city.details).toEqual({ total: 1, industry: 1 });
    expect(state.city.reserveTiles).toBe(1);
    expect(state.city.mobilityAgents.selected).toEqual(expect.objectContaining({ id: 'agent:1' }));
    expect(state.city.agentInspector.title).toBe('agent:1');
    expect(state.city.mobilityVehicles.vehicles[0]).toEqual(expect.objectContaining({
      id: 'vehicle:car:1',
      screen: { x: 20, y: 20 },
    }));
    expect(state.city.diagnostics).toEqual(diagnostics);
  });

  it('summarizes details and non-pak asset paths', () => {
    expect(detailCountsByCategory([
      { coord: { x: 0, y: 0 }, category: 'industry', assetCategory: 'factory' },
      { coord: { x: 1, y: 0 }, category: 'industry', assetCategory: 'depot' },
      { coord: { x: 2, y: 0 }, category: 'station', assetCategory: 'station' },
    ])).toEqual({ total: 3, industry: 2, station: 1 });
    expect(nonPak128AssetPaths([
      '/simutrans-assets/pak128/base/foo.png',
      '/other/foo.png',
    ])).toEqual(['/other/foo.png']);
  });
});
