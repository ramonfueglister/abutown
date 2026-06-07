# Abutopia Live-Visible (Blockers 2+3) — Design Spec

Date: 2026-06-07

## Status

Approved from brainstorm (evidence-first, "alles gemäss specs"). Resolves
Blockers 2+3 from `docs/superpowers/plans/2026-06-06-slice2b-followups.md` so the
demographics↔economy merge is visible live: citizens shop at market 9002 on the
residential corridor. Base: `feat/abutopia-live-visible` off `origin/main`
(`53cd2e3`, Blocker-1).

## Goal

Get the abutopia frontend rendering the live world with citizens visibly routed
to market 9002, and gather rigorous in-repo evidence about the economy's long-run
price stability — without adding code the existing specs already cover or
explicitly deferred.

## Background — what the specs already settle

- **Persistence gate (Blocker-3):** `2026-05-30-mobility-persistence-liveness-design.md`
  intentionally makes the frontend **fail-closed** — a `stale` persistence health
  "must not boot" the canvas. That hard-block is **by design** (don't render if the
  backend genuinely cannot persist). `2026-06-05-persistence-supabase-sota-design.md`
  then fixed the *false*-stale root cause (Defect A: 6 pools × 5 conns → pooler
  exhaustion) with a shared self-reclaiming pool and graceful degradation, and is
  **already merged** to origin/main (verified: `backend/crates/sim-server/src/db.rs`
  with `statement_cache_capacity(0)`, `idle_timeout`, `max_lifetime`,
  `ABUTOWN_DB_MAX_CONNECTIONS`; `persistence_liveness.rs` `PERSIST_FAILURE_TOLERANCE=2`;
  `app/mod.rs` `health.ok` false **only** for `Stale`; `backendGate.ts` accepts
  `degraded`). The SOTA spec's **only open item is operator config**: "flip `.env`
  `DATABASE_URL` to the `:6543` transaction-pooler endpoint." Verified the live
  `.env` is still on `:5432` (session pooler) — the exact root cause. **So the gate
  needs no code; it needs the spec's documented config flip + a fresh (small) seed.**
- **Pricing (Blocker-2):** `2026-06-04-economy-free-prices-design.md` guarantees only
  *scarcity-responsive, damped, bounded-in-[floor,ceiling], clearable* reservation
  prices, and is **explicit** that it does **not** guarantee convergence for
  one-sided source↔sink markets ("Gemessen, nicht garantiert… Kein Over-Claim";
  full LoOP convergence + recovery are **deferred**, §2/§11). A *starved* market
  ratcheting to the ceiling stays bounded-in-band — within the spec's honest scope.
  Its Test #10 *does* guarantee the **self-sustaining** steady state stays "lebend &
  beschränkt" (alive & bounded). Market 9002 is **supplied** by 9001 (net_gain
  ≈ +650/unit), so on a fresh seed it should consume and stay in-band.

## Design

### 1. Gate (Blocker-3) — operational, no code

Flip `.env` `DATABASE_URL` from the `:5432` session pooler to the `:6543`
transaction pooler (same host/user, port only). The shared pool + `statement_cache_capacity(0)`
(already shipped) make this pooler-mode the SOTA-correct target. No code change —
the SOTA spec is implemented; adding gate code would duplicate a shipped spec.

### 2. Fresh seed — operational (authorized)

