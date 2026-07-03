# Winterthur Traffic Simulation — Design Spec

Date: 2026-07-03
Status: draft for review

## 1. Goal

An authoritative, deterministic, microscopic car-traffic simulation for the
real Winterthur street network, running in the Rust backend, rendered by the
existing Winterthur frontend (three.js/WebGPU instanced pipeline). Every
vehicle is simulated individually (car-following + lane changes + real
intersections); demand comes from real Swiss census/commuter data. The
architecture must serve an MMO-style deployment: up to ~2000 concurrent
spectating/playing clients against a single simulation authority.

State-of-the-art positioning (2026): the design follows the architecture of
current GPU-resident microscopic simulators (MOSS, LPSim — flat SoA buffers,
two-phase update, IDM + randomized MOBIL) executed on CPU first with a 1:1
GPU (wgpu) port path, combined with census-based demand generation
(the "Subway Builder" approach) and MMO-standard spatial interest management.
Deliberately NOT included: learned/neural driver models and LLM-generated
activity plans — they conflict with the authoritative/deterministic/persistable
requirement and target AV research, not city simulation.

## 2. Non-goals (v1)

- Pedestrian–vehicle interaction, public transit, parking search, accidents.
- Economy coupling (attachment point is defined: the trip generator; nothing
  in the dynamics core may know about the economy).
- The GPU port itself (only the data layout that makes it a 1:1 port).
- Day-to-day replanning / equilibrium learning, calibration against real
  count stations (both are known follow-up slices, out of scope here).
- Player gameplay design. Only the command-queue attachment point is defined.

## 3. Architecture overview

Backend workspace gains four crates and one binary:

```
traffic-net      lane-graph data model + baked binary asset format + loader
net-import      (offline tool) OSM Winterthur → lane graph asset
demand-gen      (offline tool) STATPOP + commuter matrix → trips asset
traffic-core     pure simulation kernel: SoA fleet, fixed timestep, no I/O,
                 no bevy, no allocation in the tick path
winterthur-traffic  (binary) headless bevy_ecs app: orchestration shell,
                 WS gateway, command queue
```

Process model (single-writer is mandatory, as everywhere in this project):

```
winterthur-traffic (ONE authoritative process, 10 Hz tick)
  ├─ bevy_ecs systems: clock, spawner, routing, re-routing, signals,
  │    kernel step (calls traffic-core), snapshot encode
  ├─ publishes per tick: per-cell delta buffers as Arc<[Bytes]> (zero-copy,
  │    same lesson as the #93 read-view Arc cache)
  └─ gateway tasks (tokio, in-process for v1): hold WS connections, filter
       per client by cell subscription, fan out with backpressure
```

The sim↔gateway interface is cut so gateways can later move to separate
stateless relay processes without touching the sim (scale-out path for more
players). The sim tick must never share an executor with connection handling
without explicit yielding — the #91 tick-loop starvation outage is the
cautionary precedent.

Bevy usage rule: `bevy_ecs` (headless) is the orchestration shell only.
The vehicle fleet is ONE ECS resource holding SoA buffers
(`Fleet { pos: Vec<f32>, vel: Vec<f32>, lane: Vec<u32>, … }`), processed by
one system per tick. Entities exist only for signals, intersections,
spawners, and optional per-vehicle inspection handles. Vehicles are never
per-entity components (archetype iteration would destroy lane-ordered cache
locality and GPU portability).

## 4. Network import (`net-import`)

Offline tool, OSM extract of Winterthur (optionally cross-checked against
swisstopo), producing a baked binary lane graph (authored-asset pattern, like
`markets.json`): byte-stable output for a given input + tool version.

Fidelity ("volles Programm"):
- Lanes per direction, turn lanes (`turn:lanes`), lane connectivity across
  intersections (turn connections).
- Traffic signals: where OSM has no timings, phase plans are estimated with
  Webster's method (cycle + green splits from approach volumes; initial
  volumes from the demand tool, refinable later).
- Roundabouts: yield-on-entry with gap acceptance.
- Priority-to-the-right in residential grids; explicit priority roads from
  OSM tags.
