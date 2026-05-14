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
  expect(state.city.vehicleDiagnostics.trafficRuleDecisionCount).toEqual(expect.any(Number));
  expect(state.city.vehicleDiagnostics.reservedIntersections).toEqual(expect.any(Number));
  expect(state.city.vehicleDiagnostics.stoppedForTrafficRules).toEqual(expect.any(Number));
  const manualAdvance = await page.evaluate(() => {
    const before = JSON.parse(window.render_game_to_text?.() ?? '');
    window.advanceTime?.(1000);
    const after = JSON.parse(window.render_game_to_text?.() ?? '');
    return { before, after };
  });
  expect(
    manualAdvance.after.city.vehicleDiagnostics.trafficTick -
      manualAdvance.before.city.vehicleDiagnostics.trafficTick,
  ).toBe(20);
  expect(state.city.camera.mode).toBe('bounded-fixed-map');
  expect(state.city.camera.target).toEqual(expect.objectContaining({
    x: expect.any(Number),
    y: expect.any(Number),
    scale: expect.any(Number),
  }));
  expect(consoleErrors).toEqual([]);
});
