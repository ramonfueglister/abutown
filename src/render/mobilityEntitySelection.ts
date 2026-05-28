import type { Coord } from '../cameraController';

export type SelectableMobilityEntity = {
  id: string;
  path: readonly Coord[];
};

export type MobilityEntitySelection = {
  selectedAgentId: string | null;
  selectedVehicleId: string | null;
};

export type MobilityEntitySelectionInput<TPedestrian extends SelectableMobilityEntity, TCar extends SelectableMobilityEntity> = {
  pedestrians: readonly TPedestrian[];
  cars: readonly TCar[];
  worldPoint: Coord;
  cameraScale: number;
  gridToWorld: (coord: Coord) => Coord;
};

export function selectMobilityEntityAtWorldPoint<
  TPedestrian extends SelectableMobilityEntity,
  TCar extends SelectableMobilityEntity,
>(input: MobilityEntitySelectionInput<TPedestrian, TCar>): MobilityEntitySelection {
  const vehicleHit = findNearestProjectedEntity(
    input.cars,
    input.worldPoint,
    Math.max(10, 24 / input.cameraScale),
    input.gridToWorld,
  );
  if (vehicleHit) return { selectedAgentId: null, selectedVehicleId: vehicleHit.id };

  const agentHit = findNearestProjectedEntity(
    input.pedestrians,
    input.worldPoint,
    Math.max(8, 20 / input.cameraScale),
    input.gridToWorld,
  );
  return { selectedAgentId: agentHit?.id ?? null, selectedVehicleId: null };
}

export function findNearestProjectedEntity<T extends SelectableMobilityEntity>(
  entities: readonly T[],
  worldPoint: Coord,
  radius: number,
  gridToWorld: (coord: Coord) => Coord,
): T | null {
  let nearest: { entity: T; distance: number } | null = null;
  for (const entity of entities) {
    const projected = gridToWorld(entity.path[0]);
    const distance = Math.hypot(projected.x - worldPoint.x, projected.y - worldPoint.y);
    if (distance > radius) continue;
    if (!nearest || distance < nearest.distance) nearest = { entity, distance };
  }
  return nearest?.entity ?? null;
}
