import { describe, expect, it } from 'vitest';
import { buildRoadVehicleInspector } from '../../src/render/roadVehicleInspector';
import type { LocalRoadVehicle } from '../../src/render/localRoadVehicles';

const vehicle: LocalRoadVehicle = {
  id: 'vehicle:road:12',
  kind: 'road-vehicle',
  state: 'driving',
  coord: { x: 42.25, y: 18.75 },
  pathIndex: 7,
  nextCoord: { x: 43, y: 19 },
  speed: 1.734,
  spriteSheet: 'truck',
  role: 'vehicle.truck',
};

describe('road vehicle inspector', () => {
  it('returns null when no road vehicle is selected', () => {
    expect(buildRoadVehicleInspector(null)).toBeNull();
  });

  it('formats compact rows for the selected road vehicle', () => {
    expect(buildRoadVehicleInspector(vehicle)).toEqual({
      title: 'vehicle:road:12',
      rows: [
        { label: 'State', value: 'driving' },
        { label: 'Tile', value: '42.3, 18.8' },
        { label: 'Next', value: '43.0, 19.0' },
        { label: 'Speed', value: '1.73' },
        { label: 'Sprite', value: 'truck' },
      ],
    });
  });
});
