import { describe, expect, it } from 'vitest';
import {
  buildLocalRoadVehicles,
  findNearestLocalRoadVehicle,
  type LocalRoadVehicleSource,
} from '../../src/render/localRoadVehicles';

const vehicles: LocalRoadVehicleSource[] = [
  {
    path: [{ x: 10, y: 20 }, { x: 14, y: 20 }, { x: 14, y: 24 }],
    offset: 0.5,
    speed: 1.4,
    sprite: { sheet: 'bus', role: 'vehicle.bus' },
  },
  {
    path: [{ x: 40, y: 8 }, { x: 40, y: 12 }],
    offset: 1.25,
    speed: 1.9,
    sprite: { sheet: 'truck', role: 'vehicle.truck' },
  },
];

describe('local road vehicles', () => {
  it('projects rendered cars into stable local vehicles', () => {
    expect(buildLocalRoadVehicles(vehicles)).toEqual([
      {
        id: 'vehicle:road:0',
        kind: 'road-vehicle',
        state: 'driving',
        coord: { x: 12, y: 20 },
        pathIndex: 0,
        nextCoord: { x: 14, y: 20 },
        speed: 1.4,
        spriteSheet: 'bus',
        role: 'vehicle.bus',
      },
      {
        id: 'vehicle:road:1',
        kind: 'road-vehicle',
        state: 'driving',
        coord: { x: 40, y: 11 },
        pathIndex: 1,
        nextCoord: { x: 40, y: 8 },
        speed: 1.9,
        spriteSheet: 'truck',
        role: 'vehicle.truck',
      },
    ]);
  });

  it('finds the nearest local road vehicle inside the click radius', () => {
    const localVehicles = buildLocalRoadVehicles(vehicles);
    const hit = findNearestLocalRoadVehicle(localVehicles, { x: 24, y: 40 }, (coord) => ({ x: coord.x * 2, y: coord.y * 2 }), 5);

    expect(hit?.id).toBe('vehicle:road:0');
  });

  it('returns null when no local road vehicle is close enough', () => {
    const localVehicles = buildLocalRoadVehicles(vehicles);
    const hit = findNearestLocalRoadVehicle(localVehicles, { x: 25, y: 41 }, (coord) => ({ x: coord.x * 2, y: coord.y * 2 }), 1);

    expect(hit).toBeNull();
  });
});
