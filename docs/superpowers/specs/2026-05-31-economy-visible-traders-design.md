# Visible Economy Traders — traders as real walking mobility agents

Date: 2026-05-31

## Status

Economy roadmap **slice 4, third sub-slice**: the visible materialization that
`economy-lod-v0` explicitly deferred — *"active chunks materialize traders … as
walking mobility agents"* — the first economy feature to cross the
**frontend↔backend render boundary**. Per CLAUDE.md a real **browser smoke is
mandatory** (the Phase-7a coord-mismatch shipped 100% broken with all unit tests
green).

This spec **creates** a design that no prior spec fully pins down; it is grounded
in two established specs and preserves both their invariants:

- **`round-trip-movement` (2026-05-30)** — the canonical agent movement model:
  purposeful walking between waypoints over the footway graph via HPA*/flow-field,
  backend-only, streamed `world_coord` traces the path, client interpolates.
- **`economy-lod-v0` (2026-05-30)** — the economy core stays **graph-free,
  countdown-authoritative, freeze/resume on dormancy, conservation-exact**;
  markets/traders anchored to chunks via `MarketChunks`/`DormantMarkets`; traders
  anchored to their **source** ("home") market.

**User scope decisions:** (1) state-of-the-art, performant, beautiful,
**server-authoritative**, built *with* the developed architecture — no parallel
engine, no half-measures; (2) traders move as **real walking mobility agents** on
the real footway network (movement is the feature, not decoration); (3) a
**distinct trader sprite from the start**; (4) seed a small **demo economy into
the live world** so traders are actually visible.

## The core idea: traders are agents, at the architecture's own LOD

The simulation already applies one thesis everywhere — *full per-entity fidelity
when observed, a cheap aggregate/abstract model when not* (mobility: per-agent in
Active/Hot, flow cells in Warm; economy-lod: full auction in Active/Hot, warm-flow
aggregate in Warm, frozen in Asleep). **Visible traders are the same thesis applied
to trader travel:**

| Tier | Trader travel representation |
|------|------------------------------|
| **Active/Hot** (observed) | a **real walking agent** on a real footway route, replicated per-tick, interpolated, distinct sprite |
| **Warm/Asleep/dormant** (unobserved) | **no agent**; the abstract `remaining` countdown advances (or freezes when asleep) — exactly `economy-lod-v0` today |

There is **one source of truth** for travel progress across both tiers: the
economy's durable, deterministic, persisted `TraderState` countdown
(`ToDest{remaining}` / `ToSource{remaining}`). The observed agent **renders that
progress** as a real footway walk; it never owns a second clock. This is the
decisive design choice (see §"Design decision" for the rejected alternative).

## Architecture

### A. The trader-agent entity (render-only, owned by the bridge)

A materialized trader is a dedicated entity carrying **render components only — no
`AgentMarker`** — so none of the mobility movement/bookkeeping systems
(`walk_advance`, `route_assignment`, `route_advance`, `compute_world_coord_system`,
`compute_direction_system`, `track_chunk_populations_system`,
`demote_active_to_warm_system` — all filter `With<AgentMarker>`, verified in
`mobility/systems/*`) ever touch it. The materialization bridge is its sole owner.

Components (mirroring `spawn_agent_from_record` in `mobility/api.rs:401` so the DTO
builder works unchanged):

- `TraderAgent` — new marker (`mobility/components.rs`).
- `StableAgentId(AgentId("trader:{actor}"))` — deterministic per economic actor.
- `Position { x, y }` — **authoritative** world (tile) coord, written by the bridge.
- `Direction`, `SpriteKey("trader:{hash}")`, `BirthTick`.
- `AgentMobilityStateComponent(Walking{..benign..})` and `WalkPlan{stages:[],..}` —
  only to satisfy the DTO's `state`/`plan_cursor` fields; not used for movement.

`world_coord_for_agent(world, id)` (`mobility/api.rs`) gains a `TraderAgent`
intercept: return the entity's `Position` verbatim (its position is authoritative,
not state-derived). The agent DTO shape (`AgentMobilityDto`,
`protocol/src/lib.rs`) is **unchanged** — still `world_coord{x,y}` in tile units.

> This component model + the `world_coord_for_agent` intercept are the *salvageable*
> core of the prior attempt; what was wrong there is fixed in B and C.

### B. Real footway routes (not straight lines)

