use std::collections::BTreeSet;

use bevy_ecs::prelude::*;
use bevy_ecs::query::Or;

use crate::economy::production::RawDeposits;
use crate::economy::{
    AccountBook, BuyerOutlays, DemandPools, DirtyMarketGoods, DormantMarkets, EconomyError,
    EconomyEvent, FlowRateEwma, FlowShipmentParams, GoodId, HouseholdSector, InventoryBook,
    MarketChunks,
    MarketDistances, MarketGoods, MarketId, Money, NextOrderId, OrderBook, ProductionPools,
    SellerReceipts, SettlementPolicy, SupplyPools, TradeLedger, WageTelemetry,
    clear_market_good_with_receipts, expire_orders_at_tick, generate_pool_orders_at_tick,
    integer_ewma, run_consumption_at_tick, run_consumption_update_at_tick,
    run_distribute_profit_at_tick, run_macro_flow_at_tick, run_pay_wages_at_tick,
    run_production_at_tick, run_regen_at_tick, run_transport_rebate_at_tick,
};
use crate::ids::ChunkCoord;
use crate::mobility::resources::Tick;
use crate::world::components::{ActiveChunk, ChunkCoordComp, HotChunk};

#[derive(SystemSet, Hash, Eq, PartialEq, Debug, Clone)]
pub enum EconomySet {
    RefreshCapita,
    ResetReceipts,
    RefreshLod,
    ExpireOrders,
    Regenerate,
    Production,
    GeneratePoolOrders,
    ClearMarkets,
    MacroFlow,
    PayWages,
    TransportRebate,
    Consume,
    Attribution,
    Materialize,
    Telemetry,
    AdjustReservationPrices,
    UpdateConsumption,
    TickAudit,
}

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
    pub fn validated_labor_share_bps(&self) -> Result<i128, crate::economy::EconomyError> {
        if self.labor_share_bps > 10_000 {
            return Err(crate::economy::EconomyError::InvalidOrder);
        }
        Ok(self.labor_share_bps as i128)
    }

    /// `dividend_share_bps` as an i128, refusing `> 10_000` (a config bug that would
    /// over-distribute). Boundary `== 10_000` allowed (full distribution). Mirrors
    /// `validated_labor_share_bps`.
    pub fn validated_dividend_share_bps(&self) -> Result<i128, crate::economy::EconomyError> {
        if self.dividend_share_bps > 10_000 {
            return Err(crate::economy::EconomyError::InvalidOrder);
        }
        Ok(self.dividend_share_bps as i128)
    }

    /// `price_adjust_k_bps` as i128, refusing `> 10_000`. Boundary `== 10_000` allowed.
    pub fn validated_price_adjust_k_bps(&self) -> Result<i128, crate::economy::EconomyError> {
        if self.price_adjust_k_bps > 10_000 {
            return Err(crate::economy::EconomyError::InvalidOrder);
        }
        Ok(self.price_adjust_k_bps as i128)
    }

    /// `price_adjust_max_step_bps` as i128, refusing `> 10_000`.
    pub fn validated_price_adjust_max_step_bps(
        &self,
    ) -> Result<i128, crate::economy::EconomyError> {
        if self.price_adjust_max_step_bps > 10_000 {
            return Err(crate::economy::EconomyError::InvalidOrder);
        }
        Ok(self.price_adjust_max_step_bps as i128)
    }

    /// `(price_floor, price_ceiling)` as i64s, refusing `floor <= 0` or `floor >= ceiling`
    /// (a config bug that would allow a 0/negative price or an empty guardrail band).
    pub fn validated_price_band(&self) -> Result<(i64, i64), crate::economy::EconomyError> {
        if self.price_floor.0 <= 0 || self.price_floor.0 >= self.price_ceiling.0 {
            return Err(crate::economy::EconomyError::InvalidOrder);
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
            capita_baseline: crate::economy::capita::CAPITA_BASELINE_IDENTITY,
        }
    }
}

