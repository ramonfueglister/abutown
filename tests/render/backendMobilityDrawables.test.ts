import { describe, expect, it } from 'vitest';
import { pedestriansFromMobilityState, carsFromMobilityState } from '../../src/render/backendMobilityDrawables';

const pedestrianSprites = [
  { sheet: 'pak128/peds.0', frameWidth: 16, frameHeight: 32 },
  { sheet: 'pak128/peds.1', frameWidth: 16, frameHeight: 32 },
];
const vehicleSprites = [
  { sheet: 'pak128/cars.0', frameWidth: 32, frameHeight: 32, scale: 1, role: 'vehicle.0' },
  { sheet: 'pak128/cars.1', frameWidth: 32, frameHeight: 32, scale: 1, role: 'vehicle.1' },
];

const mobilityState = {
  status: 'connected' as const,
  tick: 1,
  agents: (() => {
    const agent = {
      id: 'agent:seed:0',
      state: { type: 'walking' as const, link_id: 'link:walk:default', progress: 0.5 },
      plan_cursor: 0,
      world_coord: { x: 10.5, y: 20.0 },
      direction: 'e' as const,
      sprite_key: 'pedestrian:0',
    };
    return new Map([
      ['agent:seed:0', { prev: agent, current: agent, lastTickAt: 0 }],
    ]);
  })(),
  vehicles: new Map(),
  stops: new Map(),
  roadVehicles: {
    tick: 1,
    vehicles: new Map([
      ['road_vehicle:seed:0', {
        id: 'road_vehicle:seed:0',
        world_coord: { x: 32.0, y: 32.0 },
        direction: 'n' as const,
        sprite_key: 'vehicle:0',
      }],
    ]),
    invalidMessages: 0,
    lastUpdatedAt: 0,
  },
  invalidMessages: 0,
  lastError: null,
  lastUpdatedAt: 0,
};

describe('backendMobilityDrawables', () => {
  it('projects agents into pedestrians with backend world_coord', () => {
    const pedestrians = pedestriansFromMobilityState(mobilityState, pedestrianSprites);
    expect(pedestrians).toHaveLength(1);
    expect(pedestrians[0].path[0]).toEqual({ x: 10.5, y: 20.0 });
    expect(pedestrians[0].sprite.sheet).toBe('pak128/peds.0');
    expect(pedestrians[0].id).toBe('agent:seed:0');
  });

  it('projects road vehicles into cars with backend world_coord', () => {
    const cars = carsFromMobilityState(mobilityState, vehicleSprites);
    expect(cars).toHaveLength(1);
    expect(cars[0].path[0]).toEqual({ x: 32.0, y: 32.0 });
    expect(cars[0].sprite.role).toBe('vehicle.0');
    expect(cars[0].id).toBe('road_vehicle:seed:0');
  });

  it('returns empty arrays when no sprites are available', () => {
    expect(pedestriansFromMobilityState(mobilityState, [])).toEqual([]);
    expect(carsFromMobilityState(mobilityState, [])).toEqual([]);
  });
});
