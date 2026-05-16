import { describe, expect, it } from 'vitest';
import {
  applyRoadVehicleDelta,
  applyRoadVehicleSnapshot,
  createRoadVehicleOverlayState,
  interpolatedRoadVehicles,
} from '../../src/backend/roadVehicleState';
import type { RoadVehicleDto } from '../../src/backend/roadVehicleProtocol';

function vehicleAt(id: string, x: number, y: number): RoadVehicleDto {
  return { id, world_coord: { x, y }, direction: 'e', sprite_key: 'vehicle:0' };
}

describe('road vehicle state interpolation buffer', () => {
  it('snapshot then delta updates prev+current+lastTickAt', () => {
    let state = applyRoadVehicleSnapshot(
      createRoadVehicleOverlayState(),
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        vehicles: [vehicleAt('road_vehicle:seed:0', 100, 200)],
      },
      1000,
    );
    let entry = state.vehicles.get('road_vehicle:seed:0')!;
    expect(entry.prev.world_coord).toEqual({ x: 100, y: 200 });
    expect(entry.current.world_coord).toEqual({ x: 100, y: 200 });
    expect(entry.lastTickAt).toBe(1000);

    state = applyRoadVehicleDelta(
      state,
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 2,
        changed: [vehicleAt('road_vehicle:seed:0', 110, 200)],
      },
      1100,
    );
    entry = state.vehicles.get('road_vehicle:seed:0')!;
    expect(entry.prev.world_coord).toEqual({ x: 100, y: 200 });
    expect(entry.current.world_coord).toEqual({ x: 110, y: 200 });
    expect(entry.lastTickAt).toBe(1100);
  });

  it('delta for a new vehicle sets prev == current', () => {
    let state = createRoadVehicleOverlayState();
    state = applyRoadVehicleDelta(
      state,
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        changed: [vehicleAt('road_vehicle:seed:0', 50, 60)],
      },
      500,
    );
    const entry = state.vehicles.get('road_vehicle:seed:0')!;
    expect(entry.prev.world_coord).toEqual({ x: 50, y: 60 });
    expect(entry.current.world_coord).toEqual({ x: 50, y: 60 });
  });

  it('interpolatedRoadVehicles lerps world_coord at t = 0.5', () => {
    let state = applyRoadVehicleSnapshot(
      createRoadVehicleOverlayState(),
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        vehicles: [vehicleAt('road_vehicle:seed:0', 0, 0)],
      },
      1000,
    );
    state = applyRoadVehicleDelta(
      state,
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 2,
        changed: [vehicleAt('road_vehicle:seed:0', 100, 0)],
      },
      1100,
    );
    const vehicles = interpolatedRoadVehicles(state, 1150, 100);
    expect(vehicles).toHaveLength(1);
    expect(vehicles[0].world_coord.x).toBeCloseTo(50.0, 5);
    expect(vehicles[0].world_coord.y).toBeCloseTo(0.0, 5);
  });

  it('interpolatedRoadVehicles clamps t to [0, 1]', () => {
    let state = applyRoadVehicleSnapshot(
      createRoadVehicleOverlayState(),
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        vehicles: [vehicleAt('road_vehicle:seed:0', 0, 0)],
      },
      0,
    );
    state = applyRoadVehicleDelta(
      state,
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 2,
        changed: [vehicleAt('road_vehicle:seed:0', 100, 0)],
      },
      1000,
    );
    const earlyVehicles = interpolatedRoadVehicles(state, 500, 100);
    expect(earlyVehicles[0].world_coord.x).toBeCloseTo(0, 5);
    const lateVehicles = interpolatedRoadVehicles(state, 5000, 100);
    expect(lateVehicles[0].world_coord.x).toBeCloseTo(100, 5);
  });
});
