import type { BackendCar, BackendPedestrian, Coord } from './backendMobilityDrawables';

export type EntityInspectorRow = { label: string; value: string };
export type EntityInspector = { title: string; rows: EntityInspectorRow[] } | null;

export function formatBackendCoord(coord: Coord): string {
  return `${coord.x.toFixed(1)}, ${coord.y.toFixed(1)}`;
}

export function buildBackendPedestrianInspector(agent: BackendPedestrian | null): EntityInspector {
  if (!agent) return null;
  return {
    title: agent.id,
    rows: [
      { label: 'State', value: 'walking' },
      { label: 'Tile', value: formatBackendCoord(agent.path[0]) },
      { label: 'Next', value: formatBackendCoord(agent.path[1] ?? agent.path[0]) },
      { label: 'Direction', value: agent.direction },
      { label: 'Sprite', value: agent.sprite.sheet },
    ],
  };
}

export function buildBackendCarInspector(vehicle: BackendCar | null): EntityInspector {
  if (!vehicle) return null;
  return {
    title: vehicle.id,
    rows: [
      { label: 'State', value: 'driving' },
      { label: 'Tile', value: formatBackendCoord(vehicle.path[0]) },
      { label: 'Next', value: formatBackendCoord(vehicle.path[1] ?? vehicle.path[0]) },
      { label: 'Direction', value: vehicle.direction },
      { label: 'Sprite', value: vehicle.sprite.role },
    ],
  };
}
