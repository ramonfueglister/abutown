# Million-Agent World Roadmap

**Date:** 2026-05-16
**Status:** Master roadmap — phase-1 spec is committed alongside; phases 2–8 will get individual detail specs immediately before their implementation slice.

**Scale target:** One persistent always-on `abutown-main` world that durably hosts up to **1,000,000 simultaneously simulated agents** with subset-visible viewport delivery to connected browser clients. Multi-process worker sharding (SpatialOS / ECW-style horizontal scale, the path to 10M+) is explicitly *not* in this roadmap. After 1M is stable and observable, 10M can be re-scoped from a position of strength rather than guessed up-front.

This document is the canonical sequencing reference for everything mobility-related from foundation through production. Each phase is summarized here and gets its own detailed design spec (and its own plan) immediately before the corresponding implementation PR.

---

## Predecessor architecture (already in place)

Already merged on `main`, referenced by this roadmap rather than redesigned:

- `docs/superpowers/specs/2026-05-14-abutown-simulation-architecture-v2-design.md` — Chunks, scheduler (`Asleep/Warm/Active/Hot`), bevy_ecs `MaterializedRuntime`, dense tile arrays, append-only event log, periodic chunk snapshots, viewport-filtered replication intent, Supabase as durable source of record.
- `docs/superpowers/plans/2026-05-14-simulation-foundation-v2.md` — backend workspace, protocol DTOs, chunk storage, ECS materialization, scheduler.
- `docs/superpowers/plans/2026-05-14-agent-mobility-foundation.md` — `AgentMobilityState` (walk/wait/board/ride/alight) + `VehicleRecord` (transit) + `StopRecord` + `RouteRecord` + plans. SUMO + MATSim inspired per `docs/literature/agent-simulation/README.md`.
- `docs/superpowers/plans/2026-05-15-authoritative-command-event-boundary.md` — `POST /commands`, `WorldEventDto`, `InMemoryWorldEventStore`.
- `docs/superpowers/plans/2026-05-15-persistent-world-event-store.md` — Postgres `world_events` table + adapter.
- `docs/superpowers/plans/2026-05-15-supabase-backed-chunk-snapshots.md` — Postgres `chunk_snapshots` + adapter.
- `docs/superpowers/plans/2026-05-15-chunk-recovery.md` — `chunk_recovery` migration, `Chunk::from_snapshot`/`apply_event`, `hydrate_from_stores`, command idempotency via UNIQUE constraint.
- `docs/superpowers/plans/2026-05-16-mobility-population.md` — `mobility::seed::initial_world()` (20 agents + 4 vehicles), `MobilitySnapshotStore` (trait + InMemory + Postgres adapter), runtime hydration of mobility.
- `docs/superpowers/plans/2026-05-15-mobility-client-bridge.md` — frontend `mobilityProtocol.ts`, `mobilityState.ts`, `mobilityClient.ts` (currently diagnostic-only).
- `docs/superpowers/plans/2026-05-15-backend-required-mobility-runtime.md` — frontend refuses to boot without backend `/health`.
- `docs/superpowers/plans/2026-05-15-pedestrian-agents.md` and `2026-05-15-local-road-vehicles.md` — explicit "people are agents, cars are vehicles" separation on the frontend; backend mirror still incomplete (no road-vehicle subsystem yet).

## Curated literature (already in repo)

`docs/literature/agent-simulation/sources/` contains the primary-source references this roadmap respects:

- **Mobility model**: SUMO (`sumo-persons.html`, `sumo-public-transport.html`, `sumo-pedestrians.html`), MATSim (`matsim-book-part-one-latest.pdf`).
- **ECS / data-oriented runtime**: Bevy (`bevy-ecs-docs.html`), Flecs (`flecs-docs.html`, `flecs-systems.html`), Unity Entities (`unity-entities-archetypes.html`), Unreal Mass (`unreal-mass-entity.html`, `unreal-mass-gameplay-overview.html`).
- **Replication**: Unity Netcode for Entities (`unity-netcode-for-entities.html`, `unity-netcode-ghost-snapshots.html`).
- **Large-scale & LOD**: `dynamic-lod-large-scale-agent-urban-simulations-aamas2011.pdf` (AAMAS 2011, dynamic LOD), `ai-metropolis-mlsys-2025.pdf` (MLSys 2025), `scalesim-2601.21473.pdf`.

