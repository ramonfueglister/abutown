// src/diorama/ksw/geo/treeImpostors.ts
// Far-field tree LOD: octahedral impostors. At boot we bake, once, a hemi-
// octahedral view atlas per archetype (OCT_GRID×OCT_GRID orthographic snapshots
// spanning the upper view hemisphere) into one power-of-two RenderTarget. Distant
// trees then draw as a SINGLE InstancedMesh of camera-facing quads that sample
// the atlas cell nearest the current view direction — thousands of full crown
// meshes collapse to one cheap draw with silhouettes that still turn with the
// camera. The impostors are pre-lit by the bake (the bake used the real clay
// crown colors under the ambient bake clear), so the draw material is a
// MeshBasicNodeMaterial — no live lighting, no wind (far field, imperceptible).
//
// Coordinate / convention notes (self-review anchors):
//  - hemiOctUv / viewDirFor are pure and round-trip exactly (unit-tested). They
//    speak GRID-CELL space: (0..OCT_GRID-1)², zenith → center, horizon → border.
//  - Per-instance transform is carried ENTIRELY in attributes, not the
//    instanceMatrix (which stays identity — three applies it on top of
//    positionNode, so a matrix scale would also scale the node's translation;
//    same identity-matrix pattern as agentMeshes.ts): `aCenter` = world
//    (x,0,z), `aSize` = world (2·r, h·squash). Those are exactly the full
//    mesh's world extents under treeLayer's compose (geometry reach
//    ±crownRadius × y∈[0,1], scale (r/crownRadius, h·squash)), and the bake
//    frames precisely that envelope edge-to-edge — silhouettes line up across
//    the LOD swap.
//  - Atlas UV: cell (cx,cy) occupies texels [cx·CELL_PX .. +CELL_PX) in X and
//    [cy·CELL_PX .. +CELL_PX) in Y, TOP-DOWN in atlas-layout space (row 0 = top).
//    WebGPU render targets are bottom-up in NDC, so when baking we place cell
//    (cx,cy)'s VIEWPORT at y = height − (cy+1)·CELL_PX (flip), and the sampler
//    UV in the shader flips back with v = 1 − rowV. Net: what atlasLayout calls
//    "row 0" reads as row 0 in the shader regardless of the target's Y origin.

import * as THREE from 'three/webgpu';
import {
  abs,
  atan,
  attribute,
  cameraPosition,
  cos,
  float,
  floor,
  instancedBufferAttribute,
  mix,
  positionLocal,
  select,
  sin,
  smoothstep,
  step,
  texture,
  uv,
  vec2,
  vec3,
  vec4,
} from 'three/tsl';
import { kswCity, kswCityStyle } from '../../designTokens';
import type { TreeArchetype } from './treeArchetypes';
import type { TreeInstance } from './treeLayer';

export const OCT_GRID = 4; // 4×4 hemi-octahedral views per archetype
export const CELL_PX = 128;

// The impostor collapses to full-detail geometry inside this radius. The full
// mesh already appears at NEAR_TREE_DIST = nearR × 1.1 (treeLayer), so the two
// bands overlap by design — nothing gaps during the handoff.
const NEAR_COLLAPSE = kswCityStyle.lod.nearR;

// ── pure mapping (unit-tested) ────────────────────────────────────────────

// Atlas block layout: archCount archetypes, each an OCT_GRID×OCT_GRID grid of
// CELL_PX cells. We pack the archetype BLOCKS into a near-square grid, then
// round the pixel dimensions UP to the next power of two (WebGPU-friendly
// mipmaps + the test's log2 integrality check).
// `cols`/`rows` count CELLS (each archetype contributes an OCT_GRID×OCT_GRID
// block of cells); `blockCols` is the archetype-block column count needed to
// map a linear archetype id → (block col,row). width/height are the power-of-
// two pixel dimensions.
export function atlasLayout(
  archCount: number,
): { cols: number; rows: number; blockCols: number; width: number; height: number } {
  const blockCols = Math.ceil(Math.sqrt(archCount));
  const blockRows = Math.ceil(archCount / blockCols);
  const cols = blockCols * OCT_GRID;
  const rows = blockRows * OCT_GRID;
  const pow2 = (n: number) => 2 ** Math.ceil(Math.log2(n));
  return { cols, rows, blockCols, width: pow2(cols * CELL_PX), height: pow2(rows * CELL_PX) };
}

