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

// ── Realtime environment (klinik/look-prototype) ────────────────────────
// Art-directed keyframes over real sun elevation + real weather. The
// prototype's look.ts and environment/* consume these, and the KSW city
// (ksw/main.ts) was rewired onto the same realtime environment in Task 4;
// the old preset architecture (lightPresets/skyPhys/etc.) was deleted in
// Task 5.

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
  sunBoost: number;
};

export const envKeyframes: { night: EnvKeyframe; goldenMorning: EnvKeyframe; goldenEvening: EnvKeyframe; day: EnvKeyframe } = {
  // hemiIntensity values below fold in the old prototype's gi.hemiCut=0.5
  // (deleted from environment.ts) so hemisphere brightness matches the
  // art-directed original instead of coming out 2x too bright.
  // Recurated: was reading as a warm-lit evening, not night. Dropped exposure
  // and GI so the room goes dark enough for stars + the moon terminator to
  // register; cooler mist/hemi keeps the night blue rather than amber.
  // SOTA-night rebalance (2026-07-06): the readable-city share moved from the
  // moon key (now a 0.5-peak rim, environment.ts) onto a stronger cool hemi —
  // silhouettes stay legible ("never true black") while warm windows + lamps
  // carry the scene.
  night: {
    // Urban skyglow: a real city is never black — cool sky dome + a warm
    // ground bounce from thousands of lamps. The hemi COLORS are the ambient
    // ceiling (intensity alone can't lift a 0x3a-dark tint), so they carry
    // the readable-silhouette level.
    // Intensity 1.3: calibrated live against AgX — 0.44 rendered pitch black,
    // 6 read as blue daylight; 1.3–1.4 is the readable-but-nocturnal band.
    hemiSky: 0x6e87b8, hemiGround: 0x5c5040, hemiIntensity: 1.3,
    fogColor: 0x1c2536, fogNear: 30, fogFar: 85,
    exposure: 1.0, mistColor: 0x3c4d62, mistOpacity: 0.12,
    giScale: 0.68, saturation: 1.08, contrast: 1.08,
    godraysMix: 0, lampOn01: 1,
    turbidity: 2, rayleigh: 1, mieCoefficient: 0.005, mieG: 0.8,
    sunBoost: 0,
  },
  goldenMorning: {
    hemiSky: 0xc4dcda, hemiGround: 0xe4d3ba, hemiIntensity: 0.3,
    fogColor: 0xeee2cf, fogNear: 20, fogFar: 48,
    exposure: 1.18, mistColor: 0xf6e9d2, mistOpacity: 0.16,
    giScale: 0.7, saturation: 1.1, contrast: 1.0,
    godraysMix: 0.35, lampOn01: 0,
    turbidity: 2.2, rayleigh: 2.6, mieCoefficient: 0.006, mieG: 0.8,
    sunBoost: 1.3,
  },
  // The DREDGE moment — amber horizon under deep teal — now fires at the REAL dusk.
  goldenEvening: {
    hemiSky: 0x4f7d84, hemiGround: 0x5c5348, hemiIntensity: 0.21,
    fogColor: 0x486e74, fogNear: 18, fogFar: 46,
    exposure: 0.96, mistColor: 0x6f949a, mistOpacity: 0.22,
    giScale: 0.55, saturation: 1.16, contrast: 1.06,
    godraysMix: 0.6, lampOn01: 1,
    turbidity: 6, rayleigh: 3.0, mieCoefficient: 0.02, mieG: 0.9,
    sunBoost: 2.3,
  },
  // NEW curation: bright, neutral midday — flat contrast, no drama, lamp off.
  // Recurated: exposure/hemi/sunBoost were blowing the scene to near-white;
  // pulled back so surfaces read their clay tint and keep gentle contrast.
  // SOTA-2026 de-milk (2026-07-06): the city framing read as a washed-out
  // white veil — mist + near fog + godrays stacked over the whole frame.
  // Push fog out, halve the mist, drop the godrays veil, and let saturation
  // carry the cozy palette instead of haze.
  day: {
    hemiSky: 0xbfd9e6, hemiGround: 0xe7dcc4, hemiIntensity: 0.28,
    fogColor: 0xe8eef2, fogNear: 34, fogFar: 90,
    exposure: 0.98, mistColor: 0xf2f3ee, mistOpacity: 0.04,
    giScale: 0.7, saturation: 1.1, contrast: 1.05,
    godraysMix: 0.07, lampOn01: 0,
    turbidity: 3, rayleigh: 2.2, mieCoefficient: 0.005, mieG: 0.8,
    sunBoost: 0.66,
  },
};