pub fn install_systems(schedule: &mut bevy_ecs::schedule::Schedule) {
    schedule.configure_sets(
        (
            EconomySet::RefreshCapita,
            EconomySet::ResetReceipts,
            EconomySet::RefreshLod,
            EconomySet::ExpireOrders,
            EconomySet::Regenerate,
            EconomySet::Production,
            EconomySet::GeneratePoolOrders,
            EconomySet::ClearMarkets,
            EconomySet::MacroFlow,
            EconomySet::PayWages,
            EconomySet::TransportRebate,
            EconomySet::Consume,
            EconomySet::Attribution,
            EconomySet::Materialize,
            EconomySet::Telemetry,
            EconomySet::AdjustReservationPrices,
            EconomySet::UpdateConsumption,
            EconomySet::TickAudit,
        )
            .chain(),
    );
    // The macro flow is stateful (writes dormant prices), so the set of markets
    // it mutates must be a deterministic function of LOD classification. Anchor
    // RefreshLod after CoreSet::LodReclassify. Inert (the CoreSet is simply not
    // configured) when EconomyPlugin installs without CorePlugin; load-bearing
    // only in the full SimPlugin stack, where it removes a classify/mutate race.
    schedule.configure_sets(
        EconomySet::RefreshLod.after(crate::world::schedule::CoreSet::LodReclassify),
    );
    // RefreshCapita: exclusive system, registered separately (mirrors
    // run_citizen_attribution_system / run_tick_audit_system). First in the chain
    // so the factor is current before Regenerate/Production/GeneratePoolOrders/
    // UpdateConsumption read it this tick.
    schedule.add_systems(
        crate::economy::capita::refresh_capita_factor_system
            .in_set(EconomySet::RefreshCapita)
            .before(crate::mobility::systems::tick_increment_system),
    );
    schedule.add_systems(
        (
            reset_seller_receipts_system.in_set(EconomySet::ResetReceipts),
            refresh_dormant_markets_system.in_set(EconomySet::RefreshLod),
            expire_orders_system.in_set(EconomySet::ExpireOrders),
            run_regen_system.in_set(EconomySet::Regenerate),
            run_production_system.in_set(EconomySet::Production),
            generate_pool_orders_system.in_set(EconomySet::GeneratePoolOrders),
            clear_dirty_markets_system.in_set(EconomySet::ClearMarkets),
            run_macro_flow_system.in_set(EconomySet::MacroFlow),
            run_pay_wages_system.in_set(EconomySet::PayWages),
            run_transport_rebate_system.in_set(EconomySet::TransportRebate),
            run_consumption_system.in_set(EconomySet::Consume),
            update_market_telemetry_system.in_set(EconomySet::Telemetry),
            run_adjust_reservation_prices_system.in_set(EconomySet::AdjustReservationPrices),
            run_consumption_update_system.in_set(EconomySet::UpdateConsumption),
        )
            .before(crate::mobility::systems::tick_increment_system),
    );
    // Tick-audit: registered separately (mirrors run_distribute_profit_system pattern —
    // keeps the main tuple at its original size so intra-tuple scheduling is unaffected)
    // and anchored to the TickAudit set (last in the chain, after UpdateConsumption) and
    // before tick_increment so it fires after ALL money moves this tick.
    schedule.add_systems(
        run_tick_audit_system
            .in_set(EconomySet::TickAudit)
            .before(crate::mobility::systems::tick_increment_system),
    );
    // Profit distribution: registered separately so the .after(run_pay_wages_system) edge
    // is unambiguously applied (intra-tuple .after is not always enforced in Bevy 0.18
    // when the tuple also carries a .before combinator). Placed in PayWages set, after wages,
    // before tick_increment — deterministic income accumulation: wage credit fires first,
    // profit credit adds on top.
    schedule.add_systems(
        run_distribute_profit_system
            .in_set(EconomySet::PayWages)
            .after(run_pay_wages_system)
            .before(crate::mobility::systems::tick_increment_system),
    );
    // Citizen attribution is an exclusive system (it reads the spatial Graph and
    // the observed-chunk set plus realized consumption/wage telemetry, then queries
    // citizen components). Registered separately (exclusive). The set chain places it
    // after Consume (so consumed_qty_last_tick is valid) and before Materialize.
    schedule.add_systems(
        crate::economy::attribution::run_citizen_attribution_system
            .in_set(EconomySet::Attribution)
            .before(crate::mobility::systems::tick_increment_system),
    );
    // Render-only trader materialization is an exclusive system (it needs &mut
    // World to spawn/despawn agents), so it is registered separately from the
    // parallel economy systems above. The set chain places it after MacroFlow.
    schedule.add_systems(
        crate::economy::materialize::materialize_traders_system
            .in_set(EconomySet::Materialize)
            .before(crate::mobility::systems::tick_increment_system),
    );
}

