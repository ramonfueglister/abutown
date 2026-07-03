# Winterthur Traffic Sim v1 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cars drive microscopically (IDM + MOBIL + real intersections, right-hand traffic) on the real Winterthur lane graph, authoritative in a Rust `winterthur-traffic` server, visible in the browser diorama via WS/AOI streaming.

**Architecture:** Offline Node bake (reusing `scripts/geo/lib/project.mjs` for the exact world frame) produces `data/winterthur/trafficnet.json`. Rust crates: `traffic-net` (load/validate), `traffic-core` (pure SoA two-phase kernel, no bevy/no I/O), `winterthur-traffic` binary (headless `bevy_ecs` shell + axum WS gateway with AOI cells). Frontend: instanced car layer in the existing diorama app with dead-reckoning.

**Tech Stack:** Rust edition 2024 workspace (`backend/`), bevy_ecs 0.18 (already a workspace dep), rayon, fast_paths, axum ws, prost/buf, Node bake scripts, three.js/TSL frontend, playwright smoke.

**Spec:** `docs/superpowers/specs/2026-07-03-winterthur-traffic-sim-design.md`

## Global Constraints

- ALL cargo through `scripts/cargo-serial.sh …` — never two cargo at once; Rust subagents one at a time; scoped commands only (`-p <crate>`), never `--workspace --all-targets` during iteration.
- Right-hand traffic (Switzerland). Lane offsets go to the RIGHT of the travel direction. This must hold visually in the browser (verified by smoke).
- Coordinate frame: `scripts/geo/lib/project.mjs` toLocal — x = east meters, second component = z as used by the existing bake (`roads.json` pts are `[x, z]` in this frame). The traffic bake MUST import and use this module; never reimplement the projection.
- Determinism in sim crates: no HashMap iteration in the sim path (use Vec/BTreeMap or sorted); randomness only via splitmix64 finalizer over (seed, tick, id); fixed partition order; state-hash must be identical across thread counts.
- traffic-core: no allocation in the tick hot path (buffers pre-sized/reused), no bevy dep, no I/O, SoA only (`Vec<f32>`/`Vec<u32>` fields, never `Vec<Vehicle>`).
- dt = 0.1 s, 10 Hz tick. All physical units SI (m, s, m/s).
- Frontend typecheck: `npm run typecheck` covers src+tests+scripts; run vitest + build too before claiming done.
- Browser smoke is MANDATORY before the feature is called complete (CLAUDE.md).
- Criterion benches: build with `--no-run` in agent tasks, never execute in subagents.

---

### Task 1: Traffic-net bake — OSM → lane graph JSON

**Files:**
- Modify: `scripts/geo/fetch-winterthur.mjs` (add traffic-node query)
- Create: `scripts/geo/bake-traffic-net.mjs`, `scripts/geo/lib/trafficnet.mjs`
- Create: `data/winterthur/trafficnet.json` (baked output, committed)
- Test: `tests/geo/trafficnet.test.ts` (vitest)

**Interfaces:**
- Produces `data/winterthur/trafficnet.json`:

```jsonc
{
  "meta": { "anchor": {...}, "laneWidth": 3.0, "cellSize": 128 },
  "nodes": [ { "id": 0, "x": -338.2, "z": 734.1,
               "kind": "signal" | "roundabout" | "priority" | "uncontrolled" | "dead_end",
               "signal": { "cycleS": 60, "phases": [ { "greenS": 27, "turns": [3,4] } ] } | null } ],
  "edges": [ { "id": 0, "from": 0, "to": 1, "speedMs": 8.33, "laneCount": 1,
               "priorityRoad": true, "lanes": [0] } ],
  "lanes": [ { "id": 0, "edge": 0, "index": 0, "lengthM": 213.4,
               "pts": [[x,z], ...] } ],
  "turns": [ { "id": 0, "fromLane": 0, "toLane": 7, "node": 1,
               "conflictsWith": [2,5], "yieldsTo": [2] } ]
}
```

