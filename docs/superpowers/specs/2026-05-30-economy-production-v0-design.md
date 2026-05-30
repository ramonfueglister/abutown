# Economy — Production v0 Design

Date: 2026-05-30

## Status

Approved direction (next slice of the economy roadmap). Builds directly on the
merged economy-v0 market core (`sim_core::economy`, PR #49). This is **deferred
slice 1** from the economy-v0 spec: "aggregate producers consume inputs and emit
explicit `Produced`/`Consumed` ledger events." Backend-only, deterministic, no
wire/API change. No moving traders, no transport costs, no LOD — those remain
later slices.

## Goal

Add aggregate **producers** that, on a fixed cadence, **consume input goods and
produce output goods** per a recipe, recording every change as explicit
`Consumed`/`Produced` ledger events. Demonstrable: a producer with a recipe
turns inputs into outputs deterministically; the goods delta is fully accounted
in the ledger; money is untouched.

## Spec-conformance

- Production is part of `sim_core::economy`; it reuses the existing
  `InventoryBook`, `TradeLedger`, `EconomyConfig`, `EconomySet` schedule, and the
  pool-cadence pattern (`interval_ticks`/`last_generated_tick`).
- No floats; `Quantity` fixed-point (×`ECONOMY_SCALE`) as in v0. Checked
  arithmetic; errors, not panics.
- Deterministic: `BTreeMap`-keyed pools, sorted iteration, tick-driven cadence —
  no rng/wall-clock/hash-order.
- Installed by the existing `EconomyPlugin` (an empty `ProductionPools` resource
  in runtime, like the demand/supply pools — actual producers are seeded later or
  in tests).

## Non-goals (later slices)

- Per-recipe price formation / cost-of-production pricing.
- Producers reacting to market prices / choosing recipes.
- Multi-step production graphs, byproducts, capacity ramps.
- Labor/population coupling, energy, spoilage.
- Moving trader agents, transport costs, LOD, persistence.

## Architecture

### New module `economy/production.rs`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    pub inputs: Vec<(GoodId, Quantity)>,
    pub outputs: Vec<(GoodId, Quantity)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductionPool {
    pub actor: EconomicActorId,
    pub recipe: Recipe,
    pub interval_ticks: u64,
    pub last_generated_tick: Option<u64>,
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ProductionPools(pub BTreeMap<EconomicActorId, ProductionPool>);
```

`run_production_at_tick(inventory: &mut InventoryBook, ledger: &mut TradeLedger,
production: &mut ProductionPools, current_tick: u64) -> Result<(), EconomyError>`:
for each pool (BTreeMap order = sorted by `EconomicActorId`):
- if the interval has not elapsed (`interval_elapsed(last, current, interval)`),
  skip without touching `last_generated_tick`;
- else: check the actor's **available** balance for every input good covers one
  batch. If ALL inputs are covered:
  - for each input `(good, qty)`: `inventory.consume(actor, good, qty)?` and push
    `EconomyEvent::Consumed { actor, good, qty }`;
  - for each output `(good, qty)`: `inventory.deposit(actor, good, qty)?` and push
    `EconomyEvent::Produced { actor, good, qty }`;
  - (consume-all-then-produce-all, so a recipe whose input good == output good
    nets correctly.)
- if any input is insufficient: produce nothing (no inventory change, no
  Consumed/Produced events) — v0 skips rather than partial-produces.
- always set `last_generated_tick = Some(current_tick)` after evaluating (whether
  it produced or skipped), so the cadence advances.

### `InventoryBook::consume` (new method, inventory.rs)

```rust
pub fn consume(&mut self, actor, good, qty) -> Result<(), EconomyError> {
    if qty.0 < 0 { return Err(EconomyError::NegativeQuantity); }
    let mut bal = self.balance(actor, good);
    if bal.available < qty { return Err(EconomyError::InsufficientGoods); }
    bal.available = bal.available.checked_sub(qty)?;
    self.balances.insert((actor, good), bal);
    Ok(())
}
```

Debits **available** directly (production is instantaneous; no lock phase). This
is the only new book primitive needed.

### Ledger events (ledger.rs)

Add to `EconomyEvent`:
```rust
    Produced { actor: EconomicActorId, good: GoodId, qty: Quantity },
    Consumed { actor: EconomicActorId, good: GoodId, qty: Quantity },
```

### Schedule (systems.rs)

Add `EconomySet::Production` and run production **before** pool order generation
so freshly-produced goods can be offered the same tick:

```
ExpireOrders -> Production -> GeneratePoolOrders -> ClearMarkets -> Telemetry
```

`run_production_system(tick: Res<Tick>, mut inventory: ResMut<InventoryBook>,
mut ledger: ResMut<TradeLedger>, mut production: ResMut<ProductionPools>)` — a
normal `Res`/`ResMut` system (idiomatic; NOT exclusive / no `resource_scope`)
that calls `run_production_at_tick`. `EconomyPlugin::install` inserts
`ProductionPools::default()`.

Reuse `interval_elapsed` from `pools.rs` (make it `pub(crate)`), don't duplicate.

## Conservation / invariants

- **Money:** untouched by production → total money strictly conserved.
- **Goods:** production CONVERTS goods, so total goods is NOT conserved. Instead
  the invariant is **full accounting**: for each actor/good, the net inventory
  change across a production run equals `(Σ Produced) − (Σ Consumed)` for that
  actor/good in the ledger. Tests assert this exact equality.
- Insufficient inputs ⇒ no change at all (atomic per pool: check all inputs
  before consuming any).

## Testing

- **Unit (inventory):** `consume` debits available; `consume` more than available
  → `InsufficientGoods`; negative qty → `NegativeQuantity`.
- **Production:** `production_consumes_inputs_and_produces_outputs` — recipe
  `2 WOOD + 1 IRON -> 1 TOOLS` (Quantities ×1000), actor stocked exactly one
  batch ⇒ after run: WOOD 0, IRON 0, TOOLS `Quantity(1_000)`; ledger has
  `Consumed WOOD 2000`, `Consumed IRON 1000`, `Produced TOOLS 1000`.
- `production_skips_when_inputs_insufficient` — missing IRON ⇒ no inventory change,
  no Consumed/Produced events, `last_generated_tick` still advances.
- `production_respects_interval` — does not produce again before `interval_ticks`.
- `production_conserves_money` — an `AccountBook` total is unchanged by a run.
- `production_ledger_accounts_for_goods_delta` — Δ(total available+locked) for each
  good == Σ Produced − Σ Consumed for that good in the ledger.
- `production_is_deterministic` — two identical worlds ⇒ identical ledger event
  sequence.
- **End-to-end (schedule):** install `EconomyPlugin`, seed a `ProductionPool` +
  the actor's inputs, `schedule.run` once ⇒ the `ProductionSet` runs and the
  outputs appear (asserts the wired path, like economy-v0's e2e trade test).
- **Wiring:** `EconomyPlugin` installs `ProductionPools`.
- Full gate: `cargo test --workspace`, `clippy --workspace --all-targets -D
  warnings`, `fmt --check`, `build -p sim-server`.

## Open questions (resolve in planning, against real code)

1. Whether `interval_elapsed` is currently `fn` (private) in `pools.rs` — promote
   to `pub(crate)` and reuse (confirm the signature).
2. `Recipe` with `Vec` fields means `ProductionPool`/`ProductionPools` are `Clone`
   but the `Vec` makes them non-`Copy` — confirm nothing requires `Copy` (the
   demand/supply pools were `Copy`; production pools won't be — adjust any `[k]`
   index-copy access to `.clone()` or `&` like the plan's pool loop).
3. Whether to emit a `ProductionSkipped` event on insufficient inputs — v0 says no
   (silent skip); revisit if observability needs it.
