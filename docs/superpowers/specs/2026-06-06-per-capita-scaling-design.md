# Per-Capita Economic Scaling + Visible Density — Design (Slice 2)

Date: 2026-06-06

## Status

Approved direction from brainstorm. Slice 2 of the demographics↔economy merge.
Slice 1 made aging citizens the economy's visible bodies but is deliberately
**"correct but sparse"**: at demo-scale macro aggregates only a handful of trips
per tick are justified, and the attribution caps are *absolute* (≤4/market), so
most of the ~300 citizens stay on their geometric routine. Slice 2 makes the
economy's throughput and the visible economic density **track the live citizen
count**, so the city stops looking idle.

Settled decisions (brainstorm):

1. **Goal = realistic macro scale AND visible density** (not macro-only). This
   requires scaling both the economic *magnitude* and the *visible attribution
   cohort*, because Slice 1 deliberately made magnitude orthogonal to body count.
2. **Scale signal = the live aging-citizen count** (`AgentIdIndex.len()`, ~300,
   grows/shrinks via births/deaths), not the abstract `HouseholdSector.population`
   and not a fixed multiplier. Density genuinely tracks the simulated bodies, and
   the factor stays modest (~10–30×, not ~1000×) → far lower overflow/stability
   risk.
3. **Magnitude mechanism = hybrid, audit-safe** (below): scale real-quantity
   flows + `opening_cash` symmetrically; wages stay labor-share of the larger
   revenue (no artificial wage multiply).
