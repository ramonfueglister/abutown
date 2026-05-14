import {
  EMPTY_TRAFFIC_DIAGNOSTICS,
  type TrafficDecision,
  type TrafficReservation,
  type TrafficRuleDiagnostics,
  type TrafficRuleSnapshot,
  type TrafficVehicleRequest,
  trafficReservationId,
} from './trafficTypes';

export type StepLocalTrafficAuthorityInput = {
  snapshot: TrafficRuleSnapshot;
  tick: number;
  requests: readonly TrafficVehicleRequest[];
};

export type StepLocalTrafficAuthorityResult = {
  snapshot: TrafficRuleSnapshot;
  decisions: Map<string, TrafficDecision>;
  diagnostics: TrafficRuleDiagnostics;
};

const NEAR_STOP_BOUNDARY_TILES = 0.16;
const NEAR_INTERSECTION_TILES = 0.48;
const YIELD_SPEED_FACTOR = 0.28;

export function stepLocalTrafficAuthority(input: StepLocalTrafficAuthorityInput): StepLocalTrafficAuthorityResult {
  const diagnostics: TrafficRuleDiagnostics = { ...EMPTY_TRAFFIC_DIAGNOSTICS };
  const activeReservations = input.snapshot.reservations.filter((reservation) => {
    const active = reservation.exitTick > input.tick;
    if (!active) diagnostics.expiredReservations += 1;
    return active;
  });
  const nextReservations: TrafficReservation[] = [...activeReservations];
  const decisions = new Map<string, TrafficDecision>();

  for (const request of sortedTrafficRequests(input.requests)) {
    diagnostics.trafficRuleDecisionCount += 1;

    const existing = nextReservations.find((reservation) => reservation.vehicleId === request.vehicleId);
    if (existing) {
      decisions.set(request.vehicleId, goDecision(request, existing.reservationId));
      continue;
    }

    const conflict = nextReservations.find((reservation) => reservationsConflict(reservation, request));
    if (conflict) {
      diagnostics.intersectionConflictsPrevented += 1;
      diagnostics.blockedVehicles += 1;

      const stopAdvance = distanceToStopBoundary(request.currentOffset, request.stopOffset);
      const mustStop = stopAdvance <= NEAR_STOP_BOUNDARY_TILES || request.distanceToIntersection <= NEAR_INTERSECTION_TILES;
      if (mustStop) {
        diagnostics.stoppedForTrafficRules += 1;
        decisions.set(request.vehicleId, {
          vehicleId: request.vehicleId,
          kind: 'stop',
          speedFactor: 0,
          maxAdvance: roundTileDistance(stopAdvance),
          intersectionId: request.intersectionId,
        });
      } else {
        diagnostics.yieldingVehicles += 1;
        decisions.set(request.vehicleId, {
          vehicleId: request.vehicleId,
          kind: 'yield',
          speedFactor: YIELD_SPEED_FACTOR,
          intersectionId: request.intersectionId,
        });
      }
      continue;
    }

    const reservation = reservationForRequest(request);
    nextReservations.push(reservation);
    decisions.set(request.vehicleId, goDecision(request, reservation.reservationId));
  }

  diagnostics.reservedIntersections = new Set(
    nextReservations.map((reservation) => reservation.intersectionId),
  ).size;

  return {
    snapshot: {
      tick: input.tick,
      version: input.snapshot.version + 1,
      reservations: nextReservations,
      diagnostics,
    },
    decisions,
    diagnostics,
  };
}

function sortedTrafficRequests(requests: readonly TrafficVehicleRequest[]): TrafficVehicleRequest[] {
  return [...requests].sort((a, b) =>
    a.enterTick - b.enterTick ||
    a.priority - b.priority ||
    a.vehicleId.localeCompare(b.vehicleId)
  );
}

function reservationForRequest(request: TrafficVehicleRequest): TrafficReservation {
  return {
    reservationId: trafficReservationId(request.intersectionId, request.vehicleId, request.enterTick),
    intersectionId: request.intersectionId,
    vehicleId: request.vehicleId,
    enterTick: request.enterTick,
    exitTick: request.exitTick,
    approachEdge: request.approachEdge,
    exitEdge: request.exitEdge,
    conflictMask: request.conflictMask,
    priority: request.priority,
  };
}

function goDecision(request: TrafficVehicleRequest, reservationId: TrafficReservation['reservationId']): TrafficDecision {
  return {
    vehicleId: request.vehicleId,
    kind: 'go',
    speedFactor: 1,
    intersectionId: request.intersectionId,
    reservationId,
  };
}

function reservationsConflict(reservation: TrafficReservation, request: TrafficVehicleRequest): boolean {
  return (
    reservation.intersectionId === request.intersectionId &&
    windowsOverlap(reservation.enterTick, reservation.exitTick, request.enterTick, request.exitTick) &&
    (reservation.conflictMask & request.conflictMask) !== 0
  );
}

function windowsOverlap(aStart: number, aEnd: number, bStart: number, bEnd: number): boolean {
  return aStart < bEnd && bStart < aEnd;
}

function distanceToStopBoundary(currentOffset: number, stopOffset: number): number {
  if (stopOffset < currentOffset) return 0;
  return stopOffset - currentOffset;
}

function roundTileDistance(distance: number): number {
  return Math.max(0, Number(distance.toFixed(3)));
}
