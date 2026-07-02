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
  // pure white: neutral base for materials whose diffuse comes from
  // per-instance/vertex colors, and the clay sheen's lerp target
  trueWhite: 0xffffff,
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
  // sheenColor = diffuse color lerped this far toward palette.trueWhite
  sheenLerp: 0.5,
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

// Legacy fbm cloud dome — still drives the KSW scene's cloud shell
// (ksw/main.ts); look.ts moved on to the volumetric slab above.
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

// ── KSW hospital diorama (ksw.html) ─────────────────────────────────────
// Additions for the full-hospital scene; the prototype tokens above stay
// untouched so look.ts keeps rendering identically.

export const kswPalette = {
  roofClay: 0xd9a184, // terracotta roof lids
  roofTrim: 0xe8bfa0,
  corridorFloor: 0xf2ead9,
  plazaPath: 0xe5d9c0,
  opLight: 0xfff3d6, // OP lamp discs (unlit mesh color)
  screenGlow: 0xbfe3d9, // device screens
  crossRed: 0xd8574a, // Swiss hospital cross signage block
} as const;

// Dynamic diorama camera: wheel dolly, left-drag orbit. Roofs fade with the
// zoom radius so zooming in reveals the interiors.
export const kswCamera = {
  fov: 24,
  overviewPosition: [-68, 42, 78] as [number, number, number],
  target: [0, 0.6, 0] as [number, number, number],
  radiusMin: 7,
  radiusMax: 320,
  zoomSpeed: 0.0012,
  dragSpeed: 0.005,
  pitchMin: 0.18,
  pitchMax: 1.25,
  // roofs are FULLY gone by the time a screen-filling chunk of the hospital
  // is in view (radius <= 60) — no lingering translucency in the nav range
  roofFadeNear: 60,
  roofFadeFar: 95,
  zoomSmoothing: 10, // 1/s — wheel zoom eases toward its target radius
  // AoE2-style edge scrolling: cursor at the viewport edge pans the target
  panMarginPx: 36,
  panSpeed: 30,
  panBoundsX: 34,
  panBoundsZ: 26,
} as const;

// Scene scale-up relative to the one-room prototype.
export const kswScene = {
  fogScale: 7,
  domeRadius: 400, // clouds/stars stay around the camera even at max zoom-out
  skyScale: 900, // sky sphere beyond radiusMax, inside camera.far
  sunDistance: 70,
  shadowExtent: 46,
  shadowMapSize: 4096,
  giProbeY: 9,
  wallHeight: 2.9,
  wallThicknessOuter: 0.42,
  wallThicknessInner: 0.28,
  roofThickness: 0.26,
  roofOverhang: 0.2,
  plateThickness: 0.5,
  openingSill: 0.95, // window sill height
  openingHead: 2.35, // shared head height for doors and windows
} as const;

// KSW post overrides (the prototype `post` tokens belong to look.ts).
export const kswPost = {
  dof: { focalLength: 1.4, bokehScale: 1.6 },
  // the big plate sees mostly open sky in the GI probe — damp the white
  // wash per preset so the sun stays the protagonist (the unfogged morning
  // sky is by far the brightest)
  envScale: { morning: 0.32, dusk: 0.6, night: 0.8 },
  // the unfogged bright sky feeds screen-space godrays/bloom — keep the veil off
  godraysMix: { morning: 0.12, dusk: 0.5 },
  bloomThreshold: 1.05,
  // unfogged sky only where it reads better: the low morning sun renders a
  // near-white sky that washes the whole frame, so morning keeps the fog tint
  skyUnfogged: { morning: false, dusk: true, night: true },
} as const;

// Agent crowds (Slice D of the 10k-perf design). Above crowdThreshold the
// renderer switches to crowd mode: GPU LOD/frustum classification + blob
// shadows instead of real shadow casters. At or below it (the authored 72)
// nothing changes — real PCSS shadows, LOD0 beans only.
export const kswAgents = {
  crowdThreshold: 200,
  maxAgents: 20000, // ?agents=N clamp ceiling
  lodDistance: 35, // camera distance (m) where the bean swaps to the capsule LOD1
  cullDistance: 600, // beyond this an instance collapses entirely (scale 0)
  frustumMargin: 2.5, // meters of slack around the frustum planes (agent extent)
  planBudget: 64, // max routePath plans per frame; expired dwells retry next frame
  blob: { radius: 0.55, opacity: 0.25, lift: 0.03, color: 0x000000 }, // soft ground-disc shadow
} as const;

// Roof-fade policy thresholds, shared by staticBatch.setFade (castShadow /
// visibility / depthWrite flips) and main.ts (GI + shadow-map refresh
// triggers, fade-settle detection). Single source — the two sides MUST agree.
export const roofFadePolicy = {
  castShadow: 0.6, // fade above this: the roof batch casts shadows
  visible: 0.02, // fade above this: the roof batch renders at all
  opaque: 0.999, // fade at/above this counts as fully settled/opaque (depthWrite on)
} as const;

// GI cube-probe refresh cadence (giProbe.ts). Static presets render one
// probe face every staticFaceInterval frames in the background (full cube
// per 6x that — the pre-Slice-E cadence, minus the 6-in-one-frame hitch);
// dirty triggers still walk 6 consecutive faces at one per frame.
export const kswGi = {
  staticFaceInterval: 40,
  // PMREM blurs the environment heavily anyway; 128px keeps the background
  // face render under the frame budget at 10k agents.
  probeSize: 128,
} as const;

// Camera contract — the diorama has ONE gaze, like a built miniature.
// From the south-west, looking into the corner formed by the north + east walls.
export const cameraContract = {
  fov: 24,
  position: [-9.2, 6.8, 10.8] as [number, number, number],
  target: [0.4, 0.9, -0.5] as [number, number, number],
} as const;

// Winterthur city context (geo slices S1/S2). Scale-extension values only —
// the hero look tokens above are untouched; these govern the big city plate.
export const kswCity = {
  radiusMax: 1500, // wheel dolly ceiling to frame the whole Bahnhof↔ZAG span
  domeRadius: 1800, // clouds/stars dome swallows the city plate
  skyScale: 4000,
  cameraFar: 12000,
  roadY: 0.04, // road ribbons float just above the plate (no z-fight)
  railY: 0.05,
} as const;