- Rules: split OSM ways at shared intersection nodes; `oneway=yes` → edges in one direction only, otherwise one edge per direction; laneCount from `lanes`/`lanes:forward|backward` tags (default 1/direction); lane polylines offset `(index + 0.5) * laneWidth` to the RIGHT of travel direction (perpendicular offset per segment, mitered at joints); node kind: `highway=traffic_signals` on the node or within 20 m on an approach → `signal`; `junction=roundabout` ways → `roundabout` nodes with circulating lanes, entries yield to circulating; else `priority` when exactly one through pair of edges carries a higher `class` rank (or `priority_road` tag), `uncontrolled` (right-before-left) otherwise. Signals get Webster-style defaults: cycle 60 s, green split proportional to approach laneCount, min green 7 s, all-red 2 s between phases; phases gate turn ids. `conflictsWith`: turns whose straight-line paths through the node intersect; `yieldsTo`: subset the turn must gap-accept against (minor→major, entry→circulating, left-turner→oncoming straight).
- Keep only drivable classes: motorway|trunk|primary|secondary|tertiary|unclassified|residential|living_street|service(+ their `_link`s). Drop footway/path/cycleway/pedestrian/steps/track.

**Steps:**

- [ ] **Step 1:** Extend `fetch-winterthur.mjs` with one more Overpass call (after osm-roads):

```js
await overpass(
  `[out:json][timeout:60];(
    node["highway"~"^(traffic_signals|stop|give_way|crossing)$"](${BBOX});
  );out;`,
  `${OUT}/osm-traffic-nodes.json`,
);
```

Run `node scripts/geo/fetch-winterthur.mjs` (network ok) so `scratch/geo/osm-roads.json` + `osm-traffic-nodes.json` exist.

- [ ] **Step 2:** Write failing vitest `tests/geo/trafficnet.test.ts` against the *baked file*: loads `data/winterthur/trafficnet.json`, asserts (a) >100 edges, (b) every `lane.edge`/`edge.from/to`/`turn.fromLane/toLane` id resolves, (c) every non-dead_end node with ≥1 incoming and ≥1 outgoing edge has ≥1 turn, (d) every lane `lengthM` ≈ polyline length ±1%, (e) right-hand check: for ≥95% of two-way edge pairs, lane 0 of edge A lies to the right of A's travel direction (cross-product sign test at the midpoint), (f) signal nodes have a phase table covering every incoming turn id exactly once per cycle, (g) at least one roundabout exists on the plate (Winterthur has them) and its entry turns list non-empty `yieldsTo`.
- [ ] **Step 3:** Implement `lib/trafficnet.mjs` (way splitting, edge/lane/turn synthesis, signal phase defaults, conflict geometry) + thin `bake-traffic-net.mjs` CLI that reads `scratch/geo/osm-roads.json` + `osm-traffic-nodes.json`, writes `data/winterthur/trafficnet.json` deterministically (stable sort by OSM id before assigning ids; JSON with fixed key order and 2-decimal coords).
- [ ] **Step 4:** `npm run test -- tests/geo/trafficnet.test.ts` → PASS; `npm run typecheck` clean.
- [ ] **Step 5:** Commit bake script + baked asset + test.

### Task 2: `traffic-net` crate — load + validate

**Files:**
- Create: `backend/crates/traffic-net/{Cargo.toml,src/lib.rs,src/types.rs,src/validate.rs}`
- Modify: `backend/Cargo.toml` (workspace member)
- Test: in-crate `#[cfg(test)]` + fixture `backend/crates/traffic-net/tests/fixtures/mini.json` (hand-written 4-node network: one signal, one priority, one two-lane edge)

**Interfaces:**
- Produces:

```rust
pub struct TrafficNet { pub nodes: Vec<Node>, pub edges: Vec<Edge>, pub lanes: Vec<Lane>, pub turns: Vec<Turn> }
pub fn load(json: &str) -> Result<TrafficNet, NetError>   // serde + validate()
impl TrafficNet {
    pub fn lane_len(&self, lane: u32) -> f32;
    pub fn turns_from(&self, lane: u32) -> &[u32];         // precomputed CSR
    pub fn pos_at(&self, lane: u32, s: f32) -> ([f32; 2], [f32; 2]); // (xz, unit tangent)
}
```

- Validation = the same invariants as the vitest (dangling ids, length mismatch, uncovered signal turns) → typed `NetError`, fail-fast, no heal-on-load (project rule: no defensive cruft).

**Steps:**

