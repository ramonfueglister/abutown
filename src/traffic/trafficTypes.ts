export type TrafficCoord = { x: number; y: number };

export type VehicleId = `vehicle:${number}`;
export type IntersectionId = `intersection:${number}:${number}`;
export type ReservationId = `${IntersectionId}:${VehicleId}:${number}`;

export type TrafficDirection = 'north' | 'east' | 'south' | 'west';
export type TrafficDecisionKind = 'go' | 'yield' | 'stop' | 'blocked';

export type TrafficIntersection = {
  intersectionId: IntersectionId;
  coord: TrafficCoord;
  connectedDirections: readonly TrafficDirection[];
};

export type TrafficReservation = {
  reservationId: ReservationId;
  intersectionId: IntersectionId;
  vehicleId: VehicleId;
  enterTick: number;
  exitTick: number;
  approachEdge: TrafficDirection;
  exitEdge: TrafficDirection;
  conflictMask: number;
  priority: number;
};

export type TrafficVehicleRequest = {
  vehicleId: VehicleId;
  intersectionId: IntersectionId;
  distanceToIntersection: number;
  stopOffset: number;
  currentOffset: number;
  enterTick: number;
  exitTick: number;
  approachEdge: TrafficDirection;
  exitEdge: TrafficDirection;
  conflictMask: number;
  priority: number;
};

export type TrafficDecision = {
  vehicleId: VehicleId;
  kind: TrafficDecisionKind;
  speedFactor: number;
  maxAdvance?: number;
  intersectionId?: IntersectionId;
  reservationId?: ReservationId;
};

export type TrafficRuleDiagnostics = {
  reservedIntersections: number;
  yieldingVehicles: number;
  stoppedForTrafficRules: number;
  blockedVehicles: number;
  intersectionConflictsPrevented: number;
  expiredReservations: number;
  trafficRuleDecisionCount: number;
  unclassifiedTrafficRequests: number;
};

export type TrafficRuleSnapshot = {
  tick: number;
  version: number;
  reservations: readonly TrafficReservation[];
  diagnostics: TrafficRuleDiagnostics;
};

export const EMPTY_TRAFFIC_DIAGNOSTICS = {
  reservedIntersections: 0,
  yieldingVehicles: 0,
  stoppedForTrafficRules: 0,
  blockedVehicles: 0,
  intersectionConflictsPrevented: 0,
  expiredReservations: 0,
  trafficRuleDecisionCount: 0,
  unclassifiedTrafficRequests: 0,
} as const satisfies Readonly<TrafficRuleDiagnostics>;

export function createInitialTrafficRuleSnapshot(): TrafficRuleSnapshot {
  return {
    tick: 0,
    version: 0,
    reservations: [],
    diagnostics: { ...EMPTY_TRAFFIC_DIAGNOSTICS },
  };
}

export function trafficIntersectionId(coord: TrafficCoord): IntersectionId {
  return `intersection:${Math.round(coord.x)}:${Math.round(coord.y)}`;
}

export function trafficReservationId(
  intersectionId: IntersectionId,
  vehicleId: VehicleId,
  enterTick: number,
): ReservationId {
  return `${intersectionId}:${vehicleId}:${enterTick}`;
}

export function trafficKey(coord: TrafficCoord): string {
  return `${Math.round(coord.x)}:${Math.round(coord.y)}`;
}
