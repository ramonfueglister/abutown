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
  expect(state.city.bridges).toBeGreaterThanOrEqual(3);
  expect(state.city.railCrossings).toBeGreaterThanOrEqual(1);
  expect(state.city.trees).toBeGreaterThan(3000);
  expect(state.city.reserveTiles).toBeGreaterThan(2500);
  expect(state.city.invalidBuildings).toBe(0);
  expect(state.city.roadRailOverlap).toBe(0);
  expect(state.city.camera.mode).toBe('bounded-fixed-map');
  expect(state.city.camera.target).toEqual(expect.objectContaining({
    x: expect.any(Number),
    y: expect.any(Number),
    scale: expect.any(Number),
  }));
  expect(consoleErrors).toEqual([]);
});
