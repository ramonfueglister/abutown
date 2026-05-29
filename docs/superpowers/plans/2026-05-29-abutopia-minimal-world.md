# Abutopia Minimal World Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Zurich world entirely with **abutopia** — a tiny deterministic sandbox (16×8 grass, one straight 10-tile road, a house at each end, one walking pedestrian) — keeping all generic world/terrain machinery and removing only Zurich-specific generation, data, and naming.

**Architecture:** A world is a base-world bundle (`manifest.json` + 5 layer files) loaded by `BaseWorldBundle::load_from_dir`; the backend builds the routing graph from `transport.json`, seeds mobility from `spawns.json`, and streams state to the frontend. We hand-author abutopia via a small self-contained generator, repoint all world constants to it, rewrite the Zurich-coupled tests for abutopia's tiny content, then delete the dead Zurich code/data.

**Tech Stack:** Rust (sim-core/sim-server, cargo), TypeScript (Vite/Vitest/Playwright), Node generator script.

**Spec:** `docs/superpowers/specs/2026-05-29-abutopia-minimal-world-design.md`

**Branch / isolation (codex):** Run in your own worktree on a fresh branch off the latest `main` (after the `mobility/systems.rs` refactor has merged):
`git worktree add ../abutown-abutopia -b codex/abutopia-world main && cd ../abutown-abutopia`
Set `export CARGO_TARGET_DIR=/tmp/abutown-abutopia-target` so your cargo never shares a build lock with other agents. **Route every cargo command through `scripts/cargo-serial.sh`** (e.g. `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core`). Never run two cargo at once.

## Abutopia content (the single source of truth)

- World: `world_id: "abutopia"`, `display_name: "Abutopia"`, `schema_version: 1`, `chunk_size: 32`, `world_tiles: { width: 16, height: 8 }`.
- Road: row `y=3`, tiles `x=3..=12` (10 tiles), `kind: "street"`. Masks (N=1,E=2,S=4,W=8): `x=3`→`2` (E), `x=4..=11`→`10` (E+W), `x=12`→`8` (W).
- Houses: building footprints at `(2,3)` and `(13,3)` (adjacent to the road ends).
- Pedestrian corridor `corridor:main`: points `(2,3),(3,3),(4,3),…,(13,3)` (12 points, house→road→house).
- Spawns: one `pedestrian_group` `{ id:"spawn:ped:main", corridor_id:"corridor:main", agents_per_corridor:1 }`. No cars, no trams.
- Terrain: `tiles: []` (everything grass; road/building tile-kinds derive from the transport/buildings layers).

---

## Task 1: Author the abutopia world (generator + data + load test)

**Files:**
- Create: `scripts/generate-abutopia-world.mjs`
- Create (generated): `data/worlds/abutopia/manifest.json` + `data/worlds/abutopia/layers/{terrain,transport,buildings,spawns,decorations}.json`
- Test: `backend/crates/sim-core/tests/abutopia_bundle.rs`

- [ ] **Step 1: Write the failing bundle-load test**

Create `backend/crates/sim-core/tests/abutopia_bundle.rs`:

```rust
use sim_core::base_world::BaseWorldBundle;

fn abutopia_root() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR = backend/crates/sim-core → repo root is 3 levels up.
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("workspace root")
        .join("data/worlds/abutopia")
}

#[test]
fn loads_abutopia_base_world() {
    let bundle = BaseWorldBundle::load_from_dir(abutopia_root())
        .expect("abutopia bundle loads");
    assert_eq!(bundle.world_id(), "abutopia");
    assert_eq!(bundle.chunk_size(), 32);
    assert_eq!(bundle.transport.roads.len(), 10);
    assert_eq!(bundle.buildings.footprints.len(), 2);
    assert_eq!(bundle.transport.pedestrian_corridors.len(), 1);
    assert_eq!(bundle.spawns.pedestrian_groups.len(), 1);
    assert_eq!(bundle.spawns.pedestrian_groups[0].agents_per_corridor, 1);
    assert!(bundle.spawns.car_groups.is_empty());
}
```

