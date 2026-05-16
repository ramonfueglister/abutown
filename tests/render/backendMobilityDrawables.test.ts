import { describe, expect, it } from 'vitest';
import { pedestriansFromMobilityState, carsFromMobilityState } from '../../src/render/backendMobilityDrawables';
import {
  applyMobilityDelta,
  applyMobilitySnapshot,
  createMobilityOverlayState,
} from '../../src/backend/mobilityState';
import type {
  AgentMobilityDto,
  VehicleMobilityDto,
} from '../../src/backend/mobilityProtocol';

const pedestrianSprites = [
  { sheet: 'pak128/peds.0', frameWidth: 16, frameHeight: 32 },
  { sheet: 'pak128/peds.1', frameWidth: 16, frameHeight: 32 },
];
const vehicleSprites = [
  { sheet: 'pak128/cars.0', frameWidth: 32, frameHeight: 32, scale: 1, role: 'vehicle.0' },
  { sheet: 'pak128/cars.1', frameWidth: 32, frameHeight: 32, scale: 1, role: 'vehicle.1' },
];

function makeStateWith(agents: AgentMobilityDto[], vehicles: VehicleMobilityDto[]) {
  return applyMobilitySnapshot(
    createMobilityOverlayState(),
    {
      protocol_version: 1,
      world_id: 'abutown-main',
      tick: 1,
      agents,
      vehicles,
      stops: [],
    },
    0,
  );
}

describe('backendMobilityDrawables (interpolated)', () => {
  it('projects agents at interpolated coord based on now and tickPeriodMs', () => {
    let state = makeStateWith(
      [
        {
          id: 'agent:seed:0',
          state: { type: 'walking', link_id: 'link:walk:default', progress: 0 },
          plan_cursor: 0,
          world_coord: { x: 0, y: 0 },
          direction: 'e',
          sprite_key: 'pedestrian:0',
        },
      ],
      [],
    );
    state = applyMobilityDelta(
      state,
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 2,
        changed_agents: [
          {
            id: 'agent:seed:0',
            state: { type: 'walking', link_id: 'link:walk:default', progress: 0.5 },
            plan_cursor: 0,
            world_coord: { x: 100, y: 0 },
            direction: 'e',
            sprite_key: 'pedestrian:0',
          },
        ],
        changed_vehicles: [],
      },
      100,
    );
    const pedestrians = pedestriansFromMobilityState(state, pedestrianSprites, 150, 100);
    expect(pedestrians).toHaveLength(1);
    expect(pedestrians[0].path[0].x).toBeCloseTo(50, 5);
  });

  it('projects cars (kind=car) at interpolated coord', () => {
    let state = makeStateWith(
      [],
      [
        {
          id: 'vehicle:car:0',
          kind: 'car',
          route_id: 'route:car-loop',
          link_index: 0,
          progress: 0,
          capacity: 1,
          occupants: [],
          dwell_ticks_remaining: 0,
          world_coord: { x: 0, y: 0 },
          direction: 'e',
          sprite_key: 'vehicle:0',
        },
      ],
    );
    state = applyMobilityDelta(
      state,
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 2,
        changed_agents: [],
        changed_vehicles: [
          {
            id: 'vehicle:car:0',
            kind: 'car',
            route_id: 'route:car-loop',
            link_index: 0,
            progress: 0.5,
            capacity: 1,
            occupants: [],
            dwell_ticks_remaining: 0,
            world_coord: { x: 100, y: 0 },
            direction: 'e',
            sprite_key: 'vehicle:0',
          },
        ],
      },
      100,
    );
    const cars = carsFromMobilityState(state, vehicleSprites, 150, 100);
    expect(cars).toHaveLength(1);
    expect(cars[0].path[0].x).toBeCloseTo(50, 5);
  });

  it('cars source filters vehicles by kind=car (excludes trams)', () => {
    const state = makeStateWith(
      [],
      [
        {
          id: 'vehicle:car:0',
          kind: 'car',
          route_id: 'route:car-loop',
          link_index: 0,
          progress: 0,
          capacity: 1,
          occupants: [],
          dwell_ticks_remaining: 0,
          world_coord: { x: 50, y: 50 },
          direction: 'e',
          sprite_key: 'vehicle:0',
        },
        {
          id: 'vehicle:tram:0',
          kind: 'tram',
          route_id: 'route:tram-loop',
          link_index: 0,
          progress: 0,
          capacity: 24,
          occupants: [],
          dwell_ticks_remaining: 0,
          world_coord: { x: 60, y: 60 },
          direction: 'e',
          sprite_key: 'tram:0',
        },
      ],
    );
    const cars = carsFromMobilityState(state, vehicleSprites, 0, 100);
    expect(cars).toHaveLength(1);
    expect(cars[0].id).toBe('vehicle:car:0');
  });

  it('pedestrians exclude in_vehicle agents', () => {
    const state = makeStateWith(
      [
        {
          id: 'agent:walker:0',
          state: { type: 'walking', link_id: 'link:walk:default', progress: 0 },
          plan_cursor: 0,
          world_coord: { x: 10, y: 10 },
          direction: 'e',
          sprite_key: 'pedestrian:0',
        },
        {
          id: 'agent:driver:0',
          state: { type: 'in_vehicle', vehicle_id: 'vehicle:car:0', seat_index: 0 },
          plan_cursor: 0,
          world_coord: { x: 50, y: 50 },
          direction: 'e',
          sprite_key: 'pedestrian:0',
        },
      ],
      [],
    );
    const peds = pedestriansFromMobilityState(state, pedestrianSprites, 0, 100);
    expect(peds).toHaveLength(1);
    expect(peds[0].id).toBe('agent:walker:0');
  });

  it('returns empty arrays when no sprites are available', () => {
    const state = makeStateWith([], []);
    expect(pedestriansFromMobilityState(state, [], 0, 100)).toEqual([]);
    expect(carsFromMobilityState(state, [], 0, 100)).toEqual([]);
  });
});