// View direction → grid-cell coordinates in (0..OCT_GRID-1)² space.
// Standard hemi-oct: project the upper-hemisphere dir onto the octahedron's
// top face (a diamond |x|+|z| ≤ 1 in the y≥0 half), rotate 45° to fill the
// unit square, then scale so cell CENTERS land on integer coords.
export function hemiOctUv(dir: THREE.Vector3): { u: number; v: number } {
  const x = dir.x;
  const y = Math.max(0, dir.y);
  const z = dir.z;
  const l1 = Math.abs(x) + y + Math.abs(z) || 1; // guard the zero vector
  const px = x / l1;
  const pz = z / l1;
  // rotate 45°: diamond |px|+|pz| ≤ 1 → unit square [0,1]²
  const uSquare = (px + pz) * 0.5 + 0.5;
  const vSquare = (pz - px) * 0.5 + 0.5;
  // cell centers at integer coords 0..OCT_GRID-1
  return { u: uSquare * (OCT_GRID - 1), v: vSquare * (OCT_GRID - 1) };
}

// Inverse of hemiOctUv for a cell center: grid cell (ix,iy) → the unit bake
// camera direction (upper hemisphere, y ≥ 0) that snapshot views the tree from.
export function viewDirFor(ix: number, iy: number): THREE.Vector3 {
  const uSquare = ix / (OCT_GRID - 1);
  const vSquare = iy / (OCT_GRID - 1);
  // invert the 45° rotation
  const px = (uSquare - vSquare); // = ((2u-1) - (2v-1)) / 2 ... derived below
  const pz = (uSquare + vSquare - 1);
  // Above: uSquare = (px+pz)/2 + .5, vSquare = (pz-px)/2 + .5
  //   uSquare - vSquare = px ;  uSquare + vSquare - 1 = pz
  // reconstruct y from the L1 = 1 octahedron constraint: |px|+y+|pz| = 1
  const y = 1 - Math.abs(px) - Math.abs(pz);
  return new THREE.Vector3(px, y, pz).normalize();
}

// ── boot bake ─────────────────────────────────────────────────────────────

const baseGreen = new THREE.Color(kswCity.treeGreen);
const baseWood = new THREE.Color(kswCity.woodGreen);
const TRUNK_R = ((kswCity.treeTrunk >> 16) & 0xff) / 255;
const TRUNK_G = ((kswCity.treeTrunk >> 8) & 0xff) / 255;
const TRUNK_B = (kswCity.treeTrunk & 0xff) / 255;

// Node values are runtime-typed `any` (same un-modellable-node situation
// treeLayer / agentMeshes document).
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type TSLNode = any;

// Bake material for one archetype: the SAME two-tone crown gradient as
// treeLayer.treeMaterial, but pre-lit (MeshBasic) and WITHOUT the per-instance
// tint (tint is multiplied onto the sprite at draw time). conifer archetypes
// bake off woodGreen, broadleaf off treeGreen — matching the full mesh's base.
function bakeMaterial(arch: TreeArchetype): THREE.MeshBasicNodeMaterial {
  const base = arch.family === 'conic' || arch.family === 'slender' ? baseWood : baseGreen;
  const mat = new THREE.MeshBasicNodeMaterial({ transparent: true });
  const aPuff: TSLNode = attribute('aPuff', 'vec4');
  const isWood = aPuff.w.lessThan(float(0));
  // crown gradient identical to treeLayer: mix(0.82, 1.12, smoothstep(crownBaseY,1,y))
  const grad = smoothstep(float(arch.crownBaseY), float(1), positionLocal.y);
  const gradient = mix(float(0.82), float(1.12), grad);
  const crownColor = vec3(float(base.r), float(base.g), float(base.b)).mul(gradient);
  const trunkColor = vec3(float(TRUNK_R), float(TRUNK_G), float(TRUNK_B));
  mat.colorNode = vec4(select(isWood, trunkColor, crownColor), float(1));
  return mat;
}

