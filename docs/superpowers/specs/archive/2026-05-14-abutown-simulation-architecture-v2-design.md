# Abutown Simulation Architecture v2

Date: 2026-05-14

## Status

Draft architecture update. This replaces the older traffic-first backend direction with a compact state-of-the-art architecture target for one persistent aquarium world.

## Goal

Abutown is one persistent, always-on browser game world. The first product mode remains aquarium-first: players observe the same authoritative world, pan, zoom, and later submit validated indirect actions. The backend must be able to grow toward roughly 2000 connected players and 1M+ durable world entities/items/assets without turning the browser or database into the hot simulation loop.

## Non-Goals

- No economy, ledger, citizen, combat, or production mechanics in this spec.
- No exact database schema or implementation plan.
- No player-facing shards, rooms, matches, or instancing.
- No per-frame database writes.

## Architecture

```text
Browser client
  -> HTTP snapshot and asset metadata
  -> Rust-owned WebSocket delta stream
  -> Rust authoritative simulation mesh
  -> hot chunk state in memory
  -> batched events and chunk snapshots
  -> Supabase/Postgres durable world state
```

The Rust backend is the only simulation authority. Supabase/Postgres stores durable world state, tile state, snapshots, event history, account-adjacent data, and admin/query projections. Supabase Edge Functions and Supabase Realtime are not the primary game simulation path because the hot loop needs long-lived CPU work, bounded tick budgets, and custom interest management.

## ECS And Data Layout

Use a Rust data-oriented simulation core. `bevy_ecs` is the preferred starting point because it is current, standalone-capable, table/sparse-set aware, change-detection capable, and built around parallel systems. Keep an internal abstraction boundary so hot subsystems can move to custom SoA arrays if measurement shows ECS overhead.

Important rule: every tile is durable, but every tile is not a heavy ECS entity in the hot path.

- Tiles live in dense chunk-local arrays.
- Tile properties use compact component arrays, dirty bitsets, and version counters.
- Dynamic materialized objects use ECS entities with stable external IDs and dense runtime indices.
- Cold or unobserved entities may stay as durable records until a chunk activates.
- Hot systems operate on contiguous arrays and ECS queries, not deep object graphs.

## Chunks And Authority

The world is partitioned into fixed chunks. Chunks are internal scheduling and persistence units, not visible worlds.

Chunk states:

- `asleep`: no player nearby; only durable state plus scheduled catch-up.
- `warm`: low-frequency aggregate updates and preload candidate.
- `active`: normal simulation and replication.
- `hot`: high player density, many mutations, or important events.

Each active chunk has one authoritative simulation worker at a time. Workers may own multiple chunks. Authority transfer must be explicit, versioned, and recoverable.

## Tile Persistence

Every world tile exists in Supabase/Postgres with stable coordinates and mutable properties. The live server loads chunks into memory, mutates tile arrays authoritatively, marks changed tiles/chunks dirty, and writes durable state in batches.

Persistence shape:

- immutable base world generation seed/version,
- queryable durable tile state,
- append-only domain/tile mutation events,
- periodic chunk snapshots,
- compacted latest chunk state for fast recovery.

Critical accepted mutations are event-sourced promptly. Full tile/chunk state is flushed periodically, on chunk unload, and before maintenance shutdown. Postgres partitioning should be used for large event/tile tables by world/chunk/time where queries benefit from partition pruning.

## Simulation LOD

The whole world is internally simulated, but not at one uniform frequency. Simulation LOD is part of the truth model, not a visual trick.

- Hot chunks run detailed, frequent ticks.
- Active chunks run normal game ticks.
- Warm chunks run coarser scheduled updates.
- Asleep chunks use lazy catch-up from durable events, timers, and aggregate rules.

The invariant is causal consistency: when a chunk wakes, its state must be explainable from prior state plus accepted events plus elapsed simulation time.

## Replication

The browser receives relevance-filtered deltas, never the full million-entity world stream.

Replication rules:

- initial snapshot by viewport and subscribed interests,
- monotonic tick/version IDs,
- delta coalescing for slow clients,
- resync on gaps,
- priority budgets for nearby, selected, owned, or otherwise relevant entities,
- interpolation/prediction only for presentation, never authority.

Large gatherings are allowed in one world. If 2000 players converge, the system degrades by reducing detail, lowering update rates, coalescing deltas, and optionally slowing local simulation time rather than creating fake instances.

## Database Role

Supabase/Postgres is the durable source of record, not the realtime engine.

- The Rust service writes through controlled server credentials.
- Browser clients do not directly mutate canonical world tables.
- Row Level Security protects exposed user/admin tables.
- Bulk world initialization and compaction should use Postgres bulk-loading patterns where practical.
- Supabase Realtime may support low-frequency admin/app views, not high-frequency game deltas.

## Browser Role

The browser is a high-performance renderer and interaction client. WebGPU can be used for rendering and compute-assisted presentation where available. WebAssembly and workers can support local prediction, decoding, path preview, and tooling, but server state remains authoritative.

## Scale Principles

- Measure before claiming scale.
- No unbounded per-entity async tasks.
- No database writes in the fixed-tick hot path.
- No full-world broadcast.
- Store hot state cache-friendly.
- Keep stable durable IDs separate from dense runtime indices.
- Use dirty tracking, chunk snapshots, and event compaction.
- Treat every subsystem as budgeted work inside a scheduler.

## References

- Bevy ECS docs: https://docs.rs/bevy/latest/bevy/ecs/index.html
- Flecs ECS design reference: https://www.flecs.dev/flecs/index.html
- Unity Netcode for Entities overview: https://docs.unity.com/en-us/multiplayer/netcode/netcode
- Unreal Replication Graph overview: https://www.unrealengine.com/tech-blog/replication-graph-overview-and-proper-replication-methods
- EVE Online Time Dilation: https://www.eveonline.com/news/view/introducing-time-dilation-tidi
- EVE fleet fight node allocation: https://www.eveonline.com/de/news/view/fleet-fight-notification-tool
- Supabase Row Level Security: https://supabase.com/docs/guides/database/postgres/row-level-security
- Supabase Edge Function limits: https://supabase.com/docs/guides/functions/limits
- PostgreSQL partitioning: https://www.postgresql.org/docs/current/ddl-partitioning.html
- PostgreSQL bulk loading guidance: https://www.postgresql.org/docs/17/populate.html
- MDN WebGPU: https://developer.mozilla.org/en-US/docs/Web/API/WebGPU_API
- WebAssembly threads guide: https://web.dev/articles/webassembly-threads
