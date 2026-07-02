// Instanced bean people for the KSW diorama (Slice C of the 10k-perf design).
// One merged, vertex-colored geometry + one InstancedMesh per role replaces
// the per-agent THREE.Group from props.buildPerson. Per-agent state lives in
// storage buffers (TSL instancedArray, CPU-written for now — Slice D writes
// them from compute) and the vertex stage applies everything main.ts used to
// do on Object3Ds: walk/breathing squash, waddle roll, yaw and the
// (posX, yLift, posZ) translation. The smoothing state (eased y-lift,
// lerped yaw, decaying roll) stays CPU-side in flat per-agent slots.
//
// Look contract: identical bean proportions, part colors, role accessories
// and animation amplitudes/frequencies as the deleted buildPerson +
// per-agent animate loop.

import * as THREE from 'three/webgpu';
import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';
import {
  abs,
  attribute,
  cos,
  float,
  instancedArray,
  mix,
  normalLocal,
  positionLocal,
  sin,
  transformNormalToView,
  uniform,
  vec3,
} from 'three/tsl';
import { clay, palette, radii } from '../designTokens';
import type { PersonRole } from './floorPlan';
import { cyl, roundedBox, sph, tor } from './geometryCache';

// ── CPU mirror math (pure, unit-tested) ─────────────────────────────────

// Shortest-arc angle lerp — behavior-identical to the pre-Slice-C helper in
// main.ts (yaw smoothing rate is min(1, dt*9) at the call site).
export function lerpAngle(a: number, b: number, t: number): number {
  let d = (b - a) % (Math.PI * 2);
  if (d > Math.PI) d -= Math.PI * 2;
  if (d < -Math.PI) d += Math.PI * 2;
  return a + d * t;
}

// Frame-rate-aware exponential approach: current += (target-current)*min(1, dt*rate).
// Used for the y-lift easing toward 0.14 inside the building (rate 10).
export function approach(current: number, target: number, dt: number, rate: number): number {
  return current + (target - current) * Math.min(1, dt * rate);
}

// ── per-role merged geometry ────────────────────────────────────────────

export const CHILD_SCALE = 0.68;

type PartSpec = {
  geo: THREE.BufferGeometry;
  color: number;
  position: [number, number, number];
  rotation?: [number, number, number];
};

// Same radius clamp as props.box() so accessory boxes keep their shape.
function boxPart(w: number, h: number, d: number, r: number): THREE.BufferGeometry {
  const radius = Math.max(0.01, Math.min(r, w / 2 - 1e-3, h / 2 - 1e-3, d / 2 - 1e-3));
  return roundedBox(w, h, d, 4, radius);
}

// The bean itself: capsule body, two eyes, sideways capsule mouth
// (proportions identical to the deleted props.beanPerson).
function beanParts(bodyColor: number): PartSpec[] {
  return [
    { geo: new THREE.CapsuleGeometry(0.34, 0.55, 8, 24), color: bodyColor, position: [0, 0.62, 0] },
    { geo: sph(0.052, 12), color: palette.eye, position: [-0.105, 0.92, 0.305] },
    { geo: sph(0.052, 12), color: palette.eye, position: [0.105, 0.92, 0.305] },
    { geo: new THREE.CapsuleGeometry(0.02, 0.06, 4, 8), color: palette.eye, position: [0, 0.8, 0.33], rotation: [0, 0, Math.PI / 2] },
  ];
}

const badgePart = (): PartSpec => ({ geo: boxPart(0.11, 0.14, 0.03, radii.xs), color: palette.white, position: [0.14, 0.72, 0.31] });

