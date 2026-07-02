// src/diorama/ksw/geo/cityMassing.ts
// Renders the baked swisstopo LoD2 city as clay massing: every wall surface
// of every building merged into ONE mesh, every roof surface into another —
// two draw calls for ~800 real buildings, real roof shapes included. Colors
// and material come from the existing clay tokens, so the city reads as the
// same handmade material as the hero hospital.
import * as THREE from 'three/webgpu';
import { kswPalette, palette } from '../../designTokens';
import { clayMat } from '../props';
import type { BakedBuilding, BakedMesh } from './geoData';

function mergeBaked(parts: BakedMesh[]): THREE.BufferGeometry {
  let vtx = 0;
  let tri = 0;
  for (const p of parts) {
    vtx += p.pos.length / 3;
    tri += p.idx.length;
  }
  const positions = new Float32Array(vtx * 3);
  const indices = vtx > 65535 ? new Uint32Array(tri) : new Uint16Array(tri);
  let vo = 0;
  let io = 0;
  for (const p of parts) {
    const base = vo / 3;
    for (let i = 0; i < p.pos.length; i++) positions[vo + i] = p.pos[i] / 100; // cm → m
    for (let i = 0; i < p.idx.length; i++) indices[io + i] = base + p.idx[i];
    vo += p.pos.length;
    io += p.idx.length;
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  geo.setIndex(new THREE.BufferAttribute(indices, 1));
  geo.computeVertexNormals();
  return geo;
}

export function buildCityMassing(buildings: BakedBuilding[]): THREE.Group {
  const group = new THREE.Group();
  group.name = 'cityMassing';

  const make = (name: string, parts: BakedMesh[], color: number): THREE.Mesh => {
    const mesh = new THREE.Mesh(mergeBaked(parts), clayMat(color));
    mesh.name = name;
    mesh.castShadow = true;
    mesh.receiveShadow = true;
    group.add(mesh);
    return mesh;
  };

  make('cityWalls', buildings.map((b) => b.wall), palette.creamBase);
  make('cityRoofs', buildings.map((b) => b.roof), kswPalette.roofClay);
  return group;
}
