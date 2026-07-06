# Building zone + GWR enrichment, persisted in Supabase, shown on hover

Date: 2026-07-06
Branch: `feat/building-zone-gwr` (worktree off `origin/main`)
Status: design — approved in brainstorming, pending spec review

## Goal

Give every Winterthur building two authoritative attributes and surface them on
hover:

- **`bauzone`** — the legal ÖREB *Grundnutzung* (Bauzone): what the parcel is
  **allowed** to be (e.g. `Wohnzone W3`, `Zentrumszone`, `Industriezone`).
- **`gwrCategory`** — the GWR building category (GKAT/GKLAS): what the building
  **is** (e.g. `Wohngebäude`, `Gebäude mit Wohn- und Nebennutzung`).

Persist both in Supabase as the authoritative record, and show the combination
`ist / erlaubt` in an ultra-minimal card when the user hovers a building in the
diorama.

## Non-goals

- No real Bauzone *editing* UI yet (that is M4 Neubau/Abriss territory; the
  Supabase table is the future edit surface, but this feature only reads).
- No live client→Supabase fetch in this PR (see "Client read path" — the client
  reads the baked static artifact now; the Supabase-backed API path is designed
  for but deferred to the Vercel+Fly cutover).
- No change to any WebSocket / subscription / coordinate-projection code.

## Context / current state (verified against `origin/main` @ 9d97aa2)

- The diorama is a static three.js client. Buildings are a **build-time static
  import**: `import buildingsJson from 'data/winterthur/buildings.json'` in
  `src/diorama/ksw/geo/geoData.ts`. No runtime fetch, no server, no Supabase in
  the building path today.
- Each baked building (`BakedBuilding` in `geoData.ts`) carries
  `{ id, name?, usage?, zone: 'ksw'|'city', footprint, height, eaveH, wall, roof, door? }`.
  `id` is the **swissBUILDINGS3D 3.0 UUID** (`{A83DFCB5-…}`). `zone` here is a
  *render-style* tag (`ksw` vs `city`), **not** the legal Bauzone.
- **No EGID anywhere in the pipeline.** swissBUILDINGS3D UUID ≠ EGID and does not
  map 1:1 to a GWR entity. → GWR must be joined **spatially**, not by key.
- Bake pipeline lives in `scripts/geo/`:
  - `geo:fetch` (`fetch-winterthur.mjs`) — network-only; pulls swissBUILDINGS3D
    tiles + OSM (Overpass) + DEM for the Gemeinde bbox `8.63,47.44,8.81,47.57`,
    reprojects via `ogr2ogr … -t_srs EPSG:4326`. Writes to `scratch/geo/`.
  - `geo:bake` (`bake-winterthur.mjs`) — **offline**, deterministic. Projects
    WGS84 → local plate metres via `scripts/geo/lib/project.mjs`
    (`makeProjector(ANCHOR).toLocal(lon,lat) → [x,z]`), builds wall/roof meshes,
    writes `data/winterthur/{buildings,meta,roads,nature}.json`.
- Backend `sim-server` (Rust, axum + sqlx) already owns Supabase: migrations in
  `backend/crates/sim-server/migrations/`, auto-run on startup; connects via
  `DATABASE_URL` (Supabase **session pooler `:5432`** — never `:6543`) with
  `PGSSLROOTCERT`. Deploys to Fly (`fly.toml`, app `abutown-abutopia`, region
  `lhr`, `internal_port 8080`).
- Env (`.env` present locally, `.env.example` tracked): `DATABASE_URL`,
  `PGSSLROOTCERT`, `SUPABASE_URL`, `CORS_ALLOWED_ORIGINS`,
  `VITE_SUPABASE_URL`, `VITE_SUPABASE_PUBLISHABLE_KEY`,
  `VITE_ABUTOWN_BACKEND_URL`. **No `service_role` key** — and none needed:
  enrichment writes go through the backend's existing `DATABASE_URL`, never a
  client-exposed key.

## Data flow

```
[network, one-time]   geo:fetch-attributes  → scratch/geo/{gwr,bauzonen}.geojson   (reprojected to EPSG:4326)
[bake, deterministic] geo:bake (extended)   → spatial-join per building → buildings.json gains {bauzone, bauzoneCode, gwrCategory, egid}
                                             → data/winterthur/building-attributes.json  (golden artifact, id-keyed)
[persist]             backend ingest         → upsert building-attributes.json into Supabase `building_attributes` (via DATABASE_URL)
[serve, future]       GET /building-attributes?world_id=… → id-keyed enrichment (reads Supabase)
[client, now]         geoData static import  → BakedBuilding already carries enrichment
[interaction]         pointermove raycast    → building id → accessor lookup → ultra-minimal card
```

