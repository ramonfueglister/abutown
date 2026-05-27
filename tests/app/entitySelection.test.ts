import { describe, expect, it } from 'vitest';
import { createEntitySelection, type SelectableEntity } from '../../src/app/entitySelection';

function createEntity(id: string, x: number, y: number): SelectableEntity {
  return { id, path: [{ x, y }] };
}

describe('createEntitySelection', () => {
  it('selects vehicles before pedestrians when both are hit', () => {
    const pedestrian = createEntity('pedestrian-1', 5, 5);
    const vehicle = createEntity('vehicle-1', 5, 5);
    const selection = createEntitySelection({
      getPedestrians: () => [pedestrian],
      getVehicles: () => [vehicle],
      screenToWorld: (point) => point,
      projectPedestrian: (entity) => entity.path[0],
      projectVehicle: (entity) => entity.path[0],
      pedestrianRadius: () => 10,
      vehicleRadius: () => 10,
    });

    selection.selectAtScreenPoint({ x: 5, y: 5 });

    expect(selection.selectedVehicleId()).toBe('vehicle-1');
    expect(selection.selectedVehicle()).toBe(vehicle);
    expect(selection.selectedAgentId()).toBeNull();
    expect(selection.selectedPedestrian()).toBeNull();
  });

  it('selects pedestrians and clears vehicle selection when no vehicle is hit', () => {
    const pedestrian = createEntity('pedestrian-1', 5, 12);
    const vehicle = createEntity('vehicle-1', 5, 5);
    const selection = createEntitySelection({
      getPedestrians: () => [pedestrian],
      getVehicles: () => [vehicle],
      screenToWorld: (point) => point,
      projectPedestrian: (entity) => entity.path[0],
      projectVehicle: (entity) => entity.path[0],
      pedestrianRadius: () => 4,
      vehicleRadius: () => 4,
    });

    selection.selectAtScreenPoint({ x: 5, y: 5 });
    selection.selectAtScreenPoint({ x: 5, y: 12 });

    expect(selection.selectedAgentId()).toBe('pedestrian-1');
    expect(selection.selectedPedestrian()).toBe(pedestrian);
    expect(selection.selectedVehicleId()).toBeNull();
    expect(selection.selectedVehicle()).toBeNull();
  });

  it('clears both selections when no entity is hit', () => {
    const pedestrian = createEntity('pedestrian-1', 5, 5);
    const vehicle = createEntity('vehicle-1', 5, 5);
    const selection = createEntitySelection({
      getPedestrians: () => [pedestrian],
      getVehicles: () => [vehicle],
      screenToWorld: (point) => point,
      projectPedestrian: (entity) => entity.path[0],
      projectVehicle: (entity) => entity.path[0],
      pedestrianRadius: () => 10,
      vehicleRadius: () => 10,
    });

    selection.selectAtScreenPoint({ x: 5, y: 5 });
    selection.selectAtScreenPoint({ x: 50, y: 50 });

    expect(selection.selectedAgentId()).toBeNull();
    expect(selection.selectedPedestrian()).toBeNull();
    expect(selection.selectedVehicleId()).toBeNull();
    expect(selection.selectedVehicle()).toBeNull();
  });
});
