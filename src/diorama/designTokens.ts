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
  hemiSky: number;
  hemiGround: number;
  hemiIntensity: number;
  fogColor: number;
  fogNear: number;
  fogFar: number;
  exposure: number;
  mistColor: number;
  mistOpacity: number;
  giScale: number;
  saturation: number;
  contrast: number;
  lampBoost: number;
  showStars: boolean;
  lampOn: boolean;
};

export const lightPresets: Record<'morning' | 'dusk' | 'night', LightPreset> = {
  morning: {
    hemiSky: 0xc4dcda,
    hemiGround: 0xe4d3ba,
    hemiIntensity: 0.6,
    fogColor: 0xeee2cf,
    fogNear: 20,
    fogFar: 48,
    exposure: 1.18,
    mistColor: 0xf6e9d2,
    mistOpacity: 0.16,
    giScale: 0.7,
    saturation: 1.1,
    contrast: 1.0,
    lampBoost: 1.0,
    showStars: false,
    lampOn: false,
  },
  // The DREDGE moment: amber horizon burning under a deep teal sky.
  dusk: {
    hemiSky: 0x4f7d84,
    hemiGround: 0x5c5348,
    hemiIntensity: 0.42,
    fogColor: 0x486e74,
    fogNear: 18,
    fogFar: 46,
    exposure: 0.96,
    mistColor: 0x6f949a,
    mistOpacity: 0.22,
    giScale: 0.55,
    saturation: 1.12,
    contrast: 1.06,
    lampBoost: 1.35,
    showStars: false,
    lampOn: true,
  },
  night: {
    hemiSky: 0x4a5f7d,
    hemiGround: 0x3d4652,
    hemiIntensity: 0.4,
    fogColor: 0x2c3a50,
    fogNear: 18,
    fogFar: 46,
    exposure: 0.95,
    mistColor: 0x46586e,
    mistOpacity: 0.18,
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
  morning: { turbidity: 2.2, rayleigh: 2.6, mieCoefficient: 0.006, mieG: 0.8, timeOfDay: 0.12, sunBoost: 1.3 },
  dusk: { turbidity: 6, rayleigh: 3.0, mieCoefficient: 0.02, mieG: 0.9, timeOfDay: 0.96, sunBoost: 2.3 },
  night: { turbidity: 2, rayleigh: 1, mieCoefficient: 0.005, mieG: 0.8, timeOfDay: 1.08, sunBoost: 0 },
} as const;

// --- Realtime environment: art-directed keyframes over real sun elevation ---
// The old presets live on as keyframes: night (<-6°), goldenMorning/-Evening
// (anchored at +4°, chosen by whether the sun is rising), day (>25°, NEW).
export type EnvKeyframe = {
  hemiSky: number; hemiGround: number; hemiIntensity: number;
  fogColor: number; fogNear: number; fogFar: number;
  exposure: number; mistColor: number; mistOpacity: number;
  giScale: number; saturation: number; contrast: number;
  godraysMix: number; lampOn01: number;
  turbidity: number; rayleigh: number; mieCoefficient: number; mieG: number;
  sunBoost: number; cloudCoverageBase: number;
};

export const envKeyframes: { night: EnvKeyframe; goldenMorning: EnvKeyframe; goldenEvening: EnvKeyframe; day: EnvKeyframe } = {
  night: {
    hemiSky: 0x4a5f7d, hemiGround: 0x3d4652, hemiIntensity: 0.4,
    fogColor: 0x2c3a50, fogNear: 18, fogFar: 46,
    exposure: 0.95, mistColor: 0x46586e, mistOpacity: 0.18,
    giScale: 0.9, saturation: 1.08, contrast: 1.05,
    godraysMix: 0, lampOn01: 1,
    turbidity: 2, rayleigh: 1, mieCoefficient: 0.005, mieG: 0.8,
    sunBoost: 0, cloudCoverageBase: 0.4,
  },
  goldenMorning: {
    hemiSky: 0xc4dcda, hemiGround: 0xe4d3ba, hemiIntensity: 0.6,
    fogColor: 0xeee2cf, fogNear: 20, fogFar: 48,
    exposure: 1.18, mistColor: 0xf6e9d2, mistOpacity: 0.16,
    giScale: 0.7, saturation: 1.1, contrast: 1.0,
    godraysMix: 0.35, lampOn01: 0,
    turbidity: 2.2, rayleigh: 2.6, mieCoefficient: 0.006, mieG: 0.8,
    sunBoost: 1.3, cloudCoverageBase: 0.44,
  },
  // The DREDGE moment — amber horizon under deep teal — now fires at the REAL dusk.
  goldenEvening: {
    hemiSky: 0x4f7d84, hemiGround: 0x5c5348, hemiIntensity: 0.42,
    fogColor: 0x486e74, fogNear: 18, fogFar: 46,
    exposure: 0.96, mistColor: 0x6f949a, mistOpacity: 0.22,
    giScale: 0.55, saturation: 1.12, contrast: 1.06,
    godraysMix: 0.6, lampOn01: 1,
    turbidity: 6, rayleigh: 3.0, mieCoefficient: 0.02, mieG: 0.9,
    sunBoost: 2.3, cloudCoverageBase: 0.62,
  },
  // NEW curation: bright, neutral midday — flat contrast, no drama, lamp off.
  day: {
    hemiSky: 0xbfd9e6, hemiGround: 0xe7dcc4, hemiIntensity: 0.75,
    fogColor: 0xe8eef2, fogNear: 22, fogFar: 52,
    exposure: 1.12, mistColor: 0xf2f3ee, mistOpacity: 0.1,
    giScale: 0.75, saturation: 1.04, contrast: 1.0,
    godraysMix: 0.15, lampOn01: 0,
    turbidity: 3, rayleigh: 2.2, mieCoefficient: 0.005, mieG: 0.8,
    sunBoost: 1.0, cloudCoverageBase: 0.44,
  },
};

// Sun-elevation anchors (degrees) for keyframe interpolation.
export const envAnchors = { nightBelowDeg: -6, goldenPeakDeg: 4, dayAboveDeg: 25 } as const;

// How real weather modulates the look. All weather→look constants live here.
export const weatherLook = {
  coverageMin: 0.15, coverageMax: 0.85, // cloud_cover 0..1 → raymarcher coverage window
  sunDampMax: 0.75, // full overcast removes 75% of direct sun
  hemiBoostMax: 0.35, // ...and adds up to 35% diffuse hemi
  fogVisFullM: 200, fogVisClearM: 4000, // visibility → fog factor ramp
  fogNearMin: 4, fogFarMin: 22, // fully fogged near/far
  precipFullMmPerH: 5, // 5 mm/h = full-intensity particles
  snowTempC: 1, // precip at or below this temperature falls as snow
  driftBase: 0.006, driftPerMs: 0.0011, // cloud drift = base + speed(m/s) * perMs
  rainColor: 0xaebfd4, snowColor: 0xf4f7fb,
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

// Volumetric clouds: raymarched height-band slab (Beer-Lambert + powder).
export const cloudVol = {
  base: 13, // slab bottom (world y)
  top: 22, // slab top (world y)
  steps: 36, // primary march samples
  lightSteps: 4, // secondary march toward the light
  lightStep: 1.6, // world units per light-march step
  scale: 0.05, // world -> noise frequency
  coverage: { morning: 0.44, dusk: 0.62, night: 0.4 },
  density: 1.3, // extinction multiplier on the primary march
  absorption: 1.3, // extinction multiplier on the light march
  drift: 0.015, // noise-space wind per second
  litBoost: 1.35,
  maxDist: 90, // clamp the near-horizon march length
} as const;

// Warm interior glow for lamp-lit presets.
export const nightGlow = {
  bulb: 0xffc98a,
  lampIntensity: 26,
} as const;

// Moonlight (the night preset's key light — the sun arc is parked below horizon).
export const moonLight = { color: 0xa8c4e8, intensity: 1.0, position: [-6, 7, 6] as [number, number, number] } as const;

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
