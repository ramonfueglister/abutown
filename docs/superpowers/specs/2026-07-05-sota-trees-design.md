# SOTA Trees for the Winterthur Renderer — Design

Date: 2026-07-05
Status: approved design, pre-implementation

## Problem

Trees currently look uniform and out of scale:

- The renderer (`src/diorama/ksw/geo/nature.ts`) has exactly **two archetypes** —
  broadleaf = one fixed 4-puff icosahedron merge, conifer = one fixed 2-cone
  stack. Per-tree variation is only ±15% y-squash plus a slight hue/lightness
  nudge; silhouettes are identical across thousands of instances.
- Scale is a **data problem**: `treeSpec` (`scripts/geo/lib/style.mjs`) uses OSM
  height/crown tags only when present, which is almost never. Nearly every tree
  gets the defaults (broad h=9 m / r=3 m, conifer h=14 m / r=2 m), and
  `forestFill` scatters thousands more identical default broadleafs.

## Goal

Tens of thousands of individually varied, species-shaped, scale-accurate trees
at real geodetic positions, in the existing clay / Nova-Roma-adjacent stylized
look, within the established performance budget (100–120 fps with the 10k-agent
pipeline) and a handful of draw calls.

Non-goals: photorealism (no gaussian-splat / photogrammetry vegetation), moving
or inventing tree positions (except the pre-existing forest fill), replacing
OSM data.

## User decisions (recorded)

- Full package: real species data + procedural archetype library + shader
  variation. OSM tree positions are kept — every tree stands at its real
  surveyed location.
- Wind: yes, coupled to the existing live-weather system.
- Three hardenings are in scope, not optional: octahedral impostors,
  GPU-side culling/LOD, and a dedicated screenshot-driven polish loop.

## 1. Data pipeline (bake)

**New source — Baumkataster Winterthur** (Stadtgrün Winterthur, via GDIW /
geocat.ch, opendata.swiss). Point data in LV95 (existing `project.mjs`
transformation applies). Per tree: coordinate, scientific species name,
planting year.

**Species mapping.** Scientific name → ~10 form families:

| family | examples | silhouette |
|---|---|---|
| broad-spreading | Platanus, Acer | wide layered crown |
| oval | Tilia, Fagus | tall oval crown |
| columnar | Populus nigra 'Italica', Quercus 'Fastigiata' | narrow column |
| light/open | Betula | small airy puffs, pale trunk |
| weeping | Salix | drooping crown (Eulach banks) |
| small-round | fruit trees (Malus, Prunus, Pyrus) | low ball |
| conic conifer | Picea, Abies | stacked cone rings |
| umbrella conifer | Pinus | bare trunk, flat crown |
| generic broad | fallback | current 4-puff look, improved |
| generic conifer | fallback | current cone stack, improved |

Unknown species fall back to the generic families and are logged with counts so
the mapping can be extended.

**Size from age.** `age = bakeYear − plantingYear`; per family a saturating
growth curve `h(age) = h∞ · age / (age + t½)` (same form for crown radius).
Per-family constants (h∞, r∞, t½) researched once and documented as a table in
the bake script. A freshly planted 2022 lime renders as a sapling; the station
plane trees render full-size.

**Merge order.**
1. Baumkataster (city-maintained street/park/avenue trees) — wins.
2. OSM `natural=tree` — kept in full for everything the Kataster doesn't cover
   (private ground). Points within 3 m of a Kataster tree are the same tree →
   dropped (reuse the existing 4 m spatial-hash grid mechanic from
   `forestFill`).
3. Forest fill — unchanged placement logic, but gets a deterministic species
   mix per area (beech/spruce/fir weighting from the area's OSM
   `leaf_type`/`landuse`) and size spread instead of uniform defaults.

**Format.** `TreeSpec` grows `family: u8` and `seed: u8` (seed derived
deterministically from the coordinate). Tile schema extension → one full
re-bake of the world pyramid; byte-determinism convention holds.

## 2. Archetype generator (boot-time, client)