4. **Cohort caps go from absolute-4 to population-aware** (a deliberate revision
   of Slice 1's absolute caps), preserving viewport-independence.
5. **Delivered as audited sub-slices 2a–2e**, default factor at identity (1)
   until the end, with the `#78` conservation audit as the canary throughout.

**Base branch.** `feat/per-capita-scaling` off `origin/main` (`d79f85c`, Slice 1
merged).

## Goal

Wire a single `capita_factor` (derived from the live citizen count) into the
economy so that (a) wage/consumption **magnitude** tracks the population, and
(b) the **visible attribution cohort** grows with it, so most bound, observed
citizens with realized demand are visibly shopping/commuting — while keeping the
`#78` per-tick money-conservation audit byte-invariant and all i64 arithmetic
overflow-safe.

## Background — verified current state (`d79f85c`)

- **`HouseholdSector.population`** (`u64`, `wages.rs:49`) is seeded from
  `markets.json household.population = 1_000_000` (`markets_layer.rs:218`),
  persisted (`economy/persist.rs`), and **inert** — read by zero economic-math
  functions. Doc: "reserved for the per-capita consumption-scaling slice and is
  intentionally inert until then" (`wages.rs:43-46`).
- **Today's money-flow scale is set by `markets.json` quantities, not
  population:** 3 supply + 3 extractor pools at `qty=10`/tick, 3 demand pools at
  `qty=10`/tick, prices 500–2000, `ECONOMY_SCALE=1000` (`money.rs:1`). Total
  baseline demand ≈ 30 units/tick (abstract aggregates).
- **Money is minted once at seed** via `opening_cash` deposits (3 demand actors ×
  1_000_000, `markets_layer.rs:133`); `opening_inventory` is goods, not money.
  Every runtime move is a double-entry `AccountBook::transfer`, so `Σ(available +
  locked)` is byte-constant. `run_tick_audit_at_tick` (`audit.rs:26`) re-sums
  `total_money()` each tick and returns `ConservationViolation` on drift;
  `HOUSEHOLD_SECTOR` (`u64::MAX-1`) and `TRANSPORT_OPERATOR` (`u64::MAX`) are
  net-zero sentinels asserted each tick (`wages.rs:147,232`). **The audit is
  magnitude-indifferent** — valid under any scaling *as long as scaling is done
  via transfers/recomputed flows and never a runtime mint*.
- **Demand is Keynesian:** `C = autonomous + mpc_bps·income/10_000`
  (`pools.rs target_spend`), `autonomous=5000`, `mpc_bps=8000`;
  `desired_qty = spend_to_qty(C, ewma_ref_price)`. Wages:
  `wage = floor(revenue·labor_share_bps/10_000)`, `labor_share_bps=6000`;
  dividend `dividend_share_bps=10000` (firms net to zero).
- **Overflow:** all hot paths use i128 intermediates + checked ops + `i64::try_from
  → EconomyError::Overflow` (`wage_for_revenue`, `target_spend`, `spend_to_qty`,
  `affordable_qty`, `checked_order_value`). Headroom ≈ `i64::MAX/SCALE ≈ 9.2e15`
  for qty/cash. An existing test drives `revenue = i64::MAX/2` at `population=1M`
  and passes — but population is a no-op there, so **per-capita multiplication is
  untested**.
- **Live count** = `world.resource::<AgentIdIndex>().0.len()`
  (`mobility/resources.rs:95`), ~300 (`spawns.json agents_per_corridor=300`),
  grows/shrinks in `population_monthly_system`. Rendered bodies carry **no money
  accounts** → live count and money stock are independent today; reading the
  count from the economy would be the **first economy→mobility money-math
  dependency**.
- **Attribution cohort** (Slice 1, `attribution.rs`):
  `count = min(realized/per_unit, cap, candidates)` with `shoppers_per_unit=3`,
  `max_shoppers_per_market=4`, `commuters_per_wage_unit=100`,
  `max_commuters_per_market=4` (`systems.rs:148-153`). Today the *binding*
  constraint is usually `realized/per_unit` (≈3), not the cap.

## Non-Goals

- Per-citizen microfoundation (each citizen with its own account/budget) — the
  macro stays authoritative (Slice 1 decision unchanged).
- A dynamic labor market, per-citizen wages, or elasticity-shaped demand.
- New markets / map content (density comes from scaling the cohort at existing
  markets, not adding markets).
- Driving the factor by the abstract `1M` or a fixed multiplier (rejected).
- Frontend changes beyond what the existing agent stream already carries.

## Architecture

A single derived `capita_factor` flows into three places, all preserving the
mint-once-at-seed money model:

1. **Seed-time money + supply/demand stock** scale by the factor (so scaled
   demand is fundable and supply can meet it).
2. **Per-tick real-quantity flows** (consumption `desired_qty`/`autonomous`,
   supply `offered_qty`) scale by the factor (throughput tracks population).
3. **Attribution cohort caps** become population-aware (visible density).

Wages are *never* multiplied directly — they remain `labor_share·revenue`, and
revenue grows because quantities grow. This keeps `wage ≤ revenue` and the
`HOUSEHOLD_SECTOR` net-zero sentinel intact.

## Component — the `capita_factor`

- A resource `CapitaFactor(pub i64)` (or fixed-point), **derived**, not persisted.
- **Driver:** the live count `AgentIdIndex.len()` relative to a baseline (the
  seed count the demo quantities represent — `capita_baseline`, a config, default
  the seed population so `factor=1` at seed).
- **Cadence:** recomputed at the **monthly boundary** (in/after
  `population_monthly_system`, where the count changes), snapshotted into
  `CapitaFactor`, and read by the economy systems within the month. Deterministic;
  no per-tick cross-module churn.
- **Identity default:** while the knob/baseline make `factor = 1`, every formula
  below is byte-identical to today (the sub-slice 2a guarantee).

## Component — magnitude scaling (hybrid)

- **Demand (`pools.rs`):** `autonomous` and the generated `desired_qty` scale by
  `capita_factor` (scale `target_spend` output before `spend_to_qty`, and the
  autonomous floor). Checked arithmetic throughout.
- **Supply (`pools.rs` / generation):** `offered_qty` per supply/extractor pool
  scales by `capita_factor` (symmetric — so revenue and thus wages grow to fund
  the scaled demand, and excess-demand price blowup is avoided).
- **Seed money + inventory:** `opening_cash` (and `opening_inventory`) scale by
  the factor at seed. This is the **only** legitimate way to fund scaled demand —
  money is still minted once, only more of it, at seed. ⚠️ Requires a one-time
  **`DELETE FROM economy_snapshots`** before deploy (new seed = new money stock;
  an old snapshot would restore the un-scaled stock).
- **Wages/dividends unchanged in form:** still `labor_share·revenue` and
  full-profit dividend → firms net to zero, `wage ≤ revenue` holds, sentinels
  stay net-zero. The audit's per-tick byte-invariance is **unchanged**.

## Component — visible density (population-aware cohort cap)

Replace the absolute `max_shoppers_per_market`/`max_commuters_per_market = 4`
with a **population-aware** bound so the cohort can grow with the citizenry:

- The attribution cohort stays `min(realized/per_unit, pop_cap, candidates)`.
- `pop_cap` scales with `capita_factor` (e.g. a per-market share of the live
  count), so it no longer artificially clamps at 4.
- `per_unit` (`shoppers_per_unit`) and the per-citizen consumption rate are
  calibrated so `realized/per_unit ≈` the count of citizens bound to a market —
  i.e. each attributed citizen represents ~one citizen's worth of realized
  activity. Effect: **practically every bound, observed citizen with realized
  demand is visibly attributed** → density.
- **Viewport-independence preserved:** `pop_cap` depends on the *population*, not
  on observation. Only *which* bound citizens are depicted depends on the viewport
  (the `candidates` term, exactly as in Slice 1). The macro/correctness still does
  not depend on what is observed.

## Overflow safety

- Live count ~300 → factor ~10–30× → flows stay far below the ~9.2e15 ceiling.
- Every scaled multiply uses the existing checked/i128 path and returns
  `EconomyError::Overflow` (fail-fast), never silent wrap.
- Add the **missing per-capita overflow stress test** (sub-slice 2e): drive
  revenue toward `i64::MAX/2` *with* `capita_factor` and a firm count of 10–100,
  across multiple ticks (to stress `wage_bill` and `traded_qty_last_tick`
  accumulators), asserting `Overflow` is returned at the ceiling and conservation
  holds below it.

## Persistence & migration

- `CapitaFactor` is **derived** (from the live count each month), not persisted —
  on restore it re-derives from the rehydrated count.
- Scaling `opening_cash`/`opening_inventory` changes the seeded money/goods stock
  → a one-time **`DELETE FROM economy_snapshots`** before deploying past sub-slice
  2b (consistent with the established economy-deploy discipline). No serde-default
  shim, no heal-on-restore.
- **No mobility-snapshot change** in Slice 2 (the binding fields landed in
  Slice 1) → no `mobility_snapshots` DELETE for this slice.
- Frozen-time safe: on restart the count rehydrates, the factor re-derives, and
  the audit's ephemeral `LastTickMoney` re-seeds from the restored `total_money`;
  since all flows remain transfers, the audit survives (magnitude may step once on
  the first post-restart monthly recompute — deterministic).

## Determinism

- The factor is a pure function of the (deterministic) live count + a config
  baseline, recomputed at the deterministic monthly boundary. No RNG, no
  wall-clock, no hash-map iteration influencing outputs.
- Same inputs + same tick → byte-identical flows and a byte-identical audit.

## Sub-slicing (the `#78` audit is the canary throughout)

Each sub-slice is independently green (full cargo gate) and ships behind the
`capita_factor` knob, which defaults to **identity (1)** until 2e.

- **2a — knob, identity default, demand-side real-qty scaling.** Add
  `CapitaFactor` (default 1) + `capita_baseline` config; scale `autonomous` +
  `desired_qty` in `pools.rs`. At factor=1 behaviour is byte-identical. Property
  test: audit byte-invariant for factor ∈ {1, 2, 10}.
- **2b — symmetric supply + seed money.** Scale `offered_qty` and
  `opening_cash`/`opening_inventory` at seed by the same knob; one-time
  `economy_snapshots` DELETE; test `total_money` post-seed == Σ(scaled
  opening_cash) and the per-tick audit stays invariant.
- **2c — wire the live-count signal.** Derive `capita_factor` from
  `AgentIdIndex.len()` / `capita_baseline`, snapshotted monthly in/after
  `population_monthly_system`. Factor becomes emergent. Test the monthly recompute
  + determinism + that the audit holds as the count changes.
- **2d — population-aware cohort cap (visible density).** Replace absolute 4 with
  `pop_cap`; calibrate `per_unit`/per-citizen rate so the cohort tracks bound
  citizens. Test cohort grows with population, stays viewport-independent, and
  attribution still moves no money.
- **2e — overflow stress test + ramp.** Add the per-capita overflow stress test;
  only then ramp the default so the live count produces a real factor > 1. Note
  the ramp direction: `factor = live_count / capita_baseline`, so identity is
  `capita_baseline = seed_count` (≈300 → factor 1); to scale throughput **up** you
  **lower** `capita_baseline` (or raise the per-citizen rate), e.g.
  `capita_baseline = 10` → factor ≈ 30 at 300 citizens. Ramp by *lowering* the
  baseline, never raising it.

## Testing & Gate

- Conservation: `run_tick_audit_at_tick` byte-invariant at multiple factors;
  `HOUSEHOLD_SECTOR`/`TRANSPORT_OPERATOR` net-zero sentinels hold; `wage ≤ revenue`.
- Magnitude: scaled demand is fundable (no cash-starvation collapse) once supply
  + opening_cash scale together; prices stay within `price_floor`/`price_ceiling`
  (no tâtonnement blowup).
- Density: cohort grows with the live count; viewport-independent; attribution
  writes no money/goods.
- Overflow: the new per-capita stress test (2e).
- Determinism: monthly recompute reproducible; factor=1 byte-identical to base.
- **Mandatory browser-smoke:** the agent stream changes (more citizens visibly
  economic) — run the headless render-smoke; the render-smoke 300-pin still holds
  (citizens keep `agent:walk:*` ids), but more of them are economically routed.
- Full CI gate (Rust fmt/clippy/test workspace; frontend typecheck/vitest/build;
  e2e), green per sub-slice.

## Acceptance Criteria

Slice 2 is complete when:

- `capita_factor` is derived from the live count (monthly), defaulting to identity
  until ramped, with `capita_baseline` config.
- Demand, supply, and seed money/inventory scale symmetrically by the factor;
  wages remain labor-share of the larger revenue (no artificial multiply).
- The cohort cap is population-aware; visible economic density tracks the live
  count while preserving viewport-independence.
- The `#78` audit is byte-invariant at all tested factors; sentinels and
  `wage ≤ revenue` hold; the per-capita overflow stress test passes.
- A one-time `DELETE FROM economy_snapshots` is documented as the deploy step; no
  serde-default/heal shim; no `mobility_snapshots` change.
- Full CI gate green per sub-slice; browser-smoke confirms increased economic
  routing; the 300-pin holds.

## Risks & Mitigations

- **Audit panic (highest):** any runtime mint, or scaling wages above revenue,
  trips the release-grade `ConservationViolation`/net-zero `.expect`. *Mitigation:*
  scale only via transfers/recomputed flows + seed-time mint; wages stay
  labor-share; the audit is the per-sub-slice canary.
- **Cash starvation / demand collapse:** scaling demand without scaling
  `opening_cash` + supply drains pools to `InsufficientFunds`. *Mitigation:* the
  hybrid scales all three symmetrically (2b before any ramp).
- **i64 overflow → fail-fast:** large factors stress checked paths.
  *Mitigation:* live count keeps the factor ~10–30×; the 2e stress test pins the
  ceiling behaviour; ramp only after green.
- **Price blowup/oscillation:** demand scaled past supply pushes prices to the
  ceiling. *Mitigation:* symmetric supply scaling keeps excess demand bounded;
  test prices stay within guardrails.
- **Persistence/migration:** scaled seed money needs the one-time
  `economy_snapshots` DELETE; skipping it restores an un-scaled stock.
  *Mitigation:* documented deploy step; fail-fast surfaces a mismatch.
- **Cross-module coupling:** economy reading `AgentIdIndex` is the first such
  dependency. *Mitigation:* read a monthly snapshot (not mid-month transient);
  backend-only, no frontend wire.

## Deferred Slices

1. **Dynamic labor market** (per-citizen wages, firm matching).
2. **Elasticity-shaped demand** (the free-prices follow-on).
3. **Multi-stage production chains** (firms-as-buyers) — the other major economy
   direction, independent of per-capita.
4. **Distinct economic-citizen sprites** (shopping/commuting visual states).
