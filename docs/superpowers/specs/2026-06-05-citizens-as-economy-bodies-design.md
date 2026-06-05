# Citizens as the Economy's Bodies — Demographics↔Economy Merge (Slice 1)

Date: 2026-06-05

## Status

Approved direction from brainstorm. This spec merges the two formerly-decoupled
agent layers — demographic aging citizens and the mean-field economy's ephemeral
"shadow" agents — so that **persistent, aging citizens are the visible bodies of
the economy**. The shopper/commuter shadows are removed; the citizens do those
errands, economically targeted by realized demand and wages.

Two design decisions were settled in the brainstorm and are load-bearing for the
rest of this spec:

1. **Causality = Hybrid (macro authoritative).** The mean-field macro layer stays
   the sole economic authority — viewport-independent, `O(sectors)`, with the
   per-tick SFC conservation audit intact. Citizens are routed to economic nodes
   and a per-observed-citizen settlement reconciles, conservation-exact, against
   the macro aggregate. Citizens are real bodies; transaction *magnitude* stays
   macro-governed. This is **not** full microfoundation (agents do not become the
   economic authority).
2. **Binding = static seed-time.** Each citizen carries a `home_market` (where it
   shops) and a `work_market` (where it earns wages), assigned deterministically
   at seed time from the `markets.json` anchors. Newborns inherit the mother's
   binding. The binding is persisted.

