# Winterthur Traffic Plan 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Census-based car demand on the whole Gemeinde network (incl. A1 + boundary gateways), spawned against the real wall clock, plus an aggregate far-LOD flow channel for zoomed-out views.

**Architecture:** Stage 1 re-bakes the lane net at Gemeinde scale with gateway stubs cut at the municipality boundary, adds an offline `demand-gen` Rust tool (STATPOP × BFS commuter matrix × gravity model → `trips.bin`), and replaces the synthetic spawner with a wall-clock trip spawner. Stage 2 adds a `FlowFrame` broadcast on the existing `/traffic` socket and an impostor `FlowLayer` in the frontend. Two PRs.

**Tech Stack:** Node bake scripts (`scripts/geo/`), Rust workspace (`traffic-net`, `traffic-core`, `winterthur-traffic`, new `demand-gen`), buf/prost/protobuf-es wire, three.js instancing frontend.

**Spec:** `docs/superpowers/specs/2026-07-05-winterthur-traffic-demand-plan2-design.md`

## Global Constraints

- ALL cargo via `scripts/cargo-serial.sh <cmd> --manifest-path backend/Cargo.toml -p <crate>` — never two cargo at once, never `--workspace --all-targets` during iteration (CLAUDE.md).
- Determinism: all randomness via `traffic_core::u01(seed, tick, id)`; no HashMap iteration order in sim paths; same `(seed, trips.bin, boot_s_of_day, day_kind)` → identical state hash.
- One shared projection: `scripts/geo/lib/project.mjs` `ANCHOR = {lon: 8.7285, lat: 47.5069}`, x=east, z=south (`[x, -north]`).
- Wire: additive proto changes only; existing field numbers untouched.
- Assets byte-stable: 2-decimal coord quantization, stable sort orders, fixed key order.
- `dt = 0.1 s` (traffic_core::DT), 10 Hz tick; publish every 2nd tick.
- Browser smoke mandatory before claiming a frontend-touching task complete (CLAUDE.md).
- Rust fmt + clippy `-D warnings` clean before every commit that touches Rust.

## Preflight (Task 0) — local data artifacts

The worktree lacks gitignored artifacts. Before Task 1:

- [ ] **Step 1:** Check the main checkout for existing artifacts:
  `ls /Users/ramonfuglister/Coding/abutown/scratch/geo/osm-roads.json /Users/ramonfuglister/Coding/abutown/data/winterthur/world/ 2>/dev/null`
- [ ] **Step 2:** If present, copy (do NOT symlink — bakes rewrite files):
  `mkdir -p scratch/geo && cp -R /Users/ramonfuglister/Coding/abutown/scratch/geo/. scratch/geo/ && mkdir -p data/winterthur/world && cp -R /Users/ramonfuglister/Coding/abutown/data/winterthur/world/. data/winterthur/world/`