- [ ] **Step 2: Run it, verify FAIL**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --test abutopia_bundle`
Expected: FAIL — `data/worlds/abutopia` does not exist yet.

- [ ] **Step 3: Write the generator**

Create `scripts/generate-abutopia-world.mjs` (self-contained — no `src/city/*` imports):

```js
#!/usr/bin/env node
import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';

const worldId = 'abutopia';
const schemaVersion = 1;
const root = resolve('data/worlds', worldId);
const width = 16;
const height = 8;
const chunkSize = 32;
const roadY = 3;
const roadX0 = 3;
const roadX1 = 12; // inclusive → 10 tiles
const houseAX = 2;
const houseBX = 13;

const N = 1, E = 2, S = 4, W = 8;
const roads = [];
for (let x = roadX0; x <= roadX1; x++) {
  let mask = 0;
  if (x > roadX0) mask |= W;
  if (x < roadX1) mask |= E;
  roads.push({ x, y: roadY, kind: 'street', mask });
}

const corridorPoints = [];
for (let x = houseAX; x <= houseBX; x++) corridorPoints.push({ x, y: roadY });

const manifest = {
  schema_version: schemaVersion,
  world_id: worldId,
  display_name: 'Abutopia',
  chunk_size: chunkSize,
  world_tiles: { width, height },
  layers: {
    terrain: 'layers/terrain.json',
    transport: 'layers/transport.json',
    buildings: 'layers/buildings.json',
    decorations: 'layers/decorations.json',
    spawns: 'layers/spawns.json',
  },
};

const terrain = { schema_version: schemaVersion, world_id: worldId, tiles: [] };

const transport = {
  schema_version: schemaVersion,
  world_id: worldId,
  roads,
  rails: [],
  arterial_paths: [],
  rail_paths: [],
  pedestrian_corridors: [{ id: 'corridor:main', points: corridorPoints }],
};

const buildings = {
  schema_version: schemaVersion,
  world_id: worldId,
  footprints: [
    { id: 'building:house-a', tiles: [{ x: houseAX, y: roadY }], sheet: 'oldhouses', frame: 0 },
    { id: 'building:house-b', tiles: [{ x: houseBX, y: roadY }], sheet: 'oldhouses', frame: 1 },
  ],
};

const spawns = {
  schema_version: schemaVersion,
  world_id: worldId,
  pedestrian_groups: [
    { id: 'spawn:ped:main', corridor_id: 'corridor:main', agents_per_corridor: 1 },
  ],
  car_groups: [],
  tram_lines: [],
};

const decorations = { schema_version: schemaVersion, world_id: worldId, trees: [], details: [] };

async function main() {
  await mkdir(resolve(root, 'layers'), { recursive: true });
  const write = (rel, obj) => writeFile(resolve(root, rel), JSON.stringify(obj, null, 2) + '\n');
  await write('manifest.json', manifest);
  await write('layers/terrain.json', terrain);
  await write('layers/transport.json', transport);
  await write('layers/buildings.json', buildings);
  await write('layers/spawns.json', spawns);
  await write('layers/decorations.json', decorations);
  console.log(`✓ wrote ${root}`);
}

main();
```

- [ ] **Step 4: Add an npm script and generate the data**

Add to `package.json` `scripts` (after `generate:base-world`):
```json
    "generate:abutopia": "node scripts/generate-abutopia-world.mjs",
```
Run: `npm run generate:abutopia`
Expected: writes `data/worlds/abutopia/` with the 6 files.

- [ ] **Step 5: Run the load test, verify PASS**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --test abutopia_bundle`
Expected: PASS.

- [ ] **Step 6: Verify the bundle seeds exactly one pedestrian and no cars**

The serialized BaseWorldBundle layer structs are validated by the loader. Add to `abutopia_bundle.rs`:
```rust
#[test]
fn abutopia_seeds_one_pedestrian_corridor() {
    let bundle = BaseWorldBundle::load_from_dir(abutopia_root()).unwrap();
    let group = &bundle.spawns.pedestrian_groups[0];
    assert_eq!(group.corridor_id, "corridor:main");
    let corridor = bundle
        .transport
        .pedestrian_corridors
        .iter()
        .find(|c| c.id == group.corridor_id)
        .expect("referenced corridor exists");
    assert_eq!(corridor.points.len(), 12);
}
```
Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --test abutopia_bundle`
Expected: PASS (both tests).

- [ ] **Step 7: Commit**

```bash
git add scripts/generate-abutopia-world.mjs package.json data/worlds/abutopia backend/crates/sim-core/tests/abutopia_bundle.rs
git commit -m "feat(world): author abutopia minimal base world + generator

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Make abutopia the backend default + rewrite Zurich-coupled backend tests

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs:50-51`
- Modify: `backend/crates/sim-server/src/app.rs:46`, `app.rs:1465`
- Modify (tests): `backend/crates/sim-server/tests/http.rs`, `backend/crates/sim-server/tests/websocket.rs`, `backend/crates/sim-core/tests/base_world_bundle.rs`, `backend/crates/sim-core/src/persistence.rs` (string literals), `backend/crates/sim-server/src/runtime.rs` (its `#[cfg(test)]` module)

- [ ] **Step 1: Repoint the constants**

In `backend/crates/sim-server/src/runtime.rs`:
```rust
const WORLD_ID: &str = "abutopia";
pub const BASE_WORLD_DEFAULT_PATH: &str = "data/worlds/abutopia";
```
In `backend/crates/sim-server/src/app.rs:46`:
```rust
const BASE_WORLD_DEFAULT_PATH: &str = "data/worlds/abutopia";
```
In `backend/crates/sim-server/src/app.rs:1465`, change the hardcoded `WorldId("zurich-river-city-v1")` to `WorldId("abutopia")`.

- [ ] **Step 2: Update `base_world_bundle.rs`**

Rename the test and repoint it from zurich to abutopia (path `data/worlds/abutopia`, `assert_eq!(bundle.world_id(), "abutopia")`). If the test asserts zurich-specific tile/chunk counts, replace them with abutopia's (10 roads, 2 buildings, 16×8). Keep the test's intent (a real bundle loads), just on abutopia. (`abutopia_bundle.rs` from Task 1 already covers abutopia; if `base_world_bundle.rs` becomes a duplicate, delete it instead.)

- [ ] **Step 3: Update `http.rs` and `websocket.rs` assertions**

Replace every `"zurich-river-city-v1"` literal with `"abutopia"`, and replace zurich-content assertions with abutopia's tiny numbers. Known sites:
- `http.rs:79,92,120,142,283,356,563` — world_id strings / path → abutopia.
- `http.rs:358` `assert!(mobility.agents.len() >= 50)` → `assert_eq!(mobility.agents.len(), 1)`.
- `websocket.rs:55,165,202,260` — world_id strings → abutopia.
- Any chunk-coordinate the tests subscribe to that assumed a central zurich chunk → use a chunk that contains abutopia's content (chunk `(0,0)`, since the 16×8 world is one chunk).

Read each file, change the literals/counts, keep the assertion structure.

- [ ] **Step 4: Update `persistence.rs` and `runtime.rs` test literals**

In `persistence.rs` the `"zurich-river-city-v1"` strings are arbitrary world ids in unit tests — replace with `"abutopia"`. In `runtime.rs`'s `#[cfg(test)]` module, repoint the fixture root to `data/worlds/abutopia`, drop/replace the "central Zurich chunk" assertion with an abutopia chunk `(0,0)` assertion, and for tests that load `data/city/zurich-network.json` directly: those test the `CityNetwork` loader — either delete them (the loader is still covered by `city_network.rs`'s own unit test) or point them at a tiny inline network. Prefer deletion if they only re-prove the loader on zurich data.

- [ ] **Step 5: Verify backend green**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace`
Expected: PASS. Then:
Run: `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(world): default backend to abutopia, repoint backend tests

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Extract shared frontend types, repoint frontend to abutopia

**Files:**
- Create: `src/render/worldRuntimeTypes.ts` (or `src/app/worldRuntimeTypes.ts`)
- Modify: `src/city/worldTypes.ts` (rename `Zurich*` → generic, drop gen-only types)
- Modify: `src/render/minimalMapRenderer.ts`, `src/main.ts`, `src/backend/baseWorldClient.ts`
- Modify (tests): `tests/app/appRuntime.test.ts`, `tests/app/mainComposition.test.ts`, `tests/app/baseWorldBundle.test.ts`, `tests/app/noDemoWorldAuthority.test.ts`, `tests/app/noProductionFallbacks.test.ts`

- [ ] **Step 1: Extract the renderer's runtime types out of `zurichRuntimeContext.ts`**

`minimalMapRenderer.ts` imports `RuntimeBuilding` (and possibly sibling runtime types) from `../app/zurichRuntimeContext`. Move those still-needed runtime type declarations into a new generic module `src/render/worldRuntimeTypes.ts`, and re-point `minimalMapRenderer.ts` to import them from there. Do NOT move the zurich generation logic — only the type declarations the renderer needs.

- [ ] **Step 2: De-Zurich `worldTypes.ts`**

In `src/city/worldTypes.ts`, rename the still-used types to generic names and delete the gen-only ones:
- Keep + rename: `ZurichTerrainKind`→`TerrainKind`, `ZurichDetail`→`WorldDetail` (these are imported by `minimalMapRenderer.ts` and `main.ts`). Keep `Coord` and the helpers `key/parseKey/inside/distance`.
- Delete (gen-only, will be removed with the gen code in Task 4): `ZurichZoneKind`, `ZurichZone`, `ZurichTerrainTile`, `ZurichWorld`, `ZurichRoadKind`, `ZurichRoadTile`, `ZurichRailTile`, `ZurichBuildingSheet`, `ZurichBuilding`, `ZurichValidationResult` — but only once Task 4 deletes their consumers. For THIS task, leave them in place if `src/city/zurich*.ts` still imports them (they're deleted in Task 4); just do the renames of the kept types and update importers.

Update `minimalMapRenderer.ts` and `main.ts` to import `TerrainKind`/`WorldDetail`.

- [ ] **Step 3: Repoint `main.ts`**

In `src/main.ts`:
```ts
let worldId = 'abutopia';
let WIDTH = 16;
let HEIGHT = 8;
let chunkSize = 32;
```
(These are reassigned from the loaded base world at boot, but the defaults must not name zurich.)

- [ ] **Step 4: Repoint `baseWorldClient.ts` validation to abutopia**

In `src/backend/baseWorldClient.ts`, change the world-id guard (`:74`) to `'abutopia'`, and replace the hardcoded zurich dimension/content checks (`:78-87`: 256×256, chunk 32, 3 arterials, 160 corridors, 1800+ roads, 2250+ buildings, 3000+ trees, 256 rails) with abutopia's: width 16, height 8, chunk_size 32, 0 arterial paths, 1 pedestrian corridor, 10 roads, 2 buildings, 0 trees, 0 rails. Keep the validation *shape*; change the expected numbers.

- [ ] **Step 5: Update the frontend tests**

In `tests/app/appRuntime.test.ts`, `mainComposition.test.ts`, `baseWorldBundle.test.ts`, `noDemoWorldAuthority.test.ts`, `noProductionFallbacks.test.ts`: replace zurich world-id/dimension expectations with abutopia's (id `abutopia`, 16×8, 10 roads, 2 buildings, 1 pedestrian). Read each file and adapt assertions to the abutopia content; keep each test's intent.

- [ ] **Step 6: Verify frontend green**

Run: `npm run typecheck && npm test`
Expected: both pass.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(world): repoint frontend to abutopia, extract shared world types

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Delete the dead Zurich code, data, and generators

**Files (delete — only after proving dead):**
- `src/city/zurichPlacement.ts`, `zurichTransport.ts`, `zurichValidation.ts`, `zurichWorld.ts`, `src/app/zurichRuntimeContext.ts`
- `tests/app/zurichRuntimeContext.test.ts`, `tests/city/zurichPlacement.test.ts`, `tests/city/zurichTransport.test.ts`, `tests/city/zurichWorld.test.ts`
- `data/worlds/zurich-river-city-v1/`, `data/city/zurich-network.json`, `artifacts/abutown-zurich-river-city-2026-05-14.png`
- `scripts/generate-city-network.mjs`, `scripts/generate-base-world.mjs`
- the now-orphaned gen-only types in `src/city/worldTypes.ts` (from Task 3 Step 2)

- [ ] **Step 1: Prove the zurich gen files are dead**

Run: `rg -n "zurichRuntimeContext|zurichWorld|zurichPlacement|zurichTransport|zurichValidation" src tests --glob '!src/city/zurich*' --glob '!src/app/zurichRuntimeContext.ts'`
Expected: no live importers outside the zurich files themselves (Task 3 should have removed renderer/main imports). If anything remains, fix that importer first.

- [ ] **Step 2: Delete the gen files, tests, data, generators**

```bash
git rm src/city/zurichPlacement.ts src/city/zurichTransport.ts src/city/zurichValidation.ts src/city/zurichWorld.ts src/app/zurichRuntimeContext.ts
git rm tests/app/zurichRuntimeContext.test.ts tests/city/zurichPlacement.test.ts tests/city/zurichTransport.test.ts tests/city/zurichWorld.test.ts
git rm -r data/worlds/zurich-river-city-v1 data/city
git rm artifacts/abutown-zurich-river-city-2026-05-14.png
git rm scripts/generate-city-network.mjs scripts/generate-base-world.mjs
```

- [ ] **Step 3: Remove orphaned gen-only types + npm scripts**

Delete the gen-only `Zurich*` types from `src/city/worldTypes.ts` (left in place during Task 3). Remove the `generate:base-world` and `generate:city-network` entries from `package.json` `scripts`.

- [ ] **Step 4: Prove nothing references the deletions**

Run: `git grep -il zurich -- ':!docs' ':!progress.md'`
Expected: empty (only docs/progress retain historical zurich mentions).
Run: `npm run typecheck && npm test`
Expected: both pass.
Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "chore(world): delete dead zurich gen code, data, and generators

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Rewrite the browser smoke for abutopia

**Files:**
- Modify: `tests/e2e/render-smoke.spec.ts`

- [ ] **Step 1: Rewrite the smoke assertions for abutopia**

Replace zurich-specific expectations with abutopia's: the world renders (canvas non-empty), exactly one road line and two houses are present, and the mobility stream carries exactly one backend-driven pedestrian that moves over time (sample its position across two frames and assert it changed). Remove assertions about zurich's agent/car counts and any retired-asset checks tied to zurich content. Keep the structure of the existing smoke (boot dev stack, drive the browser, read `window.render_game_to_text?.()`).

- [ ] **Step 2: Run the smoke**

Run (set the env the CI e2e job uses):
```bash
CORS_ALLOWED_ORIGINS=http://127.0.0.1:5173 npm run build && CORS_ALLOWED_ORIGINS=http://127.0.0.1:5173 npx playwright test tests/e2e/render-smoke.spec.ts
```
Expected: PASS — one pedestrian visibly walks the abutopia road.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/render-smoke.spec.ts
git commit -m "test(e2e): rewrite render smoke for abutopia minimal world

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Final verification

- [ ] `git grep -il zurich -- ':!docs' ':!progress.md'` returns empty.
- [ ] `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check` clean.
- [ ] `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings` clean.
- [ ] `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace` passes.
- [ ] `npm run typecheck && npm test` pass.
- [ ] The app boots on abutopia and one pedestrian walks the road end-to-end (the back-and-forth behaviour is the next movement-system feature to build *on* this world — out of scope here).
- [ ] Use `superpowers:finishing-a-development-branch` to integrate.

## Notes for the movement-system work that follows

The current seed (`mobility/seed.rs::seed_pedestrians_from_bundle`) spawns the agent walking the corridor end-to-end once; it does **not** loop back. "House A ↔ House B back and forth" is the first movement-system feature to develop on abutopia — it belongs to that follow-up work, not this world-replacement plan.
