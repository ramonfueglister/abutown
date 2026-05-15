# Simutrans pak128 Hard Cutover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the OpenTTD/OpenGFX visual stack with a pak128-only Simutrans renderer and remove the old OpenTTD asset files, import scripts, catalogs, and runtime references.

**Architecture:** Keep the Zurich world model, placement, topology, and movement systems. Replace every renderer asset path with semantic pak128 catalog lookup, use native pak128 frame metadata, and make missing roles fail tests and startup instead of silently drawing an old asset. Delete the retired OpenTTD/OpenGFX asset pipeline once the pak128 catalog covers the visible runtime categories.

**Tech Stack:** Vite, TypeScript, Canvas 2D, Vitest, Playwright, Node.js pak128 import script, Simutrans pak128 PNG/DAT assets, Artistic License 2.0 attribution.

---

## Hard Requirements

- The browser runtime must not load `/opengfx2/` or `/openttd-fan-assets/` paths.
- `src/main.ts`, renderer helpers, and tests must not import `opengfxCatalog`, `opengfxCatalog.generated`, or OpenTTD fan-asset data.
- `package.json` must remove `assets:opengfx`.
- `scripts/import-opengfx-assets.mjs`, `scripts/decode-openttd-fan-grfs.mjs`, `src/assets/opengfxCatalog.ts`, `src/assets/opengfxCatalog.generated.ts`, `tests/render/opengfxCatalog.test.ts`, `public/opengfx2`, and `public/openttd-fan-assets` must be deleted.
- Every visible category must resolve to pak128 metadata: terrain, water, riverbank, roads, bridges, rail, rail station, residential buildings, commercial buildings, civic buildings, industrial buildings, trees, park/details, industry details, buses, trucks, trains, wagons, and pedestrians.
- The asset-pack API must not contain substitute-role mappings. A missing role is a bug and must throw a clear error.
- The final verification must include a guard test that fails if runtime source or public runtime assets still contain OpenTTD/OpenGFX references.

## File Structure

- Create `/Users/ramonfuglister/Desktop/Coding/abutown/src/assets/assetPack.ts`: pak128-only semantic asset-pack types and strict lookup helpers.
- Create `/Users/ramonfuglister/Desktop/Coding/abutown/src/assets/pak128Catalog.ts`: complete runtime pak128 roles, source rectangles, anchors, scale, direction metadata, and provenance.
- Create `/Users/ramonfuglister/Desktop/Coding/abutown/scripts/import-pak128-assets.mjs`: sparse-imports curated pak128 PNG/DAT/license files pinned to revision `acdf2f0793a6beee5ea34ea85d308fbbeccf50c5`.
- Create `/Users/ramonfuglister/Desktop/Coding/abutown/tests/render/assetPack.test.ts`: strict lookup coverage.
- Create `/Users/ramonfuglister/Desktop/Coding/abutown/tests/render/pak128Catalog.test.ts`: role coverage, metadata, and DAT-backed source-frame coverage.
- Create `/Users/ramonfuglister/Desktop/Coding/abutown/tests/render/noRetiredAssets.test.ts`: guard against retired asset paths in runtime files.
- Modify `/Users/ramonfuglister/Desktop/Coding/abutown/src/main.ts`: load and draw only pak128 catalog assets.
- Modify `/Users/ramonfuglister/Desktop/Coding/abutown/src/render/simutransPedestrianSprites.ts`: use pak128 catalog paths.
- Modify `/Users/ramonfuglister/Desktop/Coding/abutown/src/render/vehicleSprites.ts`: replace OpenGFX vehicle sheets with pak128 vehicle direction metadata.
- Modify `/Users/ramonfuglister/Desktop/Coding/abutown/src/render/spriteCleanup.ts`: keep generic transparent-source cleanup and remove OpenGFX shape-row special casing after OpenGFX assets are gone.
- Modify `/Users/ramonfuglister/Desktop/Coding/abutown/tests/e2e/render-smoke.spec.ts`: assert pak128-only runtime state and zero browser console errors.
- Modify `/Users/ramonfuglister/Desktop/Coding/abutown/package.json`: add `assets:pak128` and remove `assets:opengfx`.
- Create `/Users/ramonfuglister/Desktop/Coding/abutown/public/simutrans-assets/pak128/README.md`: source revision, imported-file list, DAT notes, and license notes.
- Delete `/Users/ramonfuglister/Desktop/Coding/abutown/src/assets/opengfxCatalog.ts`.
- Delete `/Users/ramonfuglister/Desktop/Coding/abutown/src/assets/opengfxCatalog.generated.ts`.
- Delete `/Users/ramonfuglister/Desktop/Coding/abutown/tests/render/opengfxCatalog.test.ts`.
- Delete `/Users/ramonfuglister/Desktop/Coding/abutown/scripts/import-opengfx-assets.mjs`.
- Delete `/Users/ramonfuglister/Desktop/Coding/abutown/scripts/decode-openttd-fan-grfs.mjs`.
- Delete `/Users/ramonfuglister/Desktop/Coding/abutown/public/opengfx2`.
- Delete `/Users/ramonfuglister/Desktop/Coding/abutown/public/openttd-fan-assets`.

