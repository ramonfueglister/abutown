use std::collections::BTreeSet;

use bevy_ecs::prelude::*;
use bevy_ecs::query::Or;

use crate::economy::commuters::{CommuterTrips, NextCommuterId, capture_commuter_trips};
use crate::economy::{
    AccountBook, DemandPools, DirtyMarketGoods, DormantMarkets, EconomyError, EconomyEvent,
    FlowShipments, GoodId, HouseholdSector, InventoryBook, MarketChunks, MarketDistances,
    MarketGoods, MarketId, Money, NextOrderId, NextShipmentId, OrderBook, ProductionPools,
    SellerReceipts, SettlementPolicy, SupplyPools, TradeLedger, WageTelemetry,
    clear_market_good_with_receipts, expire_orders_at_tick, generate_pool_orders_at_tick,
    integer_ewma, run_consumption_at_tick, run_consumption_update_at_tick, run_macro_flow_at_tick,
    run_pay_wages_at_tick, run_production_at_tick,
};
use crate::ids::ChunkCoord;
use crate::mobility::resources::Tick;
use crate::world::components::{ActiveChunk, ChunkCoordComp, HotChunk};

#[derive(SystemSet, Hash, Eq, PartialEq, Debug, Clone)]
pub enum EconomySet {
    ResetReceipts,
    RefreshLod,
    ExpireOrders,
    Production,
    GeneratePoolOrders,
    ClearMarkets,
    MacroFlow,
    PayWages,
    Consume,
    ShopperCapture,
    CommuterCapture,
    Materialize,
    Telemetry,
    UpdateConsumption,
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
    /// How many unmet-demand units one visible shopper represents.
    pub shoppers_per_unit: i64,
    /// Cap on simultaneous shoppers rendered per market (keeps it a handful, not hundreds).
    pub max_shoppers_per_market: usize,
    /// Radius (tiles) around a market to pick shopper origin nodes.
    pub shopper_radius_tiles: f32,
    /// When TRUE, the macro flow drains active/observed markets' post-auction
    /// residual orders into the inter-market flow (S3). FALSE keeps the flow
    /// dormant-only (S1/S2 land dark). Defaulted FALSE; S3 flips it.
    pub drain_active_residual: bool,
    /// Labor share of value added (basis points, 0..=10_000). Default 6_000 = 0.60
    /// (Kaldor stylized fact). VALIDATED `0..=10_000` so `wage <= revenue` ⇒ no overdraft.
    pub labor_share_bps: u16,
    /// How many wage-Money units one visible commuter represents.
    pub commuters_per_wage_unit: i64,
    /// Absolute cap on simultaneous commuters rendered per market (viewport-bounded;
    /// NEVER derived from the wage magnitude, else the 1M population would leak in).
    pub max_commuters_per_market: usize,
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
            shopper_radius_tiles: 24.0,
            drain_active_residual: true,
            labor_share_bps: 6_000,
            commuters_per_wage_unit: 100,
            max_commuters_per_market: 4,
        }
    }
}