When a trader enters a travel leg (`ToDest`/`ToSource`) the bridge computes a
**real footway route** between the source and destination market nodes using the
**existing routing machinery** — the same HPA* corridor + flow-field path that
`route_assignment_system` uses for pedestrians (`hpa.corridor_between(origin, dest,
RoutingProfileKey::Walk)` → flow field → `materialize_route_steps` →
edge polyline). The route is computed once per leg and cached as a polyline
(reusing `mobility_geometry::world_coord_at_progress_slice`). Traders therefore
walk sidewalks/grass and avoid roads/buildings/water, identical to every other
walker (`grass-footway-walking`).

Per tick, the bridge samples the cached route polyline at
`progress = (travel − remaining) / travel`, where `travel =
transport_ticks(distance_tiles, config)` — i.e. the agent's on-screen position is
the economy's authoritative travel progress projected onto the real path.
`Buying` → at source node; `Selling` → at dest node. `Direction` is derived from
the polyline tangent at `progress` (reusing `direction_at_progress_slice`).

If no walking route exists between two market nodes the bridge logs and falls back
to not materializing that leg (the markets must be footway-reachable; the seed
guarantees this and a test asserts it) — never a panic, never a straight line
through terrain.

### C. The materialization bridge (`materialize_traders_system`)

A new system in a new `EconomySet::Materialize` (after `WarmFlow`, before
`Telemetry`: `… ClearMarkets → WarmFlow → Materialize → Telemetry`). It reads
`Traders`, `Markets`, `MarketChunks`, the routing `Graph`, the Active/Hot chunk
set (the same `Query<&ChunkCoordComp, Or<(With<ActiveChunk>, With<HotChunk>)>>`
the LOD bridge uses), `EconomyConfig`, the `Tick`; and owns a new resource
`MaterializedTraders(BTreeMap<EconomicActorId, Entity>)` via `Commands`.

Per trader, each tick:
1. Compute current world position from `TraderState` + route (§B).
2. `chunk = chunk_of(pos, CHUNK_SIZE)`.
3. If `chunk ∈ Active∪Hot`: **materialize/update** — spawn the trader-agent if
   absent (else write its `Position`/`Direction`), and **insert its entity into
   `DirtyAgents`** so it flows through the existing per-tick delta builder
   (`tick_mobility` in `mobility/api.rs:761` drains `DirtyAgents` → per-chunk
   `MobilityChunkDelta.changed_agents` → WebSocket). **This — feeding the
   per-tick *delta*, not only the subscribe-time *snapshot* — is what makes the
   trader move smoothly on the client** (the prior attempt only touched the
   snapshot, so it jumped on resubscribe and never walked).
4. Else (current chunk unobserved): **despawn** the agent if present and drop it
   from `MaterializedTraders` (the delta's chunk-departure handling via
   `PreviousAgentChunks` emits the `left_agents` removal so the client clears it).

The system is **strictly render-only**: it never touches
accounts/inventory/orders/ledger/`Traders` state, so it cannot affect money/goods
conservation. It only mirrors authoritative trader progress into a visible walker.

> Render materialization gates on the trader's **current-position** chunk (so a
> trader walking through an observed chunk is visible there), while the economy's
> existing **dormancy** gates compute on the **source** chunk (`economy-lod-v0`,
> unchanged). For the demo, the seed places the whole route inside the default
> observed view so the trader is visible end-to-end without panning; cross-LOD
> materialize/dematerialize mid-walk is handled by the same current-chunk gate.

### D. Distinct trader sprite (zero protobuf change)

The trader's `SpriteKey` is `"trader:{hash}"`. The frontend already carries
`sprite_key` on the wire (`AgentMobilityDto`) and selects a sprite via
`spriteIndexFromKey` (`render/backendMobilityDrawables.ts`). We extend the sprite
layer to recognize the `trader:` prefix and render a **distinct trader
sprite/color** (a new entry in `render/minimalPedestrianSprites.ts` +
`drawPedestrian` in `render/minimalMapRenderer.ts`). **No new protobuf field, no
new DTO** — the existing `sprite_key` string channel carries the kind.

### E. Live demo economy seed (data-driven)

`EconomyPlugin` is already installed in the live runtime
(`sim-server/src/runtime/mod.rs:217` fresh path and `:340` hydrate path) but **no
markets/traders/pools are seeded**, so the live economy is currently inert. A new
`economy::seed::seed_demo_economy(world)` runs **only on the fresh-world path**
(after the line-217 install). The economy fully persists (`EconomyPersistSnapshot`
round-trips Markets, MarketChunks, pools, Traders, accounts, inventory), so a
hydrated world restores the demo economy from persistence — the seed is **not**
run on the hydrate path, and there is **no double-seed guard / heal-on-restore
shim** (re-seeding would duplicate markets and reset trader progress — the
demographic-replay failure class). **Data-driven — no hardcoded coordinates**
(world-drift lesson): it picks
two footway-reachable base-world graph nodes near the default abutopia view
(deterministically), creates two `MarketSite`s anchored to them, adds their
`MarketChunks` entries via `chunk_of(node.position)`, seeds a supplier pool
(sells a good cheap at market A), a demand pool (buys dear at market B), funds the
supplier's goods and the trader/consumer cash, and inserts **one `Trader`** cycling
A↔B. The existing economy drives the trader through Buying→ToDest→Selling→ToSource;
the bridge renders it walking whenever its current chunk is observed.