/// Tick-start: clear `SellerReceipts` and `BuyerOutlays` so the settle points accumulate
/// exactly one tick of revenue/charges (mirrors `run_consumption_at_tick`'s
/// reset-all-then-accumulate).
pub fn reset_seller_receipts_system(
    mut receipts: ResMut<SellerReceipts>,
    mut outlays: ResMut<BuyerOutlays>,
) {
    receipts.0.clear();
    outlays.0.clear();
}

/// Bridge: derive `DormantMarkets` from chunk LOD. A market anchored (in
/// `MarketChunks`) to a chunk that is not Active/Hot is dormant.
/// Cheap: one pass over active chunk coords + one over the anchor map.
/// Deterministic (BTree iteration, set membership).
#[allow(clippy::type_complexity)]
pub fn refresh_dormant_markets_system(
    anchors: Res<MarketChunks>,
    active_chunks: Query<&ChunkCoordComp, Or<(With<ActiveChunk>, With<HotChunk>)>>,
    mut dormant: ResMut<DormantMarkets>,
) {
    let active: BTreeSet<ChunkCoord> = active_chunks.iter().map(|c| c.0).collect();
    dormant.0 = anchors
        .0
        .iter()
        .filter(|(_, coord)| !active.contains(coord))
        .map(|(market, _)| *market)
        .collect();
}

pub fn expire_orders_system(
    tick: Res<Tick>,
    mut accounts: ResMut<AccountBook>,
    mut inventory: ResMut<InventoryBook>,
    mut orders: ResMut<OrderBook>,
    mut ledger: ResMut<TradeLedger>,
    mut dirty: ResMut<DirtyMarketGoods>,
) {
    let _ = expire_orders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        tick.0,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn generate_pool_orders_system(
    tick: Res<Tick>,
    config: Res<EconomyConfig>,
    dormant: Res<DormantMarkets>,
    capita: Res<crate::economy::capita::CapitaFactor>,
    mut accounts: ResMut<AccountBook>,
    mut inventory: ResMut<InventoryBook>,
    mut orders: ResMut<OrderBook>,
    mut ledger: ResMut<TradeLedger>,
    mut dirty: ResMut<DirtyMarketGoods>,
    mut next: ResMut<NextOrderId>,
    mut demand: ResMut<DemandPools>,
    mut supply: ResMut<SupplyPools>,
) {
    generate_pool_orders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut demand,
        &mut supply,
        tick.0,
        config.default_order_ttl_ticks,
        &dormant.0,
        capita.0,
    )
    .expect(
        "generate_pool_orders_at_tick: an Err (Overflow from a too-large capita_factor, or ZeroPrice) is a bug/mis-scale — fail loud rather than silently drop orders (OrderRejected for insufficient funds/goods is a ledger event, not an Err)",
    );
}

#[allow(clippy::too_many_arguments)]
pub fn clear_dirty_markets_system(
    tick: Res<Tick>,
    config: Res<EconomyConfig>,
    mut accounts: ResMut<AccountBook>,
    mut inventory: ResMut<InventoryBook>,
    mut orders: ResMut<OrderBook>,
    mut ledger: ResMut<TradeLedger>,
    mut goods: ResMut<MarketGoods>,
    mut dirty: ResMut<DirtyMarketGoods>,
    mut receipts: ResMut<SellerReceipts>,
    mut outlays: ResMut<BuyerOutlays>,
) {
    let keys: Vec<_> = dirty.0.iter().copied().collect();
    dirty.0.clear();
    for key in keys {
        if let Err(reason) = clear_market_good_with_receipts(
            &mut accounts,
            &mut inventory,
            &mut orders,
            &mut ledger,
            &mut goods,
            key,
            tick.0,
            config.settlement_policy,
            &mut receipts.0,
            &mut outlays.0,
        ) {
            ledger.0.push(EconomyEvent::MarketClearFailed {
                market: key.market,
                good: key.good,
                reason,
            });
        }
    }
}