## Scope Boundary

This is a hard visual cutover, not an incremental mixed-asset slice. It is acceptable for the first pak128 result to need tuning, but it is not acceptable for roads, rail, trains, buildings, vehicles, terrain, or details to draw old OpenTTD/OpenGFX assets. If a pak128 source cannot be found for a role, implementation stops and chooses a real pak128 source before continuing.

### Task 1: Strict Asset-Pack Contract

**Files:**
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/src/assets/assetPack.ts`
- Test: `/Users/ramonfuglister/Desktop/Coding/abutown/tests/render/assetPack.test.ts`

- [ ] **Step 1: Write strict lookup tests**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/tests/render/assetPack.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { createAssetPack, missingAssetRoleError } from '../../src/assets/assetPack';

describe('asset pack lookup', () => {
  const pack = createAssetPack({
    id: 'test-pak128',
    tile: { width: 128, height: 64 },
    assets: [
      {
        role: 'terrain.grass',
        path: '/simutrans-assets/pak128/landscape/grounds/texture-climate.png',
        source: { x: 0, y: 0, width: 128, height: 64 },
        anchor: { x: 64, y: 32 },
        baseline: 32,
        scale: 1,
        cleanup: 'pak128',
        provenance: {
          sourcePath: 'landscape/grounds/texture-climate.png',
          datPath: 'landscape/grounds/texture-climate.dat',
          license: 'Artistic-2.0',
          revision: 'acdf2f0793a6beee5ea34ea85d308fbbeccf50c5',
        },
      },
    ],
  });

  it('resolves exact semantic roles', () => {
    expect(pack.require('terrain.grass')).toEqual(expect.objectContaining({
      role: 'terrain.grass',
      path: '/simutrans-assets/pak128/landscape/grounds/texture-climate.png',
    }));
  });

  it('returns undefined for missing optional lookup', () => {
    expect(pack.resolve('road.straight')).toBeUndefined();
  });

  it('throws a clear error for missing required roles', () => {
    expect(() => pack.require('road.straight')).toThrow(missingAssetRoleError('test-pak128', 'road.straight'));
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
npm test -- tests/render/assetPack.test.ts
```

Expected: FAIL with an import error for `../../src/assets/assetPack`.

- [ ] **Step 3: Implement strict asset-pack contract**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/src/assets/assetPack.ts`:

```ts
export type AssetRole =
  | 'terrain.grass'
  | 'terrain.water'
  | 'terrain.riverbank'
  | 'road.straight'
  | 'road.curve'
  | 'road.intersection'
  | 'road.bridge'
  | 'rail.straight'
  | 'rail.station'
  | 'building.residential.low'
  | 'building.commercial.mid'
  | 'building.civic'
  | 'building.industrial'
  | 'vegetation.tree'
  | 'detail.park'
  | 'detail.industry'
  | 'detail.dock'
  | 'detail.quay'
  | 'vehicle.bus'
  | 'vehicle.truck'
  | 'vehicle.train.engine'
  | 'vehicle.train.wagon'
  | 'agent.pedestrian';

