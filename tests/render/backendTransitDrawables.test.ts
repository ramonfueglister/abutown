import { describe, expect, it } from 'vitest';
import { applyMobilitySnapshot, createMobilityOverlayState } from '../../src/backend/mobilityState';
import { carsFromMobilityState, tramsFromMobilityState } from '../../src/render/backendMobilityDrawables';

const carSprite = { sheet: 'city-bus', role: 'vehicle.bus' };
const tramSprite = { sheet: 'metro-line', role: 'vehicle.tram' };

describe('backend transit drawables', () => {
  it('projects backend trams separately from backend cars', () => {
    const state = applyMobilitySnapshot(createMobilityOverlayState(), {
      protocol_version: 1,
      world_id: 'zurich-river-city-v1',
      tick: 5,
      agents: [],
      stops: [],
      vehicles: [
        {
          id: 'vehicle:car:0:0',
          kind: 'car',
          route_id: 'route:arterial:0',
          link_index: 0,
          progress: 0.5,
          capacity: 1,
          occupants: [],
          dwell_ticks_remaining: 0,
          world_coord: { x: 20, y: 30 },
          direction: 's',
          sprite_key: 'vehicle:0',
        },
        {
          id: 'vehicle:tram:0:0',
          kind: 'tram',
          route_id: 'tram:rail:0',
          link_index: 0,
          progress: 0.25,
          capacity: 80,
          occupants: [],
          dwell_ticks_remaining: 0,
          world_coord: { x: 150, y: 64 },
          direction: 'n',
          sprite_key: 'tram:0',
        },
      ],
    }, 1000);

    const cars = carsFromMobilityState(state, [carSprite], 1000, 100);
    const trams = tramsFromMobilityState(state, [tramSprite], 1000, 100);

    expect(cars).toHaveLength(1);
    expect(cars[0].id).toBe('vehicle:car:0:0');
    expect(trams).toHaveLength(1);
    expect(trams[0]).toMatchObject({
      id: 'vehicle:tram:0:0',
      path: [{ x: 150, y: 64 }, { x: 150, y: 63 }],
      sprite: tramSprite,
      direction: 'n',
    });
  });
});
