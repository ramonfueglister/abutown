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
  kswGi,
  kswPost,
  kswScene,
  lightPresets,
  moonLight,
  nightGlow,
  palette,
  post,
  roofFadePolicy,
  skyPhys,
  sunArcCfg,
} from '../designTokens';
import { applyDrag, applyPan, applyZoom, edgePanVelocity, rigFromLookAt, rigPosition, roofFade, type CameraRigState } from './cameraRig';
import { buildHospital } from './building';
import { kswPlan } from './floorPlan';
import { approach, createAgentInstances, lerpAngle, type AgentSlot } from './agentMeshes';
import { buildNav } from './nav';
import { buildSpawnSpecs } from './agentSpawn';
import { ANIMATED_TAGS } from './staticBatch';
import { advancePlanCursor, createAgent, updateAgent, type Agent } from './agents';
import { GiProbeScheduler, renderProbeFace } from './giProbe';
import { boxGeo } from './geometryCache';
import { clayMat } from './props';
import { buildCityMassing } from './geo/cityMassing';
import { buildRoads } from './geo/roads';
import { cityBuildings, cityMeta, cityNature, cityRails, cityRoads } from './geo/geoData';
import { buildWindows } from './geo/windows';
import { buildLamps } from './geo/lamps';
import { buildNature } from './geo/nature';
import { applyCityLod, cityLodState, type CityLodRefs } from './geo/lod';
import type { PersonRole } from './floorPlan';

