# Abutopia Minimal World Design

Date: 2026-05-29

## Status

Approved in brainstorming (direction confirmed by the user: replace the Zurich
world entirely with a minimal sandbox named **abutopia**). This is its own
spec → plan → implementation cycle, intended to be executed by the parallel
**codex** agent **after** it finishes the `mobility/systems.rs` refactor
(`codex/split-mobility-systems`). The in-flight security/CI plan
(`plan/security-ci-guardrails`) is finished first, on the current world, so it
stays independently shippable.

## Goal

Replace the Zurich world (`zurich-river-city-v1`) — code, data, naming, and
tests — with **abutopia**, a deliberately tiny, deterministic world used to
develop the agent movement system in isolation. Abutopia becomes the project's
only world. Its initial content:

```
. . . . . . . . . . . . . . . .
. . . . . . . . . . . . . . . .
. . . . . . . . . . . . . . . .
. . H R R R R R R R R R R H . .      grass everywhere; one straight
. . . . . . . . . . . . . . . .      10-tile road; a house adjacent to
. . . . . . . . . . . . . . . .      each end; one pedestrian.
. . . . . . . . . . . . . . . .
```

- `.` grass, `R` road (10 tiles, straight), `H` house at each road end.
- **One** pedestrian agent walks house A → road → house B and back,
  deterministically. (The count is a single number in the generator; trivial to
  raise later. Crowd/LOD behaviour is explicitly NOT what abutopia is for.)

This gives a clean stage to observe and develop walking/pathing one entity at a
time, without Zurich's ~1011-agent procedural complexity.

## Why "replace", not "add alongside"

The user's intent is that Zurich is gone ("Müll"), not kept behind an env var.
Abutopia takes Zurich's place as the default and only world. There is no
dual-world switching to maintain.

## Naming

- world_id: `abutopia`
- display_name: `Abutopia`
- world dir: `data/worlds/abutopia/`
- Versioning stays in `schema_version` (as today), not in the id, so the id
  doesn't churn as the world content evolves.

## Architecture

A world is authored as a base-world bundle: `manifest.json` plus five layer
files (`terrain`, `transport`, `buildings`, `spawns`, `decorations`) under
`data/worlds/<id>/layers/`, loaded by `BaseWorldBundle::load_from_dir`. The
runtime is backend-authoritative: the server loads the bundle, builds the
routing graph from `transport.roads`, seeds mobility from `spawns`, and streams
state to the frontend. The frontend renders what the backend serves.

Abutopia is authored by a small, self-contained generator and made the default;
all Zurich-specific authoring code is deleted.

### Layer contents (abutopia)

World size **16×8**, `chunk_size 32` (one chunk). Coordinates below are the
proposal; the generator computes them.

- **manifest.json** — `world_id: "abutopia"`, `display_name: "Abutopia"`,
  `schema_version: 1`, `chunk_size: 32`, `world_tiles: { width: 16, height: 8 }`.
- **terrain.json** — grass. (The plan resolves whether terrain lists every tile
  or only non-default tiles by reading how the loader fills a chunk; the
  generator emits whichever the loader requires.)
- **transport.json** — `roads`: 10 `street` tiles in a row (e.g. y=3, x=3..12)
  with correct east–west connectivity `mask`s (endpoints connect one way, middle
  tiles both ways).
- **buildings.json** — two single-tile `footprints`, one adjacent to each road
  end (e.g. (2,3) and (13,3)).
- **spawns.json** — one `pedestrian_group` referencing a single corridor that
  runs house A → road → house B, `agents_per_corridor: 1`.
- **decorations.json** — empty.

### Generator

`scripts/generate-flat-test-world.mjs` → writes the five layers + manifest for
`data/worlds/abutopia/`. Self-contained (computes coordinates and road masks in
code); does **not** depend on any `src/city/*` code. Replaces the Zurich
generators.

## Guiding principle: remove Zurich, keep the world machinery

The generic world/terrain infrastructure is **reused** by abutopia and must NOT
be deleted. Only Zurich-specific *procedural generation*, *data*, and *naming*
go. Deletion is driven by "what is provably dead after repointing to abutopia"
(proven by `tsc`/`cargo build` + `git grep`), never by filename pattern.

### Keep (generic infrastructure — do NOT delete)

- Backend: `BaseWorldBundle` + the five-layer format, the routing-graph builder
  (builds from `transport.roads`), mobility seeding (from `spawns`), the
  chunk/LOD system, persistence, the protobuf wire.
- Frontend: the renderer (`src/render/*`, terrain/road/building drawing), the
  terrain-**kind** concept (grass/water/road/…), camera, mobility client.
- The terrain/world TYPES in `src/city/worldTypes.ts` — these describe terrain
  kinds and tiles the renderer and the abutopia generator both need.

### De-Zurich (rename, do NOT delete)

