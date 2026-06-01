use crate::economy::macro_flow::run_macro_flow_at_tick;
use crate::economy::{
    AccountBook, DemandPool, DemandPools, DirtyMarketGoods, EconomicActorId, EconomyConfig,
    InventoryBook, MarketDistances, MarketGoodKey, MarketGoodState, MarketGoods, Money, SupplyPool,
    SupplyPools, TradeLedger,
};
use crate::economy::{FlowShipment, FlowShipments, GoodId, MarketId, NextShipmentId, Quantity};
use std::collections::BTreeSet;

#[test]
fn shipment_progress_and_arrival() {
    let s = FlowShipment {
        id: 0,
        from_market: MarketId(1),
        to_market: MarketId(2),
        good: GoodId(0),
        qty: Quantity(10),
        start_tick: 100,
        travel_ticks: 10,
    };
    assert_eq!(s.progress(100), 0.0);
    assert_eq!(s.progress(105), 0.5);
    assert_eq!(s.progress(110), 1.0);
    assert!(!s.arrived(109));
    assert!(s.arrived(110));
    let mut n = NextShipmentId::default();
    assert_eq!(n.next(), 0);
    assert_eq!(n.next(), 1);
    assert_eq!(FlowShipments::default().0.len(), 0);
}

#[test]
fn macro_flow_captures_one_shipment_per_cross_edge() {
    let a = MarketId(1);
    let b = MarketId(2);
    let good = GoodId(0);
    let seller = EconomicActorId(10);
    let buyer = EconomicActorId(20);

    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    accounts.deposit(buyer, Money(1_000_000)).unwrap();
    inventory.deposit(seller, good, Quantity(1_000)).unwrap();

    let mut supply = SupplyPools::default();
    supply.0.insert(
        seller,
        SupplyPool {
            actor: seller,
            market: a,
            good,
            offered_qty_per_tick: Quantity(100),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    let mut demand = DemandPools::default();
    demand.0.insert(
        buyer,
        DemandPool {
            actor: buyer,
            market: b,
            good,
            desired_qty_per_tick: Quantity(100),
            max_price: Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );

    let mut mg = MarketGoods::default();
    mg.0.insert(
        MarketGoodKey { market: a, good },
        MarketGoodState::new(MarketGoodKey { market: a, good }),
    );
    mg.0.insert(
        MarketGoodKey { market: b, good },
        MarketGoodState::new(MarketGoodKey { market: b, good }),
    );

    // dist=4: transport=5*4=20, net_gain=200-50-20=130>0 → profitable cross-edge.
    // (dist=40 would give transport=200, net_gain=-50, pruning the edge.)
    let mut dist = MarketDistances::default();
    dist.0.insert((a, b), 4);
    dist.0.insert((b, a), 4);
    let dormant: BTreeSet<MarketId> = [a, b].into_iter().collect();

    let config = EconomyConfig {
        transport_cost_per_tile_unit: Money(50),
        ..Default::default()
    };
    let dirty = DirtyMarketGoods::default();
    let mut ledger = TradeLedger::default();
    let mut shipments = FlowShipments::default();
    let mut next_id = NextShipmentId::default();

    run_macro_flow_at_tick(
        &mut accounts,
        &mut inventory,
        &mut ledger,
        &demand,
        &supply,
        &mut mg,
        &dirty,
        &dormant,
        &dist,
        &config,
        /*tick=*/ 0,
        &mut shipments,
        &mut next_id,
    )
    .unwrap();

    assert_eq!(shipments.0.len(), 1, "one shipment for the A->B cross edge");
    let s = shipments.0.values().next().unwrap();
    assert_eq!((s.from_market, s.to_market, s.good), (a, b, good));
    assert_eq!(s.start_tick, 0);
    assert!(s.travel_ticks > 0);
    assert_eq!(next_id.0, 1);
}

#[test]
fn expire_arrived_drops_only_arrived() {
    let mut s = FlowShipments::default();
    s.0.insert(
        0,
        FlowShipment {
            id: 0,
            from_market: MarketId(1),
            to_market: MarketId(2),
            good: GoodId(0),
            qty: Quantity(1),
            start_tick: 0,
            travel_ticks: 10,
        },
    );
    s.0.insert(
        1,
        FlowShipment {
            id: 1,
            from_market: MarketId(1),
            to_market: MarketId(2),
            good: GoodId(0),
            qty: Quantity(1),
            start_tick: 5,
            travel_ticks: 10,
        },
    );
    crate::economy::flow_shipments::expire_arrived(&mut s, /*tick=*/ 12);
    assert_eq!(
        s.0.keys().copied().collect::<Vec<_>>(),
        vec![1],
        "shipment 0 arrived (0+10<=12), 1 not (5+10>12)"
    );
}
