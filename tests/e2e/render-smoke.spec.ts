import { expect, test, type Page } from '@playwright/test';

type ScreenEntity = { id: string; screen: { x: number; y: number } };

const retiredAssetResourcePattern = new RegExp([
  ['pak', '128'].join(''),
  ['simu', 'trans'].join(''),
  ['open', 'gfx'].join(''),
  ['open', 'ttd'].join(''),
].join('|'), 'i');

async function readCityState(page: Page): Promise<any> {
  const raw = await page.evaluate(() => window.render_game_to_text?.() ?? '');
  return JSON.parse(raw);
}

test('renders the city with a bounded fixed-map camera', async ({ page }) => {
  await page.setViewportSize({ width: 409, height: 519 });
  const consoleErrors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text());
  });

  await page.goto('/');
  await expect(page.locator('#game')).toHaveAttribute('data-ready', 'true');
  await expect.poll(async () => {
    const state = await readCityState(page);
    return state.city.mobilityAgents.agents.length;
  }, { timeout: 10_000 }).toBeGreaterThanOrEqual(50);
  await expect.poll(async () => {
    const state = await readCityState(page);
    return state.city.mobilityVehicles.vehicles.length;
  }, { timeout: 10_000 }).toBeGreaterThanOrEqual(1);
  await expect.poll(async () => {
    const state = await readCityState(page);
    return visibleEntities(state.city.mobilityVehicles.vehicles, { width: 409, height: 519 }).length;
  }, { timeout: 10_000 }).toBeGreaterThanOrEqual(1);
  await expect.poll(async () => {
    const state = await readCityState(page);
    return state.city.mobilityTrams.trams.length;
  }, { timeout: 10_000 }).toBe(4);
  const state = await readCityState(page);
  const oldResourceRequests = await page.evaluate((patternSource) =>
    performance
      .getEntriesByType('resource')
      .map((entry) => entry.name)
      .filter((name) => new RegExp(patternSource, 'i').test(name)),
  retiredAssetResourcePattern.source,
  );

  expect(state.city.roadTiles).toBeGreaterThan(0);
  expect(state.city.buildings).toBeGreaterThan(0);
  expect(state.city.cars).toBeGreaterThanOrEqual(1);
  expect(state.city.trains).toBe(4);
  expect(state.city.worldId).toBe('zurich-river-city-v1');
  expect(state.city.visualStyle).toEqual({
    id: 'minimal-motorways',
    renderer: 'canvas-vector',
    spriteDrawing: 'disabled',
  });
  expect(state.city.visualAssets).toEqual({
    id: 'minimal-vector',
    tile: { width: 18, height: 18 },
  });
  expect(state.city.loadedRasterAssetPaths).toEqual([]);
  expect(oldResourceRequests).toEqual([]);
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
  expect(state.city.pedestrians).toBeGreaterThanOrEqual(50);
  expect(state.city.backend).toEqual(expect.objectContaining({
    required: true,
    baseUrl: 'http://127.0.0.1:8080',
    status: expect.objectContaining({
      service: 'abutown-sim',
      world_id: 'zurich-river-city-v1',
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
  expect(state.city.mobilityAgents.agents.length).toBeGreaterThanOrEqual(50);
  expect(state.city.mobilityAgents.agents[0]).toEqual(expect.objectContaining({
    id: expect.stringMatching(/^agent:(walk|walker|driver|seed|lod):/),
    kind: 'pedestrian',
    state: 'walking',
    coord: expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
    screen: expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
  }));
  expect(state.city.mobilityVehicles.count).toBe(state.city.cars);
  expect(state.city.mobilityVehicles.selectedId).toBeNull();
  expect(state.city.mobilityVehicles.vehicles.length).toBe(state.city.cars);
  const visibleVehicles = visibleEntities(state.city.mobilityVehicles.vehicles, { width: 409, height: 519 });
  expect(visibleVehicles.length).toBeGreaterThanOrEqual(1);
  const uniqueVehicleScreens = new Set(
    state.city.mobilityVehicles.vehicles.map(
      (vehicle: ScreenEntity) => `${Math.round(vehicle.screen.x)}:${Math.round(vehicle.screen.y)}`,
    ),
  );
  expect(uniqueVehicleScreens.size).toBeGreaterThan(1);
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
  expect(state.city.mobilityTrams.count).toBe(4);
  expect(state.city.mobilityTrams.trams[0]).toEqual(expect.objectContaining({
    id: expect.stringMatching(/^vehicle:tram:/),
    kind: 'tram',
    state: 'driving',
    coord: expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
    screen: expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
  }));
  expect(state.city.train).toEqual(expect.objectContaining({
    id: expect.stringMatching(/^vehicle:tram:/),
    position: expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
  }));
  // Backend-driven movement: compare consecutive subscribed samples. The HTTP
  // full snapshot can be replaced by chunk snapshots once the WebSocket
  // subscription opens, so anchoring to a pre-subscribe set is flaky.
  await expect.poll(movementObserver(page, (sample) => sample.city.mobilityAgents.agents), {
    timeout: 10_000,
  }).toBeGreaterThan(0);
  await expect.poll(movementObserver(page, (sample) => sample.city.mobilityVehicles.vehicles), {
    timeout: 10_000,
  }).toBeGreaterThan(0);
  await expect.poll(movementObserver(page, (sample) => sample.city.mobilityTrams.trams), {
    timeout: 10_000,
  }).toBeGreaterThan(0);
  const agentCandidates = await visibleAgentCandidates(page, { width: 409, height: 519 });
  expect(agentCandidates.length).toBeGreaterThan(0);
  let selectedState = await readCityState(page);
  for (const { entity: clickableAgent } of agentCandidates.slice(0, 8)) {
    await page.mouse.click(clickableAgent.screen.x, clickableAgent.screen.y);
    selectedState = await readCityState(page);
    if (selectedState.city.mobilityAgents.selectedId === clickableAgent.id) break;
  }
  expect(selectedState.city.mobilityAgents.selectedId).toEqual(expect.any(String));
  expect(selectedState.city.mobilityAgents.selected).toEqual(expect.objectContaining({
    id: selectedState.city.mobilityAgents.selectedId,
    state: 'walking',
  }));
  expect(selectedState.city.agentInspector).toEqual(expect.objectContaining({
    title: selectedState.city.mobilityAgents.selectedId,
    rows: expect.arrayContaining([
      expect.objectContaining({ label: 'State', value: 'walking' }),
      expect.objectContaining({ label: 'Tile', value: expect.any(String) }),
      expect.objectContaining({ label: 'Direction', value: expect.any(String) }),
    ]),
  }));
  const clickableVehicle = await visibleVehicle(page, { width: 409, height: 519 });
  expect(clickableVehicle).toBeTruthy();
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
  expect(consoleErrors).toEqual([]);
});

function isolatedVisibleEntity<T extends ScreenEntity>(
  entities: T[],
  viewport: { width: number; height: number },
): T | undefined {
  return rankedVisibleEntities(entities, viewport, entities)
    .find(({ nearestNeighbor }) => nearestNeighbor > 32)?.entity;
}

function rankedVisibleEntities<T extends ScreenEntity>(
  entities: T[],
  viewport: { width: number; height: number },
  neighbors: ScreenEntity[],
): { entity: T; nearestNeighbor: number }[] {
  return visibleEntities(entities, viewport)
    .map((entity) => ({ entity, nearestNeighbor: nearestNeighborDistance(entity, neighbors) }))
    .sort((a, b) => b.nearestNeighbor - a.nearestNeighbor);
}

function visibleEntities<T extends ScreenEntity>(
  entities: T[],
  viewport: { width: number; height: number },
): T[] {
  return entities.filter((entity) => (
    entity.screen.x > 16 &&
    entity.screen.x < viewport.width - 16 &&
    entity.screen.y > 16 &&
    entity.screen.y < viewport.height - 16
  ));
}

async function visibleAgentCandidates(
  page: Page,
  viewport: { width: number; height: number },
): Promise<{ entity: ScreenEntity; nearestNeighbor: number }[]> {
  for (let attempt = 0; attempt < 4; attempt += 1) {
    const state = await readCityState(page);
    const neighbors = [
      ...state.city.mobilityAgents.agents,
      ...state.city.mobilityVehicles.vehicles,
      ...state.city.mobilityTrams.trams,
    ];
    const candidates = rankedVisibleEntities(state.city.mobilityAgents.agents, viewport, neighbors);
    if (candidates.length > 0) return candidates;
    await panNearestEntityIntoViewport(page, state.city.mobilityAgents.agents, viewport);
  }
  return [];
}

async function visibleVehicle(
  page: Page,
  viewport: { width: number; height: number },
): Promise<ScreenEntity | undefined> {
  for (let attempt = 0; attempt < 4; attempt += 1) {
    const state = await readCityState(page);
    const vehicle = visibleEntities(state.city.mobilityVehicles.vehicles, viewport)[0];
    if (vehicle) return vehicle;
    await panNearestEntityIntoViewport(page, state.city.mobilityVehicles.vehicles, viewport);
  }
  return undefined;
}

async function panNearestEntityIntoViewport(
  page: Page,
  entities: ScreenEntity[],
  viewport: { width: number; height: number },
): Promise<void> {
  const entity = nearestToViewportCenter(entities, viewport);
  if (!entity) return;
  const center = { x: viewport.width / 2, y: viewport.height / 2 };
  await page.mouse.move(center.x, center.y);
  await page.mouse.down();
  await page.mouse.move(center.x + center.x - entity.screen.x, center.y + center.y - entity.screen.y, { steps: 8 });
  await page.mouse.up();
  await page.waitForTimeout(250);
}

function nearestToViewportCenter<T extends ScreenEntity>(
  entities: T[],
  viewport: { width: number; height: number },
): T | undefined {
  const center = { x: viewport.width / 2, y: viewport.height / 2 };
  return [...entities].sort((a, b) =>
    Math.hypot(a.screen.x - center.x, a.screen.y - center.y) -
    Math.hypot(b.screen.x - center.x, b.screen.y - center.y),
  )[0];
}

function maxCoordMovement(
  before: { id: string; coord: { x: number; y: number } }[],
  after: { id: string; coord: { x: number; y: number } }[],
): number {
  const laterById = new Map(after.map((entity) => [entity.id, entity]));
  return before.reduce((largest, entity) => {
    const later = laterById.get(entity.id);
    if (!later) return largest;
    const delta = Math.abs(later.coord.x - entity.coord.x) + Math.abs(later.coord.y - entity.coord.y);
    return Math.max(largest, delta);
  }, 0);
}

function movementObserver(
  page: Page,
  selectEntities: (sample: any) => { id: string; coord: { x: number; y: number } }[],
): () => Promise<number> {
  let previous: { id: string; coord: { x: number; y: number } }[] | null = null;
  return async () => {
    const sample = await readCityState(page);
    const current = selectEntities(sample);
    const movement = previous ? maxCoordMovement(previous, current) : 0;
    previous = current;
    return movement;
  };
}

function nearestNeighborDistance(entity: ScreenEntity, entities: ScreenEntity[]): number {
  let nearest = Number.POSITIVE_INFINITY;
  for (const other of entities) {
    if (other.id === entity.id) continue;
    nearest = Math.min(nearest, Math.hypot(entity.screen.x - other.screen.x, entity.screen.y - other.screen.y));
  }
  return nearest;
}
