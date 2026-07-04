// Klinik-Diorama look prototype: one ER room corner, two bean people, morning light.
// Everything procedural, all values from designTokens.

import * as THREE from 'three/webgpu';
import { Break, Fn, If, Loop, cameraPosition, dot, exp, float, int, mix, mx_fractal_noise_float, nodeObject, pass, mrt, output, normalView, positionWorld, select, smoothstep, texture, uniform, vec2, vec3, vec4, velocity } from 'three/tsl';
import { ao } from 'three/addons/tsl/display/GTAONode.js';
import { dof } from 'three/addons/tsl/display/DepthOfFieldNode.js';
import { bloom } from 'three/addons/tsl/display/BloomNode.js';
import { film } from 'three/addons/tsl/display/FilmNode.js';
import { godrays } from 'three/addons/tsl/display/GodraysNode.js';
import { traa } from 'three/addons/tsl/display/TRAANode.js';
import { sss } from 'three/addons/tsl/display/SSSNode.js';
import { boxBlur } from 'three/addons/tsl/display/boxBlur.js';
import { SkyMesh } from 'three/addons/objects/SkyMesh.js';
import { RoundedBoxGeometry } from 'three/addons/geometries/RoundedBoxGeometry.js';
import { palette, radii, clay, cameraContract, post, nightGlow, gi, grade, cloudVol, moonLight, nightSkyLook } from './designTokens';
import { computeEnvironment, type EnvironmentState } from './environment/environment';
import { parseAtParam } from './environment/atParam';
import { applyEnvironment, type EnvironmentTargets } from './environment/applyEnvironment';
import { createPrecipitation } from './environment/precipitation';
import { createStarField, createMoonDisc } from './environment/nightSky';
import { CLEAR_SKY, sampleWeather, startWeatherLoop, type WeatherSeries, type WeatherState } from './environment/weather';

declare global {
  interface Window {
    __LOOK_READY?: boolean;
    __LOOK_BACKEND?: string;
    __ENV_STATE?: unknown;
  }
}

const WX_OVERRIDES: Record<string, WeatherState> = {
  clear: CLEAR_SKY,
  overcast: { ...CLEAR_SKY, cloudCover: 0.97, windSpeedMs: 3 },
  rain: { ...CLEAR_SKY, cloudCover: 0.9, precipMmPerH: 4, windSpeedMs: 5, temperatureC: 10 },
  snow: { ...CLEAR_SKY, cloudCover: 0.9, precipMmPerH: 3, snow: true, temperatureC: -2 },
  fog: { ...CLEAR_SKY, cloudCover: 0.6, visibilityM: 150, fog: true, windSpeedMs: 0.5 },
};

const materialCache = new Map<number, THREE.MeshPhysicalMaterial>();
function clayMat(color: number): THREE.MeshPhysicalMaterial {
  let m = materialCache.get(color);
  if (!m) {
    m = new THREE.MeshPhysicalMaterial({ color, roughness: clay.roughness, metalness: clay.metalness });
    m.sheen = clay.sheen;
    m.sheenRoughness = clay.sheenRoughness;
    m.sheenColor = new THREE.Color(color).lerp(new THREE.Color(0xffffff), 0.5);
    materialCache.set(color, m);
  }
  return m;
}

let glassMaterial: THREE.MeshStandardMaterial | null = null;
function glassMat(): THREE.MeshStandardMaterial {
  if (!glassMaterial) {
    glassMaterial = new THREE.MeshStandardMaterial({
      color: palette.glass,
      roughness: 0.4,
      metalness: 0,
      transparent: true,
      opacity: 0.16,
    });
  }
  return glassMaterial;
}

function box(w: number, h: number, d: number, color: number, r: number = radii.s): THREE.Mesh {
  const radius = Math.max(0.01, Math.min(r, w / 2 - 1e-3, h / 2 - 1e-3, d / 2 - 1e-3));
  const mesh = new THREE.Mesh(new RoundedBoxGeometry(w, h, d, 4, radius), clayMat(color));
  mesh.castShadow = true;
  mesh.receiveShadow = true;
  return mesh;
}

function cylinder(rTop: number, rBot: number, h: number, color: number, seg = 20): THREE.Mesh {
  const mesh = new THREE.Mesh(new THREE.CylinderGeometry(rTop, rBot, h, seg), clayMat(color));
  mesh.castShadow = true;
  mesh.receiveShadow = true;
  return mesh;
}