declare global {
  interface Window {
    __LOOK_READY?: boolean;
    __LOOK_BACKEND?: string;
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
  overview: (() => {
    const s = rigFromLookAt(kswCamera.overviewPosition, kswCamera.target);
    return { target: kswCamera.target, radius: s.radius, yaw: s.yaw, pitch: s.pitch };
  })(),
  // zoomed into the emergency ward: radius below roofFadeNear, roofs gone
  er: { target: [-22.5, 0.4, 12], radius: 14, yaw: -0.5, pitch: 0.72 },
  // surgery block from above the open roof, south-east
  ops: { target: [-24, 0.2, -16], radius: 13, yaw: 0.45, pitch: 1.05 },
  // real city landmarks (local meters from the KSW anchor, see cityMeta):
  // pulled back and tilted down so the camera sits above the dense district
  bahnhof: { target: [cityMeta.landmarks.bahnhof[0], 2, cityMeta.landmarks.bahnhof[1]], radius: 280, yaw: -0.6, pitch: 1.02 },
  zag: { target: [cityMeta.landmarks.zagTurbinenstrasse[0], 2, cityMeta.landmarks.zagTurbinenstrasse[1]], radius: 280, yaw: 0.4, pitch: 1.02 },
  // establishing shot: the whole KSW↔Bahnhof↔ZAG span from high up
  city: { target: [cityMeta.landmarks.bahnhof[0] * 0.6, 2, cityMeta.landmarks.bahnhof[1] * 0.6], radius: 820, yaw: -0.5, pitch: 1.12 },
};

async function boot(): Promise<void> {
  const params = new URLSearchParams(window.location.search);
  const rawPreset = params.get('preset');
  const presetName = rawPreset === 'night' || rawPreset === 'dusk' ? rawPreset : 'morning';
  const camRaw = params.get('cam');
  const cityCams: CamPresetName[] = ['er', 'ops', 'bahnhof', 'zag', 'city'];
  const camPreset: CamPresetName = cityCams.includes(camRaw as CamPresetName) ? (camRaw as CamPresetName) : 'overview';
  const cycleMode = params.get('cycle') === '1';
  // ?agents=N scales the crowd (clamped; default = the authored plan people)
  const agentsRaw = Number.parseInt(params.get('agents') ?? '', 10);
  const agentTarget = Number.isNaN(agentsRaw) ? undefined : Math.min(Math.max(agentsRaw, 1), kswAgents.maxAgents);
  const preset = lightPresets[presetName];

  const renderer = new THREE.WebGPURenderer({ antialias: false });
  await renderer.init();
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  renderer.setSize(window.innerWidth, window.innerHeight);
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.PCFSoftShadowMap;
  renderer.toneMapping = THREE.AgXToneMapping;
  renderer.toneMappingExposure = preset.exposure;
  document.body.appendChild(renderer.domElement);
  window.__LOOK_BACKEND = (renderer.backend as { isWebGPUBackend?: boolean }).isWebGPUBackend
    ? 'webgpu'
    : 'webgl2';

  const scene = new THREE.Scene();
  // fog scales with the zoom radius in animate(): identical look at the
  // overview framing, no white-out when zooming far out
  const fogBaseNear = preset.fogNear * kswScene.fogScale;
  const fogBaseFar = preset.fogFar * kswScene.fogScale;
  scene.fog = new THREE.Fog(preset.fogColor, fogBaseNear, fogBaseFar);

  // Sun day-arc (shared recipe with the prototype)
  const sunDirFor = (t: number): THREE.Vector3 => {
    const elev = sunArcCfg.elevBase + sunArcCfg.elevMax * Math.sin(Math.PI * Math.min(Math.max(t, 0), 1));
    const az = sunArcCfg.azRise + (sunArcCfg.azSet - sunArcCfg.azRise) * t;
    return new THREE.Vector3(Math.cos(elev) * Math.cos(az), Math.sin(elev), Math.cos(elev) * Math.sin(az));
  };
  const sunLightFor = (dir: THREE.Vector3, boost: number): { color: THREE.Color; intensity: number } => {
    const elevN = Math.min(Math.max(dir.y / 0.8, 0), 1);
    const eased = elevN * elevN * (3 - 2 * elevN);
    return {
      color: new THREE.Color(sunArcCfg.colorLow).lerp(new THREE.Color(sunArcCfg.colorHigh), eased),
      intensity: (0.8 + 6.2 * eased) * boost,
    };
  };
  const phys = skyPhys[presetName];
  const initialSunDir = sunDirFor(phys.timeOfDay);

  const skyMesh = new SkyMesh();
  // sky is a directional shader — enlarging the shell only moves it beyond the
  // city plate (so distant buildings never poke through), the look is unchanged
  skyMesh.scale.setScalar(kswCity.skyScale);
  skyMesh.turbidity.value = phys.turbidity;
  skyMesh.rayleigh.value = phys.rayleigh;
  skyMesh.mieCoefficient.value = phys.mieCoefficient;
  skyMesh.mieDirectionalG.value = phys.mieG;
  skyMesh.sunPosition.value.copy(initialSunDir);
  // the sky sphere sits beyond fogFar and would be tinted flat by the fog;
  // per-preset choice — the morning sky is so bright that the fog tint is
  // actually the better look there
  (skyMesh.material as THREE.Material & { fog: boolean }).fog = !kswPost.skyUnfogged[presetName];
  scene.add(skyMesh);

  // Procedural cloud dome (fbm, sun-lit silver lining)
  const sunDirUniform = uniform(initialSunDir.clone());
  const cloudLit = uniform(new THREE.Color(0xffffff));
  const cloudShadow = uniform(new THREE.Color(0x9aa8b5));
  const driftU = uniform(0);
  const cloudMatDome = new THREE.MeshBasicNodeMaterial();
  cloudMatDome.transparent = true;
  cloudMatDome.side = THREE.BackSide;
  cloudMatDome.depthWrite = false;
  cloudMatDome.fog = false;
  {
    const dir = positionWorld.normalize();
    const p = vec3(dir.x.mul(float(cloudCfg.scale)).add(driftU), dir.y.mul(float(cloudCfg.scale * 1.6)), dir.z.mul(float(cloudCfg.scale)));
    const n = mx_fractal_noise_float(p, 4, 2.0, 0.55, 1.0);
    const coverage = float(cloudCfg.coverage[presetName]);
    const dens = smoothstep(float(0.06), float(0.34), n.add(coverage.sub(0.5)));
    const horizonFade = smoothstep(float(0.0), float(0.07), dir.y);
    cloudMatDome.opacityNode = dens.mul(horizonFade);
    const facing = dot(dir, sunDirUniform).mul(0.5).add(0.5);
    type Vec3Node = ReturnType<typeof vec3>;
    const shadowN = cloudShadow as unknown as Vec3Node;
    const litN = (cloudLit as unknown as Vec3Node).mul(float(cloudCfg.litBoost));
    cloudMatDome.colorNode = mix(shadowN, litN, facing.pow(2.0));
  }
  const cloudDome = new THREE.Mesh(new THREE.SphereGeometry(kswScene.domeRadius, 32, 24), cloudMatDome);
  scene.add(cloudDome);

  const discDist = kswScene.domeRadius * 0.82;
  const sunDisc = new THREE.Mesh(
    new THREE.SphereGeometry(5.2, 20, 20),
    new THREE.MeshBasicMaterial({ color: 0xfff0d5, fog: false }),
  );
  scene.add(sunDisc);
  const moonDisc = new THREE.Mesh(
    new THREE.SphereGeometry(3.4, 20, 20),
    new THREE.MeshBasicMaterial({ color: palette.star, fog: false }),
  );
  moonDisc.position.set(-14, 21, 26).normalize().multiplyScalar(discDist);
  moonDisc.visible = presetName === 'night';
  scene.add(moonDisc);

  const sun = new THREE.DirectionalLight(0xffffff, 1);
  const applySunState = (t: number): void => {
    const dir = sunDirFor(t);
    skyMesh.sunPosition.value.copy(dir);
    (sunDirUniform.value as THREE.Vector3).copy(dir);
    if (presetName !== 'night') {
      const lightState = sunLightFor(dir, phys.sunBoost);
      sun.position.copy(dir.clone().multiplyScalar(kswScene.sunDistance));
      sun.color.copy(lightState.color);
      sun.intensity = Math.max(lightState.intensity, 0.05);
      (cloudLit.value as THREE.Color).copy(lightState.color).lerp(new THREE.Color(0xffffff), 0.3);
      (cloudShadow.value as THREE.Color).copy(new THREE.Color(0x8795a3).lerp(lightState.color, 0.15));
    } else {
      (cloudLit.value as THREE.Color).set(0x9fb2cc);
      (cloudShadow.value as THREE.Color).set(0x39485c);
    }
    sunDisc.position.copy(dir.clone().multiplyScalar(discDist));
    sunDisc.visible = presetName !== 'night' && dir.y > 0.015;
    moonDisc.visible = presetName === 'night' || dir.y <= 0.015;
  };

  if (preset.showStars) {
    const starPositions: number[] = [];
    let seed = 42;
    const rand = () => {
      seed = (seed * 1103515245 + 12345) % 2147483648;
      return seed / 2147483648;
    };
    for (let i = 0; i < 220; i++) {
      const az = rand() * Math.PI * 2;
      const el = 0.15 + rand() * 1.25;
      const r = kswScene.domeRadius * 0.85;
      starPositions.push(r * Math.cos(el) * Math.cos(az), r * Math.sin(el), r * Math.cos(el) * Math.sin(az));
    }
    const starGeo = new THREE.BufferGeometry();
    starGeo.setAttribute('position', new THREE.Float32BufferAttribute(starPositions, 3));
    const stars = new THREE.Points(
      starGeo,
      new THREE.PointsMaterial({ color: palette.star, size: 0.8, sizeAttenuation: true, transparent: true, opacity: 0.85, fog: false }),
    );
    scene.add(stars);
  }

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
  if (presetName !== 'night') {
    applySunState(phys.timeOfDay);
  } else {
    sun.color.set(moonLight.color);
    sun.intensity = moonLight.intensity;
    sun.position.set(...moonLight.position).normalize().multiplyScalar(kswScene.sunDistance);
    applySunState(phys.timeOfDay);
  }
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
      const penumbra = blockerDist.mul(260).clamp(0.6, 11);
      const filterR = fnode(penumbra.mul(float(texel)));
      let lit: FN = float(0);
      for (const off of taps) lit = fnode(lit.add(cmp(off, filterR, fnode(z))));
      return select(occSum.lessThan(0.02), float(1), lit.mul(1 / 16));
    });
    (sun.shadow as unknown as { filterNode?: unknown }).filterNode = pcss;
  }
  scene.add(sun);

  const hemi = new THREE.HemisphereLight(preset.hemiSky, preset.hemiGround, preset.hemiIntensity * gi.hemiCut);
  scene.add(hemi);

  // ── the hospital ───────────────────────────────────────────────────────
  const { group: hospital, roofs } = buildHospital(kswPlan, { lampGlow: preset.lampOn });
  scene.add(hospital);
  roofs.setFade(roofFade(rig.radius, kswCamera));

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
  cityRoot.add(buildNature(cityNature, { excludeRect: { x: 0, z: 0, w: kswPlan.plate.w, d: kswPlan.plate.d } }));
  cityRoot.add(buildWindows(cityBuildings, { lampGlow: preset.lampOn }));
  cityRoot.add(buildLamps(cityRoads, { lampGlow: preset.lampOn }));
  scene.add(cityRoot);

  // 3-ring semantic LOD (Task 10, spec §2c): detail follows the camera radius.
  // getObjectByName can legitimately miss (design-legal), so refs are
  // collected defensively and applyCityLod is null-tolerant.
  const lodRefs: CityLodRefs = {
    windows: cityRoot.getObjectByName('cityWindows') ?? null,
    lamps: cityRoot.getObjectByName('cityLamps') ?? null,
    footways: cityRoot.getObjectByName('footwayRibbons') ?? null,
    treesFull: ['treeCanopies', 'treeConifers']
      .map((n) => cityRoot.getObjectByName(n))
      .filter((o): o is THREE.Object3D => o !== undefined),
    treeImpostors: cityRoot.getObjectByName('treeImpostors') ?? null,
    setTreeShadows: (on: boolean) => {
      const canopies = cityRoot.getObjectByName('treeCanopies');
      const conifers = cityRoot.getObjectByName('treeConifers');
      if (canopies) canopies.castShadow = on;
      if (conifers) conifers.castShadow = on;
    },
  };
  let cityRing = cityLodState(rig.radius, 'far');
  applyCityLod(cityRing, lodRefs);

  // collect animated bits: ambulance light pulses, helicopter rotor idles.
  // Tag contract shared with staticBatch.isAnimated via ANIMATED_TAGS.
  const animated: Record<(typeof ANIMATED_TAGS)[number], THREE.Object3D[]> = { blink: [], rotor: [] };
  hospital.traverse((o) => {
    for (const tag of ANIMATED_TAGS) if (o.userData[tag]) animated[tag].push(o);
  });
  const blinkers = animated.blink as THREE.Mesh[];
  const rotors = animated.rotor;

  // ── everyone is an agent: dwell -> pick a destination -> walk the nav
  // graph (room -> door -> corridor -> target) -> dwell. Deterministic.
  // Rendering is per-role instanced (agentMeshes.ts): the shader animates
  // squash/waddle/yaw from storage buffers, the CPU keeps only the agent
  // state machine plus flat smoothing slots (eased y, lerped yaw, roll).
  const nav = buildNav(kswPlan);
  const inBuilding = (x: number, z: number): boolean =>
    Math.abs(x - kswPlan.building.x) < kswPlan.building.w / 2 &&
    Math.abs(z - kswPlan.building.z) < kswPlan.building.d / 2;
  // The authored plan people first, then seeded extras up to ?agents=N.
  const spawnSpecs = buildSpawnSpecs(kswPlan, nav, agentTarget);
  // Crowd mode (Slice D): GPU LOD/cull classification + blob shadows instead
  // of real casters. At or below the threshold the authored look is untouched.
  const crowd = spawnSpecs.length > kswAgents.crowdThreshold;
  // Shadow caching (Slice E): in crowd mode every caster is static — agents
  // use blob shadows (castShadow=false, agentMeshes.ts), and outside cycle
  // mode the sun stands still — so re-rendering the shadow map every frame
  // buys nothing (it was ~3.5k of ~3.8k draw calls). The r185 node-based
  // shadow system (ShadowNode.updateBefore) honors the classic
  // light.shadow.autoUpdate / needsUpdate flags: autoUpdate=false freezes
  // the cached depth map, needsUpdate=true re-renders it exactly once.
  // Refresh trigger: the roof fade crossing the castShadow threshold
  // (staticBatch toggles roofBatch.castShadow there). At or below the crowd
  // threshold (real moving agent shadows) and in cycle mode (the sun moves)
  // the per-frame path stays untouched.
  const shadowCached = crowd && !cycleMode;
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
  for (const m of agentInstances.meshes) hospital.add(m);
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

  // Edge mist ring around the plate rim
  const mistMat = new THREE.MeshBasicMaterial({
    color: preset.mistColor,
    transparent: true,
    opacity: preset.mistOpacity,
    depthWrite: false,
  });
  const rimX = kswPlan.plate.w / 2;
  const rimZ = kswPlan.plate.d / 2;
  // walk the plate's rectangle perimeter (an ellipse would dip onto the lawn
  // near the corners) and hug it with small flattened puffs
  {
    const pad = 2.2;
    const hw = rimX + pad;
    const hd = rimZ + pad;
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
      const mist = new THREE.Mesh(new THREE.SphereGeometry(2.4 + (i % 3) * 0.5, 16, 16), mistMat);
      mist.position.set(mx, 0.25, mz);
      mist.scale.y = 0.22;
      scene.add(mist);
    }
  }

  // Night life: window glow + lamp bulbs are baked into the glowNight batch
  // at build time (staticBatch.ts); only the actual light pools live here.
  if (preset.lampOn) {
    // two plaza lampposts actually cast warm pools
    for (const [lx, lz] of [
      [-9.5, 18.3],
      [4.5, 18.3],
    ] as const) {
      const pool = new THREE.PointLight(nightGlow.bulb, 14 * preset.lampBoost, 12, 2);
      pool.position.set(lx, 3.0, lz);
      scene.add(pool);
    }
  }

  // Warm interior points at night: entrance + emergency glow
  if (preset.lampOn) {
    for (const [lx, lz] of [
      [-2.5, 12],
      [-23.5, 12],
      [4, -2],
    ] as const) {
      const lamp = new THREE.PointLight(nightGlow.bulb, 20 * preset.lampBoost, 16, 2);
      lamp.position.set(lx, 2.2, lz);
      scene.add(lamp);
    }
  }

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
  const giScheduler = new GiProbeScheduler(cycleMode ? 'cycle' : 'static');
  scene.environment = cubeRT.texture;
  scene.environmentIntensity = gi.environmentIntensity * preset.giScale * kswPost.envScale[presetName];

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
  let lit = withAo;
  if (presetName !== 'night') {
    const raysNode = godrays(scenePassDepth, camera, sun);
    raysNode.density.value = post.godraysDensity;
    raysNode.maxDensity.value = post.godraysMaxDensity;
    lit = withAo.add(chain(raysNode).mul(kswPost.godraysMix[presetName]));
  }
  // Tilt-shift focus follows the dolly: focus distance = orbit radius.
  const focusU = uniform(rig.radius);
  const withDof = chain(dof(lit, viewZ, focusU, kswPost.dof.focalLength, kswPost.dof.bokehScale));
  const bloomPass = chain(bloom(withDof, post.bloom.strength, post.bloom.radius, kswPost.bloomThreshold));
  const composed = withDof.add(bloomPass);
  const lum = dot(composed.rgb, vec3(0.299, 0.587, 0.114));
  const tone = smoothstep(float(grade.low), float(grade.high), lum);
  const tint = mix(vec3(...grade.shadowTint), vec3(...grade.highlightTint), tone);
  const toned = composed.rgb.mul(tint);
  const satLum = dot(toned, vec3(0.299, 0.587, 0.114));
  const saturated = mix(vec3(satLum, satLum, satLum), toned, float(preset.saturation));
  const contrasted = saturated.sub(float(0.5)).mul(float(preset.contrast)).add(float(0.5)).clamp(0, 1);
  const graded = vec4(contrasted, composed.a);
  postProcessing.outputNode = film(graded, float(post.filmGrain));

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
    const nextRing = cityLodState(rig.radius, cityRing);
    if (nextRing !== cityRing) {
      cityRing = nextRing;
      applyCityLod(cityRing, lodRefs);
      if (shadowCached) sun.shadow.needsUpdate = true;
    }
    const fogZoom = Math.max(1, rig.radius / 110);
    (scene.fog as THREE.Fog).near = fogBaseNear * fogZoom;
    (scene.fog as THREE.Fog).far = fogBaseFar * fogZoom;
    const fade = roofFade(rig.radius, kswCamera);
    roofs.setFade(fade);
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
    focusU.value = rig.radius;
    // edge mist is a close-up treatment; from the overview it would read as
    // separate discs, so it thins out as the camera pulls back
    mistMat.opacity = preset.mistOpacity * (1 - fade * 0.75);
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
    driftU.value = t * cloudCfg.drift;
    frameCount++;
    if (cycleMode) applySunState((t / sunArcCfg.cycleSeconds) % 1);
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
