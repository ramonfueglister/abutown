// KSW hospital diorama entry: full Kantonsspital Winterthur on one level,
// clay look inherited from the prototype tokens, dynamic camera (wheel
// dolly zoom, left-drag orbit) and roofs that fade out as you zoom in.
// Scene scaffolding (sky, sun arc, clouds, GI, post stack) follows
// src/diorama/look.ts, re-tuned for the large plate via kswScene tokens.

import * as THREE from 'three/webgpu';
import { Fn, dot, float, mix, mx_fractal_noise_float, nodeObject, pass, mrt, output, normalView, positionWorld, select, smoothstep, texture, uniform, vec2, vec3, vec4, velocity } from 'three/tsl';
import { ao } from 'three/addons/tsl/display/GTAONode.js';
import { dof } from 'three/addons/tsl/display/DepthOfFieldNode.js';
import { bloom } from 'three/addons/tsl/display/BloomNode.js';
import { film } from 'three/addons/tsl/display/FilmNode.js';
import { godrays } from 'three/addons/tsl/display/GodraysNode.js';
import { traa } from 'three/addons/tsl/display/TRAANode.js';
import { SkyMesh } from 'three/addons/objects/SkyMesh.js';
import {
  cloudCfg,
  gi,
  grade,
  kswAgents,
  kswCamera,
  kswCity,
  kswCityStyle,
  kswGi,
  kswPost,
  kswScene,
  nightGlow,
  nightSkyLook,
  palette,
  post,
  roofFadePolicy,
} from '../designTokens';
import { computeEnvironment, type EnvironmentState } from '../environment/environment';
import { applyCityEnvironment, type CityEnvironmentTargets } from './applyCityEnvironment';
import { createPrecipitation } from '../environment/precipitation';
import { createStarField, createMoonDisc } from '../environment/nightSky';
import { CLEAR_SKY, sampleWeather, startWeatherLoop, type WeatherSeries, type WeatherState } from '../environment/weather';
import { precipLook } from '../designTokens';
import { applyDrag, applyPan, applyZoom, edgePanVelocity, rigPosition, roofFade, type CameraRigState } from './cameraRig';
import { approach, createAgentInstances, lerpAngle, type AgentSlot } from './agentMeshes';
import { buildNav } from './nav';
import { buildSpawnSpecs } from './agentSpawn';
import { ANIMATED_TAGS } from './staticBatch';
import { advancePlanCursor, createAgent, updateAgent, type Agent } from './agents';
import { GiProbeScheduler, renderProbeFace } from './giProbe';
import { boxGeo } from './geometryCache';
import { clayMat } from './props';
import { buildCityMassing } from './geo/cityMassing';
import { buildKswCampus } from './geo/kswCampus';
import { decomposeToZones, type Zone } from './interior/zones';
import { generateInteriorPlan, departmentCenter } from './interior/generatePlan';
import { buildInterior } from './interior/buildInterior';
import { buildPlaza, buildHelipad } from './interior/plaza';
import { cutawayState } from './interior/cutaway';
import { buildRoads } from './geo/roads';
import { cityBuildings, cityMeta, cityNature, cityRails, cityRoads, kswBuildings } from './geo/geoData';
import { buildWindows } from './geo/windows';
import { buildLamps } from './geo/lamps';
import { lampGlowU } from './glowUniform';
import { buildNature } from './geo/nature';
import { applyCityLod, cityLodState, type CityLodRefs } from './geo/lod';
import type { PersonRole } from './floorPlan';

declare global {
  interface Window {
    __LOOK_READY?: boolean;
    __LOOK_BACKEND?: string;
    __ENV_STATE?: unknown;
    __KSW?: {
      radius: number;
      yaw: number;
      pitch: number;
      roofFade: number;
      target: [number, number, number];
      agents: { total: number; walking: number; samples: Array<[number, number]> };
    };
    __KSW_INFO?: () => {
      drawCalls: number;
      triangles: number;
      // main-thread cost per frame (EMA, ms): whole animate body, the agent
      // behavior+buffer-write loop, and the render call (command encoding)
      cpu: { frame: number; agents: number; render: number };
    };
  }
}

type CamPresetName = 'overview' | 'er' | 'ops' | 'bahnhof' | 'zag' | 'city';
const camPresets: Record<CamPresetName, { target: [number, number, number]; radius: number; yaw: number; pitch: number }> = {
  // Reframed for the real KSW complex (tower + wings) instead of the
  // stylized hero hospital footprint (S3a/T15): high and wide enough that
  // the whole campus plus a ring of city context reads as one diorama.
  overview: { target: [0, 4, -15], radius: 520, yaw: -0.55, pitch: 1.02 },
  // er/ops are re-aimed at boot onto the generated Notfall / OP zone centers
  // (departmentCenter, T18) with the cutaway-active radii (40 / 35). These are
  // provisional and overwritten before the rig initializes.
  er: { target: [-22.5, 0.4, 12], radius: 40, yaw: -0.5, pitch: 0.72 },
  ops: { target: [-24, 0.2, -16], radius: 35, yaw: 0.45, pitch: 1.05 },
  // real city landmarks (local meters from the KSW anchor, see cityMeta):
  // pulled back and tilted down so the camera sits above the dense district
  bahnhof: { target: [cityMeta.landmarks.bahnhof[0], 2, cityMeta.landmarks.bahnhof[1]], radius: 280, yaw: -0.6, pitch: 1.02 },
  zag: { target: [cityMeta.landmarks.zagTurbinenstrasse[0], 2, cityMeta.landmarks.zagTurbinenstrasse[1]], radius: 280, yaw: 0.4, pitch: 1.02 },
  // establishing shot: the whole KSW↔Bahnhof↔ZAG span from high up
  city: { target: [cityMeta.landmarks.bahnhof[0] * 0.6, 2, cityMeta.landmarks.bahnhof[1] * 0.6], radius: 820, yaw: -0.5, pitch: 1.12 },
};

// ?wx= pins a fixed weather state (else the live Open-Meteo loop drives it).
// Same table as look.ts (the room prototype) — kept 1:1.
const WX_OVERRIDES: Record<string, WeatherState> = {
  clear: CLEAR_SKY,
  overcast: { ...CLEAR_SKY, cloudCover: 0.97, windSpeedMs: 3 },
  rain: { ...CLEAR_SKY, cloudCover: 0.9, precipMmPerH: 4, windSpeedMs: 5, temperatureC: 10 },
  snow: { ...CLEAR_SKY, cloudCover: 0.9, precipMmPerH: 3, snow: true, temperatureC: -2 },
  fog: { ...CLEAR_SKY, cloudCover: 0.6, visibilityM: 150, fog: true, windSpeedMs: 0.5 },
};