// Sun-elevation anchors (degrees) for keyframe interpolation.
// nightBelowDeg −10 (was −6): full night only past nautical-twilight depth —
// the −6 cut snapped a 17:35 winter evening straight to pitch black, skipping
// the blue hour entirely (SOTA-night pass 2026-07-06).
export const envAnchors = { nightBelowDeg: -10, goldenPeakDeg: 4, dayAboveDeg: 25 } as const;

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

// Precipitation particle geometry/alpha (rain streak vs snow flake), plus the
// box/count value sets. `room` = the prototype's former COUNT/BOX constants
// (unchanged); `city` = curated starting values for the KSW scene (Task 6
// capture-review tunes further). Kept next to weatherLook since these are the
// visual constants for precipitation.ts.
export const precipLook = {
  rainSx: 0.02, rainSy: 0.55, // thin vertical rain streak (scene units)
  snowSx: 0.06, snowSy: 0.06, // small square snow flake
  rainAlpha: 0.4, snowAlpha: 0.85,
  room: { boxX: 24, boxY: 14, boxZ: 20, count: 3000 },
  // City precip read too thin at the diorama scale (Task-4 note): the 0.02×0.55
  // rain streak and 0.06 snow flake all but vanished over the 90-unit box. Give
  // the city its own slightly heavier streak/flake + higher count so rain fäden
  // and snow flocken actually register at the pulled-back city framing.
  // Task 6 look-review round 1: rain still barely read at the pulled-back city
  // framing — streaks were too thin/short/sparse to register as fäden. Curated
  // heavier: thicker+longer streak, higher alpha, denser count.
  rainCitySx: 0.11, rainCitySy: 1.8, snowCitySx: 0.14, snowCitySy: 0.14,
  rainCityAlpha: 0.6,
  city: { boxX: 90, boxY: 40, boxZ: 90, count: 13000 },
} as const;

// Curated cloud tint colors for applyEnvironment. Day mixes the sun color
// toward litWhite; night uses fixed lit/shadow tints.
export const cloudLook = {
  shadowBase: 0x6e8092,
  nightLit: 0x9fb2cc,
  nightShadow: 0x39485c,
  litWhite: 0xffffff,
  litWhiteMix: 0.3,
} as const;

// Sun color easing endpoints. environment.ts reads these to lerp the sun
// color over the day; the old day-arc geometry (azRise/azSet/elev*/
// cycleSeconds) was deleted in Task 5 along with the KSW preset architecture.
export const sunArcCfg = {
  colorLow: 0xff6f2a,
  colorHigh: 0xfff1dd,
} as const;

// Volumetric clouds: raymarched height-band slab (Beer-Lambert + powder).
// The realtime environment drives coverage/drift dynamically now, so those
// static fields were dropped from this token (see weatherLook for the ramp).
export const cloudVol = {
  base: 13, // slab bottom (world y)
  top: 22, // slab top (world y)
  steps: 36, // primary march samples
  lightSteps: 4, // secondary march toward the light
  lightStep: 1.6, // world units per light-march step
  scale: 0.05, // world -> noise frequency
  density: 1.3, // extinction multiplier on the primary march
  absorption: 1.3, // extinction multiplier on the light march
  litBoost: 1.35,
  maxDist: 90, // clamp the near-horizon march length
} as const;