- [ ] **Step 3:** Else run `npm run geo:fetch` (Overpass + swisstopo downloads, ~10–30 min) and `npm run geo:bake-world` (needs the DEM/GDB fetches; multi-GB, ~long — only if world/*.pb missing; smoke needs them via `window.__LOOK_READY`).
- [ ] **Step 4:** Verify: `node -e "console.log(require('fs').existsSync('scratch/geo/boundary-winterthur.geojson'))"` → `true`.

---

### Task 1: Gemeinde-scale traffic-net bake with gateway stubs

**Files:**
- Modify: `scripts/geo/lib/trafficnet.mjs`
- Modify: `scripts/geo/bake-traffic-net.mjs`
- Test: `tests/geo/trafficnet-gateway.test.ts` (vitest, alongside existing trafficnet tests — check `tests/` for the existing file and extend there if one exists)

**Interfaces:**
- Consumes: `buildTrafficNet({osmRoads, osmTrafficNodes, projector, anchor})`, `scratch/geo/boundary-winterthur.geojson` (1 polygon feature).
- Produces: `buildTrafficNet({..., boundary})` where `boundary` is the GeoJSON geometry (lon/lat ring(s)). New output fields: node gains `kind: 'gateway'` (degree-1 boundary-cut endpoints); net.meta gains `gatewayCount`. Node/edge/lane/turn schemas otherwise unchanged: nodes `{id,x,z,kind,signal}`, edges `{id,from,to,speedMs,laneCount,priorityRoad,lanes}`, lanes `{id,edge,index,lengthM,pts}`, turns `{id,fromLane,toLane,node,conflictsWith,yieldsTo}`.

**Behavior to implement in `trafficnet.mjs`:**
1. Accept optional `boundary` (GeoJSON Polygon/MultiPolygon in lon/lat). Before projecting, clip each drivable way's geometry at the boundary: keep the inside part; at each crossing insert the intersection point as the way's new terminal vertex.
2. The terminal node created by a boundary cut is classified `kind: 'gateway'` (before the dead_end rule; a gateway is degree-1 by construction). Point-in-polygon: ray casting on lon/lat (boundary is small; exactness at vertices irrelevant — roads cross it transversally).
3. Ways entirely outside → dropped. Ways crossing twice (in-out-in) → split into separate inside segments (each with its own gateway endpoints).
4. After graph build, drop all but the largest strongly-connected component measured by lane length, BUT gateway stubs attached to the main component stay. Log dropped length.
5. Keep everything else (signals, roundabouts, priority, turn generation) unchanged.

- [ ] **Step 1:** Write failing vitest: feed a synthetic 3-way network where one way crosses a square boundary; assert the outside part is cut, the cut node has `kind: 'gateway'`, `meta.gatewayCount === 1`, and coordinates are 2-decimal quantized.
- [ ] **Step 2:** `npx vitest run tests/geo/trafficnet-gateway.test.ts` → FAIL (boundary option unknown).
- [ ] **Step 3:** Implement clipping + gateway classification + component pruning in `trafficnet.mjs`; wire `bake-traffic-net.mjs` to read `scratch/geo/boundary-winterthur.geojson` and pass `boundary` (hard error if missing — no silent fallback).
- [ ] **Step 4:** Test green; then run the real bake: `npm run geo:bake-traffic`. Record printed counts. Expected order of magnitude: thousands of nodes, 5–15 k lanes, gatewayCount ≈ 15–40 (A1 ×2 directions, cantonal roads, minor roads).
- [ ] **Step 5:** Asset-size decision (spec §3.1): `ls -lh data/winterthur/trafficnet.json`. If ≤ 15 MB → stays committed. If bigger: gzip check; if still unmanageable, move to the gitignored-bake pattern (#119) with committed blake3 manifest — implement whichever branch reality picks, document in the commit message.
- [ ] **Step 6:** Determinism: re-run bake, `git diff --stat data/winterthur/trafficnet.json` → empty.
- [ ] **Step 7:** Commit `feat(geo): Gemeinde-scale traffic net with boundary gateway stubs`.

### Task 2: Rust `traffic-net` — gateway kind + Gemeinde validations

**Files:**
- Modify: `backend/crates/traffic-net/src/types.rs` (NodeKind), `backend/crates/traffic-net/src/validate.rs`, `backend/crates/traffic-net/src/lib.rs`
- Test: in-crate `#[cfg(test)]` alongside existing tests

**Interfaces:**
- Consumes: baked `data/winterthur/trafficnet.json` from Task 1.
- Produces: `NodeKind::Gateway` variant (serde `"gateway"`); `TrafficNet::gateways(&self) -> &[u32]` (node ids, sorted, precomputed in `from_doc`); `TrafficNet::gateway_lanes_in(&self) -> &[u32]` / `gateway_lanes_out(&self) -> &[u32]` (lanes whose edge ends at / starts at a gateway node, sorted by lane id). Validation additions.

**Validation additions (validate.rs):**
- Gateway nodes are degree ≤ 2 total (one in-edge and/or one out-edge) and have NO turns (extend rule 9's exemption: `dead_end` → `dead_end | gateway`).
- Signals never on `motorway`-speed nodes is NOT checkable here (class not in schema) — skip; the bake owns it.
- Largest-component coverage ≥ 95 % of lane length is enforced at bake time (Task 1); Rust only asserts every lane is reachable from ≥ 1 gateway OR the net has 0 gateways (test nets).

- [ ] **Step 1:** Failing test: minimal JSON doc with a `"gateway"` node → loads; a gateway node with a turn → `NetError::Validation` mentioning "gateway".
- [ ] **Step 2:** `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p traffic-net` → FAIL (unknown variant).
- [ ] **Step 3:** Implement variant + accessors + validations.
- [ ] **Step 4:** Tests green. Also add an `#[ignore]`d test (pattern already exists in the crate) that loads the real `data/winterthur/trafficnet.json` and asserts `gateways().len() >= 10`.
- [ ] **Step 5:** fmt + clippy + commit `feat(traffic-net): gateway node kind, accessors, validations`.

### Task 3: Demand data fetch script

**Files:**
- Create: `scripts/geo/fetch-demand-data.mjs`
- Modify: `package.json` (add `"geo:fetch-demand": "node scripts/geo/fetch-demand-data.mjs"`)

**Interfaces:**
- Produces in `scratch/demand/` (gitignored): `statpop.csv` (hectare grid, cols incl. E/N koordinates + total residents), `pendlermatrix.csv` (commune→commune worker flows), `communes.csv` (BFS-Nr, name, centroid E/N — extracted from the already-fetched `scratch/geo/swissboundaries3d.gpkg` via ogr2ogr, layer `tlm_hoheitsgebiet`).

**Notes for the implementer:**
- STATPOP hectare geodata: BFS DAM asset (CSV, LV95 hectare coords `E_KOORD`,`N_KOORD`, population `B*BTOT`). Resolve the current asset URL from https://www.bfs.admin.ch/bfs/de/home/statistiken/bevoelkerung/erhebungen/statpop.html ("Geodaten"); download via `https://dam-api.bfs.admin.ch/hub/api/dam/assets/<id>/master`. Pin the resolved id in the script with a comment naming the vintage.
- Pendlermatrix: BFS "Pendlermobilität: Gemeindematrix" (XLSX; convert to CSV in-script with a tiny XLSX reader or instruct manual export — if XLSX parsing is disproportionate, fetch the matrix as published CSV if available; else vendor a one-time converted CSV under `scratch/demand/` with the download+conversion steps documented in the script header, and the script only validates its presence). Do NOT commit raw BFS files.
- Filter both to Winterthur (BFS-Gemeindenummer 230): STATPOP rows inside the Gemeinde bbox; matrix rows where origin or destination = 230.
- LV95→WGS84: use the exact affine-free approximation from swisstopo's published formulas (implement `lv95ToWgs84(E, N)` in the script; unit-test against the known anchor: LV95 2’697’000/1’262’000 ≈ Winterthur).
- Script must be idempotent and re-runnable; hard error with actionable message if a download 404s.

- [ ] **Step 1:** Implement script (download → filter → write the three CSVs, print row counts).
- [ ] **Step 2:** Run it; verify `wc -l scratch/demand/*.csv` all > 0 and STATPOP Winterthur population sums to ~115–120 k.
- [ ] **Step 3:** Commit script + package.json (no data files) `feat(geo): demand data fetch (STATPOP, Pendlermatrix, commune centroids)`.

### Task 4: `demand-gen` crate — gravity model → `trips.bin`

**Files:**
- Create: `backend/crates/demand-gen/` (`Cargo.toml`, `src/main.rs`, `src/inputs.rs`, `src/gravity.rs`, `src/gateways.rs`, `src/profiles.rs`, `src/output.rs`)
- Create: `data/winterthur/demand-authored.json` (through-traffic volumes + tuning constants; committed)
- Modify: `backend/Cargo.toml` (workspace member)
- Test: in-crate unit tests + one golden test

**Interfaces:**
- Consumes: `scratch/demand/*.csv` (Task 3), `data/winterthur/trafficnet.json` (Task 1), `traffic_net::load`, `traffic_net::TrafficNet::{gateways, gateway_lanes_in, gateway_lanes_out, pos_at}`, `data/winterthur/demand-authored.json`.
- Produces: `data/winterthur/trips.bin` (committed) with header `{magic: u32 = 0x54524950 "TRIP", version: u16 = 1, net_hash: [u8;32] (blake3 of trafficnet.json bytes), weekday_count: u32, weekend_count: u32}` then records sorted by `(day_kind, departure_s, origin_lane, dest_lane)`: `departure_s: u32, origin_lane: u32, dest_lane: u32, segment: u8 (0=internal,1=inbound,2=outbound,3=through), vehicle_class: u8 (0=car)` — 14 B LE each, weekday block then weekend block.
- CLI: `demand-gen --net data/winterthur/trafficnet.json --demand-dir scratch/demand --authored data/winterthur/demand-authored.json --out data/winterthur/trips.bin`.

**Model (spec §4.2), concretely:**
- **Zones:** STATPOP hectares (origins, weight = residents) and destination zones = OSM-landuse-derived: reuse the building-cluster idea — destination weight per lane = count of `osm-landuse.json` work/retail/education polygons + `buildings.json` footprints snapped within 150 m. To keep this tractable and deterministic: grid the Gemeinde into 200 m cells; per cell compute `res_weight` (STATPOP) and `work_weight` (landuse area m² × class factor: commercial/industrial/retail 1.0, residential 0.15); snap each nonzero cell to the nearest drivable non-motorway lane midpoint (linear scan is fine offline).
- **Internal trips:** doubly-constrained gravity `T_ij = A_i O_i B_j D_j f(c_ij)`, `f(c) = exp(-c/λ)` with `c_ij` = euclidean km between cell centers, λ = 4 km; Furness-balance A/B until row+col error < 0.1 % or 100 iters. `O_i` = res_weight × workers_per_resident (authored, 0.5) × car_share (authored, 0.36 — BFS modal split Winterthur order of magnitude, refinable). `D_j` ∝ work_weight, scaled so ΣD = ΣO.
- **In/out:** matrix rows/cols for BFS 230. Each external commune → gateway via bearing from Gemeinde centroid to commune centroid; if commune distance > 8 km and a motorway gateway lies within ±60° of the bearing, pick the nearest such motorway gateway, else the bearing-nearest gateway of any class (spec amended to this rule). Inbound: origin = that gateway's in-lane, dest sampled ∝ work_weight. Outbound: origin sampled ∝ res_weight, dest = gateway out-lane. Evening return trips mirror each morning trip (swap O/D, departure from the evening profile).
- **Through:** `demand-authored.json` entries `{fromGateway, toGateway, vehPerDay, profile: "through"}` — author A1 both directions using ASTRA AADT order (~60–70 k veh/day on the A1 near Winterthur; split by direction, minus the share that exits = authored numbers with a source comment).
- **Departure profiles** (`profiles.rs`): piecewise-linear PDFs (hour → weight), workday: commuter peaks 07:00–08:00 and 17:00–18:00; weekend: flat noon hump; through: plateau 06:00–20:00. Sample each trip's `departure_s` by inverse-CDF with `u01(seed=0xD3, index, salt)` (splitmix-based, reuse `traffic_core::u01` by depending on traffic-core OR copy the 10-line finalizer into demand-gen with a comment — prefer the dependency).
- Trip counts get one global multiplier `trips_scale` from `demand-authored.json` (default 1.0 — this is the *bake* knob; the *runtime* `demand_scale` thinning is Task 6).

- [ ] **Step 1:** Failing unit tests: (a) Furness converges on a 3×3 toy matrix to given row/col sums; (b) gateway mapping picks the motorway gateway for a far commune aligned with it and a local-road gateway for a near commune; (c) record encode/decode round-trips; (d) records come out sorted.
- [ ] **Step 2:** `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p demand-gen` → FAIL, implement modules, → PASS.
- [ ] **Step 3:** Run the real bake; sanity-print: total weekday trips (expect ~150–350 k at scale 1.0), segment shares, top-5 gateways by volume (A1 must lead).
- [ ] **Step 4:** Golden test: bake twice → identical blake3 of `trips.bin` (byte-stable). Size check ≤ 10 MB; if bigger, halve via `trips_scale` is NOT the answer — instead store departure_s as u32 (already) and confirm counts are plausible; commit whatever is real.
- [ ] **Step 5:** fmt + clippy + commit `feat(demand-gen): census gravity demand → trips.bin` (include the asset).

### Task 5: `trips.bin` loader + wall-clock module in `winterthur-traffic`

**Files:**
- Create: `backend/crates/winterthur-traffic/src/demand.rs` (loader + `TripSchedule`), `backend/crates/winterthur-traffic/src/clock.rs`
- Modify: `backend/crates/winterthur-traffic/src/lib.rs` (mod decls), `backend/Cargo.toml` (add `chrono`, `chrono-tz` to workspace deps; blake3 if not present)

**Interfaces:**
- Produces:
  - `demand::TripSchedule::load(path: &Path, net_json_bytes: &[u8]) -> Result<TripSchedule, DemandError>` — verifies magic/version/net_hash (blake3 of the bytes actually loaded; mismatch = hard error, no fallback).
  - `TripSchedule::trips_in(&self, day: DayKind, window: Range<u32>) -> &[Trip]` — binary-search slice by departure_s; `Trip {departure_s: u32, origin_lane: u32, dest_lane: u32, segment: Segment, index: u32}` (index = position in file, used for deterministic thinning).
  - `clock::WallClock::new(now_utc: DateTime<Utc>, override_at: Option<NaiveTime>) -> WallClock`; `WallClock::s_of_day(&self, tick: u64) -> u32` (= `(boot_s + tick·DT) mod 86400`); `WallClock::day_kind(&self, tick: u64) -> DayKind` (Europe/Zurich weekday/weekend + authored holiday list `const HOLIDAYS: &[(u32, u32)]` month/day, fixed Swiss national ones); `DayKind {Workday, Weekend}`.
- Consumes: `ABUTOWN_TRAFFIC_AT` env (HH:MM) — parsed in main.rs (Task 6) and passed as `override_at`.

- [ ] **Step 1:** Failing tests: loader rejects wrong net_hash; `trips_in` returns exactly the records in a window (build a tiny trips.bin in-test via the Task 4 record writer — expose `demand_gen::output::write_trips` as a lib fn and dev-dependency, or duplicate the 20-line writer in test code); WallClock: `override_at=07:30` → s_of_day(0)=27000, tick wrap at midnight flips day_kind on a Sunday→Monday boundary.
- [ ] **Step 2:** Run scoped tests → FAIL → implement → PASS.
- [ ] **Step 3:** fmt + clippy + commit `feat(winterthur-traffic): trips.bin loader + Europe/Zurich wall clock`.

### Task 6: Real-time trip spawner (replaces synthetic spawner)

**Files:**
- Rewrite: `backend/crates/winterthur-traffic/src/spawner.rs`
- Modify: `backend/crates/winterthur-traffic/src/shell.rs` (resource + system wiring), `backend/crates/winterthur-traffic/src/main.rs` (env: `TRIPS_BIN` path default `data/winterthur/trips.bin`, `ABUTOWN_TRAFFIC_AT`, `DEMAND_SCALE` default from const)

**Interfaces:**
- Consumes: `TripSchedule`, `WallClock`, `Router::route(net, from_edge, to_edge) -> Option<Vec<u32>>`, `Core` spawn via the same path the old spawner used (`try_spawn_one` → `fleet.alloc(lane, s, v, len_m, route)`), `u01(seed, tick, id)`.
- Produces: `TripSpawner` resource + `spawn_trips` system (same schedule slot as before: `drain_commands → spawn_trips → core_tick → measure_edges → publish_snapshot`); gateway arrivals: vehicles whose route ends at a gateway in-lane despawn on arrival exactly like normal end-of-route (no code change expected in traffic-core — verify with a test).
- **Delete** the old synthetic attractor spawner entirely (no legacy path — project rule). `SpawnConfig` is replaced by `SpawnerCfg { demand_scale: f32 }` (default 1.0 until Task 8 measures; `MAX_CONCURRENT` safety valve stays as a const re-homed here).

**Behavior:**
- Per tick: `window = [s_of_day(t), s_of_day(t+1))` (handle midnight wrap as two windows + day_kind flip); for each trip in window: spawn iff `u01(seed ^ 0x5EED_DE44, trip.index as u64, 0) < demand_scale`. Route via `router.route(origin_edge, dest_edge)` (edge = lane→edge lookup); on `None` (disconnected) count + skip (log rate-limited).
- Spawn kinematics: gateway origins enter at `s=0` with `v = 0.8 × lane speed`; internal origins at `s=0, v=0` (as v1 did).
- **Warm start:** at boot, collect trips with `departure_s ∈ [boot_s − 900, boot_s)` (same thinning), and release them uniformly over the first 600 ticks (deterministic slot: `trip.index % 600`), flagged so their spawn tick uses the *original* thinning draw (no double randomness).
- Suppress spawns while `core.alive() >= MAX_CONCURRENT` (drop, count, log once per window-close — same valve semantics as v1).

- [ ] **Step 1:** Failing tests (use a tiny in-test net + in-test trips.bin): (a) with `ABUTOMN`… careful: `ABUTOWN_TRAFFIC_AT=07:30` equivalent via `WallClock::new(_, Some(07:30))`, a 60 s run spawns ≫ than at 03:00 (assert ratio > 5×); (b) `demand_scale=0.5` spawns the SAME subset across two identical runs (state-hash equality); (c) warm start populates alive() > 0 at tick 300 for a 12:00 boot; (d) determinism hash test from v1 extended with boot anchor (same anchor → same hash, different anchor → different).
- [ ] **Step 2:** Scoped tests FAIL → implement → PASS. Also run the full crate: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p winterthur-traffic` (old spawner tests get deleted/rewritten here, deliberately).
- [ ] **Step 3:** Boot log line (the #97 lesson): `info!(boot_s_of_day, day_kind = ?dk, trips_weekday = n1, trips_weekend = n2, demand_scale, "traffic demand bound to wall clock")`.
- [ ] **Step 4:** fmt + clippy + commit `feat(winterthur-traffic): wall-clock census trip spawner (replaces synthetic)`.

### Task 7: Vehicle conservation with gateway sinks + Gemeinde-net integration test

**Files:**
- Modify: `backend/crates/winterthur-traffic/src/shell.rs` (or a new `audit.rs`) — counter resource `Conservation { spawned: u64, arrived: u64, skipped_no_route: u64 }`
- Test: in-crate

- [ ] **Step 1:** Failing test: run 5 000 ticks on the tiny net; assert `spawned == arrived + core.alive() as u64` every 500 ticks (arrivals include gateway despawns).
- [ ] **Step 2:** Implement counters (increment in spawn path; arrivals from core's despawn list — check how v1 counts arrivals in `Spawner::step`'s `spawned` vec and mirror the mechanism), PASS.
- [ ] **Step 3:** `#[ignore]`d integration test on the REAL net + real trips.bin (skipped in CI, run locally): 30 sim-minutes at 07:30, assert alive() climbs and no validation/conservation failure.
- [ ] **Step 4:** fmt + clippy + commit `test(winterthur-traffic): vehicle conservation incl. gateway sinks`.

### Task 8: Stage-1 perf measurement + demand_scale calibration

**Files:**
- Modify: none expected; possibly `SpawnerCfg::demand_scale` default and `demand-authored.json`
- Create: `scratch/` notes only (not committed); results go in the PR body

- [ ] **Step 1:** Release-mode timed run (NOT criterion, `--no-run` rule irrelevant here — plain binary): `scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p winterthur-traffic --release`, then run with `ABUTOWN_TRAFFIC_AT=07:30` for 5 sim-minutes; log per-1000-tick wall time + peak alive().
- [ ] **Step 2:** Budget check (spec §7): tick ≤ 50 ms mean at peak (50 % of 100 ms). If over: reduce `demand_scale` default until inside budget; record measured headroom + chosen value in the PR body. If CH rebuild (measure window flush) blocks > 1 tick, lengthen `WINDOW_TICKS` via `with_window` wiring instead — but only if measured.
- [ ] **Step 3:** Commit any constant changes `perf(winterthur-traffic): calibrate demand_scale from measured tick budget`.

### Task 9: Stage-1 browser smoke + full gate + PR 1

**Files:**
- Modify: `scripts/smoke-traffic.mjs` (new assertions), possibly `scripts/capture-traffic.mjs` (re-point clusters)

- [ ] **Step 1:** Extend smoke: launch backend with `ABUTOWN_TRAFFIC_AT=07:30`; keep assertions (a)–(d); add (e) client vehicle table > 40 within 60 s at 07:30; then relaunch with `ABUTOWN_TRAFFIC_AT=03:00` and assert table < e-threshold/4. (Thresholds scale with demand_scale — read the value the backend logs and derive.)
- [ ] **Step 2:** Run smoke: `node scripts/smoke-traffic.mjs` → all green (needs Task 0 world artifacts).
- [ ] **Step 3:** Full local gate: Rust `fmt --check` + `clippy --workspace --all-targets -D warnings` + `test --workspace` (serial script, ONE run, allowed here as the gate), frontend `npx tsc -p tsconfig.typecheck.json`, `npx vitest run`, `npm run build` (wrapper), e2e if present.
- [ ] **Step 4:** Push branch, open PR 1 "Winterthur traffic Plan 2 / Stage 1 — census demand on the whole Gemeinde"; body: per-task commits, gate results, smoke output, perf numbers, demand_scale rationale. Wait for ALL checks green (gh --exit-status; never merge UNSTABLE), merge, delete branch.

---

## Stage 2 — far-LOD flow channel (PR 2, branch `traffic/plan2-flowlod` off fresh main)

### Task 10: Proto — flow messages

**Files:**
- Modify: `backend/crates/protocol/proto/traffic.proto`; regen via buf (existing build.rs / npm codegen path — find the exact command in protocol/README or build.rs and run it)

**Interfaces (verbatim proto to add — additive only):**
```protobuf
message TrafficClientMsg {
  repeated uint32 subscribe_cells = 1;
  repeated uint32 unsubscribe_cells = 2;
  optional bool subscribe_flow = 3;   // NEW: true=on, false=off
}
message FlowState {
  uint32 edge = 1;    // edge id
  uint32 count = 2;   // vehicles on edge, saturating at 255
  uint32 v_q = 3;     // mean speed, 0.25 m/s units (same quantization as VehicleState.v_q)
}
message FlowFrame {
  uint64 tick = 1;
  repeated FlowState edges = 2;  // only edges with count >= 1; self-contained (no deltas)
}
message TrafficServerMsg {
  repeated CellFrame cells = 1;
  optional FlowFrame flow = 2;   // NEW
}
```

- [ ] **Step 1:** Edit proto, regen Rust + TS, both build. Commit `feat(protocol): traffic flow-LOD messages`.

### Task 11: Server flow publisher

**Files:**
- Modify: `backend/crates/winterthur-traffic/src/gateway.rs` (session flow flag + fanout), `shell.rs` or `measure.rs`-adjacent new `flow.rs` (sampler)

**Interfaces:**
- Produces: every `FLOW_EVERY_N_TICKS: u64 = 20` (2 s): iterate fleet once, bucket per EDGE (lane→edge map from net), `count` + mean `v`; encode ONE `TrafficServerMsg{flow}` as `Arc<[u8]>`; fan out to sessions with `flow_subscribed`. Reader task handles `subscribe_flow`.
- Read-only SnapshotHook discipline: the sampler runs inside the existing publish hook (it already receives `&Core` + `&TrafficNet`), gated on tick % 20.

- [ ] **Step 1:** Failing test: encode/decode round-trip of a sampled frame from a 3-vehicle toy core (counts and v_q correct; empty edges omitted).
- [ ] **Step 2:** Implement; PASS; fmt + clippy.
- [ ] **Step 3:** Wire-level test (existing gateway test patterns): a session that sent `subscribe_flow=true` receives a flow frame within 25 published ticks; one that didn't, doesn't.
- [ ] **Step 4:** Commit `feat(winterthur-traffic): aggregate flow channel (2 s per-edge density/speed)`.

### Task 12: Frontend FlowLayer impostors

**Files:**
- Create: `src/diorama/traffic/flowLayer.ts`
- Modify: `src/diorama/traffic/trafficClient.ts` (decode flow, expose `flow: Map<number, {count, v}>`, send subscribe_flow when zoom exceeds threshold), `src/diorama/ksw/main.ts` (wire layer + update loop)
- Test: `tests/traffic/flowLayer.test.ts` (vitest, node-side logic only: placement/exclusion math extracted pure)

**Interfaces:**
- Produces: `createFlowLayer(net: TrafficNetGeom, groundYAt?: GroundYAt): FlowLayer` with `object3d` and `update(flow: Map<number, FlowEdge>, subscribedCells: Set<number>, nowS: number): void`; pure helper `placeImpostors(edgeGeom, count, nowS, edgeId): Array<{x, z, yaw, fade}>` — deterministic offsets `hash(edgeId, slot)`, advected `(offset + v·nowS) mod lengthM`; `fade` from distance-to-subscribed-region: 0 inside subscribed cells, ramp to 1 over one CELL_SIZE_M ring (spec crossfade).
- Impostor rendering: second InstancedMesh reusing the clay-car geometry at fixed dim color, capacity 8192, `frustumCulled = false`, per-instance opacity via instance color × material transparency (three.js instanceColor path — no custom shader).
- Exclusion: an edge contributes impostors only where its polyline points fall OUTSIDE the subscribed 3×3 set minus the fade ring (use client `CellGrid.cell_of` mirror — trafficClient already replicates the cell scheme).

- [ ] **Step 1:** Failing vitest for `placeImpostors`: deterministic across calls at same nowS; advection moves impostors; fade=0 inside a subscribed cell.
- [ ] **Step 2:** Implement pure helpers → PASS; then the three.js layer + client wiring (subscribe_flow sent when camera height > authored threshold OR always-on — simpler: always subscribe, render only when impostors would be visible; choose always-on and note the ~30 KB/s cost).
- [ ] **Step 3:** Typecheck + vitest green. Commit `feat(traffic-frontend): far-LOD impostor flow layer`.

### Task 13: Stage-2 smoke + gate + PR 2

- [ ] **Step 1:** Extend `scripts/smoke-traffic.mjs`: (f) after subscribe_flow, ≥1 binary frame decodes to a FlowFrame with edges.length > 0; (g) `window.__traffic.flowCount() > 0` while camera zoomed out (add the debug counter to the `?traffic` hook).
- [ ] **Step 2:** Run smoke → green; visual check via `scripts/capture-traffic.mjs` zoomed-out capture — impostor streams visible on the A1 (attach PNG to PR).
- [ ] **Step 3:** Full gate (same as Task 9 Step 3). PR 2 "Stage 2 — far-LOD flow channel"; wait green; merge; delete branch; tidy worktrees.

---

## Self-Review Notes (done at write time)

- Spec coverage: §3→T1/T2, §4→T3/T4, §5→T5/T6, §7→T8, §8→T7/T9 (+per-task tests), §6→T10–T12, §9→T9/T13. Gateway-mapping rule simplified vs. spec (bearing+motorway-preference) — spec amended in the same commit as this plan.
- Deviation log: client subscribes flow always-on (T12) instead of zoom-gated — noted in task, cheap on the wire.
- Types cross-checked against the interface report of 2026-07-05 (fleet.alloc, Router::route, SnapshotHook, CellGrid, proto field numbers 1–2 existing / 3 new).
