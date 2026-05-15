export type RiverbankSource = { x: number; y: number; width: number; height: number };

export const RIVERBANK_NORTH = 1;
export const RIVERBANK_EAST = 2;
export const RIVERBANK_SOUTH = 4;
export const RIVERBANK_WEST = 8;

const CELL_SIZE = 128;
const RIVER_30_ROW = 25;

export function riverbankSourceFromMask(mask: number): RiverbankSource {
  const normalized = mask & (RIVERBANK_NORTH | RIVERBANK_EAST | RIVERBANK_SOUTH | RIVERBANK_WEST);
  if (normalized === 0) return cell(RIVER_30_ROW, 2);
  if (normalized === RIVERBANK_NORTH) return cell(30, 2);
  if (normalized === RIVERBANK_SOUTH) return cell(30, 0);
  if (normalized === RIVERBANK_EAST) return cell(30, 1);
  if (normalized === RIVERBANK_WEST) return cell(30, 3);
  if (normalized === (RIVERBANK_NORTH | RIVERBANK_SOUTH)) return cell(RIVER_30_ROW, 0);
  if (normalized === (RIVERBANK_EAST | RIVERBANK_WEST)) return cell(RIVER_30_ROW, 1);
  if (normalized === (RIVERBANK_NORTH | RIVERBANK_SOUTH | RIVERBANK_EAST)) return cell(26, 0);
  if (normalized === (RIVERBANK_NORTH | RIVERBANK_SOUTH | RIVERBANK_WEST)) return cell(26, 1);
  if (normalized === (RIVERBANK_NORTH | RIVERBANK_EAST | RIVERBANK_WEST)) return cell(26, 2);
  if (normalized === (RIVERBANK_SOUTH | RIVERBANK_EAST | RIVERBANK_WEST)) return cell(26, 3);
  if (normalized === (RIVERBANK_NORTH | RIVERBANK_SOUTH | RIVERBANK_EAST | RIVERBANK_WEST)) return cell(RIVER_30_ROW, 3);
  if (normalized === (RIVERBANK_NORTH | RIVERBANK_EAST)) return cell(27, 0);
  if (normalized === (RIVERBANK_SOUTH | RIVERBANK_EAST)) return cell(27, 1);
  if (normalized === (RIVERBANK_NORTH | RIVERBANK_WEST)) return cell(27, 2);
  return cell(27, 3);
}

export function riverSurfaceSourceFromMask(mask: number): RiverbankSource {
  return riverbankSourceFromMask(mask);
}

function cell(row: number, col: number): RiverbankSource {
  return { x: col * CELL_SIZE, y: row * CELL_SIZE, width: CELL_SIZE, height: CELL_SIZE };
}