function beanPerson(bodyColor: number, faceYaw: number): THREE.Group {
  const g = new THREE.Group();
  const body = new THREE.Mesh(new THREE.CapsuleGeometry(0.34, 0.55, 8, 24), clayMat(bodyColor));
  body.position.y = 0.62;
  body.castShadow = true;
  body.receiveShadow = true;
  g.add(body);
  const eyeGeo = new THREE.SphereGeometry(0.052, 12, 12);
  for (const side of [-1, 1]) {
    const eye = new THREE.Mesh(eyeGeo, clayMat(palette.eye));
    eye.position.set(side * 0.105, 0.92, 0.305);
    g.add(eye);
  }
  const mouth = new THREE.Mesh(new THREE.CapsuleGeometry(0.02, 0.06, 4, 8), clayMat(palette.eye));
  mouth.rotation.z = Math.PI / 2;
  mouth.position.set(0, 0.8, 0.33);
  g.add(mouth);
  g.rotation.y = faceYaw;
  return g;
}

type Opening = { center: number; width: number; sillY: number; headY: number };

// Wall built from three horizontal bands (no vertical slits): full-length base
// band, full-length head band, and mid-band segments between the openings.
function wallWithWindows(length: number, height: number, thickness: number, openings: Opening[]): THREE.Group {
  const g = new THREE.Group();
  const sill = Math.min(...openings.map((o) => o.sillY));
  const head = Math.max(...openings.map((o) => o.headY));

  const lap = 0.06;
  const base = box(length, sill + lap, thickness, palette.creamBase, radii.xs);
  base.position.y = (sill + lap) / 2;
  g.add(base);

  const topH = height - head;
  if (topH > 0.05) {
    const top = box(length, topH + lap, thickness, palette.creamBase, radii.xs);
    top.position.y = head - lap + (topH + lap) / 2;
    g.add(top);
  }

  const midH = head - sill;
  let cursor = -length / 2;
  const sorted = [...openings].sort((a, b) => a.center - b.center);
  for (const o of sorted) {
    const left = o.center - o.width / 2;
    if (left - cursor > 0.05) {
      const w = left - cursor;
      const seg = box(w, midH, thickness, palette.creamBase, radii.xs);
      seg.position.set(cursor + w / 2, sill + midH / 2, 0);
      g.add(seg);
    }
    cursor = o.center + o.width / 2;
  }
  if (length / 2 - cursor > 0.05) {
    const w = length / 2 - cursor;
    const seg = box(w, midH, thickness, palette.creamBase, radii.xs);
    seg.position.set(cursor + w / 2, sill + midH / 2, 0);
    g.add(seg);
  }

  // White window frames + mullion for each opening
  for (const o of sorted) {
    const fh = o.headY - o.sillY;
    const ft = 0.1;
    const depth = thickness + 0.08;
    const bottom = box(o.width + 0.16, ft, depth, palette.white, radii.xs);
    bottom.position.set(o.center, o.sillY + ft / 2 - 0.02, 0);
    g.add(bottom);
    const top = box(o.width + 0.16, ft, depth, palette.white, radii.xs);
    top.position.set(o.center, o.headY - ft / 2 + 0.02, 0);
    g.add(top);
    for (const side of [-1, 1]) {
      const jamb = box(ft, fh, depth, palette.white, radii.xs);
      jamb.position.set(o.center + side * (o.width / 2 + 0.03), o.sillY + fh / 2, 0);
      g.add(jamb);
    }
    const mullion = box(0.07, fh - 0.1, 0.09, palette.white, radii.xs);
    mullion.position.set(o.center, o.sillY + fh / 2, 0);
    g.add(mullion);
    const pane = new THREE.Mesh(new THREE.BoxGeometry(o.width - 0.04, fh - 0.06, 0.03), glassMat());
    pane.position.set(o.center, o.sillY + fh / 2, 0);
    g.add(pane);
  }
  return g;
}

function hospitalBed(): THREE.Group {
  const g = new THREE.Group();
  const frame = box(2.0, 0.32, 0.95, palette.woodSoft, radii.m);
  frame.position.y = 0.3;
  g.add(frame);
  const mattress = box(1.9, 0.2, 0.85, palette.white, radii.m);
  mattress.position.y = 0.56;
  g.add(mattress);
  const blanket = box(1.0, 0.13, 0.87, palette.coralSoft, radii.m);
  blanket.position.set(-0.5, 0.67, 0);
  g.add(blanket);
  const pillow = box(0.42, 0.15, 0.55, palette.creamLight, radii.m);
  pillow.position.set(0.68, 0.68, 0);
  g.add(pillow);
  const head = box(0.1, 0.75, 0.95, palette.woodSoft, radii.m);
  head.position.set(0.99, 0.46, 0);
  g.add(head);
  return g;
}

