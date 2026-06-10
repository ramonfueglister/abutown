import { describe, expect, it } from 'vitest';
import {
  buildBackendCarInspector,
  buildBackendPedestrianInspector,
  formatBackendCoord,
  formatAgentLifetime,
} from '../../src/render/entityInspector';

describe('entity inspector', () => {
  it('formats backend coordinates with one decimal place', () => {
    expect(formatBackendCoord({ x: 12, y: 3.456 })).toBe('12.0, 3.5');
  });

  it('formats agent lifetime as days and hours before a full year', () => {
    expect(formatAgentLifetime(8 * 86_400 + 10 * 3600 + 59 * 60)).toBe('8d 10h');
  });

  it('formats agent lifetime with years, days, and hours after a full year', () => {
    expect(formatAgentLifetime(2 * 31_536_000 + 3 * 86_400 + 4 * 3600)).toBe('2yr 3d 4h');
  });

  it('builds pedestrian inspector rows', () => {
    expect(buildBackendPedestrianInspector({
      id: 'agent:1',
      path: [{ x: 10, y: 20 }, { x: 11, y: 20 }],
      offset: 0,
      speed: 0,
      laneOffset: 0,
      direction: 'e',
      ageSeconds: 2 * 31_536_000,
      sprite: { sheet: 'minimal-peds.0', frameWidth: 16, frameHeight: 32 },
      kind: 'pedestrian',
      stateType: 'walking',
    })).toEqual({
      title: 'agent:1',
      rows: [
        { label: 'State', value: 'walking' },
        { label: 'Tile', value: '10.0, 20.0' },
        { label: 'Next', value: '11.0, 20.0' },
        { label: 'Direction', value: 'e' },
        { label: 'Age', value: '2yr 0d 0h' },
        { label: 'Sprite', value: 'minimal-peds.0' },
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
      sprite: { sheet: 'minimal-cars.0', frameWidth: 32, frameHeight: 32, scale: 1, role: 'vehicle.0' },
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