- `src/city/worldTypes.ts` — keep the types, rename the `Zurich*` identifiers to
  generic names (`ZurichTerrainKind` → `TerrainKind`, `ZurichWorld` → `World`,
  etc.) and update importers. This satisfies "no `zurich` naming" without losing
  the terrain machinery.
- Any other shared symbol that merely carries `zurich` in its name (not its
  purpose) gets a generic rename rather than deletion.

### Delete (genuinely Zurich-specific, dead after repointing)

- Procedural city-gen for Zurich's specific layout:
  `src/city/zurichPlacement.ts`, `zurichTransport.ts`, `zurichValidation.ts`,
  `zurichWorld.ts`, `src/app/zurichRuntimeContext.ts`, and their tests
  (`tests/app/zurichRuntimeContext.test.ts`, `tests/city/zurich*.test.ts`).
  Only after confirming nothing abutopia needs imports them.
- Data: `data/worlds/zurich-river-city-v1/`,
  `artifacts/abutown-zurich-river-city-2026-05-14.png`, and
  `data/city/zurich-network.json` (subject to the city_network open question).
- Generators: `scripts/generate-city-network.mjs`,
  `scripts/generate-base-world.mjs` (Zurich-driven) — replaced by the abutopia
  generator.

## Components to repoint (Zurich id/path → abutopia)

- `backend/crates/sim-server/src/runtime.rs` — `WORLD_ID`,
  `BASE_WORLD_DEFAULT_PATH` (lines ~50–51).
- `backend/crates/sim-server/src/app.rs` — `BASE_WORLD_DEFAULT_PATH` (line 46),
  hardcoded world_id (line ~1465).
- `src/backend/baseWorldClient.ts` — the hardcoded
  `payload.world_id !== 'zurich-river-city-v1'` guard (line ~74).
- `src/main.ts` — `worldId` and world dimensions.

## Tests to rewrite for abutopia's minimal content (the bulk of the work)

- Backend: `tests/base_world_bundle.rs` (`loads_zurich_base_world_fixture` →
  abutopia), `tests/http.rs` (world_id asserts; `mobility.agents.len() >= 50` →
  the abutopia count), `tests/websocket.rs` (world_id asserts; chunk/entity
  expectations), `runtime.rs` unit tests (Zurich fixture + "central Zurich
  chunk" + `zurich-network.json` loads), `persistence.rs` (Zurich world-id
  string literals → a neutral/abutopia id), and `city_network.rs` (see open
  question).
- Frontend: `tests/app/appRuntime.test.ts`, `mainComposition.test.ts`,
  `baseWorldBundle.test.ts`, `noDemoWorldAuthority.test.ts`,
  `noProductionFallbacks.test.ts`.
- E2E: `tests/e2e/render-smoke.spec.ts` — rewrite to assert abutopia's content
  (one road, two houses, one walking pedestrian, backend-driven movement)
  instead of Zurich's seed.

## New tests

- A bundle-load test that loads `data/worlds/abutopia/`, asserts
  `world_id == "abutopia"`, the expected tile/road/building counts, and that
  seeding produces exactly the configured pedestrian count.
- The app boots on abutopia (server starts, serves the world summary).

## Open question (resolve during planning)

Whether the `city_network` machinery (the separate network JSON +
`city_network.rs`) is still needed, or whether the routing graph can be built
directly from `transport.roads` for abutopia. If unused, `city_network.rs`,
`generate-city-network.mjs`, and `data/city/` are deleted too; if the routing
builder still requires a network input, the generator emits a minimal one.

## Non-goals

- Procedural/complex world generation — abutopia stays minimal until movement
  work needs more.
- Rewriting historical docs (`docs/superpowers/specs|plans`, `progress.md`).
  Those record what was true at the time and are left as history.
- Crowd/LOD/stress scenarios.

## Method & sequencing

- TDD where there is behaviour to assert (bundle load, seed count, app boot,
  render smoke). Test rewrites are verification-gated.
- All `cargo` commands route through `scripts/cargo-serial.sh`; codex runs in its
  own worktree with its own target dir, so no build-lock contention.
- Order: (1) author abutopia generator + data, (2) make it the default +
  repoint constants, (3) rewrite the coupled tests, (4) delete the dead Zurich
  code/data/generators, (5) rewrite the e2e smoke. Deletion comes after the
  repoints/rewrites compile green, so each step stays shippable.

## Success criteria

- No `zurich` references remain in non-doc files (`git grep -il zurich` returns
  only `docs/` + `progress.md`).
- `data/worlds/abutopia/` loads; the app boots on it with one walking pedestrian
  visible end-to-end.
- `cargo test --workspace`, `cargo clippy --workspace --all-targets -D warnings`,
  `npm run typecheck`, `npm test`, and the rewritten e2e smoke all pass.
