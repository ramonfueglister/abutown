// Night sky: star field (InstancedMesh billboards) + moon disc, extracted
// from look.ts so both the room prototype and the KSW city can build their
// own dome-scaled instance (see designTokens nightSkyLook.room / .city).
import * as THREE from 'three/webgpu';
import { dot, float, mix, normalLocal, smoothstep, uniform, vec3 } from 'three/tsl';
import { palette, moonDisc as moonDiscTokens } from '../designTokens';

export type StarFieldOpts = { radius: number; quadSize: number; count: number; seed?: number };

// Pure direction generator: deterministic LCG PRNG (identical to the former
// inline look.ts code), full-sphere uniform via sin(el) sampled in [-1, 1]
// for equal-area coverage.
export function starDirections(count: number, seed: number): Array<[number, number, number]> {
  let s = seed;
  const rand = (): number => {
    s = (s * 1103515245 + 12345) % 2147483648;
    return s / 2147483648;
  };
  const dirs: Array<[number, number, number]> = [];
  for (let i = 0; i < count; i++) {
    const az = rand() * Math.PI * 2;
    const sinEl = rand() * 2 - 1;
    const cosEl = Math.sqrt(Math.max(0, 1 - sinEl * sinEl));
    dirs.push([cosEl * Math.cos(az), sinEl, cosEl * Math.sin(az)]);
  }
  return dirs;
}

export function createStarField(opts: StarFieldOpts): {
  object3d: THREE.Object3D;
  material: { opacity: number };
} {
  const { radius, quadSize, count } = opts;
  const seed = opts.seed ?? 42;
  const dirs = starDirections(count, seed);

  // Stars are tiny billboarded quads via an InstancedMesh — NOT THREE.Points.
  // A transparent Points cloud never wrote scene depth in the WebGPU MRT pass, so
  // the tilt-shift DoF read the sky's far depth at every star pixel and smeared
  // each one across a giant bokeh → the night sky rendered empty regardless of
  // size/opacity/radius. Real instanced meshes write depth normally, so DoF sees
  // them at the dome radius (≈ focus) and keeps them crisp. sizeAttenuation is
  // emulated: a fixed world-size quad at fixed dome radius reads as a
  // near-constant screen dot.
  const starQuadGeo = new THREE.PlaneGeometry(quadSize, quadSize);
  const starsMaterial = new THREE.MeshBasicMaterial({
    color: palette.star,
    transparent: true,
    opacity: 0,
    fog: false,
    side: THREE.DoubleSide,
    toneMapped: false,
  });
  const stars = new THREE.InstancedMesh(starQuadGeo, starsMaterial, count);
  stars.frustumCulled = false;
  const starMat4 = new THREE.Matrix4();
  const starPos = new THREE.Vector3();
  const starQuat = new THREE.Quaternion();
  const starScale = new THREE.Vector3(1, 1, 1);
  const starLookM = new THREE.Matrix4();
  for (let i = 0; i < count; i++) {
    const [dx, dy, dz] = dirs[i];
    starPos.set(dx * radius, dy * radius, dz * radius);
    // Face the dome centre (≈ camera): billboard each quad toward the origin.
    starLookM.lookAt(starPos, new THREE.Vector3(0, 0, 0), new THREE.Vector3(0, 1, 0));
    starQuat.setFromRotationMatrix(starLookM);
    starMat4.compose(starPos, starQuat, starScale);
    stars.setMatrixAt(i, starMat4);
  }
  stars.instanceMatrix.needsUpdate = true;
  stars.visible = false;

  return { object3d: stars, material: starsMaterial };
}

export function createMoonDisc(opts: { radius: number; distance: number }): {
  mesh: THREE.Mesh;
  phaseDir: { value: THREE.Vector3 };
} {
  const moonPhaseDirU = uniform(new THREE.Vector3(0, 0, -1));
  const moonMat = new THREE.MeshBasicNodeMaterial({ fog: false });
  {
    // TSL uniform nodes expose a runtime `.value`; @types/three r185 doesn't
    // model that on the node union, so the uniform is cast at this boundary only.
    // moonPhaseLightDir is documented as DISC-LOCAL (disc faces viewer, -z toward
    // viewer); the moon disc mesh carries no rotation of its own, so its local
    // space matches that convention — dot against normalLocal, not normalView.
    // Light travels along moonPhaseDirU onto the sphere, so the lit hemisphere is
    // where the outward normal points against that direction, i.e. negate() first.
    const litSide = dot(normalLocal.negate(), moonPhaseDirU as unknown as ReturnType<typeof vec3>);
    const lit = smoothstep(float(-0.15), float(0.15), litSide);
    const moonDarkU = uniform(new THREE.Color(moonDiscTokens.dark));
    const moonLitU = uniform(new THREE.Color(moonDiscTokens.lit));
    moonMat.colorNode = mix(moonDarkU as unknown as ReturnType<typeof vec3>, moonLitU as unknown as ReturnType<typeof vec3>, lit);
  }
  // opts.distance is where applyEnvironment places the disc each frame (kept
  // here only to document the intended dome distance, not used for geometry).
  void opts.distance;
  const moonDisc = new THREE.Mesh(new THREE.SphereGeometry(opts.radius, 20, 20), moonMat);
  moonDisc.visible = false;

  return { mesh: moonDisc, phaseDir: moonPhaseDirU as unknown as { value: THREE.Vector3 } };
}
