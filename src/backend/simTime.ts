export const SIM_SECONDS_PER_DAY = 86_400;
export const SIM_SECONDS_PER_YEAR = 31_536_000;

/**
 * Formats elapsed sim-seconds into a compact human-readable date string.
 * Example: `formatSimDate(31_536_000 + 86_400)` → `"Year 1, Day 1"`
 */
export function formatSimDate(simSeconds: number): string {
  const year = Math.floor(simSeconds / SIM_SECONDS_PER_YEAR);
  const dayOfYear = Math.floor((simSeconds % SIM_SECONDS_PER_YEAR) / SIM_SECONDS_PER_DAY);
  return `Year ${year}, Day ${dayOfYear}`;
}
