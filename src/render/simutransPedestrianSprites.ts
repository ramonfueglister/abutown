export type SimutransPedestrianSheetName = 'pedestrians-1' | 'pedestrians-2' | 'pedestrians-3';
export type SimutransPedestrianKind = 'pedestrian' | 'walker';
export type SimutransDirection = 'S' | 'N' | 'E' | 'W' | 'SE' | 'NW' | 'NE' | 'SW';

export type SimutransPedestrianSprite = {
  sheet: SimutransPedestrianSheetName;
  row: number;
  kind: SimutransPedestrianKind;
  scale: number;
};

export type SimutransPedestrianFrameRect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type GridDelta = { x: number; y: number };

const TILE_SIZE = 128;

const SHEET_ROWS: Record<SimutransPedestrianSheetName, number> = {
  'pedestrians-1': 8,
  'pedestrians-2': 4,
  'pedestrians-3': 6,
};

const DIRECTION_COLUMNS: Record<SimutransDirection, number> = {
  S: 0,
  N: 1,
  E: 2,
  W: 3,
  SE: 4,
  NW: 5,
  NE: 6,
  SW: 7,
};

export const SIMUTRANS_PEDESTRIAN_ASSET_PATHS: Record<SimutransPedestrianSheetName, string> = {
  'pedestrians-1': '/simutrans-assets/pak128-pedestrians/raw/privat-pedestrians-128.png',
  'pedestrians-2': '/simutrans-assets/pak128-pedestrians/raw/privat-pedestrians_2-128.png',
  'pedestrians-3': '/simutrans-assets/pak128-pedestrians/raw/privat-pedestrians_3-128.png',
};

export function candidateSimutransPedestrianSprites(): SimutransPedestrianSprite[] {
  return [
    ...[1, 3, 4, 6, 7].map((row) => ({
      sheet: 'pedestrians-1' as const,
      row,
      kind: 'pedestrian' as const,
      scale: 0.96,
    })),
  ];
}

export function simutransPedestrianFrameRect(
  sprite: Pick<SimutransPedestrianSprite, 'row'>,
  direction: SimutransDirection,
): SimutransPedestrianFrameRect {
  return {
    x: DIRECTION_COLUMNS[direction] * TILE_SIZE,
    y: sprite.row * TILE_SIZE,
    width: TILE_SIZE,
    height: TILE_SIZE,
  };
}

export function simutransPedestrianDisplayScale(baseScale: number, cameraScale: number): number {
  return baseScale * Math.min(1.25, Math.max(0.45, 0.68 / Math.max(0.1, cameraScale)));
}

export function simutransPedestrianFrameForGridDelta(delta: GridDelta): SimutransDirection {
  const dx = Math.sign(delta.x);
  const dy = Math.sign(delta.y);
  if (dx > 0 && dy > 0) return 'S';
  if (dx < 0 && dy < 0) return 'N';
  if (dx > 0 && dy < 0) return 'E';
  if (dx < 0 && dy > 0) return 'W';
  if (dx > 0) return 'SE';
  if (dy > 0) return 'SW';
  if (dx < 0) return 'NW';
  if (dy < 0) return 'NE';
  return 'S';
}
