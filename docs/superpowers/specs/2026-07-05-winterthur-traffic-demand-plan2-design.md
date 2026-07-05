# Winterthur Traffic — Plan 2: Real Demand, Whole Municipality, Far LOD

Date: 2026-07-05
Status: approved in brainstorming (user), staged delivery
Predecessor: `2026-07-03-winterthur-traffic-sim-design.md` (v1, shipped as PR #121)

## 1. Goal

Replace v1's synthetic-structured spawner with **census-based demand on the
whole municipality**, and make distant traffic **visible when zoomed out**.
Two independently shippable stages:

- **Stage 1 — demand + network:** expand the lane-level network from the
  hero plate (~1.2 × 1.5 km) to the full Gemeinde including the A1 motorway
  and boundary gateways; generate trips offline from STATPOP residents,
  the BFS commuter matrix, and OSM land use; spawn them against the **real
  local wall clock** (same real-time philosophy as the environment work,
  PR #116/#120). A single authored demand scaler keeps the fleet inside the
  CPU budget.
- **Stage 2 — far LOD:** an aggregate per-edge flow channel on the wire so
  streets outside the AOI radius show statistically correct traffic when
  the camera zooms out.

User-approved scope decisions (2026-07-05 brainstorming): real-time
coupling (no separate sim day); internal + in/out commuter + A1 through
traffic; CPU with demand scaler (GPU port stays a separate slice);
aggregated far LOD in scope.

## 2. Non-goals

- GPU port of the kernels (follow-up slice 1 of the v1 spec — unchanged).
- Day-to-day replanning / equilibrium learning, calibration against count
  stations beyond the authored A1 volumes (follow-up slices 2–3).
- Pedestrian–vehicle interaction, public transit on the network, parking.
- Economy coupling (the trip table remains the defined attachment point;
  nothing new may know about the economy).
- Per-vehicle persistence. Traffic remains ephemeral across restarts; only
  the baked assets are durable. (The frozen-time persistence model applies
  to the economy sim, not to this fleet.)
- Time-lapse/`?speed=` controls (user chose strict real time).

## 3. Stage 1 — network expansion

### 3.1 Scope of the bake

`scripts/geo/fetch-winterthur.mjs` already fetches OSM for the full
Gemeinde bbox (since #119/#121). The traffic-net bake
(`scripts/geo/bake-traffic-net.mjs` + `lib/trafficnet.mjs`) keeps its
drivable-class filter (motorway…service + `_link`) but consumes the full
Gemeinde extract instead of the plate clip. Same projector/anchor
(`lib/project.mjs` `ANCHOR`) as the world bake — **one shared projection
for terrain, buildings, and traffic net**; a divergence here is this
repo's most expensive bug class (CLAUDE.md, Phase 7a).

New in the network model:

- **Motorway semantics:** `motorway`/`motorway_link` edges already carry
  100 km/h free-flow speed in the class table; verify ramps (merge lanes)
  produce valid turn connections and that signals are never attached to
  motorway nodes.
- **Gateway edges:** every edge crossing the municipality boundary
  (swissBOUNDARIES3D polygon, already used by #119) is cut at the boundary
  and its outer stub marked `gateway: true` with a stable `gatewayId`
  (bearing-sorted). Gateways are spawn/despawn portals for external demand.
- Expected size: low-thousands of nodes, ~5–10 k directed edges. The JSON
  asset stays committed if it remains manageable (< ~15 MB pretty-printed);
  otherwise switch the asset to the gitignored-bake pattern of #119 with a
  committed hash manifest. Decide in the plan by measuring, not guessing.

### 3.2 Validation

`traffic-net` validation gains: gateway stubs must be sources/sinks only
(no turn connections beyond the boundary), motorway lanes ≥ 1, the strongly
connected component must cover ≥ 95 % of drivable lane length (dead
fragments are dropped at bake time, logged).

## 4. Stage 1 — demand generation (`demand-gen`)

Offline tool (new backend crate + npm script wrapper, same authored-asset
pattern as the net bake): inputs are public datasets, output is a
byte-stable `trips.bin`.

### 4.1 Inputs

- **STATPOP** (BFS): hectare-grid resident counts for the Gemeinde →
  home locations, snapped to the nearest residential/living-street lane.
- **BFS commuter matrix** (Pendlermobilität, commune↔commune): flows
  Winterthur↔every other commune, both directions.
- **OSM land use + buildings** (already fetched): work/retail/education
  zone weights for destination choice inside the Gemeinde.
- **A1 through volumes:** authored constants per gateway pair informed by
  published ASTRA automatic-count-station AADT for the Winterthur A1
  segments. Authored, not fetched — refinement belongs to the calibration
  follow-up slice.

### 4.2 Model

Three trip segments, all car-only (mode-share scaler = one authored
constant, as in the v1 spec):

1. **Internal:** STATPOP origins × land-use destination weights via a
   **doubly-constrained gravity model** (entropy-maximizing form; Wilson,
   1971) balanced with Furness iterations, distance-decay calibrated to the
   observed mean Swiss commute (Ortúzar & Willumsen, 2011, ch. 5).
2. **In/out commuters:** the BFS matrix row/column for Winterthur. Each
   external commune is mapped to the gateway that a free-flow shortest
   route from that commune's centroid would enter through (computed once at
   bake time on OSM main roads outside the Gemeinde; ties broken by
   bearing). Inbound trips: gateway → workplace weight; outbound: home →
   gateway. Evening trips mirror morning trips per person-slot.
3. **Through:** gateway → gateway flows on the A1 (and, where the ASTRA
   numbers imply it, the main cantonal axes), routed like any other trip.

**Departure-time profiles:** piecewise-linear daily curves (workday and
weekend variants) with morning/evening commuter peaks; through traffic gets
a flatter profile with a daytime plateau. Profiles are authored curves in
the tool (sampled per trip via the deterministic `u01` hash), not learned.

### 4.3 Output — `trips.bin`

Fixed-record little-endian binary, header:
`magic, version, tool_hash, net_hash, record_count, weekday_count,
weekend_count`. Record: `(departure_s_of_day: u32, origin_lane: u32,
dest_lane: u32, segment: u8, vehicle_class: u8)`, sorted by
`(day_kind, departure_s_of_day, origin_lane, dest_lane)` for byte
stability. `net_hash` binds trips to the exact net bake — the loader
refuses a mismatched pair (no silent fallback, per project convention).
Size estimate: ~400 k records × 14 B ≈ 6 MB — committed.

## 5. Stage 1 — runtime: real-time spawner

Replaces the v1 synthetic spawner (`winterthur-traffic/src/spawner.rs`).

- **Clock binding:** at boot the server records
  `boot_s_of_day = now(Europe/Zurich) mod 86400` and `day_kind` (workday /
  weekend, incl. a small authored Swiss holiday list = weekend). Sim
  time-of-day at tick *t* is `boot_s_of_day + t·dt` (dt = 0.1 s), wrapping
  daily and re-evaluating `day_kind` at each wrap.
- **Spawning:** per tick, release the trips whose `departure_s_of_day`
  falls in the tick's window, thinned by the **demand scaler**
  `demand_scale ∈ (0, 1]` (authored constant, initial value chosen so peak
  concurrency ≤ 30 k; thinning by deterministic `u01(seed, trip_index)` so
  the same subset spawns every day). Origin gateway spawns enter at the
  gateway stub with class-appropriate speed; internal spawns enter at the
  origin lane as in v1. `MAX_CONCURRENT` stays as a hard safety valve.
- **Warm start:** on boot mid-day the spawner back-fills by releasing, over
  the first ~60 s of uptime, a deterministic sample of the trips that would
  currently be en route (departure within the last authored mean-trip-time
  window), so the world never boots empty at 17:00.
- **Determinism:** unchanged invariant, restated for the wall-clock world:
  same `(seed, trips.bin, boot_s_of_day, day_kind)` → identical state hash
  after N ticks, independent of thread count. The boot anchor is logged at
  startup (the #97 boot-log-verification lesson).
- **Dev override:** server flag/env `ABUTOWN_TRAFFIC_AT=HH:MM` fixes
  `boot_s_of_day` for tests and demos — the backend counterpart of the
  frontend's `?at=` (#120). The two are independent by design; smoke
  scripts set both.
- Routing, re-routing, measured live weights: unchanged from v1.

## 6. Stage 2 — far LOD (aggregate flow channel)

Goal: when the camera zooms out past the AOI radius, distant streets show
traffic whose density and speed come from the real simulation.

- **Server:** `measure.rs` already aggregates per-edge speeds in 5-minute
  windows for routing. A lighter sampler publishes, every 2 s, a
  `FlowFrame`: for each edge with ≥ 1 vehicle, `(edge_id: u32,
  count: u8 saturating, mean_v_q: u8)`. Encoded once as `Arc<[u8]>`,
  fanned out on the existing `/traffic` socket to sessions that have sent
  `subscribe_flow` (a new client message next to cell subscribe). Empty
  edges are omitted; a keyframe marker resets client state each frame
  (frames are self-contained — no delta chain to corrupt).
- **Budget:** ≤ ~10 k active edges × 6 B ≈ 60 KB / 2 s ≈ 30 KB/s per
  zoomed-out client — acceptable without AOI logic; revisit only if
  measured otherwise.
- **Client:** a `FlowLayer` scatters instanced impostors (the existing car
  instancing path with a low-poly/point LOD) along each edge's polyline —
  `count` instances at deterministic offsets hashed per (edge, slot),
  advected along the polyline at `mean_v` so distant flow visibly moves.
  Impostors render **only outside the currently subscribed AOI cells**
  (inside, real vehicles own the road; the boundary crossfades over one
  cell ring to hide the swap). Positions are statistical, not per-vehicle
  truth — acceptable at distances where individual cars are sub-pixel.
- **Proto:** new messages in `traffic.proto` via the buf toolchain;
  existing cell messages untouched (additive, no wire break).

## 7. Performance

- Same kernel, larger net: tick cost scales with fleet size, not edge
  count, except CSR/bucketing overhead — the plan includes one
  `profile_tick_phases`-style measurement at 30 k fleet on the Gemeinde
  net before the PR (target: ≤ 50 % of the 100 ms tick budget on the
  M-series dev machine, as in v1 §9).
- CH rebuild on the bigger graph: measure; if the periodic overlay rebuild
  exceeds its async slot, lengthen the rebuild interval (authored const).
- `demand_scale` is the pressure valve; it ships < 1.0 if the measurement
  says so, with the measured headroom documented in the PR.

## 8. Testing & validation

- **demand-gen unit:** Furness balancing converges (row/column sums match
  inputs within tolerance); trip conservation per segment; gateway mapping
  is total (every external commune maps to exactly one gateway);
  `trips.bin` golden-hash test (byte-stable re-bake).
- **traffic-net:** gateway/motorway validation rules above; connected-
  component coverage.
- **Runtime:** wall-clock binding unit tests via `ABUTOWN_TRAFFIC_AT`
  (rush-hour tick spawns ≫ 03:00 tick spawns); warm-start populates a
  mid-day boot; determinism hash test extended with the boot anchor;
  vehicle conservation now includes gateway sinks
  (spawned = arrived + despawned-at-gateway + en-route).
- **Stage 2:** flow-frame encode/decode round-trip; impostor placement
  determinism; AOI-boundary exclusivity (no edge renders both real cars
  and impostors in the same cell).
- **Browser smoke (mandatory, CLAUDE.md):** extend
  `scripts/smoke-traffic.mjs` — assert (a) subscribe + cell frames as
  today on the Gemeinde net, (b) with `ABUTOWN_TRAFFIC_AT=07:30` the
  client vehicle table grows past an authored threshold within 60 s and
  at `03:00` stays below a low threshold, (c) after `subscribe_flow`,
  flow frames arrive and the impostor instance count is > 0 while zoomed
  out. Note: the smoke needs the gitignored world-bake artifacts locally
  (documented v1 gap) — the plan's first task verifies/regenerates them.
- Full local gate (Rust fmt/clippy/test via `scripts/cargo-serial.sh`,
  frontend typecheck/vitest/build) before each PR, per project convention.

## 9. Delivery

Two PRs: **Stage 1** (net + demand + spawner; ships alone, far streets
simply stay empty beyond AOI) then **Stage 2** (flow channel + impostors).
Assets: `trafficnet` (bigger) and `trips.bin` are additive; no DB
migration, no snapshot deletes (traffic is ephemeral).

## 10. References (APA 7)

- Geisberger, R., Sanders, P., Schultes, D., & Delling, D. (2008).
  Contraction hierarchies: Faster and simpler hierarchical routing in
  road networks. In *Experimental Algorithms* (pp. 319–333). Springer.
  https://doi.org/10.1007/978-3-540-68552-4_24
- Horni, A., Nagel, K., & Axhausen, K. W. (Eds.). (2016). *The multi-agent
  transport simulation MATSim*. Ubiquity Press. https://doi.org/10.5334/baw
- Kesting, A., Treiber, M., & Helbing, D. (2007). General lane-changing
  model MOBIL for car-following models. *Transportation Research Record,
  1999*(1), 86–94. https://doi.org/10.3141/1999-10
- Ortúzar, J. de D., & Willumsen, L. G. (2011). *Modelling transport*
  (4th ed.). Wiley.
- Treiber, M., Hennecke, A., & Helbing, D. (2000). Congested traffic
  states in empirical observations and microscopic simulations. *Physical
  Review E, 62*(2), 1805–1824. https://doi.org/10.1103/PhysRevE.62.1805
- Wilson, A. G. (1971). A family of spatial interaction models, and
  associated developments. *Environment and Planning A, 3*(1), 1–32.
  https://doi.org/10.1068/a030001
- Bundesamt für Statistik. (2024). *Statistik der Bevölkerung und der
  Haushalte (STATPOP), Hektarraster* [Data set]. BFS.
- Bundesamt für Statistik. (2024). *Pendlermobilität: Pendlermatrix der
  Gemeinden* [Data set]. BFS.
- Bundesamt für Strassen ASTRA. (2025). *Automatische Strassenverkehrs-
  zählung (SASVZ), Jahresergebnisse* [Data set]. ASTRA.
