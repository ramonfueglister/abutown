import { describe, expect, it } from 'vitest';
import {
  findNearestProjectedEntity,
  selectMobilityEntityAtWorldPoint,
} from '../../src/render/mobilityEntitySelection';

const gridToWorld = (coord: { x: number; y: number }) => ({ x: coord.x * 10, y: coord.y * 10 });

describe('mobilityEntitySelection', () => {
  it('finds the nearest projected entity inside the radius', () => {
    const entities = [
      { id: 'far', path: [{ x: 10, y: 10 }] },
      { id: 'near', path: [{ x: 2, y: 1 }] },
    ];

    expect(findNearestProjectedEntity(entities, { x: 23, y: 12 }, 8, gridToWorld)?.id).toBe('near');
    expect(findNearestProjectedEntity(entities, { x: 23, y: 12 }, 2, gridToWorld)).toBeNull();
  });

  it('prefers vehicles over pedestrians when both are under the pointer', () => {
    const selection = selectMobilityEntityAtWorldPoint({
      pedestrians: [{ id: 'agent:1', path: [{ x: 2, y: 2 }] }],
      cars: [{ id: 'vehicle:1', path: [{ x: 2, y: 2 }] }],
      worldPoint: { x: 20, y: 20 },
      cameraScale: 1,
      gridToWorld,
    });

    expect(selection).toEqual({ selectedAgentId: null, selectedVehicleId: 'vehicle:1' });
  });

  it('selects the nearest pedestrian when no vehicle is in range', () => {
    const selection = selectMobilityEntityAtWorldPoint({
      pedestrians: [
        { id: 'agent:far', path: [{ x: 7, y: 7 }] },
        { id: 'agent:near', path: [{ x: 2, y: 2 }] },
      ],
      cars: [{ id: 'vehicle:far', path: [{ x: 20, y: 20 }] }],
      worldPoint: { x: 22, y: 21 },
      cameraScale: 1,
      gridToWorld,
    });

    expect(selection).toEqual({ selectedAgentId: 'agent:near', selectedVehicleId: null });
  });

  it('clears selection when nothing is close enough', () => {
    const selection = selectMobilityEntityAtWorldPoint({
      pedestrians: [{ id: 'agent:1', path: [{ x: 10, y: 10 }] }],
      cars: [{ id: 'vehicle:1', path: [{ x: 20, y: 20 }] }],
      worldPoint: { x: 0, y: 0 },
      cameraScale: 1,
      gridToWorld,
    });

    expect(selection).toEqual({ selectedAgentId: null, selectedVehicleId: null });
  });
});