Enrichment is computed **inside the bake, in WGS84, before the plate
projection collapses coordinates** — reusing `makeProjector`/the raw OSM+
swissBUILDINGS3D rings — so joins line up with reality and we never invert the
plate transform.

## Component 1 — offline data acquisition (`geo:fetch-attributes`)

New script `scripts/geo/fetch-attributes.mjs` (network-only, sibling to
`fetch-winterthur.mjs`, same `scratch/geo/` output, same `ogr2ogr -t_srs
EPSG:4326` reprojection convention):

- **Bauzonen**: ÖREB *Nutzungsplanung — Grundnutzung* polygons for the Gemeinde
  bbox, pulled from the Kanton ZH geodata service (WFS/GeoJSON), reprojected
  LV95→WGS84. Output `scratch/geo/bauzonen.geojson`. Fields kept: zone label +
  machine code.
- **GWR**: building/entrance records (EGID + GKAT + GKLAS + coordinates) for the
  bbox, from the federal/ZH GWR export. GWR coordinates are LV95
  (GKODE/GKODN) → reproject to WGS84. Output `scratch/geo/gwr.geojson` as points.

Both are checked for non-emptiness and bbox coverage; the script fails loudly
(no silent empty output) per the "no fallback cruft" convention.

Determinism: `scratch/geo/` is gitignored (like the rest of the raw geo pull);
the **deterministic golden output** is `data/winterthur/building-attributes.json`
produced by the bake, which *is* committed.

## Component 2 — the two joins (deterministic, in the bake)

Extend `bake-winterthur.mjs` (or a `scripts/geo/lib/enrich.mjs` it calls) with
pure, unit-tested geometry:

- **Bauzone (allowed)**: point-in-polygon of the building footprint **centroid**
  (WGS84) within the Bauzonen polygons → `{ bauzone, bauzoneCode }`. Building
  spanning >1 zone → centroid's zone wins. No match → `bauzone: null`.
- **GWR (is)**: point-in-polygon of each GWR entrance point within the building
  footprint (WGS84). A footprint may contain 0/1/many EGIDs:
  - 1+ → `egid` = primary (dominant GKAT; ties → lowest EGID for determinism);
    `gwrCategory` = its GKAT label; all matched EGIDs kept in `raw`.
  - 0 → `egid: null`, `gwrCategory: null` (e.g. sheds, sub-divided 3D volumes).

Pure functions live in `scripts/geo/lib/enrich.mjs`:
`pointInPolygon(pt, ring)`, `centroid(ring)`, `joinBauzone(building, zones)`,
`joinGwr(building, gwrPoints)`. Tested in `scripts/geo/lib/enrich.test.ts` with
fixtures + known-building assertions (e.g. Bahnhof centroid → a Zentrums-/
öffentliche-Bauten zone; a residential block → `Wohngebäude`).

Bake writes the four new fields into each `buildings.json` building **and** emits
`data/winterthur/building-attributes.json`:

```jsonc
{ "worldId": "winterthur",
  "buildings": [
    { "id": "{A83DFCB5-…}", "egid": 150404, "gwrCategory": "Wohngebäude",
      "gwrClass": "1122", "bauzone": "Wohnzone W3", "bauzoneCode": "W3",
      "raw": { "egids": [150404, 150405] } }
  ] }
```

## Component 3 — Supabase persistence (authoritative record)

New migration `backend/crates/sim-server/migrations/202607060001_building_attributes.sql`
(next free sequence after `202607050001_world_core_snapshots.sql`):

```sql
CREATE TABLE IF NOT EXISTS building_attributes (
  world_id      TEXT        NOT NULL,
  building_id   TEXT        NOT NULL,           -- swissBUILDINGS3D UUID
  egid          BIGINT,
  gwr_category  TEXT,
  gwr_class     TEXT,
  bauzone       TEXT,
  bauzone_code  TEXT,
  raw           JSONB,
  updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (world_id, building_id)
);
```

