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
  kswPeel,
  kswPost,
  kswScene,
  nightGlow,
  nightSkyLook,
  post,
  precipLook,
  roofFadePolicy,
} from '../designTokens';
import { computeEnvironment, type EnvironmentState } from '../environment/environment';
import { parseAtParam } from '../environment/atParam';
import { applyCityEnvironment, type CityEnvironmentTargets } from './applyCityEnvironment';
import { createPrecipitation } from '../environment/precipitation';
import { createStarField, createMoonDisc } from '../environment/nightSky';
import { CLEAR_SKY, sampleWeather, startWeatherLoop, type WeatherSeries, type WeatherState } from '../environment/weather';
import { applyDrag, applyPan, applyZoom, edgePanVelocity, keyboardPanVelocity, rigPosition, roofFade, type CameraRigState } from './cameraRig';
import { approach, createAgentInstances, lerpAngle, type AgentSlot } from './agentMeshes';
import { buildNav } from './nav';
import { buildSpawnSpecs } from './agentSpawn';
import { ANIMATED_TAGS } from './staticBatch';
import { advancePlanCursor, createAgent, updateAgent, type Agent } from './agents';
import { GiProbeScheduler, renderProbeFace } from './giProbe';
import { buildCityMassing } from './geo/cityMassing';
import { createHoverPicker } from './hoverPick';
import { createHoverCard } from './hoverCard';
import { getBuildingHoverInfo } from './geo/buildingAttributes';
import { buildKswCampus, largestBuilding, type CutawayUniforms } from './geo/kswCampus';
import { decomposeOriented, type Zone } from './interior/zones';
import { departmentCenter, generateBuildingPlan } from './interior/generatePlan';
import { buildBuildingInterior } from './interior/buildInterior';
import { buildPlaza, buildHelipad } from './interior/plaza';
import { peelState, closedPeel, type PeelCfg, type PeelState } from './interior/cutaway';
import { buildRoads } from './geo/roads';
import { makeCorridorGround } from './geo/groundSampler';
import { cityBuildings, cityMeta, cityNature, cityRails, cityRoads, kswBuildings } from './geo/geoData';
import { loadWorld, anchorGroundHeight, makeHeightSampler, fetchTileBin, decodeTileBin, type DecodedTile } from './geo/worldData';
import { buildL0Backdrop } from './geo/terrain';
import { updateCorridorDiscardAnchor } from './geo/corridorDiscardRegion';
import { loadCorridorMask } from './geo/corridorMask';
import { DEFAULT_RINGS, TileStreamer, tileCenter, type TileKey, type TileMeta } from './geo/tileStreamer';
import { materializeTile, subCellKey, type TileContent } from './geo/tileContent';
import type { TileRef, WorldTile } from '../../proto/world_pb.js';
import { buildWindows } from './geo/windows';
import { buildLamps } from './geo/lamps';
import { lampGlowU, snowU } from './glowUniform';
import { buildNature } from './geo/nature';
import { buildTreeLayer } from './geo/treeLayer';
import { bakeImpostorAtlas, buildImpostorMesh } from './geo/treeImpostors';
import { allArchetypes } from './geo/treeArchetypes';
import { windAmpU } from './windUniform';
import { applyCityLod, cityLodState, lampLodVisibility, type CityLodRefs } from './geo/lod';
import type { PersonRole } from './floorPlan';
import {
  TrafficClient,
  DEFAULT_TRAFFIC_WS,
  PROD_TRAFFIC_WS,
  buildDefaultCellGrid,
} from '../traffic/trafficClient';
import { createCarLayer } from '../traffic/carLayer';
import { createFlowLayer } from '../traffic/flowLayer';
import { poseAt } from '../traffic/deadReckon';
import { poseAtBlended } from '../traffic/laneBlend';
import { createLiveClient, DEFAULT_LIVE_WS, type LiveVitals } from '../live/liveClient';
import { createCitizensLayer } from '../live/citizensLayer';
import { createVitalsHud } from '../live/vitalsHud';
import { createAttributionFooter } from '../live/attribution';
import { ensureWebGpu } from '../webgpuGate';

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
      peel?: { p: number; storeyCount: number; storeyH: number };
    };
    __KSW_INFO?: () => {
      drawCalls: number;
      triangles: number;
      // main-thread cost per frame (EMA, ms): whole animate body, the agent
      // behavior+buffer-write loop, and the render call (command encoding)
      cpu: { frame: number; agents: number; render: number };
    };
    // Dev-only traffic debug surface, present ONLY under ?traffic=1 (Task 10
    // browser smoke). Exposes the live vehicle count + a dead-reckoned pose
    // sampler so the smoke can assert cars stream, move, and drive on the right
    // without instrumenting the WS at the app layer. `sample()` returns each
    // tracked vehicle's id, its lane id, and its current world pose (x, z, yaw)
    // — the SAME poseAt the car layer draws, so the numbers match the pixels.
    __traffic?: {
      count: () => number;
      serverTick: () => number;
      // Number of far-LOD flow impostor instances currently drawn (Task 13
      // smoke assertion (g)) — mirrors flowLayer's InstancedMesh.count after
      // its last update() call, i.e. exactly what's on screen this frame.
      flowCount: () => number;
      sample: () => Array<{
        id: number;
        lane: number;
        x: number;
        z: number;
        yaw: number;
        cls: number;
      }>;
      // Re-aim the CAMERA (and therefore the AOI subscription, which follows
      // rig.target) at a world (x, z) with an optional zoom radius + orbit
      // angles — lets the smoke/capture harness frame a dense corridor or a
      // named landmark deterministically, without synthetic mouse input. A near-
      // top-down pitch (~1.5 rad) gives the capture harness a clear read on which
      // side of the road each car is on.
      lookAt: (x: number, z: number, opts?: { radius?: number; yaw?: number; pitch?: number }) => void;
      // Instanced car-layer debug (Task 4 smoke): per-variant body counts, the
      // total drawn wheel count, and one wheel instance matrix as 16 JSON-safe
      // numbers (column-major) — lets the smoke assert bodies/glass/wheels draw
      // and that wheels roll+steer between frames.
      cars: () => number[];
      wheels: () => number;
      wheelMatrix: (i: number) => number[] | null;
    };
    // Dev-only live-channel debug surface, present ONLY under ?live=1 /
    // VITE_LIVE_WS (Task 15 browser smoke): tracked/drawn citizen counts and
    // the latest vitals as plain JSON-safe data.
    __live?: {
      citizenCount: () => number;
      instanceCount: () => number;
      vitals: () => {
        worldTick: number;
        sOfWorldDay: number;
        population: number;
        totalMoney: number;
        auditOk: boolean;
        tripsActive: number;
        prices: Array<{ marketId: number; goodId: number; ewmaPrice: number; marketName: string }>;
      } | null;
    };
    // Dev tree-layer debug surface (unconditional): the archetype count, the
    // live full-detail instance count (post-compaction), and the current
    // weather-coupled wind amplitude. The tree browser smoke asserts on these.
    __trees?: {
      archetypes: number;
      fullCount: () => number;
      // #141 smoke surface: registered tree-pool keys (per-sub-cell keys
      // `L1/x_y#i_j` + whole-tile L2 keys) and the total impostor instance
      // count over all pools — proves the mid-ring forest is registered.
      tileKeys: () => string[];
      impostorCount: () => number;
      windAmp: () => number;
      // Re-aim the camera at a world (x, z) with an optional zoom radius, then
      // force an immediate near-set recompaction — lets the Task 7 browser
      // smoke drive the compaction LOD deterministically (street vs.
      // establishing framing) without synthetic mouse input, mirroring
      // __traffic.lookAt above.
      lookAt: (x: number, z: number, radius?: number) => void;
    };
    // Dev tile-streaming debug surface (unconditional, Task 6/M3): live
    // streamed-tile count (L1+L2, excludes the boot-resident L0), the number
    // of tiles disposed since boot, and the permanently-failed fetch count.
    // liveKeys lists the materialized tile keys — the Task 7 fly-through
    // smoke asserts the live SET changes per leg (the raw count can stay
    // equal across regions with similar coverage while tiles churn).
    __stream?: {
      live: () => number;
      liveKeys: () => string[];
      disposed: () => number;
      failed: () => number;
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
  // WebGPU gate (Task 15): everything below assumes THREE.WebGPURenderer —
  // without navigator.gpu the app used to die into an opaque black screen.
  if (!ensureWebGpu()) return;
  const params = new URLSearchParams(window.location.search);
  // ?at= freezes the clock: full ISO instant, or HH:MM = today local time.
  const frozenAt = parseAtParam(params.get('at'));
  const wxParam = params.get('wx'); // 'clear'|'overcast'|'rain'|'snow'|'fog'|null
  const now = (): Date => frozenAt ?? new Date();
  const camRaw = params.get('cam');
  const cityCams: CamPresetName[] = ['er', 'ops', 'bahnhof', 'zag', 'city'];
  const camPreset: CamPresetName = cityCams.includes(camRaw as CamPresetName) ? (camRaw as CamPresetName) : 'overview';
  // ?agents=N scales the crowd (clamped; default = the authored plan people)
  const agentsRaw = Number.parseInt(params.get('agents') ?? '', 10);
  const agentTarget = Number.isNaN(agentsRaw) ? undefined : Math.min(Math.max(agentsRaw, 1), kswAgents.maxAgents);
  // Live instanced car layer (WS to the sim-server /traffic gateway). Cars must
  // ALWAYS appear on a deployed site, so on any non-localhost host traffic is
  // ON by default and points at the production backend — no URL param, no env
  // var required. On localhost it stays opt-in (?traffic=1 / VITE_TRAFFIC_WS)
  // so the dev server does not spam WS connection errors when no local backend
  // is running. Explicit ?traffic=0 force-disables; ?traffic=1 / VITE_TRAFFIC_WS
  // force-enable. Endpoint resolution: ?trafficWs= > VITE_TRAFFIC_WS env >
  // host-aware default (localhost gateway in dev, PROD_TRAFFIC_WS when deployed).
  const isLocalDev = ['localhost', '127.0.0.1', ''].includes(window.location.hostname);
  const envTrafficWs = (import.meta.env.VITE_TRAFFIC_WS as string | undefined) || undefined;
  const trafficDefaultWs = isLocalDev ? DEFAULT_TRAFFIC_WS : PROD_TRAFFIC_WS;
  const trafficEnabled =
    params.get('traffic') !== '0' &&
    (params.get('traffic') === '1' || envTrafficWs !== undefined || !isLocalDev);
  const trafficWsUrl = params.get('trafficWs') ?? envTrafficWs ?? trafficDefaultWs;
  // ?live=1 (or a configured VITE_LIVE_WS) enables the live world channel
  // (citizens AOI + vitals HUD, Task 15). URL override > env > default.
  const envLiveWs = (import.meta.env.VITE_LIVE_WS as string | undefined) || undefined;
  const liveEnabled = params.get('live') === '1' || envLiveWs !== undefined;
  const liveWsUrl = params.get('liveWs') ?? envLiveWs ?? DEFAULT_LIVE_WS;
  // Realtime environment: physical sun/moon/stars for now() steered by live (or
  // ?wx-pinned) weather. Re-evaluated every frame; this is the boot seed.
  let lastEnv: EnvironmentState = computeEnvironment(now(), WX_OVERRIDES[wxParam ?? ''] ?? CLEAR_SKY);

  // ── generated interior plan (Phase A): ONE source for the main building
  // (kswCampus.largestBuilding — the same call buildKswCampus makes), zones
  // decomposed in the footprint's dominant-wall-angle frame, one FloorPlan per
  // real storey (eaveH-derived). All plan geometry lives in the plan-local
  // frame; frame.toWorld/group.rotation.y map it back onto the world footprint.
  const mainBuildingFp = largestBuilding(kswBuildings);
  const { zones: interiorZones, frame: planFrame } = decomposeOriented(mainBuildingFp.footprint);
  const mainDoorWorld = mainBuildingFp.door ?? (() => {
    const [wx, wz] = planFrame.toWorld(interiorZones[0]?.x ?? 0, interiorZones[0]?.z ?? 0);
    return { x: wx, z: wz, yaw: 0 };
  })();
  const [doorLx, doorLz] = planFrame.toLocal(mainDoorWorld.x, mainDoorWorld.z);
  const mainDoor = { x: doorLx, z: doorLz, yaw: mainDoorWorld.yaw };
  const buildingPlan = generateBuildingPlan(interiorZones, mainDoor, mainBuildingFp.eaveH);
  const interiorPlan = buildingPlan.storeys[0]; // EG — nav/agents/plaza anchor (2D systems)
  // Re-aim er/ops onto the real department centers — departmentCenter returns
  // plan-local coords, transform to world for the camera targets.
  const [erLx, erLz] = departmentCenter(interiorPlan, 'Notfall');
  const [opLx, opLz] = departmentCenter(buildingPlan.storeys[Math.min(1, buildingPlan.storeyCount - 1)], 'OP');
  const [erX, erZ] = planFrame.toWorld(erLx, erLz);
  const [opX, opZ] = planFrame.toWorld(opLx, opLz);
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
  // Single city cloud layer (the hero dome was removed): the 1800 city dome is
  // the only cloud layer, always on. The old hero dome was an origin-centred
  // BackSide sphere of radius kswScene.domeRadius (400) that the camera dollied
  // out of past ~400, showing its far inner shell as a dark hemisphere over the
  // KSW. The city dome (1800 > radiusMax 1500) is never exited. The mist still
  // fades with zoom via cloudMix in animate().
  const cityCloudOpacity = uniform(1);

  // city cloud layer: big dome (kswCity.domeRadius), coarser noise (scale × 3).
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
  // Shadow-refresh threshold (Slice E + Task 4): the realtime sun creeps
  // continuously, so re-rendering the cached crowd shadow map on every
  // direction change would defeat the cache. Track the direction the map was
  // last rendered at and only refresh once the sun has drifted past this
  // angle. 0.002 rad =~ one refresh every ~27s of real sun motion
  // (~0.25deg/min creep) — sub-texel under PCSS, so no visible staleness.
  const SHADOW_SUN_REFRESH_RAD = 0.002;
  const lastShadowSunDir = currentSunDir.clone();

  // ── dynamic camera: wheel dolly + left-drag orbit ──────────────────────
  const start = camPresets[camPreset];
  let rig: CameraRigState = {
    yaw: start.yaw,
    pitch: start.pitch,
    radius: start.radius,
    target: start.target,
  };
  const camera = new THREE.PerspectiveCamera(kswCamera.fov, window.innerWidth / window.innerHeight, 0.1, kswCity.cameraFar);
  // layer 1 = terrainRoot (real DEM backdrop): visible to the main camera but
  // excluded from the GI probe's cube-face cameras (see terrainRoot below),
  // which stay on the default layer-0-only mask.
  camera.layers.enable(1);
  // zoom config: hero settings, but the dolly may pull back far enough to
  // frame the whole Bahnhof↔ZAG city (roof-fade still keyed off kswCamera)
  const zoomCfg = { ...kswCamera, radiusMax: kswCity.radiusMax };
  // pan uses the whole-Gemeinde roam bounds, not kswCamera's hero-room ±34/±26
  const panCfg = { ...kswCamera, panBoundsX: kswCity.panBoundsX, panBoundsZ: kswCity.panBoundsZ };
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
    if (e.button !== 2) return; // right button rotates (Cities-Skylines standard)
    dragging = true;
    renderer.domElement.setPointerCapture(e.pointerId);
  });
  // right-drag rotates → suppress the browser context menu over the canvas
  renderer.domElement.addEventListener('contextmenu', (e: Event) => e.preventDefault());
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

  // ── keyboard: WASD/arrows pan, Q/E rotate (Cities-Skylines standard) ──────
  const held = { up: false, down: false, left: false, right: false, rotL: false, rotR: false };
  const keyFlag = (code: string): keyof typeof held | null => {
    switch (code) {
      case 'KeyW':
      case 'ArrowUp':
        return 'up';
      case 'KeyS':
      case 'ArrowDown':
        return 'down';
      case 'KeyA':
      case 'ArrowLeft':
        return 'left';
      case 'KeyD':
      case 'ArrowRight':
        return 'right';
      case 'KeyQ':
        return 'rotL';
      case 'KeyE':
        return 'rotR';
      default:
        return null;
    }
  };
  const typingTarget = (): boolean => {
    const el = document.activeElement;
    return !!el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA');
  };
  window.addEventListener('keydown', (e: KeyboardEvent) => {
    if (typingTarget()) return;
    const f = keyFlag(e.code);
    if (!f) return;
    held[f] = true;
    if (e.code.startsWith('Arrow')) e.preventDefault(); // no page scroll
  });
  window.addEventListener('keyup', (e: KeyboardEvent) => {
    const f = keyFlag(e.code);
    if (f) held[f] = false;
  });

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
  const setCutaway = kswCampus.userData.setCutaway as (u: CutawayUniforms) => void;

  // ── the generated multi-storey interior (Phase A): per-storey groups whose
  // fades the peel drives every frame. Plan coords are frame-local; the group
  // rotation maps them onto the world footprint.
  const interiorCtl = buildBuildingInterior(buildingPlan, planFrame);
  const interior = interiorCtl.group;
  interior.visible = false; // closed at boot (overview) — the peel opens it
  scene.add(interior);

  // ── the real forecourt + rooftop helipad (T19) ──────────────────────────
  // Plaza: slab at the real main door, a path to the nearest real road, an
  // ambulance under a canopy at the emergency (door) zone edge, and 6 props.
  // The emergency zone is the door zone (Empfang+Notfall lead its ladder).
  const erZoneLocal: Zone =
    interiorZones.reduce<{ z: Zone; d: number } | null>((best, z) => {
      const d = Math.hypot(mainDoor.x - z.x, mainDoor.z - z.z);
      const inside =
        mainDoor.x >= z.x - z.w / 2 && mainDoor.x <= z.x + z.w / 2 && mainDoor.z >= z.z - z.d / 2 && mainDoor.z <= z.z + z.d / 2;
      const score = inside ? d - 1e6 : d;
      return best === null || score < best.d ? { z, d: score } : best;
    }, null)?.z ?? interiorZones[0];
  const [ezWx, ezWz] = planFrame.toWorld(erZoneLocal.x, erZoneLocal.z);
  const erZone: Zone = { ...erZoneLocal, x: ezWx, z: ezWz };
  const plaza = buildPlaza(mainDoorWorld, erZone, cityRoads);
  scene.add(plaza);
  // Helipad on the main building's largest high flat roof face; fades with the
  // cutaway roofFade (same as the roof) so it vanishes when the house opens.
  const { group: helipad, setFade: setHelipadFade } = buildHelipad(mainBuilding);
  scene.add(helipad);

  // Main-building world bbox (shared by the cutaway-active test below and the
  // heroRect nature/tree exclusion further down): interiorPlan/interiorZones
  // are plan-local (frame-rotated), so a rotated rect's local bbox is NOT its
  // world bbox — derive it straight from the world footprint instead.
  const fpB = (() => {
    let minX = Infinity, maxX = -Infinity, minZ = Infinity, maxZ = -Infinity;
    for (const [x, z] of mainBuildingFp.footprint) {
      if (x < minX) minX = x; if (x > maxX) maxX = x;
      if (z < minZ) minZ = z; if (z > maxZ) maxZ = z;
    }
    return { minX, maxX, minZ, maxZ };
  })();
  const mbBounds = (() => {
    // a little slack so a target near the wall still counts as "inside"
    return { minX: fpB.minX - 6, maxX: fpB.maxX + 6, minZ: fpB.minZ - 6, maxZ: fpB.maxZ + 6 };
  })();
  const targetOverMain = (): boolean =>
    rig.target[0] >= mbBounds.minX &&
    rig.target[0] <= mbBounds.maxX &&
    rig.target[2] >= mbBounds.minZ &&
    rig.target[2] <= mbBounds.maxZ;

  // ── real terrain under the whole diorama (Task 12 Slice 1 → M3 Task 6) ──
  // M3: boot no longer fetches the whole 77 MB pyramid. It loads the L0
  // overview tile (whole-municipality backdrop terrain) PLUS, synchronously,
  // the L2 tiles under the hero plate & the static city-nature footprint —
  // those fine tiles feed `groundYAt`, which the plate layers (roads, lamps,
  // tree layer, traffic) sample AT BUILD TIME. Without them the plate drape
  // would silently degrade to L0's 1000 m grid. Everything else streams in
  // camera-driven via TileStreamer below. Outside the sampler rect,
  // `groundYAt` falls back to L0 bilinear (far roads only — the streamed
  // terrain out there is L0/L1 anyway).
  //
  // Failure here is a hard boot error (no silent fallback to a flat plate):
  // if `/winterthur-world/` 404s, that's a dev-setup problem (missing the
  // `public/winterthur-world` symlink to `data/winterthur/world/`, see
  // worldData.ts) or a deploy problem, and should surface loudly rather than
  // quietly reverting to the old flat look.
  const samplerRect = (() => {
    const p = cityMeta.plate;
    let minX = p.cx - p.w / 2;
    let maxX = p.cx + p.w / 2;
    let minZ = p.cz - p.d / 2;
    let maxZ = p.cz + p.d / 2;
    // city nature (OSM trees/greens) extends past the plate; its trees are
    // y-positioned via groundYAt at build time, so cover them too.
    for (const t of cityNature.trees) {
      minX = Math.min(minX, t.x);
      maxX = Math.max(maxX, t.x);
      minZ = Math.min(minZ, t.z);
      maxZ = Math.max(maxZ, t.z);
    }
    const pad = 100;
    return { minX: minX - pad, maxX: maxX + pad, minZ: minZ - pad, maxZ: maxZ + pad };
  })();
  const world = await loadWorld(undefined, (ref, m) => {
    if (ref.level === 0) return true;
    if (ref.level !== 2) return false;
    // bake convention: 4x4 subdivision per level → L2 cell = size/16
    const cell = m.size / 16;
    const x0 = m.minX + ref.x * cell;
    const z0 = m.minZ + ref.y * cell;
    return (
      x0 < samplerRect.maxX && x0 + cell > samplerRect.minX && z0 < samplerRect.maxZ && z0 + cell > samplerRect.minZ
    );
  });
  // Corridor mask (Task 5e, spec §5 terrain-discard): the terrain shader
  // discards fragments inside road/rail corridors so rendered terrain can never
  // pierce a road surface; ribbon skirts close the hole. Hard boot error if
  // mask.bin is missing (no silent skip — a mask that fails to load must
  // surface loudly, same discipline as the world load above). The mask is
  // world-space and level-independent, so the L0 backdrop AND every streamed
  // tile (materializeTile ctx below) apply the exact same discard footprint.
  const corridorMask = await loadCorridorMask();
  const terrainRoot = new THREE.Group();
  terrainRoot.name = 'terrainRoot';
  // Only the L0 backdrop terrain is scene-resident from boot; the fine boot
  // L2 tiles above exist for the height sampler only — their region is
  // materialized (terrain + massing + trees) by the streamer right below.
  // The backdrop is split into 16x16 sub-meshes aligned to L2 tile regions:
  // the coarse L0 surface sits up to ~20 m ABOVE the fine terrain in parts of
  // the city (1000 m grid vs 12.5 m), so wherever a fine tile is live the
  // covering backdrop cell must be hidden or it veils the whole district.
  const l0Tile = world.tiles.find((t) => t.level === 0);
  if (!l0Tile) throw new Error('boot: manifest has no L0 tile');
  // #144: NO corridor discard on the coarse L0 backdrop — its surface deviates
  // up to ~20 m from the fine heights the road platform was built against, so
  // discarding it opens uncloseable corridor slots (see tileContent.ts).
  const backdrop = buildL0Backdrop(l0Tile, 16, {});
  terrainRoot.add(backdrop.group);
  // Tile heights are absolute DEM metres (~400-590 m); the hero city + KSW
  // sit at y≈0 (the anchor). Shift the whole terrain group down by the
  // anchor's ground height so the anchor point lines up with real y≈0 and
  // hills rise/fall around it from there. anchorGroundHeight prefers the
  // finest covering tile, and the boot L2 set covers the origin (the plate
  // rect contains it) — so the anchor value is identical to the pre-M3
  // load-everything boot.
  const anchorGround = anchorGroundHeight(world);
  terrainRoot.position.y = -anchorGround;
  // Ground-height sampler shared by the draped road ribbons AND the traffic
  // cars: heightAt returns absolute DEM metres, and terrainRoot is shifted by
  // -anchorGround, so the visible surface y (in the y=0 cityRoot frame) is
  // heightAt(x,z) - anchorGround. Roads drape onto it (else the flat ribbons
  // bury under / float over the undulating plate) and cars ride it per-vehicle.
  // Finest-first inside makeHeightSampler: L2 where the boot set covers,
  // L0 bilinear beyond it. `worldHeightAt` is a mutable binding: the streamer
  // below rebuilds it whenever fine tiles arrive/leave, so late consumers
  // (streamed tile trees, far traffic/flow drape) sample the SAME surface the
  // streamed terrain renders — the plate layers built right here still see
  // exact fine heights via the synchronous boot L2 set above.
  let worldHeightAt = makeHeightSampler(world);
  // `tileGroundYAt` closes over the MUTABLE `worldHeightAt` binding — the
  // corridor sampler below calls it per-query, so streamer rebuilds propagate
  // through automatically without re-running makeCorridorGround.
  const tileGroundYAt = (x: number, z: number): number => worldHeightAt(x, z) - anchorGround;
  // Task 5c (spec §5 amendment): roads own their surface height. The tile
  // heightfield samples ~12.5 m and cannot represent a ~6 m carriageway
  // bench, so wrap it with a corridor-aware sampler that returns the baked
  // per-way longitudinal profile height inside every road/rail corridor
  // (blended to tileGroundYAt at the corridor edge). Every existing
  // consumer below (buildRoads, carLayer, flowLayer, citizensLayer, lamp
  // placement) picks this up automatically since they all take the same
  // `groundYAt` function — see task-5c-report.md for the consumer list.
  const groundYAt = makeCorridorGround(cityRoads, cityRails, tileGroundYAt);

  // ── hospital ground elevation (bugfix, task 7) ───────────────────────────
  // The KSW shell (kswCampus), its generated interior, and the plaza are all
  // authored with a FLAT base at y=0 (the bake anchors buildings at the world
  // origin). But the terrain under the footprint is real DEM: after the
  // -anchorGround shift the surface here sits at ~+3.4 m (mean ~+5 m over the
  // footprint), so a y=0 building is BURIED — the EG storey (y 0..3) and the
  // shell base vanish under the grass, and only the upper storeys poke out.
  // Lift the whole hospital island onto its local ground so the EG sits on the
  // surface. One flat base can't follow the sloped DEM exactly, so use the
  // mean over the footprint vertices: it clears the terrain across the framed
  // (centre) region while keeping the float on the high edge minimal. The peel
  // cut bands (positionWorld.y in the shell shader) key off this same baseY, so
  // interior slabs and shell slice heights stay in lockstep.
  //
  // Sample the RAW tile terrain (tileGroundYAt), NOT the corridor-aware
  // groundYAt: the hospital footprint is a building plaza, and if a road/rail
  // corridor grazes a footprint vertex, groundYAt there returns the road-bench
  // profile height — which would bias the island off the visible grass surface.
  // tileGroundYAt is the same finest-first DEM surface the terrain renders, so
  // the mean lands the EG floor on the ground you actually see. (#139/#140:
  // makeCorridorGround now wraps tileGroundYAt; pre-merge groundYAt WAS the raw
  // tile sampler, so tileGroundYAt preserves the original placement semantics.)
  const hospitalBaseY = (() => {
    let sum = 0;
    for (const [vx, vz] of mainBuildingFp.footprint) sum += tileGroundYAt(vx, vz);
    return sum / mainBuildingFp.footprint.length;
  })();
  kswCampus.position.y = hospitalBaseY;
  interior.position.y = hospitalBaseY;
  plaza.position.y = hospitalBaseY;
  helipad.position.y = hospitalBaseY;

  // The er/ops orbit centers were authored against the y=0 ground plane —
  // lift them onto the DEM island so the camera pivots at the real EG floor.
  camPresets.er.target[1] += hospitalBaseY;
  camPresets.ops.target[1] += hospitalBaseY;

  // GI-probe exclusion: terrain is backdrop, not part of the hero-grade GI
  // capture. CubeCamera's 6 face cameras render with `cubeCam.layers`
  // (CubeCamera.js: `cameraPX.layers = this.layers`, shared across faces), so
  // putting terrainRoot on a dedicated layer that only the main `camera`
  // enables keeps it out of every renderProbeFace() call without touching
  // the city's own (unchanged, still-included) GI behavior below.
  terrainRoot.traverse((o) => o.layers.set(1));
  scene.add(terrainRoot);
  // Streamed tiles carry their own -anchorGround shift (materializeTile bakes
  // it into group.position.y), so they live under a separate unshifted root —
  // adding them to terrainRoot would double-shift them.
  const streamRoot = new THREE.Group();
  streamRoot.name = 'streamRoot';
  scene.add(streamRoot);

  // ── the real Winterthur city around it (swisstopo LoD2 + OSM, clay) ──────
  // The whole city lives under one named group — later tasks (LOD rings,
  // follow-mode shadows, the wandering GI probe) hang their objects here too.
  // Quality gate: the city STAYS inside the GI probe scene (scene.add(cityRoot)
  // below, not excluded from renderProbeFace) — every zoom point keeps
  // hero-grade GI. If the probe cadence turns out to be the frame-time cost,
  // the only allowed knob is kswGi.staticFaceInterval, never excluding the city.
  const cityRoot = new THREE.Group();
  cityRoot.name = 'cityRoot';
  cityRoot.add(buildCityMassing(cityBuildings));
  cityRoot.add(buildRoads(cityRoads, cityRails, groundYAt, tileGroundYAt));
  // real OSM nature: parks/woods, the Eulach, and ~4k mapped trees (instanced).
  // The hero plate keeps its authored trees — city trees skip that rect.
  // Tree canopies default to no cast-shadow (nature.ts) — cheap far-field
  // trees don't need to punch holes in the sun's shadow map; the LOD ring
  // (Task 10) re-enables it for the near ring around the camera.
  const heroRect = {
    x: (fpB.minX + fpB.maxX) / 2,
    z: (fpB.minZ + fpB.maxZ) / 2,
    w: fpB.maxX - fpB.minX,
    d: fpB.maxZ - fpB.minZ,
  };
  cityRoot.add(buildNature(cityNature, { excludeRect: heroRect }));
  // Trees: the archetype tree layer (instanced full-detail near-set + octahedral
  // impostor field). Same excludeRect as nature so the hero plate keeps its own
  // authored trees. The impostor mesh is added after the renderer bakes its
  // atlas (below); full trees + compaction are ready immediately.
  const treeLayer = buildTreeLayer(cityNature.trees, { excludeRect: heroRect, groundYAt });
  cityRoot.add(treeLayer.group);
  // Bake the octahedral impostor atlas now that the renderer exists (it
  // restores renderer state), then attach the far-field impostor mesh.
  const treeAtlas = await bakeImpostorAtlas(renderer, allArchetypes());
  treeLayer.group.add(buildImpostorMesh(treeLayer.instances, treeAtlas, allArchetypes().length));
  // Task 5 (M3): per-tile pools build their own impostor meshes off the same
  // atlas — hand the layer the shared texture once, right after the bake.
  treeLayer.setImpostorContext(treeAtlas, allArchetypes().length);
  // Dev scene + camera + uniform handles — visual-polish smokes toggle layers
  // and inspect shared uniforms through them.
  (window as unknown as { __SCENE?: THREE.Scene }).__SCENE = scene;
  (window as unknown as { __CAM?: THREE.Camera }).__CAM = camera;
  (window as unknown as { __UNIFORMS?: object }).__UNIFORMS = { lampGlowU, snowU };
  // Dev tree-layer debug surface (unconditional) — the browser smoke reads it.
  window.__trees = {
    archetypes: allArchetypes().length,
    fullCount: () => treeLayer.fullMeshes.reduce((s, m) => s + m.count, 0),
    // #141 smoke surface: registered tile-pool keys (subcell keys L1/x_y#i_j)
    // and the total far-field impostor instance count across all pools.
    tileKeys: () => treeLayer.tileKeys(),
    impostorCount: () =>
      treeLayer.group.children
        .filter((c): c is THREE.InstancedMesh => c.name.startsWith('treeImpostors'))
        .reduce((s, m) => s + m.count, 0),
    windAmp: () => windAmpU.value as number,
    lookAt: (x: number, z: number, radius?: number) => {
      rig = {
        ...rig,
        target: [x, groundYAt(x, z), z],
        radius: radius ?? rig.radius,
      };
      if (radius !== undefined) zoomTarget = radius;
      applyRig();
      treeLayer.compactNear(camera.position.x, camera.position.z);
    },
  };
  cityRoot.add(buildWindows(cityBuildings));
  cityRoot.add(buildLamps(cityRoads, groundYAt));
  scene.add(cityRoot);

  // ── live traffic (Task 9, ?traffic=1): instanced cars dead-reckoned from the
  // winterthur-traffic gateway. The client fetches trafficnet.json, opens the
  // WS, and manages camera-driven AOI cell subscriptions; the car layer draws
  // its dead-reckoned vehicle table each frame. Both are null until the async
  // connect resolves, so the animate loop guards on `trafficClient`.
  let trafficClient: TrafficClient | null = null;
  // Per-vehicle ground height so cars sit ON the draped road everywhere on the
  // traffic plate (the DEM undulates ~±10 m across the net) — same sampler the
  // road ribbons drape onto, so cars ride exactly the visible surface.
  const carLayer = trafficEnabled ? createCarLayer(groundYAt) : null;
  if (carLayer) cityRoot.add(carLayer.object3d);
  // Far-LOD impostor flow layer (Task 12): built once the client's net + grid
  // are available (they come from the same connect() resolution as the car
  // layer's dead-reckoning source). Renders ONLY outside the subscribed AOI
  // (minus the one-CELL_SIZE_M fade ring) — see flowLayer.ts's module banner.
  let flowLayer: ReturnType<typeof createFlowLayer> | null = null;
  let lastTrafficCamUpdate = 0; // wall-clock seconds, throttled to ~2 Hz
  // Tree near-set compaction: throttled like the traffic AOI update. Recompact
  // when the camera moved > 5 m (planar) since the last compaction, or ≥ 500 ms
  // passed and it moved at all. Uses the camera's world position, not rig.target.
  treeLayer.compactNear(camera.position.x, camera.position.z);
  let lastTreeCompaction = 0; // wall-clock seconds
  let lastTreeCamX = camera.position.x;
  let lastTreeCamZ = camera.position.z;

  // ── M3 tile streaming (Task 6): camera-driven L1/L2 pyramid ─────────────
  // Constructed only HERE — after bakeImpostorAtlas + setImpostorContext +
  // the first compactNear above (Task-5 finding, binding): addTileTrees
  // throws without the impostor context, so no tile fetch may even start
  // before this point. L0 is deliberately NOT in the streamer's tile list —
  // it is boot-resident in terrainRoot and must never be fetched again or
  // unloaded.
  //
  // L1/L2 overlap rule ("pro Region gewinnt L2"): L1 and L2 tiles carry the
  // SAME buildings/trees (each feature is assigned once per level by the
  // bake), so an L1 tile whose region has any live L2 tile would double-render
  // them. The bake subdivides 4x4 per level (LEVEL_CELLS = [1, 4, 16] in
  // scripts/geo/lib/tiles.mjs — 16 L1 vs 256 L2 tiles), so L2/x_y lies under
  // L1/(x>>2)_(y>>2). #141: L1 tiles are materialized in 4×4 sub-cells exactly
  // congruent with their L2 children; a live L2 tile hides ONLY its one
  // sub-cell (terrain + massing meshes + that sub-cell's tree pool) and the
  // sub-cell is restored when the L2 tile unloads — hiding the whole 5-km
  // group killed the mid-ring forest/massing (scratch/streaming/forest.png).
  const plateRect = { x: cityMeta.plate.cx, z: cityMeta.plate.cz, w: cityMeta.plate.w, d: cityMeta.plate.d };
  // Tree-only exclusion, wider than plateRect: converts `samplerRect` (the
  // min/max rect already covering the boot nature-tree bbox, see its
  // definition above) into materializeTile's center/size rect shape. Fixes
  // the M3 Task-6 double-tree bug — nature.json trees extend past the plate,
  // and without this the streamed tile trees out there duplicated them.
  const treeExcludeRect = {
    x: (samplerRect.minX + samplerRect.maxX) / 2,
    z: (samplerRect.minZ + samplerRect.maxZ) / 2,
    w: samplerRect.maxX - samplerRect.minX,
    d: samplerRect.maxZ - samplerRect.minZ,
  };
  const worldBase = '/winterthur-world/';
  const refByKey = new Map<TileKey, TileRef>();
  const tileMetas: TileMeta[] = [];
  for (const ref of world.manifest.tiles) {
    if (ref.level === 0) continue;
    const key: TileKey = `L${ref.level}/${ref.x}_${ref.y}`;
    refByKey.set(key, ref);
    const [cx, cz] = tileCenter(world.manifest, ref);
    tileMetas.push({ key, level: ref.level, cx, cz });
  }
  const liveTileContent = new Map<TileKey, TileContent>();
  const registeredTreeKeys = new Set<string>();
  let disposedTileCount = 0;
  // Live streamed tiles (decoded) feed the mutable height sampler: rebuilt on
  // every arrival/departure so tile trees planted in onReady (and far
  // traffic/flow drape) ride the streamed fine terrain instead of the L0
  // fallback. Rebuild is a sort + closure over <100 tiles — trivial next to
  // the materialization it accompanies.
  const streamedDecoded = new Map<TileKey, DecodedTile>();
  const rebuildHeightSampler = (): void => {
    worldHeightAt = makeHeightSampler({ ...world, tiles: [...world.tiles, ...streamedDecoded.values()] });
  };
  const setTileTrees = (content: TileContent, on: boolean): void => {
    if (!content.treeKey) return;
    if (on && !registeredTreeKeys.has(content.treeKey)) {
      treeLayer.addTileTrees(content.treeKey, content.trees);
      registeredTreeKeys.add(content.treeKey);
    } else if (!on && registeredTreeKeys.has(content.treeKey)) {
      treeLayer.removeTileTrees(content.treeKey);
      registeredTreeKeys.delete(content.treeKey);
    }
  };
  // #141 per-sub-cell L1 hide: an L1 tile is materialized in 4×4 sub-cells
  // congruent with its L2 children (tileContent.l1SubCellOfL2), and a live L2
  // tile hides/restores exactly the ONE sub-cell it replaces — meshes via
  // `visible`, trees via the sub-cell's own pool key (add/removeTileTrees
  // with the held specs). Coverage is derived directly from liveTileContent
  // (exactly one L2 child per sub-cell), so no separate cover counter exists.
  const applyL1SubCell = (l1x: number, l1y: number, i: number, j: number): void => {
    const content = liveTileContent.get(`L1/${l1x}_${l1y}`);
    const sc = content?.subCells?.get(subCellKey(i, j));
    if (!sc) return;
    const covered = liveTileContent.has(`L2/${l1x * 4 + i}_${l1y * 4 + j}`);
    for (const m of sc.meshes) m.visible = !covered;
    if (!sc.treeKey) return;
    if (!covered && !registeredTreeKeys.has(sc.treeKey)) {
      treeLayer.addTileTrees(sc.treeKey, sc.trees);
      registeredTreeKeys.add(sc.treeKey);
    } else if (covered && registeredTreeKeys.has(sc.treeKey)) {
      treeLayer.removeTileTrees(sc.treeKey);
      registeredTreeKeys.delete(sc.treeKey);
    }
  };
  const applyAllL1SubCells = (l1x: number, l1y: number): void => {
    for (let j = 0; j < 4; j++) {
      for (let i = 0; i < 4; i++) applyL1SubCell(l1x, l1y, i, j);
    }
  };
  // Backdrop cell (L2-index space) hides when fine terrain renders above it:
  // its own L2 tile is live, or its L1 parent is live — a live L1 always
  // renders terrain in every sub-cell not covered by a live L2 (#141: only
  // the exact covered sub-cells are hidden, and those ARE covered by the L2's
  // fine terrain), so "L1 live" alone guarantees the cell is covered.
  const updateBackdropCell = (x: number, y: number): void => {
    const mesh = backdrop.meshes.get(`${x}_${y}`);
    if (!mesh) return;
    mesh.visible = !(liveTileContent.has(`L2/${x}_${y}`) || liveTileContent.has(`L1/${x >> 2}_${y >> 2}`));
  };
  // Any ready/unload in an L1 region can flip that L1's visibility, which
  // affects all 16 backdrop cells under it — refresh the whole region.
  const refreshBackdropRegion = (l1x: number, l1y: number): void => {
    for (let dy = 0; dy < 4; dy++) {
      for (let dx = 0; dx < 4; dx++) updateBackdropCell(l1x * 4 + dx, l1y * 4 + dy);
    }
  };
  const streamer = new TileStreamer({
    all: tileMetas,
    fetchTile: async (meta) => decodeTileBin(await fetchTileBin(worldBase, refByKey.get(meta.key)!.path)),
    onReady: (meta, tile) => {
      const ref = refByKey.get(meta.key)!;
      const dec: DecodedTile = { level: ref.level, x: ref.x, y: ref.y, tile: tile as WorldTile };
      // Sampler first: the tile trees registered below plant via groundYAt
      // and must see THIS tile's fine heights.
      streamedDecoded.set(meta.key, dec);
      rebuildHeightSampler();
      const content = materializeTile(
        dec,
        // L2: full massing + trees; L1: same — its trees render as impostors
        // in practice (the compaction near-set rarely reaches the mid ring)
        // and its massing fills the r1 ring. groundShiftY mirrors terrainRoot.
        // Buildings still exclude via plateRect only — buildings.json covers
        // just the plate, so widening the exclusion here would drop massing
        // the mid ring needs. Trees exclude via `samplerRect` instead: the
        // boot nature.json trees extend ~100 m+ past the plate (Task 6
        // finding — 2501/7350 outside plateRect, most coinciding with
        // streamed tile trees), and `samplerRect` is the exact rect that
        // already covers that boot nature-tree bbox (+100 m pad).
        // corridorMask: streamed terrain applies the SAME road/rail fragment
        // discard as the boot terrain (spec §5) — else roads sink into it.
        { plateRect, treeExcludeRect, groundShiftY: -anchorGround, buildings: true, trees: true, corridorMask },
      );
      // Same GI-probe exclusion as terrainRoot: streamed content is backdrop.
      content.group.traverse((o) => o.layers.set(1));
      streamRoot.add(content.group);
      liveTileContent.set(meta.key, content);
      if (ref.level === 2) {
        setTileTrees(content, true);
        // Hide exactly the parent L1 sub-cell this L2 tile replaces.
        applyL1SubCell(ref.x >> 2, ref.y >> 2, ref.x & 3, ref.y & 3);
        refreshBackdropRegion(ref.x >> 2, ref.y >> 2);
      } else {
        // L1: per-sub-cell visibility + tree registration follow the current
        // L2 cover (covered sub-cells stay hidden/unregistered from the start).
        applyAllL1SubCells(ref.x, ref.y);
        refreshBackdropRegion(ref.x, ref.y);
      }
    },
    onUnload: (key) => {
      const content = liveTileContent.get(key);
      if (!content) return;
      liveTileContent.delete(key);
      streamedDecoded.delete(key);
      rebuildHeightSampler();
      setTileTrees(content, false);
      // Sub-celled (L1) tiles register trees per sub-cell — unregister every
      // pool this tile still holds before its group leaves the scene.
      if (content.subCells) {
        for (const sc of content.subCells.values()) {
          if (sc.treeKey && registeredTreeKeys.has(sc.treeKey)) {
            treeLayer.removeTileTrees(sc.treeKey);
            registeredTreeKeys.delete(sc.treeKey);
          }
        }
      }
      streamRoot.remove(content.group);
      content.dispose();
      disposedTileCount++;
      const ref = refByKey.get(key)!;
      if (ref.level === 2) {
        // Restore the parent L1 sub-cell this L2 tile was covering (the
        // liveTileContent delete above already flipped `covered` to false):
        // meshes back to visible, tree pool re-registered from the held specs.
        applyL1SubCell(ref.x >> 2, ref.y >> 2, ref.x & 3, ref.y & 3);
        refreshBackdropRegion(ref.x >> 2, ref.y >> 2);
      } else {
        refreshBackdropRegion(ref.x, ref.y);
      }
    },
    onError: (meta, err) => {
      // eslint-disable-next-line no-console
      console.error(`[stream] tile ${meta.key} failed permanently:`, err);
    },
  });
  // Prime the initial near ring from the boot camera; animate re-runs this on
  // the compactNear throttle (~2 Hz) as the camera moves.
  streamer.update(camera.position.x, camera.position.z);
  window.__stream = {
    live: () => streamer.liveCount,
    liveKeys: () => [...liveTileContent.keys()],
    disposed: () => disposedTileCount,
    failed: () => streamer.failed.size,
  };
  if (trafficEnabled) {
    void TrafficClient.connect({ url: trafficWsUrl })
      .then((client) => {
        trafficClient = client;
        flowLayer = createFlowLayer(client.net, client.grid, groundYAt);
        cityRoot.add(flowLayer.object3d);
        // Prime the subscription immediately from the boot camera target.
        client.updateCamera(rig.target[0], rig.target[2]);
        // Dev-only debug surface for the Task 10 browser smoke: read the live
        // vehicle table + dead-reckon each vehicle to the newest server tick
        // (the SAME poseAt the car layer draws with). Gated behind ?traffic=1
        // by living inside this connect block.
        window.__traffic = {
          count: () => client.vehicles.size,
          serverTick: () => client.serverTick,
          flowCount: () => flowLayer?.count() ?? 0,
          sample: () => {
            const out: Array<{
              id: number;
              lane: number;
              x: number;
              z: number;
              yaw: number;
              cls: number;
            }> = [];
            for (const [id, veh] of client.vehicles) {
              const pose = poseAtBlended(client.net, veh, client.serverTick);
              out.push({ id, lane: veh.lane, x: pose.x, z: pose.z, yaw: pose.yaw, cls: veh.cls });
            }
            return out;
          },
          lookAt: (x: number, z: number, opts?: { radius?: number; yaw?: number; pitch?: number }) => {
            // Aim the target at the DRAPED ground height under (x, z), not the
            // boot cam's fixed y. Post-#119 the traffic plate sits ~3-10 m below
            // y=0, so keeping the old target y floated the aim point above the
            // road and pushed it out of the tight capture frames (Task 10).
            rig = {
              ...rig,
              target: [x, groundYAt(x, z), z],
              radius: opts?.radius ?? rig.radius,
              yaw: opts?.yaw ?? rig.yaw,
              pitch: opts?.pitch ?? rig.pitch,
            };
            if (opts?.radius !== undefined) zoomTarget = opts.radius;
            applyRig();
            client.updateCamera(x, z);
          },
          cars: () => carLayer?.debug.variantCounts() ?? [],
          wheels: () => carLayer?.debug.wheelCount() ?? 0,
          wheelMatrix: (i: number) => carLayer?.debug.wheelMatrix(i) ?? null,
        };
      })
      .catch((err) => {
        // eslint-disable-next-line no-console
        console.error('[traffic] connect failed:', err);
      });
  }

  // ── data attribution (Task 15): fixed bottom-right footer, authoritative
  // strings from the baked world manifest (bake-world.mjs), static fallback.
  document.body.appendChild(createAttributionFooter(world.manifest.attribution));

  // ── live world channel (Task 15, ?live=1 / VITE_LIVE_WS): instanced citizen
  // capsules dead-reckoned from 1 Hz CitizenCellFrames + the vitals HUD card.
  // AOI cells derive from the SAME CellGrid as the traffic channel (shared
  // trafficnet.json derivation — cellGrid.ts) and follow the camera target on
  // the same ~2 Hz throttle as the traffic subscription.
  const citizensLayer = liveEnabled ? createCitizensLayer(groundYAt) : null;
  let liveAoiFollow: ((x: number, z: number) => void) | null = null;
  let lastLiveCamUpdate = 0; // wall-clock seconds, throttled to ~2 Hz
  if (liveEnabled && citizensLayer) {
    cityRoot.add(citizensLayer.object3d);
    const hud = createVitalsHud();
    document.body.appendChild(hud.element);
    // Same 128 m grid derivation as the traffic channel (shared cellGrid.ts).
    const liveGrid = buildDefaultCellGrid();
    const client = createLiveClient({
      url: liveWsUrl,
      onVitals: (v: LiveVitals) => hud.update(v),
      onCitizens: (cell, citizens, departed, keyframe) =>
        citizensLayer.applyFrame(cell, citizens, departed, keyframe),
    });
    // Throttled AOI follow (called from animate): 3×3 cells around the camera
    // target — same footprint the traffic client subscribes. Cells that leave
    // the set are dropped from BOTH the client's and the layer's tables (their
    // departed frames stop after unsubscribe).
    let prevCells = new Set<number>();
    liveAoiFollow = (x: number, z: number): void => {
      const want = liveGrid.cellsAround(x, z, 1);
      client.updateAoi([...want]);
      const removed: number[] = [];
      for (const c of prevCells) if (!want.has(c)) removed.push(c);
      if (removed.length > 0) citizensLayer.dropCells(removed);
      prevCells = want;
    };
    // Debug/smoke surface (browser smoke, CLAUDE.md rule): live counts + the
    // last vitals as plain data.
    window.__live = {
      citizenCount: () => client.core.citizenCount,
      instanceCount: () => citizensLayer.count(),
      vitals: () => {
        const v = hud.lastVitals;
        if (!v) return null;
        return {
          worldTick: Number(v.worldTick),
          sOfWorldDay: v.sOfWorldDay,
          population: Number(v.population),
          totalMoney: Number(v.totalMoney),
          auditOk: v.auditOk,
          tripsActive: Number(v.tripsActive),
          prices: v.prices.map((p) => ({ marketId: p.marketId, goodId: p.goodId, ewmaPrice: Number(p.ewmaPrice), marketName: p.marketName })),
        };
      },
    };
  }

  // 3-ring semantic LOD (Task 10, spec §2c): detail follows the camera radius.
  // getObjectByName can legitimately miss (design-legal), so refs are
  // collected defensively and applyCityLod is null-tolerant.
  const cityWalls = cityRoot.getObjectByName('cityWalls');
  const lodRefs: CityLodRefs = {
    setFacadeDetail: (on: boolean): void => {
      (cityWalls?.userData.setFacadeDetail as ((v: boolean) => void) | undefined)?.(on);
    },
    footways: cityRoot.getObjectByName('footwayRibbons') ?? null,
    setTreeShadows: (on: boolean) => treeLayer.setTreeShadows(on),
  };
  // Lamp LOD (2026-07-07 flicker/clutter fix): the opaque hardware and the
  // additive glow cull at their own distances (lampLodVisibility), NOT the
  // facade ring — otherwise the far-visible window raster drags 17.9k
  // sub-pixel posts/bulbs into the establishing view.
  const lampHardware = cityRoot.getObjectByName('lampHardware') ?? null;
  const lampGlow = cityRoot.getObjectByName('lampGlow') ?? null;
  let lampVis = lampLodVisibility(rig.radius, { hardware: true, glow: true });
  const applyLampVis = (): void => {
    if (lampHardware) lampHardware.visible = lampVis.hardware;
    if (lampGlow) lampGlow.visible = lampVis.glow;
  };
  applyLampVis();

  // Building hover (Task 9): pick against the merged city walls+roofs meshes
  // (both carry the buildingIdx attribute from cityMassing's merge — see
  // hoverPick.ts). cityBuildings is the SAME array buildCityMassing merged
  // above, so buildingIdx lines up. KSW campus buildings aren't part of this
  // merge — hover only covers the city massing, by design (campus picking is
  // out of scope here).
  const cityRoofs = cityRoot.getObjectByName('cityRoofs');
  const hoverPicker =
    cityWalls instanceof THREE.Mesh && cityRoofs instanceof THREE.Mesh
      ? createHoverPicker({ camera, meshes: [cityWalls, cityRoofs], buildings: cityBuildings })
      : null;
  const hoverCard = createHoverCard();
  let hoverEvent: PointerEvent | null = null;
  renderer.domElement.addEventListener('pointermove', (e: PointerEvent) => {
    hoverEvent = e;
  });
  renderer.domElement.addEventListener('pointerleave', () => {
    hoverEvent = null;
    hoverCard.hide();
  });
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
  // Shadow caching (Slice E): in crowd mode every caster is static — agents
  // use blob shadows (castShadow=false, agentMeshes.ts) — so re-rendering the
  // shadow map every frame buys nothing (it was ~3.5k of ~3.8k draw calls).
  // Task 4 put the sun on the realtime clock (it creeps ~0.25deg/min), but a
  // threshold-refresh (SHADOW_SUN_REFRESH_RAD below, in animate()) keeps the
  // depth map fresh without re-rendering every frame, so the crowd cache is
  // still worth taking. The r185 node-based shadow system
  // (ShadowNode.updateBefore) honors the classic light.shadow.autoUpdate /
  // needsUpdate flags: autoUpdate=false freezes the cached depth map,
  // needsUpdate=true re-renders it exactly once.
  const shadowCached = crowd;
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
  const egStorey = interior.getObjectByName('storey-0') ?? interior;
  for (const m of agentInstances.meshes) egStorey.add(m);
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
  // hug the real campus complex rather than the old 72×56 hero plate. Derived
  // from the world footprint bbox (fpB, same source heroRect uses) — NOT from
  // interiorPlan.building, which is plan-local (frame-rotated) and would put
  // the ring off-axis in this world-space consumer.
  const rimX = (fpB.maxX - fpB.minX) / 2;
  const rimZ = (fpB.maxZ - fpB.minZ) / 2;
  const rimCx = (fpB.minX + fpB.maxX) / 2;
  const rimCz = (fpB.minZ + fpB.maxZ) / 2;
  // walk a rectangle perimeter (an ellipse would dip onto the lawn near the
  // corners) and hug it with small flattened puffs. Parametrized so both the
  // hero plate and the city plate rim share one recipe (spec §4: city mist).
  const addMistRing = (halfW: number, halfD: number, cx: number, cz: number, mat: THREE.MeshBasicMaterial, baseY: number): void => {
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
      mist.position.set(mx + cx, baseY + 0.25, mz + cz);
      mist.scale.y = 0.22;
      scene.add(mist);
    }
  };
  addMistRing(rimX, rimZ, rimCx, rimCz, mistMat, hospitalBaseY);

  // city mist rim: same puffs around the city plate, faded in with the clouds
  // (0 below radius 300, up to preset.mistOpacity*0.8 at the city framing).
  const cityMistMat = mistMat.clone();
  cityMistMat.opacity = 0;
  addMistRing(cityMeta.plate.w / 2, cityMeta.plate.d / 2, cityMeta.plate.cx, cityMeta.plate.cz, cityMistMat, 0);

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
    const [dox, doz] = [Math.sin(mainDoorWorld.yaw), Math.cos(mainDoorWorld.yaw)];
    const perpX = Math.cos(mainDoorWorld.yaw);
    const perpZ = -Math.sin(mainDoorWorld.yaw);
    for (const side of [-1, 1]) {
      const px = mainDoorWorld.x + dox * 6 + perpX * side * 6;
      const pz = mainDoorWorld.z + doz * 6 + perpZ * side * 6;
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
  // the scheduler amortizes refreshes to at most ONE face per frame (Slice E):
  // a slow background cadence (one face per kswGi.staticFaceInterval frames)
  // plus immediate dirty walks when the roof fade crosses the castShadow /
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
  const shadowTargetNow = new THREE.Vector3(0, 0, 0);
  // Scratch for the per-frame "wanted" target — computed fresh every call but
  // never retained past this function, so one shared instance is safe (avoids
  // an allocation per frame; see Task-4 review finding).
  const wantTargetScratch = new THREE.Vector3(0, 0, 0);
  const updateShadowFrustum = (force = false): void => {
    const hero = onHeroPlate() && rig.radius <= 120;
    const wantExtent = hero
      ? kswScene.shadowExtent
      : Math.max(kswScene.shadowExtent, Math.min(kswScene.shadowExtent + (rig.radius - 120) * 0.9, 900));
    const wantTarget = hero ? wantTargetScratch.set(0, 0, 0) : wantTargetScratch.set(...rig.target);
    const extentJump = Math.abs(wantExtent - shadowExtentNow) > shadowExtentNow * 0.1;
    const targetJump = wantTarget.distanceTo(shadowTargetNow) > 20;
    if (!force && !extentJump && !targetJump) return;
    shadowExtentNow = wantExtent;
    shadowTargetNow.copy(wantTarget);
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
  const precipitation = createPrecipitation({
    ...precipLook.city,
    rainSx: precipLook.rainCitySx,
    rainSy: precipLook.rainCitySy,
    snowSx: precipLook.snowCitySx,
    snowSy: precipLook.snowCitySy,
    rainAlpha: precipLook.rainCityAlpha,
  });
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
    peel: { p: 0, storeyCount: buildingPlan.storeyCount, storeyH: buildingPlan.storeyH },
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

  // ── Storey peel (Phase A): computed each frame from the zoom radius, but
  // only when the camera target sits over the main building — otherwise forced
  // to the fully-closed peel so the other campus buildings and the distant
  // city are never sliced. The state below is applied once at boot so the
  // starting preset (overview closed / er open) is correct on the very first
  // frame.
  const peelCfg: PeelCfg = {
    storeyCount: buildingPlan.storeyCount,
    storeyH: buildingPlan.storeyH,
    baseY: hospitalBaseY, // real local ground elevation (see hospitalBaseY above): the shell/interior island is lifted onto the DEM, so the cut bands (positionWorld.y) must start here too
    startR: kswPeel.startR,
    endR: kswPeel.endR,
  };
  const computePeel = (): PeelState => (targetOverMain() ? peelState(rig.radius, peelCfg) : closedPeel(peelCfg));
  let peel = computePeel();
  const applyPeel = (s: PeelState): void => {
    setCutaway({ discardAbove: s.discardAbove, bandLo: s.bandLo, bandFade: s.bandFade, roofFade: s.roofFade });
    setHelipadFade(s.roofFade);
    interior.visible = s.p > 0.02;
    interiorCtl.setStoreyFades(s.storeyFades);
  };
  applyPeel(peel);

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
    // pan is map-relative: speed scales with zoom radius so a zoomed-out view
    // traverses the whole city quickly and a close-up nudges precisely (Cities-
    // Skylines feel). No upper cap — over the 20 km world the overview (radius
    // ~1500) must cover ground fast; the 0.15 floor keeps interior pans gentle.
    // radius is fixed for the rest of the frame (pan moves the target, not the
    // radius), so both pan sources share one scale.
    const panZoomScale = Math.max(rig.radius / 110, 0.15);
    if (mouse && !dragging) {
      const [vx, vz] = edgePanVelocity(mouse.x, mouse.y, window.innerWidth, window.innerHeight, rig.yaw, panCfg);
      if (vx !== 0 || vz !== 0) {
        rig = applyPan(rig, vx * panZoomScale, vz * panZoomScale, dt, panCfg);
      }
    }
    // keyboard pan (WASD/arrows) — same map-relative zoom scaling as edge-pan
    const [kvx, kvz] = keyboardPanVelocity(held, rig.yaw, panCfg);
    if (kvx !== 0 || kvz !== 0) {
      rig = applyPan(rig, kvx * panZoomScale, kvz * panZoomScale, dt, panCfg);
    }
    // keyboard rotate (Q/E) via the shared drag path: convert a rad/s rate into
    // the equivalent horizontal drag-pixel delta (applyDrag: yaw -= dxPx*dragSpeed)
    if (held.rotL !== held.rotR) {
      const dir = held.rotL ? 1 : -1;
      const dxPx = (dir * kswCamera.keyRotateSpeed * dt) / kswCamera.dragSpeed;
      rig = applyDrag(rig, dxPx, 0, kswCamera);
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
    // Lamp LOD rides the continuous radius (own hysteresis), not the 3-ring
    // boundaries — a plain boolean-set when a role's visibility actually flips.
    const nextLampVis = lampLodVisibility(rig.radius, lampVis);
    if (nextLampVis.hardware !== lampVis.hardware || nextLampVis.glow !== lampVis.glow) {
      lampVis = nextLampVis;
      applyLampVis();
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

    // ── storey peel: drive shell dissolve + per-storey interior fades off the
    // zoom radius. GI + the cached shadow map refresh when the peel crosses a
    // storey boundary or settles (fully closed / fully open) — same policy as
    // the roof fade above.
    const nextPeel = computePeel();
    if (nextPeel.p !== peel.p) applyPeel(nextPeel);
    const stepChanged = Math.floor(nextPeel.p) !== Math.floor(peel.p);
    const settled = (nextPeel.p === 0 || nextPeel.p === peelCfg.storeyCount) && nextPeel.p !== peel.p;
    // also refresh when the interior visibility flips (same threshold as
    // applyPeel's interior.visible = s.p > 0.02): the probe/shadow pass sees a
    // different scene the instant the interior first appears/disappears, even
    // if that doesn't happen to land on a storey-boundary or settle crossing.
    const visWas = peel.p > 0.02;
    const visNow = nextPeel.p > 0.02;
    if (stepChanged || settled || visNow !== visWas) {
      giScheduler.markDirty();
      if (shadowCached) sun.shadow.needsUpdate = true;
    }
    peel = nextPeel;

    focusU.value = rig.radius;
    // edge mist is a close-up treatment; from the overview it would read as
    // separate discs, so it thins out as the camera pulls back. Base opacity is
    // the per-frame environment value (mistBaseOpacity), fade/cloudMix on top.
    mistMat.opacity = mistBaseOpacity.value * (1 - fade * 0.75);
    // city mist fades IN on the 300→600 zoom-out ramp (the city rim reads only
    // from the overview). The city cloud dome is always on now (hero dome gone).
    const swap = kswCityStyle.cloudSwap;
    const cloudMix = Math.min(1, Math.max(0, (rig.radius - swap.start) / (swap.end - swap.start)));
    cityMistMat.opacity = mistBaseOpacity.value * 0.8 * cloudMix;
    kswSnapshot.radius = rig.radius;
    kswSnapshot.yaw = rig.yaw;
    kswSnapshot.pitch = rig.pitch;
    kswSnapshot.roofFade = fade;
    kswSnapshot.target[0] = rig.target[0];
    kswSnapshot.target[1] = rig.target[1];
    kswSnapshot.target[2] = rig.target[2];
    kswSnapshot.peel!.p = peel.p;
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
    // ── live traffic (Task 9): throttle camera-driven subscriptions to ~2 Hz,
    // then dead-reckon + draw the cars every frame.
    if (trafficClient && carLayer) {
      if (t - lastTrafficCamUpdate > 0.5) {
        lastTrafficCamUpdate = t;
        trafficClient.updateCamera(rig.target[0], rig.target[2]);
      }
      carLayer.update(trafficClient.net, trafficClient.vehicles, trafficClient.serverTick);
      if (flowLayer) {
        flowLayer.update(trafficClient.flow, trafficClient.subscribedCells, t);
      }
    }
    // ── live citizens (Task 15): AOI follows the camera on the same ~2 Hz
    // throttle as the traffic subscription; dead-reckon + draw every frame.
    if (citizensLayer) {
      if (liveAoiFollow && t - lastLiveCamUpdate > 0.5) {
        lastLiveCamUpdate = t;
        liveAoiFollow(rig.target[0], rig.target[2]);
      }
      citizensLayer.update(t);
    }
    // ── tree near-set compaction: recompact when the camera moved > 5 m
    // (planar) since the last compaction, or ≥ 500 ms passed and it moved.
    {
      const cx = camera.position.x;
      const cz = camera.position.z;
      // #144 distance-limited corridor discard: anchored to the SAME camera
      // position the streamer rings use, radius safely INSIDE the fine (L2)
      // ring so the discard never reaches the L2/L1 seam (see terrain.ts).
      // Drives the terrain holes AND the skirt walls that close them.
      updateCorridorDiscardAnchor(cx, cz, DEFAULT_RINGS.r2 * 0.8);
      const dx = cx - lastTreeCamX;
      const dz = cz - lastTreeCamZ;
      const moved2 = dx * dx + dz * dz;
      if (moved2 > 25 || (t - lastTreeCompaction > 0.5 && moved2 > 1e-6)) {
        lastTreeCompaction = t;
        lastTreeCamX = cx;
        lastTreeCamZ = cz;
        treeLayer.compactNear(cx, cz);
        // M3 tile streaming rides the same camera-movement throttle (~2 Hz):
        // load/unload decisions only matter when the camera actually moved.
        streamer.update(cx, cz);
      }
    }
    frameCount++;
    // ── realtime environment: physical sun/moon/stars for now(), steered by
    // live (or ?wx-pinned) weather. applyCityEnvironment writes every look
    // uniform (incl. cloud drift integration) and the shared currentSunDir.
    lastEnv = computeEnvironment(now(), currentWeather());
    applyCityEnvironment(envTargets, lastEnv, dt);
    // The precipitation field is periodic (positions wrap via mod inside its
    // box), so re-centring the mesh on box-multiples of the camera target is
    // seamless — this keeps rain/snow visible at far ?cam targets (e.g.
    // bahnhof/zag) instead of only ever falling over the world origin.
    precipitation.object3d.position.set(
      Math.round(rig.target[0] / precipLook.city.boxX) * precipLook.city.boxX,
      0,
      Math.round(rig.target[2] / precipLook.city.boxZ) * precipLook.city.boxZ,
    );
    // The follow rig placed sun.position from currentSunDir earlier this frame;
    // apply just moved currentSunDir, so re-sync position/target/far. Cheap: the
    // extent/target math is trivial. The sun creeps continuously in realtime, so
    // re-rendering the (cached, crowd-mode) shadow map on every direction change
    // would defeat Slice E's cache; only force a refresh once the sun has drifted
    // past SHADOW_SUN_REFRESH_RAD since the map was last rendered. Camera moves
    // (extent/target jumps) still refresh via updateShadowFrustum()'s own
    // early-out path, unaffected by this threshold.
    if (lastShadowSunDir.angleTo(currentSunDir) > SHADOW_SUN_REFRESH_RAD) {
      lastShadowSunDir.copy(currentSunDir);
      updateShadowFrustum(true);
    } else {
      updateShadowFrustum();
    }
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
    // building hover: throttled to once per rendered frame (pointermove only
    // records the event above; consumed here, not per-event) — reuses the
    // right-drag `dragging` flag so hover doesn't fight the camera drag.
    if (hoverEvent) {
      const r = renderer.domElement.getBoundingClientRect();
      const ndcX = ((hoverEvent.clientX - r.left) / r.width) * 2 - 1;
      const ndcY = -(((hoverEvent.clientY - r.top) / r.height) * 2 - 1);
      const hit = dragging || !hoverPicker ? null : hoverPicker.pick(ndcX, ndcY);
      const info = hit ? getBuildingHoverInfo(hit.id) : undefined;
      if (info) hoverCard.show(info, hoverEvent.clientX, hoverEvent.clientY);
      else hoverCard.hide();
      hoverEvent = null;
    }

    const cpuRender0 = performance.now();
    postProcessing.render();
    const cpuEnd = performance.now();
    cpu.render = ema(cpu.render, cpuEnd - cpuRender0);
    cpu.frame = ema(cpu.frame, cpuEnd - cpu0);
    // READY once the first frame rendered AND the initial streaming near-ring
    // is fully materialized (streamer.update ran at boot; pendingCount === 0
    // means nothing is queued or in flight — permanently failed tiles leave
    // the queue too, so this always terminates). L0 terrain is boot-resident
    // before the first frame by construction.
    if (!window.__LOOK_READY && streamer.pendingCount === 0) window.__LOOK_READY = true;
  }
  renderer.setAnimationLoop(animate);

  window.addEventListener('resize', () => {
    camera.aspect = window.innerWidth / window.innerHeight;
    camera.updateProjectionMatrix();
    renderer.setSize(window.innerWidth, window.innerHeight);
  });
}

void boot();