`DELETE FROM economy_snapshots` + `DELETE FROM mobility_snapshots` for the abutopia
world. On the next boot the idempotent seed re-applies markets (9002 on the
corridor, from Blocker-1's `markets.json`) + re-binds citizens, at opening prices
(1000) — clearing the degenerate price-ceiling state. The small fresh payload also
keeps snapshot writes fast (reinforcing Healthy persistence). No schema/serde
change.

### 3. Pricing-stability evidence test (the one shippable code artifact)

A deterministic backend test (`backend/crates/sim-core/src/economy/tests/`) that:
- builds the abutopia economy from the real bundle (the `seed_world` pattern) +
  runs the full economy schedule for a long run (≥ ~1000 ticks ⇒ ≥ ~100 tâtonnement
  cadences at `macro_flow_interval_ticks = 10`),
- asserts every tick: (a) `total_money` byte-invariant; (b) every
  `MarketGoodState.ewma_reference_price` ∈ `[price_floor, price_ceiling]`,
- asserts for the **supplied** demand market 9002: its consumption is **sustained**
  (`consumed_qty_last_tick > 0` in the run's latter portion, not collapsed to 0) and
  its `ewma_reference_price` does **not** pin at `price_ceiling`.

This extends the free-prices spec's Test #10 to the long-run abutopia scenario.

**Branch on the result:**
- **PASS** (9002 stays healthy): the ceiling-pinning is the spec's *already-deferred*
  one-sided/starved case. File it as a follow-up under the spec's deferred LoOP /
  recovery item. **No pricing code change** — gemäss specs.
- **FAIL** (9002 collapses: consumption → 0 and/or price pins at ceiling): this
  contradicts the free-prices spec's stability guarantee (Test #10) = a **genuine
  bug**. Escalate to a separate recovery-fix slice with its own brainstorm/spec
  (e.g. ceiling-relief or consumption-affordability dynamic). Do **not** patch it
  ad-hoc inside this slice.

### 4. Live demo (acceptance)

Restart the dev stack on the fresh-seeded world (`:6543`); drive a headless
browser-smoke; capture a screenshot showing citizens clustered at / heading to
market 9002 on the corridor; read the `economy::liveness` routed count from the
backend log. The user's "looks alive" judgment is the acceptance.

## Determinism, persistence, performance

- The stability test is deterministic (BTreeMap/sorted, no RNG/wall-clock; fixed
  `capita_baseline`), runs in CI, reuses existing harness patterns.
- No DB migration (the fresh seed is operational; no schema/serde change). The gate
  change is `.env` config only.
- The long-run test is `O(sectors × ticks)`, bounded; no population dependence.

## Acceptance Criteria

- `.env` `DATABASE_URL` is on the `:6543` transaction pooler; the abutopia world is
  freshly seeded; the backend reports persistence `Healthy` and the frontend renders
  (no "persistence stale" takeover).
- The deterministic long-run pricing-stability test exists, runs in CI, and either
  PASSES (9002 stays in-band + consuming) or FAILS with a clear signal that becomes
  the trigger for a separate recovery-fix slice.
- Live: the running abutopia stack shows citizens economically routed to 9002 on the
  corridor (screenshot + `economy::liveness` routed count); the user confirms it
  looks alive.
- No new gate code; no ad-hoc pricing code; full CI gate green incl. browser-smoke.

## Non-Goals

- No new persistence-gate code (the SOTA spec is implemented; only its operator
  config remains).
- No ad-hoc pricing/recovery mechanism — that is gated on the evidence test and, if
  warranted, becomes its own spec'd slice (the free-prices spec explicitly deferred
  it).
- No move off Supabase; no schema migration; no economy/render code change.

## Risks & Mitigations

- **`:6543` flip insufficient / writes still slow:** if persistence still goes
  `Stale` after `:6543` + fresh seed, that is a *genuine* persistence problem the
  spec's fail-closed gate is *correctly* surfacing — investigate the write path /
  pooler ceiling (`ABUTOWN_DB_MAX_CONNECTIONS`) rather than weakening the gate.
- **Stability test flakiness / runtime:** keep the run bounded (~1000–2000 ticks),
  assert sustained consumption over a window (not a single tick) to avoid warm-up
  flutter; reuse the proven capita long-run loop.
- **Live demo still blocked by remote latency:** the deterministic test is the
  rigorous evidence; the live demo is acceptance. If the remote stack remains flaky
  despite `:6543`, the in-repo proof still stands and the demo can be retried.
- **Evidence shows a real bug:** then the pricing fix is a *separate* slice — do not
  scope-creep it into this one.

## Deferred

- The pricing recovery/LoOP-convergence mechanism (only if the evidence test shows a
  supplied-market collapse) — its own brainstorm → spec → slice.
- Per-chunk mobility snapshots (the persistence-liveness spec's own follow-up).
