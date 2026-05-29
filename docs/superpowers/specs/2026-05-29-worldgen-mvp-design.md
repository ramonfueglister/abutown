# Procedural Worldgen MVP — Design

**Date:** 2026-05-29
**Status:** Approved (brainstorming gate)
**Scope:** Replace the hand-curated Zurich seed with a deterministic procedural
worldgen pipeline that produces a Mini-Metro-style 2D landscape (terrain +
biomes + rivers). Roads, transit, and POIs are explicitly out of MVP scope.

## Motivation

Today the world that `SimulationRuntime::new()` builds is fixed: either the
hard-coded `tiny_world()` (20 pedestrians + 4 trams) or the loaded Zurich city
network with a layered terrain seed. There is no concept of a parameterised,
reproducible, novel world per session.

The goal is a state-of-the-art-for-2026 generator that

- produces topologically realistic terrain (believable coastlines, branching
  rivers, plausible biome distribution),
- looks like Mini Metro at the macro level (flat color blocks, smooth coast
  silhouettes, single-line rivers, no shading or texture),
- is deterministic per `seed: u64` (single-threaded, no float-ordering drift,
  identical bytes for identical seed on any platform),
- is simple to integrate (one new module, one call site in `runtime.rs`),
- ships with the rest of the simulation untouched (mobility tick loop,
  routing, persistence, frontend rendering pipeline).

The chosen technique is the canonical Voronoi-polygon pipeline (Amit Patel /
Red Blob Games), proven over 15+ years to give the best ratio of
realistic topology to implementation simplicity for this exact aesthetic.

## Non-goals

- Procedural road, rail, or transit network generation (a later phase)
- POI / building / station placement (a later phase)
- Agent and vehicle population on the generated world (a later phase — the
  MVP world is intentionally empty of mobility entities)
- Hydraulic erosion, tensor-field roads, L-system cities, WFC, ML-based
  generation
- Lazy / on-demand chunk generation (all chunks are rastered at world start)
- Multiple worlds per server instance, world selection UI

## High-level architecture

A single new Rust module `sim-core/src/worldgen/` holds the entire pipeline.
It is stateless and side-effect-free: it takes a `WorldgenConfig` and returns
a `GeneratedWorld`. It does not touch the ECS, the tokio runtime, the
persistence layer, the protocol, or the frontend.

`sim-server/src/runtime.rs::SimulationRuntime::new()` gains exactly one new
call: where it currently constructs the Zurich-or-tiny world, it now calls
`worldgen::build(cfg)` and feeds the result into the same ECS components that
the Zurich path used to populate. The Zurich code path is removed.

```
WorldgenConfig
      │
      ▼
sim-core/src/worldgen/  →  GeneratedWorld
      │                        │
      │                        ├─ chunks: HashMap<ChunkCoord, ChunkTerrain>
      │                        ├─ rivers: Vec<RiverPolyline>
      │                        └─ biome_palette: BiomePalette
      │
      ▼
sim-server/src/runtime.rs::new()
      │
      ├─ feeds chunks into terrain ChunkStore (existing path)
      ├─ broadcasts rivers as new WorldGeometry WS message (one-shot)
      └─ exposes world_seed in WorldSummaryDto for debug/repro
```

## Module layout

```
sim-core/src/worldgen/
  mod.rs           public API: pub fn build(cfg: WorldgenConfig) -> GeneratedWorld
  config.rs        WorldgenConfig { seed, width_chunks, height_chunks,
                                    sea_level, noise_octaves, ... }
  mesh.rs          Voronoi mesh: Poisson-disk scatter + 2x Lloyd + Delaunay
  coast.rs         Simplex noise + radial falloff → Land/Ocean mask per cell
  elevation.rs     BFS from coast cells inland → f32 per cell
  moisture.rs      BFS from ocean (and seeded inland water) → f32 per cell
  rivers.rs        Downhill-walk from peaks → flow accumulation →
                   Vec<RiverPolyline> over a flow threshold
  biome.rs         Whittaker lookup: (elevation_band, moisture_band) → BiomeId
  raster.rs        Rasterise Voronoi cells onto the tile grid per chunk
```

Each sub-module is one phase, ~80–200 LOC, with its own `#[cfg(test)]`
block.

## New crate dependencies

Added to `backend/crates/sim-core/Cargo.toml`:

