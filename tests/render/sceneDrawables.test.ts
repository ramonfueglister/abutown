import { describe, expect, it } from 'vitest';
import {
  buildVisibleCarDrawables,
  buildVisiblePedestrianDrawables,
  buildVisibleTrainDrawables,
  compareGridDrawables,
  visibleTerrainCoords,
} from '../../src/render/sceneDrawables';

const gridToWorld = (coord: { x: number; y: number }) => ({ x: coord.x, y: coord.y * 10 + coord.x });

describe('sceneDrawables', () => {
  it('clips visible terrain coords to map bounds and sorts by projected y then x', () => {
    const coords = visibleTerrainCoords(
      { minX: -2, maxX: 2, minY: 1, maxY: 3 },
      { width: 3, height: 3 },
      gridToWorld,
    );

    expect(coords).toEqual([
      { x: 0, y: 1 },
      { x: 1, y: 1 },
      { x: 2, y: 1 },
      { x: 0, y: 2 },
      { x: 1, y: 2 },
      { x: 2, y: 2 },
    ]);
  });

  it('orders grid drawables through the shared render ordering rules', () => {
    const items = [
      { type: 'pedestrian' as const, coord: { x: 1, y: 1 } },
      { type: 'road' as const, coord: { x: 9, y: 9 } },
      { type: 'car' as const, coord: { x: 0, y: 0 } },
    ].sort((a, b) => compareGridDrawables(a, b, gridToWorld));

    expect(items.map((item) => item.type)).toEqual(['road', 'car', 'pedestrian']);
  });

  it('builds sorted visible car drawables with vehicle ids', () => {
    const cars = [
      { id: 'car:late', path: [{ x: 2, y: 2 }] },
      { id: 'car:hidden', path: [{ x: 6, y: 6 }] },
      { id: 'car:first', path: [{ x: 1, y: 1 }] },
    ];

    const drawables = buildVisibleCarDrawables(cars, { minX: 0, maxX: 3, minY: 0, maxY: 3 }, gridToWorld);

    expect(drawables.map((item) => item.vehicleId)).toEqual(['car:first', 'car:late']);
    expect(drawables[0].car).toBe(cars[2]);
  });

  it('builds sorted visible pedestrian drawables with agent ids', () => {
    const pedestrians = [
      { id: 'agent:late', path: [{ x: 3, y: 3 }] },
      { id: 'agent:first', path: [{ x: 0, y: 1 }] },
      { id: 'agent:hidden', path: [{ x: -1, y: 1 }] },
    ];

    const drawables = buildVisiblePedestrianDrawables(pedestrians, { minX: 0, maxX: 3, minY: 0, maxY: 3 }, gridToWorld);

    expect(drawables.map((item) => item.agentId)).toEqual(['agent:first', 'agent:late']);
    expect(drawables[1].pedestrian).toBe(pedestrians[0]);
  });

  it('builds sorted visible train drawables from computed train positions', () => {
    const trains = [
      { id: 'train:hidden', coord: { x: 4, y: 4 } },
      { id: 'train:late', coord: { x: 2, y: 2 } },
      { id: 'train:first', coord: { x: 0, y: 1 } },
    ];

    const drawables = buildVisibleTrainDrawables(
      trains,
      (train) => train.coord,
      { minX: 0, maxX: 2, minY: 0, maxY: 2 },
      gridToWorld,
    );

    expect(drawables.map((item) => item.train.id)).toEqual(['train:first', 'train:late']);
  });
});
