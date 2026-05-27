import { describe, expect, it } from 'vitest';
import { createZurichRuntimeContext } from '../../src/app/zurichRuntimeContext';

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
  });
});
