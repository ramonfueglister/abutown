import { describe, expect, it } from 'vitest';
import {
  applyRoadVehicleDelta,
  applyRoadVehicleSnapshot,
  createRoadVehicleOverlayState,
} from '../../src/backend/roadVehicleState';

const snapshot = {
  protocol_version: 1,
  world_id: 'abutown-main',
  tick: 3,
  vehicles: [
    {
      id: 'road_vehicle:seed:0',
      world_coord: { x: 1.0, y: 2.0 },
      direction: 'e' as const,
      sprite_key: 'vehicle:0',
    },
  ],
};

describe('road vehicle state', () => {
  it('applies snapshot then delta', () => {
    const initial = createRoadVehicleOverlayState();
    const afterSnap = applyRoadVehicleSnapshot(initial, snapshot);
    expect(afterSnap.tick).toBe(3);
    expect(afterSnap.vehicles.get('road_vehicle:seed:0')?.direction).toBe('e');

    const afterDelta = applyRoadVehicleDelta(afterSnap, {
      protocol_version: 1,
      world_id: 'abutown-main',
      tick: 4,
      changed: [
        {
          id: 'road_vehicle:seed:0',
          world_coord: { x: 5.0, y: 2.0 },
          direction: 'n',
          sprite_key: 'vehicle:0',
        },
      ],
    });
    expect(afterDelta.tick).toBe(4);
    expect(afterDelta.vehicles.get('road_vehicle:seed:0')?.world_coord.x).toBe(5.0);
  });
});