- Geometry: per-lane polylines with cumulative arc length (the frontend
  dead-reckons along these; coordinates in the Winterthur app's existing
  world frame — coordinate-system mismatches are this repo's most expensive
  bug class, see CLAUDE.md Phase 7a).

Asset contents: edges (directed, per-lane), lanes (polyline, length, speed
limit), nodes (intersection type, conflict groups, signal phase table), turn
connections (from-lane → to-lane with conflict-point list).

## 5. Demand (`demand-gen`)

Offline pipeline producing `trips.bin`:

1. STATPOP hectare-level residential population for Winterthur → home
   locations snapped to nearest residential lane.
2. BFS commuter matrix (municipality level) + OSM land use (work/retail
   zones) → workplace/destination weights.
3. Doubly-constrained gravity model distributes workers to workplaces
   (distance-decay calibrated to the observed mean commute).
4. Departure-time profiles (morning/evening peak curves) → trip table:
   `(departure_tick, origin_lane, dest_lane, vehicle_class)`.

Only car trips in v1; the mode-share scaler is a single authored constant.
The trip table is the future economy attachment point: an economy plugin may
append/replace trips (deliveries, work trips), nothing else.

## 6. Simulation core (`traffic-core`)

Fixed timestep dt = 0.1 s (10 Hz), all vehicles microscopic, no physics LOD.

Data layout: SoA fleet + CSR lane index (lane → contiguous, longitudinally
sorted vehicle range). Leader lookup = neighbor index. Rebucketing each tick.

Two-phase tick (this exact structure is the wgpu port):
1. **Read phase** (parallel over lane partitions, reads only tick t state):
   IDM acceleration; MOBIL lane-change desire (randomized acceptance as in
   MOSS); intersection gap acceptance / signal check → intent buffers.
2. **Write phase**: integrate, apply lane changes, edge transitions,
   rebucket. No locks; rayon over partitions; later the same kernels as
   wgpu compute passes.

Models (canonical, used unchanged by 2024–26 SOTA simulators):
- Car-following: IDM (Treiber, Hennecke, & Helbing, 2000).
- Lane change: MOBIL with politeness factor (Kesting, Treiber, & Helbing,
  2007), randomized threshold per MOSS practice.
- Signal timing estimation: Webster (1958).

Intersections: conflict groups per node. Signalized = phase table gating
turn connections. Roundabout/priority = gap acceptance against conflicting
approach vehicles; minor stream reserves a conflict point only if the
accepted gap holds.

