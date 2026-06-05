import { describe, expect, it } from 'vitest';
import { createEntitySelection, type SelectableEntity, type MarketCoord } from '../../src/app/entitySelection';

function createEntity(id: string, x: number, y: number): SelectableEntity {
  return { id, path: [{ x, y }] };
}

function createMarket(x: number, y: number): MarketCoord {
  return { x, y };
}

function makeSelection(options?: {
  pedestrians?: SelectableEntity[];
  vehicles?: SelectableEntity[];
  markets?: MarketCoord[];
}) {
  const pedestrians = options?.pedestrians ?? [];
  const vehicles = options?.vehicles ?? [];
  const markets = options?.markets ?? [];
  return createEntitySelection({
    getPedestrians: () => pedestrians,
    getVehicles: () => vehicles,
    getMarkets: () => markets,
    screenToWorld: (point) => point,
    projectPedestrian: (entity) => entity.path[0],
    projectVehicle: (entity) => entity.path[0],
    projectMarket: (market) => market,
    pedestrianRadius: () => 10,
    vehicleRadius: () => 10,
    marketRadius: () => 10,
  });
}

describe('createEntitySelection', () => {
  it('selects vehicles before pedestrians when both are hit', () => {
    const pedestrian = createEntity('pedestrian-1', 5, 5);
    const vehicle = createEntity('vehicle-1', 5, 5);
    const selection = makeSelection({ pedestrians: [pedestrian], vehicles: [vehicle] });

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
      getMarkets: () => [],
      screenToWorld: (point) => point,
      projectPedestrian: (entity) => entity.path[0],
      projectVehicle: (entity) => entity.path[0],
      projectMarket: (market) => market,
      pedestrianRadius: () => 4,
      vehicleRadius: () => 4,
      marketRadius: () => 4,
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
    const selection = makeSelection({ pedestrians: [pedestrian], vehicles: [vehicle] });

    selection.selectAtScreenPoint({ x: 5, y: 5 });
    selection.selectAtScreenPoint({ x: 50, y: 50 });

    expect(selection.selectedAgentId()).toBeNull();
    expect(selection.selectedPedestrian()).toBeNull();
    expect(selection.selectedVehicleId()).toBeNull();
    expect(selection.selectedVehicle()).toBeNull();
  });

  it('clicking near a market tile sets selectedMarketCoord and clears agent/vehicle', () => {
    const pedestrian = createEntity('pedestrian-1', 5, 5);
    const vehicle = createEntity('vehicle-1', 5, 5);
    const market = createMarket(20, 20);
    const selection = makeSelection({ pedestrians: [pedestrian], vehicles: [vehicle], markets: [market] });

    // First select a vehicle
    selection.selectAtScreenPoint({ x: 5, y: 5 });
    expect(selection.selectedVehicleId()).toBe('vehicle-1');

    // Now click near the market
    selection.selectAtScreenPoint({ x: 20, y: 20 });

    expect(selection.selectedMarketCoord()).toEqual({ x: 20, y: 20 });
    expect(selection.selectedAgentId()).toBeNull();
    expect(selection.selectedVehicleId()).toBeNull();
  });

  it('market selection takes priority over collocated vehicle and agent', () => {
    const pedestrian = createEntity('pedestrian-1', 5, 5);
    const vehicle = createEntity('vehicle-1', 5, 5);
    const market = createMarket(5, 5);
    const selection = makeSelection({ pedestrians: [pedestrian], vehicles: [vehicle], markets: [market] });

    selection.selectAtScreenPoint({ x: 5, y: 5 });

    expect(selection.selectedMarketCoord()).toEqual({ x: 5, y: 5 });
    expect(selection.selectedAgentId()).toBeNull();
    expect(selection.selectedVehicleId()).toBeNull();
  });

  it('clicking an agent clears selectedMarketCoord', () => {
    const pedestrian = createEntity('pedestrian-1', 5, 5);
    const market = createMarket(20, 20);
    const selection = makeSelection({ pedestrians: [pedestrian], markets: [market] });

    // Select the market first
    selection.selectAtScreenPoint({ x: 20, y: 20 });
    expect(selection.selectedMarketCoord()).toEqual({ x: 20, y: 20 });

    // Now click the agent — market is out of radius
    selection.selectAtScreenPoint({ x: 5, y: 5 });

    expect(selection.selectedAgentId()).toBe('pedestrian-1');
    expect(selection.selectedMarketCoord()).toBeNull();
  });

  it('selectedMarketCoord is null initially', () => {
    const selection = makeSelection();
    expect(selection.selectedMarketCoord()).toBeNull();
  });
});
