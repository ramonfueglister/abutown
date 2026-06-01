use crate::economy::{GoodId, MarketId, NextShopperId, ShopperVisit, ShopperVisits};
use crate::routing::NodeId;

#[test]
fn shopper_progress_arrival_and_id() {
    let v = ShopperVisit {
        id: 0,
        market: MarketId(1),
        good: GoodId(0),
        origin_node: NodeId(7),
        start_tick: 100,
        travel_ticks: 10,
    };
    assert_eq!(v.progress(105), 0.5);
    assert!(!v.arrived(109));
    assert!(v.arrived(110));
    let mut n = NextShopperId::default();
    assert_eq!((n.next(), n.next()), (0, 1));
    assert_eq!(ShopperVisits::default().0.len(), 0);
}