// Bake the whole atlas. Returns rt.texture. Does NOT permanently disturb
// renderer state (render target / viewport / scissor / clear color restored).
export async function bakeImpostorAtlas(
  renderer: THREE.WebGPURenderer,
  archetypes: TreeArchetype[],
): Promise<THREE.Texture> {
  const layout = atlasLayout(archetypes.length);
  const rt = new THREE.RenderTarget(layout.width, layout.height, {
    depthBuffer: true,
    format: THREE.RGBAFormat,
    type: THREE.UnsignedByteType,
    minFilter: THREE.LinearFilter,
    magFilter: THREE.LinearFilter,
  });

  // Save renderer state we touch.
  const prevRT = renderer.getRenderTarget();
  const prevClear = new THREE.Color();
  renderer.getClearColor(prevClear);
  const prevClearAlpha = renderer.getClearAlpha();
  const prevScissorTest = renderer.getScissorTest();
  const prevViewport = new THREE.Vector4();
  renderer.getViewport(prevViewport);
  const prevScissor = new THREE.Vector4();
  renderer.getScissor(prevScissor);

  const scene = new THREE.Scene();
  const cam = new THREE.OrthographicCamera();
  const meshHolder = new THREE.Object3D();
  scene.add(meshHolder);

  renderer.setRenderTarget(rt);
  renderer.setClearColor(0x000000, 0); // transparent — crown alpha carves the silhouette
  renderer.setScissorTest(true);
  // Clear the whole target once (transparent).
  renderer.setViewport(0, 0, layout.width, layout.height);
  renderer.setScissor(0, 0, layout.width, layout.height);
  renderer.clear(true, true, true);

  const target = new THREE.Vector3(0, 0.5, 0); // unit tree spans y 0..1
  const blockPx = OCT_GRID * CELL_PX;

  for (let a = 0; a < archetypes.length; a++) {
    const arch = archetypes[a];
    const mesh = new THREE.Mesh(arch.geometry, bakeMaterial(arch));
    meshHolder.clear();
    meshHolder.add(mesh);

    const blockCol = a % layout.blockCols;
    const blockRow = Math.floor(a / layout.blockCols);

    for (let iy = 0; iy < OCT_GRID; iy++) {
      for (let ix = 0; ix < OCT_GRID; ix++) {
        const dir = viewDirFor(ix, iy);
        // camera sits along +dir looking at the tree center. dir is never
        // exactly (0,1,0) at any cell center, so lookAt's up=(0,1,0) is safe.
        cam.position.copy(target).addScaledVector(dir, 3);
        cam.up.set(0, 1, 0);
        cam.lookAt(target);
        // Frame the EXACT unit envelope, edge-to-edge — no margin, no square
        // floor: width 2·crownRadius (the tree's true horizontal reach) and
        // height 1 (y∈[0,1], target y=0.5 → ±0.5). The draw quad maps its uv
        // 0..1 onto the full cell and is sized to the SAME world box
        // (aSize = (2r, h·squash), see buildImpostorMesh), so the baked
        // silhouette lands exactly where the full mesh's does at the handoff.
        cam.left = -arch.crownRadius;
        cam.right = arch.crownRadius;
        cam.top = 0.5;
        cam.bottom = -0.5;
        // near/far safely enclose the unit envelope for all 16 view dirs: the
        // camera is 3 away from target, the geometry's bounding radius around
        // the target is < 1, so depths span [2, 4] ⊂ [0.01, 10].
        cam.near = 0.01;
        cam.far = 10;
        cam.updateProjectionMatrix();
        cam.updateMatrixWorld(true);

        // Cell pixel origin in atlas-layout (top-down) space:
        const cellXpx = blockCol * blockPx + ix * CELL_PX;
        const cellYpxTop = blockRow * blockPx + iy * CELL_PX;
        // WebGPU RT is bottom-up: flip Y so layout row 0 lands at the top.
        const vpY = layout.height - (cellYpxTop + CELL_PX);
        renderer.setViewport(cellXpx, vpY, CELL_PX, CELL_PX);
        renderer.setScissor(cellXpx, vpY, CELL_PX, CELL_PX);
        // eslint-disable-next-line no-await-in-loop
        await renderer.renderAsync(scene, cam);
      }
    }
    (mesh.material as THREE.Material).dispose();
  }

  // Restore renderer state.
  renderer.setRenderTarget(prevRT);
  renderer.setClearColor(prevClear, prevClearAlpha);
  renderer.setScissorTest(prevScissorTest);
  renderer.setViewport(prevViewport);
  renderer.setScissor(prevScissor);

  return rt.texture;
}

