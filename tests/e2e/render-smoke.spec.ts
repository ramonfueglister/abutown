import { expect, test } from '@playwright/test';

test('renders the city with a bounded fixed-map camera', async ({ page }) => {
  const consoleErrors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text());
  });

  await page.goto('/');
  await expect(page.locator('#game')).toHaveAttribute('data-ready', 'true');
  const raw = await page.evaluate(() => window.render_game_to_text?.() ?? '');
  const state = JSON.parse(raw);

  expect(state.city.roadTiles).toBeGreaterThan(0);
  expect(state.city.buildings).toBeGreaterThan(0);
  expect(state.city.cars).toBeGreaterThan(0);
  expect(state.city.worldId).toBe('openttd-hamburg-512');
  expect(state.city.width).toBe(512);
  expect(state.city.height).toBe(512);
  expect(state.city.roadTiles).toBeGreaterThan(40000);
  expect(state.city.bridges).toBeGreaterThanOrEqual(100);
  expect(state.city.buildings).toBeGreaterThan(15000);
  expect(state.city.details.total).toBeGreaterThanOrEqual(24000);
  expect(state.city.details.decor).toBeGreaterThanOrEqual(20000);
  expect(state.city.details.industry).toBeGreaterThanOrEqual(2000);
  expect(state.city.details.station).toBeGreaterThanOrEqual(4);
  expect(state.city.trees).toBeGreaterThan(30000);
  expect(state.city.reserveTiles).toBe(0);
  expect(state.city.validationErrors).toBe(0);
  expect(state.city.invalidBuildings).toBe(0);
  expect(state.city.treeBuildingOverlap).toBe(0);
  expect(state.city.roadRailOverlap).toBe(0);
  expect(state.city.railCrossings).toBe(0);
  expect(state.city.railStations).toBe(0);
  expect(state.city.railStationsOnRoad).toBe(0);
  expect(state.city.railStationsOnBuildings).toBe(0);
  expect(state.city.railStationsOnRails).toBe(0);
  expect(state.city.railStationsOnTrees).toBe(0);
  expect(state.city.diagnostics).toBeUndefined();
  expect(state.city.legacyDiagnostics).toEqual(expect.any(Object));
  expect(state.city.legacyDiagnostics.railStationsOnRoad).toBe(0);
  expect(state.city.legacyDiagnostics.railStationsOnBuildings).toBe(0);
  expect(state.city.legacyDiagnostics.railStationsOnRails).toBe(0);
  expect(state.city.legacyDiagnostics.railStationsOnTrees).toBe(0);
  expect(state.city.camera.mode).toBe('bounded-fixed-map');
  expect(state.city.camera.target).toEqual(expect.objectContaining({
    x: expect.any(Number),
    y: expect.any(Number),
    scale: expect.any(Number),
  }));
  expect(consoleErrors).toEqual([]);
});
