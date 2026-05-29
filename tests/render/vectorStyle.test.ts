import { describe, expect, it } from 'vitest';
import {
  buildingJitter,
  buildingVectorColor,
  treeRenderStyle,
  vehicleVectorColor,
} from '../../src/render/vectorStyle';

describe('vectorStyle', () => {
  it('chooses building colors from sheet and district metadata', () => {
    expect(buildingVectorColor({ sheet: 'church', district: 'old-town' })).toBe('#dccb9a');
    expect(buildingVectorColor({ sheet: 'office', district: 'center' })).toBe('#c9d8dc');
    expect(buildingVectorColor({ sheet: 'houses', district: 'mill-yard' })).toBe('#cabed6');
    expect(buildingVectorColor({ sheet: 'houses', district: 'suburb' })).toBe('#d8cfbf');
  });

  it('keeps building jitter stable from district and rounded coord', () => {
    expect(buildingJitter({ district: 'mill-yard', coord: { x: 10.2, y: 14.7 } })).toEqual({
      x: -0.52,
      y: -0.26,
    });
  });

  it('keeps vehicle colors stable from vehicle id', () => {
    expect(vehicleVectorColor('car:1')).toBe('#e5a944');
    expect(vehicleVectorColor('vehicle:bus:42')).toBe('#e85d75');
  });

  it('returns tree jitter, alpha, and low-zoom visibility', () => {
    expect(treeRenderStyle({ coord: { x: 12, y: 20 }, cameraScale: 0.2, terrainBase: 'Forest' })).toEqual({
      visible: true,
      jitter: { x: 0.76, y: 0.76 },
      alpha: 0.72,
    });

    expect(treeRenderStyle({ coord: { x: 13, y: 20 }, cameraScale: 0.2, terrainBase: 'Grass' })).toEqual({
      visible: false,
      jitter: { x: 0, y: 0 },
      alpha: 0,
    });
  });
});
