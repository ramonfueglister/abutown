//! Ledger audit flush bookkeeping. The sim-server persistence loop drains new
//! `TradeLedger` events into the durable `EconomyEventStore` on the snapshot
//! cadence (append), then bounds the live ledger. These helpers own the cursor +
//! trim so the flush is robust if the economy appended more events mid-flush.

use bevy_ecs::prelude::*;

use crate::economy::{EconomyEvent, PERSISTED_LEDGER_TAIL, TradeLedger};
use crate::mobility::resources::Tick;

/// Index into `TradeLedger.0` of the first event NOT yet durably appended to the
/// audit store. Everything before it is durable.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LedgerAuditCursor(pub usize);

/// The un-appended tail of the ledger + the current tick. Non-mutating: only
/// `commit_ledger_audit` advances the cursor / trims, so a failed append (no
/// commit) leaves all state unchanged — the events are retried next cycle.
pub fn pending_ledger_audit(world: &World) -> (u64, Vec<EconomyEvent>) {
    let tick = world.get_resource::<Tick>().map_or(0, |t| t.0);
    let cursor = world.get_resource::<LedgerAuditCursor>().map_or(0, |c| c.0);
    let pending = world
        .get_resource::<TradeLedger>()
        .map(|l| l.0.get(cursor..).unwrap_or(&[]).to_vec())
        .unwrap_or_default();
    (tick, pending)
}

/// After a successful append of `appended` events, advance the cursor past them
/// and trim the live ledger to the last `PERSISTED_LEDGER_TAIL` events — but never
/// trim un-appended events (the trim is clamped to the cursor). Robust if the
/// economy pushed more events while the async append was in flight: those stay
/// pending for the next cycle.
pub fn commit_ledger_audit(world: &mut World, appended: usize) {
    let mut cursor = world.get_resource::<LedgerAuditCursor>().map_or(0, |c| c.0);
    if let Some(mut ledger) = world.get_resource_mut::<TradeLedger>() {
        let len = ledger.0.len();
        cursor = (cursor + appended).min(len);
        if len > PERSISTED_LEDGER_TAIL {
            let drain = (len - PERSISTED_LEDGER_TAIL).min(cursor);
            ledger.0.drain(0..drain);
            cursor -= drain;
        }
    } else {
        cursor += appended;
    }
    world.insert_resource(LedgerAuditCursor(cursor));
}

/// Initialize the cursor to the current ledger length. Called once after economy
/// hydration so the restored `ledger_tail` (already durably appended before
/// shutdown) is not re-appended (best-effort: a crash between the last flush and
/// shutdown loses at most the un-flushed window, acceptable for an audit log).
pub fn init_ledger_audit_cursor(world: &mut World) {
    let len = world.get_resource::<TradeLedger>().map_or(0, |l| l.0.len());
    world.insert_resource(LedgerAuditCursor(len));
}