export type Rect = { x: number; y: number; width: number; height: number };
export type Point = { x: number; y: number };
export type CleanupPolicy = 'none' | 'pak128';

export type AssetProvenance = {
  sourcePath: string;
  datPath?: string;
  license: 'Artistic-2.0';
  revision: string;
};

export type AssetFrame = {
  role: AssetRole;
  path: string;
  source: Rect;
  anchor: Point;
  baseline: number;
  scale: number;
  cleanup: CleanupPolicy;
  provenance: AssetProvenance;
  direction?: 'N' | 'NE' | 'E' | 'SE' | 'S' | 'SW' | 'W' | 'NW';
  variant?: string;
};

export type AssetPackDefinition = {
  id: string;
  tile: { width: number; height: number };
  assets: AssetFrame[];
};

export type AssetPack = {
  id: string;
  tile: { width: number; height: number };
  all: () => AssetFrame[];
  resolve: (role: AssetRole) => AssetFrame | undefined;
  require: (role: AssetRole) => AssetFrame;
};

export function missingAssetRoleError(packId: string, role: AssetRole): string {
  return `Asset pack ${packId} does not define required role ${role}`;
}

export function createAssetPack(definition: AssetPackDefinition): AssetPack {
  const assetsByRole = new Map<AssetRole, AssetFrame>();
  for (const asset of definition.assets) assetsByRole.set(asset.role, asset);

  return {
    id: definition.id,
    tile: { ...definition.tile },
    all: () => [...definition.assets],
    resolve: (role) => assetsByRole.get(role),
    require: (role) => {
      const asset = assetsByRole.get(role);
      if (!asset) throw new Error(missingAssetRoleError(definition.id, role));
      return asset;
    },
  };
}
```

- [ ] **Step 4: Run asset-pack tests**

Run:

```bash
npm test -- tests/render/assetPack.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

Run:

```bash
git add src/assets/assetPack.ts tests/render/assetPack.test.ts
git commit -m "feat: add strict pak128 asset pack contract"
```

### Task 2: pak128 Import And Complete Catalog

**Files:**
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/scripts/import-pak128-assets.mjs`
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/src/assets/pak128Catalog.ts`
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/tests/render/pak128Catalog.test.ts`
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/public/simutrans-assets/pak128/README.md`
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/package.json`

- [ ] **Step 1: Write catalog completeness tests**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/tests/render/pak128Catalog.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { pak128AssetPack, PAK128_REQUIRED_ROLES, PAK128_REVISION } from '../../src/assets/pak128Catalog';

describe('pak128 catalog', () => {
  it('pins the audited source revision', () => {
    expect(PAK128_REVISION).toBe('acdf2f0793a6beee5ea34ea85d308fbbeccf50c5');
  });

  it('defines every runtime role with pak128 provenance', () => {
    for (const role of PAK128_REQUIRED_ROLES) {
      const asset = pak128AssetPack.require(role);
      expect(asset.role).toBe(role);
      expect(asset.path).toMatch(/^\/simutrans-assets\/pak128\//u);
      expect(asset.cleanup).toBe('pak128');
      expect(asset.source.width).toBeGreaterThan(0);
      expect(asset.source.height).toBeGreaterThan(0);
      expect(asset.provenance).toEqual(expect.objectContaining({
        license: 'Artistic-2.0',
        revision: PAK128_REVISION,
      }));
    }
  });

  it('does not contain retired asset paths', () => {
    for (const asset of pak128AssetPack.all()) {
      expect(asset.path).not.toMatch(/opengfx|openttd/i);
      expect(asset.provenance.sourcePath).not.toMatch(/opengfx|openttd/i);
    }
  });

  it('uses DAT-backed frame coordinates for known directional assets', () => {
    expect(pak128AssetPack.require('agent.pedestrian').source).toEqual({ x: 0, y: 128, width: 128, height: 128 });
    expect(pak128AssetPack.require('vehicle.bus').source.y).toBe(128);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
npm test -- tests/render/pak128Catalog.test.ts
```

Expected: FAIL with an import error for `../../src/assets/pak128Catalog`.

