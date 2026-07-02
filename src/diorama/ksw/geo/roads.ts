// src/diorama/ksw/geo/roads.ts
// OSM ways as flat clay ribbons on the plate: one quad per polyline
// segment, width by road class, slightly lifted to avoid z-fighting the
// lawn. Deliberately no miter joins — at clay scale the tiny wedge gaps at
// bends read as handmade, and the merged geometry stays trivial.
import * as THREE from 'three/webgpu';
import { kswCity, kswPalette, palette } from '../../designTokens';
import { clayMat } from '../props';
import type { RoadPath } from './geoData';

function ribbonGeometry(paths: RoadPath[], y: number): THREE.BufferGeometry {
  const positions: number[] = [];
  const indices: number[] = [];
  for (const path of paths) {
    for (let i = 0; i < path.pts.length - 1; i++) {
      const [x0, z0] = path.pts[i];
      const [x1, z1] = path.pts[i + 1];
      const dx = x1 - x0;
      const dz = z1 - z0;
      const len = Math.hypot(dx, dz);
      if (len < 0.05) continue;
      const hx = (-dz / len) * (path.width / 2); // segment normal × half width
      const hz = (dx / len) * (path.width / 2);
      const base = positions.length / 3;
      positions.push(
        x0 + hx, y, z0 + hz, x0 - hx, y, z0 - hz,
        x1 + hx, y, z1 + hz, x1 - hx, y, z1 - hz,
      );
      indices.push(base, base + 2, base + 1, base + 1, base + 2, base + 3);
    }
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(positions), 3));
  geo.setIndex(positions.length / 3 > 65535 ? new THREE.BufferAttribute(new Uint32Array(indices), 1) : new THREE.BufferAttribute(new Uint16Array(indices), 1));
  geo.computeVertexNormals();
  return geo;
}

export function buildRoads(roads: RoadPath[], rails: RoadPath[]): THREE.Group {
  const group = new THREE.Group();
  group.name = 'cityRoads';

  const make = (name: string, paths: RoadPath[], color: number, y: number): void => {
    const mesh = new THREE.Mesh(ribbonGeometry(paths, y), clayMat(color));
    mesh.name = name;
    mesh.receiveShadow = true;
    mesh.castShadow = false;
    group.add(mesh);
  };

  make('roadRibbons', roads, kswPalette.plazaPath, kswCity.roadY);
  make('railRibbons', rails, palette.metalMatt, kswCity.railY);
  return group;
}
