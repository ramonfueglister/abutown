# Phase 8a — World Unification Foundation

**Date:** 2026-05-20
**Status:** Design
**Phase in roadmap:** 8a (foundation for 8b–h)

## Goal

Unify abutown's two-world architecture (the standalone `MobilityWorld` Bevy ECS plus the separate `ChunkRegistry` HashMap) into a single `SimWorld` Bevy ECS, with Chunks as entities and a plugin-based composition that lets future subsystems (routing, time, economy, weather, …) be added without touching the foundation.

This phase ships **no new features**. It builds the architectural substrate the rest of the roadmap rests on.

## Why now

The repo is committed to a 100k-agent / 10k-vehicle target ("Cities Skylines I level"). The current architecture has two problems blocking that scale and the roadmap behind it:

1. **Two worlds, two mutation paths.** `ChunkRegistry` (HashMap) and `MobilityWorld` (Bevy) do not share state. A tile mutation today cannot reach mobility's routing/cache layers; a mobility event cannot consult tile properties. Every future cross-cutting feature (routing-cache invalidation on road removal, agents bound to home-tiles, economy markets bound to storage-tiles) is blocked.

2. **Subsystems live in `SimulationRuntime`, not in plugins.** Adding a new subsystem (time, economy, AI) today means editing the runtime struct, the tick loop, and the persistence path simultaneously. There is no clean extension boundary.

Phase 8a fixes both without adding features. After 8a, each subsequent phase is a new Bevy `Plugin` that registers components, events, resources, and systems through stable interfaces.

## Reference models

Architectural decisions in this spec are informed by:

- **Bevy ECS 0.18 idioms** (Plugins, Relationships, ScheduleLabel, `Changed<T>` filters, archetype-based marker queries).
- **bevy_ecs_tilemap, bevy_voxel_world** for chunks-as-entities + dense tile arrays.
- **Songs of Syx** for individual-agent simulation at 10k+ scale with batched decisions.
- **Dwarf Fortress** for the stocks/flows + intrinsic-value model that informs how the foundation should not assume any specific economy.

The foundation expresses no opinions about features; it provides the substrate where those features can later live.

## Architectural Principles

These principles bind every design decision below.

### 1. Plugin composition

Each subsystem is a Bevy `Plugin`. Phase 8a delivers `CorePlugin` (chunks, tiles, LOD, world lifecycle) and refactors mobility into `MobilityPlugin`. Future phases each become their own plugin (`RoutingPlugin`, `TimePlugin`, `EconomyPlugin`, …). Plugins register their own components, events, resources, and systems and never reach into another plugin's internals.

### 2. Events as subsystem boundaries

The foundation emits Bevy `Event`s at every observable lifecycle moment (chunk loaded, chunk unloaded, tile changed, chunk LOD changed). Downstream plugins consume the events they care about. The foundation never knows who's listening. Adding new subsystems = adding event handlers, not editing the foundation.

### 3. Stable public API per plugin

Each plugin's `mod.rs` re-exports exactly the public surface (components, events, resource types, plugin struct). Implementation details (`pub(crate)` systems, helper functions) are not exported. Other plugins consume only the public API; internals can change without rippling.

### 4. Resource composition over mega-structs

`SimulationRuntime`'s 60-field shape goes away. State is split into small, semantic Bevy `Resource`s (`WorldId`, `TickClock`, `EventCount`, `ChunksByCoord`, …). Each plugin registers its own resources via `app.init_resource::<X>()`. Save/load, multi-world, and sharding become possible because state is no longer entangled.

### 5. ScheduleLabel hierarchy

`CoreSet` (a Bevy `SystemSet` enum) defines the foundation's execution stages: `ChunkLifecycle`, `TileMutation`, `LodReclassify`, `EventEmit`. Future plugins order their systems relative to these sets via `.after(CoreSet::TileMutation)` etc. — execution order is declarative, not hard-coded.

### 6. Determinism scaffold from day 1

A `DeterministicRng` resource (seeded from world creation) is the only RNG source. Systems never use `thread_rng()`. Cost is zero today; without it, replay / multiplayer / reproducible bug repros become impossible to retrofit later.

### 7. Persistence as a trait

Persistence is `SnapshotProvider` — a trait that knows how to read/write opaque bytes. Plugins register their `SnapshotProvider` implementations via Bevy resources. The persist loop iterates registered providers, has no knowledge of chunks or agents. Today's JSONB schema stays unchanged; it just becomes one provider implementation among potentially many.

