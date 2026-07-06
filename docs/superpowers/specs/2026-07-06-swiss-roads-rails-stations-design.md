# Swiss Roads, Rails & Stations — geodätisch korrekte Trassen mit VSS-Look

Date: 2026-07-06
Status: approved in brainstorming (user), staged delivery
Context: follows the drape fixes #131 (closed, superseded) / #132 (merged);
predecessors #119 (Gemeinde terrain), #121/#123/#127 (traffic), #129 (trees),
#132 (adaptive longitudinal drape + CS-style car variants).

## 1. Goal

Roads and rails that read as **built Swiss infrastructure**: corridors graded
into the terrain (embankments/cuts instead of ribbons painted on hillsides),
VSS-style markings (white centre/edge lines, the iconic yellow pedestrian
crossings), differentiated surfaces, proper rail geometry (steel rails,
sleepers, ballast profile, catenary masts), and stations with real platform
geometry from public geodata.

## 2. Problem measurements (2026-07-06, live net)

- #132 fixed **longitudinal** chord burial (adaptive bisection to 2.5 m).
- **Cross-slope burial is unfixed**: ribbons are planar across their width
  (`roads.ts` drapes both rails at the centreline height). Measured against
  the L2 DEM over all 2 057 road paths at 10 m steps: **5.8 % of cross-
  sections have a rail ≥ 30 cm off the ground, worst 1.49 m** (residential
  @ world [-347, 1073]). On hillsides the uphill edge digs in — the user-
  visible "Strassen versinken im Boden" residue.
- Roads today are single flat clay ribbons: no markings, no crossings, no
  surface differentiation. Rail is two flat bands (bed + ribbon).

## 3. Non-goals

- Bridges/underpasses as structures (decks, portals, abutments) — follow-up
  slice; noted where the grading pass must leave gaps (§4.3).
