# Economy Settlement Policy Design — selectable midpoint settlement

Date: 2026-05-31

## Status

Backend polish (deferred economy follow-on). Adds **midpoint** settlement as a
config-selectable policy variant behind the existing auction interface, keeping
the current **anchored** rule as the default (no player-visible change unless the
server opts in — it is server-authoritative config, not a player action).

## Goal

`EconomyConfig.settlement_policy: SettlementPolicy { Anchored (default), Midpoint }`.
`Anchored` = today's rule (clamp the last settlement price into `[marginal_ask,
marginal_bid]`). `Midpoint` = `(marginal_bid + marginal_ask) / 2` (inherently
within the band, integer floor, i128 sum to avoid overflow). The live clearing
system (`clear_dirty_markets_system`) reads the configured policy and applies it.

## Architecture (delegation — near-zero ripple)

- New `SettlementPolicy` enum (in `auction.rs`): `Debug, Clone, Copy, PartialEq,
  Eq, Default` (so `EconomyConfig` keeps its derives). `#[default] Anchored`.
- `settlement_price` stays (the anchored rule). Add
  `settlement_price_with_policy(last, bid, ask, policy)` dispatching: `Anchored →
  settlement_price(...)`, `Midpoint → Money(((bid + ask) i128 sum)/2)`.
- `build_clearing_plan_with_policy(key, bids, asks, last, policy)` is the real
  impl (the current `build_clearing_plan` body, using
  `settlement_price_with_policy`). `build_clearing_plan(key, bids, asks, last)`
  becomes a thin wrapper delegating with `SettlementPolicy::Anchored` — so all 11
  existing `build_clearing_plan` callers/tests are untouched.
- `clear_market_good_with_policy(..., policy)` is the real impl; `clear_market_good(...)`
  delegates with `Anchored` — so all 5 existing `clear_market_good` callers/tests
  are untouched.
- `clear_dirty_markets_system` gains `config: Res<EconomyConfig>` and calls
  `clear_market_good_with_policy(..., config.settlement_policy)`. No test ripple
  (tests call the bare functions; the system runs only via the schedule, and
  `EconomyConfig` is always plugin-inserted).
- `EconomyConfig` gains `settlement_policy: SettlementPolicy` (default `Anchored`).
  Update the one `EconomyConfig { … }` literal in `tests/systems.rs`.

`SettlementPolicy` needs no serde (`EconomyConfig` is re-injected at boot, not
persisted — see the persistence-6a decision).

## Conservation / determinism

- Settlement only chooses the uniform clearing price; the matched quantity,
  allocations, and the conserving transfer machinery in `clear_market_good` are
  unchanged → money/goods still conserved under either policy.
- `Midpoint` is within `[ask, bid]` (midpoint of two ordered values), so every
  matched order remains valid at the price. Deterministic integer arithmetic.

## Testing

1. `settlement_price_with_policy_midpoint`: `(1200, 1000) → 1100`; odd sum floors
   (`(1001, 1000) → 1000`); `Anchored` matches `settlement_price` exactly.
2. `build_clearing_plan_with_policy_uses_midpoint`: a contested overlap settles at
   the midpoint under `Midpoint`, at the anchored price under `Anchored`/default.
3. `clearing_with_midpoint_policy_conserves`: drive `clear_market_good_with_policy(
   …, Midpoint)` → money + goods conserved, settles at midpoint.
4. Default unchanged: `build_clearing_plan` / `clear_market_good` (no policy) and
   all existing auction/conservation/rationing/determinism tests stay green.

Full gate: fmt + clippy `-D warnings` + `test --workspace --all-targets` on the
CI-matching stable toolchain (rustc 1.96).

## What this is NOT

- No change to matching/allocation/rationing or the default price behavior.
- Not persisted/replicated; pure server config. No per-market policy override
  (one global policy in v0).
