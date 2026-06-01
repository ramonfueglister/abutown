use bevy_ecs::prelude::*;

use crate::economy::audit::{
    LedgerAuditCursor, commit_ledger_audit, init_ledger_audit_cursor, pending_ledger_audit,
};
use crate::economy::{
    EconomicActorId, EconomyEvent, GOOD_TOOLS, MarketId, Money, PERSISTED_LEDGER_TAIL, Quantity,
    TradeLedger,
};
use crate::mobility::resources::Tick;
use crate::persistence::InMemoryEconomyEventStore;

fn world_with_ledger(n: usize) -> World {
    let mut world = World::new();
    let events: Vec<EconomyEvent> = (0..n)
        .map(|i| EconomyEvent::CashLocked {
            actor: EconomicActorId(i as u64),
            amount: Money(1),
        })
        .collect();
    world.insert_resource(TradeLedger(events));
    world.insert_resource(LedgerAuditCursor(0));
    world.insert_resource(Tick(7));
    world
}

#[test]
fn economy_event_type_tags_are_stable() {
    assert_eq!(
        EconomyEvent::Trade {
            market: MarketId(1),
            good: GOOD_TOOLS,
            buyer: EconomicActorId(1),
            seller: EconomicActorId(2),
            qty: Quantity(1),
            price: Money(1),
            total: Money(1),
        }
        .event_type(),
        "trade"
    );
    assert_eq!(
        EconomyEvent::TransportPaid {
            actor: EconomicActorId(1),
            amount: Money(5),
        }
        .event_type(),
        "transport_paid"
    );
    assert_eq!(
        EconomyEvent::CashLocked {
            actor: EconomicActorId(1),
            amount: Money(1),
        }
        .event_type(),
        "cash_locked"
    );
}

#[test]
fn ledger_audit_flush_appends_new_then_trims_and_bounds() {
    let n = PERSISTED_LEDGER_TAIL + 6;
    let mut world = world_with_ledger(n);

    let (tick, pending) = pending_ledger_audit(&world);
    assert_eq!(tick, 7);
    assert_eq!(pending.len(), n, "all events pending initially");

    commit_ledger_audit(&mut world, pending.len());
    assert_eq!(
        world.resource::<TradeLedger>().0.len(),
        PERSISTED_LEDGER_TAIL,
        "live ledger bounded to the tail after flush"
    );
    assert_eq!(
        world.resource::<LedgerAuditCursor>().0,
        PERSISTED_LEDGER_TAIL
    );
    assert!(
        pending_ledger_audit(&world).1.is_empty(),
        "nothing pending right after a flush"
    );

    // New events accumulate; the next flush appends only them and stays bounded.
    world
        .resource_mut::<TradeLedger>()
        .0
        .push(EconomyEvent::TransportPaid {
            actor: EconomicActorId(99),
            amount: Money(5),
        });
    let (_, pending2) = pending_ledger_audit(&world);
    assert_eq!(pending2.len(), 1, "only the new event is pending");
    commit_ledger_audit(&mut world, pending2.len());
    assert_eq!(
        world.resource::<TradeLedger>().0.len(),
        PERSISTED_LEDGER_TAIL,
        "still bounded after the second flush"
    );
    assert_eq!(
        world.resource::<LedgerAuditCursor>().0,
        PERSISTED_LEDGER_TAIL
    );
}

#[test]
fn ledger_audit_init_skips_restored_tail() {
    let mut world = world_with_ledger(50); // simulate a restored ledger_tail of 50
    init_ledger_audit_cursor(&mut world);
    assert_eq!(world.resource::<LedgerAuditCursor>().0, 50);
    assert!(
        pending_ledger_audit(&world).1.is_empty(),
        "restored tail is treated as already-appended (no re-append on restart)"
    );
}

#[test]
fn pending_ledger_audit_does_not_mutate() {
    let world = world_with_ledger(5);
    let before_len = world.resource::<TradeLedger>().0.len();
    let before_cursor = world.resource::<LedgerAuditCursor>().0;
    let _ = pending_ledger_audit(&world);
    let _ = pending_ledger_audit(&world);
    assert_eq!(world.resource::<TradeLedger>().0.len(), before_len);
    assert_eq!(world.resource::<LedgerAuditCursor>().0, before_cursor);
}

#[tokio::test]
async fn in_memory_event_store_appends_in_order() {
    use crate::persistence::EconomyEventStore;
    let mut store = InMemoryEconomyEventStore::default();
    store
        .append(
            "w",
            1,
            &[EconomyEvent::CashLocked {
                actor: EconomicActorId(1),
                amount: Money(1),
            }],
        )
        .await
        .unwrap();
    store
        .append(
            "w",
            2,
            &[EconomyEvent::TransportPaid {
                actor: EconomicActorId(1),
                amount: Money(5),
            }],
        )
        .await
        .unwrap();

    let evs = store.events("w");
    assert_eq!(evs.len(), 2);
    assert_eq!((evs[0].0, evs[0].1.event_type()), (1, "cash_locked"));
    assert_eq!((evs[1].0, evs[1].1.event_type()), (2, "transport_paid"));
    assert_eq!(store.len("other"), 0);
}

#[test]
fn macro_flow_event_type_is_macro_flow() {
    use crate::economy::{EconomyEvent, GoodId, MarketId, Money, Quantity};
    let ev = EconomyEvent::MacroFlow {
        from_market: MarketId(1),
        to_market: MarketId(2),
        good: GoodId(4),
        qty: Quantity(10),
        price: Money(1_000),
        transport: Money(50),
    };
    assert_eq!(ev.event_type(), "macro_flow");
}