### 8. Schema versioning hooks

Each persisted component carries a stable schema version (`pub const SCHEMA_VERSION: u32 = 1;` on the component module). Save-load wires through a migration registry. Phase 8a installs the registry empty; later schema evolutions register migrations there.

### 9. Tile-entity scaffold open for extension

The foundation ships the *mechanism* for sparse tile entities (`Tile` marker component, `LocalIndex(u16)`, `BelongsToChunk(Entity)` Relationship, a `spawn_functional_tile` helper) — but **zero domain components**. Home, Workplace, Storage, RouteNode all live in their own plugins later and attach onto the scaffold without modifying foundation code.

## SimWorld topology

A single `bevy_ecs::World` replaces `MobilityWorld` + `ChunkRegistry::HashMap`. It lives on `SimulationRuntime` (the runtime struct survives, but with most fields delegated to resources):

```rust
pub struct SimulationRuntime {
    pub world: bevy_ecs::World,
    pub schedule: bevy_ecs::Schedule,
    // event_store remains here — it crosses into persistence and is not ECS-native
    pub event_store: Arc<dyn EventStore>,
}
```

State previously held as fields on the runtime (`world_id`, `tick`, `version`, `chunk_subscriptions`, …) moves into resources inside the world.

### Entities in the world

- **Agent entities** (existing, unchanged in 8a)
- **Vehicle entities** (existing, unchanged in 8a)
- **Chunk entities** (NEW — one per loaded chunk; previously a HashMap value)
- **Tile entities** (NEW — sparse, only for tiles that need behaviour. None spawned in 8a; the scaffold is installed.)

### Resources in the world

- `WorldId(String)` — moves from runtime
- `TickClock { tick: u64, version: u64, pulse_sequence: u64 }` — moves from runtime
- `EventCount(usize)` — moves from runtime
- `ChunkSizeRes(u16)` — moves from runtime
- `WorldDimensions { width_tiles: u32, height_tiles: u32 }` — moves from runtime
- `ChunksByCoord(HashMap<ChunkCoord, Entity>)` — NEW, O(1) coord → entity lookup
- `Routes`, `Stops`, `LinkPolylines` — existing mobility resources, unchanged
- `AgentIdIndex`, `VehicleIdIndex` — existing mobility resources, unchanged
- `AgentsByChunk`, `VehiclesByChunk` — existing mobility resources, unchanged
- `DirtyAgents`, `DirtyVehicles` — existing mobility resources, unchanged
- `DirtyChunks(HashSet<Entity>)` — NEW, mirror of the existing dirty-agent pattern but for tile mutations
- `DeterministicRng(StdRng)` — NEW, seeded from `WorldId`
- `SnapshotProviders(Vec<Box<dyn SnapshotProvider>>)` — NEW, persistence registry
- `MigrationRegistry` — NEW, schema-migration table (empty in 8a)
- `CityNetwork` — moves from runtime

## Chunk entity schema

A chunk is an entity with the following components.

### Identity (immutable after spawn)

```rust
#[derive(Component, Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct ChunkCoordComp(pub ChunkCoord);

#[derive(Component, Copy, Clone)]
pub struct ChunkSize(pub u16);  // 32 today; per-chunk variability is allowed by the type but not used in 8a
```

### Terrain payload (dense)

```rust
#[derive(Component)]
pub struct Tiles(pub Vec<TileRecord>);
// Always length = chunk_size * chunk_size. Direct index access.
// TileRecord is unchanged from today's definition.
```

### Versioning + dirty tracking

```rust
#[derive(Component, Copy, Clone)]
pub struct ChunkVersion(pub u64);  // monotonic; +1 per tile mutation

#[derive(Component, Default)]
pub struct DirtyTiles(pub BTreeSet<u16>);  // tile-local indices dirty since last snapshot
```

### Persistence bookkeeping

```rust
#[derive(Component, Copy, Clone)]
pub struct LastPersistedVersion(pub u64);

#[derive(Component, Copy, Clone)]
pub struct LastSnapshotAt(pub Instant);
```

### LOD markers (mutually exclusive zero-sized markers)

```rust
#[derive(Component)] pub struct AsleepChunk;
#[derive(Component)] pub struct WarmChunk;
#[derive(Component)] pub struct ActiveChunk;
#[derive(Component)] pub struct HotChunk;

#[derive(Component, Copy, Clone, Default)]
pub struct LodCooldown(pub u8);  // hysteresis timer, ticks remaining
```