**Base branch.** This work targets `origin/main` (HEAD `3a64544`), which contains
the economy module and the demographic-cursor persistence fix (PR #63). It does
**not** target `plan/persistence-liveness`, which predates both. Implementation
proceeds on `feat/citizens-as-economy-bodies` (a worktree off `origin/main`).

## Goal

Make the existing aging ECS citizens (`AgentMarker` entities) the bodies that
perform the economy's realized consumption trips (to their `home_market`) and
wage commutes (to their `work_market`), replacing the ephemeral shopper/commuter
`TraderAgent` shadows — while keeping the macro economy authoritative and the
`#78` per-tick money-conservation audit byte-invariant and unchanged.

## Background — The Two Layers Today

Verified against `origin/main`:

**Layer A — Demographic citizens** (`mobility` + `population`). A citizen is a
Bevy ECS entity tagged `AgentMarker`, identified by `StableAgentId`, carrying a
durable signed `BirthTick` (age is derived from the global `SimClock`), `Sex`,
optional `ParentId`, `Position`, and a MATSim-style `WalkPlan`.
`population_monthly_system` catches up each uncrossed sim-month with
Gompertz–Makeham mortality and Gaussian-ASFR fertility, gated by a
`LastProcessedMonth` cursor persisted in `MobilityPersistSnapshot`. Movement is
cyclic `home`↔`destination` commuting; today **both endpoints are purely
geometric** — `activity:home`/`activity:destination` resolve to the first/last
points of the seeded pedestrian corridor → nearest footway node. `routing.rs`
contains zero references to the economy. Origin/main seeds 300 walkers plus
accumulated births.

**Layer B — Mean-field economy** (`sim-core::economy`, `EconomyPlugin`). A
stock-flow-consistent macro/micro model. The macro layer is the sole authority,
viewport-independent, `O(sectors)`: a `HouseholdSector { population, pool_weights }`
(demand), firm `SupplyPool`s, extractors, per-`(market, good)` aggregates relaxing
toward Law-of-One-Price, sentinel accounts for `TRANSPORT_OPERATOR` and
`HOUSEHOLD_SECTOR`. The closed circular flow: firms pay labor-share + 100% profit
dividend + transport rebate to households (firms net to zero); households consume
via Keynesian `C = a + b·Y` with a one-tick lag. The chained `EconomySet`
schedule ends in `run_tick_audit_at_tick`, which asserts `total_money` is
byte-invariant tick-over-tick and `.expect()`-panics on drift (release-grade
fail-fast), plus `HOUSEHOLD_SECTOR` net-zero sentinels.

**How they connect today: they do not, behaviorally.** The only shared substrate
is render/wire plumbing. Economy shadow agents — flow-traders (`1<<32`), shoppers
(`2<<32`), commuters (`3<<32`) — are ephemeral render-only `TraderAgent` entities
that carry no `AgentMarker` and no economic state, are transaction-gated
(shoppers where `consumed_qty_last_tick > 0`, commuters where `WageTelemetry`
wage `> 0`, flow-traders per accepted `MacroFlow` edge), viewport-bounded (only
markets in Active/Hot chunks), hard-capped (`max_shoppers_per_market = 4`,
`max_commuters_per_market = 4`, absolute — never derived from magnitude), and
explicitly filtered out of the mobility persist snapshot. Crucially, citizens and
shadows are the **same on-wire type** (`AgentMobilityDto`), distinguished only by
an id/`sprite_key` prefix. `MarketSite` carries a `node_id`; **markets are the
only spatial anchors** — supply/demand/extractors all attach to a market, there
is no separate firm position, so both shadow kinds walk to the market node.
`HouseholdSector.population` is seeded to `1_000_000` but is **inert** — written
and persisted, never read by any economic-math file.

## Scope & Sequencing

This spec is **Slice 1 only**: correctness of the mechanism at the current
**1:1 scale (~300 citizens)**.

- **Slice 1 (this spec):** citizens economically targeted; shopper/commuter
  shadows removed and replaced by citizens; conservation-exact attribution; tests,
  gate, and diagnostics moved onto the new model.
- **Slice 2 (separate, later spec):** activate the deferred per-capita scaling
  (wire the inert `HouseholdSector.population` into wage/consumption magnitudes so
  economic throughput tracks the live citizen count), with a fresh SFC audit and
  overflow analysis.

**Honesty about Slice-1 density.** At demo-scale macro aggregates, the macro
justifies only a handful of economic trips per tick (≈ today's 4/4 caps). In
Slice 1, most of the ~300 citizens keep their existing geometric routine;
economic targeting *overlays* it for the small attributed cohort. The city will
not go dead (300 keep walking) but it will not yet look economically *dense* —
density is Slice 2. This ordering deliberately lands correctness before density,
isolating the per-capita stability risk (i64 overflow, audit fail-fast) into its
own audited slice.

## Spec-Conformance

- **Economy v0** (`2026-05-30-economy-v0-design.md`) and its follow-on roadmap
  establish the macro layer as the sole economic authority with agents as
  conservation-exact projections, and the `#78` SFC audit as the invariant. Slice
  1 preserves both: the macro math is unchanged; the projection layer's *bodies*
  change from ephemeral shadows to persistent citizens.
- **Population dynamics** (`2026-05-29-population-dynamics-minimal-design.md`) and
  **time/agent aging** (`2026-05-29-time-system-and-agent-aging-design.md`)
  establish the aging citizen as a persistent, individually-identified entity.
  Slice 1 keeps mortality/fertility/aging unchanged and adds an economic binding
  + targeting on top.
- **Mobility persistence/liveness** (`2026-05-30-mobility-persistence-liveness-design.md`)
  and the **demographic persistence fix** (`2026-05-31-demographic-persistence-fix-design.md`,
  PR #63) establish the frozen-time persistence model and the `LastProcessedMonth`
  cursor. Slice 1 adds the citizen↔market binding to the persisted state and
  requires a one-time mobility-state reset (below).
- No new wire path is introduced: citizens already serialize as `AgentMobilityDto`
  and stream through the existing chunk-delta path.

## Non-Goals

Slice 1 does **not** include:

- per-capita scaling / wiring `HouseholdSector.population` into any formula
  (Slice 2);
- full microfoundation (per-citizen `DemandPool`/budget; macro derived from
  agents);
- a dynamic labor market (wage-signal matching of citizens to firms);
- removal or rework of flow-traders (inter-market goods shipments) — they are
  retained;
- new distinct sprites for economically-active citizens beyond the existing
  pedestrian rendering (an optional Slice-1b polish, not required here);
- any change to the macro economic math (`C = a + b·Y`, wage bill, dividends,
  LoOP relaxation, auction/clearing);
- any new `economy_snapshots` field (so no economy-store migration is required by
  this slice).

## Architecture After Merge

The macro economy is untouched and authoritative. The **projection layer** is
what changes:

- **Removed:** the shopper and commuter shadow projections (their spawn/capture
  systems, their synthetic id bands `2<<32` and `3<<32`, and their exclusion from
  the persist snapshot).
- **Added:** an economic *targeting* override on citizen movement, plus a per-tick
  conservation-exact *attribution* step that maps the macro's realized
  consumption/wage quantities onto observed, market-bound citizens.
- **Retained:** flow-traders (they depict inter-market goods shipments, which no
  citizen represents; they are also the only bodies that render as distinct
  `trader:` sprites today).

One agent kind now represents households: the aging citizen.

## Component — Citizen↔Market Binding

Each citizen gains two stable fields:

- `home_market: MarketId` — the market this citizen shops at (consumption trips).
- `work_market: MarketId` — the market this citizen earns wages at (wage
  commutes); chosen `!= home_market` when more than one market exists.

**Assignment (deterministic, seed-time).**

- Seeded citizens: `home_market` = the market whose anchor is nearest the
  citizen's seeded home position (via the existing `NodeSpatialIndex` /
  market anchors); `work_market` = a deterministically chosen wage-paying market
  distinct from `home_market` (selection rule fixed by `StableAgentId`, not by
  wall-clock or RNG, consistent with the codebase's stateless deterministic
  draws).
- Newborns: inherit the **mother's** `home_market` and `work_market` (newborns
  already spawn at the mother's position with a clone of her plan).

No floats, no RNG, no wall-clock: assignment is a pure function of seed positions,
market anchors, and `StableAgentId`, so it is reproducible and replay-safe under
the frozen-time model.

## Component — Economic Movement Targeting

The citizen movement system (`mobility/systems/routing.rs`, `destination_for_stage`
/ `ActivityWaypoints`) is **augmented, not replaced**:

- When the citizen is in the tick's *attributed cohort* for its `home_market` and
  the macro reports `consumed_qty_last_tick > 0` for that market, the
  `destination` stage resolves to the `home_market` node.
- When the citizen is in the attributed cohort for its `work_market` and that
  market paid wages this tick (`WageTelemetry`), the `commute` stage resolves to
  the `work_market` node.
- Otherwise the stage falls back to today's geometric corridor endpoints.

Walk execution, routing, frame interpolation, the on-wire `AgentMobilityDto`, and
client rendering are all unchanged — only endpoint *resolution* changes. The
"attributed cohort" is defined by the attribution step below.

## Component — Attribution / Reconciliation (conservation-exact)

This is the rigor that distinguishes the chosen Hybrid from a loose visual
binding. Per tick, per `(market, good)` and per market-wage:

1. The macro's realized `consumed_qty_last_tick` (and `WageTelemetry` wage) is the
   **authority** — computed by the unchanged macro systems.
2. An attribution step selects, among observed citizens bound to that market
   (i.e. in markets within Active/Hot chunks), a cohort to represent the realized
   quantity, and partitions the realized quantity into per-citizen shares.
3. The partition is bounded so that **Σ(attributed shares) ≤ realized aggregate**,
   with the explicit remainder `realized − Σ(attributed)` carried as the
   *unobserved* share. The cohort size is bounded by the realized magnitude
   divided by a per-unit constant (the same shape as today's shopper/commuter
   `target` computation), so at Slice-1 scale the cohort stays small (≈ today's
   caps) and the bound never couples to the population — preserving
   viewport-independence.

**No money or goods are minted or moved per citizen.** The macro's existing single
aggregate transfer remains the only money/inventory movement. Attribution is a
read-only partition over already-conserved quantities, used to (a) pick which
citizens are economically targeted this tick and (b) feed diagnostics. Therefore
`run_tick_audit_at_tick` byte-invariance (`#78`) and the `HOUSEHOLD_SECTOR`
net-zero sentinels remain valid **unchanged**.

A new diagnostic invariant is asserted: for every observed `(market, good)` and
market-wage, `Σ(attributed) + unobserved == realized aggregate` (exact integer
identity). This is the slice's own conservation check, layered above — not
replacing — the `#78` money audit.

**Cohort granularity.** Attribution and the conservation identity are computed
per `(market, good)` (and per market-wage). Movement targeting, however, is
market-level: a citizen walks to a *market node*, not to a good. A citizen is in
its `home_market`'s attributed cohort (the membership the targeting component
reads) if it is selected to represent realized consumption of **any** good at
that market; likewise for `work_market` wages. So targeting uses the market-level
union of per-good cohort memberships, while the `Σ(attributed) + unobserved ==
realized` identity is verified at the finer per-`(market, good)` grain.

## Component — Deletions

Remove the shopper and commuter shadow machinery:

- `economy/shoppers.rs`, `economy/commuters.rs`, and their spawn/capture systems
  and `EconomySet` phases (the shopper/commuter capture + materialize phases).
- Their synthetic `EconomicActorId` id bands (`2<<32`, `3<<32`) and the
  `shopper:`/`commuter:` `sprite_key` prefixes.
- Their entries in the persist-snapshot `TraderAgent` exclusion filter
  (`persist_snapshot.rs`): the economic bodies are now persistent `AgentMarker`
  citizens, which the snapshot already includes — there is nothing left to exclude
  for shoppers/commuters.

Flow-trader machinery (`1<<32`, `trader:` sprites, inter-market shipments) is
retained.

## Persistence & Migration

- The two new citizen fields (`home_market`, `work_market`) are added to the
  persisted citizen state in `MobilityPersistSnapshot`.
- **A one-time mobility-state reset is required.** No serde-default shim and no
  heal-on-restore guard are added (per project convention: fix the root cause,
  surface the consequence). **Consequence, stated plainly:** existing dev saves
  lose accumulated ages and births when the mobility snapshot is reset. This is
  acceptable in the current pre-release dev context and consistent with the
  established one-time-reset practice when crossing a persisted-schema slice.
- **No `economy_snapshots` change** is introduced by this slice, so no
  `DELETE FROM economy_snapshots` is required for Slice 1.
- The binding is restored with the citizen; under the frozen-time model
  (server up = sim runs; server down = frozen; resume from saved tick) a restored
  citizen keeps its `home_market`/`work_market` and resumes economic targeting
  with no offline catch-up.

## Render

- Shoppers/commuters already render as plain pedestrians (the frontend's
  `isTraderSpriteKey` matches only the `trader:` prefix), so replacing them with
  real citizen pedestrians needs **no client render change**.
- Flow-traders keep their distinct `trader:` sprites.
- *Optional (Slice 1b, not required):* a visible "shopping"/"commuting" citizen
  state to make economically-active citizens distinguishable from idle ones,
  closing the `isTraderSpriteKey` gap. Deferred unless explicitly requested.

## Determinism

- Binding assignment, cohort selection, and share partitioning use `BTreeMap` /
  explicitly sorted key vectors and pure functions of seed data, market anchors,
  realized quantities, and `StableAgentId`.
- No `thread_rng`, `rand::random`, UUIDs, wall-clock time, or hash-map iteration
  order influence any economic targeting or attribution outcome.
- Same inputs and same tick produce byte-identical targeting + attribution and a
  byte-identical money audit.

## Testing & Gate

TDD throughout. Required coverage:

- **Binding:** seeded citizens get deterministic `home_market`/`work_market`;
  `work_market != home_market` when ≥2 markets; newborns inherit the mother's
  binding; binding round-trips through persistence.
- **Targeting:** an attributed citizen whose `home_market` consumed `> 0` retargets
  to the market node; an attributed citizen whose `work_market` paid wages
  retargets; a non-attributed citizen keeps geometric endpoints.
- **Attribution conservation:** `Σ(attributed) + unobserved == realized` for every
  observed `(market, good)` and market-wage; cohort size bounded by realized
  magnitude (never by population); off-screen markets attribute nothing.
- **Macro invariance:** `run_tick_audit_at_tick` stays byte-invariant with citizens
  as bodies (port/retain the existing shopper/commuter target tests as
  citizen-attribution tests).
- **Persistence:** save→reset→restore keeps bindings; the one-time reset path is
  covered.
- **Mandatory browser-smoke:** the frontend↔backend agent stream changes (shadows
  removed, citizens economically targeted). Run a real headless browser smoke
  (adapt `scripts/smoke-7a.mjs`) and confirm citizens stream and render, per the
  project's browser-smoke mandate.
- **Full CI gate before push:** Rust fmt-check + clippy + test (via
  `scripts/cargo-serial.sh`), frontend typecheck (src + tests + scripts) + vitest +
  build, and e2e. ⚠️ The **render-smoke pins exact agent counts** — removing the
  shadow agents changes the counts, so the pin must be updated to the new
  citizens-only expectation.

## Acceptance Criteria

Slice 1 is complete when:

- Citizens carry `home_market`/`work_market`, assigned deterministically at seed,
  inherited by newborns, and persisted.
- Shopper and commuter shadow machinery, their id bands, and their persist-filter
  entries are removed; flow-traders remain.
- Citizen movement retargets to economic nodes for the attributed cohort and falls
  back to geometric endpoints otherwise.
- The attribution step partitions realized macro quantities onto observed citizens
  with the exact `Σ(attributed) + unobserved == realized` identity, bounded by
  magnitude (not population).
- No money/goods are minted or moved per citizen; `run_tick_audit_at_tick` remains
  byte-invariant and the `HOUSEHOLD_SECTOR` net-zero sentinels hold.
- The one-time mobility-state reset is implemented (no serde-default/heal-on-restore
  shim); the consequence is documented.
- No `economy_snapshots` schema change.
- All required tests pass via the cargo wrapper; the browser-smoke confirms
  citizen streaming/rendering; the full CI gate is green with the render-smoke
  agent-count pin updated.

## Risks & Mitigations

- **Audit fail-fast on a per-citizen bug.** Because attribution moves no money, the
  `#78` audit should stay valid; but it is release-grade `.expect()`-panic, so any
  accidental money-moving code in the new path halts the tick in production.
  *Mitigation:* keep attribution strictly read-only over macro quantities; assert
  the `Σ` identity in tests; never write to `AccountBook`/`InventoryBook` from the
  targeting/attribution path.
- **Viewport-independence regression.** Making targeting depend on observation must
  not make *economic correctness* depend on observation. *Mitigation:* the macro
  remains authoritative and runs for all sectors regardless of observation;
  attribution only selects which already-realized aggregate is *depicted*, never
  what is *computed*. Off-screen markets still clear normally.
- **Sparse Slice-1 city.** At demo aggregates most citizens stay geometric.
  *Mitigation:* documented as intended; density is Slice 2 (per-capita).
- **Persistence ordering.** Adding bindings to the mobility snapshot requires the
  one-time reset before deploying this slice; skipping it means hydration loads
  citizens without bindings. *Mitigation:* the reset is part of the slice; no
  silent default-fill.
- **Base-branch staleness.** Doing this work anywhere but off `origin/main` loses
  the economy, the 300-agent scale, and PR #63. *Mitigation:* enforced — work is on
  `feat/citizens-as-economy-bodies` off `origin/main`.

## Deferred Slices

1. **Per-capita scaling (Slice 2):** wire `HouseholdSector.population` into the
   wage-bill and consumption magnitudes so throughput tracks the live citizen
   count; re-run the SFC audit; analyze i64/i128 overflow at scale. This is what
   brings economic density and busy citizens.
2. **Dynamic labor market:** citizens match to firms by wage signals; `work_market`
   becomes dynamic rather than seed-static (the static binding here is the on-ramp).
3. **Distinct economic-citizen sprites (Slice 1b):** visible shopping/commuting
   states closing the `isTraderSpriteKey` gap.
4. **Flow-trader reconciliation:** if desired later, give inter-market shipments the
   same conservation-exact attribution treatment.
