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
  slice; noted where the grading pass must leave gaps (§4.4).
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

### 4.4 Determinism & artifacts

Pure function of (DEM, osm-roads, constants): double-bake must be
byte-identical (golden test, same as #119/#123 discipline). Artifacts stay
gitignored (world/*.pb); the bake logs corridor cell counts and the
burial metric (§8) before/after.

## 5. Runtime road mesh

Unchanged in principle: `miterStrip` + #132's `subdivideForDrape` now land
on graded, cross-level corridors, so the planar-across-width ribbon becomes
**correct** instead of approximate. The 0.3 m burial budget (§8) is the
acceptance criterion, not new mesh code.

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
- **Platform bodies**: extruded from SBB edge polylines (fallback: OSM
  `railway=platform` polygons where SBB data is missing), top at **P55 =
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
- OpenStreetMap (roads, crossings, platforms fallback) via Overpass.
- SBB Open Data: Perronkante (opendata.swiss), station Dienststellen (DiDok)
  for naming if needed.
- VSS/ASTRA marking conventions (dimensions authored as constants; visual
  fidelity, not legal exactness).
