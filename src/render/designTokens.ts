// Single source of truth for the schematic visual vocabulary.
// Spec: docs/superpowers/specs/2026-06-10-schematic-map-renderer-design.md §1

// --- ground & terrain (L0) ---
export const GROUND = '#e9ede1';
export const OUT_OF_WORLD = '#182018';
export const WATER = '#cfe3ea';
export const RIVERBANK = '#dcebe7';
export const PARK = '#dde6cf';
export const PLAZA = '#eee4cd';

// --- network (L0) ---
export const ROAD_INK = '#3a3f47';
export const ROAD_CENTER_DASH = GROUND;
export const RAIL_CASING = 'rgba(122, 131, 135, 0.32)';
export const RAIL_CORE = 'rgba(122, 131, 135, 0.42)';
export const TREE = '#9bb98a';
export const DETAIL = 'rgba(92, 97, 92, 0.30)';
export const BUILDING_RESIDENTIAL = '#e2a14e';
export const BUILDING_COMMERCIAL = '#9fb4bb';
export const BUILDING_CIVIC = '#cbb878';
export const BUILDING_INDUSTRIAL = '#b3a6c9';

// --- agents (L2) ---
export const AGENT_INK = '#2e3440';
export const TRADER_RED = '#c0392b';
export const SELECTION_HALO_AGENT = '#a87309';
export const SELECTION_HALO_VEHICLE = '#166c83';
export const VEHICLE_COLORS = ['#e85d75', '#3f8fc7', '#49a879', '#e5a944', '#8c73c8', '#ef7f5a', '#28a6b0'] as const;

// --- markets (L1) ---
export const MARKET_ORANGE = '#d9783a';

// --- flows (L3) — keyed by backend GoodId (goods.rs: FOOD=1 WOOD=2 IRON=3 TOOLS=4 RAW=5) ---
export const GOOD_COLORS: Readonly<Record<number, string>> = {
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
export const FLOW_MIN_OPACITY = 0.15;
export const AGENT_SHIMMER_OPACITY = 0.35;
