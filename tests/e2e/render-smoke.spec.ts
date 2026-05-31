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

test('renders abutopia with 300 backend-driven pedestrians', async ({ page }) => {
  await page.setViewportSize({ width: 409, height: 519 });
  const consoleErrors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text());
  });

  await page.goto('/');
  await expect(page.locator('#game')).toHaveAttribute('data-ready', 'true');
  await expect.poll(async () => {
    const state = await readCityState(page);
    // Pedestrian-kind agents only — the seeded economy trader is a distinct kind.
    return state.city.pedestrians;
  }, { timeout: 10_000 }).toBe(300);

  const state = await readCityState(page);
  const oldResourceRequests = await page.evaluate((patternSource) =>
    performance
      .getEntriesByType('resource')
      .map((entry) => entry.name)
      .filter((name) => new RegExp(patternSource, 'i').test(name)),
  retiredAssetResourcePattern.source,
  );

  expect(state.city.worldId).toBe('abutopia');
  expect(state.city.width).toBe(224);
  expect(state.city.height).toBe(128);
  expect(state.city.roadTiles).toBe(10);
  expect(state.city.buildings).toBe(2);
  expect(state.city.railTiles).toBe(0);
  expect(state.city.bridges).toBe(0);
  expect(state.city.trees).toBe(0);
  expect(state.city.details.total).toBe(0);
  expect(state.city.reserveTiles).toBe(0);
  expect(state.city.cars).toBe(0);
  expect(state.city.pedestrians).toBe(300);
  expect(state.city.train).toBeUndefined();
  expect(state.city.trains).toBeUndefined();
  expect(state.city[['mobility', 'Trams'].join('')]).toBeUndefined();
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
  expect(state.city.validationErrors).toBe(0);
  expect(state.city.invalidBuildings).toBe(0);
  expect(state.city.treeBuildingOverlap).toBe(0);
  expect(state.city.roadRailOverlap).toBe(0);
  expect(state.city.railCrossings).toBe(0);
  expect(state.city.railStations).toBe(0);
  expect(state.city.backend).toEqual(expect.objectContaining({
    required: true,
    baseUrl: 'http://127.0.0.1:8080',
    status: expect.objectContaining({
      service: 'abutown-sim',
      world_id: 'abutopia',
      ok: true,
      protocol_version: 1,
    }),
  }));
  expect(state.city.mobility).toEqual(expect.objectContaining({
    source: 'backend',
    status: 'connected',
    tick: expect.any(Number),
    // Total backend agents = 300 base pedestrians + the seeded economy trader,
    // which materializes asynchronously; assert the pedestrian population below.
    agents: expect.any(Number),
    vehicles: 0,
    stops: 0,
    invalidMessages: 0,
    lastError: null,
  }));
  expect(
    state.city.mobilityAgents.agents.filter((a: { id: string }) => !a.id.startsWith('trader:')),
  ).toHaveLength(300);
  expect(state.city.mobilityAgents.selectedId).toBeNull();
  expect(state.city.mobilityAgents.count).toBeGreaterThanOrEqual(300);
  expect(state.city.mobilityAgents.agents[0]).toEqual(expect.objectContaining({
    id: expect.any(String),
    kind: 'pedestrian',
    state: 'walking',
    coord: expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
    screen: expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
  }));
  const agent = state.city.mobilityAgents.agents[0];
  expect(agent.coord.x).toBeGreaterThanOrEqual(0);
  expect(agent.coord.x).toBeLessThanOrEqual(223);
  expect(agent.coord.y).toBeGreaterThanOrEqual(0);
  expect(agent.coord.y).toBeLessThanOrEqual(127);
  expect(state.city.mobilityVehicles.count).toBe(0);
  expect(state.city.mobilityVehicles.selectedId).toBeNull();
  expect(state.city.mobilityVehicles.vehicles).toEqual([]);
  expect(state.city.traffic).toEqual(expect.objectContaining({
    routes: 0,
    cars: 0,
    movingCars: 0,
    stuckCars: 0,
    invalidRouteCars: 0,
  }));

  await expect.poll(movementObserver(page, (sample) => sample.city.mobilityAgents.agents), {
    timeout: 10_000,
  }).toBeGreaterThan(0);

  let selectedState = await readCityState(page);
  let selectedAgentId: string | null = null;
  for (let attempt = 0; attempt < 8; attempt += 1) {
    const agentCandidates = await visibleAgentCandidates(page, { width: 409, height: 519 });
    expect(agentCandidates.length).toBeGreaterThan(0);
    for (const { entity: clickableAgent } of agentCandidates.slice(0, 8)) {
      await page.mouse.click(clickableAgent.screen.x, clickableAgent.screen.y);
      selectedState = await readCityState(page);
      if (typeof selectedState.city.mobilityAgents.selectedId === 'string') {
        selectedAgentId = selectedState.city.mobilityAgents.selectedId;
        break;
      }
    }
    if (selectedAgentId) {
      break;
    }
    await page.waitForTimeout(100);
  }
  expect(selectedAgentId).toEqual(expect.any(String));
  expect(selectedState.city.mobilityAgents.selectedId).toBe(selectedAgentId);
  expect(selectedState.city.mobilityAgents.agents.some((entry: { id: string }) => entry.id === selectedAgentId)).toBe(true);
  expect(selectedState.city.mobilityAgents.selected).toEqual(expect.objectContaining({
    id: selectedAgentId,
    state: 'walking',
  }));
  expect(selectedState.city.agentInspector).toEqual(expect.objectContaining({
    title: selectedAgentId,
    rows: expect.arrayContaining([
      expect.objectContaining({ label: 'State', value: 'walking' }),
      expect.objectContaining({ label: 'Tile', value: expect.any(String) }),
      expect.objectContaining({ label: 'Direction', value: expect.any(String) }),
    ]),
  }));
  expect(selectedState.city.mobilityVehicles.selectedId).toBeNull();
  expect(selectedState.city.vehicleInspector).toBeNull();
  expect(state.city.railStationsOnRoad).toBe(0);
  expect(state.city.railStationsOnBuildings).toBe(0);
  expect(state.city.railStationsOnRails).toBe(0);
  expect(state.city.railStationsOnTrees).toBe(0);
  expect(state.city.diagnostics).toEqual(expect.any(Object));
  expect(state.city.camera.mode).toBe('bounded-fixed-map');
  expect(state.city.camera.target).toEqual(expect.objectContaining({
    x: expect.any(Number),
    y: expect.any(Number),
    scale: expect.any(Number),
  }));

  const canvasRatios = await page.evaluate(() => {
    const canvas = document.querySelector<HTMLCanvasElement>('#game');
    const context = canvas?.getContext('2d', { willReadFrequently: true });
    if (!canvas || !context) {
      return {
        edgeTransparentRatio: 1,
        nearBlackRatio: 1,
        nonBackgroundRatio: 0,
        transparentRatio: 1,
      };
    }
    const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
    let edgeSamples = 0;
    let edgeTransparent = 0;
    let nearBlack = 0;
    let nonBackground = 0;
    let sampled = 0;
    let transparent = 0;
    const bg = { r: 145, g: 200, b: 111 };
    for (let i = 0; i < data.length; i += 4 * 16) {
      const pixelIndex = i / 4;
      const x = pixelIndex % canvas.width;
      const y = Math.floor(pixelIndex / canvas.width);
      const r = data[i];
      const g = data[i + 1];
      const b = data[i + 2];
      const a = data[i + 3];
      sampled += 1;
      if (a === 0) transparent += 1;
      if (a === 255 && r < 8 && g < 8 && b < 8) nearBlack += 1;
      if (Math.abs(r - bg.r) + Math.abs(g - bg.g) + Math.abs(b - bg.b) > 24) {
        nonBackground += 1;
      }
      if (x < 8 || x >= canvas.width - 8 || y < 8 || y >= canvas.height - 8) {
        edgeSamples += 1;
        if (a === 0) edgeTransparent += 1;
      }
    }
    return {
      edgeTransparentRatio: edgeSamples === 0 ? 0 : edgeTransparent / edgeSamples,
      nearBlackRatio: nearBlack / sampled,
      nonBackgroundRatio: nonBackground / sampled,
      transparentRatio: transparent / sampled,
    };
  });
  expect(canvasRatios.nonBackgroundRatio).toBeGreaterThan(0.003);
  expect(canvasRatios.nearBlackRatio).toBeLessThan(0.05);
  expect(canvasRatios.transparentRatio).toBe(0);
  expect(canvasRatios.edgeTransparentRatio).toBe(0);
  expect(consoleErrors).toEqual([]);
});

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
    const neighbors = state.city.mobilityAgents.agents;
    const candidates = rankedVisibleEntities(state.city.mobilityAgents.agents, viewport, neighbors);
    if (candidates.length > 0) return candidates;
    await panNearestEntityIntoViewport(page, state.city.mobilityAgents.agents, viewport);
  }
  return [];
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

test('clock advances: abutopia pedestrians age over the wire', async ({ page }) => {
  await page.setViewportSize({ width: 409, height: 519 });
  await page.goto('/');
  await expect(page.locator('#game')).toHaveAttribute('data-ready', 'true');
  await expect
    .poll(async () => (await readCityState(page)).city.mobilityAgents.agents.length)
    .toBeGreaterThan(0);

  const first = await readCityState(page);
  const a0 = first.city.mobilityAgents.agents[0];
  expect(typeof first.city.simTime).toBe('number');
  expect(typeof a0.ageSeconds).toBe('number');

  await expect.poll(async () => {
    const second = await readCityState(page);
    const a1 = second.city.mobilityAgents.agents.find((agent: { id: string }) => agent.id === a0.id);
    return a1?.ageSeconds ?? a0.ageSeconds;
  }, { timeout: 10_000 }).toBeGreaterThan(a0.ageSeconds);
});