Ingest: a `sim-server` path reads `data/winterthur/building-attributes.json` and
upserts (`INSERT … ON CONFLICT (world_id, building_id) DO UPDATE`) via the
existing sqlx pool + `DATABASE_URL`/`PGSSLROOTCERT`. Runs as an explicit
one-shot (CLI subcommand or admin task invoked by an `npm run geo:load-attributes`
alias) rather than silently on every boot — keeps the boot path clean and the
load auditable. Supabase is the **source of truth**; `building-attributes.json`
is the reproducible artifact that seeds it.

## Component 4 — serve path (built now, wired later)

Backend endpoint `GET /building-attributes?world_id=winterthur` → id-keyed
enrichment read from Supabase, CORS-guarded like the existing API. Implemented
now so the Vercel+Fly cutover is a client one-liner. **Not consumed by the
client in this PR.**

## Component 5 — client accessor + hover card

- **Accessor** `src/diorama/ksw/geo/buildingAttributes.ts`: a single function
  `getBuildingAttributes(id): BuildingAttributes | undefined`. *Now* it resolves
  from the in-memory `BakedBuilding` (the four new fields baked into
  `buildings.json`). *Later* (Vercel+Fly): swap its body to fetch/cache
  `/building-attributes` from `VITE_ABUTOWN_BACKEND_URL`. Identical return shape
  → one-file pivot, and that pivot (crossing the wire) is when the browser smoke
  becomes doubly mandatory.
- `BakedBuilding` in `geoData.ts` gains optional
  `egid?, gwrCategory?, bauzone?, bauzoneCode?`.
- **Raycast hover**: on throttled `pointermove` (raycast at most once per frame),
  raycast the city building meshes. Each building mesh is tagged
  `mesh.userData.buildingId = id` at construction (in the city massing/building
  builder). On hit → `getBuildingAttributes(id)` → position a fixed DOM card near
  the cursor. On miss/leave → hide. Reuses the `designTokens` palette; DOM-
  injected like the existing HUD pattern.
- **Card content** (ultra-minimal — the `ist / erlaubt` combination):

  ```
  «name»                       ← only if building.name present
  Wohngebäude                  ← gwrCategory (IS);   fallback "Nutzung unbekannt"
  Zone W3 · erlaubt            ← bauzone (ALLOWED);  fallback "keine Bauzone"
  ```

  Two content lines + optional title. No heavy chrome. `height`/`egid` are
  intentionally omitted to keep it minimal (available in `raw`/DB if wanted).

## Testing & verification

- **Unit** (`scripts/geo/lib/enrich.test.ts`): point-in-polygon, centroid,
  join edge cases (0/1/many EGID, multi-zone), determinism (stable tie-break),
  known-building assertions.
- **Backend**: an ingest/round-trip test — seed the JSON, upsert, read back,
  assert row count == building count and a spot-checked row. Gated behind the
  opt-in Postgres test flag (per existing convention).
- **Browser smoke** (MANDATORY — touches render + picking): adapt
  `scripts/smoke-7a.mjs`; headless chromium loads the diorama, hovers a known
  building, asserts the card DOM appears with non-empty category + zone, and
  that hovering empty space hides it. This is required even though no WS wire
  changes, because the feature adds a render-side picking path.
- **Full CI gate before push**: Rust (fmt/clippy/test via `scripts/cargo-serial.sh`)
  + frontend (typecheck/vitest/build) + the browser smoke.

## Delivery

Worktree `feat/building-zone-gwr` off `origin/main` → PR. Never touches local
`main`. Verify green against `origin/main` before merge.

## Open risks

1. **GWR data acquisition endpoint/licensing** — the exact ZH/BFS GWR export URL
   + fields must be pinned in the plan (federal GWR is large; fetch only the
   Gemeinde bbox subset). If a clean bbox-filtered GWR pull isn't available, the
   fallback is the GeoAdmin identify API per centroid (the "Hybrid" option from
   brainstorming) — a plan-time decision, not a redesign.
2. **swissBUILDINGS3D 3D volumes** — one real building can be several UUID
   volumes; the GWR spatial join may attach the same EGID to sibling volumes.
   Acceptable (they share the real building); `raw.egids` records the overlap.
3. **Projection fidelity** — joins must run in WGS84 against the same rings the
   bake consumes, before `toLocal`. Reuse, don't reinvent, `project.mjs`.
```
