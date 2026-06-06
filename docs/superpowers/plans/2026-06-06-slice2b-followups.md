# Slice 2b Follow-Ups — Making the Per-Capita Ramp Visibly Alive

Date: 2026-06-06
Status: Open follow-ups, filed when Slice 2b shipped on the strength of the
backend density+safety test (live "looks alive" validation was blocked).

## Context

Slice 2b authored `capita_baseline` from `markets.json` (serde-default identity),
ramped abutopia to `capita_baseline = 10` (→ factor ~30 at ~300 citizens), and
added a backend density+safety test proving the ramped cohort is materially
larger (120 vs 4 routed in a fully-wired harness), audit byte-invariant,
price-stable, and solvent. During Slice 2b a genuine bug was also found and
fixed: `capita_baseline` reverted to the identity default on every restart
(`EconomyConfig` is rebuilt from defaults each boot and is not in the snapshot,
and the layer write sat behind the idempotent state-seed guard) — fixed by
re-applying authored config ahead of the guard, with a regression test
(`063611c`).

A controller-run live validation against the running abutopia dev stack revealed
that **Slices 1+2b are mechanically correct but currently invisible in the
actual abutopia world**: no citizen is ever economically attributed, so the
per-capita density does not appear on the map regardless of the factor. Three
independent blockers cause this — none is a per-capita-ramp defect. They are
recorded here as follow-ups.

## Blocker 1 — Structural binding mismatch (citizens never attribute)

**Symptom:** `economy::liveness` reports `routed = 0` every tick in the live
abutopia world.

**Root cause:** The 300 abutopia pedestrians spawn on the south sidewalk
corridor (`data/worlds/abutopia/layers/spawns.json` →
`corridor:sidewalk:south`, tiles ≈ x∈[106,117], y≈64.5, chunk (3,2)). The
binding rule (`mobility/market_binding.rs::assign_binding`) assigns
`home_market` = nearest market, `work_market` = second-nearest, by Euclidean
distance to the markets' snapped node positions. For the sidewalk-south
pedestrians the nearest two are markets **9003** (Flow Demo A) and **9004**
(Flow Demo B). But:
- Market 9003 receives **no final consumption** (it is a supply market) → the
  shop channel (keyed on `home_market`) attributes nobody.
- Market 9004 receives consumption (demand actor 8022, good 1) but **no wages**
  (wages are paid by firms, which sell at 9001/9003, not at the demand market
  9004) → the wage channel (keyed on `work_market`) attributes nobody.
- Market 9002 (Demo B) is the consumption market for goods 4 and 1, but **no
  pedestrian binds to it** (it is far from the sidewalk corridor).

So no observed market has both realized activity (consumption or wages) **and**
bound citizens. Attribution is structurally empty for this world layout.

**Fix options (world-data, pick one):**
1. **Re-anchor a consumption market onto the pedestrian corridor.** Move market
   9002's anchor in `markets.json` to ≈ (112, 64.5) (chunk (3,2)). Then all 300
   pedestrians bind `home_market = 9002`, and viewing the corridor observes a
   market with consumption → the shop channel routes a cohort scaled by the
   factor. Changes the 9001↔9002 macro-flow geometry (longer transport leg;
   mean-field, so only transport cost changes).
2. **Relocate the pedestrian spawn corridor** near an existing demand market
   (9002 @ chunk (0,0) or 9004 @ chunk (6,1)).
3. **Add a demand (consumption) market on the corridor** rather than moving an
   existing one — keeps the demo pairs intact.

**Acceptance:** with the stack up and the corridor in view, `economy::liveness`
`routed` climbs well above the identity baseline (expect ≈ `max_shoppers_per_market
× factor` bounded by candidates), and citizens visibly head to / cluster at the
market. Re-run the controller live-validation (screenshot + routed count) and
tune `capita_baseline` with the user.

## Blocker 2 — Degenerate long-running economy (consumption collapsed)

**Symptom:** `/economy` shows `consumed_qty_last_tick = 0` at every market while
macro-flow `traded_qty_last_tick` is large; demand markets 9002/9004 have
`ewma_reference_price ≈ 99996` (pinned at the `price_ceiling` of 100_000) while
supply markets 9001/9003 sit near the floor (≈ 95).

**Root cause (hypothesis):** Over a very long-running dev world the free-price
tâtonnement (#77) drove the demand-market prices to the ceiling; with consumer
`max_price = 2000 << 99996` the demand pools can no longer transact → final
consumption is zero. This is a pre-existing artifact from `origin/main`, not
Slice 2b. A fresh seed resets opening prices to 1000 (where consumers can buy).

**Follow-up:** investigate whether this is an expected long-run artifact or a
real divergence in the free-price mechanism / this world's parameters (e.g. the
demand-market price ceiling vs. consumer `max_price`, or transport-cost feedback
pushing demand-market prices up unbounded). At minimum, a fresh-world seed is
required before any live validation; ideally understand why the equilibrium
diverges to the ceiling. NOTE: a fresh seed means `DELETE FROM economy_snapshots`
+ `DELETE FROM mobility_snapshots` for the abutopia world on the shared remote DB.

## Blocker 3 — Persistence-stale health gate blocks rendering

**Symptom:** the frontend shows "Backend required — Backend health not OK:
persistence stale" and refuses to render the world, even though the backend is
up and the sim is ticking.

**Root cause (hypothesis):** snapshot writes to the **remote Supabase** Postgres
take 1–2.8s each (`sqlx` slow-statement warnings on every
`INSERT INTO mobility_snapshots`), and the persistence-liveness health gate
(active WIP on the `plan/persistence-liveness` branch) flips to "stale" when the
last successful persist lags the current tick beyond its threshold. Pre-existing
infra / persistence-liveness concern, unrelated to the economy.

**Follow-up:** either (a) relax/lengthen the staleness threshold to tolerate slow
remote-DB latency for local dev, (b) point local dev at a faster (local) Postgres,
or (c) allow the client to render in a degraded/observe-only mode when
persistence is stale. Coordinate with the `plan/persistence-liveness` work.

## Deferred (unchanged from the Slice 2b spec)

- Distinct economic-citizen sprites (Slice 1b) — would make routed citizens
  client-legible once Blocker 1 is fixed.
- Live tuning of `capita_baseline` with the user (the spec's Task 4 step 4) —
  blocked until Blockers 1–3 let the density show; the shipped value is the
  proven-safe start candidate `10`.