/// The goods-only raw faucet. Surfaces an invariant break (an inventory.deposit overflow)
/// via `.expect` — matching the fail-fast convention now shared by run_pay_wages_system,
/// run_consumption_update_system, run_production_system, and generate_pool_orders_system.
/// The flow-capped faucet cannot overflow a sane i64 balance, so `.expect` is a loud
/// bug-surface, not a silent discard.
pub fn run_regen_system(
    tick: Res<Tick>,
    capita: Res<crate::economy::capita::CapitaFactor>,
    mut inventory: ResMut<InventoryBook>,
    mut ledger: ResMut<TradeLedger>,
    mut deposits: ResMut<RawDeposits>,
) {
    run_regen_at_tick(&mut inventory, &mut ledger, &mut deposits, tick.0, capita.0)
        .expect("run_regen_at_tick is infallible by construction (flow-capped faucet deposit cannot overflow a sane i64 balance); an Err is a bug");
}

pub fn run_production_system(
    tick: Res<Tick>,
    capita: Res<crate::economy::capita::CapitaFactor>,
    mut inventory: ResMut<InventoryBook>,
    mut ledger: ResMut<TradeLedger>,
    mut production: ResMut<ProductionPools>,
) {
    run_production_at_tick(
        &mut inventory,
        &mut ledger,
        &mut production,
        tick.0,
        capita.0,
    )
    .expect(
        "run_production_at_tick: an Err (Overflow from a too-large capita_factor scale) is a bug/mis-scale — fail loud, never silently skip production (which would diverge inventory the money audit cannot catch)",
    );
}

/// The demand-side sink (mirror of `run_production_system`): consume delivered goods,
/// emitting `FinalConsumed`. Runs in `EconomySet::Consume` after both delivery paths
/// (`ClearMarkets` + `MacroFlow`) and before next tick's `GeneratePoolOrders`.
pub fn run_consumption_system(
    tick: Res<Tick>,
    mut inventory: ResMut<InventoryBook>,
    mut ledger: ResMut<TradeLedger>,
    mut demand: ResMut<DemandPools>,
    mut market_goods: ResMut<MarketGoods>,
) {
    let _ = run_consumption_at_tick(
        &mut inventory,
        &mut ledger,
        &mut demand,
        &mut market_goods,
        tick.0,
    );
}

/// The SFC wage step: firms pay a labor share of this tick's revenue into the
/// household sector, apportioned to consumer pools (income). Runs after BOTH settle
/// paths (ClearMarkets, MacroFlow) so all receipts are booked, before Consume.
pub fn run_pay_wages_system(
    config: Res<EconomyConfig>,
    receipts: Res<SellerReceipts>,
    household: Res<HouseholdSector>,
    mut accounts: ResMut<AccountBook>,
    mut demand: ResMut<DemandPools>,
    mut wage_telemetry: ResMut<WageTelemetry>,
    mut ledger: ResMut<TradeLedger>,
) {
    run_pay_wages_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut wage_telemetry,
        &mut ledger,
        &config,
    )
    .expect("run_pay_wages_at_tick is infallible by construction (wage <= just-credited revenue, Σweights guards); an Err is a bug");
}

/// Profit distribution: runs in the PayWages set with an explicit `.after(run_pay_wages_system)`
/// edge so the wage net-zero assert fires first and income accumulates wage→profit in a
/// deterministic order. Fallible/audited at the per-firm level inside the core; the wrapper
/// surfaces a whole-call Err (only a config-validation failure can produce one) as an audited
/// MarketClearFailed event — never `let _` (which would swallow a config bug), never `.expect`
/// (the call is genuinely fallible).
pub fn run_distribute_profit_system(
    config: Res<EconomyConfig>,
    receipts: Res<SellerReceipts>,
    household: Res<HouseholdSector>,
    mut accounts: ResMut<AccountBook>,
    mut demand: ResMut<DemandPools>,
    mut ledger: ResMut<TradeLedger>,
) {
    if let Err(reason) = run_distribute_profit_at_tick(
        &mut accounts,
        &receipts,
        &mut demand,
        &household,
        &mut ledger,
        &config,
    ) {
        assert_ne!(
            reason,
            EconomyError::ConservationViolation,
            "CONSERVATION VIOLATION: HOUSEHOLD_SECTOR sentinel non-zero after profit distribution (sentinel-stranded cash) — the SFC invariant is broken; halting. This must never happen."
        );
        ledger.0.push(EconomyEvent::MarketClearFailed {
            market: MarketId(0),
            good: GoodId(0),
            reason,
        });
    }
}