## Design decision (explicit; the one genuinely unspecified choice)

**Chosen — economy-authoritative travel, rendered as a real footway walk.** The
deterministic `remaining` countdown is the single source of truth (persists,
freezes/resumes, conserves); the observed agent renders that progress on a real
HPA*/flow-field footway route. **Rejected — a fully autonomous `walk_advance`
agent whose arrival drives the economy.** It would re-couple the economy to the
routing `Graph` at runtime and break `economy-lod-v0`'s graph-free + freeze/resume
invariant on the merged, tested, conserved core. The chosen design keeps that core
untouched while still showing genuine footway walking, and is consistent with the
LOD thesis (the agent is the Active/Hot-tier projection of the same journey the
countdown represents at the Warm/Asleep tier).

## Conservation / determinism

- **No economy mutation:** materialization reads trader state + graph and
  spawns/moves/despawns render entities only → money/goods conservation untouched
  (asserted by running the full schedule N ticks and checking totals).
- **Deterministic:** `BTreeMap`-keyed traders + `MaterializedTraders`, integer
  travel ticks, deterministic routes (routing is already deterministic),
  deterministic agent ids and sprite hash. The only float is the polyline
  position sample (pure function of integer inputs).
- **Server-authoritative:** the backend simulates and positions; the client only
  renders/interpolates streamed `world_coord` (no client-side trader logic).

## Testing

**Backend (sim-core, headless):**
1. `materialize_spawns_trader_agent_in_active_chunk` — trader Buying, source market
   in an Active chunk → exactly one `TraderAgent` at the source node position; its
   DTO `world_coord` equals the node position.
2. `materialize_follows_footway_route_during_travel` — trader `ToDest{remaining}`
   → agent position lies **on the computed footway route** at the expected
   progress (not the straight-line midpoint), and the entity is in `DirtyAgents`.
3. `materialize_despawns_when_current_chunk_unobserved` — current chunk Warm/Asleep
   → trader-agent despawned/absent.
4. `materialization_does_not_touch_money_or_goods` — full schedule N ticks with a
   seeded trading economy → total money + goods conserved.
5. `trader_agent_untouched_by_mobility_systems` — a chunk demote / a mobility tick
   does not move, despawn, or rebucket a `TraderAgent` (no `AgentMarker`).
6. `seed_demo_economy_creates_reachable_markets_and_trader` — seed yields 2 markets
   with `MarketChunks` entries, finite node positions, a **walking route exists**
   between them, and one trader.

**Browser smoke (MANDATORY — `scripts/smoke-visible-traders.mjs`, adapted from
`smoke-7b.mjs`):** launch the dev stack (backend + vite), open headless chromium,
let WS connect + initial chunk subscribe fire, and assert: (a) a per-tick mobility
**delta** arrives containing an agent whose id starts `trader:` at ~the seeded
market tile; (b) over a few seconds that agent's `world_coord` **changes along a
path** (it walks); (c) it renders with the trader sprite (kind derived from the
`trader:` `sprite_key`); (d) no console errors. This is the acceptance gate that
catches any spawn/chunk/coordinate/delta mistake the unit tests cannot.

Full gate: fmt + clippy `-D warnings` + `test --workspace --all-targets` on the
CI-matching stable toolchain, **plus** a green browser smoke run.

## What this is NOT (deferred)

- No real-walk-time-driven economy (travel timing stays the deterministic
  `transport_ticks` countdown; the route is the visible projection of it).
- No multi-good / dynamic-route traders (fixed A↔B per trader, as today).
- No player interaction with traders (server-authoritative NPCs).
- No warm/asleep visible traders (only Active/Hot materialize — consistent with
  mobility + economy LOD).

## Base branch / discard

Built on **origin/main** (this `plan/economy-visible-traders` worktree, main+2),
**not** the stale `plan/persistence-liveness` (81 commits behind main). The prior
attempt's snapshot-only path (committed `TraderAgent` + uncommitted
`materialize.rs` straight-line lerp) is **discarded**; the `TraderAgent` component
+ `world_coord_for_agent` intercept idea is re-used, with the delta-path feed and
real footway routing added.