pub fn install_systems(schedule: &mut bevy_ecs::schedule::Schedule) {
    schedule.configure_sets(
        (
            EconomySet::ResetReceipts,
            EconomySet::RefreshLod,
            EconomySet::ExpireOrders,
            EconomySet::Production,
            EconomySet::GeneratePoolOrders,
            EconomySet::ClearMarkets,
            EconomySet::MacroFlow,
            EconomySet::PayWages,
            EconomySet::Consume,
            EconomySet::ShopperCapture,
            EconomySet::CommuterCapture,
            EconomySet::Materialize,
            EconomySet::Telemetry,
            EconomySet::UpdateConsumption,
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
    schedule.add_systems(
        (
            reset_seller_receipts_system.in_set(EconomySet::ResetReceipts),
            refresh_dormant_markets_system.in_set(EconomySet::RefreshLod),
            expire_orders_system.in_set(EconomySet::ExpireOrders),
            run_production_system.in_set(EconomySet::Production),
            generate_pool_orders_system.in_set(EconomySet::GeneratePoolOrders),
            clear_dirty_markets_system.in_set(EconomySet::ClearMarkets),
            run_macro_flow_system.in_set(EconomySet::MacroFlow),
            run_pay_wages_system.in_set(EconomySet::PayWages),
            run_consumption_system.in_set(EconomySet::Consume),
            update_market_telemetry_system.in_set(EconomySet::Telemetry),
            run_consumption_update_system.in_set(EconomySet::UpdateConsumption),
        )
            .before(crate::mobility::systems::tick_increment_system),
    );
    // Shopper capture is an exclusive system (it reads the spatial Graph +
    // NodeSpatialIndex and the observed-chunk set to pick deterministic origin
    // nodes), so it is registered separately like materialize below. The set chain
    // places it after MacroFlow and before Materialize so the same tick that
    // observes unmet demand also renders its shoppers.
    schedule.add_systems(
        run_shopper_capture_system
            .in_set(EconomySet::ShopperCapture)
            .before(crate::mobility::systems::tick_increment_system),
    );
    // Commuter capture is an exclusive system (mirrors run_shopper_capture_system),
    // reading WageTelemetry instead of unmet demand. Placed after PayWages and before
    // Materialize so the same tick that pays wages also renders its commuters.
    schedule.add_systems(
        run_commuter_capture_system
            .in_set(EconomySet::CommuterCapture)
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

/// Tick-start: clear `SellerReceipts` so the settle points accumulate exactly one
/// tick of revenue (mirrors `run_consumption_at_tick`'s reset-all-then-accumulate).
pub fn reset_seller_receipts_system(mut receipts: ResMut<SellerReceipts>) {
    receipts.0.clear();
}

/// Exclusive system: fill `ShopperVisits` from observed markets' unmet demand.
///
/// Mirrors how `materialize_traders_system` derives observed Active/Hot chunks: a
/// market is observed iff the chunk containing its market node is observed. For
/// each observed market it builds a deterministic origin-candidate provider from
/// `NodeSpatialIndex::within_radius` — which returns an UNSORTED `Vec<NodeId>`
/// (rstar tree order), so the result is SORTED by `NodeId` and the market node is
/// dropped before taking the Nth — then delegates to the pure
/// `capture_shopper_visits`. Routability is deferred to `materialize` (it skips
/// origins with no Walk route). No-op when the spatial world is absent (a
/// pure-economy schedule without `RoutingPlugin`), keeping the economy graph-free.
pub fn run_shopper_capture_system(world: &mut World) {
    use crate::economy::shoppers::{NextShopperId, ShopperVisits, capture_shopper_visits};
    use crate::economy::transport::manhattan_tiles;
    use crate::routing::{Graph, NodeId, NodeSpatialIndex};

    if world.get_resource::<Graph>().is_none() || world.get_resource::<NodeSpatialIndex>().is_none()
    {
        return;
    }

    let tick = world.get_resource::<Tick>().map(|t| t.0).unwrap_or(0);

    let observed_chunks: BTreeSet<ChunkCoord> = {
        let mut q =
            world.query_filtered::<&ChunkCoordComp, Or<(With<ActiveChunk>, With<HotChunk>)>>();
        q.iter(world).map(|c| c.0).collect()
    };

    // Capture into local copies inside a borrow scope: every economy/spatial read
    // is an immutable borrow of `world`, so the (visits, next) results are computed
    // here and written back below once those borrows are released — keeping the
    // exclusive system borrow-clean.
    let captured = {
        let graph = world.resource::<Graph>();
        let markets = world.resource::<crate::economy::Markets>();
        // Observed markets: those whose market-node chunk is currently observed.
        let observed_markets: BTreeSet<MarketId> = markets
            .0
            .iter()
            .filter(|(_, site)| {
                let pos = graph.node(site.node_id).position;
                observed_chunks.contains(&crate::mobility::chunk_of(pos.0, pos.1, 32))
            })
            .map(|(id, _)| *id)
            .collect();
        if observed_markets.is_empty() {
            return;
        }

        let spatial = world.resource::<NodeSpatialIndex>();
        let config = *world.resource::<EconomyConfig>();
        let market_goods = world.resource::<MarketGoods>();

        let mut visits = world.resource::<ShopperVisits>().clone();
        let mut next = *world.resource::<NextShopperId>();
        // Deterministic origin provider: within_radius (UNSORTED) -> sort by NodeId
        // -> drop the market node -> pair with Manhattan distance (tiles) to the
        // market.
        let origins = |market_node: NodeId| -> Vec<(NodeId, i64)> {
            let pos = graph.node(market_node).position;
            let mut cands = spatial.within_radius((pos.0, pos.1), config.shopper_radius_tiles);
            cands.sort_unstable_by_key(|n| n.0);
            cands
                .into_iter()
                .filter(|n| *n != market_node)
                .map(|n| (n, manhattan_tiles(graph, n, market_node)))
                .collect()
        };
        capture_shopper_visits(
            market_goods,
            &observed_markets,
            markets,
            origins,
            &config,
            tick,
            &mut visits,
            &mut next,
        );
        (visits, next)
    };
    *world.resource_mut::<ShopperVisits>() = captured.0;
    *world.resource_mut::<NextShopperId>() = captured.1;
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
    mut accounts: ResMut<AccountBook>,
    mut inventory: ResMut<InventoryBook>,
    mut orders: ResMut<OrderBook>,
    mut ledger: ResMut<TradeLedger>,
    mut dirty: ResMut<DirtyMarketGoods>,
    mut next: ResMut<NextOrderId>,
    mut demand: ResMut<DemandPools>,
    mut supply: ResMut<SupplyPools>,
) {
    let _ = generate_pool_orders_at_tick(
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
        ) {
            ledger.0.push(EconomyEvent::MarketClearFailed {
                market: key.market,
                good: key.good,
                reason,
            });
        }
    }
}

pub fn run_production_system(
    tick: Res<Tick>,
    mut inventory: ResMut<InventoryBook>,
    mut ledger: ResMut<TradeLedger>,
    mut production: ResMut<ProductionPools>,
) {
    let _ = run_production_at_tick(&mut inventory, &mut ledger, &mut production, tick.0);
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
    mut shipments: ResMut<FlowShipments>,
    mut next_shipment_id: ResMut<NextShipmentId>,
    mut orders: ResMut<OrderBook>,
    mut next_order_id: ResMut<NextOrderId>,
    mut receipts: ResMut<SellerReceipts>,
) {
    if let Err(reason) = run_macro_flow_at_tick(
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
        &mut shipments,
        &mut next_shipment_id,
        &mut orders,
        &mut next_order_id,
        &mut receipts.0,
    ) {
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

/// Part B: rewrite each consumer pool's desired quantity from its current income +
/// the FINAL smoothed reference price. Runs after PayWages (income) and after
/// Telemetry (the ewma write). The new desired_qty becomes a bid in NEXT tick's
/// GeneratePoolOrders — the explicit 1-tick income→consumption lag.
pub fn run_consumption_update_system(mut demand: ResMut<DemandPools>, goods: Res<MarketGoods>) {
    run_consumption_update_at_tick(&mut demand, &goods)
        .expect("run_consumption_update_at_tick is infallible once every consumer market has a seeded opening price; an Err is a bug");
}

/// Exclusive system: fill `CommuterTrips` from observed markets' realized WAGES.
///
/// Mirrors `run_shopper_capture_system` exactly, reading `WageTelemetry` instead of
/// unmet demand. A market is observed iff the chunk containing its market node is
/// observed. The origin-candidate provider is built from `NodeSpatialIndex::within_radius`
/// (UNSORTED) → sorted by `NodeId` → market node dropped → paired with Manhattan distance
/// (tiles) → delegates to the pure `capture_commuter_trips`. Routability is deferred to
/// `materialize` (it skips origins with no Walk route). No-op when the spatial world is
/// absent (a pure-economy schedule without `RoutingPlugin`).
pub fn run_commuter_capture_system(world: &mut World) {
    use crate::economy::transport::manhattan_tiles;
    use crate::routing::{Graph, NodeId, NodeSpatialIndex};

    if world.get_resource::<Graph>().is_none() || world.get_resource::<NodeSpatialIndex>().is_none()
    {
        return;
    }

    let tick = world.get_resource::<Tick>().map(|t| t.0).unwrap_or(0);

    let observed_chunks: BTreeSet<ChunkCoord> = {
        let mut q =
            world.query_filtered::<&ChunkCoordComp, Or<(With<ActiveChunk>, With<HotChunk>)>>();
        q.iter(world).map(|c| c.0).collect()
    };

    // Capture into local copies inside a borrow scope: every economy/spatial read
    // is an immutable borrow of `world`, so the (trips, next) results are computed
    // here and written back below once those borrows are released.
    let captured = {
        let graph = world.resource::<Graph>();
        let markets = world.resource::<crate::economy::Markets>();
        // Observed markets: those whose market-node chunk is currently observed.
        let observed_markets: BTreeSet<MarketId> = markets
            .0
            .iter()
            .filter(|(_, site)| {
                let pos = graph.node(site.node_id).position;
                observed_chunks.contains(&crate::mobility::chunk_of(pos.0, pos.1, 32))
            })
            .map(|(id, _)| *id)
            .collect();
        if observed_markets.is_empty() {
            return;
        }

        let spatial = world.resource::<NodeSpatialIndex>();
        let config = *world.resource::<EconomyConfig>();
        let wage_telemetry = world.resource::<WageTelemetry>();

        let mut trips = world.resource::<CommuterTrips>().clone();
        let mut next = *world.resource::<NextCommuterId>();
        // Deterministic origin provider: within_radius (UNSORTED) -> sort by NodeId
        // -> drop the market node -> pair with Manhattan distance (tiles) to the
        // market.
        let origins = |market_node: NodeId| -> Vec<(NodeId, i64)> {
            let pos = graph.node(market_node).position;
            let mut cands = spatial.within_radius((pos.0, pos.1), config.shopper_radius_tiles);
            cands.sort_unstable_by_key(|n| n.0);
            cands
                .into_iter()
                .filter(|n| *n != market_node)
                .map(|n| (n, manhattan_tiles(graph, n, market_node)))
                .collect()
        };
        capture_commuter_trips(
            wage_telemetry,
            &observed_markets,
            markets,
            origins,
            &config,
            tick,
            &mut trips,
            &mut next,
        );
        (trips, next)
    };
    *world.resource_mut::<CommuterTrips>() = captured.0;
    *world.resource_mut::<NextCommuterId>() = captured.1;
}
