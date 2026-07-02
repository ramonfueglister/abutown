// Instanced bean people for the KSW diorama (Slices C+D of the 10k-perf
// design). One merged, vertex-colored geometry + InstancedMeshes per role
// replace the per-agent THREE.Group from props.buildPerson. Per-agent state
// lives in GLOBAL storage buffers (TSL instancedArray, CPU-written) shared by
// every mesh via element(instanceIndex + roleOffset); the vertex stage applies
// everything main.ts used to do on Object3Ds: walk/breathing squash, waddle
// roll, yaw and the (posX, yLift, posZ) translation. The smoothing state
// (eased y-lift, lerped yaw, decaying roll) stays CPU-side in flat slots.
//
// Crowd mode (agent count > kswAgents.crowdThreshold, Slice D):
// - two meshes per role (LOD0 bean / LOD1 capsule+head) share the buffers;
// - a TSL compute pass classifies every agent per frame into a flag buffer:
//   0 = LOD0, 1 = LOD1, 2 = culled (outside frustum+margin or too far);
// - the vertex stage collapses instances whose flag doesn't match the mesh's
//   LOD to scale 0 (zero-area triangles) — draw count stays 2 per role;
// - scale-0 still pays vertex shading, and the LOD0 bean is ~15x the LOD1
//   vertex count — at 10k that alone is >20M submitted triangles. So LOD0
//   meshes draw through a per-role slot map: the CPU (which owns the agent
//   positions anyway) collects the slots inside lodDistance+frustum with a
//   2 m slack margin, writes them into a uint slot-map buffer and bounds
//   mesh.count. The GPU flag stays authoritative per instance (the CPU list
//   is a strict superset), the LOD1/blob meshes still draw all instances;
// - agents stop casting real shadows; a single InstancedMesh of soft dark
//   ground discs (blob shadows) driven by the same buffers stands in.
// At or below the threshold nothing of this exists: one shadow-casting LOD0
// mesh per role, exactly the pre-crowd look.
//
// Look contract: identical bean proportions, part colors, role accessories
// and animation amplitudes/frequencies as the deleted buildPerson +
// per-agent animate loop.

import * as THREE from 'three/webgpu';
import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';
import {
  Fn,
  abs,
  attribute,
  cos,
  float,
  instanceIndex,
  instancedArray,
  mix,
  normalLocal,
  positionLocal,
  select,
  sin,
  smoothstep,
  transformNormalToView,
  uint,
  uniform,
  vec3,
} from 'three/tsl';
import { clay, kswAgents, palette, radii } from '../designTokens';
import { claySheenNode } from './clayNodes';
import type { PersonRole } from './floorPlan';
import { cyl, ensureSequentialIndex, roundedBox, sph, tor } from './geometryCache';

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

// The bean's body color per role — single source for the LOD0 bean body
// (via personParts) and the LOD1 capsule+head.
const roleBodyColor: Record<PersonRole, number> = {
  nurse: palette.mint,
  doctor: palette.white,
  surgeon: palette.sage,
  patient: palette.coral,
  child: palette.honey,
  visitor: palette.skin,
  labtech: palette.white,
  paramedic: palette.coralSoft,
};

// Role accessories, 1:1 from the deleted props.buildPerson switch. Body
// color always comes from roleBodyColor.
function personParts(role: PersonRole): PartSpec[] {
  const bean = beanParts(roleBodyColor[role]);
  switch (role) {
    case 'nurse':
      return [...bean, badgePart()];
    case 'doctor':
      return [
        ...bean,
        // stethoscope: half torus flipped over x
        { geo: tor(0.2, 0.028, 12, 32, Math.PI), color: palette.eye, position: [0, 0.86, 0.26], rotation: [Math.PI, 0, 0] },
      ];
    case 'surgeon':
      return [
        ...bean,
        { geo: cyl(0.24, 0.28, 0.14, 18), color: palette.mint, position: [0, 1.06, 0] },
        { geo: boxPart(0.26, 0.14, 0.05, radii.xs), color: palette.white, position: [0, 0.8, 0.31] },
      ];
    case 'patient':
      return bean;
    case 'child':
      return bean; // CHILD_SCALE baked at merge time
    case 'visitor':
      return bean;
    case 'labtech':
      return [
        ...bean,
        badgePart(),
        { geo: tor(0.07, 0.016, 12, 32, Math.PI * 2), color: palette.eye, position: [-0.105, 0.92, 0.3] },
        { geo: tor(0.07, 0.016, 12, 32, Math.PI * 2), color: palette.eye, position: [0.105, 0.92, 0.3] },
      ];
    case 'paramedic':
      return [...bean, { geo: boxPart(0.4, 0.12, 0.06, radii.xs), color: palette.white, position: [0, 0.66, 0.29] }];
  }
}