The roadmap follows these sources rather than inventing patterns.

---

## Final-state contract

The 1M-agent world is considered "delivered" when the production hardening phase passes:

1. The backend simulates **≥1,000,000 mobility entities** (any mix of pedestrians, transit vehicles, road vehicles) deterministically across the 64-chunk 256×256 world.
2. A single browser client connected to the running backend sees a **smooth animated subset** of those entities inside its viewport at 60 fps render rate, sourced exclusively from server-authoritative state.
3. The simulation sustains its tick budget continuously for **≥24 hours** without leaks, divergence, or unexplained tick deadline misses.
4. Restart (process kill → cold start) **rehydrates the full world** from Postgres in bounded time and resumes deterministically.
5. Operator-facing **metrics + structured logs** make tick budget, chunk activity distribution, persistence backlog, and client subscription health observable.

The contract does NOT include 10M, multi-process sharding, GPU rendering, or non-Postgres durable backends.

---

## Phases

Eight phases. Phase 0 is complete (the predecessor architecture). Phases 1–8 are this roadmap.

### Phase 1 — Visible Backend Mobility

**Detailed spec:** `docs/superpowers/specs/2026-05-16-visible-backend-mobility-design.md`

**Purpose:** Close the gap between server-authoritative mobility state and browser visuals. Today the backend simulates 20 agents + 4 transit vehicles, but the canvas still draws frontend-owned `buildPedestrians()` and `buildCars()` results. This phase makes the backend the only visual mobility source, adds a road-vehicle subsystem the backend was missing, and ships geometry-aware DTOs so the frontend can render without out-of-band knowledge.

**Scale after this phase:** ~200 pedestrians + ~80 road vehicles + 4 transit vehicles, server-authoritative, visible on canvas.

**Architecture:**
- New `sim-core` subsystem `RoadVehicleWorld` parallel to `MobilityWorld`. Roads vehicles carry `path: Vec<TileCoord>`, `offset: f32`, `speed`, `sprite_key`. No plans, no boarding. Mirrors the existing `local-road-vehicles` frontend split on the backend.
- New mobility geometry module `sim-core/src/mobility_geometry.rs` with deterministic hardcoded link/stop/path coordinates in tile-space.
- DTOs extended with `world_coord: {x: f32, y: f32}` and `direction: DirectionDto` (8-way enum) on `AgentMobilityDto`, `VehicleMobilityDto`, and new `RoadVehicleDto`. Server computes these per tick from `link_id`+`progress` / `path`+`offset`.
- New Postgres migration `road_vehicle_snapshots` (single row per world, UPSERT semantics — same shape as `mobility_snapshots`).
- Frontend `src/render/backendMobilityDrawables.ts` (new) projects `mobilityState` and the new `mobilityState.roadVehicles` into existing `drawPedestrian`/`drawCar` calls. `buildPedestrians()` and `buildCars()` get removed; sprite catalogs (`pedestrianSprites`, `vehicleSprites`) remain — they are now looked up by `sprite_key` string returned by the server.
- Selection / inspector switch to backend entity IDs.

**Dependencies:** none beyond Phase 0.

**Non-goals:** frame interpolation, viewport filtering, ECS migration, chunk-LOD, scaling beyond ~300 entities.

### Phase 2 — Frame Interpolation

**Spec to be written before implementation slice as:** `docs/superpowers/specs/2026-05-17-mobility-frame-interpolation-design.md` (filename illustrative).

**Purpose:** Backend ticks at ~10 Hz, browser renders at ~60 fps. Without client-side interpolation Phase-1 output looks juddery. This phase adds linear (later Hermite) interpolation between the two most recent server snapshots per entity, plus latency budget tracking.