Systems transition a chunk via `commands.entity(chunk).remove::<WarmChunk>().insert(ActiveChunk)`. Archetype-based queries (`Query<&Tiles, With<HotChunk>>`) filter without runtime branches. The four states are exhaustive — a chunk always has exactly one LOD marker (a debug_assert verifies the invariant in tests).

### Subscriber tracking

```rust
#[derive(Component, Copy, Clone, Default)]
pub struct ChunkSubscriberCount(pub u8);
```

Replaces today's `ChunkSubscribers` resource.

### Tile-entity scaffold

```rust
#[derive(Component, Copy, Clone)]
pub struct Tile;  // marker on a tile entity

#[derive(Component, Copy, Clone)]
pub struct LocalIndex(pub u16);

#[derive(Component)]
#[relationship(relationship_target = ChunkTiles)]
pub struct BelongsToChunk(pub Entity);

#[derive(Component)]
#[relationship_target(relationship = BelongsToChunk)]
pub struct ChunkTiles(Vec<Entity>);  // auto-maintained reverse link
```

No tile entities are spawned by foundation systems. A `spawn_functional_tile(commands, chunk, local_index, kind) -> Entity` helper is provided for future plugins to use.

## Plugin architecture

### `CorePlugin`

Owns: world lifecycle, chunk entities, tile components, LOD markers, the `CoreSet` system set, foundation events, the determinism RNG, the snapshot registry, the migration registry.

Registers:
- Resources listed above
- Events: `ChunkLoaded`, `ChunkUnloaded`, `TileChanged`, `ChunkLodChanged`
- Systems in `CoreSet` (see schedule below)

### `MobilityPlugin`

Refactor of today's mobility module. Owns: agent + vehicle components, mobility-specific resources, mobility systems, mobility events. Does **not** own chunks any more — it consumes `ChunkLoaded` / `ChunkUnloaded` / `ChunkLodChanged` events.

Its systems run in `MobilitySet` (existing `LOD` / `Advance` / `Output` / `Bookkeeping` sets, unchanged), ordered `.after(CoreSet::LodReclassify)`.

### `PersistencePlugin`

NEW. Owns the persistence loop and the snapshot provider registry. It does not know about chunks or agents directly — it iterates `SnapshotProviders` and asks each one to write its bytes. `CorePlugin` and `MobilityPlugin` each register a provider at startup.

This split lets us later add more providers (per-plugin persistence) without touching the persistence loop.

## Event system

The foundation emits four events:

```rust
#[derive(Event, Debug)]
pub struct ChunkLoaded {
    pub entity: Entity,
    pub coord: ChunkCoord,
    pub initial_version: u64,
}

#[derive(Event, Debug)]
pub struct ChunkUnloaded {
    pub entity: Entity,
    pub coord: ChunkCoord,
}

#[derive(Event, Debug)]
pub struct TileChanged {
    pub chunk: Entity,
    pub coord: ChunkCoord,
    pub local_index: u16,
    pub old_kind: TileKind,
    pub new_kind: TileKind,
    pub new_version: u64,
    pub tick: u64,
}

#[derive(Event, Debug)]
pub struct ChunkLodChanged {
    pub entity: Entity,
    pub coord: ChunkCoord,
    pub from: ChunkLod,
    pub to: ChunkLod,
}
```

Where `ChunkLod` is a plain enum `{ Asleep, Warm, Active, Hot }` used only inside the event payload — the live state is the marker components.

Today's needs (delta broadcast, snapshot trigger) become event consumers. Future needs (routing-cache invalidation, economy reactivity) plug in without foundation changes.

## Schedule layout

```rust
#[derive(SystemSet, Hash, Eq, PartialEq, Debug, Clone)]
pub enum CoreSet {
    ChunkLifecycle,   // load/unload chunks; resolve ChunksByCoord
    TileMutation,     // apply tile changes; bump version; populate DirtyTiles; emit TileChanged
    LodReclassify,    // hot/active/warm/asleep transitions; emit ChunkLodChanged
    EventEmit,        // flush per-tick batched events for downstream consumers
}

#[derive(SystemSet, Hash, Eq, PartialEq, Debug, Clone)]
pub enum MobilitySet {
    Lod,              // existing
    Advance,          // existing
    Output,           // existing
    Bookkeeping,      // existing
}
```

Order in the per-tick schedule:

