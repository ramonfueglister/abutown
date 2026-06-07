# Economy: One-Sided Price Convergence via Flow-Margin Feedback — Design Spec

Date: 2026-06-07

## Status

Approved from brainstorm, **literature-grounded** (see References). Implements the
deferred item of the free-prices spec (`2026-06-04-economy-free-prices-design.md`
§2, §11): "volle räumliche Law-of-One-Price-Konvergenz für einseitige Quelle↔Senke
-Paare via Flow-Margin-Feedback." Base: `feat/abutopia-live-visible` off
`origin/main` (`53cd2e3`). Regression target: `economy/tests/abutopia_price_stability.rs`.

## Problem (confirmed by instrumentation)

A **demand-only "sink" market** (abutopia 9002, fed FOOD/TOOLS from source 9001 over
a transport edge) has its bucket price pinned to `bid_ceiling = DemandPool.max_price`
(`macro_flow.rs` `synthetic_price`, demand-only branch). The flow writes that price
into the sink's `last_settlement_price` → `ewma_reference_price` tracks it → the
consumer's `desired_qty = spend / ewma` stays large. The existing reservation-price
tâtonnement sees, at the sink, only excess demand (`unmet − unsold` with `unsold = 0`),
so it nudges `max_price` **up +1 %/cadence to the ceiling with no downward force.**
Measured trajectory: `max_price 2000→4333→…→99961` over ~2000 ticks, `unmet ≈ 2500`
persistently, while goods *do* flow and consume (~20/cadence). The recorded price
diverges from the **true landed cost** the buyer actually pays (`p_src + transport`).

This is **not an implementation bug**: a purely *local*, single-market excess-demand
law is *under-determined* for a sink — local excess demand carries no information about
the source price or the edge cost, so the sink price has no anchor (Samuelson, 1947;
the canonical remedy is an inter-market spatial-arbitrage term — Samuelson, 1952).

## Theoretical grounding

Two distinct canonical theories, deliberately combined:

1. **Walrasian–Samuelson tâtonnement** (the *existing* local term). Prices adjust in
   the direction of excess demand, `dp_i/dt = k_i · Z_i(p)`, `k_i > 0` (Walras,
   1874/1954; Samuelson, 1941, 1947). The damped, speed-limited (±1 %/cadence),
   clamped discrete form is the recognized way to keep the discrete map
   `p_{t+1} = p_t + k·Z` out of the overshoot/chaos regime (Bala & Majumdar, 1992).
   Global stability is *conditional* (gross substitutes — Arrow & Hurwicz, 1958;
   Arrow, Block & Hurwicz, 1959) and not unconditional (Scarf, 1960).

2. **Spatial price equilibrium / Law of One Price under transport** (the *new* term).
   For source `i`, sink `j`, unit transport `c_ij`, shipment `Q_ij ≥ 0`, equilibrium is
   the **complementarity** condition (Enke, 1951; Samuelson, 1952; Takayama & Judge,
   1971):

   ```
   π_i + c_ij = ρ_j   if Q_ij > 0       (active route → price gap equals transport)
   π_i + c_ij ≥ ρ_j   if Q_ij = 0       (dormant route → no-trade band, gap ≤ c_ij)
   ```

   i.e. `0 ≤ Q_ij ⊥ (π_i + c_ij − ρ_j) ≥ 0`. The spatial Law of One Price
   `|ρ_j − π_i| ≤ c_ij` is the aggregate consequence.

3. **Framing.** Conservation follows Godley & Lavoie (2007) stock-flow consistency —
   prices are *multipliers, not money*; the per-tick byte-exact `total_money` audit
   (`#78`) is the SFC "no black holes" invariant operationalized. The sector
   abstraction is a **mean-field representation** in the spirit of mean-field models
   (Lasry & Lions, 2007) — a closure, *not* a solved MFG Nash fixpoint (no over-claim).

## Mechanism (corrected per the complementarity literature)

A new pure function adds the missing **inter-market arbitrage term**. It is the
*price-space, reduced-form* dual of the projected-dynamical-system that drives flows
by the arbitrage margin (Nagurney, Takayama & Zhang, 1995); we nudge prices directly,
which is a legitimate mean-field shortcut to the same monotone fixpoint, cited as such.

Per cadence boundary tick (`tick % macro_flow_interval_ticks == 0`), after
`MacroFlow` + `Telemetry`, **for each cross-market edge `S→D` per good that carried
positive realized flow this cadence** (the complementarity gate):

