import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { NIGHT_WINDOW_SHARE, batchHospital, classifyMesh, nightWindowHash } from '../../src/diorama/ksw/staticBatch';
import { clayMat, glassMat } from '../../src/diorama/ksw/props';
import { boxGeo, roundedBox } from '../../src/diorama/ksw/geometryCache';
import { nightGlow, palette, radii } from '../../src/diorama/designTokens';

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
    expect(classifyMesh(clayMesh(palette.creamBase))).toBe('clay');
    expect(classifyMesh(clayMesh(palette.mint, false))).toBe('clayNoCast');
  });

  it('routes roof-tagged meshes to roofFade regardless of material', () => {
    const roof = clayMesh(palette.metalMatt);
    roof.userData.roofFade = true;
    expect(classifyMesh(roof)).toBe('roofFade');
  });

  it('routes MeshBasicMaterial (screens, op-light faces) to glow', () => {
    const screen = new THREE.Mesh(boxGeo(0.4, 0.3, 0.02), new THREE.MeshBasicMaterial({ color: 0xdff3ef }));
    expect(classifyMesh(screen)).toBe('glow');
  });

  it('window panes: hash-selected share always route to glowNight (intensity rides lampGlowU)', () => {
    // hash(0, 0) === 0 < NIGHT_WINDOW_SHARE: this pane routes into glowNight
    const glowing = paneAt(0, 0);
    expect(nightWindowHash(0, 0)).toBeLessThan(NIGHT_WINDOW_SHARE);
    expect(classifyMesh(glowing)).toBe('glowNight');

    // a hash-dark pane stays plain glass
    let dark: [number, number] | null = null;
    for (let i = 1; i < 100 && !dark; i++) {
      if (nightWindowHash(i * 0.37, i * 0.91) >= NIGHT_WINDOW_SHARE) dark = [i * 0.37, i * 0.91];
    }
    expect(dark).not.toBeNull();
    const darkPane = paneAt(dark![0], dark![1]);
    expect(classifyMesh(darkPane)).toBe('glass');
  });

  it('lamp bulbs always route to glowNight (intensity rides lampGlowU)', () => {
    const bulb = clayMesh(palette.white);
    bulb.userData.lampBulb = true;
    expect(classifyMesh(bulb)).toBe('glowNight');
  });

  it('the world-position hash uses the pane position, not local coords', () => {
    const carrier = new THREE.Group();
    carrier.position.set(5.5, 0, -3.25);
    const pane = new THREE.Mesh(boxGeo(1, 1, 0.03), glassMat());
    pane.userData.windowPane = true;
    carrier.add(pane);
    carrier.updateMatrixWorld(true);
    const expected = nightWindowHash(5.5, -3.25) < NIGHT_WINDOW_SHARE ? 'glowNight' : 'glass';
    expect(classifyMesh(pane)).toBe(expected);
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
    const { batches } = batchHospital(group);
    const byName = new Map(batches.map((b) => [b.name, b]));
    // the pane at hash(0,0)=0 now always routes to glowNight (its intensity
    // rides lampGlowU), so the day scene has no plain-glass bucket.
    expect([...byName.keys()].sort()).toEqual(['ksw-clay', 'ksw-clayNoCast', 'ksw-glowNight', 'ksw-roofFade']);
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
    const { batches } = batchHospital(group);
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

  it('hash-selected panes always route to a glowNight bucket (intensity rides lampGlowU)', () => {
    const group = smallScene(); // its pane sits at hash(0,0)=0 -> glows
    const { batches } = batchHospital(group);
    const names = batches.map((b) => b.name);
    expect(names).toContain('ksw-glowNight');
    const glowNight = batches.find((b) => b.name === 'ksw-glowNight')!;
    // warm night-glow node material: colour still nightGlow.bulb, transparent,
    // depthWrite off, opacity driven by lampGlowU via opacityNode (not a static
    // .opacity — so we assert the presence of the node instead of a value).
    const mat = glowNight.material as THREE.MeshBasicNodeMaterial;
    expect(mat.color.getHex()).toBe(new THREE.Color(nightGlow.bulb).getHex());
    expect(mat.transparent).toBe(true);
    expect(mat.depthWrite).toBe(false); // unsorted transparent batch
    expect(mat.opacityNode).toBeTruthy(); // opacity rides the lampGlowU uniform
  });

  it('the roofs control drives the roofFade batch exactly like the old per-mesh control', () => {
    const group = smallScene();
    const { batches, roofs } = batchHospital(group);
    const roofBatch = batches.find((b) => b.name === 'ksw-roofFade')!;
    const mat = roofBatch.material as THREE.Material & { opacity: number };
    expect(mat.transparent).toBe(true);
    roofs.setFade(1);
    expect(roofBatch.castShadow).toBe(true);
    expect(roofBatch.visible).toBe(true);
    expect(mat.depthWrite).toBe(true); // fully opaque roofs must occlude
    roofs.setFade(0.4);
    expect(roofs.fade()).toBeCloseTo(0.4);
    expect(mat.opacity).toBeCloseTo(0.4);
    expect(roofBatch.castShadow).toBe(false);
    expect(roofBatch.visible).toBe(true);
    expect(mat.depthWrite).toBe(false); // mid-fade: no depth rejection between lids
    roofs.setFade(0.01);
    expect(roofBatch.visible).toBe(false);
    expect(mat.depthWrite).toBe(false);
    roofs.setFade(7);
    expect(roofs.fade()).toBe(1);
    roofs.setFade(-2);
    expect(roofs.fade()).toBe(0);
  });
});