```
CoreSet::ChunkLifecycle
  → CoreSet::TileMutation
    → CoreSet::LodReclassify
      → CoreSet::EventEmit
        → MobilitySet::Lod
          → MobilitySet::Advance
            → MobilitySet::Output
              → MobilitySet::Bookkeeping
```

Future plugins (8b, 8c, …) insert their sets relative to these labels. There is no hard-coded execution order in plugin code; everything is declarative.

## Persistence boundary

```rust
pub trait SnapshotProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn schema_version(&self) -> u32;
    /// Collect the data this provider owns into a serialisable payload.
    /// Runs inside the tick task; must be fast and non-blocking.
    fn collect(&self, world: &World) -> Vec<SnapshotItem>;
    /// Apply migrations to a loaded payload before deserialising.
    fn migrate(&self, raw: SnapshotItem, from_version: u32) -> Result<SnapshotItem, MigrationError>;
}

pub struct SnapshotItem {
    pub key: SnapshotKey,        // (world_id, kind, identifier)
    pub schema_version: u32,
    pub payload: Vec<u8>,        // opaque to the persistence loop
}
```

`CorePlugin` registers a `ChunkSnapshotProvider` that emits today's `ChunkSnapshotDto` JSONB rows (schema unchanged). `MobilityPlugin` registers a `MobilitySnapshotProvider` that emits today's `mobility_snapshots` JSONB rows (schema unchanged). Postgres tables and migrations are untouched.

Future plugins add their own providers (e.g., `EconomyPlugin` later registers a `LedgerSnapshotProvider`) without changing the persistence loop.

## Determinism scaffold

```rust
#[derive(Resource)]
pub struct DeterministicRng(rand::rngs::StdRng);

impl DeterministicRng {
    pub fn from_world_id(world_id: &str) -> Self {
        let seed = blake3::hash(world_id.as_bytes()).as_bytes()[..32].try_into().unwrap();
        Self(StdRng::from_seed(seed))
    }

    pub fn next_u32(&mut self) -> u32 { /* delegate */ }
    pub fn next_f32(&mut self) -> f32 { /* delegate */ }
    // ... only methods we need
}
```

Foundation systems that need randomness consume `ResMut<DeterministicRng>`. A clippy lint and a CI grep (`grep -rn 'thread_rng\|rand::random' --include='*.rs' backend/`) prevent regressions.

Determinism is not a goal *outcome* of 8a — only the *mechanism* is installed. Verifying tick-replay determinism is a later phase's responsibility.

## Migration strategy: Big Bang

A single phase produces the foundation. Existing functionality is preserved end-to-end.

Implementation order (executed by the writing-plans phase, sketched here for spec completeness):

1. Scaffolding: new files (`world/plugin.rs`, `world/components.rs`, `world/events.rs`, `world/systems.rs`) compiled but not wired in.
2. Move `SimulationRuntime` fields into Bevy resources inside the existing `MobilityWorld`'s wrapped `bevy_ecs::World`.
3. Spawn a chunk entity for each `LoadedChunk` at hydration time, alongside the existing `ChunkRegistry` HashMap (dual-write).
4. Migrate read sites to query chunk entities via `ChunksByCoord`; HashMap remains for write parity.
5. Migrate write sites; the HashMap becomes a redundant mirror; delete `ChunkRegistry` entirely.
6. Dissolve the `MobilityWorld` wrapper struct: its `bevy_ecs::World` field moves directly onto `SimulationRuntime` (`pub world: bevy_ecs::World`), its indices become Bevy resources. "SimWorld" is the conceptual name for this consolidated world — there is no `struct SimWorld`; it is the `bevy_ecs::World` itself.
7. Extract `PersistencePlugin`; register the two providers; remove the old persistence code paths.
8. Extract `MobilityPlugin` proper (no behavioural change, only registration restructuring).
9. Acceptance: workspace tests + vitest + clippy + smoke green; the acceptance greps below all return zero.

Each step is its own commit. Browser smoke (`scripts/smoke-7b.mjs`) is run after step 5 and step 9.

## Scope

### In scope

- Single `bevy_ecs::World` (`SimWorld`) replacing `MobilityWorld` + `ChunkRegistry`.
- Chunk entities with the schema above.
- `CorePlugin`, `MobilityPlugin`, `PersistencePlugin` split.
- Foundation events.
- `CoreSet` schedule labels.
- `DeterministicRng` resource (mechanism only).
- `SnapshotProvider` trait + registry.
- `MigrationRegistry` (empty).
- Tile-entity scaffold (`Tile`, `LocalIndex`, `BelongsToChunk` relationship, `spawn_functional_tile` helper).
- Refactor of `track_chunk_populations_system`, `chunk_subscriber` flows, `apply_set_tile_kind` to operate on entities.