**Architecture (anticipated, will detail in spec before implementation):**
- Frontend `mobilityState` retains the previous tick's positions in addition to current. Each render frame computes interpolated position via `lerp(prev, current, t)` where `t = (now - last_tick_at) / tick_period`.
- Optional Hermite spline if velocity hint is added to DTOs.
- Backend exposes `tick_period_ms` in `WorldSummaryDto` so client computes `t` correctly.
- Disconnect/reconnect zeroes the interpolation buffer; first snapshot after reconnect renders at exact server position.

**Dependencies:** Phase 1 (no point interpolating diagnostic-only data).

**Non-goals:** physics simulation, client-side prediction (Citation: server-authoritative per v2 spec).

### Phase 3 — Procedural Population & Shared Path Network

**Spec filename:** `docs/superpowers/specs/2026-05-18-procedural-population-design.md` (illustrative).

**Purpose:** The Phase-1 seeder is 200/80/4 hardcoded around chunks (4,4)/(5,4)/(4,5). To approach 1M agents we need the backend to know the full 256×256 city's road and pedestrian-corridor geometry, and the seeder must generate population per chunk activity.

**Architecture (anticipated):**
- Shared city descriptor: extract `src/city/zurichTransport.ts` (arterial paths, pedestrian corridors, stop network) into a generated JSON/TOML file consumed by both frontend renderer (for static visual elements) and backend (for mobility seed paths). Generation script lives in `scripts/`.
- Backend `mobility_geometry` module loads the shared descriptor at startup.
- Seeder is parameterized: `agents_per_chunk_density: f32`, `road_vehicles_per_chunk_density: f32`. Population scales with each chunk that wakes (`ChunkActivity::Active`/`Hot`).
- Target after this phase: ~10k pedestrians + ~3k road vehicles spread across all 64 chunks.

**Dependencies:** Phase 1 (need world_coord in DTOs before geometry-driven population matters visually). May be parallel with Phase 2.

**Non-goals:** procedural agent behavior (plans stay simple); pathfinding (paths are still pre-baked descriptor corridors).

### Phase 4 — Viewport-Filtered Replication (Interest Management)

**Spec filename:** `docs/superpowers/specs/2026-05-19-viewport-replication-design.md` (illustrative).

**Purpose:** Above ~1k visible entities, broadcasting the full mobility delta to every client saturates WebSocket bandwidth. This phase introduces Area-of-Interest filtering per the standard MMO pattern documented in `unity-netcode-ghost-snapshots.html` and `unreal-mass-entity.html`.

**Architecture (anticipated):**
- New client→server subscription message `MobilitySubscribeDto { viewport: BoundingBox, buffer_tiles: u16 }`. Sent on camera change with a small debounce.
- Server tracks per-connection viewport. `MobilityDelta` is filtered per connection to entities whose `world_coord` is inside `viewport ⊕ buffer_tiles`.
- Stable entity ID per pedestrian/road-vehicle so client can identify "entity entered viewport" vs "moved". Server tags deltas with `joined: bool` / `left: bool` to drive client-side fade-in/fade-out.
- Bandwidth budget: client-side hard cap, server-side per-connection coalescer.

**Dependencies:** Phase 1 (no viewport rendering without backend rendering); Phase 3 (no scale pressure without bigger population).

