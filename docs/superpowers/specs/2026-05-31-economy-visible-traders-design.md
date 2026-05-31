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

An **exclusive** system (`fn(&mut World)`) in a new `EconomySet::Materialize`
(after `WarmFlow`, before `Telemetry`). It reads `Traders`, `Markets`, the routing
`Graph` + `HpaIndex` + `FlowFieldCache`, `EconomyConfig`, the `Tick`; and owns a
new resource `MaterializedTraders(BTreeMap<EconomicActorId, Entity>)`. It **no-ops
when no routing graph is present** (pure-economy test schedules), keeping the
economy schedule runnable without a graph.

It runs in two borrow-clean phases:
1. **Route phase** — compute each trader's current-leg footway route polyline
   (§B) into an owned `BTreeMap<actor, polyline>` (releasing the routing borrows).
2. **Plan + apply phase** — a pure `plan_mutations` samples each route at the
   trader's authoritative progress (`leg_progress`) and decides Spawn (new) /
   Update (`Position`+`Direction`) / Despawn (trader left `Traders`).
   `apply_mutations` performs them and — critically — **inserts each live
   trader-agent into `DirtyAgents` every tick**, so it flows through the existing
   per-tick delta builder (`tick_mobility` drains `DirtyAgents` →
   `MobilityChunkDelta.changed_agents` → WebSocket). **Feeding the per-tick
   *delta* (not only the subscribe-time *snapshot*) is what makes the trader move
   smoothly on the client** — the prior attempt only touched the snapshot, so it
   jumped on resubscribe and never walked.

**Client visibility uses the standard machinery, not a special chunk gate.** The
trader-agent is kept alive and dirtied at its real position every tick (exactly
like a normal agent); when it crosses out of a client's subscribed chunk the
delta's `left_agents` (computed from `PreviousAgentChunks` on chunk change) clears
it client-side — no ghost. (An earlier "despawn when the current chunk is
unobserved" idea was dropped: a despawned entity makes `agent_record_from_entity`
return `None`, so **no** `left_agents` would be emitted and the agent would ghost
on the client.) Per-chunk LOD *despawn* of unobserved trader-agents is a deferred
optimization; the economy's dormant gate already bounds how many traders advance.

The system is **strictly render-only**: it never touches accounts/inventory/
orders/ledger/`Traders`, so it cannot affect conservation. Trader-agents are also
**excluded from mobility persistence** (`extract_from_world` skips `TraderAgent`):
they are a projection of the persisted economy, re-created on hydrate by the
bridge — never double-persisted, never counted as base-world agents.

### D. Distinct trader sprite (zero protobuf change)

The trader's `SpriteKey` is `"trader:{hash}"`. The frontend already carries
`sprite_key` on the wire (`AgentMobilityDto`) and selects a sprite via
`spriteIndexFromKey` (`render/backendMobilityDrawables.ts`). We extend the sprite
layer to recognize the `trader:` prefix and render a **distinct trader
sprite/color** (a new entry in `render/minimalPedestrianSprites.ts` +
`drawPedestrian` in `render/minimalMapRenderer.ts`). **No new protobuf field, no
new DTO** — the existing `sprite_key` string channel carries the kind.

### E. Live demo economy seed (data-driven)

`EconomyPlugin` is already installed in the live runtime but **no markets/traders/
pools are seeded**, so the live economy is currently inert. A new
`economy::seed::seed_demo_economy(world)` is an **idempotent bootstrap**: its first
act is `if !Markets.is_empty() { return; }`, so it seeds only a world that has no
economy yet (brand-new, or created before the economy existed) and no-ops once a
world already has markets. It is called on **both** runtime paths — the fresh path
**and**, crucially, the hydrate path (after the economy snapshot is restored),
because the production server **always hydrates** (`build_app_from_config` →
`hydrate_from_stores`); a fresh-path-only seed would never run live. The economy
fully persists (`EconomyPersistSnapshot` round-trips Markets, MarketChunks, pools,
Traders, accounts, inventory), so once seeded the bootstrap skips on every
subsequent hydrate — it never duplicates markets or resets trader progress (the
demographic-replay failure class). This idempotent demo-content bootstrap is **not**
a heal-on-restore shim. **Data-driven — no hardcoded coordinates** (world-drift
lesson): it picks
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

**As built — backend (sim-core, headless):**
1. `trader_render::*` (pure) — `route_polyline` concatenates+dedupes edge
   polylines; `leg_progress` maps the countdown to `[0,1]`; `is_outbound`.
2. `trader_agent_world_coord_reads_position_verbatim` — `world_coord_for_agent`
   returns a `TraderAgent`'s authoritative `Position`.
3. `materialize_spawns_trader_agent_at_route_start_and_feeds_delta` — Buying →
   one `TraderAgent` at the route start, namespaced `trader:` id, **in
   `DirtyAgents`** (delta-fed).
4. `materialize_despawns_when_trader_removed_from_economy` — trader leaves
   `Traders` → its agent is despawned + dropped from index/`MaterializedTraders`.
5. `materialize_does_not_touch_money_or_goods` — N runs → balances unchanged
   (render-only).
6. `seed_demo_economy_creates_two_markets_and_one_trader` — seed yields 2 anchored
   markets at distinct finite nodes + one trader.

**As built — backend (sim-server, real routing):**
7. `live_runtime_seeds_demo_markets_and_trader` — a fresh runtime seeds 2 markets
   + 1 trader.
8. `hydrate_with_empty_economy_store_bootstraps_demo_economy` — hydrating a world
   with no persisted economy bootstraps the demo economy (the live-server path).
9. `seeded_trader_walks_the_footway_route_and_conserves` — subscribe to the demo
   chunks, tick the full schedule: the trader is fed into the per-tick delta, its
   `world_coord` changes (walks the real route), money + goods conserved.

**As built — frontend (vitest):** `isTraderSpriteKey` + a tagging test
(`trader:`-keyed agents → `kind: 'trader'`).

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