Determinism: integer tick counter; all randomness via splitmix64-finalizer
hashing of (seed, tick, vehicle_id) — project precedent from the
stationary-age seed (#90); no HashMap iteration order in the sim path; no
floating-point reductions whose order depends on thread scheduling (per-lane
sequential inner loops, fixed partition order for cross-lane effects).
Invariant test: same seed + same trips → identical state hash after N ticks,
independent of thread count.

## 7. Routing

- Base routes: contraction hierarchies (`fast_paths`) over the directed edge
  graph, free-flow weights at boot.
- Live weights: per-edge measured travel times, 5-minute windows, MSA
  smoothing; CH overlay rebuilt periodically from smoothed weights.
- Re-routing: a vehicle whose expected remaining time degrades beyond a
  threshold re-queries with probability p per interval (stochastic, to avoid
  synchronized flapping). This yields quasi-dynamic traffic assignment —
  congestion avoidance like CS2, but with defined semantics.
- Routing runs as an async service outside the tick (request/response via
  queues); a vehicle drives its old route until the new one arrives.

## 8. Wire protocol & interest management

Protobuf via the existing buf toolchain. Broadcast-to-all is forbidden by
design (30k vehicles × 2000 clients ≈ 400 MB/s). Instead:

- The world is partitioned into AOI cells (fixed grid over the lane graph).
- Clients subscribe to the cells around their camera (the Phase 7a chunk
  subscription is the in-repo precedent — including its coordinate-frame
  lesson).
- Per tick the sim encodes one buffer per dirty cell: vehicle records
  `(vehicle_id, edge, lane, s quantized u16, v quantized u8)` ≈ 6–8 bytes.
- Cell entry → keyframe (full cell state); steady state → deltas at 3–5 Hz
  (sim runs 10 Hz; wire rate is decoupled).
- Client dead-reckons along lane polylines to full frame rate.
- Budget: a client sees 500–2000 vehicles → 10–20 KB/s per client,
  20–40 MB/s total at 2000 CCU — one machine's NIC, no cluster.

Player commands: a single command queue drained at a fixed point in the tick
(deterministic ordering: by receive order within tick, tie-broken by
client id). v1 commands: subscribe/unsubscribe cells, inspect vehicle.
Gameplay commands attach here later without touching the core.

## 9. Performance targets

- v1: 30k concurrent vehicles, 10 Hz, real-time on an M-series dev machine
  with < 50% of core budget (headroom for gateway fan-out).
- Deployment target: one dedicated ~16-core machine for sim + gateways at
  2000 CCU. More players → more gateway relays; more vehicles → the wgpu
  port (reference point: 2.46 M vehicles @ 84 Hz on one A100 in the 2024
  GPU-simulator literature).
- Criterion benches exist but are `--no-run` in agent tasks (memory lesson);
  a `profile_tick_phases`-style harness reports per-phase tick cost.

## 10. Testing & validation

- Unit: IDM properties (no collisions from valid states, equilibrium gap),
  MOBIL safety criterion, gap-acceptance edge cases, CSR rebucketing.
- Emergence: ring-road scenario MUST produce stop-and-go waves (classic IDM
  sanity check — if they don't emerge, the kernel is wrong).
- Invariants per tick (debug builds + audit sampling in release): no
  negative gaps (collision), vehicle conservation
  (spawned = arrived + en-route), determinism hash.
- Throughput: saturated signalized approach matches Webster capacity within
  tolerance.
- Browser smoke (mandatory per CLAUDE.md for anything crossing the wire):
  headless chromium against the dev stack, assert cell subscriptions go out
  and vehicle frames come back, in the smoke-7a template style.
- All cargo through `scripts/cargo-serial.sh`; scoped test invocations.

## 11. Follow-up slices (explicitly deferred)

1. GPU port of the two-phase kernels (wgpu compute).
2. Day-to-day replanning (MATSim-style co-evolution toward user
   equilibrium) — fits the frozen-time persistence model.
3. Calibration against public traffic-count stations (simulated vs. real
   daily profiles).
4. Economy coupling at the trip generator (deliveries, work trips from the
   economy simulation).
5. Player gameplay (commands beyond spectating).

## 12. References (APA 7)

- Kesting, A., Treiber, M., & Helbing, D. (2007). General lane-changing
  model MOBIL for car-following models. *Transportation Research Record,
  1999*(1), 86–94. https://doi.org/10.3141/1999-10
- Treiber, M., Hennecke, A., & Helbing, D. (2000). Congested traffic states
  in empirical observations and microscopic simulations. *Physical Review E,
  62*(2), 1805–1824. https://doi.org/10.1103/PhysRevE.62.1805
- Webster, F. V. (1958). *Traffic signal settings* (Road Research Technical
  Paper No. 39). Her Majesty's Stationery Office.
- Zhang, J., Ao, W., Yan, J., Rong, C., Jin, D., Wu, W., & Li, Y. (2024).
  MOSS: A large-scale open microscopic traffic simulation system.
  arXiv:2405.12520. https://arxiv.org/abs/2405.12520
- Zhang, J., Ao, W., Yan, J., Jin, D., & Li, Y. (2024). A GPU-accelerated
  large-scale simulator for transportation system optimization
  benchmarking. arXiv:2406.10661. https://arxiv.org/abs/2406.10661
- Jiang, X., Sengupta, R., Demmel, J., & Williams, S. (2024). LPSim: Large
  scale multi-GPU parallel computing based regional scale traffic
  simulation framework. arXiv:2406.08496. https://arxiv.org/abs/2406.08496
- Geisberger, R., Sanders, P., Schultes, D., & Delling, D. (2008).
  Contraction hierarchies: Faster and simpler hierarchical routing in road
  networks. In *Experimental Algorithms* (pp. 319–333). Springer.
  https://doi.org/10.1007/978-3-540-68552-4_24
- Horni, A., Nagel, K., & Axhausen, K. W. (Eds.). (2016). *The multi-agent
  transport simulation MATSim*. Ubiquity Press.
  https://doi.org/10.5334/baw
