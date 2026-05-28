import { describe, expect, it } from 'vitest';
import {
  buildBackendCarInspector,
  buildBackendPedestrianInspector,
  formatBackendCoord,
} from '../../src/render/entityInspector';

describe('entity inspector', () => {
  it('formats backend coordinates with one decimal place', () => {
    expect(formatBackendCoord({ x: 12, y: 3.456 })).toBe('12.0, 3.5');
  });

  it('builds pedestrian inspector rows', () => {
    expect(buildBackendPedestrianInspector({
      id: 'agent:1',
      path: [{ x: 10, y: 20 }, { x: 11, y: 20 }],
      offset: 0,
      speed: 0,
      laneOffset: 0,
      direction: 'e',
      sprite: { sheet: 'pak128/peds.0', frameWidth: 16, frameHeight: 32 },
    })).toEqual({
      title: 'agent:1',
      rows: [
        { label: 'State', value: 'walking' },
        { label: 'Tile', value: '10.0, 20.0' },
        { label: 'Next', value: '11.0, 20.0' },
        { label: 'Direction', value: 'e' },
        { label: 'Sprite', value: 'pak128/peds.0' },
      ],
    });
  });

  it('builds car inspector rows and falls back to current tile when next is missing', () => {
    expect(buildBackendCarInspector({
      id: 'vehicle:1',
      path: [{ x: 7.25, y: 8.75 }],
      offset: 0,
      speed: 0,
      direction: 'nw',
      sprite: { sheet: 'pak128/cars.0', frameWidth: 32, frameHeight: 32, scale: 1, role: 'vehicle.0' },
    })).toEqual({
      title: 'vehicle:1',
      rows: [
        { label: 'State', value: 'driving' },
        { label: 'Tile', value: '7.3, 8.8' },
        { label: 'Next', value: '7.3, 8.8' },
        { label: 'Direction', value: 'nw' },
        { label: 'Sprite', value: 'vehicle.0' },
      ],
    });
  });

  it('returns null when no entity is selected', () => {
    expect(buildBackendPedestrianInspector(null)).toBeNull();
    expect(buildBackendCarInspector(null)).toBeNull();
  });
});
