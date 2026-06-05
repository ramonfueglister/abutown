export type Coord = { x: number; y: number };

export type SelectableEntity = {
  id: string;
  path: Coord[];
};

export type MarketCoord = { x: number; y: number };

export type EntitySelectionOptions<P extends SelectableEntity, V extends SelectableEntity> = {
  getPedestrians: () => readonly P[];
  getVehicles: () => readonly V[];
  getMarkets: () => readonly MarketCoord[];
  screenToWorld: (point: Coord) => Coord;
  projectPedestrian: (entity: P) => Coord;
  projectVehicle: (entity: V) => Coord;
  projectMarket: (market: MarketCoord) => Coord;
  pedestrianRadius: () => number;
  vehicleRadius: () => number;
  marketRadius: () => number;
};

export type EntitySelection<P extends SelectableEntity, V extends SelectableEntity> = {
  selectAtScreenPoint: (point: Coord) => void;
  selectedAgentId: () => string | null;
  selectedVehicleId: () => string | null;
  selectedMarketCoord: () => MarketCoord | null;
  selectedPedestrian: () => P | null;
  selectedVehicle: () => V | null;
};

export function createEntitySelection<P extends SelectableEntity, V extends SelectableEntity>(
  options: EntitySelectionOptions<P, V>,
): EntitySelection<P, V> {
  let selectedAgentId: string | null = null;
  let selectedVehicleId: string | null = null;
  let _selectedMarketCoord: MarketCoord | null = null;

  return {
    selectAtScreenPoint: (point) => {
      const worldPoint = options.screenToWorld(point);

      const marketHit = findNearestMarket(
        options.getMarkets(),
        worldPoint,
        options.marketRadius(),
        options.projectMarket,
      );
      if (marketHit) {
        _selectedMarketCoord = marketHit;
        selectedAgentId = null;
        selectedVehicleId = null;
        return;
      }

      const vehicleHit = findNearestProjectedEntity(
        options.getVehicles(),
        worldPoint,
        options.vehicleRadius(),
        options.projectVehicle,
      );
      if (vehicleHit) {
        selectedVehicleId = vehicleHit.id;
        selectedAgentId = null;
        _selectedMarketCoord = null;
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
      _selectedMarketCoord = null;
    },
    selectedAgentId: () => selectedAgentId,
    selectedVehicleId: () => selectedVehicleId,
    selectedMarketCoord: () => _selectedMarketCoord,
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

export function findNearestMarket(
  markets: readonly MarketCoord[],
  worldPoint: Coord,
  radius: number,
  projectMarket: (market: MarketCoord) => Coord,
): MarketCoord | null {
  let nearest: { market: MarketCoord; distance: number } | null = null;
  for (const market of markets) {
    const projected = projectMarket(market);
    const distance = Math.hypot(projected.x - worldPoint.x, projected.y - worldPoint.y);
    if (distance > radius) continue;
    if (!nearest || distance < nearest.distance) nearest = { market, distance };
  }
  return nearest?.market ?? null;
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
