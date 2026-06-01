use crate::economy::{GoodId, MarketId, NextShopperId, ShopperVisit, ShopperVisits};
use crate::routing::NodeId;

#[test]
fn id_prefix_distinguishes_shoppers_from_traders() {
    use crate::economy::EconomicActorId;
    use crate::economy::flow_shipments::SHIPMENT_ACTOR_OFFSET;
    use crate::economy::materialize::id_prefix;
    use crate::economy::shoppers::SHOPPER_ACTOR_OFFSET;
    assert_eq!(id_prefix(EconomicActorId(8003)), "trader:");
    assert_eq!(
        id_prefix(EconomicActorId(SHIPMENT_ACTOR_OFFSET + 1)),
        "trader:"
    );
    assert_eq!(
        id_prefix(EconomicActorId(SHOPPER_ACTOR_OFFSET + 1)),
        "shopper:"
    );
}

#[test]
fn economy_config_has_shopper_tuning_defaults() {
    let c = crate::economy::EconomyConfig::default();
    assert!(c.shoppers_per_unit >= 1);
    assert!(c.max_shoppers_per_market >= 1);
    assert!(c.shopper_radius_tiles > 0.0);
}

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