// Merge parts into a single indexed BufferGeometry with a per-vertex color
// attribute baking the part colors (working-color-space, exactly what the
// per-part clayMat colors resolved to). RoundedBox parts are non-indexed;
// give them a trivial sequential index (geometryCache.ensureSequentialIndex)
// so the merge stays indexed and small.
function mergeParts(parts: PartSpec[], label: string): THREE.BufferGeometry {
  const color = new THREE.Color();
  const prepared = parts.map((part) => {
    // cached geometries are shared — never mutate them in place
    const g = part.geo.clone();
    ensureSequentialIndex(g);
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
  const merged = mergeGeometries(prepared, false);
  if (!merged) throw new Error(`agentMeshes: failed to merge person geometry for "${label}"`);
  merged.computeBoundingSphere();
  return merged;
}

export function mergedPersonGeometry(role: PersonRole): THREE.BufferGeometry {
  const merged = mergeParts(personParts(role), role);
  if (role === 'child') {
    merged.scale(CHILD_SCALE, CHILD_SCALE, CHILD_SCALE);
    merged.computeBoundingSphere();
  }
  return merged;
}

// LOD1 (Slice D): low-poly capsule body + head sphere in the role's body
// color. Same overall height as the LOD0 bean (top ≈ 1.235 m before child
// scaling) so distant silhouettes match; a tiny fraction of the vertices.
export function lodPersonGeometry(role: PersonRole): THREE.BufferGeometry {
  const body = roleBodyColor[role];
  const merged = mergeParts(
    [
      { geo: new THREE.CapsuleGeometry(0.3, 0.42, 1, 6), color: body, position: [0, 0.5, 0] },
      { geo: new THREE.SphereGeometry(0.235, 6, 4), color: body, position: [0, 1.0, 0] },
    ],
    `${role}-lod1`,
  );
  if (role === 'child') {
    merged.scale(CHILD_SCALE, CHILD_SCALE, CHILD_SCALE);
    merged.computeBoundingSphere();
  }
  return merged;
}

// ── instanced meshes + storage-buffer animation ─────────────────────────

export type AgentSlot = {
  // Final smoothed values for this frame; the shader adds the waddle/breathe
  // on top. roll is the CPU-decayed rotation.z, yaw the lerpAngle-smoothed one.
  set(x: number, z: number, yLift: number, yaw: number, walking: boolean, roll: number): void;
};

// The per-frame LOD work (crowd mode only): frame(camera) refreshes the
// classification uniforms + compacts the LOD0 slot maps, then `node` runs
// via renderer.compute() before rendering.
export type AgentLodPass = {
  node: Parameters<THREE.WebGPURenderer['compute']>[0];
  frame(camera: THREE.PerspectiveCamera): void;
};

export type AgentInstances = {
  meshes: THREE.InstancedMesh[];
  // phase is the global agent index (the old la.idx driving sin(t*9+idx)).
  add(role: PersonRole, phase: number): AgentSlot;
  // Per-frame after all slot writes: advance the time uniform + upload buffers.
  update(t: number): void;
  lod: AgentLodPass | null; // null below the crowd threshold
  // GI-probe rendering (crowd mode): the probe is a low-res 360° env capture
  // from the scene center — the main camera's LOD/frustum flags are wrong for
  // it. probeMode=1 shows LOD1 capsules + blob discs for ALL instances and
  // hides LOD0 entirely (correct and cheap everywhere); probeMode=0 is the
  // per-frame classified behavior. No-op below the crowd threshold (no flags).
  setProbeMode(on: boolean): void;
};

type RoleBucket = { offset: number; used: number; capacity: number };

export function createAgentInstances(
  counts: Partial<Record<PersonRole, number>>,
  opts: { crowd?: boolean } = {},
): AgentInstances {
  const crowd = opts.crowd === true;
  const entries = (Object.entries(counts) as Array<[PersonRole, number]>).filter(([, c]) => c > 0);
  const total = entries.reduce((sum, [, c]) => sum + c, 0);

  // Global buffers shared by every mesh (LOD0, LOD1, blobs):
  //   A: posX, posZ, yLift, yawSmooth   B: walkFlag, phaseSeed, roll, blobScale
  const a = new Float32Array(total * 4);
  const b = new Float32Array(total * 4);
  const bufA = instancedArray(a, 'vec4');
  const bufB = instancedArray(b, 'vec4');
  // 0 = LOD0, 1 = LOD1, 2 = culled — written by the compute pass each frame.
  const bufFlag = crowd ? instancedArray(new Uint32Array(total), 'uint') : null;
  const timeU = uniform(0);
  // 1 while rendering GI-probe faces (crowd mode): overrides the main-camera
  // classification — see AgentInstances.setProbeMode.
  const probeU = uniform(0, 'uint');
  const probeOn = probeU.equal(uint(1));
  const identity = new THREE.Matrix4();

  const buckets = new Map<PersonRole, RoleBucket>();
  const meshes: THREE.InstancedMesh[] = [];

  // `slot` is a uint node resolving to the global agent index — either
  // instanceIndex+roleOffset or a slot-map read for compacted LOD0. The two
  // aren't modellable under one @types/three r185 node type (same situation
  // as the PCSS block in main.ts) — runtime-typed.
  type TSLSlot = any; // eslint-disable-line @typescript-eslint/no-explicit-any

  const makeRoleMesh = (
    role: PersonRole,
    geometry: THREE.BufferGeometry,
    capacity: number,
    slot: TSLSlot,
    lodIndex: 0 | 1,
  ): THREE.InstancedMesh => {
    // Clay recipe from designTokens; diffuse from the baked vertex colors,
    // sheen via the shared clayNodes.claySheenNode (same as staticBatch.ts).
    const material = new THREE.MeshPhysicalNodeMaterial({
      color: palette.trueWhite,
      roughness: clay.roughness,
      metalness: clay.metalness,
      vertexColors: true,
    });
    material.sheenRoughness = clay.sheenRoughness;
    material.sheenNode = claySheenNode(attribute('color', 'vec3'));

    // Vertex animation — exact port of the old per-Object3D animate loop.
    // Object3D composed local = T * R(yaw) * R(roll) * S(squash), so:
    // squash Y, roll around Z, yaw around Y, translate to (posX, yLift, posZ).
    // Normative spec + executable reference: personTransformRef.ts
    // (personInstanceMatrix) — the TSL below hand-expands that matrix; any
    // change must be mirrored there (the TSL itself can't run under vitest).
    const A = bufA.element(slot);
    const B = bufB.element(slot);
    const walkSquash = abs(sin(timeU.mul(9.0).add(B.y))).mul(0.025).add(1.0);
    const dwellSquash = sin(timeU.mul(2.2).add(B.y.mul(0.9))).mul(0.012).add(1.0);
    const squash = mix(dwellSquash, walkSquash, B.x);
    const cr = cos(B.z);
    const sr = sin(B.z);
    const cy = cos(A.w);
    const sy = sin(A.w);
    // Crowd mode: an instance whose flag doesn't match this mesh's LOD
    // collapses to scale 0 — zero-area triangles, nothing rasterizes.
    // Probe mode overrides the flag: LOD0 hides everything, LOD1 shows
    // everything (the probe capture must not inherit main-camera culling).
    const flagVis = bufFlag ? select(bufFlag.element(slot).equal(uint(lodIndex)), float(1), float(0)) : float(1);
    const vis = bufFlag ? select(probeOn, float(lodIndex), flagVis) : flagVis;
    const p = vec3(positionLocal).mul(vis);
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

    const mesh = new THREE.InstancedMesh(geometry, material, capacity);
    mesh.name = lodIndex === 0 ? `ksw-agents-${role}` : `ksw-agents-${role}-lod1`;
    // crowds are too many real casters for the PCSS cascade — blob shadows take over
    mesh.castShadow = !crowd;
    mesh.receiveShadow = true;
    // instances move every frame; visibility is per-instance in the shader
    mesh.frustumCulled = false;
    // the buffers carry the full transform — instanceMatrix stays identity
    for (let i = 0; i < capacity; i++) mesh.setMatrixAt(i, identity);
    return mesh;
  };

  // Per-role LOD0 compaction pools (crowd mode) — filled every frame by
  // AgentLodPass.frame() from the CPU-side positions in `a`.
  type Lod0Pool = { mesh: THREE.InstancedMesh; map: Uint32Array; attr: THREE.BufferAttribute; offset: number; capacity: number };
  const lod0Pools: Lod0Pool[] = [];

  let offset = 0;
  for (const [role, capacity] of entries) {
    buckets.set(role, { offset, used: 0, capacity });
    const directSlot = instanceIndex.add(uint(offset));
    if (!crowd) {
      meshes.push(makeRoleMesh(role, mergedPersonGeometry(role), capacity, directSlot, 0));
    } else {
      const map = new Uint32Array(capacity);
      const mapBuf = instancedArray(map, 'uint');
      const mesh0 = makeRoleMesh(role, mergedPersonGeometry(role), capacity, mapBuf.element(instanceIndex), 0);
      mesh0.count = 0;
      lod0Pools.push({ mesh: mesh0, map, attr: mapBuf.value as THREE.BufferAttribute, offset, capacity });
      meshes.push(mesh0);
      meshes.push(makeRoleMesh(role, lodPersonGeometry(role), capacity, directSlot, 1));
    }
    offset += capacity;
  }

  // Blob shadows (crowd mode): one InstancedMesh of soft dark ground discs
  // under all agents, driven by the same buffers. Slightly y-lifted above the
  // floor/slab tops (no z-fight), soft radial falloff, culled with flag 2.
  if (bufFlag) {
    const blobGeo = new THREE.CircleGeometry(kswAgents.blob.radius, 20);
    blobGeo.rotateX(-Math.PI / 2);
    const material = new THREE.MeshBasicNodeMaterial({ color: kswAgents.blob.color, transparent: true, depthWrite: false });
    const A = bufA.element(instanceIndex);
    const B = bufB.element(instanceIndex);
    // probe mode shows every disc (no main-camera culling in the env capture)
    const s = select(probeOn, B.w, select(bufFlag.element(instanceIndex).equal(uint(2)), float(0), B.w));
    material.positionNode = vec3(
      positionLocal.x.mul(s).add(A.x),
      positionLocal.y.add(A.z).add(float(kswAgents.blob.lift)),
      positionLocal.z.mul(s).add(A.y),
    );
    const edge = positionLocal.xz.length().div(float(kswAgents.blob.radius));
    material.opacityNode = float(kswAgents.blob.opacity).mul(float(1).sub(smoothstep(float(0.45), float(1), edge)));
    const blobs = new THREE.InstancedMesh(blobGeo, material, total);
    blobs.name = 'ksw-agents-blobs';
    blobs.castShadow = false;
    blobs.receiveShadow = false;
    blobs.frustumCulled = false;
    for (let i = 0; i < total; i++) blobs.setMatrixAt(i, identity);
    meshes.push(blobs);
  }

  // Per-instance LOD + frustum classification (crowd mode): one dispatch over
  // all agents per frame. Frustum planes come from the camera on the CPU;
  // an instance is culled when its center (mid-body) is more than
  // frustumMargin outside any plane or beyond cullDistance.
  let lod: AgentLodPass | null = null;
  if (bufFlag) {
    const camPosU = uniform(new THREE.Vector3());
    const planesU = Array.from({ length: 6 }, () => uniform(new THREE.Vector4()));
    const classify = Fn(() => {
      const d = bufA.element(instanceIndex);
      const pos = vec3(d.x, d.z.add(0.65), d.y);
      const distCam = pos.sub(camPosU).length();
      let visible = distCam.lessThan(float(kswAgents.cullDistance));
      for (const plane of planesU) {
        visible = visible.and(plane.xyz.dot(pos).add(plane.w).greaterThan(float(-kswAgents.frustumMargin)));
      }
      const flag = select(visible, select(distCam.greaterThan(float(kswAgents.lodDistance)), uint(1), uint(0)), uint(2));
      bufFlag.element(instanceIndex).assign(flag);
    });
    const projView = new THREE.Matrix4();
    const frustum = new THREE.Frustum();
    // CPU compaction slack: strictly looser than the GPU thresholds, so the
    // slot maps are always a superset of what the GPU classifies as LOD0.
    const maxLod0Dist = kswAgents.lodDistance + 2;
    const planeSlack = -(kswAgents.frustumMargin + 2);
    lod = {
      node: classify().compute(total),
      frame(camera) {
        camera.updateMatrixWorld();
        camera.getWorldPosition(camPosU.value as THREE.Vector3);
        projView.multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse);
        frustum.setFromProjectionMatrix(projView);
        for (let i = 0; i < 6; i++) {
          const pl = frustum.planes[i];
          (planesU[i].value as THREE.Vector4).set(pl.normal.x, pl.normal.y, pl.normal.z, pl.constant);
        }
        // LOD0 slot maps: collect the near, in-frustum agents per role.
        const cam = camPosU.value as THREE.Vector3;
        const planes = frustum.planes;
        for (const pool of lod0Pools) {
          let n = 0;
          const end = pool.offset + pool.capacity;
          for (let i = pool.offset; i < end; i++) {
            const px = a[i * 4];
            const py = a[i * 4 + 2] + 0.65; // same mid-body point as the GPU pass
            const pz = a[i * 4 + 1];
            const dx = px - cam.x;
            const dy = py - cam.y;
            const dz = pz - cam.z;
            if (dx * dx + dy * dy + dz * dz > maxLod0Dist * maxLod0Dist) continue;
            let inside = true;
            for (let k = 0; k < 6; k++) {
              const pl = planes[k];
              if (pl.normal.x * px + pl.normal.y * py + pl.normal.z * pz + pl.constant < planeSlack) {
                inside = false;
                break;
              }
            }
            if (inside) pool.map[n++] = i;
          }
          pool.mesh.count = n;
          pool.attr.needsUpdate = true;
        }
      },
    };
  }

  const attrA = bufA.value as THREE.BufferAttribute;
  const attrB = bufB.value as THREE.BufferAttribute;
  return {
    meshes,
    lod,
    add(role, phase) {
      const bucket = buckets.get(role);
      if (!bucket) throw new Error(`agentMeshes: no instance bucket for role "${role}"`);
      if (bucket.used >= bucket.capacity) throw new Error(`agentMeshes: bucket "${role}" over capacity (${bucket.capacity})`);
      const i = bucket.offset + bucket.used++;
      b[i * 4 + 1] = phase;
      b[i * 4 + 3] = role === 'child' ? CHILD_SCALE : 1; // blob disc scale
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
      attrA.needsUpdate = true;
      attrB.needsUpdate = true;
    },
    setProbeMode(on) {
      if (bufFlag) probeU.value = on ? 1 : 0; // no flags below the crowd threshold — no-op
    },
  };
}