- [ ] **Step 3: Add importer**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/scripts/import-pak128-assets.mjs` with a sparse checkout that copies every source PNG/DAT referenced by `src/assets/pak128Catalog.ts` plus `LICENSE.txt` and `README.txt`. Use revision `acdf2f0793a6beee5ea34ea85d308fbbeccf50c5`. The importer must delete and recreate only `/Users/ramonfuglister/Desktop/Coding/abutown/public/simutrans-assets/pak128`.

- [ ] **Step 4: Add complete pak128 catalog**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/src/assets/pak128Catalog.ts` with:

```ts
import { createAssetPack, type AssetFrame, type AssetRole } from './assetPack';

export const PAK128_REVISION = 'acdf2f0793a6beee5ea34ea85d308fbbeccf50c5';
export const PAK128_REQUIRED_ROLES = [
  'terrain.grass',
  'terrain.water',
  'terrain.riverbank',
  'road.straight',
  'road.curve',
  'road.intersection',
  'road.bridge',
  'rail.straight',
  'rail.station',
  'building.residential.low',
  'building.commercial.mid',
  'building.civic',
  'building.industrial',
  'vegetation.tree',
  'detail.park',
  'detail.industry',
  'detail.dock',
  'detail.quay',
  'vehicle.bus',
  'vehicle.truck',
  'vehicle.train.engine',
  'vehicle.train.wagon',
  'agent.pedestrian',
] as const satisfies readonly AssetRole[];

const ROOT = '/simutrans-assets/pak128';
const provenance = (sourcePath: string, datPath?: string) => ({
  sourcePath,
  datPath,
  license: 'Artistic-2.0' as const,
  revision: PAK128_REVISION,
});

const pak128Assets: AssetFrame[] = [
  {
    role: 'terrain.grass',
    path: `${ROOT}/landscape/grounds/texture-climate.png`,
    source: { x: 0, y: 0, width: 128, height: 64 },
    anchor: { x: 64, y: 32 },
    baseline: 32,
    scale: 1,
    cleanup: 'pak128',
    provenance: provenance('landscape/grounds/texture-climate.png', 'landscape/grounds/texture-climate.dat'),
  },
  {
    role: 'vehicle.bus',
    path: `${ROOT}/vehicles/road-psg+mail/man_lions_city.png`,
    source: { x: 0, y: 128, width: 128, height: 128 },
    anchor: { x: 64, y: 88 },
    baseline: 88,
    scale: 0.42,
    cleanup: 'pak128',
    direction: 'W',
    provenance: provenance('vehicles/road-psg+mail/man_lions_city.png', 'vehicles/road-psg+mail/man_lions_city.dat'),
  },
];

export const pak128AssetPack = createAssetPack({
  id: 'simutrans-pak128',
  tile: { width: 128, height: 64 },
  assets: pak128Assets,
});

for (const role of PAK128_REQUIRED_ROLES) pak128AssetPack.require(role);
```

Extend `pak128Assets` in the same file until every `PAK128_REQUIRED_ROLES` entry resolves to a real pak128 asset. Use DAT files to derive source rectangles; do not invent rectangles where DAT coordinates exist. The `for` loop is intentional: app startup must fail during development if a role is missing.

- [ ] **Step 5: Update package script**

Modify `/Users/ramonfuglister/Desktop/Coding/abutown/package.json`:

```json
"assets:pak128": "node scripts/import-pak128-assets.mjs"
```

Remove:

```json
"assets:opengfx": "node scripts/import-opengfx-assets.mjs"
```

- [ ] **Step 6: Run importer and catalog tests**

Run:

```bash
npm run assets:pak128
npm test -- tests/render/pak128Catalog.test.ts tests/render/assetPack.test.ts
```

Expected: PASS.

- [ ] **Step 7: Commit**

Run:

```bash
git add package.json scripts/import-pak128-assets.mjs src/assets/pak128Catalog.ts tests/render/pak128Catalog.test.ts public/simutrans-assets/pak128
git commit -m "feat: add complete pak128 asset catalog"
```

### Task 3: Renderer pak128 Cutover

