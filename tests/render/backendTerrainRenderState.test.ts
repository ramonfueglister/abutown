import { describe, expect, it } from 'vitest';
import { applyLayeredChunkSnapshot, createTerrainState } from '../../src/backend/terrainState';
import { createBackendTerrainRenderState } from '../../src/render/backendTerrainRenderState';

describe('backend terrain render state', () => {
  it('projects layered backend tiles into render maps and drawable inputs', () => {
    const state = createTerrainState({ width: 64, height: 64, chunkSize: 32 });
    applyLayeredChunkSnapshot(state, {
      coord: { x: 0, y: 0 },
      tileCount: 1024,
      tiles: [
        {
          localIndex: 0,
          base: 'Water',
          surface: 'Bridge',
          cover: 'None',
          display: null,
          zoneId: null,
          roadMask: 10,
          railMask: null,
          version: 1,
        },
        {
          localIndex: 1,
          base: 'Grass',
          surface: 'RailCrossing',
          cover: 'None',
          display: null,
          zoneId: null,
          roadMask: 15,
          railMask: 5,
          version: 1,
        },
        {
          localIndex: 2,
          base: 'Grass',
          surface: 'None',
          cover: 'Building',
          display: 'office',
          zoneId: 'zone:business',
          roadMask: null,
          railMask: null,
          version: 1,
        },
        {
          localIndex: 3,
          base: 'Grass',
          surface: 'None',
          cover: 'Tree',
          display: null,
          zoneId: null,
          roadMask: null,
          railMask: null,
          version: 1,
        },
        {
          localIndex: 4,
          base: 'Grass',
          surface: 'None',
          cover: 'Detail',
          display: 'road-depot',
          zoneId: null,
          roadMask: null,
          railMask: null,
          version: 1,
        },
        {
          localIndex: 33,
          base: 'Forest',
          surface: 'Rail',
          cover: 'None',
          display: null,
          zoneId: null,
          roadMask: null,
          railMask: 1,
          version: 1,
        },
      ],
    });

    const renderState = createBackendTerrainRenderState(state, { buildingFrameVariants: 4 });

    expect(renderState.terrain.get('0:0')).toBe('water');
    expect(renderState.terrain.get('1:1')).toBe('park');
    expect(renderState.roads.get('0:0')).toMatchObject({ coord: { x: 0, y: 0 }, kind: 'bridge', mask: 10 });
    expect(renderState.roads.get('1:0')).toMatchObject({ coord: { x: 1, y: 0 }, kind: 'street', mask: 15 });
    expect(renderState.rails.get('1:0')).toMatchObject({ coord: { x: 1, y: 0 }, mask: 5 });
    expect(renderState.railCrossings.has('1:0')).toBe(true);
    expect(renderState.railReserved.has('1:1')).toBe(true);
    expect(renderState.railPaths[0]).toEqual([{ x: 1, y: 0 }, { x: 1, y: 1 }]);
    expect(renderState.railYardPaths).toEqual([]);
    expect(renderState.railStations).toEqual([]);
    expect(renderState.buildings).toEqual([
      expect.objectContaining({ coord: { x: 2, y: 0 }, sheet: 'office', district: 'zone:business' }),
    ]);
    expect(renderState.buildings[0].frame).toBeGreaterThanOrEqual(0);
    expect(renderState.buildings[0].frame).toBeLessThan(4);
    expect(renderState.trees).toEqual([{ x: 3, y: 0 }]);
    expect(renderState.details).toEqual([
      { coord: { x: 4, y: 0 }, assetCategory: 'road-depot', category: 'industry' },
    ]);
  });
});