// Role accessories, 1:1 from the deleted props.buildPerson switch.
function personParts(role: PersonRole): PartSpec[] {
  switch (role) {
    case 'nurse':
      return [...beanParts(palette.mint), badgePart()];
    case 'doctor':
      return [
        ...beanParts(palette.white),
        // stethoscope: half torus flipped over x
        { geo: tor(0.2, 0.028, 12, 32, Math.PI), color: palette.eye, position: [0, 0.86, 0.26], rotation: [Math.PI, 0, 0] },
      ];
    case 'surgeon':
      return [
        ...beanParts(palette.sage),
        { geo: cyl(0.24, 0.28, 0.14, 18), color: palette.mint, position: [0, 1.06, 0] },
        { geo: boxPart(0.26, 0.14, 0.05, radii.xs), color: palette.white, position: [0, 0.8, 0.31] },
      ];
    case 'patient':
      return beanParts(palette.coral);
    case 'child':
      return beanParts(palette.honey); // CHILD_SCALE baked at merge time
    case 'visitor':
      return beanParts(palette.skin);
    case 'labtech':
      return [
        ...beanParts(palette.white),
        badgePart(),
        { geo: tor(0.07, 0.016, 12, 32, Math.PI * 2), color: palette.eye, position: [-0.105, 0.92, 0.3] },
        { geo: tor(0.07, 0.016, 12, 32, Math.PI * 2), color: palette.eye, position: [0.105, 0.92, 0.3] },
      ];
    case 'paramedic':
      return [...beanParts(palette.coralSoft), { geo: boxPart(0.4, 0.12, 0.06, radii.xs), color: palette.white, position: [0, 0.66, 0.29] }];
  }
}

// Merge one role's parts into a single indexed BufferGeometry with a
// per-vertex color attribute baking the part colors (working-color-space,
// exactly what the per-part clayMat colors resolved to). RoundedBox parts
// are non-indexed; give them a trivial sequential index (same trick as
// staticBatch.ensureIndexed) so the merge stays indexed and small.
export function mergedPersonGeometry(role: PersonRole): THREE.BufferGeometry {
  const color = new THREE.Color();
  const parts = personParts(role).map((part) => {
    // cached geometries are shared — never mutate them in place
    const g = part.geo.clone();
    if (g.index === null) {
      const n = g.attributes.position.count;
      const index = new (n > 65535 ? Uint32Array : Uint16Array)(n);
      for (let i = 0; i < n; i++) index[i] = i;
      g.setIndex(new THREE.BufferAttribute(index, 1));
    }
    const m = new THREE.Matrix4().compose(
      new THREE.Vector3(...part.position),
      new THREE.Quaternion().setFromEuler(new THREE.Euler(...(part.rotation ?? [0, 0, 0]))),
      new THREE.Vector3(1, 1, 1),
    );
    g.applyMatrix4(m);
    color.setHex(part.color);
    const count = g.attributes.position.count;
    const colors = new Float32Array(count * 3);
    for (let i = 0; i < count; i++) {
      colors[i * 3] = color.r;
      colors[i * 3 + 1] = color.g;
      colors[i * 3 + 2] = color.b;
    }
    g.setAttribute('color', new THREE.BufferAttribute(colors, 3));
    return g;
  });
  const merged = mergeGeometries(parts, false);
  if (!merged) throw new Error(`agentMeshes: failed to merge person geometry for role "${role}"`);
  if (role === 'child') merged.scale(CHILD_SCALE, CHILD_SCALE, CHILD_SCALE);
  merged.computeBoundingSphere();
  return merged;
}

// ── instanced meshes + storage-buffer animation ─────────────────────────

export type AgentSlot = {
  // Final smoothed values for this frame; the shader adds the waddle/breathe
  // on top. roll is the CPU-decayed rotation.z, yaw the lerpAngle-smoothed one.
  set(x: number, z: number, yLift: number, yaw: number, walking: boolean, roll: number): void;
};

export type AgentInstances = {
  meshes: THREE.InstancedMesh[];
  // phase is the global agent index (the old la.idx driving sin(t*9+idx)).
  add(role: PersonRole, phase: number): AgentSlot;
  // Per-frame after all slot writes: advance the time uniform + upload buffers.
  update(t: number): void;
};

type RoleBucket = {
  mesh: THREE.InstancedMesh;
  a: Float32Array; // vec4 per agent: posX, posZ, yLift, yawSmooth
  b: Float32Array; // vec4 per agent: walkFlag, phaseSeed, roll, unused
  attrA: THREE.BufferAttribute;
  attrB: THREE.BufferAttribute;
  used: number;
  capacity: number;
};

