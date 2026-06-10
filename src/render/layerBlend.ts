import {
  AGENT_SHIMMER_OPACITY,
  FLOW_MIN_OPACITY,
  ZOOM_CITY_MIN,
  ZOOM_ECONOMY_MAX,
} from './designTokens';

export type LayerKey = 'network' | 'markets' | 'agents' | 'flows';
export type LayerBlend = { opacity: number; detail: 'aggregate' | 'individual' };

/** 0 at/below the economy band, 1 at/above the city band, linear between. */
function cityness(scale: number): number {
  if (scale <= ZOOM_ECONOMY_MAX) return 0;
  if (scale >= ZOOM_CITY_MIN) return 1;
  return (scale - ZOOM_ECONOMY_MAX) / (ZOOM_CITY_MIN - ZOOM_ECONOMY_MAX);
}

export function layerBlend(layer: LayerKey, scale: number): LayerBlend {
  const t = cityness(scale);
  switch (layer) {
    case 'network':
    case 'markets':
      return { opacity: 1, detail: 'individual' };
    case 'agents':
      return {
        opacity: AGENT_SHIMMER_OPACITY + (1 - AGENT_SHIMMER_OPACITY) * t,
        detail: t > 0 ? 'individual' : 'aggregate',
      };
    case 'flows':
      return {
        opacity: t >= 1 ? FLOW_MIN_OPACITY : 1 - (1 - FLOW_MIN_OPACITY) * t,
        detail: t < 1 ? 'individual' : 'aggregate',
      };
  }
}