- `rand_chacha = "0.3"` — deterministic ChaCha8 PRNG seeded by `WorldgenConfig.seed`
- `delaunator = "1"` — Delaunay triangulation, Mapbox JS port
- `noise = "0.9"` — Simplex/OpenSimplex noise

Explicit rejections: `voronoice` (less mature), any erosion crate, `wfc`,
any GPU/ML dependency.

## Determinism contract

- A single `ChaCha8Rng::seed_from_u64(seed)` is constructed in `worldgen::build`
  and passed by `&mut` through every phase. No phase constructs its own RNG.
- All container iteration that affects outcomes uses ordered structures
  (`Vec`, `BTreeMap`, `VecDeque`). No `HashMap`/`HashSet` iteration in
  outcome-determining code paths.
- No `rayon`, no `tokio::spawn`, no parallel iteration inside the pipeline.
- Floating-point arithmetic is single-threaded and uses standard `f32`
  operations; no `--ffast-math`-style flags. Cross-platform bit-identity is
  asserted by the snapshot test.

## Pipeline data flow

```
WorldgenConfig
      │
      ▼  mesh::build()
VoronoiMesh { points, cells, edges, neighbors }      // ~5,000 cells
      │
      ▼  coast::classify(mesh, rng)
LandMask: Vec<bool>
      │
      ▼  elevation::assign(mesh, landmask)
Elevation: Vec<f32>                                   // BFS from coast
      │
      ▼  moisture::assign(mesh, elevation, landmask)
Moisture: Vec<f32>
      │
      ▼  rivers::trace(mesh, elevation, moisture)
Rivers: Vec<RiverPolyline>                            // vector polylines
      │
      ▼  biome::classify(elevation, moisture)
Biomes: Vec<BiomeId>
      │
      ▼  raster::to_chunks(mesh, biomes, chunk_size)
GeneratedWorld { chunks, rivers, biome_palette }
```

## Key design decisions and rationale

1. **Voronoi over pure-noise tile grid.** Smooth polygonal coastlines emerge
   for free; pure-noise rasters give pixelly coasts that fight the Mini Metro
   look.
2. **Rivers stay vectorial.** They are transmitted as polylines and rendered
   with constant stroke width on the frontend. This is the single biggest
   contributor to the Mini Metro feel.
3. **Coastlines are NOT transmitted as polylines** in the MVP. The boundary
   between water-biome and land-biome cells in the rasterised tile grid is
   sharp enough, and rendering anti-aliasing makes the silhouette read as
   smooth. Coastline polylines can be added later if needed.
4. **All chunks rastered at start.** Lazy generation is rejected for the MVP
   — one world representation, no mixed-state confusion.
5. **Default world size: 32 × 32 chunks** (2,048 × 2,048 tiles). Large enough
   for believable topology, small enough that `worldgen::build` finishes in
   well under a second on a single core.
6. **Degenerate-seed fallback.** After `coast::classify`, sanity-check that
   land cells form 20–60% of total cells. If not, re-seed with `seed + 1` and
   retry. Max 5 retries; then panic with a clear message naming the original
   seed.

## Protocol extensions

`backend/crates/protocol/src/lib.rs` gains:

```rust
// Existing struct, new fields:
pub struct WorldSummaryDto {
    // ...existing fields preserved...
    pub world_kind: WorldKindDto,        // NEW: "generated"
    pub world_seed: Option<u64>,         // NEW
    pub biome_palette: BiomePaletteDto,  // NEW: BiomeId -> RGB
}

// New one-shot init message, sent once after Hello over WebSocket:
pub struct WorldGeometryDto {
    pub rivers: Vec<RiverPolylineDto>,
}

pub struct RiverPolylineDto {
    pub points: Vec<[f32; 2]>,  // world coordinates
    pub width: f32,
}
```

Tick-loop traffic (`MobilityChunkDelta`, `TilePulse`, etc.) is **unchanged**.

## Frontend integration

- **`src/main.ts` is not modified.** It is under active work by another
  agent; we route around it.
- New file `src/render/worldGeometry.ts` — owns the `WorldGeometryDto` state
  and renders polylines as a Canvas layer. Exposes a `register()` function
  that the existing render pipeline calls from its frame hook.
