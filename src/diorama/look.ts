// Klinik-Diorama look prototype: one ER room corner, two bean people, morning light.
// Everything procedural, all values from designTokens.

import * as THREE from 'three/webgpu';
import { float, nodeObject, pass, mrt, output, normalView } from 'three/tsl';
import { ao } from 'three/addons/tsl/display/GTAONode.js';
import { dof } from 'three/addons/tsl/display/DepthOfFieldNode.js';
import { bloom } from 'three/addons/tsl/display/BloomNode.js';
import { film } from 'three/addons/tsl/display/FilmNode.js';
import { godrays } from 'three/addons/tsl/display/GodraysNode.js';
import { RoundedBoxGeometry } from 'three/addons/geometries/RoundedBoxGeometry.js';
import { palette, radii, clay, lightPresets, cameraContract, post, nightGlow, gi } from './designTokens';

declare global {
  interface Window {
    __LOOK_READY?: boolean;
    __LOOK_BACKEND?: string;
  }
}

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
  const presetName = params.get('preset') === 'night' ? 'night' : 'morning';
  const camMode = params.get('cam') === 'far' ? 'far' : 'default';
  const preset = lightPresets[presetName];

  const renderer = new THREE.WebGPURenderer({ antialias: true });
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
  scene.background = new THREE.Color(preset.background);
  scene.fog = new THREE.Fog(preset.fogColor, preset.fogNear, preset.fogFar);

  const camera = new THREE.PerspectiveCamera(cameraContract.fov, window.innerWidth / window.innerHeight, 0.1, 100);
  const camScale = camMode === 'far' ? 1.45 : 1.0;
  camera.position.set(
    cameraContract.position[0] * camScale,
    cameraContract.position[1] * camScale,
    cameraContract.position[2] * camScale,
  );
  camera.lookAt(...cameraContract.target);

  const sun = new THREE.DirectionalLight(preset.sunColor, preset.sunIntensity);
  sun.position.set(...preset.sunPosition);
  sun.castShadow = true;
  sun.shadow.mapSize.set(2048, 2048);
  sun.shadow.camera.left = -9;
  sun.shadow.camera.right = 9;
  sun.shadow.camera.top = 9;
  sun.shadow.camera.bottom = -9;
  sun.shadow.camera.near = 1;
  sun.shadow.camera.far = 30;
  sun.shadow.bias = -0.0004;
  sun.shadow.normalBias = 0.03;
  sun.shadow.radius = 6;
  scene.add(sun);

  const hemi = new THREE.HemisphereLight(preset.hemiSky, preset.hemiGround, preset.hemiIntensity * gi.hemiCut);
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
  const sunDir = new THREE.Vector3(...preset.sunPosition).normalize().negate();
  for (const zc of presetName === 'morning' ? [-1.1, 1.2] : []) {
    const windowCenter = new THREE.Vector3(3.08, 1.65, zc);
    const t = (windowCenter.y - 0.16) / -sunDir.y;
    const pool = windowCenter.clone().addScaledVector(sunDir, t);
    const mid = windowCenter.clone().add(pool).multiplyScalar(0.5);
    const len = windowCenter.distanceTo(pool);
    const shaft = new THREE.Mesh(new THREE.BoxGeometry(1.15, 0.02, len), shaftMat);
    shaft.position.copy(mid);
    shaft.lookAt(pool);
    scene.add(shaft);
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

  if (presetName === 'night') {
    const bulb = new THREE.Mesh(
      new THREE.SphereGeometry(0.09, 12, 12),
      new THREE.MeshBasicMaterial({ color: nightGlow.bulb }),
    );
    bulb.position.set(0, 1.45, 0);
    lamp.add(bulb);
    const lampLight = new THREE.PointLight(nightGlow.bulb, nightGlow.lampIntensity, 12, 2);
    lampLight.position.set(0, 1.5, 0);
    lamp.add(lampLight);
  }

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

  // One-bounce GI: capture the scene from its center, feed it back as IBL
  const cubeRT = new THREE.CubeRenderTarget(256);
  const cubeCam = new THREE.CubeCamera(0.1, 60, cubeRT);
  cubeCam.position.set(0, 1.4, 0);
  scene.add(cubeCam);
  cubeCam.update(renderer as unknown as Parameters<typeof cubeCam.update>[0], scene);
  scene.environment = cubeRT.texture;
  scene.environmentIntensity = gi.environmentIntensity;

  // Post stack: GTAO x color -> tilt-shift DOF -> bloom
  const postProcessing = new THREE.PostProcessing(renderer);
  const scenePass = pass(scene, camera);
  scenePass.setMRT(mrt({ output, normal: normalView }));
  const scenePassColor = scenePass.getTextureNode('output');
  const scenePassNormal = scenePass.getTextureNode('normal');
  const scenePassDepth = scenePass.getTextureNode('depth');
  const aoPass = ao(scenePassDepth, scenePassNormal, camera);
  const withAo = scenePassColor.mul(aoPass.getTextureNode().x);
  const viewZ = scenePass.getViewZNode();
  // Runtime lifts display nodes into chainable shader-node objects via
  // nodeObject; @types/three r185 doesn't model that lift yet — one
  // localized cast at the post-chain boundary.
  const chain = (n: unknown) => nodeObject(n as never) as unknown as typeof scenePassColor;
  let lit = withAo;
  if (presetName === 'morning') {
    const raysNode = godrays(scenePassDepth, camera, sun);
    raysNode.density.value = post.godraysDensity;
    raysNode.maxDensity.value = post.godraysMaxDensity;
    lit = withAo.add(chain(raysNode).mul(post.godraysMix));
  }
  const withDof = chain(dof(lit, viewZ, post.dof.focusDistance * camScale, post.dof.focalLength, post.dof.bokehScale));
  const bloomPass = chain(bloom(withDof, post.bloom.strength, post.bloom.radius, post.bloom.threshold));
  const composed = withDof.add(bloomPass);
  postProcessing.outputNode = film(composed, float(post.filmGrain));

  const clock = new THREE.Clock();
  function animate(): void {
    const t = clock.getElapsedTime();
    nurse.scale.y = 1 + Math.sin(t * 2.2) * 0.012;
    patient.scale.y = 1 + Math.sin(t * 2.2 + 1.4) * 0.012;
    child.scale.y = 0.68 * (1 + Math.sin(t * 2.6 + 0.7) * 0.015);
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
