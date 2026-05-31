# Economy Ledger Persistence Design — bounded ledger tail in the snapshot

Date: 2026-05-31

## Status

Backend polish (deferred economy follow-on). The economy `TradeLedger` is an
append-only, unbounded telemetry/audit stream that persistence-6a deliberately
**excluded** from `EconomyPersistSnapshot` (snapshotting the whole ledger each
cadence would grow without bound). This slice persists a **bounded recent tail**
of the ledger so economic history survives restart — enough for post-restart
continuity and the `/economy` debug view — without unbounded snapshot growth.

## Goal

Make the economy event types serde-serializable and add a `ledger_tail:
Vec<EconomyEvent>` to `EconomyPersistSnapshot`, holding the most-recent
`PERSISTED_LEDGER_TAIL` events. `extract_from_world` takes the tail;
`apply_into_world` restores `TradeLedger` from it (on hydration the live ledger
starts empty, so this reinstates recent history verbatim).

## Architecture (sim-core only, additive)

- Derive `Serialize, Deserialize` on `EconomyError` (all-unit enum) and
  `EconomyEvent` (its data is ids/`Money`/`Quantity`/`EconomyError`, all
  serde after 6a + this).
- `pub const PERSISTED_LEDGER_TAIL: usize = 1024;` in `economy/persist.rs`.
- `EconomyPersistSnapshot.ledger_tail: Vec<EconomyEvent>` (oldest→newest).
- `extract_from_world`: `let start = events.len().saturating_sub(PERSISTED_LEDGER_TAIL);
  events[start..].to_vec()`.
- `apply_into_world`: `world.insert_resource(TradeLedger(snap.ledger_tail.clone()))`.

No `EconomyConfig` change (the cap is a module constant — fewer touch points than
a config field for a marginal knob). No new table/store/wiring: the tail rides
the existing snapshot, so it is automatically written + restored by the
persistence-6b store/loop/hydration once that PR is in `main`, and surfaced by
`GET /economy`.

## Determinism / conservation

- Additive: no behavior, schedule, or matching change. The ledger is telemetry —
  persisting/restoring a tail of it neither creates nor destroys money/goods.
- Deterministic: `TradeLedger` is a `Vec` appended in tick order; the tail is the
  last K in that order. Byte-stable serialization (the snapshot already is).
- The existing 6a round-trip tests stay green (their seed pushes no ledger
  events → empty tail both sides).

## Testing

- `ledger_tail_is_capped_and_round_trips`: push `PERSISTED_LEDGER_TAIL + 50`
  events → tail length == cap, newest preserved, oldest 50 dropped; serialize →
  deserialize → apply restores the tail verbatim.
- All existing persist/economy suites unaffected.

Full gate on the CI-matching stable toolchain (rustc 1.96).

## What this is NOT

- Not a full append-only audit store (durable, unbounded, queryable economy event
  log in its own Postgres table) — that is a separate, larger infra slice,
  explicitly deferred. This is the bounded-tail continuity v0.
- No per-event filtering/compaction; the raw recent tail only.