Geometry is NOT baked into the 77 MB pyramid — it is generated at boot,
deterministically, from compact per-family parameters: **~10 families × 4 seeds
≈ 40 archetypes**. Generator vocabulary stays clay:

- Broadleaf: short seeded branch skeleton (2–3 recursion levels) + puff cluster
  at branch tips (6–12 icosahedron puffs, family-shaped envelope) instead of
  the fixed 4-puff layout.
- Conifers: stacked cone/ring profiles per family (conic vs umbrella).
- Weeping: puffs displaced downward along hanging guides.

Every archetype also yields a low-cost far representation with the **same
silhouette envelope** (hard rule from Task 14: LOD swaps must not pop).

## 3. Rendering (WebGPU / TSL)

- **BatchedMesh** replaces the five InstancedMeshes: all archetypes + trunks in
  a few batches; per-instance transform + tint as today. The Task-10 LOD ring's
  `getObjectByName` coupling (`lod.ts`) is migrated to the new node names.
- **Per-instance shader variation:** `instanceIndex`-seeded puff jitter and
  asymmetric squash in the vertex stage; two-tone crown gradient (lit top
  lighter — Nova Roma read) in the color node. No two trees identical, zero
  extra geometry or draw calls.
- **Wind:** TSL vertex sway — crown amplitude high, trunk near-zero, phase from
  world position; amplitude driven by the live-weather wind uniform from the
  existing environment system; a second slower noise octave for gusts.
- **Octahedral impostors (far field):** replace the low-poly far meshes with
  pre-rendered octahedral impostors — each archetype rendered once at boot from
  ~8×8 hemispherical view directions into a shared atlas; far trees draw as
  camera-facing quads sampling the view-blended atlas. Tint + squash still
  per-instance so near↔far agree.
- **GPU culling/LOD:** move the near/far classification into a compute pass
  (pattern already established by the 10k-agent LOD compute), writing
  per-instance visibility/LOD instead of CPU ring toggling. Near ring keeps
  shadow casting; far field never touches the shadow map.

## 4. Slicing

1. **Slice 1 — forms + wind (existing data):** archetype generator, BatchedMesh
   migration, per-instance TSL variation, weather-coupled wind, octahedral
   impostors, GPU culling/LOD. Immediate visual jump; `family` inferred from
   the current `kind` field.
2. **Slice 2 — real species + scale:** Baumkataster fetch/ingest, species
   mapping, growth curves, three-source merge, tile-format extension, full
   re-bake.
3. **Slice 3 — polish loop:** dedicated screenshot harness (à la
   `capture-visuals.mjs` / `capture-env.mjs`): establishing shot, street-level
   avenue, forest edge, golden hour. Iterate palette, gradient, puff
   proportions, wind amplitude against images until approved by image review.
   Beauty is an explicit deliverable with its own acceptance step, not a side
   effect.

## 5. Testing & verification

- Bake: unit tests for species mapping (incl. fallback logging), growth curve,
  Kataster×OSM dedup merge; count guard analogous to the existing
  `trees < 3000` check for a Kataster minimum.
- Client: determinism test (same seed → identical geometry hash); silhouette
  contract full↔impostor (bounding-envelope comparison per archetype).
- Perf: frame-time probe in the smoke — must hold the 10k-pipeline budget with
  tree count ≥ current bake.
- **Browser smoke is mandatory** (CLAUDE.md): headless-chromium harness per
  slice; screenshots as proof, not "tests green".

## 6. Risks

- **Kataster access** (format/endpoint/licence) is verified at the start of
  Slice 2; if it stalls, Slice 2 falls back to OSM + species heuristics from
  surrounding landuse, and the Kataster becomes a follow-up.
- Tile-format change forces a full world re-bake (~77 MB artifact churn) —
  scheduled once, at the end of Slice 2.
- BatchedMesh migration touches the Task-10 LOD name coupling — regression
  smoke on the LOD ring is part of Slice 1.
- Octahedral impostor atlas memory: 40 archetypes × 8×8 views needs atlas
  budgeting (target ≤ 64 MB texture); if tight, share views across seeds within
  a family.
