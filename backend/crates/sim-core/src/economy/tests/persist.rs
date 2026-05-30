use crate::economy::{GOOD_FOOD, MarketId, MarketSite, Money};

#[test]
fn value_types_are_serde_serializable() {
    let m = Money(1_234);
    let json = serde_json::to_string(&m).unwrap();
    assert_eq!(serde_json::from_str::<Money>(&json).unwrap(), m);

    let site = MarketSite {
        id: MarketId(1),
        node_id: crate::routing::NodeId(7),
        name: "M1".to_string(),
    };
    let j = serde_json::to_string(&site).unwrap();
    let back: MarketSite = serde_json::from_str(&j).unwrap();
    assert_eq!(back, site);
    let _ = GOOD_FOOD;
}
