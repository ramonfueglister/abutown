import { describe, expect, it } from 'vitest';
import { buildTrafficRequestsForVehicles } from '../../src/traffic/vehicleTrafficRequests';

describe('vehicle traffic requests', () => {
  it('creates a reservation request for the next visible intersection ahead of a vehicle', () => {
    const requests = buildTrafficRequestsForVehicles({
      tick: 120,
      intersections: new Map([
        ['1:0', { intersectionId: 'intersection:1:0', coord: { x: 1, y: 0 }, connectedDirections: ['west', 'south', 'east'] }],
      ]),
      vehicles: [
        {
          vehicleId: 'vehicle:7',
          path: [{ x: 0, y: 0 }, { x: 1, y: 0 }, { x: 1, y: 1 }],
          offset: 0.25,
          speed: 1,
        },
      ],
    });

    expect(requests).toEqual([
      expect.objectContaining({
        vehicleId: 'vehicle:7',
        intersectionId: 'intersection:1:0',
        currentOffset: 0.25,
        distanceToIntersection: 0.75,
        stopOffset: expect.any(Number),
        enterTick: expect.any(Number),
        exitTick: expect.any(Number),
        approachEdge: 'west',
        exitEdge: 'south',
        conflictMask: 1,
      }),
    ]);
    expect(requests[0].stopOffset).toBeLessThan(1);
    expect(requests[0].enterTick).toBeGreaterThanOrEqual(120);
    expect(requests[0].exitTick).toBeGreaterThan(requests[0].enterTick);
  });

  it('does not request intersections behind the vehicle or beyond the lookahead', () => {
    const requests = buildTrafficRequestsForVehicles({
      tick: 9,
      lookaheadTiles: 1.25,
      intersections: new Map([
        ['3:0', { intersectionId: 'intersection:3:0', coord: { x: 3, y: 0 }, connectedDirections: ['west', 'east', 'south'] }],
      ]),
      vehicles: [
        {
          vehicleId: 'vehicle:1',
          path: [{ x: 0, y: 0 }, { x: 1, y: 0 }, { x: 2, y: 0 }, { x: 3, y: 0 }],
          offset: 0.5,
          speed: 1,
        },
      ],
    });

    expect(requests).toEqual([]);
  });
});
