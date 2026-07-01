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
  cloud: 0xfdf6ea,
  star: 0xdfe8f2,
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
  fogColor: number;
  fogNear: number;
  fogFar: number;
  exposure: number;
  // Painted sky gradient (DREDGE-style banding), bottom to top
  skyBelow: number;
  skyHorizon: number;
  skyMid: number;
  skyZenith: number;
  // Atmosphere dressing
  mistColor: number;
  mistOpacity: number;
  cloudTint: number;
  sunDiscColor: number;
  giScale: number;
  saturation: number;
  contrast: number;
  lampBoost: number;
  showStars: boolean;
  lampOn: boolean;
};

export const lightPresets: Record<'morning' | 'dusk' | 'night', LightPreset> = {
  morning: {
    sunColor: 0xffc27e,
    sunIntensity: 7.2,
    sunPosition: [8, 4.5, 1.2],
    hemiSky: 0xc4dcda,
    hemiGround: 0xe4d3ba,
    hemiIntensity: 0.6,
    fogColor: 0xeee2cf,
    fogNear: 20,
    fogFar: 48,
    exposure: 1.14,
    skyBelow: 0xc9d8d2,
    skyHorizon: 0xffd9a3,
    skyMid: 0xcfe3df,
    skyZenith: 0x9dc2cc,
    mistColor: 0xf6e9d2,
    mistOpacity: 0.16,
    cloudTint: 0xfdf6ea,
    sunDiscColor: 0xffe3b0,
    giScale: 1.0,
    saturation: 1.05,
    contrast: 1.0,
    lampBoost: 1.0,
    showStars: false,
    lampOn: false,
  },
  // The DREDGE moment: amber horizon burning under a deep teal sky.
  dusk: {
    sunColor: 0xff7d33,
    sunIntensity: 5.2,
    sunPosition: [8.5, 2.0, -4],
    hemiSky: 0x4f7d84,
    hemiGround: 0x5c5348,
    hemiIntensity: 0.42,
    fogColor: 0x486e74,
    fogNear: 18,
    fogFar: 46,
    exposure: 0.96,
    skyBelow: 0x22434c,
    skyHorizon: 0xff8a3d,
    skyMid: 0x5d8d8c,
    skyZenith: 0x143540,
    mistColor: 0x6f949a,
    mistOpacity: 0.22,
    cloudTint: 0xf2c9a0,
    sunDiscColor: 0xffb877,
    giScale: 0.55,
    saturation: 1.12,
    contrast: 1.06,
    lampBoost: 1.35,
    showStars: false,
    lampOn: true,
  },
  night: {
    sunColor: 0xa8c4e8,
    sunIntensity: 1.0,
    sunPosition: [-6, 7, 6],
    hemiSky: 0x4a5f7d,
    hemiGround: 0x3d4652,
    hemiIntensity: 0.4,
    fogColor: 0x2c3a50,
    fogNear: 18,
    fogFar: 46,
    exposure: 0.95,
    skyBelow: 0x0a121e,
    skyHorizon: 0x3d5670,
    skyMid: 0x243a54,
    skyZenith: 0x121f33,
    mistColor: 0x46586e,
    mistOpacity: 0.18,
    cloudTint: 0x8fa3bd,
    sunDiscColor: 0xdfe8f2,
    giScale: 0.9,
    saturation: 1.08,
    contrast: 1.05,
    lampBoost: 1.2,
    showStars: true,
    lampOn: true,
  },
};

// Physical sky (SkyMesh Rayleigh/Mie) per preset + the sun's day arc.
export const skyPhys = {
  morning: { turbidity: 3, rayleigh: 1.3, mieCoefficient: 0.006, mieG: 0.8, timeOfDay: 0.12, sunBoost: 1.15 },
  dusk: { turbidity: 6, rayleigh: 3.0, mieCoefficient: 0.02, mieG: 0.9, timeOfDay: 0.96, sunBoost: 2.3 },
  night: { turbidity: 2, rayleigh: 1, mieCoefficient: 0.005, mieG: 0.8, timeOfDay: 1.08, sunBoost: 0 },
} as const;

export const sunArcCfg = {
  azRise: 0.15,
  azSet: 0.95,
  elevMax: 1.15,
  elevBase: -0.04,
  colorLow: 0xff6f2a,
  colorHigh: 0xfff1dd,
  cycleSeconds: 48,
} as const;

// Procedural cloud dome (fbm noise, sun-lit)
export const cloudCfg = {
  scale: 2.1,
  coverage: { morning: 0.45, dusk: 0.6, night: 0.35 },
  drift: 0.008,
  litBoost: 1.6,
} as const;

// Warm interior glow for lamp-lit presets.
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
  godraysMixDusk: 0.6,
  godraysDensity: 0.35,
  godraysMaxDensity: 0.32,
} as const;

// DREDGE-style split toning: shadows lean teal, highlights lean amber.
export const grade = {
  shadowTint: [0.88, 1.0, 1.1] as [number, number, number],
  highlightTint: [1.07, 1.0, 0.91] as [number, number, number],
  low: 0.3,
  high: 0.8,
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
