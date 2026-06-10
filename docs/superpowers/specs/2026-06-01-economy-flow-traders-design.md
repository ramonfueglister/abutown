# Economy Slice 2 — Visible flow-traders (the macro flow's sampled view)

Date: 2026-06-01

## Status

Approved in brainstorming (decision driven by the macro-flow spec §9 + the agreed SOTA macro/micro arc; the user delegated the choice "ausschliesslich basierend auf den specs und deinen Überlegungen"). This is **Slice 2** of the SOTA mean-field macro/micro economy: the **micro layer** — visible walking traders as a **conservation-exact, read-side sampled VIEW of the macro flow**. The macro layer (#69) stays the sole economic authority.

**Decision: transit-rendering (pure projection).** A `MacroFlow{from,to,good,qty}` edge (settled off-screen between dormant markets, #69) is rendered as a discrete trader **walking the footway route from→to**, visible only on the **observed portion** of that route. It carries **no economic state** — the goods + cash already moved in the macro-flow ledger settlement; the trader is purely a visualization. So the previously-hollow off-screen economy becomes *visible* when its goods transit your viewport.

**Explicitly NOT in scope** (deferred, per the macro-flow spec): coupling observed (Active/Hot) markets into the inter-market flow (Slice 1 deferred reading active markets as boundaries); retiring the existing observed-market economic `Trader`/`run_traders_at_tick` (#64 — that is a real economic actor for observed markets, not render; unifying it with flow-derived traders needs the auction↔flow coupling). This slice is **additive rendering only — no change to economy logic, conservation, or determinism of the existing systems.**

**Crosses the frontend↔backend boundary** (new agents on the WS mobility stream) → per CLAUDE.md the **browser-smoke is mandatory**.

## 1. Goal

Make the off-screen macro flow (#69) *visible*: when a dormant→dormant `MacroFlow` ships goods and the footway route between those markets crosses an observed chunk, a distinct trader sprite walks that route through the viewport. No wire/protobuf change (reuses the `trader:` sprite-key path from #64). No economic effect. Conservation-trivial (pure projection). Demonstrable via browser-smoke.

## 2. Architecture overview

Three small, well-bounded additions, all in `sim-core` + one seed tweak; the render/wire path is reused unchanged from #64/#66:

1. **Capture** — `run_macro_flow_system` already emits `EconomyEvent::MacroFlow{from,to,good,qty,…}` into the `TradeLedger` each interval. A new render-only resource **`FlowShipments`** records one in-transit **shipment** per accepted `MacroFlow` edge: `{ id, from_market, to_market, good, qty, start_tick, travel_ticks }`. `travel_ticks` is derived from `MarketDistances[(from,to)]` (the baked Manhattan distance) and a walk speed — the same magnitude the existing `trader_travel`/route animation uses.
2. **Render** — the existing `materialize_traders_system` (exclusive `fn(&mut World)`, #64/#66) is extended to ALSO materialize one `TraderAgent` per active shipment, positioned at `progress = (current_tick − start_tick) / travel_ticks` along the from→to footway route (reusing `leg_polyline` + `route_polyline` + `leg_progress`), and driven through the **same ghost-free `plan_mutations` lifecycle** (Spawn/Update-observed/Update-leaving/Despawn) keyed on whether the trader's current position is in an observed (Active/Hot) chunk.
3. **Retire shipments on arrival** — when `progress ≥ 1.0` the shipment is dropped (and its trader despawned via the existing leaving→despawn path).

The macro flow is the authority; `FlowShipments` is a **derived, ephemeral projection** (not persisted). The existing economic `Trader`/`run_traders_at_tick` + its #64 visible rendering are untouched and coexist (observed-market tier); flow-traders are the dormant-flow transit tier.

## 3. `FlowShipments` capture

New resource in `economy/` (render-only):

```rust
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct FlowShipments(pub BTreeMap<u64, FlowShipment>);   // keyed by shipment id (deterministic counter)

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowShipment {
    pub id: u64,
    pub from_market: MarketId,
    pub to_market: MarketId,
    pub good: GoodId,
    pub qty: Quantity,
    pub start_tick: u64,
    pub travel_ticks: u64,   // > 0
}
```

**Sampling = one shipment per accepted `MacroFlow` edge per interval.** The macro flow already aggregates per (good, src→dst) per interval, so each `MacroFlow` event is exactly one inter-market shipment — the natural quantum. Self-edges (`from == to`, intra-market clearing, transport 0) emit **no** shipment (nothing transits). A new `NextShipmentId(u64)` resource gives deterministic ids — it mirrors `NextOrderId`'s **counter pattern only**; unlike `NextOrderId` (which IS persisted, persist.rs:107) it is **EPHEMERAL — NOT persisted** (resets to 0 on restore, alongside the empty `FlowShipments`; see §5). The replay-determinism claim therefore holds within a continuous run, not across save/restore — which is correct, since the projection regenerates from the resumed flow.

**Where it's captured (corrected after review — NOT the system):** the per-flow `MacroFlow` events are local to **`run_macro_flow_at_tick`** and only surface via `ledger.0.extend(events)` at the end (macro_flow.rs:606-673) — the *system* never sees individual events. So `run_macro_flow_at_tick`'s signature gains `&mut FlowShipments` + `&mut NextShipmentId`, and a `FlowShipment` is inserted **only in the `Ok(event)` settle-success arm (macro_flow.rs:654-658) where `flow.src != flow.dst`** — never on the `MarketClearFailed` arm, never for self-edges. Iterating the `PlannedFlow` list (it carries `src`/`dst`/`q`) at that point is borrow-clean. `run_macro_flow_system` (systems.rs) just threads the two `ResMut`s in. `start_tick = current_tick`, `travel_ticks = max(1, distance-derived)`. Determinism: capture order follows the already-deterministic `plan_flows` sorted-candidate order (macro_flow.rs:344-409); `FlowShipments` is a `BTreeMap`, ids a monotone counter → replay-stable within a run.

**Expiry:** the materialize step drops any shipment with `current_tick − start_tick ≥ travel_ticks` (arrived). A safety cap (`travel_ticks` bounded; arrived shipments always removed) prevents unbounded growth.

## 4. Render: materialize transit traders

Extend `materialize_traders_system` (the exclusive `fn(&mut World)`) so that, in addition to the existing demo `Trader`s, it materializes the active `FlowShipments`:

- For each shipment, resolve `from_market`/`to_market` → graph nodes (via `Markets`), compute the Walk footway polyline with the existing `leg_polyline(graph, field, from, to)`; if no walkable route, the shipment renders nothing this tick (it still expires on schedule).
- Position = `world_coord_at_progress_slice(polyline, progress)` (the pure positioning helper, mobility_geometry.rs:25 — directly reusable) at `progress = clamp((current_tick − start_tick) / travel_ticks, 0, 1)`. Linear progress (shipments have no `TraderState`).
- **Parameterize, don't call `plan_mutations` verbatim (corrected after review).** `plan_mutations(traders: &Traders, …)` is `Trader`/`TraderState`-shaped (it reads `trader_travel` + `leg_progress(&trader.state,…)`, materialize.rs:133-194) and `apply_mutations` reconstructs the despawn id as `format!("trader:{}", actor.0)` (materialize.rs:293). Refactor the **Spawn / Update-observed / Update-leaving / Despawn state machine** (the ghost-free #66 logic worth preserving) to operate on a generic render-actor `{ actor_id: EconomicActorId, polyline, progress }`; demo-`Trader`s and shipments both produce that shape and feed the same machine. Shipment-traders use a **reserved high `EconomicActorId` offset** (e.g. `1<<32 + shipment_id`, exceeding any seedable id — seeded ids are 8001-8012, demo trader 8003, seed.rs:86-134) so they never collide with demo traders in `MaterializedTraders`; the offset scheme must be identical in Spawn and Despawn id reconstruction (`trader:{actor.0}`).
- Sprite: the existing `trader:` `sprite_key` prefix → the client already renders these as the distinct trader dot (#64). **No protobuf / wire / frontend change.** (Optionally a sub-variant key like `trader:flow:<id>` for a future visual distinction — keep the `trader:` prefix so the client path is unchanged.)
- On `progress ≥ 1.0`: drop the shipment from `FlowShipments`; its trader follows the existing leaving→despawn path (clean `left_agents`).

`MaterializedTraders` is extended to track shipment-traders alongside demo-traders (same struct, keyed by the shipment-namespaced actor id). `extract_from_world` already excludes `TraderAgent` from mobility persistence (#64) — shipment-traders inherit that (they are projections).

## 5. Conservation, determinism, persistence

- **Conservation: trivial.** Shipment-traders carry no `AccountBook`/`InventoryBook`/order state. The goods + cash moved in the macro-flow settlement (already in the ledger, #69). Capturing/rendering/expiring a shipment touches **no economic resource** → goods + cash totals are provably unaffected. A test asserts `total_money`/`total_good` are identical with and without the FlowShipments+materialize path running.
- **Determinism:** `FlowShipments` (BTreeMap) + `NextShipmentId` (monotone counter) + deterministic capture order (the macro flow's already-deterministic edge order) → replay-stable. No RNG/float in capture; the render position uses the same deterministic arc-length math as the demo trader.
- **Persistence: ephemeral, NOT persisted** (consistent with the frozen-time model + #64's TraderAgent exclusion). On restart, in-transit *visuals* are lost; the macro flow regenerates new shipments from the resumed economy. `EconomyPersistSnapshot` is unchanged — no new persisted field, no migration. (A shipment is a transient view, not economic state; persisting it would be cosmetic and would risk replaying a stale visual against a resumed flow.)

## 6. Wire / render path (unchanged)

Shipment-traders are ordinary `TraderAgent`s fed through the per-tick mobility delta (`DirtyAgents` → `MobilityChunkDelta.changed_agents`), exactly like #64's demo trader. The client already projects `trader:`-prefixed `sprite_key` agents as the distinct trader marker (`src/render/`). **Zero protobuf, WS, or frontend change.** The client interpolates between deltas as for any agent.

## 7. Demonstrability + seed

A flow-trader is visible only when (a) both endpoint markets are **dormant** and (b) their footway route crosses an **observed** chunk. This lives on a knife's edge (the review's make-or-break finding) and is specified concretely here — treating it as a footnote risks shipping with **zero** flow-traders ever visible (the Phase-7a failure mode CLAUDE.md warns about). The enabling + constraining facts, verified against the code:

- **The walkable footway is a grass grid over the whole 224×128 world** (`terrain.tiles` empty → all `Grass`; base_world.rs:392,422; mobility/seed.rs grass_tile_walks). So two markets can be placed far apart on a **shared row** with a predictable straight-line Walk route between them.
- **A subscribed chunk becomes Active** (world/systems.rs:317), and an Active chunk's market is **not dormant** (systems.rs:106-118) → it would not flow. The client subscribes a **filled viewport rectangle + a `margin=1` ring** (viewportChunks.ts:36-39; mobilityClient.ts margin 1) — minimum footprint **3×3 chunks**. **Therefore both markets must sit ≥2 chunks from the transit (observed) chunk in every direction.** Layout: market A @ chunk `(c_lo, r)`, market B @ chunk `(c_hi, r)`, observe chunk `(c_mid, r)` on the same row with `c_lo, c_hi` ≥2 chunks from `c_mid`. The straight-line grass route A↔B crosses `(c_mid, r)`.
- **Avoid the pinned chunk `(3,2)`** — the south sidewalk corridor (300 pedestrians) is pinned permanently Active (runtime/mod.rs:108-122 → chunk (3,2)); neither market nor the transit chunk may be (3,2), or the market never flows / the trader shows regardless of observation.
- **Standing imbalance** (supply@A, demand@B for the good — reuse/extend the Slice-1 second-good pools) → a recurring `MacroFlow` every `macro_flow_interval_ticks` (=10).

**Browser-smoke (`smoke-flow-traders.mjs`) — the existing template's strategy is INVERTED and must be rewritten:** `smoke-visible-traders.mjs` zooms **OUT** to subscribe the whole world — which would make the market chunks Active and kill the flow. The new smoke must zoom **IN** (toward CAMERA_MAX_SCALE) + pan so that **only the transit-chunk neighborhood is subscribed** (assert via the client's own subscription set that the market chunks are NOT subscribed → stay dormant). Then assert a `trader:` agent appears in the transit chunk and its `world_coord` advances along the route. Size the observation window for the shipment lifetime: markets ~6 chunks apart (~192 tiles) at `trader_tiles_per_tick=4` → `travel_ticks ≈ 48`, flow fires every 10 ticks → tolerate shipments entering/leaving the observed window; assert over a long-enough span.

## 8. Testing

**Unit (sim-core):**
- `macro_flow_emits_one_shipment_per_cross_edge` — a dormant→dormant flow inserts exactly one `FlowShipment` (correct from/to/good/qty/start_tick); a self-edge inserts none.
- `shipment_travel_ticks_from_distance` — `travel_ticks` derived from `MarketDistances`, `> 0`.
- `shipment_trader_materializes_on_observed_route` — a shipment whose route point at the current progress is in an observed chunk produces a Spawn/Update; outside observed → Despawn (reuse the #66 lifecycle assertions).
- `shipment_expires_on_arrival` — at `progress ≥ 1.0` the shipment is dropped and the trader despawned (ghost-free `left_agents` then despawn).
- `flow_shipments_do_not_touch_economy` — `total_money`/`total_good` identical with vs without the FlowShipments+materialize path (conservation-trivial). **Extend** the existing `tests/materialize.rs:170 materialize_does_not_touch_money_or_goods` rather than duplicate it.
- `flow_shipments_are_deterministic` — `build() == build()` on the shipment map + materialized positions.
- `flow_shipments_not_persisted` — `extract_from_world`→`apply_into_world` round-trip leaves no shipment state (snapshot byte-stable, unchanged).

**Browser-smoke (mandatory, frontend boundary):** adapt `scripts/smoke-visible-traders.mjs` → `smoke-flow-traders.mjs`: launch the dev stack, subscribe to the transit chunk, assert a `trader:` agent appears on the dormant→dormant route and its `world_coord` changes over time. The e2e `render-smoke` needs **no change**: it already filters `!a.id.startsWith('trader:')` for the pinned-300 assertion + uses `count >= 300` (render-smoke.spec.ts:96,99), so flow-traders (reusing the `trader:` prefix) are auto-excluded and only bump the `>=` count — verify it stays green, no new filtering work.

## 9. Scope & deferred

**Slice 2 IS:** (a) `FlowShipments` + `NextShipmentId` capture from `MacroFlow` edges; (b) materialize transit traders through the existing ghost-free lifecycle; (c) arrival-expiry; (d) ephemeral (no persistence); (e) a seed arrangement + browser-smoke proving a flow-derived trader is visible and moving; (f) zero economic/wire change.

**Deferred:**
- Unifying the observed-market economic `Trader` (#64) into the flow-derived model (requires the deferred auction↔flow coupling so observed markets also participate in inter-market flow — then traders arrive/depart at the market you're directly observing).
- Consumer trips ("Leute gehen zum Trader bei Bedarf") — a separate micro layer coupling mobility activity to economic need.
- Carrying the shipment `good`/`qty` into the sprite/label for a richer visual; per-good trader variants.
- Spatial pruning / scale of `FlowShipments` (Slice 3 territory if many simultaneous flows).

## 10. To resolve during planning (against real code)

1. The exact `travel_ticks` formula (reuse `trader_travel`'s distance→ticks magnitude so flow-traders walk at the same visible speed as the demo trader).
2. The reserved `EconomicActorId` namespace for shipment-traders (a high offset that cannot collide with seeded demo-trader / pool actor ids).
3. The exact demo seed geometry (market coordinates + the transit chunk) against the real base-world footway graph + the smoke's subscription, so a dormant→dormant flow's route reliably crosses the observed chunk.
4. Confirm the `materialize_traders_system` signature extension (adding `Res<FlowShipments>`) keeps it a borrow-clean exclusive `fn(&mut World)` (it already reads multiple resources + the graph).

## References

Visible traders are viewport-bounded projections of the macro flow (see the
macro-demand-flow design's references for the mean-field closure); rendering
micro-agents as projections of aggregate dynamics follows the
agent-based-modeling tradition of Epstein & Axtell.

- Epstein, J. M., & Axtell, R. (1996). *Growing artificial societies: Social
  science from the bottom up*. Brookings Institution Press / MIT Press.
- Lasry, J.-M., & Lions, P.-L. (2007). Mean field games. *Japanese Journal of
  Mathematics, 2*(1), 229–260.
