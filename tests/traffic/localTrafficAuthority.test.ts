import { describe, expect, it } from 'vitest';
import { stepLocalTrafficAuthority } from '../../src/traffic/localTrafficAuthority';
import {
  createInitialTrafficRuleSnapshot,
  trafficReservationId,
  type TrafficReservation,
  type TrafficRuleSnapshot,
  type TrafficVehicleRequest,
} from '../../src/traffic/trafficTypes';

function request(overrides: Partial<TrafficVehicleRequest>): TrafficVehicleRequest {
  return {
    vehicleId: 'vehicle:1',
    intersectionId: 'intersection:4:4',
    distanceToIntersection: 0.8,
    stopOffset: 2.58,
    currentOffset: 1.9,
    enterTick: 20,
    exitTick: 26,
    approachEdge: 'west',
    exitEdge: 'east',
    conflictMask: 1,
    priority: 1,
    ...overrides,
  };
}

function snapshotWithReservations(reservations: TrafficReservation[]): TrafficRuleSnapshot {
  return {
    ...createInitialTrafficRuleSnapshot(),
    version: 4,
    reservations,
  };
}

describe('local traffic authority', () => {
  it('grants only one reservation for conflicting intersection requests', () => {
    const result = stepLocalTrafficAuthority({
      snapshot: createInitialTrafficRuleSnapshot(),
      tick: 10,
      requests: [
        request({ vehicleId: 'vehicle:2', priority: 2 }),
        request({ vehicleId: 'vehicle:1', priority: 1 }),
      ],
    });

    expect(result.snapshot.reservations).toHaveLength(1);
    expect(result.snapshot.reservations[0].vehicleId).toBe('vehicle:1');
    expect(result.decisions.get('vehicle:1')).toEqual(expect.objectContaining({ kind: 'go' }));
    expect(result.decisions.get('vehicle:2')).toEqual(expect.objectContaining({ kind: 'yield' }));
    expect(result.diagnostics.intersectionConflictsPrevented).toBe(1);
    expect(result.diagnostics.yieldingVehicles).toBe(1);
    expect(result.diagnostics.blockedVehicles).toBe(1);
    expect(result.diagnostics.trafficRuleDecisionCount).toBe(2);
    expect(result.diagnostics.reservedIntersections).toBe(1);
  });

  it('sorts conflicting requests by enter tick before priority', () => {
    const result = stepLocalTrafficAuthority({
      snapshot: createInitialTrafficRuleSnapshot(),
      tick: 10,
      requests: [
        request({ vehicleId: 'vehicle:2', enterTick: 22, exitTick: 30, priority: 0 }),
        request({ vehicleId: 'vehicle:1', enterTick: 20, exitTick: 28, priority: 9 }),
      ],
    });

    expect(result.snapshot.reservations).toHaveLength(1);
    expect(result.snapshot.reservations[0].vehicleId).toBe('vehicle:1');
    expect(result.decisions.get('vehicle:1')?.kind).toBe('go');
    expect(result.decisions.get('vehicle:2')?.kind).toBe('yield');
  });

  it('uses stable vehicle id ordering after matching enter tick and priority', () => {
    const result = stepLocalTrafficAuthority({
      snapshot: createInitialTrafficRuleSnapshot(),
      tick: 10,
      requests: [
        request({ vehicleId: 'vehicle:10', priority: 1 }),
        request({ vehicleId: 'vehicle:2', priority: 1 }),
      ],
    });

    expect(result.snapshot.reservations).toHaveLength(1);
    expect(result.snapshot.reservations[0].vehicleId).toBe('vehicle:10');
    expect(result.decisions.get('vehicle:10')?.kind).toBe('go');
    expect(result.decisions.get('vehicle:2')?.kind).toBe('yield');
  });

  it('allows non-conflicting reservations in the same time window', () => {
    const result = stepLocalTrafficAuthority({
      snapshot: createInitialTrafficRuleSnapshot(),
      tick: 10,
      requests: [
        request({ vehicleId: 'vehicle:1', conflictMask: 1 }),
        request({ vehicleId: 'vehicle:2', conflictMask: 2 }),
      ],
    });

    expect(result.snapshot.reservations.map((reservation) => reservation.vehicleId).sort()).toEqual([
      'vehicle:1',
      'vehicle:2',
    ]);
    expect(result.decisions.get('vehicle:1')?.kind).toBe('go');
    expect(result.decisions.get('vehicle:2')?.kind).toBe('go');
    expect(result.diagnostics.reservedIntersections).toBe(1);
  });

  it('allows reservations for different intersections in overlapping windows', () => {
    const result = stepLocalTrafficAuthority({
      snapshot: createInitialTrafficRuleSnapshot(),
      tick: 10,
      requests: [
        request({ vehicleId: 'vehicle:1', intersectionId: 'intersection:4:4' }),
        request({ vehicleId: 'vehicle:2', intersectionId: 'intersection:5:5' }),
      ],
    });

    expect(result.snapshot.reservations).toHaveLength(2);
    expect(result.diagnostics.reservedIntersections).toBe(2);
    expect(result.diagnostics.intersectionConflictsPrevented).toBe(0);
  });

  it('allows reservations at the same intersection when time windows do not overlap', () => {
    const result = stepLocalTrafficAuthority({
      snapshot: createInitialTrafficRuleSnapshot(),
      tick: 10,
      requests: [
        request({ vehicleId: 'vehicle:1', enterTick: 20, exitTick: 24 }),
        request({ vehicleId: 'vehicle:2', enterTick: 24, exitTick: 30 }),
      ],
    });

    expect(result.snapshot.reservations).toHaveLength(2);
    expect(result.diagnostics.intersectionConflictsPrevented).toBe(0);
  });

  it('expires old reservations before evaluating new requests', () => {
    const first = stepLocalTrafficAuthority({
      snapshot: createInitialTrafficRuleSnapshot(),
      tick: 10,
      requests: [request({ vehicleId: 'vehicle:1', enterTick: 12, exitTick: 14 })],
    });
    const second = stepLocalTrafficAuthority({
      snapshot: first.snapshot,
      tick: 15,
      requests: [request({ vehicleId: 'vehicle:2', enterTick: 16, exitTick: 18 })],
    });

    expect(second.snapshot.reservations).toHaveLength(1);
    expect(second.snapshot.reservations[0].vehicleId).toBe('vehicle:2');
    expect(second.diagnostics.expiredReservations).toBe(1);
  });

  it('updates an existing same-vehicle reservation after revalidating conflicts', () => {
    const reservation: TrafficReservation = {
      reservationId: trafficReservationId('intersection:4:4', 'vehicle:1', 20),
      intersectionId: 'intersection:4:4',
      vehicleId: 'vehicle:1',
      enterTick: 20,
      exitTick: 26,
      approachEdge: 'west',
      exitEdge: 'east',
      conflictMask: 1,
      priority: 1,
    };

    const result = stepLocalTrafficAuthority({
      snapshot: snapshotWithReservations([reservation]),
      tick: 12,
      requests: [request({
        vehicleId: 'vehicle:1',
        enterTick: 24,
        exitTick: 30,
        priority: 99,
      })],
    });

    expect(result.snapshot.reservations).toEqual([
      expect.objectContaining({
        reservationId: trafficReservationId('intersection:4:4', 'vehicle:1', 24),
        vehicleId: 'vehicle:1',
        enterTick: 24,
        exitTick: 30,
        priority: 99,
      }),
    ]);
    expect(result.decisions.get('vehicle:1')).toEqual(expect.objectContaining({
      kind: 'go',
      reservationId: trafficReservationId('intersection:4:4', 'vehicle:1', 24),
    }));
    expect(result.diagnostics.intersectionConflictsPrevented).toBe(0);
  });

  it('does not reuse stale same-vehicle reservations when the current request conflicts', () => {
    const vehicleAReservation: TrafficReservation = {
      reservationId: trafficReservationId('intersection:4:4', 'vehicle:1', 20),
      intersectionId: 'intersection:4:4',
      vehicleId: 'vehicle:1',
      enterTick: 20,
      exitTick: 26,
      approachEdge: 'west',
      exitEdge: 'east',
      conflictMask: 1,
      priority: 1,
    };
    const vehicleBReservation: TrafficReservation = {
      reservationId: trafficReservationId('intersection:4:4', 'vehicle:2', 26),
      intersectionId: 'intersection:4:4',
      vehicleId: 'vehicle:2',
      enterTick: 26,
      exitTick: 32,
      approachEdge: 'north',
      exitEdge: 'south',
      conflictMask: 1,
      priority: 2,
    };

    const result = stepLocalTrafficAuthority({
      snapshot: snapshotWithReservations([vehicleAReservation, vehicleBReservation]),
      tick: 12,
      requests: [
        request({
          vehicleId: 'vehicle:1',
          enterTick: 24,
          exitTick: 30,
          currentOffset: 2.5,
          stopOffset: 2.58,
        }),
      ],
    });

    expect(result.decisions.get('vehicle:1')).toEqual(expect.objectContaining({ kind: 'stop' }));
    expect(result.snapshot.reservations).toEqual([vehicleAReservation, vehicleBReservation]);
    expect(result.diagnostics.intersectionConflictsPrevented).toBe(1);
  });

  it('seeds diagnostics with unclassified traffic requests from request building', () => {
    const result = stepLocalTrafficAuthority({
      snapshot: createInitialTrafficRuleSnapshot(),
      tick: 10,
      requests: [],
      unclassifiedTrafficRequests: 3,
    });

    expect(result.diagnostics.unclassifiedTrafficRequests).toBe(3);
    expect(result.snapshot.diagnostics.unclassifiedTrafficRequests).toBe(3);
  });

  it('does not reuse a same-vehicle reservation for a different intersection', () => {
    const reservation: TrafficReservation = {
      reservationId: trafficReservationId('intersection:4:4', 'vehicle:1', 20),
      intersectionId: 'intersection:4:4',
      vehicleId: 'vehicle:1',
      enterTick: 20,
      exitTick: 26,
      approachEdge: 'west',
      exitEdge: 'east',
      conflictMask: 1,
      priority: 1,
    };

    const result = stepLocalTrafficAuthority({
      snapshot: snapshotWithReservations([reservation]),
      tick: 12,
      requests: [
        request({
          vehicleId: 'vehicle:1',
          intersectionId: 'intersection:5:5',
          enterTick: 20,
          exitTick: 26,
        }),
      ],
    });

    expect(result.snapshot.reservations).toHaveLength(2);
    expect(result.snapshot.reservations[1]).toEqual(expect.objectContaining({
      intersectionId: 'intersection:5:5',
      vehicleId: 'vehicle:1',
    }));
    expect(result.decisions.get('vehicle:1')?.reservationId).not.toBe(reservation.reservationId);
  });

  it('caps yielding cars at their stop boundary', () => {
    const result = stepLocalTrafficAuthority({
      snapshot: createInitialTrafficRuleSnapshot(),
      tick: 10,
      requests: [
        request({ vehicleId: 'vehicle:1', priority: 1, currentOffset: 2.1 }),
        request({ vehicleId: 'vehicle:2', priority: 2, currentOffset: 1.9, stopOffset: 2.58 }),
      ],
    });

    const blocked = result.decisions.get('vehicle:2');
    expect(blocked?.kind).toBe('yield');
    expect(blocked?.speedFactor).toBeGreaterThan(0);
    expect(blocked?.maxAdvance).toBeCloseTo(0.68, 3);
  });

  it('stops a blocked car before it crosses its stop boundary', () => {
    const result = stepLocalTrafficAuthority({
      snapshot: createInitialTrafficRuleSnapshot(),
      tick: 10,
      requests: [
        request({ vehicleId: 'vehicle:1', priority: 1, currentOffset: 2.1 }),
        request({ vehicleId: 'vehicle:2', priority: 2, currentOffset: 2.5, stopOffset: 2.58 }),
      ],
    });

    const blocked = result.decisions.get('vehicle:2');
    expect(blocked?.kind).toBe('stop');
    expect(blocked?.speedFactor).toBe(0);
    expect(blocked?.maxAdvance).toBeCloseTo(0.08, 3);
    expect(result.diagnostics.stoppedForTrafficRules).toBe(1);
    expect(result.diagnostics.blockedVehicles).toBe(1);
  });

  it('does not let denied near-seam vehicles roll through the stop boundary', () => {
    const result = stepLocalTrafficAuthority({
      snapshot: createInitialTrafficRuleSnapshot(),
      tick: 10,
      requests: [
        request({ vehicleId: 'vehicle:1', currentOffset: 3.1, stopOffset: 3.58, priority: 1 }),
        request({ vehicleId: 'vehicle:2', currentOffset: 3.99, stopOffset: 3.58, priority: 2 }),
      ],
    });

    const blocked = result.decisions.get('vehicle:2');
    expect(blocked?.kind).toBe('stop');
    expect(blocked?.maxAdvance).toBe(0);
  });

  it('does not produce a large advance when current and stop offsets straddle the route seam', () => {
    const result = stepLocalTrafficAuthority({
      snapshot: createInitialTrafficRuleSnapshot(),
      tick: 10,
      requests: [
        request({ vehicleId: 'vehicle:1', currentOffset: 3.1, stopOffset: 3.58, priority: 1 }),
        request({ vehicleId: 'vehicle:2', currentOffset: 0.05, stopOffset: 3.58, priority: 2 }),
      ],
    });

    const blocked = result.decisions.get('vehicle:2');
    expect(blocked?.kind).toBe('stop');
    expect(blocked?.maxAdvance).toBe(0);
  });
});