/// Transport rebate: gated on the SAME `tick.0.is_multiple_of(macro_flow_interval_ticks)`
/// modulo as the operator CREDIT in macro_flow (which is itself inside that gate at
/// macro_flow.rs:712), so credit and rebate are phase-locked — stateless, NO persisted
/// cursor. Mid-interval the operator balance may be `> 0`; at every interval boundary it
/// drains to zero. `run_transport_rebate_at_tick` is conservative-by-construction (it
/// drains exactly the held operator balance via `transfer`), so `.expect` here is genuinely
/// infallible — mirroring the spec-endorsed `run_pay_wages_system` wrapper convention.
pub fn run_transport_rebate_system(
    tick: Res<Tick>,
    config: Res<EconomyConfig>,
    household: Res<HouseholdSector>,
    mut accounts: ResMut<AccountBook>,
    mut demand: ResMut<DemandPools>,
    mut ledger: ResMut<TradeLedger>,
) {
    if config.macro_flow_interval_ticks == 0
        || !tick.0.is_multiple_of(config.macro_flow_interval_ticks)
    {
        return;
    }
    run_transport_rebate_at_tick(&mut accounts, &mut demand, &household, &mut ledger)
        .expect("run_transport_rebate_at_tick is infallible by construction (drains exactly the held operator balance via transfer); an Err is a bug");
}

pub fn update_market_telemetry(
    goods: &mut MarketGoods,
    config: EconomyConfig,
) -> Result<(), EconomyError> {
    for state in goods.0.values_mut() {
        state.ewma_reference_price = integer_ewma(
            state.ewma_reference_price,
            state.last_settlement_price,
            config.ewma_alpha_bps,
        )?;
    }
    Ok(())
}

pub fn update_market_telemetry_system(config: Res<EconomyConfig>, mut goods: ResMut<MarketGoods>) {
    let _ = update_market_telemetry(&mut goods, *config);
}

#[allow(clippy::too_many_arguments)]
pub fn run_macro_flow_system(
    tick: Res<Tick>,
    config: Res<EconomyConfig>,
    dormant: Res<DormantMarkets>,
    dirty: Res<DirtyMarketGoods>,
    distances: Res<MarketDistances>,
    mut accounts: ResMut<AccountBook>,
    mut inventory: ResMut<InventoryBook>,
    mut ledger: ResMut<TradeLedger>,
    demand: Res<DemandPools>,
    supply: Res<SupplyPools>,
    mut market_goods: ResMut<MarketGoods>,
    mut flow: FlowShipmentParams,
    mut orders: ResMut<OrderBook>,
    mut next_order_id: ResMut<NextOrderId>,
    mut receipts: ResMut<SellerReceipts>,
    mut flow_ewma: ResMut<FlowRateEwma>,
    mut outlays: ResMut<BuyerOutlays>,
) {
    match run_macro_flow_at_tick(
        &mut accounts,
        &mut inventory,
        &mut ledger,
        &demand,
        &supply,
        &mut market_goods,
        &dirty,
        &dormant.0,
        &distances,
        &config,
        tick.0,
        &mut flow.shipments,
        &mut flow.next_id,
        &mut flow.realized,
        &mut orders,
        &mut next_order_id,
        &mut receipts.0,
        &mut outlays.0,
    ) {
        Ok(()) => {
            // Fold this interval's realized flows into the on-wire EWMA. Gated on
            // the SAME interval condition the macro flow uses internally, so the
            // smoothing cadence is one step per macro-flow run — never per tick.
            if config.macro_flow_interval_ticks != 0
                && tick.0.is_multiple_of(config.macro_flow_interval_ticks)
            {
                crate::economy::update_flow_rate_ewma(&mut flow_ewma, &flow.realized);
            }
        }
        Err(reason) => {
            // A whole-interval failure (e.g. a bucket-build overflow) is audited; the
            // atomic boundary left the books unchanged. Per-edge settle faults are
            // already isolated inside run_macro_flow_at_tick (their own
            // MarketClearFailed events). market/good = the demo sentinel for a
            // tick-level fault that is not attributable to one (market,good).
            ledger.0.push(EconomyEvent::MarketClearFailed {
                market: MarketId(0),
                good: GoodId(0),
                reason,
            });
        }
    }
}

