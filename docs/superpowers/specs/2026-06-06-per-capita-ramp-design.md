# Ramp + Live-Validate Per-Capita Density — Design (Slice 2b)

Date: 2026-06-06

## Status

Approved from brainstorm. Slices 1+2 built the full per-capita machinery (citizens
as economic bodies + a `CapitaFactor` that scales throughput and the visible
attribution cohort), but Slice 2 ships at **identity** (`capita_baseline =
1_000_000` → factor 1 at ~300 citizens), so the density it enables is **inert at
runtime**. This slice turns it on for the abutopia world and validates it live —
the "last mile" that makes the visible economic density real.

Settled decisions (brainstorm):
1. **`capita_baseline` becomes authorable from `markets.json`** (world data), with a
   serde-default of `1_000_000` (identity) for any world that omits it. The code
   default stays identity; only abutopia's `markets.json` carries the ramp. Tuning
   = edit JSON + restart (no recompile). No DB migration (this is world data, not a
   persisted snapshot).
2. **Start candidate `capita_baseline = 10`** (≈30× at ~300 citizens — already
   proven audit-safe, solvent, overflow-safe in Slice 2), then tune by observation.

**Base branch.** `feat/per-capita-ramp` off `origin/main` (`2961e94`, Slices 1+2).

## Goal

Make the abutopia economy visibly track its ~300 citizens — a clear share of
observed citizens visibly heading to / clustered at markets — by authoring a
ramped `capita_baseline`, while keeping the #78 money-conservation audit
byte-invariant, prices stable, and performance fine. Validate both quantitatively
(backend metric) and live (running stack + screenshot).

## Background — verified current state (`2961e94`)

- `EconomyConfig` is inserted as a hardcoded `EconomyConfig::default()`
  (`economy/mod.rs:72`); `capita_baseline` defaults to `1_000_000` (identity at
  seed scale). The runtime/`seed_from_markets_layer` never builds or overrides
  `EconomyConfig` from the world bundle.
- `markets.json` has `household: { population: 1000000 }` — `population` feeds the
  inert `HouseholdSector.population` (persisted but read by no economic math).
- `CapitaFactor = max(1, live_AgentMarker_count / capita_baseline)`, recomputed
  each tick (`EconomySet::RefreshCapita`), scales demand/supply/production/cohort.
- Visible economic activity = citizens in `CitizenEconomicTargets` (Slice 1's
  attribution + routing) — observable server-side as the map's size; not directly
  flagged on the wire (citizens stream as ordinary pedestrians).

## Non-Goals

- Changing the `CapitaFactor` formula or the scaling mechanics (Slice 2, unchanged).
- A general config-file system; only `capita_baseline` becomes authorable here.
- Distinct economic-citizen sprites (deferred Slice 1b) — density is validated by
  position/clustering + a server-side count, not a new sprite.
- Removing/migrating `HouseholdSector.population` (leave it; avoid a snapshot
  migration). `capita_baseline` is a *distinct* field with distinct meaning.

## Architecture

A single authored value flows from world data into the economy:

1. **Author:** `markets.json` `household.capita_baseline` (new field).
2. **Load:** the world-bundle parse picks it up (serde-default `1_000_000`).
3. **Seed:** `seed_from_markets_layer` writes it into `EconomyConfig.capita_baseline`
   (overriding the hardcoded default) at world creation.
4. **Run:** `refresh_capita_factor_system` (unchanged) derives the factor from it
   each tick; the Slice-2 scaling does the rest.

## Component — authorable `capita_baseline`

- `base_world.rs` `HouseholdSpec` (the `household` block) gains
  `#[serde(default = "default_capita_baseline")] pub capita_baseline: i64` where the
  default fn returns `1_000_000`. Worlds omitting the field stay at identity.
- `seed_from_markets_layer` (`markets_layer.rs`): after `EconomyConfig` exists, set
  `world.resource_mut::<EconomyConfig>().capita_baseline = layer.household.capita_baseline`
  (or build/insert `EconomyConfig` with it). Single write; no other config change.
- `data/worlds/abutopia/layers/markets.json`: set `household.capita_baseline` to the
  validated value (starting candidate `10`).

## Component — validation

**Backend (deterministic, in-repo):** an integration test that builds a world with a
ramped `capita_baseline` and asserts, vs an identity run: (a) the routed cohort /
`CitizenEconomicTargets` size (or attributed-per-market count) is materially larger;
(b) the #78 audit stays byte-invariant every tick; (c) prices stay within
`price_floor`/`price_ceiling` over a longer run (no tâtonnement blow-up); (d) no
demand-collapse (no all-`InsufficientFunds` tail). Confirms the chosen factor is
safe + actually denser, without the browser.

**Live (acceptance):** fast-forward the dev worktree to this branch (after stopping
the current dev server — Slice 2b needs no DB migration, so a clean restart
suffices), restart the stack, then drive a headless browser-smoke that (i)
screenshots the city for a visual density check and (ii) reports a server-side
routed-citizen count. Tune `capita_baseline` in `markets.json` and restart until the
density looks right and prices/perf hold. The "looks alive" judgment is the user's.

## Determinism, persistence, performance

- The factor is still a deterministic function of the live count + the (now
  world-authored) baseline; no RNG/wall-clock.
- **No DB migration:** `markets.json` is world data loaded fresh each start;
  `capita_baseline` adds no persisted snapshot field. `HouseholdSector.population`
  is untouched. No `economy_snapshots`/`mobility_snapshots` change.
- Performance: scaling raises magnitudes, not sector/agent counts; the macro stays
  `O(sectors)`. The validation run confirms tick cost is unaffected at the chosen
  factor (~30×, ~300 citizens).

## Testing & Gate

- Backend density/safety test (above): routed cohort scales up; audit byte-invariant;
  prices in-band; no collapse — at the ramped factor vs identity.
- Existing economy suite stays green (code default still identity → unchanged).
- **Mandatory browser-smoke:** the agent stream now shows more citizens economically
  routed; the render-smoke 300-pin still holds (citizens keep `agent:walk:*` ids).
- Full CI gate (Rust fmt/clippy/test workspace; frontend typecheck/vitest/build; e2e).

## Acceptance Criteria

- `capita_baseline` is authorable from `markets.json` (serde-default 1M = identity);
  `seed_from_markets_layer` applies it; abutopia's JSON carries the chosen ramp.
- The backend density/safety test passes (denser than identity; audit byte-invariant;
  prices in-band; solvent).
- Live: the running abutopia stack shows a clear share of observed citizens
  economically routed (screenshot + server-side count), prices stable, perf fine;
  the user confirms it looks alive.
- Full CI gate green incl. browser-smoke; no migration required.

## Risks & Mitigations

- **Over-ramp (too dense / price blow-up / perf):** start at the Slice-2-validated
  `10` (~30×) and tune *up the baseline* (= less aggressive) if needed; the backend
  test pins price-band + audit; the live run confirms perf.
- **Identity safety regression:** the code default + all other worlds/tests stay at
  identity (serde-default) — only abutopia's JSON changes. Existing suite must stay
  green.
- **Live-run interference:** stop the current dev server before restarting on this
  branch (port/DB). No migration, so a clean restart is safe.

## Deferred

- Distinct economic-citizen sprites (Slice 1b) — would make density client-legible.
- Dynamic labor market; multi-stage production chains (the larger next directions).