- [ ] **Step 1:** Failing tests: `load(mini)` ok; corrupt fixtures (dangling lane id, length off by 5%) → specific `NetError` variants; `pos_at` returns interpolated point + tangent matching hand-computed values.
- [ ] **Step 2:** `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p traffic-net` → FAIL, implement types/serde (field names match Task 1 JSON exactly — `speedMs`, `lengthM`, camelCase via `#[serde(rename_all = "camelCase")]`), validate, CSR for `turns_from`, arc-length LUT for `pos_at`.
- [ ] **Step 3:** Test passes; also add an ignored-by-default test `loads_baked_winterthur` (path via env `TRAFFICNET_JSON`) and run it once against the real bake.
- [ ] **Step 4:** `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml` + clippy `-p traffic-net`; commit.

### Task 3: `traffic-core` — SoA fleet, CSR lane index, IDM, two-phase tick

**Files:**
- Create: `backend/crates/traffic-core/{Cargo.toml,src/lib.rs,src/fleet.rs,src/idm.rs,src/tick.rs,src/rng.rs}`
- Test: in-crate unit tests + `tests/ring.rs`

**Interfaces:**
- Produces:

```rust
pub struct Fleet {                 // SoA; index = vehicle slot, free-list reuse
    pub lane: Vec<u32>, pub s: Vec<f32>, pub v: Vec<f32>,
    pub route: Vec<RouteHandle>, pub len_m: Vec<f32>, pub alive: Vec<bool>, ...
}
pub struct LaneIndex { /* CSR: lane -> sorted slots by s desc (leader first) */ }
pub struct IdmParams { pub v0: f32, pub t_headway: f32, pub a_max: f32, pub b_comf: f32, pub s0: f32 }
pub fn idm_accel(p: &IdmParams, v: f32, dv: f32, gap: f32) -> f32;
pub struct Core { ... }
impl Core {
    pub fn new(net: &TrafficNet, cap: usize, seed: u64) -> Core;
    pub fn spawn(&mut self, lane: u32, s: f32, route: &[u32]) -> Option<VehId>;
    pub fn tick(&mut self, t: u64);          // phase1 read -> intents, phase2 write
    pub fn state_hash(&self) -> u64;         // order-independent-of-threads hash
}
pub fn u01(seed: u64, tick: u64, id: u64) -> f32;  // splitmix64 finalizer
```

- IDM (Treiber et al. 2000): `s* = s0 + max(0, v·T + v·Δv/(2·√(a·b)))`, `acc = a·(1 − (v/v0)⁴ − (s*/gap)²)`; clamp gap ≥ 0.1; leaderless: gap = ∞ term drops. Integrate: `v' = max(0, v + acc·dt)`, `s' = s + v'·dt` (ballistic-safe).
- End-of-route or lane end without turn permission (Task 5) → treat as standing obstacle at lane end.

**Steps:**