- `t = transport_cost(dist(S,D), 1 unit, rate)` (per-unit; `dist` from `MarketDistances`).
- Sink target `ρ*_D = S.last_settlement_price + t`; source target `π*_S = D.last_settlement_price − t`.
  Skip the edge if `S.last_settlement_price ≤ 0` (source has not cleared — no data, no action).
- Nudge `D`'s `DemandPool.max_price` toward `ρ*_D` and `S`'s `SupplyPool.min_price`
  toward `π*_S`, each by a damped, signed, **speed-limited** step reusing the existing
  `nudge` discipline: `step_bps = clamp(k_bps · normalized_gap / 10_000, ±max_step_bps)`,
  then clamp the result to `[price_floor, price_ceiling]`. The signed gap supplies
  **both** directions — above `p_S + t` it pulls **down** (the missing recovery force),
  below it pulls up — so the fixpoint is exactly `ρ_D − π_S = t`.

**Coexistence rule (the key correction).** A `(market, good)` pool that is on an
**active flow edge this cadence** is governed by the **margin term** (it anchors the
price to LoOP); the **local-unmet tâtonnement is NOT also applied to that pool** this
cadence — otherwise the sink's perpetual local "+1 %" fights the anchor and the two
capped steps can cancel and stall. A `(market, good)` with **no active flow edge**
(autarkic / two-sided local market) keeps the existing local-unmet tâtonnement
unchanged. This (a) respects complementarity (no forced equality on dormant edges),
(b) preserves the spec's guaranteed scarcity response for route-less markets, and
(c) prevents the anchor/scarcity fight at the sink.

## Insertion / conservation / determinism

- **File:** extend `economy/pricing.rs` with `nudge_price_toward_target(...)` (pure)
  and `run_flow_margin_feedback_at_tick(demand, supply, market_goods, distances,
  active_flows, config) -> Result<(), EconomyError>`. The local-unmet pass and the
  margin pass are reconciled by the coexistence rule above (one nudge per pool/good
  per cadence).
- **Active-flow signal:** `MacroFlow` already computes realized flows; record the set
  of `(src, dst, good)` with `q > 0` this cadence into a small resource the price pass
  reads (no new persisted field). The plan fixes the exact carrier.
- **Schedule:** same `EconomySet::AdjustReservationPrices`, same cadence, same slot
  (after `Telemetry`, before `UpdateConsumption`). Add `Res<MarketDistances>` (already
  a resource). **No new persisted snapshot field → no `DELETE FROM economy_snapshots`.**