async function boot(): Promise<void> {
  const params = new URLSearchParams(window.location.search);
  // ?at= freezes the clock to a fixed UTC instant (else real now()).
  const atParam = params.get('at');
  const frozenAt = atParam ? new Date(atParam) : null;
  if (frozenAt && Number.isNaN(frozenAt.getTime())) throw new Error(`invalid ?at=${atParam}`);
  const wxParam = params.get('wx'); // 'clear'|'overcast'|'rain'|'snow'|'fog'|null
  const now = (): Date => frozenAt ?? new Date();
  const camRaw = params.get('cam');
  const cityCams: CamPresetName[] = ['er', 'ops', 'bahnhof', 'zag', 'city'];
  const camPreset: CamPresetName = cityCams.includes(camRaw as CamPresetName) ? (camRaw as CamPresetName) : 'overview';
  // ?agents=N scales the crowd (clamped; default = the authored plan people)
  const agentsRaw = Number.parseInt(params.get('agents') ?? '', 10);
  const agentTarget = Number.isNaN(agentsRaw) ? undefined : Math.min(Math.max(agentsRaw, 1), kswAgents.maxAgents);
  // Realtime environment: physical sun/moon/stars for now() steered by live (or
  // ?wx-pinned) weather. Re-evaluated every frame; this is the boot seed.
  let lastEnv: EnvironmentState = computeEnvironment(now(), WX_OVERRIDES[wxParam ?? ''] ?? CLEAR_SKY);

  // ── generated interior plan (S3b, T17) computed up front so the T18 cutaway
  // presets (er/ops) can re-aim onto the Notfall / OP zone centers. The plan is
  // pure/deterministic; the built interior group is added to the scene below.
  const mainBuildingFp = kswBuildings.reduce((best, b) => {
    const area = (fp: number[][]): number => {
      let a = 0;
      for (let i = 0; i < fp.length; i++) {
        const [x1, z1] = fp[i];
        const [x2, z2] = fp[(i + 1) % fp.length];
        a += x1 * z2 - x2 * z1;
      }
      return Math.abs(a) / 2;
    };
    return area(b.footprint) > area(best.footprint) ? b : best;
  }, kswBuildings[0]);
  const interiorZones = decomposeToZones(mainBuildingFp.footprint);
  const mainDoor = mainBuildingFp.door ?? { x: interiorZones[0]?.x ?? 0, z: interiorZones[0]?.z ?? 0, yaw: 0 };
  const interiorPlan = generateInteriorPlan(interiorZones, mainDoor);
  // Re-aim er/ops onto the real department centers (radius keeps the cutaway
  // active: 40 for the emergency ward, 35 for the surgery block).
  const [erX, erZ] = departmentCenter(interiorPlan, 'Notfall');
  const [opX, opZ] = departmentCenter(interiorPlan, 'OP');
  camPresets.er = { target: [erX, 0.4, erZ], radius: 40, yaw: -0.5, pitch: 0.72 };
  camPresets.ops = { target: [opX, 0.2, opZ], radius: 35, yaw: 0.45, pitch: 1.05 };

  const renderer = new THREE.WebGPURenderer({ antialias: false });
  await renderer.init();
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  renderer.setSize(window.innerWidth, window.innerHeight);
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.PCFSoftShadowMap;
  renderer.toneMapping = THREE.AgXToneMapping;
  renderer.toneMappingExposure = lastEnv.exposure;
  document.body.appendChild(renderer.domElement);
  window.__LOOK_BACKEND = (renderer.backend as { isWebGPUBackend?: boolean }).isWebGPUBackend
    ? 'webgpu'
    : 'webgl2';

  const scene = new THREE.Scene();
  // fog scales with the zoom radius in animate(): identical look at the
  // overview framing, no white-out when zooming far out. applyCityEnvironment
  // writes the base near/far into fogBase each frame; animate() × zoom.
  const fogBase = { near: lastEnv.fogNear * kswScene.fogScale, far: lastEnv.fogFar * kswScene.fogScale };
  scene.fog = new THREE.Fog(lastEnv.fogColor, fogBase.near, fogBase.far);

  const initialSunDir = new THREE.Vector3(lastEnv.sunDir[0], lastEnv.sunDir[1], lastEnv.sunDir[2]);

  const skyMesh = new SkyMesh();
  // sky is a directional shader — enlarging the shell only moves it beyond the
  // city plate (so distant buildings never poke through), the look is unchanged
  skyMesh.scale.setScalar(kswCity.skyScale);
  skyMesh.turbidity.value = lastEnv.turbidity;
  skyMesh.rayleigh.value = lastEnv.rayleigh;
  skyMesh.mieCoefficient.value = lastEnv.mieCoefficient;
  skyMesh.mieDirectionalG.value = lastEnv.mieG;
  skyMesh.sunPosition.value.copy(initialSunDir);
  // The sky shell (r=skyScale) sits far beyond fogFar; leaving fog on would
  // flat-tint the whole sky. Off always (matches the room prototype).
  (skyMesh.material as THREE.Material & { fog: boolean }).fog = false;
  scene.add(skyMesh);

  // Procedural cloud dome (fbm, sun-lit silver lining)
  const sunDirUniform = uniform(initialSunDir.clone());
  const cloudLit = uniform(new THREE.Color(0xffffff));
  const cloudShadow = uniform(new THREE.Color(0x9aa8b5));
  // Wind-driven drift on scene x/z; applyCityEnvironment integrates it (the
  // noise y-component stays drift-free). Coverage is a shared live uniform.
  const driftUV = uniform(new THREE.Vector2(0, 0));
  const coverageU = uniform(0.44);
  // Two-layer clouds (spec §4): the hero dome fades out as the camera pulls
  // back to the city framing and a bigger, coarser city dome fades in. Both
  // opacities are driven by cloudMix (kswCityStyle.cloudSwap) in animate().
  const heroCloudOpacity = uniform(1);
  const cityCloudOpacity = uniform(0);
  const cloudMatDome = new THREE.MeshBasicNodeMaterial();
  cloudMatDome.transparent = true;
  cloudMatDome.side = THREE.BackSide;
  cloudMatDome.depthWrite = false;
  cloudMatDome.fog = false;
  {
    const dir = positionWorld.normalize();
    const p = vec3(dir.x.mul(float(cloudCfg.scale)).add(driftUV.x), dir.y.mul(float(cloudCfg.scale * 1.6)), dir.z.mul(float(cloudCfg.scale)).add(driftUV.y));
    const n = mx_fractal_noise_float(p, 4, 2.0, 0.55, 1.0);
    const dens = smoothstep(float(0.06), float(0.34), n.add(coverageU.sub(0.5)));
    const horizonFade = smoothstep(float(0.0), float(0.07), dir.y);
    // fold the hero fade into the opacity node before the material compiles
    cloudMatDome.opacityNode = dens.mul(horizonFade).mul(heroCloudOpacity);
    const facing = dot(dir, sunDirUniform).mul(0.5).add(0.5);
    type Vec3Node = ReturnType<typeof vec3>;
    const shadowN = cloudShadow as unknown as Vec3Node;
    const litN = (cloudLit as unknown as Vec3Node).mul(float(cloudCfg.litBoost));
    cloudMatDome.colorNode = mix(shadowN, litN, facing.pow(2.0));
  }
  const cloudDome = new THREE.Mesh(new THREE.SphereGeometry(kswScene.domeRadius, 32, 24), cloudMatDome);
  scene.add(cloudDome);

  // city cloud layer: same recipe, big dome, coarser noise (scale × 3) — takes
  // over as the hero dome fades out on zoom-out (spec: two-layer clouds).
  const cloudMatCity = new THREE.MeshBasicNodeMaterial();
  cloudMatCity.transparent = true;
  cloudMatCity.side = THREE.BackSide;
  cloudMatCity.depthWrite = false;
  cloudMatCity.fog = false;
  {
    const dir = positionWorld.normalize();
    const p = vec3(dir.x.mul(float(cloudCfg.scale * 3)).add(driftUV.x), dir.y.mul(float(cloudCfg.scale * 4.8)), dir.z.mul(float(cloudCfg.scale * 3)).add(driftUV.y));
    const n = mx_fractal_noise_float(p, 4, 2.0, 0.55, 1.0);
    const dens = smoothstep(float(0.06), float(0.34), n.add(coverageU.sub(0.5)));
    const horizonFade = smoothstep(float(0.0), float(0.07), dir.y);
    cloudMatCity.opacityNode = dens.mul(horizonFade).mul(cityCloudOpacity);
    const facing = dot(dir, sunDirUniform).mul(0.5).add(0.5);
    type Vec3Node = ReturnType<typeof vec3>;
    const shadowN = cloudShadow as unknown as Vec3Node;
    const litN = (cloudLit as unknown as Vec3Node).mul(float(cloudCfg.litBoost));
    cloudMatCity.colorNode = mix(shadowN, litN, facing.pow(2.0));
  }
  const cityCloudDome = new THREE.Mesh(new THREE.SphereGeometry(kswCity.domeRadius, 32, 24), cloudMatCity);
  scene.add(cityCloudDome);

  // Sun disc: position + visibility driven per-frame by applyCityEnvironment.
  const sunDisc = new THREE.Mesh(
    new THREE.SphereGeometry(5.2, 20, 20),
    new THREE.MeshBasicMaterial({ color: 0xfff0d5, fog: false }),
  );
  scene.add(sunDisc);

  // Moon disc + star field — extracted to environment/nightSky.ts, built with
  // the city dome values (nightSkyLook.city). Both always exist now;
  // visibility/opacity/rotation are driven by applyCityEnvironment.
  const { mesh: moonDisc, phaseDir: moonPhaseDirU } = createMoonDisc({
    radius: nightSkyLook.city.moonRadius,
    distance: nightSkyLook.city.moonDistance,
  });
  scene.add(moonDisc);

  const { object3d: starsObj, material: starsMat } = createStarField({
    radius: nightSkyLook.city.starRadius,
    quadSize: nightSkyLook.city.starQuad,
    count: nightSkyLook.city.starCount,
  });
  scene.add(starsObj);

  const sun = new THREE.DirectionalLight(0xffffff, 1);
  // Latest key-light direction — the shadow-follow rig (below) places the sun
  // relative to the camera target along this vector so the light angle stays
  // identical to the hero while the frustum walks the city. applyCityEnvironment
  // writes it IN-PLACE (day: sun dir, night: moon dir).
  const currentSunDir = initialSunDir.clone();

  // ── dynamic camera: wheel dolly + left-drag orbit ──────────────────────
  const start = camPresets[camPreset];
  let rig: CameraRigState = {
    yaw: start.yaw,
    pitch: start.pitch,
    radius: start.radius,
    target: start.target,
  };
  const camera = new THREE.PerspectiveCamera(kswCamera.fov, window.innerWidth / window.innerHeight, 0.1, kswCity.cameraFar);
  // zoom config: hero settings, but the dolly may pull back far enough to
  // frame the whole Bahnhof↔ZAG city (roof-fade still keyed off kswCamera)
  const zoomCfg = { ...kswCamera, radiusMax: kswCity.radiusMax };
  const applyRig = (): void => {
    camera.position.set(...rigPosition(rig));
    camera.lookAt(...rig.target);
  };
  applyRig();

  // the wheel only moves a target radius; animate() eases the rig toward it,
  // so both the dolly and the roof fade glide instead of stepping
  let zoomTarget = rig.radius;
  renderer.domElement.addEventListener(
    'wheel',
    (e: WheelEvent) => {
      e.preventDefault();
      zoomTarget = applyZoom({ ...rig, radius: zoomTarget }, e.deltaY, zoomCfg).radius;
    },
    { passive: false },
  );
  let dragging = false;
  // AoE2 edge scrolling: remember where the cursor is; animate() pans while
  // it sits inside the edge margin (paused during drag-rotation)
  let mouse: { x: number; y: number } | null = null;
  renderer.domElement.addEventListener('pointerdown', (e: PointerEvent) => {
    if (e.button !== 0) return;
    dragging = true;
    renderer.domElement.setPointerCapture(e.pointerId);
  });
  renderer.domElement.addEventListener('pointermove', (e: PointerEvent) => {
    mouse = { x: e.clientX, y: e.clientY };
    if (!dragging) return;
    rig = applyDrag(rig, e.movementX, e.movementY, kswCamera);
  });
  renderer.domElement.addEventListener('pointerleave', () => {
    mouse = null;
  });
  const endDrag = (e: PointerEvent): void => {
    dragging = false;
    if (renderer.domElement.hasPointerCapture(e.pointerId)) {
      renderer.domElement.releasePointerCapture(e.pointerId);
    }
  };
  renderer.domElement.addEventListener('pointerup', endDrag);
  renderer.domElement.addEventListener('pointercancel', endDrag);

  // ── light rig ──────────────────────────────────────────────────────────
  // The key light is fully driven by applyCityEnvironment (color/intensity/dir);
  // boot leaves it neutral. Only the shadow-camera setup below is static.
  sun.castShadow = true;
  sun.shadow.mapSize.set(kswScene.shadowMapSize, kswScene.shadowMapSize);
  sun.shadow.camera.left = -kswScene.shadowExtent;
  sun.shadow.camera.right = kswScene.shadowExtent;
  sun.shadow.camera.top = kswScene.shadowExtent;
  sun.shadow.camera.bottom = -kswScene.shadowExtent;
  sun.shadow.camera.near = 1;
  sun.shadow.camera.far = 220;
  sun.shadow.bias = -0.0004;
  sun.shadow.normalBias = 0.04;
  sun.shadow.radius = 5;
  // PCSS: blocker search -> penumbra-sized PCF (contact-hardening soft shadows)
  // penumbraScale corrects for the shadow-camera extent growing with zoom
  // (updateShadowFrustum, up to 900 m): the penumbra term below is tuned in
  // TEXELS for the hero extent (46 m), so a bigger extent -> bigger
  // meters-per-texel -> the same texel penumbra reads as a much larger blur
  // in world space. Scaling by shadowExtent/wantExtent keeps the blur's
  // world-space size constant; at the hero extent this is exactly 1.0, so
  // the hero frame is mathematically unchanged (pixel-treu).
  const penumbraScale = uniform(1);
  {
    const texel = 1 / kswScene.shadowMapSize;
    const taps: Array<[number, number]> = [];
    for (let i = 0; i < 16; i++) {
      const a = (i / 16) * Math.PI * 2 * 2.4;
      const r = Math.sqrt((i + 0.5) / 16);
      taps.push([Math.cos(a) * r, Math.sin(a) * r]);
    }
    // TSL var-node reassignment isn't modellable with @types/three r185 — runtime-typed.
    type FN = any;
    const fnode = (n: unknown): FN => n as FN;
    const pcss = Fn(({ depthTexture, shadowCoord }: { depthTexture: THREE.DepthTexture; shadowCoord: ReturnType<typeof vec3> }) => {
      const z = shadowCoord.z;
      const cmp = (off: [number, number], r: FN, depth: FN): FN =>
        fnode(texture(depthTexture, shadowCoord.xy.add(vec2(off[0], off[1]).mul(r))).compare(depth));
      const searchR = float(7 * texel);
      const deltas = [0.004, 0.02, 0.06];
      let occSum: FN = float(0);
      let distSum: FN = float(0);
      for (const dz of deltas) {
        let litK: FN = float(0);
        for (const off of taps.slice(0, 6)) litK = fnode(litK.add(cmp(off, searchR, fnode(z.sub(dz)))));
        const occ = float(1).sub(litK.mul(1 / 6));
        occSum = fnode(occSum.add(occ));
        distSum = fnode(distSum.add(occ.mul(dz)));
      }
      const blockerDist = distSum.div(occSum.max(0.0001));
      const penumbra = blockerDist.mul(260).mul(penumbraScale).clamp(0.6, 11);
      const filterR = fnode(penumbra.mul(float(texel)));
      let lit: FN = float(0);
      for (const off of taps) lit = fnode(lit.add(cmp(off, filterR, fnode(z))));
      return select(occSum.lessThan(0.02), float(1), lit.mul(1 / 16));
    });
    (sun.shadow as unknown as { filterNode?: unknown }).filterNode = pcss;
  }
  scene.add(sun);
  // the shadow-follow rig (defined after the GI probe below) moves sun.target
  // off the origin, so the target proxy must live in the scene graph.
  scene.add(sun.target);

  // Neutral at boot; applyCityEnvironment sets color/intensity (hemiCut is
  // already folded into the keyframe hemiIntensity values).
  const hemi = new THREE.HemisphereLight(0xffffff, 0xffffff, 1);
  scene.add(hemi);

  // The stylized hero hospital (buildHospital) is gone (T19): the real KSW
  // campus below is the building, the generated zone-ladder is its interior,
  // and buildPlaza/buildHelipad furnish the real entrance + roof. The city
  // plate (below) underlies the whole campus, so no separate hero lawn plate.

  // ── the real KSW campus (S3a, T15): reuses the city clay-massing pipeline
  // on the 26 baked zone==='ksw' buildings — walls with the TSL facade
  // shader, roofs, plinth/eave trim. Facade detail is always on (hero/near).
  // mainBuilding always comes from the FULL campus so zone decomposition keys
  // off the true largest footprint even when the shell mesh is suppressed.
  const { group: kswCampus, mainBuilding } = buildKswCampus(kswBuildings);
  scene.add(kswCampus);
  const setCutaway = kswCampus.userData.setCutaway as (u: { cutH: number; upperFade: number }) => void;

  // ── the generated zone-ladder interior (S3b, T17): the built interior is
  // ALWAYS present now (T18); the dollhouse cutaway drives its visibility —
  // hidden when the main building is closed, revealed when it opens. Footprints
  // are already in the local world frame, so it drops at the campus origin.
  const interior = buildInterior(interiorPlan);
  interior.visible = false; // closed at boot (overview) — the cutaway shows it
  scene.add(interior);

  // ── the real forecourt + rooftop helipad (T19) ──────────────────────────
  // Plaza: slab at the real main door, a path to the nearest real road, an
  // ambulance under a canopy at the emergency (door) zone edge, and 6 props.
  // The emergency zone is the door zone (Empfang+Notfall lead its ladder).
  const erZone: Zone =
    interiorZones.reduce<{ z: Zone; d: number } | null>((best, z) => {
      const d = Math.hypot(mainDoor.x - z.x, mainDoor.z - z.z);
      const inside =
        mainDoor.x >= z.x - z.w / 2 && mainDoor.x <= z.x + z.w / 2 && mainDoor.z >= z.z - z.d / 2 && mainDoor.z <= z.z + z.d / 2;
      const score = inside ? d - 1e6 : d;
      return best === null || score < best.d ? { z, d: score } : best;
    }, null)?.z ?? interiorZones[0];
  const plaza = buildPlaza(mainDoor, erZone, cityRoads);
  scene.add(plaza);
  // Helipad on the main building's largest high flat roof face; fades with the
  // cutaway upperFade (same as the roof) so it vanishes when the house opens.
  const { group: helipad, setFade: setHelipadFade } = buildHelipad(mainBuilding);
  scene.add(helipad);

  // Main-building bbox: the cutaway only engages when the camera target sits
  // over the main building (else the state is forced off so distant/other
  // buildings are never sliced). Derived from the same footprint the interior
  // and mainBuilding came from.
  const mbBounds = (() => {
    let minX = Infinity;
    let maxX = -Infinity;
    let minZ = Infinity;
    let maxZ = -Infinity;
    for (const [x, z] of mainBuilding.footprint) {
      minX = Math.min(minX, x);
      maxX = Math.max(maxX, x);
      minZ = Math.min(minZ, z);
      maxZ = Math.max(maxZ, z);
    }
    // a little slack so a target near the wall still counts as "inside"
    return { minX: minX - 6, maxX: maxX + 6, minZ: minZ - 6, maxZ: maxZ + 6 };
  })();
  const targetOverMain = (): boolean =>
    rig.target[0] >= mbBounds.minX &&
    rig.target[0] <= mbBounds.maxX &&
    rig.target[2] >= mbBounds.minZ &&
    rig.target[2] <= mbBounds.maxZ;

  // ── the real Winterthur city around it (swisstopo LoD2 + OSM, clay) ──────
  // The hero hospital keeps its own authored plate; the city sits on a bigger
  // slab 2 cm lower so the two never z-fight. Massing + roads render through
  // the existing clay builders, so the town reads as the same handmade model.
  const cityPlate = new THREE.Mesh(
    boxGeo(cityMeta.plate.w, kswScene.plateThickness, cityMeta.plate.d),
    clayMat(palette.lawn),
  );
  cityPlate.position.set(cityMeta.plate.cx, -kswScene.plateThickness / 2 - 0.02, cityMeta.plate.cz);
  cityPlate.receiveShadow = true;
  // The whole city lives under one named group — later tasks (LOD rings,
  // follow-mode shadows, the wandering GI probe) hang their objects here too.
  // Quality gate: the city STAYS inside the GI probe scene (scene.add(cityRoot)
  // below, not excluded from renderProbeFace) — every zoom point keeps
  // hero-grade GI. If the probe cadence turns out to be the frame-time cost,
  // the only allowed knob is kswGi.staticFaceInterval, never excluding the city.
  const cityRoot = new THREE.Group();
  cityRoot.name = 'cityRoot';
  cityRoot.add(cityPlate);
  cityRoot.add(buildCityMassing(cityBuildings));
  cityRoot.add(buildRoads(cityRoads, cityRails));
  // real OSM nature: parks/woods, the Eulach, and ~4k mapped trees (instanced).
  // The hero plate keeps its authored trees — city trees skip that rect.
  // Tree canopies default to no cast-shadow (nature.ts) — cheap far-field
  // trees don't need to punch holes in the sun's shadow map; the LOD ring
  // (Task 10) re-enables it for the near ring around the camera.
  cityRoot.add(
    buildNature(cityNature, {
      excludeRect: {
        x: interiorPlan.building.x,
        z: interiorPlan.building.z,
        w: interiorPlan.building.w,
        d: interiorPlan.building.d,
      },
    }),
  );
  cityRoot.add(buildWindows(cityBuildings));
  cityRoot.add(buildLamps(cityRoads));
  scene.add(cityRoot);

  // 3-ring semantic LOD (Task 10, spec §2c): detail follows the camera radius.
  // getObjectByName can legitimately miss (design-legal), so refs are
  // collected defensively and applyCityLod is null-tolerant.
  const cityWalls = cityRoot.getObjectByName('cityWalls');
  const lodRefs: CityLodRefs = {
    setFacadeDetail: (on: boolean): void => {
      (cityWalls?.userData.setFacadeDetail as ((v: boolean) => void) | undefined)?.(on);
    },
    lamps: cityRoot.getObjectByName('cityLamps') ?? null,
    footways: cityRoot.getObjectByName('footwayRibbons') ?? null,
    treesFull: ['treeCanopies', 'treeConifers']
      .map((n) => cityRoot.getObjectByName(n))
      .filter((o): o is THREE.Object3D => o !== undefined),
    treeImpostors: ['treeImpostors', 'treeImpostorsConifer']
      .map((n) => cityRoot.getObjectByName(n))
      .filter((o): o is THREE.Object3D => o !== undefined),
    setTreeShadows: (on: boolean) => {
      const canopies = cityRoot.getObjectByName('treeCanopies');
      const conifers = cityRoot.getObjectByName('treeConifers');
      if (canopies) canopies.castShadow = on;
      if (conifers) conifers.castShadow = on;
    },
  };
  let cityRing = cityLodState(rig.radius, 'far');
  applyCityLod(cityRing, lodRefs);

  // collect animated bits: ambulance light pulses (plaza), helicopter rotor
  // idles (helipad). Tag contract shared with staticBatch.isAnimated via
  // ANIMATED_TAGS — now driven off the real plaza + rooftop props (T19).
  const animated: Record<(typeof ANIMATED_TAGS)[number], THREE.Object3D[]> = { blink: [], rotor: [] };
  for (const root of [plaza, helipad]) {
    root.traverse((o) => {
      for (const tag of ANIMATED_TAGS) if (o.userData[tag]) animated[tag].push(o);
    });
  }
  const blinkers = animated.blink as THREE.Mesh[];
  const rotors = animated.rotor;

  // ── everyone is an agent: dwell -> pick a destination -> walk the nav
  // graph (room -> door -> corridor -> target) -> dwell. Deterministic.
  // Rendering is per-role instanced (agentMeshes.ts): the shader animates
  // squash/waddle/yaw from storage buffers, the CPU keeps only the agent
  // state machine plus flat smoothing slots (eased y, lerped yaw, roll).
  // The crowd lives in the GENERATED zone-ladder interior (T19): nav is built
  // over the generated plan and spawn specs come from its rooms. `inBuilding`
  // tests membership in ANY zone rect (the real footprint is decomposed into
  // several), so agents standing inside get the raised-floor y-offset.
  const nav = buildNav(interiorPlan);
  const inBuilding = (x: number, z: number): boolean =>
    interiorZones.some((zn) => Math.abs(x - zn.x) < zn.w / 2 && Math.abs(z - zn.z) < zn.d / 2);
  // The generated plan's room people first, then seeded extras up to ?agents=N.
  const spawnSpecs = buildSpawnSpecs(interiorPlan, nav, agentTarget);
  // Crowd mode (Slice D): GPU LOD/cull classification + blob shadows instead
  // of real casters. At or below the threshold the authored look is untouched.
  const crowd = spawnSpecs.length > kswAgents.crowdThreshold;
  // Shadow caching (Slice E): in crowd mode the casters are static (agents use
  // blob shadows). But Task 4 puts the sun on the realtime clock — it creeps
  // every frame — so the shadow map must re-render each frame (a frozen map
  // would go stale as now() advances). Like the old cycle mode, realtime =
  // sun-moving = never cached; updateShadowFrustum(true) re-renders it per frame.
  const shadowCached = false;
  if (shadowCached) {
    sun.shadow.autoUpdate = false;
    sun.shadow.needsUpdate = true; // boot: render the map once, then freeze
    // Still-animated individual meshes (blinker visibility toggles, rotor
    // rotates every frame) must not cast into the FROZEN map: their boot
    // pose would burn in as a stale shadow. Blob shadows still ground them.
    for (const root of [...blinkers, ...rotors]) {
      root.traverse((o) => {
        if ((o as THREE.Mesh).isMesh) o.castShadow = false;
      });
    }
  }
  const roleCounts: Partial<Record<PersonRole, number>> = {};
  for (const s of spawnSpecs) roleCounts[s.spec.role] = (roleCounts[s.spec.role] ?? 0) + 1;
  const agentInstances = createAgentInstances(roleCounts, { crowd });
  // Agents live inside the interior group — hidden with it when the house is
  // closed (overview), revealed through the cutaway when it opens (er/ops). The
  // __KSW agent snapshot below is CPU-driven, so movement is reported even
  // while the meshes are hidden.
  for (const m of agentInstances.meshes) interior.add(m);
  type LiveAgent = { agent: Agent; slot: AgentSlot; idx: number; y: number; yaw: number; roll: number };
  const liveAgents: LiveAgent[] = [];
  for (const [idx, s] of spawnSpecs.entries()) {
    const agent = createAgent(s.spec);
    agent.yaw = s.yaw;
    const y = inBuilding(s.spec.home[0], s.spec.home[1]) ? 0.14 : 0;
    const slot = agentInstances.add(s.spec.role, idx);
    slot.set(agent.pos[0], agent.pos[1], y, s.yaw, false, 0);
    liveAgents.push({ agent, slot, idx, y, yaw: s.yaw, roll: 0 });
  }
  agentInstances.update(0);

  // Edge mist ring around the plate rim. Base opacity/color are driven per-frame
  // by applyCityEnvironment (into mistBaseOpacity); animate() applies the
  // zoom-fade and city crossfade on top.
  const mistBaseOpacity = { value: lastEnv.mistOpacity };
  const mistMat = new THREE.MeshBasicMaterial({
    color: lastEnv.mistColor,
    transparent: true,
    opacity: lastEnv.mistOpacity,
    depthWrite: false,
  });
  // hug the real campus complex (the generated interior's bounding box) rather
  // than the old 72×56 hero plate.
  const rimX = interiorPlan.building.w / 2;
  const rimZ = interiorPlan.building.d / 2;
  const rimCx = interiorPlan.building.x;
  const rimCz = interiorPlan.building.z;
  // walk a rectangle perimeter (an ellipse would dip onto the lawn near the
  // corners) and hug it with small flattened puffs. Parametrized so both the
  // hero plate and the city plate rim share one recipe (spec §4: city mist).
  const addMistRing = (halfW: number, halfD: number, cx: number, cz: number, mat: THREE.MeshBasicMaterial): void => {
    const pad = 2.2;
    const hw = halfW + pad;
    const hd = halfD + pad;
    const per = 4 * (hw + hd);
    const N = 26;
    for (let i = 0; i < N; i++) {
      let t = (i / N) * per;
      let mx: number;
      let mz: number;
      if (t < 2 * hw) {
        mx = -hw + t;
        mz = -hd;
      } else if (t < 2 * hw + 2 * hd) {
        t -= 2 * hw;
        mx = hw;
        mz = -hd + t;
      } else if (t < 4 * hw + 2 * hd) {
        t -= 2 * hw + 2 * hd;
        mx = hw - t;
        mz = hd;
      } else {
        t -= 4 * hw + 2 * hd;
        mx = -hw;
        mz = hd - t;
      }
      const mist = new THREE.Mesh(new THREE.SphereGeometry(2.4 + (i % 3) * 0.5, 16, 16), mat);
      mist.position.set(mx + cx, 0.25, mz + cz);
      mist.scale.y = 0.22;
      scene.add(mist);
    }
  };
  addMistRing(rimX, rimZ, rimCx, rimCz, mistMat);

  // city mist rim: same puffs around the city plate, faded in with the clouds
  // (0 below radius 300, up to preset.mistOpacity*0.8 at the city framing).
  const cityMistMat = mistMat.clone();
  cityMistMat.opacity = 0;
  addMistRing(cityMeta.plate.w / 2, cityMeta.plate.d / 2, cityMeta.plate.cx, cityMeta.plate.cz, cityMistMat);

  // Night life: window glow + lamp bulbs are baked into the glowNight batch
  // at build time (staticBatch.ts) and ride the shared lampGlowU uniform; the
  // actual light pools live here. Rebased (T19) onto the real forecourt: two
  // warm pools flank the main door, one glows over the emergency zone. The
  // pools are ALWAYS created now; their intensity scales by glow01 (Task 3:
  // preset-driven; Task 4: continuous per-frame from lampLights/lampBaseIntensities).
  const lampLights: THREE.PointLight[] = [];
  const lampBaseIntensities: number[] = [];
  const glow01 = lastEnv.lampOn01;
  {
    const [dox, doz] = [Math.sin(mainDoor.yaw), Math.cos(mainDoor.yaw)];
    const perpX = Math.cos(mainDoor.yaw);
    const perpZ = -Math.sin(mainDoor.yaw);
    for (const side of [-1, 1]) {
      const px = mainDoor.x + dox * 6 + perpX * side * 6;
      const pz = mainDoor.z + doz * 6 + perpZ * side * 6;
      const base = nightGlow.cityPool * nightGlow.boost;
      const pool = new THREE.PointLight(nightGlow.bulb, base * glow01, 12, 2);
      pool.position.set(px, 3.0, pz);
      scene.add(pool);
      lampLights.push(pool);
      lampBaseIntensities.push(base);
    }
    // emergency-zone glow
    const emBase = nightGlow.emergency * nightGlow.boost;
    const emLamp = new THREE.PointLight(nightGlow.bulb, emBase * glow01, 16, 2);
    emLamp.position.set(erZone.x + dox * (erZone.d / 2), 2.6, erZone.z + doz * (erZone.d / 2));
    scene.add(emLamp);
    lampLights.push(emLamp);
    lampBaseIntensities.push(emBase);
  }
  // Task 3: presets still drive the glow — one shared uniform for the batched
  // glow geometry (windows + bulbs). Task 4 fades this continuously with sunset.
  lampGlowU.value = glow01;

  // One-bounce GI: capture from above the roofs, feed back as IBL. Boot does
  // the full synchronous 6-face warm-up (never a black env map); after that
  // the scheduler amortizes refreshes to at most ONE face per frame (Slice E)
  // — continuous walking in cycle mode; on the static presets a slow
  // background cadence (one face per kswGi.staticFaceInterval frames) plus
  // immediate dirty walks when the roof fade crosses the castShadow /
  // visibility thresholds or settles after a fade.
  const cubeRT = new THREE.CubeRenderTarget(kswGi.probeSize);
  const cubeCam = new THREE.CubeCamera(0.1, 400, cubeRT);
  cubeCam.position.set(0, kswScene.giProbeY, 0);
  scene.add(cubeCam);
  // Crowd mode: run one LOD classification BEFORE the warm-up. The flag
  // buffer boots zero-initialized (= everything LOD0) and the LOD0 pools
  // boot at count=0, so an unclassified warm-up would capture 10k blob
  // discs and zero bodies into the env cube.
  if (agentInstances.lod) {
    agentInstances.lod.frame(camera);
    await renderer.computeAsync(agentInstances.lod.node);
  }
  // Probe renders never use main-camera culling: LOD1 capsules + blob discs
  // everywhere, LOD0 hidden (no-op below the crowd threshold).
  agentInstances.setProbeMode(true);
  try {
    cubeCam.update(renderer as unknown as Parameters<typeof cubeCam.update>[0], scene);
  } finally {
    agentInstances.setProbeMode(false);
  }
  // The scheduler amortizes probe refreshes (≤1 face/frame). computeEnvironment
  // changes only slowly across a frame, so the static background cadence is
  // fine — no per-frame full re-walk needed for the realtime sun.
  const giScheduler = new GiProbeScheduler('static');
  scene.environment = cubeRT.texture;
  // giScale is written per-frame by applyCityEnvironment into giBase; animate()
  // computes scene.environmentIntensity = gi.environmentIntensity * giBase * envScaleScalar.
  const giBase = { value: lastEnv.giScale };
  scene.environmentIntensity = gi.environmentIntensity * giBase.value * kswPost.envScaleScalar;

  // ── camera-following sun shadows (spec §4: hero-grade light everywhere) ──
  // The shadow frustum walks with the camera target: centred on rig.target,
  // extent grown with radius (never below 46 → any street keeps hero texel
  // density), far scaled to keep the plate inside, throttled so orbiting
  // doesn't thrash the (cached) depth map. The old hero-plate origin-snap is
  // gone (T19): the real KSW campus spans ~320 m and is not at the origin, so
  // there is no small fixed plate to pin the frustum to.
  const onHeroPlate = (): boolean => false;
  let shadowExtentNow: number = kswScene.shadowExtent;
  let shadowTargetNow = new THREE.Vector3(0, 0, 0);
  const updateShadowFrustum = (force = false): void => {
    const hero = onHeroPlate() && rig.radius <= 120;
    const wantExtent = hero
      ? kswScene.shadowExtent
      : Math.max(kswScene.shadowExtent, Math.min(kswScene.shadowExtent + (rig.radius - 120) * 0.9, 900));
    const wantTarget = hero ? new THREE.Vector3(0, 0, 0) : new THREE.Vector3(...rig.target);
    const extentJump = Math.abs(wantExtent - shadowExtentNow) > shadowExtentNow * 0.1;
    const targetJump = wantTarget.distanceTo(shadowTargetNow) > 20;
    if (!force && !extentJump && !targetJump) return;
    shadowExtentNow = wantExtent;
    shadowTargetNow = wantTarget;
    sun.shadow.camera.left = -wantExtent;
    sun.shadow.camera.right = wantExtent;
    sun.shadow.camera.top = wantExtent;
    sun.shadow.camera.bottom = -wantExtent;
    sun.target.position.copy(wantTarget);
    // keep the PCSS penumbra's world-space size constant as the shadow
    // camera's meters-per-texel grows with extent; 1.0 at the hero extent.
    penumbraScale.value = kswScene.shadowExtent / wantExtent;
    // Hero restore must be value-identical to boot: exact far=220 and
    // sun.position = dir*sunDistance (NOT dir*(sunDistance+extent)) — any
    // drift here means a hero-plate return after visiting the city no
    // longer matches the pixel-identical boot frame the Hero-Guard promises.
    if (hero) {
      sun.shadow.camera.far = 220;
      sun.position.copy(currentSunDir).multiplyScalar(kswScene.sunDistance);
    } else {
      sun.shadow.camera.far = 220 + wantExtent * 2;
      sun.position.copy(wantTarget).addScaledVector(currentSunDir, kswScene.sunDistance + wantExtent);
    }
    sun.shadow.camera.updateProjectionMatrix();
    if (shadowCached) sun.shadow.needsUpdate = true;
  };

  // Roaming GI probe: same one-bounce machinery, anchor follows the camera
  // target so any zoomed-in street gets hero-grade env light. Snapped to a
  // 30 m grid so orbiting doesn't thrash the probe; every anchor move is a
  // markDirty() → amortized 6-face re-walk (1 face/frame, Slice-E scheduler).
  let probeAnchor = new THREE.Vector3(0, kswScene.giProbeY, 0);
  const updateProbeAnchor = (): void => {
    const roam = !(onHeroPlate() && rig.radius <= 120) && rig.radius <= 300;
    const want = roam
      ? new THREE.Vector3(Math.round(rig.target[0] / 30) * 30, kswScene.giProbeY, Math.round(rig.target[2] / 30) * 30)
      : new THREE.Vector3(0, kswScene.giProbeY, 0);
    if (want.equals(probeAnchor)) return;
    probeAnchor = want;
    cubeCam.position.copy(want);
    giScheduler.markDirty();
  };

  // ── post stack: TRAA -> GTAO -> godrays -> zoom-coupled DOF -> bloom ──
  const postProcessing = new THREE.PostProcessing(renderer);
  const scenePass = pass(scene, camera);
  scenePass.setMRT(mrt({ output, normal: normalView, velocity }));
  const scenePassColor = scenePass.getTextureNode('output');
  const scenePassNormal = scenePass.getTextureNode('normal');
  const scenePassDepth = scenePass.getTextureNode('depth');
  const chain = (n: unknown) => nodeObject(n as never) as unknown as typeof scenePassColor;
  const velocityTex = scenePass.getTextureNode('velocity');
  const beautyAA = chain(traa(scenePassColor, scenePassDepth, velocityTex, camera));
  const aoPass = ao(scenePassDepth, scenePassNormal, camera);
  const withAo = beautyAA.mul(aoPass.getTextureNode().x);
  const viewZ = scenePass.getViewZNode();
  // Godrays always built now; the mix is a live uniform (env.godraysMix, written
  // by applyCityEnvironment) scaled by the city's tuned veil constant.
  const godraysMixU = uniform(0);
  const raysNode = godrays(scenePassDepth, camera, sun);
  raysNode.density.value = post.godraysDensity;
  raysNode.maxDensity.value = post.godraysMaxDensity;
  const lit = withAo.add(chain(raysNode).mul(godraysMixU).mul(float(kswPost.godraysScale)));
  // Tilt-shift focus follows the dolly: focus distance = orbit radius.
  const focusU = uniform(rig.radius);
  const withDof = chain(dof(lit, viewZ, focusU, kswPost.dof.focalLength, kswPost.dof.bokehScale));
  const bloomPass = chain(bloom(withDof, post.bloom.strength, post.bloom.radius, kswPost.bloomThreshold));
  const composed = withDof.add(bloomPass);
  const lum = dot(composed.rgb, vec3(0.299, 0.587, 0.114));
  const tone = smoothstep(float(grade.low), float(grade.high), lum);
  const tint = mix(vec3(...grade.shadowTint), vec3(...grade.highlightTint), tone);
  const toned = composed.rgb.mul(tint);
  // Saturation + contrast are live uniforms (per-keyframe drama around mid-gray).
  const saturationU = uniform(1);
  const contrastU = uniform(1);
  const satLum = dot(toned, vec3(0.299, 0.587, 0.114));
  const saturated = mix(vec3(satLum, satLum, satLum), toned, saturationU);
  const contrasted = saturated.sub(float(0.5)).mul(contrastU).add(float(0.5)).clamp(0, 1);
  const graded = vec4(contrasted, composed.a);
  postProcessing.outputNode = film(graded, float(post.filmGrain));

  // Precipitation: GPU instanced rain/snow, driven by applyCityEnvironment.
  const precipitation = createPrecipitation(precipLook.city);
  scene.add(precipitation.object3d);

  // Weather: live Open-Meteo loop unless a ?wx override pins a fixed state.
  let weatherSeries: WeatherSeries | null = null;
  if (!wxParam) startWeatherLoop((s) => { weatherSeries = s; });
  const currentWeather = (): WeatherState =>
    WX_OVERRIDES[wxParam ?? ''] ?? (weatherSeries ? sampleWeather(weatherSeries, now()) : CLEAR_SKY);

  // The realtime environment target bundle. TSL uniform nodes expose a runtime
  // `.value` that @types/three r185 doesn't model on the node union, so the
  // uniform members are cast at this boundary only.
  type Holder<T> = { value: T };
  const envTargets: CityEnvironmentTargets = {
    renderer,
    fog: scene.fog as THREE.Fog,
    fogBase,
    sun,
    currentSunDir,
    hemi,
    skyMesh: skyMesh as unknown as CityEnvironmentTargets['skyMesh'],
    cloud: {
      sunDirUniform: sunDirUniform as unknown as Holder<THREE.Vector3>,
      lit: cloudLit as unknown as Holder<THREE.Color>,
      shadow: cloudShadow as unknown as Holder<THREE.Color>,
      coverageU: coverageU as unknown as Holder<number>,
      driftUV: driftUV as unknown as Holder<THREE.Vector2>,
    },
    post: {
      saturationU: saturationU as unknown as Holder<number>,
      contrastU: contrastU as unknown as Holder<number>,
      godraysMixU: godraysMixU as unknown as Holder<number>,
    },
    mist: { mat: mistMat, cityMat: cityMistMat, baseOpacity: mistBaseOpacity },
    sunDisc,
    moon: { mesh: moonDisc, phaseDir: moonPhaseDirU as unknown as Holder<THREE.Vector3> },
    stars: { object3d: starsObj, material: starsMat as THREE.MeshBasicMaterial },
    lampLights,
    lampBaseIntensities,
    precipitation,
    giBase,
    exposure: (v: number) => { renderer.toneMappingExposure = v; },
    scratch: { v3: new THREE.Vector3(), c1: new THREE.Color(), c2: new THREE.Color() },
    lampGlow: lampGlowU as unknown as Holder<number>,
  };

  // Perf probe: draw calls / triangles of the last rendered frame + EMA of
  // the main-thread frame cost (whole animate body / agent loop / render).
  const cpu = { frame: 0, agents: 0, render: 0 };
  const ema = (prev: number, sample: number): number => prev + (sample - prev) * 0.05;
  window.__KSW_INFO = () => ({
    drawCalls: renderer.info.render.drawCalls,
    triangles: renderer.info.render.triangles,
    cpu: { frame: cpu.frame, agents: cpu.agents, render: cpu.render },
  });

  // __KSW debug snapshot: camera scalars update every frame (the smoke keys
  // off them), but the agents block — sample copies over all agents — is
  // rebuilt in place only every 15 frames, with reused arrays (at 10k a
  // per-frame rebuild with fresh allocations shows up in the profile).
  const kswSnapshot: NonNullable<Window['__KSW']> = {
    radius: rig.radius,
    yaw: rig.yaw,
    pitch: rig.pitch,
    roofFade: roofFade(rig.radius, kswCamera),
    target: [rig.target[0], rig.target[1], rig.target[2]],
    agents: {
      total: liveAgents.length,
      walking: 0,
      samples: liveAgents.slice(0, 12).map((la) => [la.agent.pos[0], la.agent.pos[1]]),
    },
  };
  window.__KSW = kswSnapshot;
  const planBudget = { remaining: 0 };
  // Plan-budget fairness: rotate the iteration start each frame so the
  // budget isn't consumed by array position (see agents.advancePlanCursor).
  let planCursor = 0;

  // Roof-fade threshold tracking (Slice E, thresholds shared via
  // designTokens.roofFadePolicy): crossing the castShadow flip or the
  // visibility flip — see staticBatch.setFade — changes what the GI probe
  // and the shadow pass see. Additionally the probe refreshes when the fade
  // SETTLES (returns to fully-off or fully-opaque after having been in
  // between), so the resting state gets captured promptly — the slow
  // background cadence alone would leave a mid-fade capture lingering.
  let prevFade = roofFade(rig.radius, kswCamera);
  let fadeWasMid = prevFade > roofFadePolicy.visible && prevFade < roofFadePolicy.opaque;

  // ── Dollhouse cutaway (T18): computed each frame from the zoom radius, but
  // only when the camera target sits over the main building — otherwise forced
  // to the closed state (cutH 1e6, fade 1) so the other campus buildings and
  // the distant city are never sliced. Crossing the fade < 0.15 (slice engages)
  // and fade < 0.5 (interior becomes visible) thresholds is treated like a
  // roof-fade threshold crossing: the GI probe + (cached) shadow map refresh so
  // the newly-revealed interior lights correctly. The state below is applied
  // once at boot so the starting preset (overview closed / er open) is correct
  // on the very first frame.
  const closedCut = { cutH: 1e6, upperFade: 1 };
  const computeCut = (): { cutH: number; upperFade: number } =>
    targetOverMain() ? cutawayState(rig.radius) : closedCut;
  let cut = computeCut();
  setCutaway(cut);
  setHelipadFade(cut.upperFade); // helipad fades with the roof when the house opens
  interior.visible = cut.upperFade < 0.5;

  let frameCount = 0;
  let prevT = 0;
  const clock = new THREE.Clock();
  function animate(): void {
    const cpu0 = performance.now();
    const t = clock.getElapsedTime();
    const dt = Math.min(Math.max(t - prevT, 0), 0.1);
    prevT = t;
    if (Math.abs(zoomTarget - rig.radius) > 1e-4) {
      const k = 1 - Math.exp(-dt * kswCamera.zoomSmoothing);
      rig = { ...rig, radius: rig.radius + (zoomTarget - rig.radius) * k };
    }
    if (mouse && !dragging) {
      const [vx, vz] = edgePanVelocity(mouse.x, mouse.y, window.innerWidth, window.innerHeight, rig.yaw, kswCamera);
      if (vx !== 0 || vz !== 0) {
        // panning feels map-relative: slower when zoomed in close
        const zoomScale = Math.min(Math.max(rig.radius / 110, 0.15), 1);
        rig = applyPan(rig, vx * zoomScale, vz * zoomScale, dt, kswCamera);
      }
    }
    applyRig();
    // camera-following light: walk the shadow frustum + roaming GI anchor with
    // the camera target (both are hero-identical no-ops inside the Hero-Guard).
    updateShadowFrustum();
    updateProbeAnchor();
    const nextRing = cityLodState(rig.radius, cityRing);
    if (nextRing !== cityRing) {
      cityRing = nextRing;
      applyCityLod(cityRing, lodRefs);
      if (shadowCached) sun.shadow.needsUpdate = true;
    }
    const fogZoom = Math.max(1, rig.radius / 110);
    (scene.fog as THREE.Fog).near = fogBase.near * fogZoom;
    (scene.fog as THREE.Fog).far = fogBase.far * fogZoom;
    // roofFade is now purely a zoom-derived scalar (0 close .. 1 far): it drives
    // the edge-mist thinning + the __KSW.roofFade smoke signal + GI-refresh
    // thresholds. The campus roof itself opens via the dollhouse cutaway, not
    // this fade, so there is no roof batch to drive here anymore (T19).
    const fade = roofFade(rig.radius, kswCamera);
    const fadeIsMid = fade > roofFadePolicy.visible && fade < roofFadePolicy.opaque;
    if ((prevFade > roofFadePolicy.castShadow) !== (fade > roofFadePolicy.castShadow)) {
      // roof batch toggled castShadow: refresh GI + (cached) shadow map
      giScheduler.markDirty();
      if (shadowCached) sun.shadow.needsUpdate = true;
    } else if ((prevFade > roofFadePolicy.visible) !== (fade > roofFadePolicy.visible)) {
      // roof visibility flipped: the probe sees a different scene
      giScheduler.markDirty();
    } else if (fadeWasMid && !fadeIsMid) {
      // fade settled (fully off or fully opaque): capture the resting state
      giScheduler.markDirty();
    }
    fadeWasMid = fadeIsMid;
    prevFade = fade;

    // ── dollhouse cutaway (T18): drive the main-building slice + interior
    // reveal off the zoom radius. Crossing the fade < 0.15 (slice engages) or
    // fade < 0.5 (interior appears) thresholds refreshes GI + the cached shadow
    // map so the freshly-revealed ground floor lights correctly.
    const nextCut = computeCut();
    const wasSliced = cut.upperFade < 0.15;
    const nowSliced = nextCut.upperFade < 0.15;
    const wasOpen = cut.upperFade < 0.5;
    const nowOpen = nextCut.upperFade < 0.5;
    if (nextCut.cutH !== cut.cutH || nextCut.upperFade !== cut.upperFade) {
      setCutaway(nextCut);
      setHelipadFade(nextCut.upperFade);
    }
    if (nowOpen !== wasOpen) interior.visible = nowOpen;
    if (nowSliced !== wasSliced || nowOpen !== wasOpen) {
      // the sliced/open state flipped: the probe + shadow map see a different
      // scene (upper mass gone, interior revealed) — refresh both.
      giScheduler.markDirty();
      if (shadowCached) sun.shadow.needsUpdate = true;
    }
    cut = nextCut;

    focusU.value = rig.radius;
    // edge mist is a close-up treatment; from the overview it would read as
    // separate discs, so it thins out as the camera pulls back. Base opacity is
    // the per-frame environment value (mistBaseOpacity), fade/cloudMix on top.
    mistMat.opacity = mistBaseOpacity.value * (1 - fade * 0.75);
    // two-layer clouds + city mist crossfade on the 300→600 zoom-out ramp:
    // hero dome & hero mist rule up close, the city dome & city rim take over.
    const swap = kswCityStyle.cloudSwap;
    const cloudMix = Math.min(1, Math.max(0, (rig.radius - swap.start) / (swap.end - swap.start)));
    heroCloudOpacity.value = 1 - cloudMix;
    cityCloudOpacity.value = cloudMix;
    cityMistMat.opacity = mistBaseOpacity.value * 0.8 * cloudMix;
    kswSnapshot.radius = rig.radius;
    kswSnapshot.yaw = rig.yaw;
    kswSnapshot.pitch = rig.pitch;
    kswSnapshot.roofFade = fade;
    kswSnapshot.target[0] = rig.target[0];
    kswSnapshot.target[1] = rig.target[1];
    kswSnapshot.target[2] = rig.target[2];
    for (const b of blinkers) b.visible = Math.sin(t * 6) > -0.2;
    for (const r of rotors) r.rotation.y = t * 1.4;
    planBudget.remaining = kswAgents.planBudget;
    const cpuAgents0 = performance.now();
    let walking = 0;
    // every agent updates every frame — only the order rotates (F7 fairness)
    const agentCount = liveAgents.length;
    for (let k = 0; k < agentCount; k++) {
      const la = liveAgents[(planCursor + k) % agentCount];
      updateAgent(la.agent, dt, nav, planBudget);
      const targetY = inBuilding(la.agent.pos[0], la.agent.pos[1]) ? 0.14 : 0;
      la.y = approach(la.y, targetY, dt, 10);
      const isWalking = la.agent.phase === 'walk';
      if (isWalking) {
        walking += 1;
        la.yaw = lerpAngle(la.yaw, la.agent.yaw, Math.min(1, dt * 9));
        la.roll = Math.sin(t * 9 + la.idx) * 0.05;
      } else {
        la.roll *= Math.max(0, 1 - dt * 6);
      }
      la.slot.set(la.agent.pos[0], la.agent.pos[1], la.y, la.yaw, isWalking, la.roll);
    }
    planCursor = advancePlanCursor(planCursor, kswAgents.planBudget, agentCount);
    cpu.agents = ema(cpu.agents, performance.now() - cpuAgents0);
    if (frameCount % 15 === 0) {
      kswSnapshot.agents.walking = walking;
      const samples = kswSnapshot.agents.samples;
      for (let i = 0; i < samples.length; i++) {
        samples[i][0] = liveAgents[i].agent.pos[0];
        samples[i][1] = liveAgents[i].agent.pos[1];
      }
    }
    agentInstances.update(t);
    if (agentInstances.lod) {
      agentInstances.lod.frame(camera);
      renderer.compute(agentInstances.lod.node);
    }
    frameCount++;
    // ── realtime environment: physical sun/moon/stars for now(), steered by
    // live (or ?wx-pinned) weather. applyCityEnvironment writes every look
    // uniform (incl. cloud drift integration) and the shared currentSunDir.
    lastEnv = computeEnvironment(now(), currentWeather());
    applyCityEnvironment(envTargets, lastEnv, dt);
    // The follow rig placed sun.position from currentSunDir earlier this frame;
    // apply just moved currentSunDir, so re-sync position/target/far. Cheap: the
    // extent/target math is trivial. In the cached-shadow crowd path this also
    // marks the depth map dirty (updateShadowFrustum forces needsUpdate), so the
    // moving sun keeps re-rendering shadows — correct, at a shadow-pass cost.
    updateShadowFrustum(true);
    scene.environmentIntensity = gi.environmentIntensity * giBase.value * kswPost.envScaleScalar;
    window.__ENV_STATE = lastEnv;
    // Amortized GI probe: at most ONE cube face per frame (was: whole scene
    // 6x in one frame every 240 frames). PMREM rebuild once per full cube.
    // Probe faces render without main-camera culling (setProbeMode).
    const probe = giScheduler.next();
    if (probe) {
      agentInstances.setProbeMode(true);
      try {
        renderProbeFace(renderer, cubeCam, scene, probe.face);
      } finally {
        agentInstances.setProbeMode(false);
      }
      if (probe.cubeComplete) cubeRT.texture.needsPMREMUpdate = true;
    }
    const cpuRender0 = performance.now();
    postProcessing.render();
    const cpuEnd = performance.now();
    cpu.render = ema(cpu.render, cpuEnd - cpuRender0);
    cpu.frame = ema(cpu.frame, cpuEnd - cpu0);
    if (!window.__LOOK_READY) window.__LOOK_READY = true;
  }
  renderer.setAnimationLoop(animate);

  window.addEventListener('resize', () => {
    camera.aspect = window.innerWidth / window.innerHeight;
    camera.updateProjectionMatrix();
    renderer.setSize(window.innerWidth, window.innerHeight);
  });
}

void boot();