export function createAgentInstances(counts: Partial<Record<PersonRole, number>>): AgentInstances {
  const timeU = uniform(0);
  const buckets = new Map<PersonRole, RoleBucket>();
  const meshes: THREE.InstancedMesh[] = [];

  for (const [role, capacity] of Object.entries(counts) as Array<[PersonRole, number]>) {
    if (!capacity) continue;
    const a = new Float32Array(capacity * 4);
    const b = new Float32Array(capacity * 4);
    const bufA = instancedArray(a, 'vec4');
    const bufB = instancedArray(b, 'vec4');

    // Clay recipe from designTokens; diffuse from the baked vertex colors,
    // sheenColor = color lerped 50% to white — mirrors staticBatch.ts.
    const material = new THREE.MeshPhysicalNodeMaterial({
      color: 0xffffff,
      roughness: clay.roughness,
      metalness: clay.metalness,
      vertexColors: true,
    });
    material.sheenRoughness = clay.sheenRoughness;
    material.sheenNode = mix(attribute('color', 'vec3'), vec3(1, 1, 1), 0.5).mul(float(clay.sheen));

    // Vertex animation — exact port of the old per-Object3D animate loop.
    // Object3D composed local = T * R(yaw) * R(roll) * S(squash), so:
    // squash Y, roll around Z, yaw around Y, translate to (posX, yLift, posZ).
    const A = bufA.toAttribute();
    const B = bufB.toAttribute();
    const walkSquash = abs(sin(timeU.mul(9.0).add(B.y))).mul(0.025).add(1.0);
    const dwellSquash = sin(timeU.mul(2.2).add(B.y.mul(0.9))).mul(0.012).add(1.0);
    const squash = mix(dwellSquash, walkSquash, B.x);
    const cr = cos(B.z);
    const sr = sin(B.z);
    const cy = cos(A.w);
    const sy = sin(A.w);
    const p = positionLocal;
    const y1 = p.y.mul(squash);
    const x2 = p.x.mul(cr).sub(y1.mul(sr));
    const y2 = p.x.mul(sr).add(y1.mul(cr));
    const x3 = x2.mul(cy).add(p.z.mul(sy));
    const z3 = p.z.mul(cy).sub(x2.mul(sy));
    material.positionNode = vec3(x3.add(A.x), y2.add(A.z), z3.add(A.y));
    // rotate normals with the same roll+yaw (squash's ±2.5% shear is negligible)
    const n = normalLocal;
    const nx2 = n.x.mul(cr).sub(n.y.mul(sr));
    const ny2 = n.x.mul(sr).add(n.y.mul(cr));
    const nx3 = nx2.mul(cy).add(n.z.mul(sy));
    const nz3 = n.z.mul(cy).sub(nx2.mul(sy));
    material.normalNode = transformNormalToView(vec3(nx3, ny2, nz3));

    const mesh = new THREE.InstancedMesh(mergedPersonGeometry(role), material, capacity);
    mesh.name = `ksw-agents-${role}`;
    mesh.castShadow = true;
    mesh.receiveShadow = true;
    // instances move every frame; per-instance culling comes in Slice D
    mesh.frustumCulled = false;
    // the buffers carry the full transform — instanceMatrix stays identity
    const identity = new THREE.Matrix4();
    for (let i = 0; i < capacity; i++) mesh.setMatrixAt(i, identity);

    buckets.set(role, {
      mesh,
      a,
      b,
      attrA: bufA.value as THREE.BufferAttribute,
      attrB: bufB.value as THREE.BufferAttribute,
      used: 0,
      capacity,
    });
    meshes.push(mesh);
  }

  return {
    meshes,
    add(role, phase) {
      const bucket = buckets.get(role);
      if (!bucket) throw new Error(`agentMeshes: no instance bucket for role "${role}"`);
      if (bucket.used >= bucket.capacity) throw new Error(`agentMeshes: bucket "${role}" over capacity (${bucket.capacity})`);
      const i = bucket.used++;
      const { a, b } = bucket;
      b[i * 4 + 1] = phase;
      return {
        set(x, z, yLift, yaw, walking, roll) {
          a[i * 4] = x;
          a[i * 4 + 1] = z;
          a[i * 4 + 2] = yLift;
          a[i * 4 + 3] = yaw;
          b[i * 4] = walking ? 1 : 0;
          b[i * 4 + 2] = roll;
        },
      };
    },
    update(t) {
      timeU.value = t;
      for (const bucket of buckets.values()) {
        bucket.attrA.needsUpdate = true;
        bucket.attrB.needsUpdate = true;
      }
    },
  };
}
