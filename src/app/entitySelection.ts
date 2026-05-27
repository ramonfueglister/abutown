export type Coord = { x: number; y: number };

export type SelectableEntity = {
  id: string;
  path: Coord[];
};

export type EntitySelectionOptions<P extends SelectableEntity, V extends SelectableEntity> = {
  getPedestrians: () => readonly P[];
  getVehicles: () => readonly V[];
  screenToWorld: (point: Coord) => Coord;
  projectPedestrian: (entity: P) => Coord;
  projectVehicle: (entity: V) => Coord;
  pedestrianRadius: () => number;
  vehicleRadius: () => number;
};

export type EntitySelection<P extends SelectableEntity, V extends SelectableEntity> = {
  selectAtScreenPoint: (point: Coord) => void;
  selectedAgentId: () => string | null;
  selectedVehicleId: () => string | null;
  selectedPedestrian: () => P | null;
  selectedVehicle: () => V | null;
};

export function createEntitySelection<P extends SelectableEntity, V extends SelectableEntity>(
  options: EntitySelectionOptions<P, V>,
): EntitySelection<P, V> {
  let selectedAgentId: string | null = null;
  let selectedVehicleId: string | null = null;

  return {
    selectAtScreenPoint: (point) => {
      const worldPoint = options.screenToWorld(point);
      const vehicleHit = findNearestProjectedEntity(
        options.getVehicles(),
        worldPoint,
        options.vehicleRadius(),
        options.projectVehicle,
      );
      if (vehicleHit) {
        selectedVehicleId = vehicleHit.id;
        selectedAgentId = null;
        return;
      }

      const pedestrianHit = findNearestProjectedEntity(
        options.getPedestrians(),
        worldPoint,
        options.pedestrianRadius(),
        options.projectPedestrian,
      );
      selectedAgentId = pedestrianHit?.id ?? null;
      selectedVehicleId = null;
    },
    selectedAgentId: () => selectedAgentId,
    selectedVehicleId: () => selectedVehicleId,
    selectedPedestrian: () => {
      if (!selectedAgentId) return null;
      return options.getPedestrians().find((entity) => entity.id === selectedAgentId) ?? null;
    },
    selectedVehicle: () => {
      if (!selectedVehicleId) return null;
      return options.getVehicles().find((entity) => entity.id === selectedVehicleId) ?? null;
    },
  };
}

export function findNearestProjectedEntity<T extends SelectableEntity>(
  entities: readonly T[],
  worldPoint: Coord,
  radius: number,
  projectedPoint: (entity: T) => Coord,
): T | null {
  let nearest: { entity: T; distance: number } | null = null;
  for (const entity of entities) {
    const projected = projectedPoint(entity);
    const distance = Math.hypot(projected.x - worldPoint.x, projected.y - worldPoint.y);
    if (distance > radius) continue;
    if (!nearest || distance < nearest.distance) nearest = { entity, distance };
  }
  return nearest?.entity ?? null;
}
