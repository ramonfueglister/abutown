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
  expect(state.city.worldId).toBe('zurich-river-city-v1');
  expect(state.city.width).toBe(256);
  expect(state.city.height).toBe(256);
  expect(state.city.roadTiles).toBeGreaterThan(1800);
  expect(state.city.bridges).toBeGreaterThanOrEqual(6);
  expect(state.city.bridges).toBeLessThanOrEqual(12);
  expect(state.city.railTiles).toBeGreaterThan(250);
  expect(state.city.railTiles).toBeLessThan(400);
  expect(state.city.buildings).toBeGreaterThan(2800);
  expect(state.city.details.total).toBeGreaterThanOrEqual(350);
  expect(state.city.details.dock).toBeGreaterThanOrEqual(6);
  expect(state.city.details.dock).toBeLessThanOrEqual(10);
  expect(state.city.details.industry).toBeGreaterThanOrEqual(70);
  expect(state.city.trees).toBeGreaterThan(3000);
  expect(state.city.trees).toBeLessThan(6500);
  expect(state.city.reserveTiles).toBeGreaterThan(3500);
  expect(state.city.validationErrors).toBe(0);
  expect(state.city.invalidBuildings).toBe(0);
  expect(state.city.treeBuildingOverlap).toBe(0);
  expect(state.city.roadRailOverlap).toBe(0);
  expect(state.city.railCrossings).toBe(1);
  expect(state.city.railStations).toBeGreaterThanOrEqual(10);
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
