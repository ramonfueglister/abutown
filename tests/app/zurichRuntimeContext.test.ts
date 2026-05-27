import { describe, expect, it } from 'vitest';
import {
  buildStaticDiagnostics,
  createZurichRuntimeContext,
  type RuntimeBuilding,
} from '../../src/app/zurichRuntimeContext';

describe('zurichRuntimeContext', () => {
  it('builds the current minimal-motorways Zurich runtime context', () => {
    const context = createZurichRuntimeContext({ seed: 1848 });

    expect(context.world.id).toBe('zurich-river-city-v1');
    expect(context.world.width).toBe(256);
    expect(context.world.height).toBe(256);
    expect(context.world.chunkSize).toBe(32);
    expect(context.transport.roads.size).toBe(3396);
    expect(context.transport.roads.size).toBeGreaterThan(1800);
    expect(context.transport.rails.size).toBe(256);
    expect(context.transport.railCrossings.size).toBe(3);
    expect(context.placement.buildings.length).toBe(2268);
    expect(context.placement.buildings.length).toBeGreaterThan(2250);
    expect(context.placement.trees.length).toBe(4325);
    expect(context.placement.trees.length).toBeGreaterThan(3000);
    expect(context.placement.details.length).toBe(280);
    expect(context.validation.errors).toHaveLength(0);
    expect(context.runtime.roads.size).toBe(context.transport.roads.size);
    expect(context.runtime.rails.size).toBe(context.transport.rails.size);
    expect(context.runtime.railReserved.size).toBe(context.transport.rails.size);
    expect(context.runtime.railStations).toHaveLength(0);
  });

  it('reports static diagnostics without invented values', () => {
    const context = createZurichRuntimeContext({ seed: 1848 });
    const diagnostics = context.staticDiagnostics();

    expect(diagnostics.invalidBuildings).toBe(0);
    expect(diagnostics.roadRailOverlap).toBe(0);
    expect(diagnostics.designedRailCrossings).toBe(3);
    expect(diagnostics.railStationsOnRoad).toBe(0);
    expect(diagnostics.railStationsOnBuildings).toBe(0);
    expect(diagnostics.railStationsOnRails).toBe(0);
    expect(diagnostics.railStationsOnTrees).toBe(0);
    expect(diagnostics.buildingFramesOutsideFinishedRow).toBe(0);
  });

  it('counts building frames outside the finished first row by sheet', () => {
    const context = createZurichRuntimeContext({ seed: 1848 });
    const target = context.runtime.buildings[0];
    const runtime = {
      ...context.runtime,
      buildings: context.runtime.buildings.map((building) =>
        building === target ? { ...building, frame: finishedRowColumns(building.sheet) } : building,
      ),
    };

    const diagnostics = buildStaticDiagnostics(context.world, runtime);

    expect(diagnostics.buildingFramesOutsideFinishedRow).toBe(1);
  });
});

function finishedRowColumns(sheet: RuntimeBuilding['sheet']): number {
  switch (sheet) {
    case 'houses':
    case 'oldhouses':
    case 'office':
    case 'tower':
      return 4;
    case 'cottages':
    case 'church':
      return 1;
    case 'townhouses':
    case 'modern':
      return 2;
    case 'shops':
      return 6;
    case 'flats':
      return 3;
  }
}
