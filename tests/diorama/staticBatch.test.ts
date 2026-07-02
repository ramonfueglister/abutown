import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { NIGHT_WINDOW_SHARE, batchHospital, classifyMesh, nightWindowHash } from '../../src/diorama/ksw/staticBatch';
import { clayMat, glassMat } from '../../src/diorama/ksw/props';
import { boxGeo, roundedBox } from '../../src/diorama/ksw/geometryCache';
import { nightGlow, palette, radii } from '../../src/diorama/designTokens';

const DAY = { lampGlow: false };
const NIGHT = { lampGlow: true };

function clayMesh(color: number, castShadow = true): THREE.Mesh {
  const m = new THREE.Mesh(roundedBox(1, 1, 1, 4, radii.s), clayMat(color));
  m.castShadow = castShadow;
  m.receiveShadow = true;
  return m;
}

function paneAt(x: number, z: number): THREE.Mesh {
  const pane = new THREE.Mesh(boxGeo(1, 1, 0.03), glassMat());
  pane.userData.windowPane = true;
  pane.position.set(x, 1, z);
  pane.updateMatrixWorld(true);
  return pane;
}

describe('nightWindowHash', () => {
  it('matches the deterministic position hash the night-glow swap used', () => {
    for (const [x, z] of [
      [0, 0],
      [-22.5, 12.0],
      [111.5, 64.51],
    ] as const) {
      expect(nightWindowHash(x, z)).toBeCloseTo(Math.abs(Math.sin(x * 12.9898 + z * 78.233) * 43758.5453) % 1, 10);
    }
  });
});

describe('classifyMesh', () => {
  it('routes clay meshes by shadow flag', () => {
    expect(classifyMesh(clayMesh(palette.creamBase), DAY)).toBe('clay');
    expect(classifyMesh(clayMesh(palette.mint, false), DAY)).toBe('clayNoCast');
  });

  it('routes roof-tagged meshes to roofFade regardless of material', () => {
    const roof = clayMesh(palette.metalMatt);
    roof.userData.roofFade = true;
    expect(classifyMesh(roof, DAY)).toBe('roofFade');
    expect(classifyMesh(roof, NIGHT)).toBe('roofFade');
  });

  it('routes MeshBasicMaterial (screens, op-light faces) to glow', () => {
    const screen = new THREE.Mesh(boxGeo(0.4, 0.3, 0.02), new THREE.MeshBasicMaterial({ color: 0xdff3ef }));
    expect(classifyMesh(screen, DAY)).toBe('glow');
    expect(classifyMesh(screen, NIGHT)).toBe('glow');
  });

  it('window panes: glass by day, hash-selected share glows at night', () => {
    // hash(0, 0) === 0 < NIGHT_WINDOW_SHARE: this pane glows at night
    const glowing = paneAt(0, 0);
    expect(nightWindowHash(0, 0)).toBeLessThan(NIGHT_WINDOW_SHARE);
    expect(classifyMesh(glowing, DAY)).toBe('glass');
    expect(classifyMesh(glowing, NIGHT)).toBe('glowNight');

    // find a position the hash leaves dark and verify it stays glass
    let dark: [number, number] | null = null;
    for (let i = 1; i < 100 && !dark; i++) {
      if (nightWindowHash(i * 0.37, i * 0.91) >= NIGHT_WINDOW_SHARE) dark = [i * 0.37, i * 0.91];
    }
    expect(dark).not.toBeNull();
    const darkPane = paneAt(dark![0], dark![1]);
    expect(classifyMesh(darkPane, DAY)).toBe('glass');
    expect(classifyMesh(darkPane, NIGHT)).toBe('glass');
  });

  it('lamp bulbs are clay by day and glow at night', () => {
    const bulb = clayMesh(palette.white);
    bulb.userData.lampBulb = true;
    expect(classifyMesh(bulb, DAY)).toBe('clay');
    expect(classifyMesh(bulb, NIGHT)).toBe('glowNight');
  });

  it('the world-position hash uses the pane position, not local coords', () => {
    const carrier = new THREE.Group();
    carrier.position.set(5.5, 0, -3.25);
    const pane = new THREE.Mesh(boxGeo(1, 1, 0.03), glassMat());
    pane.userData.windowPane = true;
    carrier.add(pane);
    carrier.updateMatrixWorld(true);
    const expected = nightWindowHash(5.5, -3.25) < NIGHT_WINDOW_SHARE ? 'glowNight' : 'glass';
    expect(classifyMesh(pane, NIGHT)).toBe(expected);
  });
});

