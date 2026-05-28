import { expect, test } from '@playwright/test';

type ScreenEntity = { id: string; screen: { x: number; y: number } };

const backendBaseUrl = process.env.E2E_BACKEND_URL ?? 'http://127.0.0.1:18080';

test('renders the city with a bounded fixed-map camera', async ({ page }) => {
  await page.setViewportSize({ width: 409, height: 519 });
  const consoleErrors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text());
  });

  await page.goto('/');
  await expect(page.locator('#game')).toHaveAttribute('data-ready', 'true');
  const raw = await page.evaluate(() => window.render_game_to_text?.() ?? '');
  const state = JSON.parse(raw);

  expect(state.coordinateSystem).toBe('grid origin north-west, x east, y south, top-down minimal map projection');
  expect(state.city.roadTiles).toBeGreaterThan(0);
  expect(state.city.buildings).toBeGreaterThan(0);
  expect(state.city.trains).toBe(1);
  expect(state.city.worldId).toBe('zurich-river-city-v1');
  expect(state.city.terrainSource).toBe('backend-layered');
  expect(state.city.layeredTerrain.loadedTiles).toBeGreaterThan(0);
  expect(state.city.layeredTerrain.loadedTiles).toBeLessThanOrEqual(256 * 256);
  expect(state.city.visualStyle).toEqual({
    id: 'minimal-motorways',
    renderer: 'canvas-vector',
    spriteDrawing: 'disabled',
  });
  expect(state.city.assetPack).toEqual({
    id: 'minimal-vector',
    tile: { width: 18, height: 18 },
  });
  expect(state.city.nonPak128AssetPaths).toEqual([]);
  expect(state.city.width).toBe(256);
  expect(state.city.height).toBe(256);
  expect(state.city.roadTiles).toBeGreaterThan(1800);
  expect(state.city.bridges).toBeGreaterThanOrEqual(6);
  expect(state.city.bridges).toBeLessThanOrEqual(12);
  expect(state.city.railTiles).toBe(256);
  expect(state.city.buildings).toBeGreaterThan(2250);
  expect(state.city.details.total).toBeGreaterThanOrEqual(260);
  expect(state.city.details.dock ?? 0).toBe(0);
  expect(state.city.details.industry).toBeGreaterThanOrEqual(16);
  expect(state.city.trees).toBeGreaterThan(3000);
  expect(state.city.trees).toBeLessThan(6500);
  expect(state.city.reserveTiles).toBeGreaterThan(3500);
  expect(state.city.validationErrors).toBe(0);
  expect(state.city.invalidBuildings).toBe(0);
  expect(state.city.treeBuildingOverlap).toBe(0);
  expect(state.city.roadRailOverlap).toBe(0);
  expect(state.city.railCrossings).toBeGreaterThanOrEqual(1);
  expect(state.city.railStations).toBe(0);
  expect(state.city.backend).toEqual(expect.objectContaining({
    required: true,
    baseUrl: backendBaseUrl,
    status: expect.objectContaining({
      service: 'abutown-sim',
      world_id: 'abutown-main',
      ok: true,
      protocol_version: 1,
    }),
  }));
  expect(state.city.mobility).toEqual(expect.objectContaining({
    source: 'backend',
    status: 'connected',
    tick: expect.any(Number),
    agents: expect.any(Number),
    vehicles: expect.any(Number),
    stops: expect.any(Number),
    invalidMessages: 0,
    lastError: null,
  }));
  expect(state.city.mobilityAgents.count).toBe(state.city.pedestrians);
  expect(state.city.mobilityAgents.selectedId).toBeNull();
  expect(state.city.mobilityAgents.agents.length).toBe(state.city.pedestrians);
  if (state.city.mobilityAgents.agents.length > 0) {
    expect(state.city.mobilityAgents.agents[0]).toEqual(expect.objectContaining({
      id: expect.stringMatching(/^agent:(walk|walker|driver|seed|lod):/),
      kind: 'pedestrian',
      state: 'walking',
      coord: expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
      screen: expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
    }));
  }
  expect(state.city.mobilityVehicles.count).toBe(state.city.cars);
  expect(state.city.mobilityVehicles.selectedId).toBeNull();
  expect(state.city.mobilityVehicles.vehicles.length).toBe(state.city.cars);
  if (state.city.mobilityVehicles.vehicles.length > 0) {
    const carVehicle = state.city.mobilityVehicles.vehicles.find(
      (v: { id: string }) => v.id.startsWith('vehicle:car:'),
    );
    expect(carVehicle).toBeDefined();
    expect(state.city.mobilityVehicles.vehicles[0]).toEqual(expect.objectContaining({
      id: expect.stringMatching(/^vehicle:car:/),
      kind: 'car',
      state: 'driving',
      coord: expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
      screen: expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
    }));
  }
  expect(state.city.train.position.y).toBeGreaterThan(state.city.height - 1);
  expect(state.city.train.alpha).toBeGreaterThanOrEqual(0);
  expect(state.city.train.alpha).toBeLessThan(1);
  await page.evaluate(() => window.advanceTime?.(2500));
  const movedState = JSON.parse(await page.evaluate(() => window.render_game_to_text?.() ?? ''));
  expect(movedState.city.train.position.y).toBeLessThan(state.city.train.position.y);
  expect(movedState.city.train.alpha).toBeGreaterThan(0);
  expect(movedState.city.train.alpha).toBeGreaterThan(state.city.train.alpha);
  const firstSample = JSON.parse(await page.evaluate(() => window.render_game_to_text?.() ?? ''));
  const sampleAgent = firstSample.city.mobilityAgents.agents.find(
    (agent: { id: string }) => agent.id.startsWith('agent:walk:'),
  );
  let selectedState = movedState;
  if (sampleAgent) {
    // Frame interpolation (Phase 2): two reads ~80 ms apart should show an agent that moved.
    await page.waitForTimeout(80);
    const secondSample = JSON.parse(await page.evaluate(() => window.render_game_to_text?.() ?? ''));
    const sameAgentLater = secondSample.city.mobilityAgents.agents.find(
      (entry: { id: string }) => entry.id === sampleAgent.id,
    );
    expect(sameAgentLater).toBeDefined();
    if (secondSample.city.mobility.tick > firstSample.city.mobility.tick) {
      const movedX = Math.abs(sameAgentLater.coord.x - sampleAgent.coord.x);
      const movedY = Math.abs(sameAgentLater.coord.y - sampleAgent.coord.y);
      expect(movedX + movedY).toBeGreaterThan(0);
    }
    const clickableAgent = movedState.city.mobilityAgents.agents.find(
      (agent: { screen: { x: number; y: number } }) =>
        agent.screen.x > 16 &&
        agent.screen.x < 393 &&
        agent.screen.y > 16 &&
        agent.screen.y < 503,
    );
    if (clickableAgent) {
      await page.mouse.click(clickableAgent.screen.x, clickableAgent.screen.y);
      selectedState = JSON.parse(await page.evaluate(() => window.render_game_to_text?.() ?? ''));
      expect(selectedState.city.mobilityAgents.selectedId).toBe(clickableAgent.id);
      expect(selectedState.city.mobilityAgents.selected).toEqual(expect.objectContaining({
        id: clickableAgent.id,
        state: 'walking',
      }));
      expect(selectedState.city.agentInspector).toEqual(expect.objectContaining({
        title: clickableAgent.id,
        rows: expect.arrayContaining([
          expect.objectContaining({ label: 'State', value: 'walking' }),
          expect.objectContaining({ label: 'Tile', value: expect.any(String) }),
          expect.objectContaining({ label: 'Direction', value: expect.any(String) }),
        ]),
      }));
    }
  }
  const clickableVehicle = isolatedVisibleEntity(selectedState.city.mobilityVehicles.vehicles, { width: 409, height: 519 });
  if (clickableVehicle) {
    await page.mouse.click(clickableVehicle.screen.x, clickableVehicle.screen.y);
    const vehicleSelectedState = JSON.parse(await page.evaluate(() => window.render_game_to_text?.() ?? ''));
    expect(vehicleSelectedState.city.mobilityVehicles.selectedId).toBe(clickableVehicle.id);
    expect(vehicleSelectedState.city.mobilityAgents.selectedId).toBeNull();
    expect(vehicleSelectedState.city.vehicleInspector).toEqual(expect.objectContaining({
      title: clickableVehicle.id,
      rows: expect.arrayContaining([
        expect.objectContaining({ label: 'State', value: 'driving' }),
        expect.objectContaining({ label: 'Tile', value: expect.any(String) }),
        expect.objectContaining({ label: 'Direction', value: expect.any(String) }),
      ]),
    }));
  }
  expect(state.city.railStationsOnRoad).toBe(0);
  expect(state.city.railStationsOnBuildings).toBe(0);
  expect(state.city.railStationsOnRails).toBe(0);
  expect(state.city.railStationsOnTrees).toBe(0);
  expect(state.city.diagnostics).toEqual(expect.any(Object));
  expect(state.city.diagnostics.railStationsOnRoad).toBe(0);
  expect(state.city.diagnostics.railStationsOnBuildings).toBe(0);
  expect(state.city.diagnostics.railStationsOnRails).toBe(0);
  expect(state.city.diagnostics.railStationsOnTrees).toBe(0);
  expect(state.city.camera.mode).toBe('bounded-fixed-map');
  expect(state.city.camera.target).toEqual(expect.objectContaining({
    x: expect.any(Number),
    y: expect.any(Number),
    scale: expect.any(Number),
  }));
  const nearBlackRatio = await page.evaluate(() => {
    const canvas = document.querySelector<HTMLCanvasElement>('#game');
    const context = canvas?.getContext('2d');
    if (!canvas || !context) return 1;
    const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
    let nearBlack = 0;
    let sampled = 0;
    for (let i = 0; i < data.length; i += 4 * 16) {
      const r = data[i];
      const g = data[i + 1];
      const b = data[i + 2];
      const a = data[i + 3];
      sampled += 1;
      if (a === 255 && r < 8 && g < 8 && b < 8) nearBlack += 1;
    }
    return nearBlack / sampled;
  });
  expect(nearBlackRatio).toBeLessThan(0.05);
  const coloredPixelRatio = await page.evaluate(() => {
    const canvas = document.querySelector<HTMLCanvasElement>('#game');
    const context = canvas?.getContext('2d');
    if (!canvas || !context) return 0;
    const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
    let colored = 0;
    let sampled = 0;
    for (let i = 0; i < data.length; i += 4 * 16) {
      const r = data[i];
      const g = data[i + 1];
      const b = data[i + 2];
      const a = data[i + 3];
      sampled += 1;
      if (a > 0 && Math.max(r, g, b) - Math.min(r, g, b) > 18) colored += 1;
    }
    return colored / sampled;
  });
  expect(coloredPixelRatio).toBeGreaterThan(0.02);
  expect(consoleErrors).toEqual([]);
});

function isolatedVisibleEntity<T extends ScreenEntity>(
  entities: T[],
  viewport: { width: number; height: number },
): T | undefined {
  return entities
    .filter((entity) => (
      entity.screen.x > 16 &&
      entity.screen.x < viewport.width - 16 &&
      entity.screen.y > 16 &&
      entity.screen.y < viewport.height - 16
    ))
    .map((entity) => ({ entity, nearestNeighbor: nearestNeighborDistance(entity, entities) }))
    .sort((a, b) => b.nearestNeighbor - a.nearestNeighbor)
    .find(({ nearestNeighbor }) => nearestNeighbor > 32)?.entity;
}

function nearestNeighborDistance(entity: ScreenEntity, entities: ScreenEntity[]): number {
  let nearest = Number.POSITIVE_INFINITY;
  for (const other of entities) {
    if (other.id === entity.id) continue;
    nearest = Math.min(nearest, Math.hypot(entity.screen.x - other.screen.x, entity.screen.y - other.screen.y));
  }
  return nearest;
}
