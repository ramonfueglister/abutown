import { describe, expect, it } from 'vitest';
import { pedestriansFromMobilityState, carsFromMobilityState } from '../../src/render/backendMobilityDrawables';
import { applyMobilityDelta, applyMobilitySnapshot, createMobilityOverlayState } from '../../src/backend/mobilityState';
import { applyRoadVehicleDelta, applyRoadVehicleSnapshot } from '../../src/backend/roadVehicleState';

const pedestrianSprites = [
  { sheet: 'pak128/peds.0', frameWidth: 16, frameHeight: 32 },
  { sheet: 'pak128/peds.1', frameWidth: 16, frameHeight: 32 },
];
const vehicleSprites = [
  { sheet: 'pak128/cars.0', frameWidth: 32, frameHeight: 32, scale: 1, role: 'vehicle.0' },
  { sheet: 'pak128/cars.1', frameWidth: 32, frameHeight: 32, scale: 1, role: 'vehicle.1' },
];

describe('backendMobilityDrawables (interpolated)', () => {
  it('projects agents at interpolated coord based on now and tickPeriodMs', () => {
    let state = applyMobilitySnapshot(
      createMobilityOverlayState(),
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        agents: [
          {
            id: 'agent:seed:0',
            state: { type: 'walking', link_id: 'link:walk:default', progress: 0 },
            plan_cursor: 0,
            world_coord: { x: 0, y: 0 },
            direction: 'e',
            sprite_key: 'pedestrian:0',
          },
        ],
        vehicles: [],
        stops: [],
      },
      0,
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

  it('projects road vehicles at interpolated coord', () => {
    let state = applyMobilitySnapshot(
      createMobilityOverlayState(),
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        agents: [],
        vehicles: [],
        stops: [],
      },
      0,
    );
    state = {
      ...state,
      roadVehicles: applyRoadVehicleSnapshot(
        state.roadVehicles,
        {
          protocol_version: 1,
          world_id: 'abutown-main',
          tick: 1,
          vehicles: [
            { id: 'road_vehicle:seed:0', world_coord: { x: 0, y: 0 }, direction: 'e', sprite_key: 'vehicle:0' },
          ],
        },
        0,
      ),
    };
    state = {
      ...state,
      roadVehicles: applyRoadVehicleDelta(
        state.roadVehicles,
        {
          protocol_version: 1,
          world_id: 'abutown-main',
          tick: 2,
          changed: [{ id: 'road_vehicle:seed:0', world_coord: { x: 100, y: 0 }, direction: 'e', sprite_key: 'vehicle:0' }],
        },
        100,
      ),
    };
    const cars = carsFromMobilityState(state, vehicleSprites, 150, 100);
    expect(cars).toHaveLength(1);
    expect(cars[0].path[0].x).toBeCloseTo(50, 5);
  });

  it('returns empty arrays when no sprites are available', () => {
    const state = applyMobilitySnapshot(
      createMobilityOverlayState(),
      { protocol_version: 1, world_id: 'abutown-main', tick: 1, agents: [], vehicles: [], stops: [] },
      0,
    );
    expect(pedestriansFromMobilityState(state, [], 0, 100)).toEqual([]);
    expect(carsFromMobilityState(state, [], 0, 100)).toEqual([]);
  });
});