- Moving trains (transit.pb exists; separate slice).
- Switch/turnout geometry in the HB throat — tracks may overlap simplified.
- Catenary **wires** (masts yes; wires are sub-pixel at city distance).
- Car look (shipped separately in #132 as sedan/hatchback/van variants).
- Runtime traffic-sim changes: the lane-level sim geometry (trafficnet.json)
  is untouched; this slice is bake + rendering only.

## 4. Terrain grading pass (bake) — the geodetic core

New deterministic pass in the world bake (`scripts/geo/bake-world` pipeline),
run after DEM resample, before tile encode, applied consistently to every
pyramid level so LOD switches don't pop.

### 4.1 Road grading

For each drivable/foot way (from `osm-roads.json`, same source as the road
renderer): corridor = render width (post-#132 `correctRoadWidths` floor)
plus 1.5 m shoulder each side. Longitudinal target profile = centreline DEM
heights smoothed with a moving window (default 40 m) and clamped to a max
grade of 12 % (steep Winterthur lanes stay plausible; authored per-class
overrides possible in one constants table). DEM cells inside the corridor
are set to the profile height; a smoothstep blend returns to natural terrain
over 8 m beyond the shoulder.

### 4.2 Rail grading

Same mechanism, rail parameters: corridor = ballast bed width (ribbon width
+ 2.2 m) + 2 m shoulder, smoothing window 200 m, max grade 2.5 %. This is
what carves the classic embankment/cut silhouette of the Winterthur corridor.

### 4.3 Junctions and crossings

- Overlapping road corridors: cells take the **average of the overlapping
  profiles weighted by corridor-distance falloff** — junction aprons come out
  level and continuous.
- Road × rail at an OSM `railway=level_crossing`: rail profile wins; the
  road profile is forced to the rail height inside the rail corridor and
  blends out over its own blend zone.
- Road × rail **without** a level-crossing tag (real bridges/underpasses):
  rail wins; the road blends toward the rail profile (visually a ramp until
  the bridges slice lands). These sites are counted and logged by the bake.

### 4.4 World consistency (everything placed on the old DEM must follow)

- **Bake order**: grading runs BEFORE building/tree/prop placement so every
  placed object samples the graded DEM. No re-grounding shims afterwards —
  order, not patches.
- **Corridor clearing**: the pass exports the corridor mask; tree/prop
  scattering excludes it (no trees on carriageways).
- **Water is untouchable**: grading never modifies cells under water
  polygons (Töss, Eulach). A road/rail corridor crossing water without an
  OSM `bridge` tag is counted and logged (honest report for the bridges
  slice) — never silently graded through the riverbed.
- **Anchor pinning**: `anchorGroundHeight` is captured BEFORE grading so the
  global vertical anchor of the world cannot shift.
- **Traffic-lane coverage**: the road corridor width is the max of the
  render width and the trafficnet lane extents for that way, so vehicles
  (which sample `groundYAt` per position) always drive on graded ground.

### 4.5 Determinism & artifacts

Pure function of (DEM, osm-roads, constants): double-bake must be
byte-identical (golden test, same as #119/#123 discipline). Artifacts stay
gitignored (world/*.pb); the bake logs corridor cell counts and the
burial metric (§9) before/after.

## 5. Runtime road mesh — road-owned profiles (AMENDED 2026-07-06)

**Amendment (user decision "A+B", after the §9 measurement failed):** the
L2 tile heightfield samples every ~12.5 m and cannot represent a ~6 m
carriageway bench, regardless of bake-side grading resolution (measured:
max 2.55 m / p99 0.95 m / 8.07 % ≥ 0.3 m on the 10 m grading grid — worse
than baseline). Therefore, like every current city builder, roads own
their surface height AND the terrain conforms toward it:

- **A (terrain):** the grading grid drops to **2.5 m** (sparse
  accumulators); tiles keep their resolution — grading now removes the
  bulk of the conflict (embankments/cuts remain visible).
- **B (profile):** the bake writes each road/rail way's **smoothed
  longitudinal profile** (the §4.1/§4.2 `smoothProfile` output, stations
  every 10 m, heights relative to the shared anchor) additively into
  `data/winterthur/roads.json`. The runtime builds ONE corridor-aware
  ground sampler: inside a road/rail corridor it returns the interpolated
  profile height, outside it falls through to the tile field, with a blend
  band at the corridor edge (this is data routing — the single sampler IS
  the honest source of truth for "ground under infrastructure"; it is not
  a fallback). Ribbons, markings, cars, rail look, and platforms all
  sample this one sampler, so every consumer agrees.
- **§9 metric v2:** the burial criterion becomes terrain-poke-through:
  within every corridor, `p99(tileY − profileY) ≤ 0.05 m` and
  `max(tileY − profileY) < 0.10 m` (the ribbon's own lift covers the
  rest) — i.e. graded terrain never pierces the road surface; plus the
  ribbon itself is level across its width by construction (it drapes on
  the profile).
- **Terrain-discard (measured escalation, 2026-07-06):** grading (2.5 m
  grid) + corridor-snap tile encoding brought p99 0.99 → 0.596 m but
  cannot pass the budgets: adjacent parallel ways at conflicting profile
  heights (footway 4.4 m above a service lane in ONE 12.5 m tile cell —
  real-world retaining-wall situations) are unrepresentable in any
  per-vertex heightfield. Definitive mechanism: the bake exports a
  corridor mask; the terrain shader discards fragments inside road/rail
  corridors, and ribbons gain side skirts (vertical aprons at the ribbon
  edges, dropping to **profile − 1.5 m**) that close the hole. Rendered
  terrain inside a corridor then does not exist — piercing is impossible
  by construction. (The skirt drops to `profile − 1.5 m`, not the earlier
  draft's `min(profile, local tile height) − 1 m`: the `min()` is moot
  because the terrain UNDER the ribbon footprint is discarded, so there is
  no local tile height to clamp to; a flat 1.5 m drop below the profile
  reliably reaches below the graded shoulder the discard mask leaves
  rendered — verified by the §9 v3 skirt-reach check, `max(profile −
  tile) ≤ 1.5 m`.) The mask stamps the **ribbon** footprint (renderWidth/2
  for roads, ballast-bed/2 for rails), NOT the wider grading half-width, so
  the graded shoulder keeps rendered terrain and no see-through annulus
  opens beside a road.
- **§9 metric v3 (same criterion, rendered truth):** the criterion "no
  rendered terrain above the road surface" is unchanged; measurement
  moves off the heightfield proxy: (a) the discard mask must cover 100 %
  of corridor stations; (b) tileY − profileY budgets (v2 values) apply
  only OUTSIDE the mask (blend band), where terrain still renders. Both
  reported by the metric CLI.
- **Road platform + terrain-grounded skirts (Platform wave, 2026-07-06 —
  supersedes the v3 shoulder-annulus budget):** measuring v3 non-vacuously
  exposed that the ribbon-footprint mask + 2.5 m cell FLOOR discards a band
  BEYOND the ribbon edge for the 54 % of ways narrower than a mask cell
  (renderHW < 2.5 m), which the ribbon-edge skirt did not cover from above →
  a see-through band beside narrow ribbons (max 3.05 m shoulder breach, skirt
  requiredDrop up to 8.49 m vs the constant 1.5 m). Resolution (controller
  decision, within the approved A+B architecture): **roads own a PLATFORM =
  ribbon + apron.** The rendered road surface extends from the ribbon edge to
  the DISCARD-MASK edge — an apron strip (Swiss "Bankett"/verge) at profile
  height, carriage colour ×0.9 — so no void shows from above. The mask edge is
  the render half-width floored at the mask cell size (`max(renderHW,
  MASK_CELL_M)`), computed identically bake-side (corridormask.mjs's
  `Math.max(halfWidthM, cellSize)`) and runtime-side (groundSampler.ts
  `roadMaskHalfWidth`/`railMaskHalfWidth`, MIRROR-pinned by
  gradewidths-parity.test.ts). **Skirts move to the mask edge and drop
  PER-VERTEX to `tileGround(x,z) − 0.5 m`** (the runtime tile-ground sampler
  `tileGroundYAt`), so they ALWAYS reach the terrain — on embankments a fill
  slope, on cuts the terrain rises against the skirt as a cut bank (correct
  reality; NO budget applies to terrain height beyond the mask). The constant
  1.5 m skirt drop is retired.
- **§9 metric v4 (default; v3/v2/v1 kept behind `--v3`/`--v2`/`--v1`):** the
  criterion "no rendered terrain above the road surface & no see-through gaps"
  becomes two geometric invariants: **(a) MASK COVERAGE** — stations sampled
  ACROSS the platform (centreline + offsets to ±(maskHW − εₚ), 0.5 m steps) must
  be 100 % inside the mask, where εₚ = `cellSize·√2/2` is the mask's own
  raster-quantization tolerance (a nearest-cell mask guarantees continuous
  coverage only to `maskHW − cellSize·√2/2`; the thin fringe out to maskHW is the
  discretised edge, where terrain is either below the apron or a legitimate cut
  bank). **(b) SKIRT REACH** — at the mask edge the runtime skirt foot is
  `tileGround − 0.5`; v4 recomputes it from the same tileGround and asserts it
  sits below the terrain (deficit `max(0, bottom − tileY)` = 0 by construction),
  so the skirt always overlaps the ground it hides. The v3 annulus poke-through
  budget is removed: cut banks beyond the mask are legitimate, not a defect.
  Mutation-tested for non-vacuity: a synthetic mask hole under the platform
  fails (a); a floating skirt fails (b). Real bake (no re-bake — the mask
  artifact is unchanged): (a) 100.0000 % (86416/86416 platform-interior samples
  in mask), (b) skirt-reach deficit 0.000 m — **PASS**.

## 6. Swiss markings & surfaces (render layer)

New `roadMarkings` builder next to `buildRoads`, flat-colour geometry in the
clay style, on the existing `roadYs`/polygonOffset ladder above the
carriage:

- **Leitlinien** (white dashed centre line): only on `primary/secondary/
  tertiary/unclassified` two-way carriageways; CH pattern 6 m dash / 6 m gap,
  0.15 m wide. Residential/service stay unmarked (Swiss practice).
- **Randlinien** (white continuous edge lines) on primary/secondary, 0.10 m,
  inset 0.3 m from the ribbon edge.
- **Fussgängerstreifen** (yellow, 0.5 m bars / 0.5 m gaps across the
  carriageway) at OSM crossing data (`highway=crossing` nodes /
  `crossing=*`). `geo:fetch` is extended to keep crossing nodes; if a
  re-fetch is needed it follows the existing Overpass pipeline.
- **Belag**: per-class surface tones (asphalt for carriageways, gravel tone
  for `track/path`, paver tone for `pedestrian`), and footways along
  carriageways get a kerb read: footway ribbon lifted +6 cm with a 45°
  kerb edge strip.

All marking geometry is generated deterministically from `osm-roads.json` +
crossing nodes; unit-tested pure (dash phasing stable under re-runs).

## 7. Gleis-Look (rail rendering)

Replaces the flat rail ribbon (keeps the graded bed):

- **Ballast**: trapezoid cross-section strip (top width as today, 1:1.5
  slopes), gravel tone.
- **Two steel rails**: 1435 mm gauge, thin raised strips (0.07 m wide,
  0.12 m proud of the ballast top), light metallic tone.
- **Sleepers**: instanced boxes (2.4 × 0.24 m, 0.6 m spacing) — near-LOD
  only with distance cutoff + the #129 impostor/compaction pattern if counts
  demand it.
- **Catenary masts**: instanced poles every ~50 m along all rail lines
  (CH ≈ fully electrified), lamppost-style thin geometry, alternating sides
  on double track.

## 8. Bahnhöfe (stations)

Real geodata, Winterthur's stations rendered with true platform geometry:

- **Perronkanten**: SBB open data "Perronkante" (opendata.swiss, LV95 →
  shared projection via the existing `lv95ToWgs84` + projector). Fetched by
  a new `geo:fetch-stations` script into `scratch/geo/` (gitignored, same
  pattern as demand data).
- **Platform bodies**: extruded from SBB edge polylines — the single source
  of truth, **no fallback source**. Stations inside the Gemeinde without SBB
  Perronkante coverage get NO platform geometry, and the bake fails loudly
  with the list of uncovered stations (honest absence over silently wrong
  OSM shapes; project rule: no fallbacks, honest errors). Top at **P55 =
  0.55 m above rail top** on the graded rail profile, access ramp ends where
  the data indicates.
- Platform surface tone + yellow safety line (0.10 m, inset 0.8 m from the
  edge) — the single most recognisable Swiss platform feature.
- Station buildings/canopies remain swissBUILDINGS3D as today; underpasses
  are part of the bridges follow-up.

## 9. Testing & verification

- **Golden bake**: double-run byte-identical world tiles.
- **Burial metric as test** (from today's diagnosis script): across all road
  cross-sections at 10 m steps against the baked L2 DEM, max rail deviation
  **< 0.3 m** and p99 < 0.15 m. RED today (1.49 m / 5.8 % ≥ 0.3 m).
- Vitest: marking generation (dash pattern, crossing bar placement,
  kerb heights), platform extrusion (P55 above graded rail), mast spacing.
- The #131 longitudinal burial regression test is folded in (net-new vs
  #132's own tests).
- **Browser captures** (mandatory smoke discipline): before/after at
  (a) the worst cross-slope site [-347, 1073], (b) a primary axis with a
  crossing, (c) the rail corridor embankment, (d) Winterthur HB platforms.
  Attached to the PR(s).

## 10. Delivery

Three PRs, each independently shippable:

1. **Grading** (bake pass, §4 + §5 + burial metric test) — fixes the visible
   sinking at the root.
2. **Markings & surfaces** (§6).
3. **Gleis-Look & Bahnhöfe** (§7 + §8, incl. `geo:fetch-stations`).

Artifacts are gitignored (world re-bake local); no DB, no wire changes, no
committed-asset churn beyond scripts/src/tests.

## 11. Data sources

- swisstopo swissALTI3D (DEM, in repo pipeline), swissBUILDINGS3D 3.0.
- OpenStreetMap (roads, crossings) via Overpass.
- SBB Open Data: Perronkante (opendata.swiss), station Dienststellen (DiDok)
  for naming if needed.
- VSS/ASTRA marking conventions (dimensions authored as constants; visual
  fidelity, not legal exactness).
