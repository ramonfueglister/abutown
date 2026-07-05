// src/diorama/ksw/geo/nature.ts
// The living layer of the real city: OSM parks/woods as soft green patches and
// the Eulach and ponds as calm water. Trees are a separate concern now — the
// archetype tree layer (buildTreeLayer / treeImpostors) owns them; buildNature
// no longer touches nature.trees.
import * as THREE from 'three/webgpu';
import { kswCity } from '../../designTokens';
import { clayMat } from '../props';
import type { CityNature, GreenArea } from './geoData';

function hash01(n: number): number {
  const s = Math.sin(n * 127.1 + 311.7) * 43758.5453;
  return s - Math.floor(s);
}

// triangulate flat rings (y = const) into one merged geometry
function flatAreas(rings: Array<{ ring: number[][]; color: THREE.Color }>, y: number): THREE.BufferGeometry {
  const positions: number[] = [];
  const colors: number[] = [];
  const indices: number[] = [];
  for (const { ring, color } of rings) {
    const pts = ring.length > 1 && ring[0][0] === ring[ring.length - 1][0] && ring[0][1] === ring[ring.length - 1][1]
      ? ring.slice(0, -1)
      : ring;
    if (pts.length < 3) continue;
    const contour = pts.map(([x, z]) => new THREE.Vector2(x, z));
    const tris = THREE.ShapeUtils.triangulateShape(contour, []);
    if (tris.length === 0) continue;
    const base = positions.length / 3;
    for (const [x, z] of pts) {
      positions.push(x, y, z);
      colors.push(color.r, color.g, color.b);
    }
    for (const t of tris.flat()) indices.push(base + t);
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(positions), 3));
  geo.setAttribute('color', new THREE.BufferAttribute(new Float32Array(colors), 3));
  geo.setIndex(
    positions.length / 3 > 65535
      ? new THREE.BufferAttribute(new Uint32Array(indices), 1)
      : new THREE.BufferAttribute(new Uint16Array(indices), 1),
  );
  geo.computeVertexNormals();
  return geo;
}

// river polylines as flat ribbons, appended into the same position/color soup
function riverRibbons(rivers: CityNature['rivers'], color: THREE.Color): Array<{ ring: number[][]; color: THREE.Color }> {
  const quads: Array<{ ring: number[][]; color: THREE.Color }> = [];
  for (const r of rivers) {
    for (let i = 0; i < r.pts.length - 1; i++) {
      const [x0, z0] = r.pts[i];
      const [x1, z1] = r.pts[i + 1];
      const dx = x1 - x0;
      const dz = z1 - z0;
      const len = Math.hypot(dx, dz);
      if (len < 0.05) continue;
      const hx = (-dz / len) * (r.width / 2);
      const hz = (dx / len) * (r.width / 2);
      quads.push({ ring: [[x0 + hx, z0 + hz], [x1 + hx, z1 + hz], [x1 - hx, z1 - hz], [x0 - hx, z0 - hz]], color });
    }
  }
  return quads;
}

export function vertexTintMat(base: number): THREE.MeshPhysicalMaterial {
  const m = clayMat(base).clone();
  m.vertexColors = true;
  m.color = new THREE.Color(0xffffff);
  return m;
}

export type NatureOptions = {
  // Kept for call-site symmetry with buildTreeLayer (which consumes it) — the
  // hero plate's rect. buildNature no longer builds trees, so it ignores this.
  excludeRect?: { x: number; z: number; w: number; d: number };
};

export function buildNature(nature: CityNature, _opts: NatureOptions = {}): THREE.Group {
  const group = new THREE.Group();
  group.name = 'cityNature';

  // green patches: parks lively, woods deeper — slight per-area tint variation
  const park = new THREE.Color(kswCity.parkGreen);
  const wood = new THREE.Color(kswCity.woodGreen);
  const greenRings = nature.greens.map((g: GreenArea, i: number) => {
    const base = g.kind === 'wood' || g.kind === 'forest' || g.kind === 'scrub' ? wood : park;
    const c = base.clone().offsetHSL((hash01(i) - 0.5) * 0.02, 0, (hash01(i * 1.7) - 0.5) * 0.06);
    return { ring: g.ring, color: c };
  });
  const greens = new THREE.Mesh(flatAreas(greenRings, kswCity.greenY), vertexTintMat(kswCity.parkGreen));
  greens.name = 'natureGreens';
  greens.receiveShadow = true;
  greens.castShadow = false;
  group.add(greens);

  // water: areas + river ribbons in one calm-blue mesh
  const waterColor = new THREE.Color(kswCity.water);
  const waterRings = [
    ...nature.waterAreas.map((w) => ({ ring: w.ring, color: waterColor })),
    ...riverRibbons(nature.rivers, waterColor),
  ];
  const water = new THREE.Mesh(flatAreas(waterRings, kswCity.waterY), vertexTintMat(kswCity.water));
  water.name = 'natureWater';
  water.receiveShadow = true;
  water.castShadow = false;
  group.add(water);

  return group;
}