// Legacy fbm cloud dome — still drives the KSW scene's cloud shell
// (ksw/main.ts); look.ts moved on to the volumetric slab above. The
// preset-keyed `coverage`/`drift` fields were dead (Task 5) — the realtime
// environment drives coverage/drift dynamically now (see weatherLook).
export const cloudCfg = {
  scale: 2.1,
  litBoost: 1.6,
} as const;

// Warm interior glow for lamp-lit presets.
export const nightGlow = {
  bulb: 0xffc98a,
  lampHead: 0xffe3b0, // warm lamp-head tint mixed in as lampGlowU rises to 1
  lampIntensity: 20,
  boost: 1.2, // applyEnvironment multiplies lampIntensity by this at full lampOn01
  cityPool: 14, // base intensity of the two forecourt PointLight pools
  emergency: 20, // base intensity of the emergency-zone PointLight
  // SOTA-night street lamps (2026-07-06): every lamp gets an instanced
  // additive light-pool disc on the ground plus an HDR bulb that clears the
  // bloom threshold — the shipped stylized-city recipe (pools + glow, no
  // per-lamp real lights). 2700K-ish warm; peak tuned so pools layer softly
  // where lamps cluster instead of clipping.
  // radius 5 + lift 0.3: a 13 m disc at 0.13 m lift clipped into terrain
  // undulation and read as torn half-moons; smaller + higher floats clean.
  pool: { color: 0xffb869, radius: 5, peak: 0.65, lift: 0.5 },
  bulbHdr: 3.0, // night bulb luminance (× warm tint) — past bloomThreshold 1.05
} as const;

// Moonlight (the night preset's key light). Only color/intensity are read
// (look.ts, applyCityEnvironment.ts, applyEnvironment.ts); the old `position`
// field was dead (Task 5).
export const moonLight = { color: 0xa8c4e8, intensity: 1.0 } as const;

// Moon disc terminator shading — dark (unlit) and lit hemisphere colors.
export const moonDisc = { dark: 0x292e3b, lit: 0xdee8f2 } as const;

