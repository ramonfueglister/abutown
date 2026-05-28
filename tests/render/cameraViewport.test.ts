import { describe, expect, it } from 'vitest';
import { createCameraState } from '../../src/cameraController';
import {
  chooseInitialCameraFocus,
  initializeCameraForGridFocus,
  isCoordVisibleInGridRect,
  visibleGridRectForCamera,
} from '../../src/render/cameraViewport';

const viewport = { width: 1000, height: 700 };
const iso = (coord: { x: number; y: number }) => ({ x: (coord.x - coord.y) * 32, y: (coord.x + coord.y) * 16 });
const worldToGrid = (point: { x: number; y: number }) => {
  const projectedX = point.x / 32;
  const projectedY = point.y / 16;
  return { x: (projectedY + projectedX) / 2, y: (projectedY - projectedX) / 2 };
};

describe('cameraViewport', () => {
  it('chooses the center when no mobility coordinates are inside the map', () => {
    expect(chooseInitialCameraFocus([{ x: -1, y: 12 }, { x: 20, y: 300 }], { width: 256, height: 128 }))
      .toEqual({ x: 128, y: 64 });
  });

  it('blends the average in-bounds mobility coordinate toward the map center', () => {
    const focus = chooseInitialCameraFocus(
      [{ x: 20, y: 60 }, { x: 100, y: 20 }, { x: -4, y: 10 }],
      { width: 200, height: 100 },
    );

    expect(focus.x).toBeCloseTo(74);
    expect(focus.y).toBeCloseTo(43.5);
  });

  it('initializes the current and target camera around the grid focus using the map vertical anchor', () => {
    const camera = createCameraState({ x: 0, y: 0, scale: 0.5 });

    initializeCameraForGridFocus(camera, { x: 10, y: 20 }, viewport, iso, { verticalAnchor: 0.52 });

    expect(camera.targetX).toBe(500 - iso({ x: 10, y: 20 }).x * 0.5);
    expect(camera.targetY).toBe(700 * 0.52 - iso({ x: 10, y: 20 }).y * 0.5);
    expect(camera.x).toBe(camera.targetX);
    expect(camera.y).toBe(camera.targetY);
    expect(camera.scale).toBe(camera.targetScale);
  });

  it('builds a padded grid rect from the four viewport corners', () => {
    const camera = createCameraState({ x: 500, y: 350, scale: 1 });
    const rect = visibleGridRectForCamera(camera, viewport, worldToGrid, 9);

    expect(rect).toEqual({
      minX: Math.floor(Math.min(
        worldToGrid({ x: -500, y: -350 }).x,
        worldToGrid({ x: 500, y: -350 }).x,
        worldToGrid({ x: -500, y: 350 }).x,
        worldToGrid({ x: 500, y: 350 }).x,
      )) - 9,
      maxX: Math.ceil(Math.max(
        worldToGrid({ x: -500, y: -350 }).x,
        worldToGrid({ x: 500, y: -350 }).x,
        worldToGrid({ x: -500, y: 350 }).x,
        worldToGrid({ x: 500, y: 350 }).x,
      )) + 9,
      minY: Math.floor(Math.min(
        worldToGrid({ x: -500, y: -350 }).y,
        worldToGrid({ x: 500, y: -350 }).y,
        worldToGrid({ x: -500, y: 350 }).y,
        worldToGrid({ x: 500, y: 350 }).y,
      )) - 9,
      maxY: Math.ceil(Math.max(
        worldToGrid({ x: -500, y: -350 }).y,
        worldToGrid({ x: 500, y: -350 }).y,
        worldToGrid({ x: -500, y: 350 }).y,
        worldToGrid({ x: 500, y: 350 }).y,
      )) + 9,
    });
  });

  it('checks whether a grid coordinate is inside a visible grid rect', () => {
    const rect = { minX: 2, maxX: 6, minY: 3, maxY: 8 };

    expect(isCoordVisibleInGridRect({ x: 2, y: 8 }, rect)).toBe(true);
    expect(isCoordVisibleInGridRect({ x: 7, y: 8 }, rect)).toBe(false);
    expect(isCoordVisibleInGridRect({ x: 3, y: 2 }, rect)).toBe(false);
  });
});