**Files:**
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/src/main.ts`
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/src/render/simutransPedestrianSprites.ts`
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/src/render/vehicleSprites.ts`
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/src/render/spriteCleanup.ts`
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/tests/e2e/render-smoke.spec.ts`
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/tests/render/simutransPedestrianSprites.test.ts`
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/tests/render/vehicleSprites.test.ts`
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/tests/render/spriteCleanup.test.ts`

- [ ] **Step 1: Update smoke test first**

In `/Users/ramonfuglister/Desktop/Coding/abutown/tests/e2e/render-smoke.spec.ts`, assert:

```ts
expect(state.city.assetPack).toEqual({
  id: 'simutrans-pak128',
  tile: { width: 128, height: 64 },
});
expect(state.city.nonPak128AssetPaths).toEqual([]);
```

- [ ] **Step 2: Run smoke test to verify it fails**

Run:

```bash
npm run test:e2e -- tests/e2e/render-smoke.spec.ts
```

Expected: FAIL because `assetPack` and `nonPak128AssetPaths` are not reported yet.

- [ ] **Step 3: Replace runtime asset loading**

In `/Users/ramonfuglister/Desktop/Coding/abutown/src/main.ts`, remove `assetPaths`, `buildingSheets`, `/opengfx2/` image loading, and OpenGFX draw branches. Add:

```ts
import { pak128AssetPack } from './assets/pak128Catalog';
import type { AssetFrame, AssetRole } from './assets/assetPack';

const activeAssetPack = pak128AssetPack;
const TILE_W = activeAssetPack.tile.width;
const TILE_H = activeAssetPack.tile.height;
```

Load only:

```ts
const imageEntries = [
  ...activeAssetPack.all().map((asset) => [asset.path, asset.path] as const),
  ...Object.values(SIMUTRANS_PEDESTRIAN_ASSET_PATHS).map((path) => [path, path] as const),
];
```

- [ ] **Step 4: Draw all categories through pak128 roles**

Update terrain, road, rail, rail station, detail, building, tree, car, train, and pedestrian draw helpers so every category calls `activeAssetPack.require(role)` before drawing. Keep per-category layout logic, but remove every old path constant.

- [ ] **Step 5: Report pak128 runtime diagnostics**

In `window.render_game_to_text`, add:

```ts
assetPack: {
  id: activeAssetPack.id,
  tile: activeAssetPack.tile,
},
nonPak128AssetPaths: [...images.keys()].filter((path) => !path.startsWith('/simutrans-assets/pak128/')),
```

- [ ] **Step 6: Run focused tests**

Run:

```bash
npm test -- tests/render/simutransPedestrianSprites.test.ts tests/render/vehicleSprites.test.ts tests/render/spriteCleanup.test.ts tests/render/pak128Catalog.test.ts
npm run test:e2e -- tests/e2e/render-smoke.spec.ts
```

Expected: PASS and `nonPak128AssetPaths` is `[]`.

- [ ] **Step 7: Commit**

Run:

```bash
git add src/main.ts src/render/simutransPedestrianSprites.ts src/render/vehicleSprites.ts src/render/spriteCleanup.ts tests/e2e/render-smoke.spec.ts tests/render/simutransPedestrianSprites.test.ts tests/render/vehicleSprites.test.ts tests/render/spriteCleanup.test.ts
git commit -m "feat: cut renderer over to pak128 assets"
```

### Task 4: Remove Retired OpenTTD/OpenGFX Assets

**Files:**
- Delete: `/Users/ramonfuglister/Desktop/Coding/abutown/src/assets/opengfxCatalog.ts`
- Delete: `/Users/ramonfuglister/Desktop/Coding/abutown/src/assets/opengfxCatalog.generated.ts`
- Delete: `/Users/ramonfuglister/Desktop/Coding/abutown/tests/render/opengfxCatalog.test.ts`
- Delete: `/Users/ramonfuglister/Desktop/Coding/abutown/scripts/import-opengfx-assets.mjs`
- Delete: `/Users/ramonfuglister/Desktop/Coding/abutown/scripts/decode-openttd-fan-grfs.mjs`
- Delete: `/Users/ramonfuglister/Desktop/Coding/abutown/public/opengfx2`
- Delete: `/Users/ramonfuglister/Desktop/Coding/abutown/public/openttd-fan-assets`
- Test: `/Users/ramonfuglister/Desktop/Coding/abutown/tests/render/noRetiredAssets.test.ts`

