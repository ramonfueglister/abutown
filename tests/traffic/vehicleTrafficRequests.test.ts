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

  it('normalizes unbounded offsets when the next intersection crosses the route seam', () => {
    const requests = buildTrafficRequestsForVehicles({
      tick: 50,
      intersections: new Map([
        ['0:0', { intersectionId: 'intersection:0:0', coord: { x: 0, y: 0 }, connectedDirections: ['south', 'east', 'west'] }],
      ]),
      vehicles: [
        {
          vehicleId: 'vehicle:2',
          path: [{ x: 0, y: 0 }, { x: 1, y: 0 }, { x: 1, y: 1 }, { x: 0, y: 1 }],
          offset: 7.6,
          speed: 1,
        },
      ],
    });

    expect(requests).toEqual([
      expect.objectContaining({
        vehicleId: 'vehicle:2',
        intersectionId: 'intersection:0:0',
        currentOffset: 3.6,
        distanceToIntersection: 0.4,
        stopOffset: 3.58,
        approachEdge: 'south',
        exitEdge: 'east',
      }),
    ]);
  });

  it('keeps near-seam offsets precise when finding the next intersection', () => {
    const requests = buildTrafficRequestsForVehicles({
      tick: 75,
      intersections: new Map([
        ['0:0', { intersectionId: 'intersection:0:0', coord: { x: 0, y: 0 }, connectedDirections: ['south', 'east', 'west'] }],
      ]),
      vehicles: [
        {
          vehicleId: 'vehicle:6',
          path: [{ x: 0, y: 0 }, { x: 1, y: 0 }, { x: 1, y: 1 }, { x: 0, y: 1 }],
          offset: 3.9996,
          speed: 1,
        },
      ],
    });

    expect(requests).toEqual([
      expect.objectContaining({
        vehicleId: 'vehicle:6',
        intersectionId: 'intersection:0:0',
        distanceToIntersection: 0,
        approachEdge: 'south',
        exitEdge: 'east',
      }),
    ]);
  });

  it('makes faster vehicles request earlier entry ticks at the same distance', () => {
    const requests = buildTrafficRequestsForVehicles({
      tick: 100,
      intersections: new Map([
        ['1:0', { intersectionId: 'intersection:1:0', coord: { x: 1, y: 0 }, connectedDirections: ['west', 'south', 'east'] }],
      ]),
      vehicles: [
        {
          vehicleId: 'vehicle:3',
          path: [{ x: 0, y: 0 }, { x: 1, y: 0 }, { x: 1, y: 1 }],
          offset: 0.25,
          speed: 2,
        },
        {
          vehicleId: 'vehicle:4',
          path: [{ x: 0, y: 0 }, { x: 1, y: 0 }, { x: 1, y: 1 }],
          offset: 0.25,
          speed: 0.5,
        },
      ],
    });

    expect(requests.map((request) => request.enterTick)).toEqual([103, 112]);
    expect(requests[0].enterTick).toBeLessThan(requests[1].enterTick);
  });

  it('does not request intersections when route steps are not adjacent cardinal tiles', () => {
    const requests = buildTrafficRequestsForVehicles({
      tick: 30,
      intersections: new Map([
        ['2:0', { intersectionId: 'intersection:2:0', coord: { x: 2, y: 0 }, connectedDirections: ['west', 'south', 'east'] }],
      ]),
      vehicles: [
        {
          vehicleId: 'vehicle:5',
          path: [{ x: 0, y: 0 }, { x: 2, y: 0 }, { x: 2, y: 1 }],
          offset: 0.25,
          speed: 1,
        },
      ],
    });

    expect(requests).toEqual([]);
  });
});
