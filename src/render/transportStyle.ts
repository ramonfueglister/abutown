import { screenStableWorldSize } from './minimalGlyphScale';

export const ROAD_CASING = '#c7d1cf';
export const ROAD_CORE = '#fffdf7';
export const ROAD_BRIDGE_CASING = '#8fc9d7';
export const ROAD_BRIDGE_CORE = '#fff9e9';
export const RAIL_CASING = 'rgba(122, 131, 135, 0.32)';
export const RAIL_CORE = 'rgba(122, 131, 135, 0.42)';
export const TRAIN_CORE = '#5f6f75';

export type RoadKindForStyle = 'street' | 'bridge';

export type MaskLineStyle = {
  casing: string;
  core: string;
  casingWidth: number;
  coreWidth: number;
};

export function roadLineStyle(kind: RoadKindForStyle, cameraScale: number): MaskLineStyle {
  const bridge = kind === 'bridge';
  return {
    casing: bridge ? ROAD_BRIDGE_CASING : ROAD_CASING,
    core: bridge ? ROAD_BRIDGE_CORE : ROAD_CORE,
    casingWidth: bridge
      ? screenStableWorldSize(5.5, cameraScale, { minWorld: 10.5, maxWorld: 17 })
      : screenStableWorldSize(4.8, cameraScale, { minWorld: 9.2, maxWorld: 16 }),
    coreWidth: bridge
      ? screenStableWorldSize(3.8, cameraScale, { minWorld: 7, maxWorld: 12 })
      : screenStableWorldSize(3.4, cameraScale, { minWorld: 6.4, maxWorld: 10.5 }),
  };
}

export function railLineStyle(cameraScale: number): MaskLineStyle {
  return {
    casing: RAIL_CASING,
    core: RAIL_CORE,
    casingWidth: screenStableWorldSize(2.8, cameraScale, { minWorld: 4.8, maxWorld: 9 }),
    coreWidth: screenStableWorldSize(1.2, cameraScale, { minWorld: 1.8, maxWorld: 4 }),
  };
}
