//! `EconomyConfig` — harvested from the head of sim-core's economy/systems.rs
//! (bbd0159). The schedule/system wrappers of systems.rs are deliberately NOT
//! harvested (Task 6 rebuilds a leaner chain without Attribution/Materialize/
//! LOD); the config struct is a hard dependency of the pure economy core.

use bevy_ecs::prelude::*;

use crate::econ::{Money, SettlementPolicy};

#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct EconomyConfig {
    pub ewma_alpha_bps: u16,
    pub default_order_ttl_ticks: u64,
    pub transport_cost_per_tile_unit: Money,
    pub trader_tiles_per_tick: u64,
    pub trader_default_ref_price: Money,
    pub macro_flow_interval_ticks: u64,
    pub settlement_policy: SettlementPolicy,
    /// How many consumed-good units one attributed shopper-role citizen represents
    /// (the divisor in attribution's per-market cohort size).
    pub shoppers_per_unit: i64,
    /// Per-market BASELINE cap on attributed shopper-role citizens. Since 2d the
    /// EFFECTIVE cap is `max_shoppers_per_market * CapitaFactor` (scales with the live
    /// population), so visible density grows with the citizenry. Still derived from the
    /// POPULATION factor, never from the consumption magnitude (viewport-independent).
    pub max_shoppers_per_market: usize,
    /// When TRUE, the macro flow drains active/observed markets' post-auction
    /// residual orders into the inter-market flow (S3). FALSE keeps the flow
    /// dormant-only (S1/S2 land dark). Defaulted FALSE; S3 flips it.
    pub drain_active_residual: bool,
    /// Labor share of value added (basis points, 0..=10_000). Default 6_000 = 0.60
    /// (Kaldor stylized fact). VALIDATED `0..=10_000` so `wage <= revenue` ⇒ no overdraft.
    pub labor_share_bps: u16,
    /// How many wage-Money units one attributed commuter-role citizen represents
    /// (the divisor in attribution's per-market wage cohort size).
    pub commuters_per_wage_unit: i64,
    /// Per-market BASELINE cap on attributed commuter-role citizens. Since 2d the
    /// EFFECTIVE cap is `max_commuters_per_market * CapitaFactor` (scales with the live
    /// population). Still derived from the POPULATION factor, NEVER from the wage
    /// magnitude (viewport-independent — observation can't widen it).
    pub max_commuters_per_market: usize,
    /// Share of firm PROFIT (revenue − wage) distributed to labor households (basis
    /// points, 0..=10_000). Default 10_000 = full distribution: firms net to zero each
    /// tick (no retained earnings, no capitalist class — lead decision). A value < 10_000
    /// would strand profit in firm accounts and the loop would NOT be self-sustaining.
    pub dividend_share_bps: u16,
    /// Tâtonnement gain (basis points) applied to the normalized excess-demand intensity
    /// when nudging reservation prices. Default 500 = 5%. VALIDATED `0..=10_000`.
    pub price_adjust_k_bps: u16,
    /// Hard per-interval speed limit on a reservation-price move (basis points of the
    /// current price). Default 100 = 1%/interval — the load-bearing anti-oscillation guard.
    /// VALIDATED `0..=10_000`.
    pub price_adjust_max_step_bps: u16,
    /// Absolute lower guardrail for any reservation price (MUST be > 0 so a price never
    /// reaches 0 and trips ZeroPrice). Default Money(1).
    pub price_floor: Money,
    /// Absolute upper guardrail for any reservation price. Default Money(100_000).
    pub price_ceiling: Money,
    /// Per-capita scaling baseline: `capita_factor = max(1, live_count / capita_baseline)`,
    /// recomputed each tick by `refresh_capita_factor_system` from the live `AgentMarker`
    /// citizen count. Default 1_000_000 keeps the factor at 1 (identity) at the ~300-citizen
    /// seed scale; LOWER it to ramp throughput up (e.g. 10 -> ~30x at 300 citizens). Raising
    /// it above the default is a no-op at seed scale (factor stays clamped at 1).
    pub capita_baseline: i64,
}

impl EconomyConfig {
    /// `labor_share_bps` as an i128, refusing `> 10_000` (a config bug that would
    /// over-pay). Exposed for the pure `run_pay_wages_at_tick` core. Boundary
    /// `== 10_000` is allowed (full labor share).
    pub fn validated_labor_share_bps(&self) -> Result<i128, crate::econ::EconomyError> {
        if self.labor_share_bps > 10_000 {
            return Err(crate::econ::EconomyError::InvalidOrder);
        }
        Ok(self.labor_share_bps as i128)
    }

    /// `dividend_share_bps` as an i128, refusing `> 10_000` (a config bug that would
    /// over-distribute). Boundary `== 10_000` allowed (full distribution). Mirrors
    /// `validated_labor_share_bps`.
    pub fn validated_dividend_share_bps(&self) -> Result<i128, crate::econ::EconomyError> {
        if self.dividend_share_bps > 10_000 {
            return Err(crate::econ::EconomyError::InvalidOrder);
        }
        Ok(self.dividend_share_bps as i128)
    }

    /// `price_adjust_k_bps` as i128, refusing `> 10_000`. Boundary `== 10_000` allowed.
    pub fn validated_price_adjust_k_bps(&self) -> Result<i128, crate::econ::EconomyError> {
        if self.price_adjust_k_bps > 10_000 {
            return Err(crate::econ::EconomyError::InvalidOrder);
        }
        Ok(self.price_adjust_k_bps as i128)
    }

    /// `price_adjust_max_step_bps` as i128, refusing `> 10_000`.
    pub fn validated_price_adjust_max_step_bps(&self) -> Result<i128, crate::econ::EconomyError> {
        if self.price_adjust_max_step_bps > 10_000 {
            return Err(crate::econ::EconomyError::InvalidOrder);
        }
        Ok(self.price_adjust_max_step_bps as i128)
    }

    /// `(price_floor, price_ceiling)` as i64s, refusing `floor <= 0` or `floor >= ceiling`
    /// (a config bug that would allow a 0/negative price or an empty guardrail band).
    pub fn validated_price_band(&self) -> Result<(i64, i64), crate::econ::EconomyError> {
        if self.price_floor.0 <= 0 || self.price_floor.0 >= self.price_ceiling.0 {
            return Err(crate::econ::EconomyError::InvalidOrder);
        }
        Ok((self.price_floor.0, self.price_ceiling.0))
    }
}

impl Default for EconomyConfig {
    fn default() -> Self {
        Self {
            ewma_alpha_bps: 2_000,
            default_order_ttl_ticks: 10,
            transport_cost_per_tile_unit: Money(5),
            trader_tiles_per_tick: 4,
            trader_default_ref_price: Money(1_000),
            macro_flow_interval_ticks: 10,
            settlement_policy: SettlementPolicy::Anchored,
            shoppers_per_unit: 3,
            max_shoppers_per_market: 4,
            drain_active_residual: true,
            labor_share_bps: 6_000,
            commuters_per_wage_unit: 100,
            max_commuters_per_market: 4,
            dividend_share_bps: 10_000,
            price_adjust_k_bps: 500,
            price_adjust_max_step_bps: 100,
            price_floor: Money(1),
            price_ceiling: Money(100_000),
            capita_baseline: crate::econ::capita::CAPITA_BASELINE_IDENTITY,
        }
    }
}