- [ ] **Step 1:** Failing unit tests: (a) equilibrium: vehicle behind leader at equal speed and gap `s0 + v·T` gets |acc| < 0.01; (b) closing fast from behind → strong braking (acc < −2); (c) free road → accelerates toward v0, never past it.
- [ ] **Step 2:** Implement idm.rs + rng.rs (splitmix64 finalizer per #90 precedent), pass.
- [ ] **Step 3:** Failing `tests/ring.rs`: single circular lane (synthetic 1-lane ring net, 1000 m), 40 vehicles seeded uniformly with tiny deterministic speed noise via `u01`; run 3000 ticks; assert (a) zero collisions (min gap > 0 every tick), (b) stop-and-go emergence: stddev of speeds in final 500 ticks > 1.0 m/s (waves) while mean speed < v0·0.8, (c) determinism: same seed twice → identical `state_hash` at tick 3000, and identical with `RAYON_NUM_THREADS=1` vs `4`.
- [ ] **Step 4:** Implement fleet/laneindex/tick two-phase (phase 1 rayon over lane partitions writing intent buffers only; phase 2 sequential-deterministic apply + rebucket). Pass. No allocations in tick (assert via reused buffers; add debug_assert on capacity).
- [ ] **Step 5:** fmt + clippy `-p traffic-core`; commit.

### Task 4: MOBIL lane changes

**Files:**
- Create: `backend/crates/traffic-core/src/mobil.rs`
- Modify: `src/tick.rs` (lane-change intent in phase 1, apply in phase 2)
- Test: unit + `tests/overtake.rs`

**Interfaces:**
- `pub struct MobilParams { pub politeness: f32, pub a_thr: f32, pub b_safe: f32, pub bias_right: f32 }` (defaults 0.3, 0.2, 4.0, 0.2 — bias_right = European keep-right rule as an incentive bonus for right target lanes).
- Criterion (Kesting et al. 2007): change if `a_new_self − a_old_self + p·(Δa_followers) > a_thr + bias` and new follower decel `> −b_safe`. Randomized acceptance: change executes only if `u01(seed, tick, id) < 0.9` (MOSS practice, avoids synchronized flapping).

**Steps:**

- [ ] **Step 1:** Failing tests: (a) safety veto: gap too small behind → no change even if incentive high; (b) two-lane road, slow truck ahead → fast car changes left, passes, returns right (keep-right bias) within N ticks; (c) determinism hash test still passes with lane changes on.
- [ ] **Step 2:** Implement; only adjacent lanes of the same edge are targets; s is preserved across change (lanes of one edge share arc-length parameterization by construction of the bake).
- [ ] **Step 3:** Pass, fmt + clippy, commit.

### Task 5: Intersections — turns, signals, gap acceptance

**Files:**
- Create: `backend/crates/traffic-core/src/junction.rs`
- Modify: `src/tick.rs`
- Test: `tests/junction.rs`

**Interfaces:**
- `pub struct SignalState { /* per signal node: current phase, phase clock */ }` advanced inside `Core::tick` from the net's phase tables (cycle-position = `t·dt mod cycleS`, so it is stateless/deterministic).
- Vehicle at lane end consults its route's next turn: signal red → obstacle at stop line; green → check `conflictsWith` occupancy; `yieldsTo` → gap acceptance: accept iff every conflicting approaching vehicle is farther than `t_gap·v_conflict + margin` from the conflict point (t_gap 4 s roundabout entry, 6 s left across oncoming, 5 s right-before-left).
- Crossing the node: vehicle transitions `fromLane end → toLane start` in one phase-2 apply (node interior is not itself a lane in v1; conflict safety comes from the gap acceptance + signal gating).

**Steps:**

- [ ] **Step 1:** Failing tests on the `mini.json` fixture (+ a roundabout fixture `tests/fixtures/roundabout.json`): (a) red light: queue forms, nobody crosses; green: queue discharges, throughput within ±20% of `1800 veh/h·green share·lanes` (Webster-consistent); (b) roundabout: entering vehicle waits for circulating gap, no conflict-point co-occupancy ever (assert per tick); (c) right-before-left node: vehicle from the left yields; (d) determinism hash across thread counts.
- [ ] **Step 2:** Implement; conflict-point occupancy bookkeeping must be part of phase-2 sequential apply (fixed node order) to stay deterministic.
- [ ] **Step 3:** Pass, fmt + clippy `-p traffic-core`, commit.

### Task 6: Routing — CH base routes + stochastic re-routing

**Files:**
- Create: `backend/crates/traffic-core/src/routing.rs` (route repr) and `backend/crates/winterthur-traffic/src/router.rs` (CH service; fast_paths)
- Modify: `backend/Cargo.toml` workspace deps: `fast_paths = "1"`, `rayon = "1"`
- Test: unit in router.rs

**Interfaces:**
- `Router::new(net: &TrafficNet)` builds fast_paths CH over the edge graph (weights = lengthM/speedMs). `Router::route(from_edge, to_edge) -> Vec<u32 /*lane-level route: turn ids*/>` (expand edge path to turns; lane choice at each edge = any lane with a turn to the next edge, kernel handles getting into it via MOBIL "mandatory" incentive ramping near lane end).
- Live weights: `Router::update_weights(&[f32])` from per-edge smoothed travel times (MSA α=0.5, 5-min windows measured by the shell); rebuild CH at most every 5 sim-minutes.
- Re-route: shell samples vehicles whose `delay_ratio > 1.5` with p=0.1 per 30 s.

**Steps:**

- [ ] **Step 1:** Failing tests: shortest route on mini fixture matches hand-computed; after weight update penalizing an edge ×10, route avoids it.
- [ ] **Step 2:** Implement, pass, fmt + clippy `-p winterthur-traffic` (crate scaffolded here as lib+bin), commit.

### Task 7: `winterthur-traffic` binary — bevy_ecs shell, spawner, tick loop

**Files:**
- Create: `backend/crates/winterthur-traffic/src/{main.rs,shell.rs,spawner.rs,measure.rs}`
- Test: `tests/shell.rs` (headless: run 1000 ticks on the real baked net)

**Interfaces:**
- bevy_ecs `World` with resources: `Res<TrafficNet>`, `ResMut<Core>`, `ResMut<Router>`, `SimClock { tick: u64 }`. Systems in fixed order: `drain_commands → spawn_trips → signals(inside core) → core_tick → measure_edges → publish_snapshot`.
- Spawner v1 (synthetic-structured; STATPOP is Plan 2): attractor set = plate-boundary edges (gateways) + 30 heaviest building clusters from `data/winterthur/buildings.json` footprint centroids snapped to nearest lane; trip rate follows a two-peak daily curve (07–08 h and 17–18 h peaks, base 20%/peak 100% of `max_spawn_rate`); OD sampled ∝ attractor weights via `u01`; target fleet ~1500 concurrent at peak on the plate (tunable const).
- Tick loop: tokio, `interval(100ms)` with `MissedTickBehavior::Delay` + `yield_now` after each tick (per #91 lesson). Real-time factor 1.
- Determinism test: two runs, same seed, 1000 ticks → same hash; and the loop must keep an axum health endpoint responsive (probe during test).

**Steps:**

- [ ] **Step 1:** Failing `tests/shell.rs`: boots with `data/winterthur/trafficnet.json`, runs 1000 ticks headless-fast (no sleep in test mode), fleet population reaches >200, zero collisions, determinism hash equal across two runs.
- [ ] **Step 2:** Implement shell + spawner + measure (per-edge harmonic-mean speed, 5-min windows), pass.
- [ ] **Step 3:** fmt + clippy, commit.

### Task 8: Wire — proto, AOI cells, WS gateway

**Files:**
- Create: `backend/crates/protocol/proto/traffic.proto` (new file; do NOT touch reserved tags of existing protos)
- Create: `backend/crates/winterthur-traffic/src/{gateway.rs,cells.rs}`
- Modify: buf codegen config only if a new file needs wiring
- Test: `tests/gateway.rs` (tokio: connect ws client, subscribe cells, assert keyframe then deltas)

**Interfaces:**

```proto
message TrafficClientMsg { repeated uint32 subscribe_cells = 1; repeated uint32 unsubscribe_cells = 2; }
message VehicleState { uint32 id = 1; uint32 lane = 2; uint32 s_q = 3;  // s * 10 (dm)
                       uint32 v_q = 4; }                                // v * 4 (0.25 m/s)
message CellFrame { uint32 cell = 1; uint64 tick = 2; bool keyframe = 3;
                    repeated VehicleState vehicles = 4; repeated uint32 departed = 5; }
message TrafficServerMsg { repeated CellFrame cells = 1; }
```

- Cells: fixed 128 m grid over the plate bbox; `cells.rs` maps lane→cell list at boot (a lane can span cells; vehicle's cell from `pos_at(lane, s)` cached per lane segment). Publish at 5 Hz (every 2nd tick): per dirty cell one `CellFrame`, encoded once, shared as `Arc<[u8]>` to all subscribing sessions (zero-copy fan-out, #93 lesson). Keyframe on subscribe + every 5 s per cell.
- Gateway runs as separate tokio tasks; per-session bounded channel (drop-oldest on backpressure, never block the sim).
- WS endpoint `/traffic` on a new port env `TRAFFIC_PORT` (default 8790), plus `/healthz`.

**Steps:**

- [ ] **Step 1:** proto + failing gateway test (subscribe 1 cell → keyframe with ≥1 vehicle within 2 s; unsubscribed cell → no frames; deltas arrive at ~5 Hz).
- [ ] **Step 2:** Implement cells.rs + gateway.rs + publish system, pass.
- [ ] **Step 3:** fmt + clippy + `npm run proto:gen` (or the repo's buf gen script) so TS types exist; commit.

### Task 9: Frontend — instanced car layer with dead-reckoning

**Files:**
- Create: `src/diorama/traffic/{trafficClient.ts,carLayer.ts,deadReckon.ts}`
- Modify: the diorama city entry (`src/diorama/ksw/…` boot path that renders the city; wire behind `?traffic=1` URL param + `TRAFFIC_WS` default `ws://localhost:8790/traffic`)
- Test: `tests/traffic/deadReckon.test.ts` (vitest, pure math)

**Interfaces:**
- `trafficClient.ts`: loads `data/winterthur/trafficnet.json` (fetch; same asset the server uses — single source of truth for lane polylines), opens WS, maintains cell subscriptions from camera frustum center (3×3 cells), applies CellFrames into a `Map<vehId, {lane, s, v, tickAt}>`.
- `deadReckon.ts`: `poseAt(net, veh, nowTick): {x, z, yaw}` — advance `s + v·(now−tickAt)·dt` clamped to lane length, position+tangent via arc-length interpolation of the lane polyline (port of `pos_at`, must match server semantics).
- `carLayer.ts`: `InstancedMesh` (simple two-box car, ~4k instances cap), per-frame pose update, lane-accurate yaw; cars must visibly drive on the RIGHT side.

**Steps:**

- [ ] **Step 1:** Failing vitest for `deadReckon` (interp position/tangent on a hand-made 2-segment lane; extrapolation clamps at lane end).
- [ ] **Step 2:** Implement all three modules + boot wiring; `npm run typecheck` + vitest green; `npm run build` green.
- [ ] **Step 3:** Commit.

### Task 10: Browser smoke + visual verification loop

**Files:**
- Create: `scripts/smoke-traffic.mjs` (template: `scripts/smoke-7a.mjs`)
- Create: `scripts/capture-traffic.mjs` (screenshot harness; CDP screenshots — `page.screenshot` hangs on live canvas, per memory)

**Steps:**

- [ ] **Step 1:** `smoke-traffic.mjs`: launches dev stack (vite + `winterthur-traffic` binary), headless chromium on the city view with `?traffic=1`; asserts (a) client sends subscribe frames, (b) server sends CellFrames with vehicles, (c) sampled vehicle positions CHANGE over 5 s (cars actually move), (d) right-hand check: for ≥90% of sampled vehicles, the vehicle's offset from its edge centerline is to the right of its heading (computed from consecutive poses + trafficnet geometry).
- [ ] **Step 2:** `capture-traffic.mjs`: CDP screenshots at 3 landmarks (bahnhof, a signal intersection, a roundabout) at morning-peak sim time; inspect the images (Read tool) — cars on roads not sidewalks, right side, plausible queues at red lights, roundabout flow.
- [ ] **Step 3:** Iterate on kernel/bake/frontend until smoke passes AND screenshots look right. Fix root causes, no cosmetic hacks.
- [ ] **Step 4:** Commit smoke + captures wiring.

### Task 11: Full gate + PR

**Steps:**

- [ ] **Step 1:** Full local gate: Rust fmt-check/clippy/tests (serial, scoped per crate then one workspace pass), `npm run typecheck`, vitest, `npm run build` (via `scripts/build.mjs`), e2e/playwright if configured, smoke-traffic.
- [ ] **Step 2:** Push branch, open PR against main with summary + screenshots; verify CI green (`gh pr checks --watch --exit-status`), never merge on UNSTABLE (memory rule); wait for ALL checks pass.

## Self-Review Notes

- Spec coverage: §4 → Task 1; §3/§6 → Tasks 2–5; §7 → Task 6; §3 shell → Task 7; §8 → Tasks 8–9; §10 → Tasks 3/5/7/10. Deviation from spec, deliberate: demand v1 is synthetic-structured on the small plate (STATPOP/BFS = Plan 2 with bbox expansion) — user-approved priority is "cars driving correctly".
- Types consistent: `trafficnet.json` field names (camelCase) = serde rename_all camelCase in Task 2; `pos_at` semantics shared server (Task 2) / client (Task 9).
- No placeholder steps; formulas and parameter defaults are stated where an implementer needs them.
