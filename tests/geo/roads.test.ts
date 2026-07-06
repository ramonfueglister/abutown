// tests/geo/roads.test.ts
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { apronStrip, buildRoads, miterOffsets, miterStrip, skirtStrip } from '../../src/diorama/ksw/geo/roads';

describe('miterStrip', () => {
  it('builds a continuous strip: 2 verts per point, no seams', () => {
    const g = miterStrip([[0, 0], [10, 0], [10, 10]], 6, 0.04);
    expect(g.positions.length / 3).toBe(6); // 3 pts × 2
    expect(g.indices.length).toBe(12); // 2 segments × 2 tris
  });
  it('miter joint bisects a right angle (outer corner further than half width)', () => {
    const g = miterStrip([[0, 0], [10, 0], [10, 10]], 6, 0);
    // corner verts are at index 2,3: bisector direction (±(1,-1)/√2) × 3√2
    const cx = [g.positions[6], g.positions[9]];
    for (const x of cx) expect(Math.abs(x - 10)).toBeCloseTo(3, 3);
  });
  it('caps extreme spikes', () => {
    const g = miterStrip([[0, 0], [10, 0], [0, 0.4]], 6, 0); // ~176° turn
    for (let i = 0; i < g.positions.length; i += 3) {
      expect(Math.abs(g.positions[i])).toBeLessThan(25); // no infinite miter spike
    }
  });
});

describe('apronStrip (Bankett/verge: ribbon edge → mask edge)', () => {
  // Straight ribbon along +x: ribbon width 2 (renderHW 1), mask width 5 (maskHW
  // 2.5 — the cell floor for a narrow footway), draped flat on profile=10.
  const GROUND = 10;
  const Y = 0.04;
  const ground = () => GROUND;
  const a = apronStrip([[0, 0], [10, 0]], 2, 5, Y, ground);

  it('emits two flat verge strips (both edges), inner=ribbon outer=mask', () => {
    // 2 sides × 2 points × 2 verts (inner+outer) = 8 verts; 2 sides × 1 seg × 2 tris.
    expect(a.positions.length / 3).toBe(8);
    expect(a.indices.length).toBe(4 * 3);
  });

  it('inner z = ±ribbonHalf (1), outer z = ±maskHalf (2.5) at profile height', () => {
    const zs = [];
    const ys = [];
    for (let i = 0; i < a.positions.length; i += 3) {
      ys.push(a.positions[i + 1]);
      zs.push(Math.round(Math.abs(a.positions[i + 2]) * 100) / 100);
    }
    // inner z at ribbonHalf (1) or its ×2 endcap (2); outer at maskHalf (2.5)
    // or its ×2 endcap (5). Never anything else, and never 0 (the centreline).
    expect(zs.every((z) => z === 1 || z === 2 || z === 2.5 || z === 5)).toBe(true);
    expect(zs.some((z) => z === 1 || z === 2)).toBe(true); // ribbon inner edge present
    expect(zs.some((z) => z === 2.5 || z === 5)).toBe(true); // mask outer edge present
    for (const y of ys) expect(y).toBeCloseTo(GROUND + Y, 5); // flat at profile
  });

  it('degenerate: mask width ≤ ribbon width emits nothing (wide ways)', () => {
    // renderHW 3 (width 6) ≥ 2.5 m cell floor → mask width == ribbon width → no apron.
    const none = apronStrip([[0, 0], [10, 0]], 6, 6, Y, ground);
    expect(none.positions.length).toBe(0);
    expect(none.indices.length).toBe(0);
  });
});

describe('skirtStrip (mask edge → tile ground − 0.5)', () => {
  // A straight platform along +x, mask width 5 (maskHW 2.5), layer offset y=0.04,
  // ribbon/apron draped on profile=10, tile ground SLOPING (a rising cut on one
  // side, falling embankment on the other). Skirt top at profile, bottom PER
  // VERTEX at tileGround(edge) − 0.5.
  const Y = 0.04;
  const FOOT = 0.5;
  const profile = () => 10;
  // tile ground varies with z: left edge (+z) sits at 8 (embankment), right edge
  // (−z) sits at 12 (cut bank rising above the profile).
  const tile = (_x: number, z: number) => 10 - z; // z=+5→5, z=−5→15 (endcap ×2)
  const s = skirtStrip([[0, 0], [10, 0]], 5, Y, profile, tile, FOOT);

  it('emits two vertical skirt strips (both mask edges)', () => {
    expect(s.positions.length / 3).toBe(8);
    expect(s.indices.length).toBe(4 * 3);
  });

  it('top at profile+lift; bottom PER-VERTEX at tileGround(edge) − foot', () => {
    // Walk vertices: top then bottom, for each point of each side.
    for (let v = 0; v < s.positions.length / 3; v += 2) {
      const topY = s.positions[v * 3 + 1];
      const botX = s.positions[(v + 1) * 3 + 0];
      const botZ = s.positions[(v + 1) * 3 + 2];
      const botY = s.positions[(v + 1) * 3 + 1];
      expect(topY).toBeCloseTo(10 + Y, 5); // top = profile + lift
      // bottom reaches the TILE ground (not the profile) minus the foot: this is
      // the terrain-grounded contract — the skirt always reaches the terrain.
      expect(botY).toBeCloseTo(tile(botX, botZ) - FOOT, 5);
    }
  });

  it('skirt x/z lie on the MASK edges (miter-consistent), never the centreline', () => {
    // mask width 5 → half 2.5 along +x; endcap ×2 → |z|=5 at open ends, |z|=2.5
    // interior — NEVER z=0 (the centreline).
    const midStrip = skirtStrip([[0, 0], [10, 0], [20, 0]], 5, Y, profile, tile, FOOT);
    const zs = [];
    for (let i = 2; i < midStrip.positions.length; i += 3) zs.push(Math.abs(Math.round(midStrip.positions[i] * 100) / 100));
    expect(zs.every((z) => z === 2.5 || z === 5)).toBe(true);
    expect(zs.some((z) => z === 0)).toBe(false);
  });
});