function careCart(): THREE.Group {
  const g = new THREE.Group();
  const body = box(0.66, 0.74, 0.46, palette.white, radii.m);
  body.position.y = 0.5;
  g.add(body);
  for (const dy of [-0.2, 0.02, 0.24]) {
    const drawer = box(0.56, 0.015, 0.02, palette.metalMatt, radii.xs);
    drawer.position.set(0, 0.5 + dy, 0.235);
    g.add(drawer);
  }
  const rim = box(0.68, 0.05, 0.48, palette.mint, radii.xs);
  rim.position.y = 0.9;
  g.add(rim);
  const bottleA = cylinder(0.045, 0.045, 0.15, palette.mint, 12);
  bottleA.position.set(-0.16, 1.0, 0.02);
  g.add(bottleA);
  const bottleB = cylinder(0.04, 0.04, 0.12, palette.white, 12);
  bottleB.position.set(0.0, 0.99, -0.08);
  g.add(bottleB);
  for (const [wx, wz] of [
    [-0.24, 0.15],
    [0.24, 0.15],
    [-0.24, -0.15],
    [0.24, -0.15],
  ] as Array<[number, number]>) {
    const wheel = new THREE.Mesh(new THREE.SphereGeometry(0.055, 12, 12), clayMat(palette.metalDark));
    wheel.position.set(wx, 0.055, wz);
    wheel.castShadow = true;
    g.add(wheel);
  }
  return g;
}

function ivStand(): THREE.Group {
  const g = new THREE.Group();
  const pole = cylinder(0.05, 0.05, 1.65, palette.white, 14);
  pole.position.y = 0.85;
  g.add(pole);
  const base = cylinder(0.2, 0.26, 0.09, palette.white);
  base.position.y = 0.045;
  g.add(base);
  const bag = box(0.2, 0.32, 0.09, palette.mint, radii.s);
  bag.position.set(0.14, 1.5, 0);
  g.add(bag);
  const arm = cylinder(0.028, 0.028, 0.3, palette.white, 10);
  arm.rotation.z = Math.PI / 2;
  arm.position.set(0.08, 1.68, 0);
  g.add(arm);
  return g;
}

function vitalsMonitor(): THREE.Group {
  const g = new THREE.Group();
  const pole = cylinder(0.045, 0.045, 1.15, palette.white, 12);
  pole.position.y = 0.6;
  g.add(pole);
  const base = cylinder(0.18, 0.23, 0.08, palette.white);
  base.position.y = 0.04;
  g.add(base);
  const screen = box(0.46, 0.34, 0.1, palette.eye, radii.s);
  screen.position.y = 1.32;
  g.add(screen);
  const trace = box(0.3, 0.03, 0.02, palette.mint, radii.xs);
  trace.position.set(0, 1.32, 0.06);
  g.add(trace);
  const blip = box(0.05, 0.09, 0.02, palette.mint, radii.xs);
  blip.position.set(0.05, 1.34, 0.06);
  g.add(blip);
  return g;
}

function plant(scale = 1): THREE.Group {
  const g = new THREE.Group();
  const pot = cylinder(0.19, 0.24, 0.3, palette.plantPot);
  pot.position.y = 0.15;
  g.add(pot);
  const puffs: Array<[number, number, number, number]> = [
    [0, 0.56, 0, 0.25],
    [0.15, 0.44, 0.07, 0.16],
    [-0.13, 0.47, -0.06, 0.17],
  ];
  for (const [x, y, z, r] of puffs) {
    const puff = new THREE.Mesh(new THREE.SphereGeometry(r, 18, 18), clayMat(palette.plantGreen));
    puff.position.set(x, y, z);
    puff.castShadow = true;
    g.add(puff);
  }
  g.scale.setScalar(scale);
  return g;
}

function sideTable(): THREE.Group {
  const g = new THREE.Group();
  const body = box(0.52, 0.52, 0.48, palette.woodSoft, radii.m);
  body.position.y = 0.26;
  g.add(body);
  const cup = cylinder(0.055, 0.05, 0.11, palette.white, 14);
  cup.position.set(0.08, 0.58, 0.05);
  g.add(cup);
  return g;
}