- Existing `src/backend/mobilityState.ts:176` — the no-op branch for unknown
  WebSocket messages gains one arm for `WorldGeometry`, dispatching into
  `worldGeometry.ts`. One-line change.
- Z-order in the renderer: `biome_fill → river_stroke → agents → POIs → UI`.

## Persistence and compatibility

- Chunk snapshots gain a `world_kind` field in their header. Loading a
  snapshot whose `world_kind` does not match the configured
  `WorldgenConfig.world_kind` is **refused** with a clear log message; the
  server starts fresh.
- This intentionally breaks compatibility with any existing Zurich snapshot.
  Since the Zurich code path is removed in the same change, that is the
  correct behaviour — keeping incompatible snapshots loadable would be a
  trap.

## Empty mobility on MVP worlds

The MVP world has no road or rail graph and therefore no agent/vehicle
spawning. The simulation starts with a populated terrain and zero mobility
entities. This is a deliberate scope cut, documented prominently in the
runtime startup log and in the world summary. Mobility on generated worlds
is a separate, later spec.

## Testing strategy

1. **Unit tests per phase** (`#[cfg(test)]` in each sub-module):
   - `mesh`: identical seed ⇒ identical mesh, point by point
   - `coast`: across 100 seeds, ≥95% land-ratio in `[0.20, 0.60]`
   - `elevation`: every land cell has a strictly descending path to coast
   - `rivers`: every river ends in an ocean cell; no cycles
   - `biome`: every cell has a defined `BiomeId` (no `Unknown`)

2. **Snapshot test** (`sim-core/tests/worldgen_snapshot.rs`):
   - Seed `42` → SHA-256 of the serialised `GeneratedWorld` is pinned. A
     mismatch surfaces either a determinism break or an intentional pipeline
     change.

3. **Browser smoke** (`scripts/smoke-worldgen.mjs`):
   - Mandatory per project convention (`CLAUDE.md` browser-smoke rule —
     this change crosses the frontend↔backend boundary).
   - Launches the dev stack, asserts the WebSocket `WorldGeometry` frame
     arrives and a river-shaped path is drawn on the canvas (pixel sample
     at known river coordinates returns the water colour).

## Worktree workflow

The implementation happens inside an isolated git worktree (branch
`worldgen/mvp`) to avoid stepping on other agents currently editing
`src/main.ts`, `runtime.rs`, and `seed.rs`. The user merges the worktree
back into `main` manually after the other agents' work has landed and is
responsible for the merge conflict resolution. The spec document itself
(`docs/superpowers/specs/2026-05-29-worldgen-mvp-design.md`) is committed
on `main` because documentation does not conflict.

## Code that is removed

(all inside the worktree, invisible to other agents until merge):

- `sim-core/src/mobility/seed.rs::tiny_world` and the Zurich branch in
  `from_network`
- The layered Zurich terrain seed logic in
  `sim-server/src/runtime.rs::SimulationRuntime::new()`
- Any Zurich-specific assets discovered in `public/simutrans-assets/` — the
  exact list is identified during implementation by searching for "zurich"
  references
- Tests referencing `tiny_world()` — rewritten against the generated path or
  the worldgen unit tests

## Code that stays unchanged

- The mobility tick loop, routing plugins, persistence layers
- Protocol base structure (extensions are purely additive)
- `src/main.ts`
- The snapshot store on-disk format (only the `world_kind` header field is
  added)

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Persisted Zurich snapshots become unloadable | Detected by `world_kind` check; clear log message; fresh start |
| MVP world has no agents, looks "empty" | Documented as intentional scope; visible in startup log and `WorldSummary` |
| Determinism breaks across platforms | Single-threaded, no parallel iter, no `HashMap` in outcome paths; snapshot test guards |
| Merge conflict with other agents' work in `runtime.rs` / `seed.rs` | Worktree isolation; user owns merge resolution |
| Degenerate seed (all water / all land) | `coast::classify` sanity check; up to 5 re-seeds; then panic |

## Out of scope (explicit non-goals, restated)

- Procedural roads / rail / transit
- POIs, buildings, stations
- Agent / vehicle spawning on generated worlds
- Hydraulic erosion, tensor-field roads, L-systems, WFC, ML
- Lazy chunk generation
- Multiple worlds per server, world selection UI
- Frontend configuration of seed / size (server-config only for MVP)