describe('miterOffsets (shared by miterStrip, apronStrip AND skirtStrip)', () => {
  const pts = [[0, 0], [10, 0], [10, 10], [0, 0.4]];
  it('produces the SAME edge offsets all three builders consume (no drift)', () => {
    // miterStrip/apronStrip/skirtStrip flat (no drape) all consume miterOffsets
    // on the untouched pts. The apron OUTER edge, the skirt TOP edge, and a
    // ribbon of the same (mask) width must all land on identical x/z — the seam
    // where ribbon → apron → skirt meet. Prove they read the SAME offsets.
    const offs = miterOffsets(pts);
    const RIBBON_W = 6; // renderHW 3
    const MASK_W = 8; // maskHW 4 (apron fills 3→4; skirt top at 4)
    const ribbonHalf = MASK_W / 2; // a ribbon of the mask width for the seam check
    const ribbon = miterStrip(pts, MASK_W, 0.04); // no groundYAt → no subdivide
    const apron = apronStrip(pts, RIBBON_W, MASK_W, 0.04); // outer edge at maskHalf
    const skirt = skirtStrip(pts, MASK_W, 0.04); // top edge at maskHalf
    for (let i = 0; i < pts.length; i++) {
      const { mx, mz, scale } = offs[i];
      const ex = pts[i][0] + mx * ribbonHalf * scale;
      const ez = pts[i][1] + mz * ribbonHalf * scale;
      // ribbon vertex 2i is the +side edge (see miterStrip push order)
      expect(ribbon.positions[i * 6 + 0]).toBeCloseTo(ex, 9);
      expect(ribbon.positions[i * 6 + 2]).toBeCloseTo(ez, 9);
      // apron side=+1 strip: OUTER vertex of point i is at index (i*2+1)*3
      expect(apron.positions[i * 6 + 3]).toBeCloseTo(ex, 9);
      expect(apron.positions[i * 6 + 5]).toBeCloseTo(ez, 9);
      // skirt side=+1 strip: TOP vertex of point i is at index (i*2)*3
      expect(skirt.positions[i * 6 + 0]).toBeCloseTo(ex, 9);
      expect(skirt.positions[i * 6 + 2]).toBeCloseTo(ez, 9);
    }
  });
});

describe('buildRoads', () => {
  const group = buildRoads(
    [
      { class: 'residential', width: 6, pts: [[0, 0], [10, 0]] },
      { class: 'footway', width: 2.2, pts: [[0, 5], [10, 5]] },
    ],
    [{ class: 'rail', width: 3, pts: [[0, 9], [10, 9]] }],
  );
  it('splits carriage / footway / rail(+bed) into named layers', () => {
    for (const n of ['carriageRibbons', 'footwayRibbons', 'railBeds', 'railRibbons'])
      expect(group.getObjectByName(n)).toBeTruthy();
  });
  it('layers sit on distinct heights (no z-fight)', () => {
    const y = (n: string) => (group.getObjectByName(n) as THREE.Mesh).geometry.getAttribute('position').getY(0);
    expect(y('carriageRibbons')).not.toBeCloseTo(y('footwayRibbons'), 3);
  });

  it('platform (apron + terrain-grounded skirts) built only with both samplers', () => {
    // No samplers → bare ribbons, no platform (pre-#119 anchor look).
    const bare = buildRoads([{ class: 'residential', width: 6, pts: [[0, 0], [10, 0]] }], []);
    expect(bare.getObjectByName('carriageSkirts')).toBeFalsy();
    expect(bare.getObjectByName('carriageAprons')).toBeFalsy();
    // A NARROW footway (2.2 m → renderHW 1.1 < 2.5 m cell floor) gets an apron
    // (ribbon→mask edge) AND a skirt reaching the tile ground.
    const PROFILE = 20;
    const TILE = 12; // embankment: terrain 8 m below the profile
    const profile = () => PROFILE;
    const tile = () => TILE;
    const g = buildRoads([], [], profile, tile) as THREE.Group; // no ways → empty but built
    expect(g).toBeTruthy();
    const withFoot = buildRoads(
      [{ class: 'footway', width: 2.2, pts: [[0, 0], [40, 0]] }],
      [],
      profile,
      tile,
    );
    const apron = withFoot.getObjectByName('footwayAprons') as THREE.Mesh;
    const skirt = withFoot.getObjectByName('footwaySkirts') as THREE.Mesh;
    expect(apron).toBeTruthy();
    expect(skirt).toBeTruthy();
    // Apron has geometry (renderHW 1.1 < maskHW 2.5 → non-degenerate).
    expect(apron.geometry.getAttribute('position').count).toBeGreaterThan(0);
    // Skirt bottom vertices reach the TILE ground − 0.5 m (terrain-grounded),
    // NOT profile − const. Min y over the skirt must equal TILE − 0.5.
    const pos = skirt.geometry.getAttribute('position');
    let minY = Infinity;
    for (let i = 0; i < pos.count; i++) minY = Math.min(minY, pos.getY(i));
    expect(minY).toBeCloseTo(TILE - 0.5, 4);
  });
});
