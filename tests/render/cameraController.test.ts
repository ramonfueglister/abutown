import { describe, expect, it } from 'vitest';
import {
  constrainCameraTargetToGrid,
  createCameraState,
  dampCamera,
  screenToWorld,
  zoomCameraAt,
} from '../../src/cameraController';

const viewport = { width: 1000, height: 700 };
const iso = (coord: { x: number; y: number }) => ({ x: (coord.x - coord.y) * 32, y: (coord.x + coord.y) * 16 });
const worldToGrid = (point: { x: number; y: number }) => {
  const projectedX = point.x / 32;
  const projectedY = point.y / 16;
  return { x: (projectedY + projectedX) / 2, y: (projectedY - projectedX) / 2 };
};

describe('cameraController', () => {
  it('keeps hard-constrained target center inside the fixed map bounds', () => {
    const camera = createCameraState({ x: 500, y: 350, scale: 1 });
    camera.targetX = 500 - iso({ x: -40, y: -30 }).x;
    camera.targetY = 350 - iso({ x: -40, y: -30 }).y;

    constrainCameraTargetToGrid(camera, viewport, worldToGrid, iso, {
      minX: -8,
      maxX: 103,
      minY: -8,
      maxY: 85,
      softness: 4,
      allowOverscroll: false,
    });

    const center = worldToGrid(screenToWorld(camera, { x: 500, y: 350 }, 'target'));
    expect(center.x).toBeGreaterThanOrEqual(-8);
    expect(center.y).toBeGreaterThanOrEqual(-8);
  });

  it('allows limited overscroll while dragging but damps it near the edge', () => {
    const camera = createCameraState({ x: 500, y: 350, scale: 1 });
    camera.targetX = 500 - iso({ x: -40, y: 40 }).x;
    camera.targetY = 350 - iso({ x: -40, y: 40 }).y;

    constrainCameraTargetToGrid(camera, viewport, worldToGrid, iso, {
      minX: -8,
      maxX: 103,
      minY: -8,
      maxY: 85,
      softness: 4,
      allowOverscroll: true,
    });

    const center = worldToGrid(screenToWorld(camera, { x: 500, y: 350 }, 'target'));
    expect(center.x).toBeLessThan(-8);
    expect(center.x).toBeGreaterThan(-13);
  });

  it('zooms around the pointer on the target camera', () => {
    const camera = createCameraState({ x: 120, y: 80, scale: 1 });
    const pointer = { x: 420, y: 260 };
    const before = screenToWorld(camera, pointer, 'target');

    zoomCameraAt(camera, pointer, -120, 0, { minScale: 0.5, maxScale: 3.2 });

    const after = screenToWorld(camera, pointer, 'target');
    expect(after.x).toBeCloseTo(before.x, 6);
    expect(after.y).toBeCloseTo(before.y, 6);
    expect(camera.targetScale).toBeGreaterThan(1);
  });

  it('damps current camera values toward target values', () => {
    const camera = createCameraState({ x: 0, y: 0, scale: 1 });
    camera.targetX = 100;
    camera.targetY = 50;
    camera.targetScale = 2;

    dampCamera(camera, 0.016, 18);

    expect(camera.x).toBeGreaterThan(0);
    expect(camera.x).toBeLessThan(100);
    expect(camera.scale).toBeGreaterThan(1);
    expect(camera.scale).toBeLessThan(2);
  });
});