### Out of scope (for 8a, deferred to later phases)

- Any new feature components (Home, Workplace, Storage, RouteNode, Money, Inventory, …).
- Routing changes (A*, HPA*, flow fields).
- Wire-protocol changes — wire stays Protobuf as shipped.
- Persistence schema changes — JSONB tables stay identical.
- Determinism verification (replay test) — mechanism only, verification later.
- Parallel system execution (rayon `par_iter`) — sequential schedule stays.
- Per-tile entities for terrain — `Tiles(Vec<TileRecord>)` stays dense.
- TimescaleDB or any new persistence backend — Postgres JSONB stays.

### Future phases (mentioned only to validate the foundation supports them; not committed by this spec)

Each becomes its own brainstorm → spec → plan cycle. Listed in informal dependency order, with no commitment to scope or timing:

- 8b — Graph & spatial index (Plugin: `RoutingPlugin`'s data layer)
- 8c — A* + multi-modal + cache (Plugin: `PathfindingPlugin`)
- 8d — HPA*
- 8e — Flow fields
- 8g — Domain tiles + Bevy relationships (Home, Workplace, Storage, …)
- 8h — Economy + ledger
- 8i — Time + calendar
- 8j — Production chains
- 8k — Behavior AI
- 8l — Population dynamics
- 8m — Weather
- 8n — Determinism verification + replay
- 8o — Observability

The foundation's correctness criterion is: each of the above must be addable as a `Plugin` that registers components/events/resources/systems against the public API of `CorePlugin` and `MobilityPlugin`, without modifying foundation code.

## Acceptance criteria

8a is "done" when all of the following hold:

1. `grep -rn 'ChunkRegistry' --include='*.rs' backend/` returns zero matches.
2. `grep -rn 'pub struct MobilityWorld' --include='*.rs' backend/` returns zero matches (renamed to `SimWorld`).
3. There is exactly one `bevy_ecs::World` instance in `SimulationRuntime`.
4. `CorePlugin`, `MobilityPlugin`, `PersistencePlugin` each compile and pass their own unit tests in isolation (each plugin can be loaded into a minimal `App` without the others, for testability).
5. `grep -rn 'thread_rng\\|rand::random' --include='*.rs' backend/crates/sim-core backend/crates/sim-server` returns zero matches.
6. Persistence Postgres schema is byte-identical to today's (`chunk_snapshots` + `mobility_snapshots` tables unchanged).
7. Wire protocol bytes are byte-identical to today's (no proto schema changes).
8. Existing test suite green: cargo workspace tests, vitest, clippy `-D warnings`, tsc.
9. Browser smoke `scripts/smoke-7b.mjs` passes 9/9 with binary frames.
10. Tick performance regression budget: ≤5% on `tick_100k_all_active`. Documented either way.

## Risks

- **Bevy 0.18 Relationship gotchas.** Relationships are new; `ChunkTiles` reverse-link maintenance has edge cases on entity despawn. Mitigation: dedicated unit tests for spawn/despawn/relationship-resolution; isolate the `spawn_functional_tile` helper so all callers go through one tested path.

- **Resource extraction churn.** Splitting `SimulationRuntime` into ~12 resources risks test rewrites everywhere. Mitigation: introduce resources behind getter methods on `SimulationRuntime` first, migrate call sites incrementally, then remove the methods.

- **Hidden coupling in mobility's HashMap usage.** Mobility currently assumes `ChunkRegistry` is queriable by `ChunkCoord` directly. After migration it queries via `ChunksByCoord`. Mitigation: grep-driven enumeration of all call sites before the cutover commit; integration test that subscribes a chunk, mutates a tile in it, and verifies the delta reaches the subscribed client.

- **Persistence-loop refactor.** Splitting the persist loop from `chunk_registry` is the highest-risk single step (today they are tightly coupled). Mitigation: persistence step is its own step in the migration order with its own commit and smoke run.

## Open questions

None. All architectural decisions are settled. Open *implementation* choices (e.g., exact resource granularity, exact migration registry API shape) are deferred to the writing-plans phase, which has the freedom to refine them within these constraints.
