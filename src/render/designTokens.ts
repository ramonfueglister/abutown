// Single source of truth for the schematic visual vocabulary.

// --- paper ---
export const GROUND = '#e9ede1';

// --- agents ---
export const AGENT_INK = '#2e3440';
export const TRADER_RED = '#c0392b';
export const SELECTION_HALO_AGENT = '#a87309';
export const SELECTION_HALO_VEHICLE = '#166c83';
export const VEHICLE_COLORS = ['#e85d75', '#3f8fc7', '#49a879', '#e5a944', '#8c73c8', '#ef7f5a', '#28a6b0'] as const;

// --- places ---
export const MARKET_ORANGE = '#d9783a';
export const STATION_FILL = '#fdfcf5';
export const MARKET_GUIDE_CORE = '#6f7f8c';

// --- flows ---
export const FLOW_CASING = '#fdfcf5';

// keyed by backend GoodId (goods.rs: FOOD=1 WOOD=2 IRON=3 TOOLS=4 RAW=5)
export const GOOD_COLORS: Readonly<Partial<Record<number, string>>> = {
  1: '#7a9e4f', // FOOD green
  2: '#8c6f4a', // WOOD brown
  3: '#5f7d8c', // IRON slate
  4: '#8c73c8', // TOOLS violet
  5: '#d98c3a', // RAW orange
};
export const GOOD_COLOR_FALLBACK = '#8a8f94';

// --- semantic zoom bands (camera scale runs 0.18..2.8, see src/main.ts) ---
export const ZOOM_ECONOMY_MAX = 0.6;
export const ZOOM_CITY_MIN = 1.0;
export const ECONOMY_OVERLAY_CITY_OPACITY = 0;
export const AGENT_SHIMMER_OPACITY = 0.55;
