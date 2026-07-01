// Design tokens — single source of truth for the diorama look.
// Rule: no color/material/radius values anywhere outside this file.

export const palette = {
  // 60% — warm base
  creamBase: 0xf7ecdc,
  creamLight: 0xfbf4e8,
  floorWarm: 0xeed9b2,
  lawn: 0xbcd8a5,
  // 30% — muted secondaries
  sage: 0xb8cfb9,
  mint: 0xa9dcc3,
  woodSoft: 0xdfc09a,
  white: 0xfaf7f0,
  // 10% — the one accent
  coral: 0xec7f60,
  coralSoft: 0xf0967e,
  // people & details
  skin: 0xf2c9a8,
  honey: 0xf2d98c,
  eye: 0x3a342e,
  plantGreen: 0x97c78f,
  plantPot: 0xc9906b,
  metalMatt: 0x9aa4ad,
  metalDark: 0x7d8891,
  glass: 0xcfe4ec,
  sunShaft: 0xffdca8,
} as const;

// Chunky clay form language — radii scale (meters). No sharp edges, no thin sticks.
export const radii = { xs: 0.03, s: 0.06, m: 0.12, l: 0.22 } as const;

// Clay material recipe (applied uniformly; renderer may only build materials from this).
export const clay = {
  roughness: 0.88,
  metalness: 0.0,
  // SSS approximation: sheen lifts the terminator like light entering clay/wax.
  sheen: 0.55,
  sheenRoughness: 0.75,
} as const;

export type LightPreset = {
  sunColor: number;
  sunIntensity: number;
  sunPosition: [number, number, number];
  hemiSky: number;
  hemiGround: number;
  hemiIntensity: number;
  background: number;
  fogColor: number;
  fogNear: number;
  fogFar: number;
  exposure: number;
};

export const lightPresets: Record<'morning' | 'night', LightPreset> = {
  morning: {
    sunColor: 0xffc27e,
    sunIntensity: 7.2,
    sunPosition: [8, 4.5, 1.2],
    hemiSky: 0xcfe0ec,
    hemiGround: 0xe4d3ba,
    hemiIntensity: 0.6,
    background: 0xf6ead6,
    fogColor: 0xf6ead6,
    fogNear: 20,
    fogFar: 48,
    exposure: 1.14,
  },
  night: {
    sunColor: 0xa8c4e8,
    sunIntensity: 1.0,
    sunPosition: [-6, 7, 6],
    hemiSky: 0x4a5f7d,
    hemiGround: 0x3d4652,
    hemiIntensity: 0.4,
    background: 0x2e3b52,
    fogColor: 0x2e3b52,
    fogNear: 20,
    fogFar: 48,
    exposure: 1.05,
  },
};

// Warm interior glow for the night preset.
export const nightGlow = {
  bulb: 0xffc98a,
  lampIntensity: 26,
  bedsideIntensity: 5,
} as const;

// Post-processing recipe — the miniature magic. All post values live here.
export const post = {
  dof: { focusDistance: 16.5, focalLength: 1.4, bokehScale: 2.2 },
  bloom: { strength: 0.12, radius: 0.3, threshold: 0.9 },
  filmGrain: 0.08,
  godraysMix: 0.35,
  godraysDensity: 0.35,
  godraysMaxDensity: 0.32,
} as const;

// One-bounce GI: the scene is captured from its own center and fed back as
// image-based lighting, so walls/lawn tint the shadows.
export const gi = { environmentIntensity: 0.28, hemiCut: 0.5 } as const;

// Camera contract — the diorama has ONE gaze, like a built miniature.
// From the south-west, looking into the corner formed by the north + east walls.
export const cameraContract = {
  fov: 24,
  position: [-9.2, 6.8, 10.8] as [number, number, number],
  target: [0.4, 0.9, -0.5] as [number, number, number],
} as const;
