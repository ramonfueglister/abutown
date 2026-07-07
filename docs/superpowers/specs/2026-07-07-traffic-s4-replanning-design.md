# Traffic SOTA S4 — Day-to-day replanning (MATSim co-evolution)

**Status:** core + convergence proof IMPLEMENTED (branch
`claude/traffic-sota-s4-spec`, stacked on PR #147); shell integration
remaining. Depends on PR #147 (S1–S3) landing before merge.

**Done:**
- `winterthur_traffic::replanning` — pure Charypar-Nagel scoring, bounded
  `PlanMemory` (EWMA scores, worst-scored eviction, unscored-plan
  protection), deterministic logit selection, `decide_action` /
  `time_mutation_offset`, all pure `u01`-driven. 6 unit tests.
- `tests/replanning_convergence.rs` — BPR two-route fixture proving the
  day-to-day loop reaches Wardrop equilibrium (day-0 ~180% imbalance →
  ~5% tail-averaged, both routes used, faster route carries more) +
  determinism. Key finding: logit θ must match the trip-utility scale
  (MATSim's default θ=2 assumes full-day activity scores; trip-only
  scoring needs θ≈12).

**Shell manager DONE** (branch `s4-shell-integration`): `ReplanningState` —
`BTreeMap<(day_kind, trip_index), PlanMemory>` (recurring census-trip
identity), in-flight `veh → (trip, spawn_tick)` tracking, and
`planned_route` / `note_spawn` / `note_despawn` /
`between_day(world_day, edge_of_lane, reroute_closure)`. Route-choice-only
v1 (TimeMutate collapses to Select). 3 unit tests. Unwired library API.

**ECS wiring DONE** (branch `s4-shell-integration`): spawner consults
`planned_route` and runs the learned route (else fresh census routing);
`SpawnRecord`/`QueuedEntry` carry `day_kind`; shell resources `ReplanningRes`
+ `LastWorldDay`; `spawn_trips` calls `note_spawn`; `replan_reap` scores
arrivals over `in_flight` (all despawn paths); `replan_between_day` fires on
the world-midnight wrap before spawns, re-routing on realized router weights;
both schedule chains wired; θ=12. Integration test (`s4_replanning_shell.rs`,
2000 ticks real net over midnight): between-day fires, memories accumulate, no
collision / conservation break, seed-reproducible. All 43 existing
winterthur-traffic tests still green (incl. real-net collision test with
replanning active); clippy `-D warnings` clean incl. sim-server. **S4 is now
live in the sim-server.**

**Snapshot serialization DONE** (branch `s4-persistence`, stacked on the
snapshot write-cost PR): the learned plan memories now survive a server
restart. `Plan`/`PlanMemory` derive serde; `ReplanningState::to_snapshot_value`
/ `restore_from_snapshot_value` serialise ONLY the `(day_kind, trip_index) →
PlanMemory` map (sorted, byte-stable) — `in_flight` is excluded (the kernel
fleet is not persisted, so trips re-spawn on resume) and `params`/`seed` are
re-authored by `ReplanningState::new`. Cross-crate seam: `WorldCoreSnapshot`
gains an OPAQUE `replanning: Option<serde_json::Value>` field (world-core can't
see the traffic crate), bumped to snapshot v2 with a No-Wipe `1 | 2` migrate arm
(`#[serde(default)]` ⇒ old v1 rows load as `None`); the sim-server orchestrator
fills it after `extract` via `winterthur_traffic::shell::harvest_replanning`,
and the shell restores it in `build_sim`. Failure-isolated: a decode miss or
absent field degrades benignly to empty memories (agents re-learn — the exact
pre-persistence behaviour), never a boot failure. The earlier OOM concern is
moot: the snapshot never held the fleet, and the write-cost PR's zstd
compression makes the added ~2-3 k-trip memory payload negligible. Tests:
`snapshot_value_round_trips_plan_memories` (lossless + benign-decode-miss),
`migrate_lifts_v1_row_without_replanning_field` (No-Wipe), and the opt-in PG
`world_store` round-trip carries a representative blob through zstd + Postgres.

**Historical wiring checklist (now done):**
1. `ReplanningState` as a `#[derive(Resource)]` wrapper.
2. **Spawner** (`spawn_trip`): consult `planned_route(day_kind, index)` and
   spawn on the learned route when present, else route from the census OD
   (current behaviour), then `note_spawn(...)`. Pass the spawner an
   `Option<&mut ReplanningState>` so traffic-only builds stay `None`.
3. **Despawn scoring**: a system after `core_tick` calling `note_despawn`
   for each `core.despawned_last_tick()` slot — before the slot can be
   reused (mirror the `arrivals_system` slot-reuse ordering).
4. **Between-day**: a system firing when `world_clock.world_day()` increments
   (stored last-day resource), calling `between_day(day, |l|
   net.lanes[l].edge, |o,d| router.route(net, o, d))`. After the spawner.
5. **Determinism guard**: a ≥3-world-day soak integration test asserting
   `state_hash` thread-count invariance with replanning active, that plan
   memories evolve, and conservation holds.
6. **Snapshot** (LAST, gated on delta-snapshots): serialize `ReplanningState`
   alongside citizens/economy. Deferred — per-trip plan memory would grow the
   full-payload snapshot (the production OOM root cause). At
   `WORLD_BG_DEMAND_SCALE = 0.2` the fleet is ~2-3 k so it is tolerable, but
   delta-snapshots are the clean path. Until then, plan memory resets on
   resume (agents re-learn — documented, benign degradation).

## Goal

Close the last mechanism gap between the Winterthur microsim and the SOTA
of activity-based transport simulation: **day-to-day replanning toward a
stochastic user equilibrium (SUE)**. Today every census trip runs a route
that is optimal only against *free-flow* edge weights (with an intra-day
MSA congestion reroute as a reactive patch). Real travellers *learn* across
days — they remember that a corridor was slow yesterday and shift route (and
eventually departure time) tomorrow. MATSim's co-evolutionary algorithm
(Nagel & Kötter 2016; Horni, Nagel & Axhausen 2016) is the reference method
and is a natural fit for this project's frozen-time persistence model: a
"day" is exactly one world day (`WORLD_SECONDS_PER_DAY`), and the between-day
replanning step runs at the world-midnight boundary the world clock already
emits.

## Why it fits here specifically

- The world already advances in discrete **world days** (`WorldClock::
  world_day`), 4 real hours each. The replanning loop is "once per world
  day," so it rides the existing clock with no new scheduler.
- The persistence model freezes time when the server is down and resumes at
  the tick (memory: `persistence-frozen-time-model`). Per-agent plan memory
  (the choice set + scores) is snapshot state, restored on resume — the same
  seam the citizen/economy snapshots already use.
- The S2 calibration showed the network gridlocks under compressed peak
  demand. Day-to-day route spreading is exactly the mechanism that relieves a
  single over-loaded corridor by migrating demand to parallel routes — it
  should *raise* effective network capacity at the fluid operating point and
  narrow the S2 station-flow gap without touching the demand bake.

## Model (MATSim co-evolutionary loop, single mode = car)

Each agent (a census trip identity) holds a **plan memory**: a bounded choice
set of `(route, departure_offset)` plans, each with an executed **score**.
One world day = one iteration:

1. **Mobsim (execution).** The day is simulated exactly as today — the kernel
   moves vehicles, the shell measures realized per-edge travel times
   (`EdgeMeasure`, already MSA-smoothed).
2. **Scoring.** Each executed plan is scored with a Charypar-Nagel utility:
   `S = β_trav · t_travel + β_late · max(0, t_arrival − t_pref)` (car-only,
   no activity chain in v1 — the trip's realized travel time plus a
   late-arrival penalty against its census-preferred arrival). Scores are
   EWMA-blended across days so a single noisy day doesn't dominate.
3. **Replanning (between days, at world midnight).** A configurable fraction
   of agents (MATSim default ~10 %) replan; the rest re-execute their
   best-scored plan:
   - **ReRoute** (share ρ_r): recompute the route on the *previous day's*
     realized edge weights (the CH router already accepts live weights via
     `Router::update_weights`), add it as a new plan if novel.
   - **TimeMutation** (share ρ_t): jitter the departure offset by ±Δ.
   - The non-replanning majority selects among existing plans by a
     multinomial-logit (softmax over scores, temperature θ) — the standard
     MATSim `SelectExpBeta`.
   - Plan memory is bounded (keep best K, e.g. 5); the worst is dropped.
4. **Convergence.** Track the day-over-day change in mean score and in the
   route-share vector at the S2 count stations; the loop has reached a
   stochastic user equilibrium when both fall below a tolerance (or a max
   iteration count). Report the equilibrium station flows through the S2
   calibration harness — this is the honest test of whether learning
   narrows the demand-normalized gap.

## Determinism

Every stochastic choice (which agents replan, ReRoute vs TimeMutation, the
logit draw) is a pure function of `u01(seed, world_day, agent_id ^ salt)` —
the same discipline as `demand-gen` and the kernel noise. The between-day
step runs sequentially in ascending agent id at the world-midnight tick, so
the plan-memory evolution is thread-independent and snapshot-reproducible.

## Data model / seams

- New per-agent resource `PlanMemory { plans: SmallVec<[Plan; K]>, ... }`,
  keyed by the trip identity the spawner already assigns. Snapshot-serialized
  alongside citizens/economy.
- The between-day system slots into the shell schedule at the world-midnight
  wrap the spawner already detects (`spawner.rs` splits the release window at
  the wrap — the same boundary triggers replanning).
- No wire/protocol change: replanning is server-internal; the browser keeps
  receiving per-tick vehicle frames.

## Non-goals (v1)

- Activity chains / mode choice (car-only; the census trip is the unit).
- Capacity-based scoring beyond realized travel time (no toll/comfort terms).
- Online (within-day) replanning beyond the existing congestion reroute.

## Validation plan

1. Unit: scoring monotonicity, logit selection distribution, plan-memory
   bounding, determinism across thread counts (mirror the ring/junction
   determinism tests).
2. Integration: a two-route fixture where one route is capacity-limited —
   assert demand splits across days toward the analytic Wardrop share.
3. System: run N world days through `--bin calibrate` (extended to iterate
   days) and report the day-over-day station-flow trajectory + whether the
   demand-normalized S2 ratio improves at equilibrium vs the single-day
   baseline.

## References (APA 7)

- Horni, A., Nagel, K., & Axhausen, K. W. (Eds.). (2016). *The multi-agent
  transport simulation MATSim*. Ubiquity Press.
  https://doi.org/10.5334/baw
- Nagel, K., & Kötter, T. (2016). Choosing (and scoring) plans. In *The
  multi-agent transport simulation MATSim* (Ch. 4). Ubiquity Press.
- Charypar, D., & Nagel, K. (2005). Generating complete all-day activity
  plans with genetic algorithms. *Transportation, 32*(4), 369–397.
  https://doi.org/10.1007/s11116-004-8287-y
