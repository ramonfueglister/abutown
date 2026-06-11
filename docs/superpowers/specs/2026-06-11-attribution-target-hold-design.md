# Economy: Attribution Target Hold — Sample-and-Hold Across the Macro-Flow Interval

Date: 2026-06-11

## Status

Approved, **literature-grounded** (see References). Fixes the strobing
`routed_citizens` vitals gauge and stabilizes pedestrian economic destinations.
Base: `worktree-attribution-target-hold` off `origin/main` (`2315685`).

## Problem (verified by phase-bucketed WS sampling, 2026-06-11)

`run_citizen_attribution_system` (`economy/attribution.rs`) overwrites
`CitizenEconomicTargets` **every tick** from `MarketGoodState.consumed_qty_last_tick`
and `WageTelemetry`. But `consumed_qty_last_tick` is zeroed every tick
(`pools.rs`) and is nonzero only on macro-flow delivery ticks
(`EconomyConfig.macro_flow_interval_ticks = 10`). Result: the attribution cohort
pulses to 120+ citizens exactly on `tick % 10 == 1` and is **empty on the other
9 phases**. Downstream:

- The vitals gauge `routed_citizens` (read in `sim-server/src/app/mod.rs`) strobes
  with a 10% duty cycle; the frontend HUD samples per snapshot frame and reads
  "constant 0".
- The `economy::liveness` heartbeat logs every 60 ticks, and `60 ≡ 0 (mod 10)`
  **aliases onto the pulse phase** — it reads "sustained routed=124" while the
  gauge is 0 for 90% of sim time. Both observers are misled by the same signal.
- Pedestrian economic destinations (`route_assignment_system` reads the targets)
  exist for only 1 tick in 10 — the intended "citizens visibly walk to markets"
  behavior is mostly off.

Not a regression: semantics identical since PR #86. Diagnosis recipe lesson:
**never trust a fixed-cadence liveness log for a cadence-sensitive gauge whose
period divides the log period — sample per tick and bucket by `tick mod interval`.**

## Theoretical grounding

The macro flow is a **slow-timescale process** (period `N = macro_flow_interval_ticks`)
inside a fast-tick simulation — a multirate sampled-data system. Attribution is the
sampler that projects the slow process onto fast-tick agents.

1. **Zero-order hold (ZOH).** In sampled-data control, the canonical reconstruction
   of a slow signal between samples is the zero-order hold: the last sampled value
   is held constant until the next sample (Åström & Wittenmark, 2011). Emitting
   zero between samples — the current behavior — is not a reconstruction at all;
   it aliases the sampling cadence into the signal. The fix is to make attribution
   a ZOH: the cohort computed at a delivery tick persists until the next delivery
   tick recomputes it.

2. **Sticky information / staggered updating.** In macroeconomics, agents acting on
   their **last-computed plan** between infrequent re-optimizations is canonical:
   Calvo (1983) staggered updating, and Mankiw & Reis (2002) sticky information,
   where agents re-plan only when an information update arrives and otherwise
   continue executing the stale plan. A citizen attributed to a market at the last
   delivery keeps walking there until the next delivery re-attributes — that is
   the sticky-information semantics, not an approximation of something else.

3. **Conservation framing.** Attribution remains READ-ONLY over economy quantities
   (Godley & Lavoie, 2007, "no black holes"): it selects which citizens *depict*
   realized flows; it mints and moves no money. The `#78` per-tick byte-exact
   `total_money` audit is untouched by construction. The partition identity
   `attributed + unobserved == realized` continues to hold per delivery tick;
   the hold only extends the *display/destination lifetime* of the selection.

## Design

### Mechanism: hold-on-fresh, expire-after-interval

In `run_citizen_attribution_system`, after computing this tick's cohort `targets`:

- **Fresh** (`!targets.is_empty()`): overwrite `CitizenEconomicTargets` and stamp
  `last_refresh_tick = tick`. (Macro flow is globally gated on one cadence, so all
  markets deliver on the same phase — a non-empty cohort is a delivery tick.)
- **Hold** (`targets.is_empty()` and `tick − last_refresh_tick < N`): keep the
  held map unchanged.
- **Expire** (`targets.is_empty()` and `tick − last_refresh_tick ≥ N`): overwrite
  with the empty map. A full macro-flow interval elapsed with zero realized
  activity → the economy is genuinely producing nothing observable; the gauge
  must honestly read 0, not hold stale targets forever.
- **Off-screen** (`observed_markets.is_empty()`): clear immediately, as today.
  Occlusion semantics are orthogonal to the hold — citizens must not walk to
  unobserved markets, and the gauge tracks *observed* attribution.
- `macro_flow_interval_ticks == 0` (macro flow disabled) ⇒ hold disabled
  (always overwrite) — exact legacy per-tick semantics.

State: one new **ephemeral** resource `AttributionHold { last_refresh_tick: u64 }`,
initialized on demand inside the exclusive system (avoids the PR #86 class of
constructor-wiring bugs). Never persisted: after a restart the hold re-arms within
one delivery interval (≤ N ticks of empty gauge), which matches the frozen-time
persistence model. **No snapshot schema change, no `DELETE FROM economy_snapshots`.**

`Tick` absent (minimal test worlds) ⇒ treated as tick 0; fresh cohorts still
overwrite, so existing unit tests are unaffected.

### Documented limitations

- A citizen that dies mid-interval can linger in the held map for < N ticks
  (routing simply finds no such agent; the gauge over-counts by the death count
  for ≤ N ticks). Negligible at N = 10 and demographically rare; revisit only if
  N grows large.
- If a future change staggers market delivery phases (per-market cadence), the
  overwrite-on-fresh rule would drop other markets' cohorts on each partial
  delivery. The remedy then is a per-window union (integrate-and-dump sampler);
  out of scope while the macro gate is global.

### Explicitly NOT in scope

- No display-side smoothing (windowed max / EWMA in the vitals snapshot) — the
  alternative minimal fix. Rejected: it fixes the gauge but leaves pedestrian
  destinations strobing, and it makes the wire report a value that no simulation
  state corresponds to.
- No change to the `#78` audit, money movement, pools, macro flow, or pricing.
- No change to the 60-tick liveness log cadence: once the signal is a ZOH it is
  phase-invariant and any sampling cadence reads it correctly.

## Tests (TDD)

In `economy/attribution.rs` (system-level, mirroring existing exclusive-system tests):

1. **Hold:** delivery tick attributes a cohort; subsequent tick with zeroed
   telemetry retains the identical map (RED under current per-tick overwrite).
2. **Expire:** after `macro_flow_interval_ticks` ticks with no fresh telemetry,
   the map clears.
3. **Fresh overwrite:** a later delivery with a different realized magnitude
   replaces (not merges) the held cohort.
4. **Off-screen clear wins over hold:** with a held cohort, removing all
   observed chunks clears immediately.

Existing tests must pass unchanged (they exercise the fresh path or the
off-screen path). Full local CI gate per `run-full-ci-gate-before-push`.

## References

Åström, K. J., & Wittenmark, B. (2011). *Computer-controlled systems: Theory and
design* (3rd ed.). Dover Publications.

Calvo, G. A. (1983). Staggered prices in a utility-maximizing framework.
*Journal of Monetary Economics, 12*(3), 383–398.

Godley, W., & Lavoie, M. (2007). *Monetary economics: An integrated approach to
credit, money, income, production and wealth*. Palgrave Macmillan.

Mankiw, N. G., & Reis, R. (2002). Sticky information versus sticky prices: A
proposal to replace the New Keynesian Phillips curve. *The Quarterly Journal of
Economics, 117*(4), 1295–1328.