/// Part B: rewrite each consumer pool's desired quantity from its current income +
/// the FINAL smoothed reference price. Runs after PayWages (income) and after
/// Telemetry (the ewma write). The new desired_qty becomes a bid in NEXT tick's
/// GeneratePoolOrders — the explicit 1-tick income→consumption lag.
pub fn run_consumption_update_system(
    mut demand: ResMut<DemandPools>,
    goods: Res<MarketGoods>,
    capita: Res<crate::economy::capita::CapitaFactor>,
) {
    run_consumption_update_at_tick(&mut demand, &goods, capita.0)
        .expect("run_consumption_update_at_tick is infallible once every consumer market has a seeded opening price; an Err is a bug");
}

/// Cadence-gated reservation-price nudge. Runs every `macro_flow_interval_ticks` (same slow
/// timescale as macro_flow, so the fast EWMA quantity loop settles between nudges). Surfaces a
/// genuine Err (config-validation / overflow) as an audited `MarketClearFailed` — never `let _`,
/// never a silent default.
///
/// Two-pass structure: first, compute the flow-coupled key sets from `RealizedFlows`; then run
/// the local-unmet tâtonnement (skipping flow-coupled pools so the margin anchors them); then run
/// the flow-margin feedback that nudges those pools toward the spatial LoOP target. Either pass's
/// Err is surfaced as a `MarketClearFailed` audit event and the other pass is still attempted
/// (independent operations, neither moves money, either partial result is safe).
pub fn run_adjust_reservation_prices_system(
    tick: Res<Tick>,
    config: Res<EconomyConfig>,
    market_goods: Res<MarketGoods>,
    realized: Res<crate::economy::RealizedFlows>,
    mut demand: ResMut<DemandPools>,
    mut supply: ResMut<SupplyPools>,
    mut ledger: ResMut<TradeLedger>,
) {
    if config.macro_flow_interval_ticks == 0
        || !tick.0.is_multiple_of(config.macro_flow_interval_ticks)
    {
        return;
    }
    let (skip_demand, skip_supply) = crate::economy::pricing::flow_coupled_keys(&realized);
    if let Err(reason) = crate::economy::pricing::run_adjust_reservation_prices_at_tick(
        &mut demand,
        &mut supply,
        &market_goods,
        &config,
        &skip_demand,
        &skip_supply,
    ) {
        ledger.0.push(EconomyEvent::MarketClearFailed {
            market: MarketId(0),
            good: GoodId(0),
            reason,
        });
    }
    if let Err(reason) = crate::economy::pricing::run_flow_margin_feedback_at_tick(
        &mut demand,
        &mut supply,
        &realized,
        &config,
    ) {
        ledger.0.push(EconomyEvent::MarketClearFailed {
            market: MarketId(0),
            good: GoodId(0),
            reason,
        });
    }
}

/// End-of-tick SFC conservation audit. Runs LAST (after UpdateConsumption, so all of this tick's
/// money moves are settled). Fail-fast: a drift is an unrecoverable invariant break, so a returned
/// Err panics — exactly like the codebase's other "this is impossible" .expect points. Emits a
/// TickAudit heartbeat every tick (the queryable conservation trace).
pub fn run_tick_audit_system(
    tick: Res<Tick>,
    accounts: Res<AccountBook>,
    mut ledger: ResMut<TradeLedger>,
    mut last: ResMut<crate::economy::audit::LastTickMoney>,
) {
    crate::economy::audit::run_tick_audit_at_tick(&accounts, &mut ledger, &mut last, tick.0)
        .expect("CONSERVATION VIOLATION: total_money changed between ticks (money minted/destroyed) — the SFC byte-invariant is broken; halting the tick. This must never happen.");
}