- **Conservation (SFC):** writes only i64 `max_price`/`min_price`; reads
  `last_settlement_price`, `MarketDistances`, the active-flow set. Moves no money →
  `total_money` byte-invariant (#78 unaffected). Prices are order-parameters.
- **Determinism:** keys-first over `MarketDistances`/pools, i128 intermediates, floor
  division, clamp. Byte-identical across runs + persist/restore.
- **Stability discipline:** reuse `k_bps`/`max_step_bps` (±1 %); the coupled
  (local + spatial) system is *not* covered by single-market gross-substitute proofs,
  so keep `k` small, keep the speed limit on the new term, and treat the #78 audit +
  the long-run steady-state test as the safety canaries (Scarf, 1960; Bala &
  Majumdar, 1992 cautions apply at the network level).

## Why this fixes 9002

The margin term anchors `9002.max_price` to `p_9001 + transport ≈ p_9001 + 860`
(≈ 1360–1860), comfortably **below** the consumer `max_price` 2000 and far below the
100 000 ceiling. The ratchet halts; the recorded/ewma price stabilizes at LoOP; the
consumer's `desired_qty` becomes sane; consumption is steady at the sane price → the
shop channel routes citizens → the merge is visible. **Honest scope:** this converges
the *price* to LoOP; it does **not** raise the supply *quantity* — 9002 remains
supply-constrained (~10/tick at factor 1), which is *legitimate scarcity at a correct
price*, not the ratchet bug (a separate calibration concern, out of scope).

## Testing

- **Pure unit tests** for `nudge_price_toward_target`: signed gap → correct direction
  (above target pulls down, below pulls up); speed-limited to ±`max_step_bps`; clamped
  to `[floor, ceiling]`; `min < max` preserved; deterministic (keys-first); skip on
  `src.last_settlement_price ≤ 0`.
- **Complementarity test:** an edge with **zero** realized flow gets **no** margin
  nudge (no forced equality on a dormant route).
- **Conservation:** `total_money` byte-invariant over N ticks with the new term active.
- **Convergence (the regression):** extend `economy/tests/abutopia_price_stability.rs`
  — over a long run (~2000 ticks) 9002's `ewma_reference_price` converges to ≈
  `p_9001 + transport` (in-band, well below ceiling, e.g. `< ceiling/10`) and
  consumption is sustained (the currently-failing test now passes).
- **Non-destabilization:** the existing self-sustaining steady state stays alive +
  bounded with the new term (no network oscillation).
- Full CI gate incl. browser-smoke (no wire change, but run it).

## Acceptance Criteria

- The flow-margin feedback exists, is complementarity-gated (active flow only),
  margin-anchored (no local/margin fight at sinks), conservation-exact, deterministic.
- The abutopia long-run test passes: 9002 converges to ≈ source+transport, in-band,
  sustained consumption — the ratchet is gone.
- Existing economy suites stay green; no migration.
- The spec carries verifiable APA7 references (below); the implementation comments
  cite the governing condition (spatial LoOP complementarity).

## Non-Goals / Deferred

- **Multi-source sinks** (`ρ_D = min_k(π_k + t_k)` for the cheapest active source) —
  abutopia is single-source; defer aggregation.
- **Flow-dependent (congestion) transport** `c_ij(Q)` — `t = rate×dist` is fixed here.
- **Supply-quantity calibration** (the 9002 demand≫supply imbalance) — legitimate
  scarcity at a now-correct price; separate slice if desired.
- The literal NTZ flow-driven projected dynamical system — we use its price-space
  mean-field reduction.

## Risks & Mitigations

- **Coupled-system instability / oscillation** (Scarf, 1960; Bala & Majumdar, 1992):
  keep `k_bps` small, share the ±1 % speed limit, gate on cadence; the steady-state +
  conservation tests are the canaries. If oscillation appears, lower `k`/`max_step`.
- **Over-constraining dormant edges:** the complementarity gate (active-flow only)
  prevents forcing `p_D = p_S + t` where no trade flows.
- **Anchor/scarcity fight at sinks:** the coexistence rule (margin governs flow-coupled
  pools; local governs autarkic) prevents the cancel-stall.
- **Source not yet cleared:** skip edges with `S.last_settlement_price ≤ 0` (no data).
- **Over-claiming:** we cite mean-field/SFC as framing, not as solved MFG/full SFC
  models; we converge the price, not the supply quantity — stated honestly.

## References (APA7)

- Arrow, K. J., & Hurwicz, L. (1958). On the stability of the competitive equilibrium, I. *Econometrica, 26*(4), 522–552. https://doi.org/10.2307/1907515
- Arrow, K. J., Block, H. D., & Hurwicz, L. (1959). On the stability of the competitive equilibrium, II. *Econometrica, 27*(1), 82–109. https://doi.org/10.2307/1907779
- Bala, V., & Majumdar, M. (1992). Chaotic tâtonnement. *Economic Theory, 2*(4), 437–445. https://doi.org/10.1007/BF01212469
- Copeland, M. A. (1949). Social accounting for moneyflows. *The Accounting Review, 24*(3), 254–264.
- Enke, S. (1951). Equilibrium among spatially separated markets: Solution by electric analogue. *Econometrica, 19*(1), 40–47. https://doi.org/10.2307/1907907
- Godley, W., & Lavoie, M. (2007). *Monetary economics: An integrated approach to credit, money, income, production and wealth*. Palgrave Macmillan.
- Lasry, J.-M., & Lions, P.-L. (2007). Mean field games. *Japanese Journal of Mathematics, 2*(1), 229–260. https://doi.org/10.1007/s11537-007-0657-8
- Nagurney, A., Takayama, T., & Zhang, D. (1995). Massively parallel computation of spatial price equilibrium problems as dynamical systems. *Journal of Economic Dynamics and Control, 19*(1–2), 3–37. https://doi.org/10.1016/0165-1889(93)00800-Z
- Samuelson, P. A. (1941). The stability of equilibrium: Comparative statics and dynamics. *Econometrica, 9*(2), 97–120. https://doi.org/10.2307/1906872
- Samuelson, P. A. (1947). *Foundations of economic analysis*. Harvard University Press.
- Samuelson, P. A. (1952). Spatial price equilibrium and linear programming. *The American Economic Review, 42*(3), 283–303.
- Scarf, H. (1960). Some examples of global instability of the competitive equilibrium. *International Economic Review, 1*(3), 157–172. https://doi.org/10.2307/2556215
- Takayama, T., & Judge, G. G. (1971). *Spatial and temporal price and allocation models*. North-Holland.
- Walras, L. (1954). *Elements of pure economics, or the theory of social wealth* (W. Jaffé, Trans.). George Allen & Unwin. (Original work published 1874)
