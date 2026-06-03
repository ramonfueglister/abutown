//! The SFC wage / income side of the economy: per-tick seller revenue capture
//! (`SellerReceipts`), the household clearing sentinel (`HOUSEHOLD_SECTOR`), and
//! (added later) the conservative two-leg wage transfer. Money is byte-invariant:
//! every move is an `AccountBook::transfer`; the wage sentinel nets to zero each tick.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

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

/// Wage Money paid per MARKET this tick (the commuter-projection driver). Ephemeral,
/// NOT persisted, reset-all-then-accumulate by `run_pay_wages_at_tick`. NOT on
/// `MarketGoodState` (avoids the constructor fan-out + an extra DELETE).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct WageTelemetry(pub BTreeMap<MarketId, Money>);

/// The mean-field household sector. PERSISTED. `population` parametrizes the sector
/// budget arithmetically (never materializes per-person accounts — the loop is
/// O(firms + pools)). `pool_weights` is the largest-remainder split of the wage bill
/// across consumer pools (equal weights v0). At least one weight MUST be positive
/// (seed assert), else the wage bill would strand in HOUSEHOLD_SECTOR.
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

/// The SFC wage step. Pure over its refs (no `World`). For each `(firm, market)` in
/// `receipts` (keys-first → ascending), pays `wage = floor(revenue * labor_share / 10_000)`
/// from the firm into `HOUSEHOLD_SECTOR` via `transfer` (two-leg, conservative); the
/// wage bill is summed ONLY from transfers that actually succeeded. Then largest-remainder
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
) -> Result<(), EconomyError> {
    for pool in demand.0.values_mut() {
        pool.income_last_tick = Money::ZERO;
    }
    wage_telemetry.0.clear();

    let labor_share = config.validated_labor_share_bps()?;

    let pool_ids: Vec<EconomicActorId> = demand.0.keys().copied().collect();
    let weights: Vec<i64> = pool_ids
        .iter()
        .map(|a| household.pool_weights.get(a).copied().unwrap_or(0))
        .collect();
    let weight_sum: i128 = weights.iter().map(|w| *w as i128).sum();

    // FIRST LEG: firms → HOUSEHOLD_SECTOR. Skipped entirely when there is no payout
    // target (Σweights==0), so the bill never strands in the sentinel.
    let mut wage_bill: i64 = 0;
    if weight_sum > 0 {
        for (&(firm, market), &revenue) in receipts.0.iter() {
            let wage = wage_for_revenue(revenue, labor_share)?;
            if wage.0 <= 0 {
                continue;
            }
            // `wage <= revenue`: the firm was credited `revenue` as a SELLER this tick
            // (ClearMarkets/MacroFlow run before PayWages) and spends nothing before
            // wages settle, so it always holds the cash. The transfer is therefore
            // total-conserving and cannot fault; `?` surfaces a genuine bug rather than
            // papering over an unreachable state.
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
        for (idx, actor) in pool_ids.iter().enumerate() {
            let share = Money(splits[idx]);
            if share.0 <= 0 {
                continue;
            }
            accounts.transfer(HOUSEHOLD_SECTOR, *actor, share)?;
            if let Some(pool) = demand.0.get_mut(actor) {
                pool.income_last_tick = pool.income_last_tick.checked_add(share)?;
            }
        }
    }

    debug_assert_eq!(
        accounts.account(HOUSEHOLD_SECTOR).available,
        Money::ZERO,
        "HOUSEHOLD_SECTOR must net to zero after PayWages (sentinel-stranded cash)"
    );
    Ok(())
}
