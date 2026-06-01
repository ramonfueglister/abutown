use crate::economy::{FlowShipment, FlowShipments, GoodId, MarketId, NextShipmentId, Quantity};

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