async function boot(): Promise<void> {
  const params = new URLSearchParams(window.location.search);
  // ?at= freezes the clock: full ISO instant, or HH:MM = today local time.
  const frozenAt = parseAtParam(params.get('at'));
  const wxParam = params.get('wx'); // 'clear'|'overcast'|'rain'|'snow'|'fog'|null
  const camModeRaw = params.get('cam');
  const camMode = camModeRaw === 'far' || camModeRaw === 'sky' ? camModeRaw : 'default';
  const now = (): Date => frozenAt ?? new Date();
  const initialWeather = (): WeatherState => WX_OVERRIDES[wxParam ?? ''] ?? CLEAR_SKY;
  const initialEnv = computeEnvironment(now(), initialWeather());

  // No MSAA: TRAA is the anti-aliasing (multisampled depth also breaks TRAA history).
  const renderer = new THREE.WebGPURenderer({ antialias: false });
  await renderer.init();
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  renderer.setSize(window.innerWidth, window.innerHeight);
  renderer.shadowMap.enabled = true;
  renderer.toneMapping = THREE.AgXToneMapping;
  renderer.toneMappingExposure = initialEnv.exposure;
  document.body.appendChild(renderer.domElement);
  window.__LOOK_BACKEND = (renderer.backend as { isWebGPUBackend?: boolean }).isWebGPUBackend
    ? 'webgpu'
    : 'webgl2';

  const scene = new THREE.Scene();
  const fog = new THREE.Fog(initialEnv.fogColor, initialEnv.fogNear, initialEnv.fogFar);
  scene.fog = fog;

  const initialSunDir = new THREE.Vector3(initialEnv.sunDir[0], initialEnv.sunDir[1], initialEnv.sunDir[2]);

  // Physical sky (Rayleigh/Mie scattering) — real sunrise/sunset colors
  const skyMesh = new SkyMesh();
  skyMesh.scale.setScalar(400);
  skyMesh.turbidity.value = initialEnv.turbidity;
  skyMesh.rayleigh.value = initialEnv.rayleigh;
  skyMesh.mieCoefficient.value = initialEnv.mieCoefficient;
  skyMesh.mieDirectionalG.value = initialEnv.mieG;
  skyMesh.sunPosition.value.copy(initialSunDir);
  // Scene fog (far 46-48) would otherwise flat-tint the whole sky shell (r=400).
  (skyMesh.material as THREE.Material & { fog: boolean }).fog = false;
  scene.add(skyMesh);

  // Volumetric clouds: raymarch a height-band slab (y in [base..top]) from the
  // camera, front-to-back Beer-Lambert with a short secondary march toward the
  // light (beer * powder = bright cores, dark sun-facing edges).
  const cloudLightDir = uniform(initialSunDir.clone());
  const cloudLit = uniform(new THREE.Color(0xffffff));
  const cloudShadow = uniform(new THREE.Color(0x9aa8b5));
  const driftUV = uniform(new THREE.Vector2(0, 0));
  const coverageU = uniform(0.44);
  const cloudMatVol = new THREE.MeshBasicNodeMaterial();
  cloudMatVol.transparent = true;
  cloudMatVol.side = THREE.BackSide;
  cloudMatVol.depthWrite = false;
  cloudMatVol.fog = false;
  {
    // TSL var/loop nodes aren't modellable with @types/three r185 — runtime-typed.
    type N = any;
    const fnode = (n: unknown): N => n as N;
    const slabH = cloudVol.top - cloudVol.base;
    const densityAt = (p: N): N => {
      const q = vec3(
        p.x.mul(float(cloudVol.scale)).add(driftUV.x),
        p.y.mul(float(cloudVol.scale * 1.35)),
        p.z.mul(float(cloudVol.scale)).add(driftUV.y),
      );
      const n = fnode(mx_fractal_noise_float(q, 4, 2.0, 0.55, 1.0)).mul(0.5).add(0.5);
      const hN = p.y.sub(float(cloudVol.base)).mul(1 / slabH).clamp(0, 1);
      const profile = smoothstep(float(0.0), float(0.16), hN).mul(smoothstep(float(1.0), float(0.5), hN));
      return n.mul(profile).sub(float(1).sub(coverageU)).max(0).mul(float(cloudVol.density));
    };
    type Vec3Node = ReturnType<typeof vec3>;
    const shadowN = cloudShadow as unknown as Vec3Node;
    const litN = (cloudLit as unknown as Vec3Node).mul(float(cloudVol.litBoost));
    const vol = fnode(
      Fn(() => {
        const resCol = fnode(vec3(0).toVar());
        const resA = fnode(float(0).toVar());
        const rd = fnode(positionWorld.sub(cameraPosition).normalize());
        If(rd.y.greaterThan(0.015), () => {
          const t0 = fnode(float(cloudVol.base).sub(cameraPosition.y).div(rd.y));
          const t1 = fnode(float(cloudVol.top).sub(cameraPosition.y).div(rd.y));
          const dt = t1.min(t0.add(float(cloudVol.maxDist))).sub(t0).mul(1 / cloudVol.steps);
          const trans = fnode(float(1).toVar());
          const acc = fnode(vec3(0).toVar());
          Loop(cloudVol.steps, ({ i }: { i: N }) => {
            const p = fnode(cameraPosition.add(rd.mul(t0.add(dt.mul(float(i).add(0.5))))));
            const d = densityAt(p);
            If(d.greaterThan(0.002), () => {
              const dl = fnode(float(0).toVar());
              Loop(cloudVol.lightSteps, ({ i: j }: { i: N }) => {
                const lp = fnode(p.add(fnode(cloudLightDir).mul(float(j).add(1).mul(float(cloudVol.lightStep)))));
                dl.addAssign(densityAt(lp));
              });
              const depthL = dl.mul(cloudVol.lightStep * cloudVol.absorption);
              const beer = exp(depthL.negate());
              const powder = float(1).sub(exp(depthL.mul(-2)));
              const col = fnode(shadowN).add(fnode(litN).mul(beer.mul(powder).mul(2)));
              const aStep = float(1).sub(exp(d.mul(dt).negate()));
              acc.addAssign(col.mul(aStep).mul(trans));
              trans.mulAssign(float(1).sub(aStep));
            });
            If(trans.lessThan(0.02), () => {
              Break();
            });
          });
          const alpha = float(1).sub(trans);
          const horizonFade = smoothstep(float(0.015), float(0.12), rd.y);
          resA.assign(alpha.mul(horizonFade));
          resCol.assign(acc.div(alpha.max(0.0001)));
        });
        return vec4(resCol, resA);
      })(),
    );
    cloudMatVol.colorNode = vol.rgb;
    cloudMatVol.opacityNode = vol.a;
  }
  const cloudDome = new THREE.Mesh(new THREE.SphereGeometry(46, 32, 24), cloudMatVol);
  scene.add(cloudDome);

  // Only the sun disc sits beyond the cloud sphere (r=46) so clouds occlude it
  // by day. The moon (and stars) sit inside at r=17; their cloud occlusion is
  // emulated via starVisibility opacity/visibility damping, not geometry.
  const sunDisc = new THREE.Mesh(
    new THREE.SphereGeometry(2.4, 20, 20),
    new THREE.MeshBasicMaterial({ color: 0xfff0d5, fog: false }),
  );
  scene.add(sunDisc);
  // Moon disc + star field — extracted to environment/nightSky.ts (room
  // values from designTokens.nightSkyLook.room, byte-identical to the former
  // inline constants: STAR_R=17, quad 0.05, count 420, moon 0.46 @ dist 17).
  // radius 0.46 at dome-distance 17 ≈ the old 1.6 at distance 60 (same apparent size)
  const { mesh: moonDisc, phaseDir: moonPhaseDirU } = createMoonDisc({
    radius: nightSkyLook.room.moonRadius,
    distance: nightSkyLook.room.moonDistance,
  });
  scene.add(moonDisc);

  // Stars — always built; visibility/opacity/rotation driven by applyEnvironment.
  // Radius sits ON the DoF focal plane (focusDistance 16.5) so the tilt-shift bokeh
  // keeps the stars crisp, while still comfortably behind the ~5-unit diorama.
  const { object3d: starsObj, material: starsMat } = createStarField({
    radius: nightSkyLook.room.starRadius,
    quadSize: nightSkyLook.room.starQuad,
    count: nightSkyLook.room.starCount,
  });
  const stars = starsObj as THREE.InstancedMesh;
  const starsMaterial = starsMat as THREE.MeshBasicMaterial;
  scene.add(stars);

  const camera = new THREE.PerspectiveCamera(cameraContract.fov, window.innerWidth / window.innerHeight, 0.1, 100);
  const camScale = camMode === 'far' ? 1.45 : 1.0;
  camera.position.set(
    cameraContract.position[0] * camScale,
    cameraContract.position[1] * camScale,
    cameraContract.position[2] * camScale,
  );
  camera.lookAt(...cameraContract.target);
  if (camMode === 'sky') {
    camera.position.set(-6, 2.5, 9);
    camera.lookAt(14, 9, -10);
    camera.fov = 55;
    camera.updateProjectionMatrix();
  }

  const sun = new THREE.DirectionalLight(moonLight.color, moonLight.intensity);
  sun.position.copy(initialSunDir).multiplyScalar(12);
  sun.castShadow = true;
  sun.shadow.mapSize.set(2048, 2048);
  sun.shadow.camera.left = -12;
  sun.shadow.camera.right = 12;
  sun.shadow.camera.top = 14;
  sun.shadow.camera.bottom = -12;
  sun.shadow.camera.near = 1;
  sun.shadow.camera.far = 45;
  sun.shadow.bias = -0.0004;
  sun.shadow.normalBias = 0.03;
  // PCSS: blocker search -> penumbra-sized PCF (contact-hardening soft shadows)
  {
    const texel = 1 / 2048;
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

  const hemi = new THREE.HemisphereLight(initialEnv.hemiSky, initialEnv.hemiGround, initialEnv.hemiIntensity);
  scene.add(hemi);

  // Diorama base: a soft lawn plate
  const plate = box(14, 0.5, 11, palette.lawn, radii.l);
  plate.position.y = -0.25;
  scene.add(plate);

  // Room floor — distinctly warmer than the walls
  const floor = box(7, 0.14, 5.6, palette.floorWarm, radii.m);
  floor.position.set(0, 0.07, 0);
  scene.add(floor);

  // Walls: north (back, z-) one window; east (x+, sun side) two windows.
  const wallH = 2.9;
  const wallT = 0.42;
  const north = wallWithWindows(7, wallH, wallT, [
    { center: -1.4, width: 1.5, sillY: 0.95, headY: 2.35 },
  ]);
  north.position.set(0, 0.14, -2.8 + wallT / 2);
  scene.add(north);

  const east = wallWithWindows(5.6, wallH, wallT, [
    { center: -1.1, width: 1.3, sillY: 0.95, headY: 2.35 },
    { center: 1.2, width: 1.3, sillY: 0.95, headY: 2.35 },
  ]);
  east.rotation.y = -Math.PI / 2;
  east.position.set(3.5 - wallT / 2, 0.14, 0);
  scene.add(east);

  // Furniture — staged for the SW camera
  const bed = hospitalBed();
  bed.position.set(1.35, 0.14, -1.5);
  scene.add(bed);

  const cart = careCart();
  cart.position.set(-0.38, 0.14, -1.3);
  cart.rotation.y = 0.35;
  scene.add(cart);

  const iv = ivStand();
  iv.position.set(2.82, 0.14, -0.95);
  scene.add(iv);

  const monitor = vitalsMonitor();
  monitor.position.set(1.95, 0.14, -2.08);
  monitor.rotation.y = 0.35;
  scene.add(monitor);

  const table = sideTable();
  table.position.set(0.05, 0.14, -2.15);
  scene.add(table);

  const rug = cylinder(1.15, 1.15, 0.06, palette.mint, 36);
  rug.position.set(-0.75, 0.14 + 0.03, 0.8);
  rug.castShadow = false;
  scene.add(rug);

  const plantBig = plant(1.15);
  plantBig.position.set(2.9, 0.14, 2.35);
  scene.add(plantBig);

  const plantSmall = plant(0.8);
  plantSmall.position.set(4.9, -0.02, 3.4);
  scene.add(plantSmall);

  // Cozy density: cabinet, wall art, waiting bench, curtains
  const cabinet = box(1.0, 1.3, 0.42, palette.sage, radii.m);
  cabinet.position.set(-2.35, 0.14 + 0.65, -2.32);
  scene.add(cabinet);
  for (const dy of [-0.28, 0.12]) {
    const knob = new THREE.Mesh(new THREE.SphereGeometry(0.04, 10, 10), clayMat(palette.white));
    knob.position.set(-2.35, 0.14 + 0.65 + dy, -2.09);
    scene.add(knob);
  }

  const art1 = box(0.72, 0.52, 0.07, palette.mint, radii.s);
  art1.position.set(0.78, 1.85, -2.34);
  scene.add(art1);
  const art2 = box(0.46, 0.62, 0.07, palette.coralSoft, radii.s);
  art2.position.set(-0.12, 1.72, -2.34);
  scene.add(art2);

  const bench = box(0.52, 0.4, 1.5, palette.woodSoft, radii.m);
  bench.position.set(3.0, 0.14 + 0.2, 1.35);
  scene.add(bench);
  const benchBack = box(0.12, 0.5, 1.5, palette.woodSoft, radii.s);
  benchBack.position.set(3.22, 0.14 + 0.62, 1.35);
  scene.add(benchBack);

  const curtainA = box(0.26, 1.62, 0.12, palette.coralSoft, radii.s);
  curtainA.position.set(3.05, 1.72, 1.98);
  scene.add(curtainA);
  const curtainB = box(0.26, 1.62, 0.12, palette.coralSoft, radii.s);
  curtainB.position.set(3.05, 1.72, -0.28);
  scene.add(curtainB);
  for (const zc of [1.1, -1.2]) {
    const rod = cylinder(0.03, 0.03, 1.9, palette.white, 10);
    rod.rotation.x = Math.PI / 2;
    rod.position.set(3.04, 2.52, -zc);
    scene.add(rod);
  }

  // Visible morning light shafts: slabs aimed from each east window along the sun
  // direction to its floor pool (computed, not eyeballed).
  const shaftMat = new THREE.MeshBasicMaterial({
    color: palette.sunShaft,
    transparent: true,
    opacity: 0.07,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
    side: THREE.DoubleSide,
  });
  // Shafts are built with a fixed unit length (SHAFT_BASE_LEN = 3) and scaled
  // per frame in applyEnvironment; both always exist, faded via shaft opacity.
  const shaftWindows = [new THREE.Vector3(3.08, 1.65, -1.1), new THREE.Vector3(3.08, 1.65, 1.2)];
  const shafts: THREE.Mesh[] = [];
  for (const windowCenter of shaftWindows) {
    const shaft = new THREE.Mesh(new THREE.BoxGeometry(1.15, 0.02, 3), shaftMat);
    shaft.position.copy(windowCenter);
    shaft.visible = false;
    scene.add(shaft);
    shafts.push(shaft);
  }

  const lamp = new THREE.Group();
  const lampPole = cylinder(0.04, 0.05, 1.45, palette.woodSoft, 12);
  lampPole.position.y = 0.72;
  lamp.add(lampPole);
  const lampShade = cylinder(0.24, 0.34, 0.34, palette.creamLight, 20);
  lampShade.position.y = 1.55;
  lamp.add(lampShade);
  lamp.position.set(-3.0, 0.14, -0.9);
  scene.add(lamp);

  // Bulb + light always built; intensity/visibility driven by applyEnvironment (lampOn01).
  const lampBulb = new THREE.Mesh(
    new THREE.SphereGeometry(0.09, 12, 12),
    new THREE.MeshBasicMaterial({ color: nightGlow.bulb }),
  );
  lampBulb.position.set(0, 1.45, 0);
  lampBulb.visible = false;
  lamp.add(lampBulb);
  const lampLight = new THREE.PointLight(nightGlow.bulb, 0, 12, 2);
  lampLight.position.set(0, 1.5, 0);
  lamp.add(lampLight);

  // People — eyes toward the camera side
  const nurse = beanPerson(palette.mint, -1.1);
  nurse.position.set(1.9, 0.14, -0.5);
  const badge = box(0.11, 0.14, 0.03, palette.white, radii.xs);
  badge.position.set(0.14, 0.72, 0.31);
  nurse.add(badge);
  scene.add(nurse);

  const patient = beanPerson(palette.coral, -0.75);
  patient.position.set(-1.55, 0.14, 1.05);
  scene.add(patient);

  const child = beanPerson(palette.honey, -0.9);
  child.scale.setScalar(0.68);
  child.position.set(-0.8, 0.14, 1.35);
  scene.add(child);

  // Edge mist: soft flattened puffs hugging the plate rim (the diorama floats in haze)
  const mistMat = new THREE.MeshBasicMaterial({
    color: initialEnv.mistColor,
    transparent: true,
    opacity: initialEnv.mistOpacity,
    depthWrite: false,
  });
  const mistSpecs: Array<[number, number, number, number]> = [
    [-2.5, 0.25, -6.4, 2.8],
    [2.8, 0.3, -6.8, 3.2],
    [7.6, 0.25, -5.6, 2.6],
    [9.2, 0.3, -1.5, 3.0],
    [9.6, 0.25, 3.2, 2.6],
    [-7.9, 0.25, -5.9, 2.4],
  ];
  for (const [mx, my, mz, mr] of mistSpecs) {
    const mist = new THREE.Mesh(new THREE.SphereGeometry(mr, 16, 16), mistMat);
    mist.position.set(mx, my, mz);
    mist.scale.y = 0.22;
    scene.add(mist);
  }

  // One-bounce GI: capture the scene from its center, feed it back as IBL
  const cubeRT = new THREE.CubeRenderTarget(256);
  const cubeCam = new THREE.CubeCamera(0.1, 60, cubeRT);
  cubeCam.position.set(0, 1.4, 0);
  scene.add(cubeCam);
  cubeCam.update(renderer as unknown as Parameters<typeof cubeCam.update>[0], scene);
  scene.environment = cubeRT.texture;
  scene.environmentIntensity = gi.environmentIntensity * initialEnv.giScale;

  // Post stack: GTAO x color -> tilt-shift DOF -> bloom
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
  const contactRaw = sss(scenePassDepth, camera, sun);
  const contactBlur = chain(boxBlur((contactRaw as unknown as { r: unknown }).r as never, { size: int(2) as never, separation: int(1) as never }));
  // Screen-space contact shadows: integrated but disabled for the clay style
  // (adds speckle on smooth blob geometry; meant for high-detail meshes).
  const contact = mix(float(1), contactBlur.x, float(0.0));
  const withAo = beautyAA.mul(aoPass.getTextureNode().x).mul(contact);
  const viewZ = scenePass.getViewZNode();
  // Runtime lifts display nodes into chainable shader-node objects via
  // nodeObject; @types/three r185 doesn't model that lift yet — one
  // localized cast at the post-chain boundary.
  // Godrays always built now; the mix is a live uniform driven by applyEnvironment.
  const godraysMixU = uniform(0);
  const raysNode = godrays(scenePassDepth, camera, sun);
  raysNode.density.value = post.godraysDensity;
  raysNode.maxDensity.value = post.godraysMaxDensity;
  const lit = withAo.add(chain(raysNode).mul(godraysMixU));
  const withDof = chain(dof(lit, viewZ, post.dof.focusDistance * camScale, post.dof.focalLength, post.dof.bokehScale));
  const bloomPass = chain(bloom(withDof, post.bloom.strength, post.bloom.radius, post.bloom.threshold));
  const composed = withDof.add(bloomPass);
  // DREDGE split toning: shadows lean teal, highlights lean amber
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

  // Precipitation: GPU instanced rain/snow particles, animated per-frame from
  // uniforms (see environment/precipitation.ts). Driven by applyEnvironment.
  const precipitation = createPrecipitation();
  scene.add(precipitation.object3d);

  // Weather: live loop unless a ?wx override pins a fixed state.
  let weatherSeries: WeatherSeries | null = null;
  if (!wxParam) startWeatherLoop((s) => { weatherSeries = s; });
  const currentWeather = (): WeatherState =>
    WX_OVERRIDES[wxParam ?? ''] ?? (weatherSeries ? sampleWeather(weatherSeries, now()) : CLEAR_SKY);

  // TSL uniform nodes expose a runtime `.value`; @types/three r185 doesn't model
  // that on the node union, so the uniform bundle is cast at this boundary only.
  const targets: EnvironmentTargets = {
    renderer,
    fog,
    sun,
    hemi,
    skyMesh: skyMesh as unknown as EnvironmentTargets['skyMesh'],
    cloudUniforms: {
      lightDir: cloudLightDir as unknown as EnvironmentTargets['cloudUniforms']['lightDir'],
      lit: cloudLit as unknown as EnvironmentTargets['cloudUniforms']['lit'],
      shadow: cloudShadow as unknown as EnvironmentTargets['cloudUniforms']['shadow'],
      coverage: coverageU as unknown as EnvironmentTargets['cloudUniforms']['coverage'],
      driftUV: driftUV as unknown as EnvironmentTargets['cloudUniforms']['driftUV'],
    },
    postUniforms: {
      saturation: saturationU as unknown as EnvironmentTargets['postUniforms']['saturation'],
      contrast: contrastU as unknown as EnvironmentTargets['postUniforms']['contrast'],
      godraysMix: godraysMixU as unknown as EnvironmentTargets['postUniforms']['godraysMix'],
    },
    mistMaterial: mistMat,
    sunDisc,
    moonDisc,
    moonDistance: nightSkyLook.room.moonDistance,
    moonPhaseDir: moonPhaseDirU as unknown as EnvironmentTargets['moonPhaseDir'],
    lampLight,
    lampBulb,
    stars,
    starsMaterial,
    shaftMaterial: shaftMat,
    shafts,
    shaftWindows,
    precipitation,
    scratch: { v3: new THREE.Vector3(), c1: new THREE.Color(), c2: new THREE.Color() },
  };

  let lastEnv: EnvironmentState = computeEnvironment(now(), currentWeather());
  let frameCount = 0;
  let lastT = 0;
  const clock = new THREE.Clock();
  function animate(): void {
    const t = clock.getElapsedTime();
    const dt = Math.min(t - lastT, 0.1);
    lastT = t;
    nurse.scale.y = 1 + Math.sin(t * 2.2) * 0.012;
    patient.scale.y = 1 + Math.sin(t * 2.2 + 1.4) * 0.012;
    child.scale.y = 0.68 * (1 + Math.sin(t * 2.6 + 0.7) * 0.015);
    lastEnv = computeEnvironment(now(), currentWeather());
    applyEnvironment(targets, lastEnv, dt);
    scene.environmentIntensity = gi.environmentIntensity * lastEnv.giScale;
    window.__ENV_STATE = lastEnv;
    frameCount++;
    if (frameCount % 240 === 0) cubeCam.update(renderer as unknown as Parameters<typeof cubeCam.update>[0], scene);
    postProcessing.render();
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