// Post-processing recipe — the miniature magic. All post values live here.
export const post = {
  dof: { focusDistance: 16.5, focalLength: 1.4, bokehScale: 2.2 },
  bloom: { strength: 0.12, radius: 0.3, threshold: 0.9 },
  filmGrain: 0.08,
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
export const gi = { environmentIntensity: 0.28 } as const;

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
  keyRotateSpeed: 1.2, // rad/s for Q/E keyboard rotation (~69 deg/s)
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

// KSW post overrides (the prototype `post` tokens belong to look.ts). The
// preset-keyed `envScale`/`godraysMix` (Record) and `skyUnfogged` fields were
// dead (Task 5) — superseded by the scalar/derived fields below, which the
// realtime environment drives per-frame instead.
export const kswPost = {
  // bokehScale 1.6 blurred most of every establishing frame into mush —
  // the tilt-shift only earns its keep as a whisper (SOTA pass 2026-07-06).
  dof: { focalLength: 1.4, bokehScale: 0.7 },
  // Task 4: the realtime environment supplies a per-frame giScale; the city
  // damps the GI-probe white-wash by this fixed scalar on top (= the former
  // envScale.morning, the value the overview framing was tuned at).
  envScaleScalar: 0.32,
  // Task 4: env.godraysMix ranges over the keyframe godraysMix (peak 0.35 at
  // golden morning); the city's tuned veil was a fixed 0.12, so scale the live
  // uniform by 0.12/0.35 to land on the same look. = godraysMix.morning /
  // envKeyframes.goldenMorning.godraysMix = 0.12 / 0.35.
  godraysScale: 0.12 / 0.35,
  bloomThreshold: 1.05,
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
  // Pan roam bounds for the whole-Gemeinde world (Endless Horizon #119) —
  // kswCamera's ±34/±26 is a hero-room leftover, smaller than the KSW footprint.
  // The built city plate (meta.json: center (-451, 545), 1187×1506) reaches
  // x=1044.5, z=1298 from the KSW origin; these clear it with margin so WASD /
  // edge pan can roam KSW↔Bahnhof↔ZAG and a ring of surrounding terrain.
  panBoundsX: 1500,
  panBoundsZ: 1800,
  domeRadius: 1800, // clouds/stars dome swallows the city plate
  skyScale: 4000,
  cameraFar: 12000,
  roadY: 0.04, // road ribbons float just above the plate (no z-fight)
  railY: 0.05,
  // nature layer (real OSM greens/water/trees), below the road ribbons
  greenY: 0.025,
  waterY: 0.03,
  parkGreen: 0xa9cf92, // parks, grass, pitches — a step livelier than the lawn
  woodGreen: 0x86b478, // forest/wood patches read deeper
  water: 0xa8cfdd, // Eulach + ponds, calm glass blue
  // treeGreen deepened + saturated (was 0x8fbf83 pale sage — canopies read as
  // washed-out eggs at every framing; SOTA pass 2026-07-06). Cozy refs (Tiny
  // Glade) run saturated mid-greens and let hue variation do the talking.
  treeGreen: 0x74ad5c, // canopy base; per-instance tint varies around it
  treeTrunk: 0xb08a62,
  // roads v2: per-class colors + heights (carriage/footway/rail on distinct
  // levels so junctions never z-fight; footways read thinner+lighter)
  roadColors: { carriage: 0xcfc4b2, footway: 0xe5dcc8, rail: 0x8d949c, railBed: 0xb9b2a4 },
  // Deliberate lift LADDER (metres above the draped DEM surface), bottom→top:
  //   railBed 0.06  <  carriage 0.10  ≈  footway 0.11  <  rail 0.16
  // Rationale: post-#119 the ribbons drape onto per-vertex DEM samples. Even
  // after subdividing each segment to terrain resolution (roads.ts SUBDIVIDE_M)
  // two layers that drape onto their OWN centrelines (rail vs carriage at a
  // crossing) sample the DEM at slightly different points, so the old 10 mm
  // rail-over-carriage gap was smaller than that inter-sample noise → z-fight
  // flicker. This ladder opens the gaps to ≥40 mm (carriage→rail = 60 mm) so
  // the height alone resolves the ordering even with sample jitter; the ribbon
  // materials additionally carry polygonOffset (roads.ts) as belt-and-braces.
  // The base carriage lift is 0.10 m (was 0.04) so a convex terrain bump
  // between subdivided vertices can no longer poke up through the ribbon.
  roadYs: { carriage: 0.1, footway: 0.11, railBed: 0.06, rail: 0.16 },
} as const;

// S3c Dollhouse-Cutaway (T18): additive token block. `cutSeam` = the bright
// trim band thickness (m) just below the currently-dissolving storey's cut
// line; `seamColor` = the warm cut-edge tone. The zoom-radius peel window
// (when the cut engages, how far it opens) has moved to `kswPeel` below —
// this block only carries the seam's own look. Separate from
// roofFadePolicy — the cutaway drives the MAIN KSW building only, never the
// city roofs.
export const kswS3 = { cutSeam: 0.25, seamColor: 0xf3e2c8 } as const;
// Storey-peel dollhouse (Phase A): the orbit-radius window over which the main
// building peels open storey-by-storey (startR: closed; endR: only the EG
// remains), and the storey-height model derived from the baked eave height.
export const kswPeel = { startR: 110, endR: 40, nominalStoreyH: 3.4, minStoreyH: 2.4, maxStoreyH: 4.5, maxStoreys: 12 } as const;

// Night sky (star field + moon disc) value sets. `room` values are EXACTLY
// the former look.ts inline constants (STAR_R=17, quad 0.05, count 420, moon
// radius 0.46 @ distance 17); `city` scales onto the KSW dome (domeRadius
// 400) with curatable star quad/count — Task 6 capture-review tunes further.
export const nightSkyLook = {
  room: { starRadius: 17, starQuad: 0.05, starCount: 420, moonRadius: 0.46, moonDistance: 17 },
  // Scale the night sky onto the CITY dome (kswCity.domeRadius 1800), not the
  // hero dome (kswScene.domeRadius 400). The establishing cam=city framing
  // dollies out to radius 820 — well outside the old 340-unit star sphere, so
  // stars + moon fell behind the camera and city-night rendered black. The
  // cloud layer already swaps onto the 1800 dome; the sky must too. Quad/moon
  // sizes scale up proportionally (~×4.5) to keep the same apparent size, and
  // the star count is raised so the far-larger sphere doesn't read sparse.
  // Task 6 look-review round 1: at 1080p the ×4.5-scaled 2.7 quad still read as
  // ~4.5px specks — the night sky looked starless. Curated up (quad ×2, count
  // ~×1.8) until a real starfield reads at the city framing, and the moon disc
  // enlarged so it's an unmistakable disc, not a dot. Apparent-size honesty
  // holds — the moon still sits at its true elevation for the capture time.
  // sunDistance == moonDistance: sun and moon share the one city sky dome.
  city: { starRadius: kswCity.domeRadius * 0.85, starQuad: 5.5, starCount: 2200, moonRadius: 26, moonDistance: kswCity.domeRadius * 0.82, sunDistance: kswCity.domeRadius * 0.82 },
} as const;

// Tree size policy (SOTA pass 2026-07-06): the baked OSM/estimate heights are
// data-honest (median 12.5 m, q95 20 m) but park giants of 27 m dwarf the
// 2–3-storey Winterthur stock and the crowns read as fat broccoli at the city
// framing. Cap the outliers and slim every crown — height stays believable
// (cozy games keep trees TALL relative to houses), the silhouette gets airier.
export const treeLook = { maxH: 19, crownSlim: 0.8 } as const;

// Terrain landcover tint table (geo terrain tiles, Task 11). Anchored to the
// existing city-nature palette: meadow reuses kswCity.parkGreen exactly,
// forest a touch deeper than kswCity.woodGreen (bare grid terrain reads
// flatter than instanced tree canopies, so it wants a bit more contrast),
// water reuses kswCity.water exactly. Residential/industrial/farmland/rock
// are new muted clay tones kept inside the same desaturated family — no
// saturated "landuse map" colors.
export const terrainLook = {
  meadow: kswCity.parkGreen,
  forest: 0x6f9b64,
  farmland: 0xcdb583,
  residentialLu: 0xd8c7ab,
  industrialLu: 0xb9ac9a,
  water: kswCity.water,
  rock: 0x9a978d,
} as const;

// Diorama-style layer for the geodetic city (style slice). Additive only.
export const kswCityStyle = {
  plinthH: 0.5, plinthOut: 0.12, plinthSink: 0.3, // sockel: height, outset, below-plate sink
  eaveBandH: 0.18, eaveBandOut: 0.08,
  tintL: 0.06, tintHue: 0.012, // tamed variation (was ±14% L)
  windowW: 1.3, windowH: 1.4, windowSpacing: 2.4, storeyH: 3.0, sillFrac: 0.32,
  doorW: 1.5, doorH: 2.2,
  lamp: { spacing: { primary: 25, secondary: 28, tertiary: 30, residential: 35, unclassified: 35, living_street: 35, service: 45, pedestrian: 30 } as Record<string, number>, sideOffset: 1.2 },
  // midR 1200 (was 600): the facade window raster is a shader branch, not
  // geometry — hiding it at the city establishing framing (radius ~820) left
  // every building a naked clay block. Lamps/footways ride the same ring;
  // both are single instanced/merged draws, so keeping them on is free.
  lod: { nearR: 150, midR: 1200, hysteresis: 0.1 },
  cloudSwap: { start: 300, end: 600 },
} as const;
