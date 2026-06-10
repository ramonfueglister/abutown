//! The SFC wage / income side of the economy: per-tick seller revenue capture
//! (`SellerReceipts`), the household clearing sentinel (`HOUSEHOLD_SECTOR`), and
//! (added later) the conservative two-leg wage transfer. Money is byte-invariant:
//! every move is an `AccountBook::transfer`; the wage sentinel nets to zero each tick.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::economy::producers::{InputPools, ProducerPolicies, wc_target};
use crate::economy::{
    AccountBook, DemandPools, EconomicActorId, EconomyConfig, EconomyError, EconomyEvent, MarketId,
    Money, TradeLedger, apportion_cash,
};

/// Reserved clearing-account sentinel for the household sector, adjacent to
/// `TRANSPORT_OPERATOR = EconomicActorId(u64::MAX)`. Firms pay wages INTO this
/// account; it is fully apportioned out to consumer pools in the same tick, so
/// it nets to ZERO every PayWages (asserted in debug). Distinct from every
/// seeded id (8_001..8_022) and the actor-offset bands (`n << 32`).
pub const HOUSEHOLD_SECTOR: EconomicActorId = EconomicActorId(u64::MAX - 1);

/// Gross sales revenue credited to each `(firm, market)` THIS tick. A non-monetary
/// running statistic (NOT a money store), zeroed at the very start of every tick
/// (`EconomySet::ResetReceipts`) and NEVER persisted. The `(actor, market)` key
/// carries the market dimension for commuter attribution. Captured at the settle
/// points where seller id + market + amount are all in scope (auction + macro flow),
/// so it is coherent with the money move: a fault that discards the settle clone
/// discards its receipts too.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct SellerReceipts(pub BTreeMap<(EconomicActorId, MarketId), Money>);

/// Buyer-side mirror of `SellerReceipts`: gross purchase charges debited from each
/// `(buyer, market)` THIS tick (auction: actual cost; macro flow: full charge INCLUDING
/// the transport premium — so chain input costs are transport-inclusive by construction).
/// Captured UNCONDITIONALLY for every buyer (consumer outlays are a harmless unused
/// statistic; only the PayWages join reads this). Non-monetary statistic, zeroed in
/// `EconomySet::ResetReceipts`, NEVER persisted. Captured in the settle scratch zone,
/// so a discarded settle discards its outlays too.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct BuyerOutlays(pub BTreeMap<(EconomicActorId, MarketId), Money>);

/// Wage Money paid per MARKET this tick (the commuter-projection driver). Ephemeral,
/// NOT persisted, reset-all-then-accumulate by `run_pay_wages_at_tick`. NOT on
/// `MarketGoodState` (avoids the constructor fan-out + an extra DELETE).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct WageTelemetry(pub BTreeMap<MarketId, Money>);

/// The mean-field household sector. PERSISTED. The mean-field design keeps the wage
/// authority O(firms + pools) regardless of how many people the sector represents —
/// it never materializes per-person accounts. `pool_weights` is the largest-remainder
/// split of the wage bill across consumer pools (equal weights v0); at least one weight
/// MUST be positive (seed assert), else the wage bill would strand in HOUSEHOLD_SECTOR.
/// `population` is the head-count this sector stands for; it is carried as persisted
/// state but does NOT yet enter any computation (the v0 wage bill is purely the
/// labor-share of firm revenue, headcount-independent) — it is reserved for the
/// per-capita consumption-scaling slice and is intentionally inert until then.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct HouseholdSector {
    pub population: u64,
    pub pool_weights: BTreeMap<EconomicActorId, i64>,
}

/// `wage = floor(revenue * labor_share_bps / 10_000)`. `labor_share_bps <= 10_000`
/// (validated by the caller) ⇒ `wage <= revenue` ⇒ no overdraft. Floor leaves the
/// rounding remainder at the firm (never minted). i128 intermediate, `try_from` → Overflow.
pub(crate) fn wage_for_revenue(
    revenue: Money,
    labor_share_bps: i128,
) -> Result<Money, EconomyError> {
    let raw = (revenue.0 as i128) * labor_share_bps / 10_000;
    Ok(Money(
        i64::try_from(raw).map_err(|_| EconomyError::Overflow)?,
    ))
}

