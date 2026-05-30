import type { BackendCar, BackendPedestrian, Coord } from './backendMobilityDrawables';
import { SIM_SECONDS_PER_DAY, SIM_SECONDS_PER_YEAR } from '../backend/simTime';

export type EntityInspectorRow = { label: string; value: string };
export type EntityInspector = { title: string; rows: EntityInspectorRow[] } | null;
const SIM_SECONDS_PER_HOUR = 3600;

export function formatBackendCoord(coord: Coord): string {
  return `${coord.x.toFixed(1)}, ${coord.y.toFixed(1)}`;
}

export function formatAgentLifetime(ageSeconds: number): string {
  const safeAgeSeconds = Number.isFinite(ageSeconds) ? Math.max(0, Math.floor(ageSeconds)) : 0;
  const years = Math.floor(safeAgeSeconds / SIM_SECONDS_PER_YEAR);
  const remainderAfterYears = safeAgeSeconds % SIM_SECONDS_PER_YEAR;
  const days = Math.floor(remainderAfterYears / SIM_SECONDS_PER_DAY);
  const hours = Math.floor((remainderAfterYears % SIM_SECONDS_PER_DAY) / SIM_SECONDS_PER_HOUR);
  if (years > 0) return `${years}yr ${days}d ${hours}h`;
  return `${days}d ${hours}h`;
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
      { label: 'Age', value: formatAgentLifetime(agent.ageSeconds) },
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
