# Economy Slice 3 — Visible shoppers (the demand side's people)

Date: 2026-06-01

## Status

Approved in brainstorming. The **demand-side micro layer**, twin of the supply-side flow-traders (#70): when an **observed** market has unmet demand, people are seen **walking to it** — realizing the user's original vision, *"leute gehen zum trader wenn needs da sind"*. The economy (macro flow #69 + auction) remains the sole authority; shoppers are a **conservation-exact, read-side projection of aggregate demand**, exactly as flow-traders project the macro flow.

**Decision: dedicated ephemeral shopper-agents** (not a mutation of base-world walkers). Per observed market with `unmet_demand_last_tick > 0`, a deterministic, sampled set of shopper render-agents (count ∝ unmet demand, capped) walk from a nearby footway point **to** the market, then despawn on arrival. **No economic effect** (demand still clears aggregately), ephemeral (not persisted), observed-only. They render as ordinary **pedestrians** (a `shopper:` sprite-key → the client's `isTraderSpriteKey` else-branch already maps non-`trader:` keys to `kind:'pedestrian'`, so **no client render change**); the only frontend touch is excluding `shopper:` ids from the pinned base-world pedestrian count (mirrors #64's `trader:` exclusion). Visible effect: **busy markets attract foot traffic proportional to demand.**

**Explicitly NOT in scope** (deferred): hijacking base-world walkers' persisted `WalkPlan`s (faithful but mutates persisted plans in the perf-critical path — a later, heavier slice); making the visit economically real (the shopper actually drawing from the demand pool — couples conservation); demand-responsive intensity tuning beyond a simple proportional cap.

**Crosses the frontend↔backend boundary** (new agents on the WS mobility stream + the count-exclusion) → **browser-smoke is mandatory** (CLAUDE.md).

## 1. Goal

Make aggregate demand *visible as people*: an observed market with unmet demand has shopper-agents walking toward it, count proportional to the shortfall. No economic effect, no new client render kind, conservation-trivial, ephemeral, demonstrable via browser-smoke. The exact mirror of #70 on the demand side.

## 2. Architecture overview

Three additions in `sim-core` + a tiny render-smoke/diagnostic exclusion; the render/wire path is reused from #64/#70:

1. **Capture** — a new system reads **observed** markets' `MarketGoodState.unmet_demand_last_tick` and records, per (market, good) with shortfall, a deterministic sampled set of **shopper visits** in a render-only `ShopperVisits` resource: `{ id, market, good, origin_node, start_tick, travel_ticks }`. Count = `min(unmet_demand / SHOPPERS_PER_UNIT, MAX_SHOPPERS_PER_MARKET)`, deterministic (no RNG). `origin_node` is a deterministic nearby footway node (so the shopper walks *in* from the neighborhood). Existing visits for a market are reconciled each capture epoch (top up / let arrived ones expire) rather than re-spawned.
2. **Render** — the existing `materialize_traders_system` (extended in #70 to a parameterized `plan_render_mutations`) ALSO materializes one pedestrian-styled `TraderAgent` per active shopper visit, at `progress = (current_tick − start_tick) / travel_ticks` along the `origin_node → market_node` footway route (reusing `leg_polyline` + `world_coord_at_progress_slice`), driven through the same ghost-free Spawn/Update/Despawn lifecycle, observed-only.
3. **Expire on arrival** — at `progress ≥ 1.0` the visit is dropped and its shopper despawned (ghost-free).

`ShopperVisits` is a derived, ephemeral projection (not persisted), exactly like #70's `FlowShipments`. The economy is untouched.

## 3. `ShopperVisits` capture

New resource (render-only) in `economy/`:

```rust
pub const SHOPPER_ACTOR_OFFSET: u64 = 2 << 32;   // distinct from flow-traders' 1<<32

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShopperVisit {
    pub id: u64,
    pub market: MarketId,
    pub good: GoodId,
    pub origin_node: crate::routing::NodeId,
    pub start_tick: u64,
    pub travel_ticks: u64,   // > 0
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ShopperVisits(pub BTreeMap<u64, ShopperVisit>);   // keyed by deterministic id

#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NextShopperId(pub u64);   // EPHEMERAL — NOT persisted (like #70's NextShipmentId)
```

**Capture system** (`run_shopper_capture_system`) — in a NEW `EconomySet::ShopperCapture` ordered **after `MacroFlow`, before `Materialize`** (corrected after review). It must run BEFORE `Materialize` because `materialize_traders_system` reads `ShopperVisits` to render them same-tick (mirroring how it reads `FlowShipments`); placing it in/after Telemetry would render every shopper one tick late + blank the first tick. `unmet_demand_last_tick` is already current after `ClearMarkets`/the auction (`update_market_telemetry` only folds the EWMA — it does NOT set `unmet_demand`), so waiting for Telemetry buys nothing. Observed-only; needs the observed-chunk set (like `materialize_traders_system`), `Markets`, `MarketGoods`, `NodeSpatialIndex`/`Graph`. For each **observed** market-good with `unmet_demand_last_tick > 0`:
- `target = min((unmet_demand / config.shoppers_per_unit).max(0), config.max_shoppers_per_market)`.
- Reconcile: count this market's current visits in `ShopperVisits`; if fewer than `target`, spawn `(target − current)` new visits with deterministic `origin_node`, `start_tick = current_tick`, `travel_ticks = max(1, manhattan_tiles(origin,market)/walk_speed)`. New ids from `NextShopperId`.
- **`origin_node` selection (corrected after review):** `NodeSpatialIndex::within_radius` returns an **UNSORTED** `Vec<NodeId>` (rstar tree order) over ALL nodes — so the candidates MUST be sorted (by `NodeId`, or distance-then-`NodeId`) before taking the Nth, or determinism breaks. EXCLUDE the market node itself (else `leg_polyline` short-circuits to a zero-length walk → shopper spawns AT the market and instantly "arrives"), and SKIP candidates with no Walk route (`leg_polyline` returns `None`). If too few valid candidates, spawn fewer than `target`.

Determinism: `BTreeMap` + monotone counter + deterministic origin selection (sorted spatial neighbors) + the already-deterministic market/good iteration. **Observed-only** so off-screen demand (the bulk) costs nothing — the visits are a viewport projection.

## 4. Render: materialize shoppers

Extend `materialize_traders_system` (already parameterized in #70). In the same cache `resource_scope`, after building demo-trader + flow-shipment render-actors, ALSO build shopper render-actors from `ShopperVisits`:
- route = `leg_polyline(graph, hpa, &mut cache, visit.origin_node, market.node_id)`; skip if no route.
- `actor = EconomicActorId(SHOPPER_ACTOR_OFFSET + visit.id)`; `progress = visit.progress(tick)` (linear, clamped).
- sprite key uses a **`shopper:` prefix** (so the client renders `kind:'pedestrian'` via the existing `isTraderSpriteKey` else-branch — no client render change). The Spawn arm builds the agent id/sprite from this prefix; the Despawn arm reconstructs the same id.
- Feed into the same `plan_render_mutations` lifecycle (ghost-free observed Spawn/Update/Despawn).
After `apply_mutations`, drop arrived visits: `expire_arrived_shoppers(&mut shopper_visits, tick)`.

**Prefix reconstruction lives in TWO places (corrected after review)** — the **Spawn** side builds `agent_id`/`sprite` inside `plan_render_mutations` (materialize.rs:189-190, `format!("trader:{}", …)`), and the **Despawn** side rebuilds it in `apply_mutations` (materialize.rs:362). BOTH must switch on the actor-id namespace via one shared helper `fn id_prefix(actor) -> &'static str` (`>= SHOPPER_ACTOR_OFFSET` → `"shopper:"`, else `"trader:"`) and stay identical — updating only `apply_mutations` would spawn `trader:`-prefixed shoppers (wrong kind + double-counted). **Also:** the #70 shipment-id filter `rendering_shipment_ids` (materialize.rs:373-380, `checked_sub(SHIPMENT_ACTOR_OFFSET)`) must additionally **exclude ids `>= SHOPPER_ACTOR_OFFSET`** — otherwise a shopper id `2<<32 + k` yields `checked_sub(1<<32) = 1<<32 + k` and is mis-attributed as a shipment.

## 5. Conservation, determinism, persistence

- **Conservation: trivial.** Shopper capture/render/expiry touch no `AccountBook`/`InventoryBook`/`MarketGoods` write path (they READ `unmet_demand_last_tick` only). The demand still clears aggregately in the economy. A test asserts `total_money`/`total_good` unchanged with vs without the shopper path.
- **Determinism:** `ShopperVisits` (BTreeMap) + `NextShopperId` (monotone) + deterministic origin selection + deterministic market iteration; no RNG/float beyond the existing arc-length positioning. Replay-stable within a run.
- **Persistence: ephemeral, NOT persisted** (like #70). `EconomyPersistSnapshot` unchanged; `NextShopperId` resets to 0 on restore alongside empty `ShopperVisits`. On restart, visits regenerate from the resumed demand.

## 6. Wire / render path

Shoppers are `TraderAgent` render entities (the #64/#70 path) with a `shopper:` sprite key → the client projects them as `kind:'pedestrian'` (backendMobilityDrawables.ts:97 else-branch — **no client render change**). Per-tick mobility delta as usual. **Frontend touch = ONE exclusion:** `shopper:` ids must be excluded from the pinned base-world pedestrian count, exactly where #64 excludes `trader:`:
- `tests/e2e/render-smoke.spec.ts` — extend the `!a.id.startsWith('trader:')` filter (and the `count >= 300`) to also exclude `shopper:`.
- `src/app/runtimeDiagnostics.ts` — if the `pedestrians` count must stay base-world-only, exclude `shopper:` ids there too (confirm against the current diagnostic; #64 set `pedestrians` to count `kind==='pedestrian'` — shoppers are `kind:'pedestrian'`, so the diagnostic count needs the id-prefix exclusion).

No protobuf change (sprite_key is already a string field).

## 7. Demonstrability + seed

The mirror of #70's constraint: a shopper is visible only when its market is **observed** AND has standing `unmet_demand_last_tick`. The review CONFIRMED this already holds in the demo with **no seed change**: the existing **FOOD demand pool at `m_b = MarketId(9_002)`** (REF_B, tile (13,3) → chunk (0,0); consumer 8_012, 10/tick, `seed.rs:162-175`) has **no FOOD supply at `m_b`** and the demo trader 8_003 carries TOOLS only — so the FOOD bid dirties `(m_b, FOOD)` every tick, the auction clears it with no asks, and `unmet_demand_last_tick ≈ 10` **persistently every tick** (auction.rs:208-216,364-372). Chunk (0,0) is observable (the render-smoke already sees the demo trader there). So:
- **Key the shopper demo on FOOD-at-`m_b`** (zero local supply → reliable standing unmet demand). No new seed needed; if a config threshold (`shoppers_per_unit`) would suppress a 10-unit shortfall, tune the default so 10 unmet → a small visible count. (Do NOT key on TOOLS-at-`m_b` — the trader intermittently supplies it.)
- **Browser-smoke `smoke-shoppers.mjs`** must actively pull chunk (0,0) into the subscription (zoom out / pan, mirroring `smoke-visible-traders.mjs:75-80` which zooms out "regardless of the default camera center") — do NOT assume the default viewport observes `m_b`. Then assert `shopper:`-prefixed agents appear and their `world_coord` advances **toward** the `m_b` node, count > 0.

## 8. Testing

**Unit (sim-core):**
- `shopper_capture_spawns_proportional_to_unmet_demand` — observed market with `unmet_demand=N` spawns `min(N/per_unit, cap)` visits; zero unmet demand → none; dormant market → none.
- `shopper_capture_is_deterministic` — `build()==build()` on `ShopperVisits` + `NextShopperId`; deterministic origin nodes.
- `shopper_travel_ticks_positive` + `shopper_progress_and_arrival`.
- `shopper_materializes_on_observed_route` + `shopper_expires_on_arrival` (reuse the #70 lifecycle assertions; shopper actor-id namespace).
- `shoppers_do_not_touch_economy` — extend `materialize_does_not_touch_money_or_goods`: `total_money`/`total_good` unchanged with active `ShopperVisits`.
- `shoppers_not_persisted` — snapshot round-trip leaves no shopper state.

**Browser-smoke (mandatory):** `scripts/smoke-shoppers.mjs` — launch the in-memory `e2e_server` (fresh seed, no prod DB — same as #70) + vite; subscribe to the observed demand market's chunk; assert ≥1 `shopper:` agent appears and moves toward the market; no console errors. Also run e2e `render-smoke` to confirm the pinned-300 count holds after excluding `shopper:`.

## 9. Scope & deferred

**Slice 3 IS:** (a) `ShopperVisits`/`NextShopperId` capture from observed markets' unmet demand (sampled ∝ shortfall, deterministic); (b) materialize shoppers as pedestrian-styled render-agents via the #70 lifecycle; (c) arrival-expiry; (d) ephemeral (no persistence); (e) the `shopper:` count-exclusion (render-smoke + diagnostic); (f) seed an observed market with standing unmet demand + a browser-smoke proving shoppers are visible and moving toward it; (g) zero economic effect.

**Deferred:**
- Hijacking base-world walkers' persisted plans so *existing* people run market errands (the richer, persisted-plan version).
- Economically-real visits (the shopper draws from the demand pool on arrival).
- Round-trip (dwell at market + return home) — Slice 3 is one-way (walk in, despawn on arrival); a return leg is additive later.
- Per-good shopper visuals; demand-responsive intensity curves; off-screen aggregate "foot traffic" stats.

## 10. To resolve during planning (against real code)

1. The shared `id_prefix(actor)` helper (§4) wired into BOTH `plan_render_mutations` (Spawn, materialize.rs:189-190) AND `apply_mutations` (Despawn, materialize.rs:362), plus the `rendering_shipment_ids` exclusion of `>= SHOPPER_ACTOR_OFFSET` (materialize.rs:373-380).
2. The exact `NodeSpatialIndex::within_radius` call + the SORT (it returns unsorted), market-node exclusion, and routeless-candidate skip (§3); the radius + `shoppers_per_unit` magnitudes so a ~10-unit shortfall yields a small visible count.
3. `config.shoppers_per_unit` + `config.max_shoppers_per_market` defaults (small — a handful of visible shoppers per busy market, not hundreds), added to `EconomyConfig` (ephemeral tuning, not persisted).
4. Both `shopper:` count-exclusions are required (confirmed): `runtimeDiagnostics.ts:147` (`pedestrians` filter — shoppers are `kind:'pedestrian'`, a NEW exclusion vs #64's `trader:` which is `kind:'trader'`) AND `render-smoke.spec.ts:96` (the `mobilityAgents.agents` `!startsWith('trader:')` filter). Confirm the `shopper:<hash>` key form parses to a valid pedestrian sprite via `spriteIndexFromKey`.
5. Wire the new `EconomySet::ShopperCapture` **after `MacroFlow`, before `Materialize`** in the `.chain()` (NOT Telemetry — §3); confirm `run_shopper_capture_system` stays borrow-clean alongside `materialize_traders_system`.
