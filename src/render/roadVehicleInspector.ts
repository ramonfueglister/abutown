import type { LocalRoadVehicle } from './localRoadVehicles';

export type RoadVehicleInspectorRow = {
  label: string;
  value: string;
};

export type RoadVehicleInspector = {
  title: string;
  rows: RoadVehicleInspectorRow[];
};

export function buildRoadVehicleInspector(vehicle: LocalRoadVehicle | null): RoadVehicleInspector | null {
  if (!vehicle) return null;
  return {
    title: vehicle.id,
    rows: [
      { label: 'State', value: vehicle.state },
      { label: 'Tile', value: formatCoord(vehicle.coord) },
      { label: 'Next', value: formatCoord(vehicle.nextCoord) },
      { label: 'Speed', value: vehicle.speed.toFixed(2) },
      { label: 'Sprite', value: vehicle.spriteSheet },
    ],
  };
}

function formatCoord(coord: { x: number; y: number }): string {
  return `${coord.x.toFixed(1)}, ${coord.y.toFixed(1)}`;
}