describe('batchHospital', () => {
  function smallScene(): THREE.Group {
    const group = new THREE.Group();
    const wall = new THREE.Group();
    wall.position.set(2, 0, -1);
    // three instances of the SAME cached geometry: capacity is sized from
    // unique geometries, so a failed dedup would overflow and throw
    for (let i = 0; i < 3; i++) wall.add(clayMesh(palette.creamBase).translateX(i));
    group.add(wall);
    const inlay = clayMesh(palette.mint, false);
    group.add(inlay);
    group.add(paneAt(0, 0));
    const roof = clayMesh(palette.metalMatt);
    roof.userData.roofFade = true;
    roof.position.y = 3;
    group.add(roof);
    const blink = new THREE.Mesh(boxGeo(0.4, 0.12, 0.5), new THREE.MeshBasicMaterial({ color: palette.coral }));
    blink.userData.blink = true;
    group.add(blink);
    return group;
  }

  it('hoists meshes into buckets, dedupes geometry, keeps animated meshes', () => {
    const group = smallScene();
    const { batches } = batchHospital(group, DAY);
    const byName = new Map(batches.map((b) => [b.name, b]));
    expect([...byName.keys()].sort()).toEqual(['ksw-clay', 'ksw-clayNoCast', 'ksw-glass', 'ksw-roofFade']);
    expect(byName.get('ksw-clay')!.instanceCount).toBe(3);
    expect(byName.get('ksw-clayNoCast')!.instanceCount).toBe(1);
    // the originals are gone; only the blinker survives as a loose mesh
    const loose: THREE.Mesh[] = [];
    group.traverse((o) => {
      const m = o as THREE.Mesh;
      if (m.isMesh && !(m as THREE.BatchedMesh).isBatchedMesh) loose.push(m);
    });
    expect(loose.length).toBe(1);
    expect(loose[0].userData.blink).toBe(true);
    // emptied builder groups are pruned
    expect(group.children.filter((o) => !(o as THREE.Mesh).isMesh)).toHaveLength(0);
  });

  it('bakes world transforms and per-instance colors into the batch', () => {
    const group = smallScene();
    const { batches } = batchHospital(group, DAY);
    const clayBatch = batches.find((b) => b.name === 'ksw-clay')!;
    const m = new THREE.Matrix4();
    clayBatch.getMatrixAt(1, m);
    const pos = new THREE.Vector3().setFromMatrixPosition(m);
    expect(pos.x).toBeCloseTo(3); // wall group offset 2 + translateX(1)
    expect(pos.z).toBeCloseTo(-1);
    const c = new THREE.Color();
    clayBatch.getColorAt(0, c);
    expect(c.getHex()).toBe(new THREE.Color(palette.creamBase).getHex());
  });

  it('night mode moves the hash-selected panes into a glowNight bucket', () => {
    const group = smallScene(); // its pane sits at hash(0,0)=0 -> glows
    const { batches } = batchHospital(group, NIGHT);
    const names = batches.map((b) => b.name);
    expect(names).toContain('ksw-glowNight');
    expect(names).not.toContain('ksw-glass');
    const glowNight = batches.find((b) => b.name === 'ksw-glowNight')!;
    const mat = glowNight.material as THREE.MeshBasicMaterial;
    expect(mat.color.getHex()).toBe(new THREE.Color(nightGlow.bulb).getHex());
    expect(mat.transparent).toBe(true);
    expect(mat.opacity).toBeCloseTo(0.9);
  });

  it('the roofs control drives the roofFade batch exactly like the old per-mesh control', () => {
    const group = smallScene();
    const { batches, roofs } = batchHospital(group, DAY);
    const roofBatch = batches.find((b) => b.name === 'ksw-roofFade')!;
    const mat = roofBatch.material as THREE.Material & { opacity: number };
    expect(mat.transparent).toBe(true);
    roofs.setFade(1);
    expect(roofBatch.castShadow).toBe(true);
    expect(roofBatch.visible).toBe(true);
    roofs.setFade(0.4);
    expect(roofs.fade()).toBeCloseTo(0.4);
    expect(mat.opacity).toBeCloseTo(0.4);
    expect(roofBatch.castShadow).toBe(false);
    expect(roofBatch.visible).toBe(true);
    roofs.setFade(0.01);
    expect(roofBatch.visible).toBe(false);
    roofs.setFade(7);
    expect(roofs.fade()).toBe(1);
    roofs.setFade(-2);
    expect(roofs.fade()).toBe(0);
  });
});