**Non-goals:** server-side spatial index (still O(n) scan over entities per tick in this phase; that's Phase 6).

### Phase 5 — ECS Hot-Path Migration

**Spec filename:** `docs/superpowers/specs/2026-05-20-ecs-mobility-migration-design.md` (illustrative).

**Purpose:** HashMap iteration becomes the tick bottleneck around 10k–100k entities (per `bevy-ecs-docs.html` and `unity-entities-archetypes.html`, the literature in this repo). Migrate `MobilityWorld` and `RoadVehicleWorld` storage to `bevy_ecs` archetypes for cache-friendly dense iteration.

**Architecture (anticipated):**
- Components: `MobilityKind`, `Position{x,y}`, `Velocity{x,y}`, `WalkPlan`, `RoutePosition{route, link, progress}`, `SpriteKey`. Per-component tables enable SIMD-friendly tick systems.
- Systems: `walk_advance_system`, `road_vehicle_advance_system`, `compute_world_coord_system`, `dirty_marker_system`.
- `MaterializedRuntime` (already in `sim-core/src/ecs_runtime.rs` for Players/Items) gets extended with mobility component sets.
- Persistence adapter unchanged from outside; the Postgres-shape of `mobility_snapshots` survives because the JSON-payload abstracts over storage.

**Dependencies:** Phase 4 (we want viewport filtering working before tick budget pressure forces the migration).

**Non-goals:** SoA/AoS comparison microbenchmarks (literature-guided design only); changing the persistence adapter contract.

### Phase 6 — Chunk-LOD Simulation

**Spec filename:** `docs/superpowers/specs/2026-05-21-chunk-lod-mobility-design.md` (illustrative).

**Purpose:** 1M entities × 10 Hz × per-agent simulation = 10M ops/sec — beyond a sustainable budget. Chunks already have `Asleep/Warm/Active/Hot` activity states per `scheduler.rs`. This phase wires those states to actual mobility tick budgets, per `dynamic-lod-large-scale-agent-urban-simulations-aamas2011.pdf` and the Citybound / Subway Simulator gravity-model precedent.

**Architecture (anticipated):**
- Mobility tick is per-chunk: `hot chunks` run full per-agent fidelity; `active chunks` run normal fidelity; `warm chunks` run aggregate update (gravity-model commuter flow between stops); `asleep chunks` run lazy catch-up only when activated (per v2 spec invariant: "when a chunk wakes, its state must be explainable from prior state plus accepted events plus elapsed simulation time").
- Aggregate model is a tile-space gravity flow: `flow(A → B) ∝ population(A) * attractiveness(B) / distance(A,B)^2`, integrated per coarse tick. Agents in warm chunks are tracked as flow totals, not individuals.
- Chunk activation promotes flow totals back into discrete agents at the chunk's boundary; deactivation collapses individuals back into flow totals.

**Dependencies:** Phase 5 (need ECS dense iteration before per-chunk tick budgeting is meaningful).

**Non-goals:** continuous lod (we use the existing 4-state scheduler, not a continuous spectrum); player-visible behavior changes within `Active`/`Hot` chunks.

### Phase 7 — Per-Chunk Persistence Partitioning

**Spec filename:** `docs/superpowers/specs/2026-05-22-chunk-mobility-persistence-design.md` (illustrative).

**Purpose:** `mobility_snapshots` is currently one row per world. At 1M agents that's a ~GB JSON blob per UPSERT — untenable. Partition per chunk so each tick's snapshot loop writes only the chunks that changed.

**Architecture (anticipated):**
- New table `chunk_mobility_snapshots` with PK `(world_id, chunk_x, chunk_y)` and per-chunk JSONB payload.
- Snapshot loop iterates chunks (already chunk-aware from existing `chunk_snapshots` work) and writes only dirty chunks.
- Hydration reads only chunks that need to be activated (lazy load), per v2 spec's "asleep chunks use lazy catch-up from durable events, timers, and aggregate rules."
- Old `mobility_snapshots` table deprecated and dropped in a follow-up migration once `chunk_mobility_snapshots` is verified in production.

**Dependencies:** Phase 6 (per-chunk simulation is prerequisite for per-chunk persistence to have meaning).

**Non-goals:** event-log partitioning (events stay single-table with existing per-chunk_version index from chunk-recovery).

### Phase 8 — Production Hardening

**Spec filename:** `docs/superpowers/specs/2026-05-23-mobility-production-hardening-design.md` (illustrative).

**Purpose:** Make the 1M-agent simulation production-grade: load-tested, observable, recoverable, deployable.

**Architecture (anticipated):**
- Synthetic load harness: spawn N headless WebSocket clients each subscribing to random viewports; verify server holds tick budget at target N.
- Metrics endpoint exposing Prometheus-format: tick duration p50/p99, chunk activity distribution, persistence write backlog, WebSocket subscription count, viewport size distribution, snapshot decode failures.
- Structured logging via `tracing` already present in the backend; this phase adds per-tick span sampling and operator-facing log levels.
- Migration safety: pre-flight dedup before `CREATE UNIQUE INDEX` on `world_events` (deferred follow-up from chunk-recovery final review). Migrations gated on schema version table.
- Deploy pipeline: Dockerfile, infrastructure-as-code (out-of-repo, scoped to deploy story), Postgres connection pooling tuned for the load profile.
- Documentation: runbook for operator (start/stop/restart, log destinations, alert thresholds).

**Dependencies:** Phase 7 (no point hardening before scale story is complete).

**Non-goals:** auth + permissions (separate roadmap), Supabase service-role-key usage, frontend production hardening (this roadmap is backend-scoped).

---

## Dependency graph

```text
Phase 1 (Visible Mobility)
  ├── Phase 2 (Interpolation)
  └── Phase 3 (Procedural Population)
            ↓ both
         Phase 4 (Viewport Replication)
            ↓
         Phase 5 (ECS Hot Path)
            ↓
         Phase 6 (Chunk-LOD)
            ↓
         Phase 7 (Per-Chunk Persistence)
            ↓
         Phase 8 (Production Hardening)
```

Phases 2 and 3 can run in parallel after Phase 1.

## Scale checkpoints

| After phase | Sustainable agent count (backend) | Visible-on-canvas count | Notes |
|---|---|---|---|
| Phase 1 | ~300 | ~300 | All entities sent to client; HashMap storage |
| Phase 2 | ~300 | ~300 | Smooth motion via interpolation |
| Phase 3 | ~10k | ~10k | Still no viewport filter; only acceptable because we still test single client |
| Phase 4 | ~10k | ~hundreds | Client only sees viewport subset |
| Phase 5 | ~100k | ~hundreds | ECS dense storage; tick budget breathing room |
| Phase 6 | ~1M | ~hundreds | LOD makes per-tick work proportional to active-chunk count, not entity count |
| Phase 7 | ~1M (durable) | ~hundreds | Persistence partitioned per chunk |
| Phase 8 | ≥1M, 24h continuous | ~hundreds | Production contract satisfied |

## Resolved questions

- **Final scale**: 1M. 10M is explicitly out of scope and will be re-scoped after 1M ships.
- **Decomposition strategy**: master roadmap + Phase-1 detail spec written now. Phases 2–8 get their detail specs immediately before their implementation slice.
- **Backend authority**: unchanged from v2 spec — Rust authoritative, browser observes and submits validated commands.
- **Path data sharing (Phase 3)**: backend will own the canonical city descriptor; frontend reads the same generated file. Source-of-truth shifts from frontend to a generated artifact, not from frontend to backend manually maintained code.
- **LOD strategy (Phase 6)**: discrete 4-state via existing scheduler + gravity-model aggregate, not continuous LOD. Aligned with `dynamic-lod-large-scale-agent-urban-simulations-aamas2011.pdf` discrete-LOD recommendation.
- **People vs vehicles separation (Phase 1+)**: explicit per `local-road-vehicles` and `pedestrian-agents` specs already in place. `RoadVehicleWorld` is a new backend subsystem distinct from the existing transit `VehicleRecord`.

## Out-of-scope (entire roadmap)

- Multi-process worker sharding (path to 10M+, SpatialOS / Entity-Component-Worker pattern). Acknowledged in `docs/superpowers/specs/2026-05-14-abutown-simulation-architecture-v2-design.md` as the post-1M direction.
- WebGPU/WebGL accelerated frontend rendering (canvas 2D + viewport filtering should hold for hundreds-of-visible target).
- Authentication / player accounts / permissions / Row Level Security beyond what already exists for card-hand.
- Binary wire protocol (JSON remains throughout; binary is a Phase-8+ optimization if metrics show it's needed).
- Cross-region replication / DR.
- Economy, ledger, production, combat — entirely separate game-mechanics roadmap.