// ── impostor mesh ───────────────────────────────────────────────────────

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Node = any;

// TSL hemi-oct: view dir (world, y≥0) → cell coord in (0..OCT_GRID-1)². Mirror
// of the CPU hemiOctUv above; must stay in lockstep.
const hemiOctUvNode = (dir: Node): Node => {
  const y = dir.y.max(float(0));
  const l1 = abs(dir.x).add(y).add(abs(dir.z)).max(float(1e-4));
  const px = dir.x.div(l1);
  const pz = dir.z.div(l1);
  const uSquare = px.add(pz).mul(float(0.5)).add(float(0.5));
  const vSquare = pz.sub(px).mul(float(0.5)).add(float(0.5));
  return vec2(uSquare.mul(float(OCT_GRID - 1)), vSquare.mul(float(OCT_GRID - 1)));
};

export function buildImpostorMesh(
  instances: readonly TreeInstance[],
  atlas: THREE.Texture,
  archCount: number,
): THREE.InstancedMesh {
  const layout = atlasLayout(archCount);
  // WebGPU zero-buffer rule: at least one instance of capacity.
  const cap = Math.max(1, instances.length);

  // Unit quad anchored at y 0..1 (matches the normalized tree envelope);
  // per-instance aSize (below) turns it into the world-sized billboard.
  const quad = new THREE.PlaneGeometry(1, 1);
  quad.translate(0, 0.5, 0); // y 0..1

  // Per-instance attributes.
  const centerArr = new Float32Array(cap * 3); // world (x,0,z)
  const archArr = new Float32Array(cap); // archetype id (float)
  const tintArr = new Float32Array(cap * 3); // per-instance tint
  const sizeArr = new Float32Array(cap * 2); // world (width, height) of the billboard
  const centerAttr = new THREE.InstancedBufferAttribute(centerArr, 3);
  const archAttr = new THREE.InstancedBufferAttribute(archArr, 1);
  const tintAttr = new THREE.InstancedBufferAttribute(tintArr, 3);
  const sizeAttr = new THREE.InstancedBufferAttribute(sizeArr, 2);
  // Runtime-typed nodes: @types/three r185 models these as opaque Node<string>
  // without swizzle/op methods (same situation treeLayer/agentMeshes document).
  const aCenter: Node = instancedBufferAttribute(centerAttr, 'vec3');
  const aArch: Node = instancedBufferAttribute(archAttr, 'float');
  const aTint: Node = instancedBufferAttribute(tintAttr, 'vec3');
  const aSize: Node = instancedBufferAttribute(sizeAttr, 'vec2');
  const camPos: Node = cameraPosition;
  const posLocal: Node = positionLocal;
  const quadUvN: Node = uv();

  const material = new THREE.MeshBasicNodeMaterial({ transparent: true, alphaTest: 0.5 });
  const atlasTex: Node = texture(atlas);

  // ── positionNode: cylindrical (yaw-only) billboard toward the camera ─────
  // Sizing scheme (must agree with the bake, which frames the exact unit
  // envelope edge-to-edge — width 2·crownRadius × height 1): the quad is a
  // shared unit plane (x∈[−½,½], y∈[0,1]) and the WORLD billboard box comes
  // from the per-instance aSize = (2·r, h·squash) — i.e. half-width r and
  // height h·squash, exactly the full mesh's world extents (its geometry
  // reaches ±crownRadius × y∈[0,1], scaled by (r/crownRadius, h·squash)). The
  // instanceMatrix stays IDENTITY (agentMeshes pattern): three applies it on
  // top of positionNode, so any non-identity scale would also scale the
  // aCenter translation — everything lives in the node instead.
  const toCam: Node = vec3(camPos.x.sub(aCenter.x), float(0), camPos.z.sub(aCenter.z));
  const yaw: Node = atan(toCam.x, toCam.z); // yaw about +y, +z toward camera
  const cy = cos(yaw);
  const sy = sin(yaw);
  // world-sized quad point before yaw: (x·width, y·height, 0)
  const p: Node = posLocal;
  const wx = p.x.mul(aSize.x);
  const wy = p.y.mul(aSize.y);
  const rx = wx.mul(cy); // plane z = 0, so the yaw rotation reduces to this
  const rz = wx.negate().mul(sy);

  // Near collapse: inside NEAR_COLLAPSE the full mesh takes over, so shrink the
  // impostor to zero (step is 0 below the radius, 1 above).
  const dist = length2Node(camPos, aCenter);
  const vis = step(float(NEAR_COLLAPSE), dist);

  material.positionNode = vec3(rx.mul(vis).add(aCenter.x), wy.mul(vis), rz.mul(vis).add(aCenter.z));

  // ── UV: pick the hemi-oct cell for this view dir, offset into the atlas ──
  // View dir = camera − instance center (normalized), upper hemisphere.
  const viewDir: Node = vec3(
    camPos.x.sub(aCenter.x),
    camPos.y.sub(aCenter.y),
    camPos.z.sub(aCenter.z),
  ).normalize();
  const cellCoord = hemiOctUvNode(viewDir); // (0..OCT_GRID-1)²
  const cx = floor(cellCoord.x.add(float(0.5))).clamp(0, OCT_GRID - 1);
  const cyCell = floor(cellCoord.y.add(float(0.5))).clamp(0, OCT_GRID - 1);

  // Atlas block for this archetype.
  const blockCol = aArch.mod(float(layout.blockCols));
  const blockRow = floor(aArch.div(float(layout.blockCols)));

  const blockPxU = float((OCT_GRID * CELL_PX) / layout.width);
  const blockPxV = float((OCT_GRID * CELL_PX) / layout.height);
  const cellU = float(CELL_PX / layout.width);
  const cellV = float(CELL_PX / layout.height);

  // quad uv ∈ [0,1]; map into the chosen cell. Row index runs TOP-DOWN in
  // layout space; the sampler's v origin is bottom-left, so flip: the cell we
  // stored at layout-row R reads at v = 1 − (its bottom edge). We baked with a
  // matching Y flip (see bakeImpostorAtlas), so here we simply flip the whole
  // atlas v: uvV = 1 − (rowOrigin + quadV·cellV).
  const baseU = blockCol.mul(blockPxU).add(cx.mul(cellU));
  const baseVtop = blockRow.mul(blockPxV).add(cyCell.mul(cellV)); // top edge, layout space
  const sampleU = baseU.add(quadUvN.x.mul(cellU));
  // flip v: layout-top-down → sampler-bottom-up
  const sampleV = float(1).sub(baseVtop.add(float(1).sub(quadUvN.y).mul(cellV)));

  const sampled: Node = atlasTex.sample(vec2(sampleU, sampleV));
  material.colorNode = sampled.rgb.mul(aTint);
  material.opacityNode = sampled.a;

  const mesh = new THREE.InstancedMesh(quad, material, cap);
  mesh.name = 'treeImpostors';
  mesh.castShadow = false;
  mesh.receiveShadow = false;
  mesh.frustumCulled = false; // always visible; near-collapse handles the near set

  // Fill per-instance data. The instanceMatrix stays identity — position and
  // sizing are fully carried by aCenter/aSize in the positionNode (see above);
  // aSize reproduces treeLayer's compose extents: world half-width r, world
  // height h·squash.
  const identity = new THREE.Matrix4();
  for (let i = 0; i < cap; i++) mesh.setMatrixAt(i, identity);
  for (let i = 0; i < instances.length; i++) {
    const inst = instances[i];
    sizeArr[i * 2] = inst.spec.r * 2; // world billboard width (half-width = r)
    sizeArr[i * 2 + 1] = inst.spec.h * inst.squash; // world billboard height
    centerArr[i * 3] = inst.spec.x;
    centerArr[i * 3 + 1] = 0;
    centerArr[i * 3 + 2] = inst.spec.z;
    archArr[i] = inst.archetype;
    tintArr[i * 3] = inst.tint.r;
    tintArr[i * 3 + 1] = inst.tint.g;
    tintArr[i * 3 + 2] = inst.tint.b;
  }
  mesh.count = instances.length;
  mesh.instanceMatrix.needsUpdate = true;
  return mesh;
}

// planar (xz) distance helper (TSL) between two vec3 nodes.
function length2Node(a: Node, b: Node): Node {
  const dx = a.x.sub(b.x);
  const dz = a.z.sub(b.z);
  return dx.mul(dx).add(dz.mul(dz)).sqrt();
}