/// Value added for one `(firm, market)`: revenue minus this tick's buyer outlays,
/// floored at zero (a buy-heavy tick pays zero wage, never a negative transfer).
/// Missing outlay key → `spent = 0` → value_added == revenue (extractor / pure-seller
/// regression safety). `checked_sub` surfaces a genuine i64 arithmetic overflow (only
/// possible for extreme edge values, not for normal positive money amounts).
pub(crate) fn value_added_for(
    revenue: Money,
    outlays: &BuyerOutlays,
    firm: EconomicActorId,
    market: MarketId,
) -> Result<Money, EconomyError> {
    let spent = outlays
        .0
        .get(&(firm, market))
        .copied()
        .unwrap_or(Money::ZERO);
    let raw = revenue.checked_sub(spent)?;
    Ok(Money(raw.0.max(0)))
}

/// The SFC wage step. Pure over its refs (no `World`). For each `(firm, market)` in
/// `receipts` (keys-first → ascending), pays a wage on VALUE ADDED (revenue minus this
/// tick's buyer outlays for the same `(firm, market)`, floored at zero) rather than on
/// gross revenue: `wage = floor(value_added * labor_share / 10_000)`. Firms that bought
/// inputs pay wages only on the margin they created; extractors and pure sellers (no
/// outlay entry) are unchanged (`value_added == revenue`). The wage is transferred from
/// the firm into `HOUSEHOLD_SECTOR` via `transfer` (two-leg, conservative); the wage
/// bill is summed ONLY from transfers that actually succeeded. Then largest-remainder
/// splits the wage bill across consumer pools (`pool_weights`, ties-by-ascending-index)
/// and transfers each share `HOUSEHOLD_SECTOR → consumer`, crediting `income_last_tick`
/// from the COMPLETED `to`-side. Resets `income_last_tick` (keys-first) and `WageTelemetry`
/// first. Conservation: `total_money` byte-invariant (only transfers); `HOUSEHOLD_SECTOR`
/// nets to zero (`apportion_cash` is exactly sum-preserving when Σweights>0). When Σweights==0
/// BOTH legs are skipped so nothing strands.
#[allow(clippy::too_many_arguments)]
pub fn run_pay_wages_at_tick(
    accounts: &mut AccountBook,
    receipts: &SellerReceipts,
    demand: &mut DemandPools,
    household: &HouseholdSector,
    wage_telemetry: &mut WageTelemetry,
    ledger: &mut TradeLedger,
    config: &EconomyConfig,
    outlays: &BuyerOutlays,
) -> Result<(), EconomyError> {
    for pool in demand.0.values_mut() {
        pool.income_last_tick = Money::ZERO;
    }
    wage_telemetry.0.clear();

    let labor_share = config.validated_labor_share_bps()?;

    // Payees and weights come straight from the household sector's weight map — the
    // authoritative definition of who is paid and by how much. No Option-defaulting.
    let payees: Vec<EconomicActorId> = household.pool_weights.keys().copied().collect();
    let weights: Vec<i64> = household.pool_weights.values().copied().collect();
    let weight_sum: i128 = weights.iter().map(|w| *w as i128).sum();

    // FIRST LEG: firms → HOUSEHOLD_SECTOR. Skipped entirely when there is no payout
    // target (Σweights==0), so the bill never strands in the sentinel.
    let mut wage_bill: i64 = 0;
    if weight_sum > 0 {
        for (&(firm, market), &revenue) in receipts.0.iter() {
            let value_added = value_added_for(revenue, outlays, firm, market)?;
            let wage = wage_for_revenue(value_added, labor_share)?;
            if wage.0 <= 0 {
                continue;
            }
            // `wage <= value_added = revenue − outlays`: the firm was credited `revenue`
            // as a SELLER this tick (ClearMarkets/MacroFlow run before PayWages) and its
            // same-tick purchases are already subtracted from the wage base, so even a
            // firm that bought inputs still holds at least `value_added >= wage` when
            // wages settle (balance = prior_cash + revenue − outlays >= value_added).
            // The transfer is therefore total-conserving and cannot fault; `?` surfaces
            // a genuine bug rather than papering over an unreachable state.
            accounts.transfer(firm, HOUSEHOLD_SECTOR, wage)?;
            wage_bill = wage_bill
                .checked_add(wage.0)
                .ok_or(EconomyError::Overflow)?;
            let slot = wage_telemetry.0.entry(market).or_insert(Money::ZERO);
            *slot = slot.checked_add(wage)?;
            ledger.0.push(EconomyEvent::WagePaid {
                firm,
                market,
                amount: wage,
            });
        }
    }

    // SECOND LEG: HOUSEHOLD_SECTOR → consumer pools (largest-remainder, sum-preserving).
    if wage_bill > 0 && weight_sum > 0 {
        let splits = apportion_cash(&weights, wage_bill);
        for (idx, actor) in payees.iter().enumerate() {
            let share = Money(splits[idx]);
            if share.0 <= 0 {
                continue;
            }
            // A weight entry MUST reference a real consumer pool (the seed builds
            // pool_weights FROM the demand pools). Surface a seed inconsistency loudly
            // rather than silently dropping the income, which would strand cash here.
            let pool = demand
                .0
                .get_mut(actor)
                .expect("pool_weights key must reference a seeded demand pool");
            accounts.transfer(HOUSEHOLD_SECTOR, *actor, share)?;
            pool.income_last_tick = pool.income_last_tick.checked_add(share)?;
        }
    }

    if accounts.account(HOUSEHOLD_SECTOR).available != Money::ZERO {
        // HOUSEHOLD_SECTOR must net to zero after PayWages — stranded sentinel cash is a
        // conservation violation. Release-grade (the wrapper .expect surfaces it fail-fast).
        return Err(EconomyError::ConservationViolation);
    }
    Ok(())
}