- [ ] **Step 1: Add retired-asset guard test**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/tests/render/noRetiredAssets.test.ts`:

```ts
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const root = process.cwd();
const runtimeFiles = [
  'package.json',
  'src/main.ts',
  'src/render/vehicleSprites.ts',
  'src/render/simutransPedestrianSprites.ts',
  'src/render/spriteCleanup.ts',
];

describe('retired OpenTTD/OpenGFX asset removal', () => {
  it('removes old asset directories and import scripts', () => {
    for (const path of [
      'public/opengfx2',
      'public/openttd-fan-assets',
      'scripts/import-opengfx-assets.mjs',
      'scripts/decode-openttd-fan-grfs.mjs',
      'src/assets/opengfxCatalog.ts',
      'src/assets/opengfxCatalog.generated.ts',
    ]) {
      expect(existsSync(join(root, path)), path).toBe(false);
    }
  });

  it('keeps runtime files free of retired asset references', () => {
    for (const file of runtimeFiles) {
      const contents = readFileSync(join(root, file), 'utf8');
      expect(contents, file).not.toMatch(/opengfx|openttd-fan-assets|assets:opengfx/i);
    }
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
npm test -- tests/render/noRetiredAssets.test.ts
```

Expected: FAIL while retired files/directories still exist.

- [ ] **Step 3: Delete retired files**

Run:

```bash
git rm -r public/opengfx2 public/openttd-fan-assets
git rm scripts/import-opengfx-assets.mjs scripts/decode-openttd-fan-grfs.mjs src/assets/opengfxCatalog.ts src/assets/opengfxCatalog.generated.ts tests/render/opengfxCatalog.test.ts
```

- [ ] **Step 4: Run guard test**

Run:

```bash
npm test -- tests/render/noRetiredAssets.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

Run:

```bash
git add -A public/opengfx2 public/openttd-fan-assets scripts/import-opengfx-assets.mjs scripts/decode-openttd-fan-grfs.mjs src/assets/opengfxCatalog.ts src/assets/opengfxCatalog.generated.ts tests/render/opengfxCatalog.test.ts tests/render/noRetiredAssets.test.ts package.json
git commit -m "chore: remove retired OpenTTD asset pipeline"
```

### Task 5: Full Verification

**Files:**
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/public/simutrans-assets/pak128/README.md`

- [ ] **Step 1: Confirm pak128 provenance**

Ensure `/Users/ramonfuglister/Desktop/Coding/abutown/public/simutrans-assets/pak128/README.md` records:

```md
# Simutrans pak128 Assets

Source: https://github.com/simutrans/pak128.git
Revision: `acdf2f0793a6beee5ea34ea85d308fbbeccf50c5`

This directory contains the pak128 source PNG and DAT files used by Abutown's runtime renderer.

License: Artistic License 2.0 unless an imported DAT file declares otherwise. See `LICENSE.txt` and individual DAT files.
```

- [ ] **Step 2: Run full verification**

Run:

```bash
npm test
npm run build
npm run test:e2e
rg -n "opengfx2|openttd-fan-assets|assets:opengfx|opengfxCatalog|import-opengfx|decode-openttd" src tests scripts package.json public
```

Expected: tests, build, and e2e PASS. The `rg` command prints no matches.

- [ ] **Step 3: Commit final provenance changes if needed**

Run:

```bash
git add public/simutrans-assets/pak128/README.md
git commit -m "docs: record pak128 runtime provenance"
```

Skip this commit if Task 2 already produced the final README content.

## Self-Review Notes

- This plan intentionally rejects mixed OpenTTD/OpenGFX and pak128 runtime rendering.
- The catalog must cover every runtime role before the renderer cutover can pass.
- The old OpenTTD/OpenGFX asset directories, scripts, generated catalogs, and tests are deleted after the renderer no longer uses them.
- Verification includes a guard test plus an `rg` check so retired asset references cannot drift back in unnoticed.