/// θ-dividend distribution to the labor households (no owner/capitalist class, v0),
/// capped by the producer's working-capital target.
///
/// For each `(firm, market)` in `receipts` (keys-first → ascending): recompute the wage
/// the SAME way `run_pay_wages_at_tick` did — on VALUE ADDED (`value_added_for`: revenue
/// minus this tick's buyer outlays for the same `(firm, market)`, floored at zero) with
/// identical flooring — so `profit = value_added − wage >= 0` (labor_share <= 10_000).
/// The payout share θ comes from the firm's `ProducerPolicy.theta_bps`, and the firm
/// retains a working-capital buffer `wc_target(policy, pool)` — the settle-value of
/// `batches_target` input batches at the pool's participation bound (Caiani-style
/// self-financing of next inputs): `intended = floor(profit·θ/10_000)`,
/// `covered = min(intended, max(held − wc_target, 0))`. Actors WITHOUT a policy
/// (no `ProducerPolicies` + `InputPools` entry) keep the #75 behavior byte-identically:
/// θ = config `dividend_share_bps` (default 10_000 → dividend == profit, the firm
/// drains to zero) and `wc_target = 0`.
///
/// SHORTFALL SEMANTICS (no-fallback discipline): withholding cash up to `wc_target` is
/// the WANTED liquidity buffer, not an anomaly — a policy firm's shortfall is by
/// construction always explained by it (when capped it retains exactly `wc_target`;
/// while still below the target it retains everything), so it is NEVER audited. Only a
/// NO-policy firm that holds `< intended` (it BOTH sold and bought via macro_flow this
/// tick) books just what it holds and pushes the audited
/// `MarketClearFailed { market, good=GoodId(0), reason: InsufficientFunds }` event —
/// the unchanged #75 audit. We do NOT `.expect` (latent process panic) and do NOT
/// silently skip (stranded profit, broken loop).
///
/// The covered amount flows firm → HOUSEHOLD_SECTOR → households via `apportion_cash`
/// over the SAME `pool_weights` as wages, crediting `income_last_tick` (ADD, not reset —
/// wages credited it first). Conservation: only `transfer`s ⇒ `total_money`
/// byte-invariant; own independent `HOUSEHOLD_SECTOR` net-zero check (release-grade).
#[allow(clippy::too_many_arguments)]
pub fn run_distribute_profit_at_tick(
    accounts: &mut AccountBook,
    receipts: &SellerReceipts,
    demand: &mut DemandPools,
    household: &HouseholdSector,
    ledger: &mut TradeLedger,
    config: &EconomyConfig,
    outlays: &BuyerOutlays,
    policies: &ProducerPolicies,
    input_pools: &InputPools,
) -> Result<(), EconomyError> {
    let labor_share = config.validated_labor_share_bps()?;
    let dividend_share = config.validated_dividend_share_bps()?;

    let payees: Vec<EconomicActorId> = household.pool_weights.keys().copied().collect();
    let weights: Vec<i64> = household.pool_weights.values().copied().collect();
    let weight_sum: i128 = weights.iter().map(|w| *w as i128).sum();

    for (&(firm, market), &revenue) in receipts.0.iter() {
        let value_added = value_added_for(revenue, outlays, firm, market)?;
        let wage = wage_for_revenue(value_added, labor_share)?;
        let profit = value_added.checked_sub(wage)?; // wage <= value_added ⇒ profit >= 0
        let (theta_bps, target, has_policy) =
            match (policies.0.get(&firm), input_pools.0.get(&firm)) {
                (Some(policy), Some(pool)) => (
                    i128::from(policy.theta_bps),
                    wc_target(*policy, pool)?,
                    true,
                ),
                // No policy: config-θ (default 10_000) and no buffer — the #75 path.
                _ => (dividend_share, Money::ZERO, false),
            };
        let intended = Money(
            i64::try_from((profit.0 as i128) * theta_bps / 10_000)
                .map_err(|_| EconomyError::Overflow)?,
        );
        if intended.0 <= 0 || weight_sum <= 0 {
            continue; // nothing to distribute, or no payout target
        }
        // Book only what the firm holds ABOVE its working-capital target.
        let held = accounts.account(firm).available;
        let distributable = Money(held.0.saturating_sub(target.0).max(0));
        let covered = Money(intended.0.min(distributable.0));
        if covered.0 < intended.0 && !has_policy {
            // Unexplained shortfall (the #75 audit): no policy, yet the firm cannot cover.
            ledger.0.push(EconomyEvent::MarketClearFailed {
                market,
                good: crate::economy::GoodId(0),
                reason: EconomyError::InsufficientFunds,
            });
        }
        if covered.0 <= 0 {
            continue;
        }
        // LEG 1: firm → HOUSEHOLD_SECTOR. `covered <= distributable <= held` ⇒ cannot fault.
        accounts.transfer(firm, HOUSEHOLD_SECTOR, covered)?;
        // LEG 2: HOUSEHOLD_SECTOR → households (largest-remainder, sum-preserving).
        let splits = apportion_cash(&weights, covered.0);
        for (idx, actor) in payees.iter().enumerate() {
            let share = Money(splits[idx]);
            if share.0 <= 0 {
                continue;
            }
            let pool = demand
                .0
                .get_mut(actor)
                .expect("pool_weights key must reference a seeded demand pool");
            accounts.transfer(HOUSEHOLD_SECTOR, *actor, share)?;
            pool.income_last_tick = pool.income_last_tick.checked_add(share)?;
        }
        ledger.0.push(EconomyEvent::ProfitDistributed {
            firm,
            market,
            amount: covered,
        });
    }

    if accounts.account(HOUSEHOLD_SECTOR).available != Money::ZERO {
        // HOUSEHOLD_SECTOR must net to zero after profit distribution — stranded sentinel cash
        // is a conservation violation. Surfaced fail-fast by the wrapper (see Step 5).
        return Err(EconomyError::ConservationViolation);
    }
    Ok(())
}
