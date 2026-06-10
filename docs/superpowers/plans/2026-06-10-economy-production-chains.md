# Economy Production Chains (Firms-as-Buyers WOOD→TOOLS) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Die TOOLS-Firma (8031) kauft ihr Input-Gut WOOD über den bestehenden Order/Settle/Macro-Flow-Pfad von einem neuen WOOD-Extraktor (8041, Markt 9003); Löhne auf Wertschöpfung; θ-Dividende mit Working-Capital-Kappung ersetzt den 100%-Payout.

**Architecture:** Spec: `docs/superpowers/specs/2026-06-10-economy-production-chains-design.md` (LESEN vor Start). Neue Bausteine: `BuyerOutlays` (per-Tick-Spiegel von `SellerReceipts`, Capture an beiden Settle-Punkten), `InputPools`/`ProducerPolicies` in neuem Modul `economy/producers.rs`, `ProducerSpec` in `markets.json`. Bestandsakteure verhalten sich byte-identisch (Outlays 0 → value_added = revenue; θ=10000/wc=0 Default).

**Tech Stack:** Rust (bevy_ecs, fixed-point i64/i128, BTreeMap keys-first), prost/buf Protobuf, TypeScript-Frontend. Cargo NUR über `scripts/cargo-serial.sh`, nie zwei cargo parallel (CLAUDE.md).

**Konventionen (gelten für JEDE Task):**
- Test-Kommandos: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core <filter>` (vom Worktree-Root).
- Determinismus: keys-first-Iteration, floor-div via i128, keine Floats/RNG in der Autorität.
- Kein serde-default auf persistierten Snapshot-Feldern; Welt-Layer-Daten (`base_world.rs`) DÜRFEN `#[serde(default)]` tragen (Präzedenz: `capita_baseline`).
- Fehler ehrlich propagieren (`?`/`.expect` mit Begründung), keine silent defaults.

---

### Task 0: Worktree + Spec-Commits in den Branch holen

**Files:** keine Code-Änderung.

- [ ] **Step 1:** Worktree anlegen (EnterWorktree, Name `economy-production-chains`). Basis ist origin/main.
- [ ] **Step 2:** Spec- und Plan-Commits von lokalem `main` cherry-picken, damit beide via PR auf origin landen. Hashes dynamisch auflösen (nicht raten):

```bash
git log --oneline main --not origin/main -- docs/superpowers/specs/2026-06-10-economy-production-chains-design.md docs/superpowers/plans/2026-06-10-economy-production-chains.md
# erwartete 3 Treffer (Plan, Spec-Korrektur, Spec) → in CHRONOLOGISCHER Reihenfolge (älteste zuerst) cherry-picken:
git cherry-pick <spec-hash> <spec-fix-hash> <plan-hash>
```

Expected: drei Commits, ausschließlich Dateien unter `docs/superpowers/`. NICHT mitnehmen: fremde Commits/Änderungen auf lokalem main (z. B. `app/tests.rs`-WIP des Users).
- [ ] **Step 3:** Baseline: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core` → alle Tests PASS. Vorher `pgrep -f cargo` und Orphans klären.

---

### Task 1: `BuyerOutlays` — Resource + Capture an beiden Settle-Punkten

**Files:**
- Modify: `backend/crates/sim-core/src/economy/wages.rs` (Resource neben `SellerReceipts`)
- Modify: `backend/crates/sim-core/src/economy/auction.rs` (`clear_market_good_with_receipts`)
- Modify: `backend/crates/sim-core/src/economy/macro_flow.rs` (`settle_flow_with_receipts`)
- Modify: `backend/crates/sim-core/src/economy/systems.rs` (`reset_seller_receipts_system`, Wrapper-Callsites)
- Modify: `backend/crates/sim-core/src/economy/mod.rs` (Re-Export `BuyerOutlays`; Resource-Init dort, wo `SellerReceipts` initialisiert wird — `grep -rn "init_resource::<SellerReceipts>\|insert_resource(SellerReceipts" backend/crates/sim-core/src` und exakt das Muster spiegeln)

- [ ] **Step 1: Failing Test (Auktion captured Buyer-Outlay)** — in `auction.rs` `mod tests`, neben dem bestehenden Receipts-Test (per `grep -n "receipts" auction.rs` Test-Vorbild suchen und dessen Setup-Helfer wiederverwenden):

```rust
#[test]
fn auction_settle_records_buyer_outlay_at_actual_cost() {
    // Setup wie der bestehende SellerReceipts-Settle-Test: 1 Bid, 1 Ask, clear.
    // Nach dem Clear:
    // receipts[(seller, market)] == actual_cost  (bestehende Zusicherung)
    // outlays[(buyer, market)]  == actual_cost   (NEU)
}
```

(Vollständiges Setup aus dem Nachbar-Test kopieren; die Assertion auf `outlays` ist der neue Kern.)

- [ ] **Step 2:** Test laufen lassen → FAIL (Param/Resource existiert nicht).
- [ ] **Step 3: Implementation.** In `wages.rs` direkt unter `SellerReceipts`:

```rust
/// Buyer-side mirror of `SellerReceipts`: gross purchase charges debited from each
/// `(buyer, market)` THIS tick (auction: actual cost; macro flow: full charge INCLUDING
/// the transport premium — so chain input costs are transport-inclusive by construction).
/// Captured UNCONDITIONALLY for every buyer (consumer outlays are a harmless unused
/// statistic; only the PayWages join reads this). Non-monetary statistic, zeroed in
/// `EconomySet::ResetReceipts`, NEVER persisted. Captured in the settle scratch zone,
/// so a discarded settle discards its outlays too.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct BuyerOutlays(pub BTreeMap<(EconomicActorId, MarketId), Money>);
```

In `auction.rs`: `clear_market_good_with_receipts` bekommt den Parameter `outlays: &mut std::collections::BTreeMap<(EconomicActorId, MarketId), Money>` (hinter `receipts`); in der Scratch-Zone `let mut next_outlays = outlays.clone();`, im Fill-Loop direkt nach dem Seller-Receipt-Block:

```rust
// Accumulate buyer charge into next_outlays (scratch zone, before any commit).
let slot = next_outlays
    .entry((bid.owner, key.market))
    .or_insert(Money::ZERO);
*slot = slot.checked_add(actual_cost)?;
```

Im Commit-Block `*outlays = next_outlays;`. Der dünne Wrapper `clear_market_good` (Zeile ~344) reicht ein zweites `&mut discard` durch (zweite lokale `BTreeMap::new()`).

In `macro_flow.rs`: `settle_flow_with_receipts` bekommt `outlays: &mut BTreeMap<(EconomicActorId, MarketId), Money>`; im Buyer-Loop nach dem `debit_locked`:

```rust
if charge.0 > 0 {
    let slot = outlays.entry((*actor, flow.dst)).or_insert(Money::ZERO);
    *slot = slot.checked_add(charge)?;
}
```

`settle_flow` (dünner Wrapper) reicht ein `&mut BTreeMap::new()` durch. ACHTUNG: macro_flow settled direkt auf den echten Refs (keine Scratch-Clones) — Outlay-Buchung NACH der gelungenen Geldbewegung ist dort die korrekte Kohärenz.

In `systems.rs`: `reset_seller_receipts_system` erweitern:

```rust
pub fn reset_seller_receipts_system(
    mut receipts: ResMut<SellerReceipts>,
    mut outlays: ResMut<BuyerOutlays>,
) {
    receipts.0.clear();
    outlays.0.clear();
}
```

Alle Callsites der beiden `_with_receipts`-Funktionen (Wrapper in `systems.rs`/`macro_flow.rs` — per `grep -rn "with_receipts" backend/crates` vollständig finden) reichen `&mut buyer_outlays.0` durch. Resource-Init exakt da, wo `SellerReceipts` initialisiert wird.

- [ ] **Step 4: Failing Test (macro_flow inkl. Transport)** — in `macro_flow.rs` `mod tests`, neben dem bestehenden settle-Receipts-Test:

```rust
#[test]
fn flow_settle_records_buyer_outlays_including_transport() {
    // Setup wie der bestehende settle_flow-Test (1 seller, 1 buyer, dist > 0).
    // outlays[(buyer, flow.dst)] == src_revenue + transport_total (== dst_payment)
}
```

- [ ] **Step 5:** Beide neuen Tests + bestehende `-p sim-core` Tests laufen lassen → PASS.
- [ ] **Step 6:** Commit: `feat(economy): BuyerOutlays — per-tick buyer-charge capture at both settle points`

---

### Task 2: Value-added-Löhne

**Files:**
- Modify: `backend/crates/sim-core/src/economy/wages.rs`
- Modify: `backend/crates/sim-core/src/economy/systems.rs` (`run_pay_wages_system`-Wrapper: `Res<BuyerOutlays>` durchreichen)

- [ ] **Step 1: Failing Tests** — in `wages.rs` `mod tests` (Vorbild: bestehende `run_pay_wages_at_tick`-Tests):

```rust
#[test]
fn wage_basis_is_value_added_when_firm_bought_inputs() {
    // receipts[(firm, m)] = 1000, outlays[(firm, m)] = 400
    // → wage == floor(0.6 * 600) == 360 (statt 600)
}

#[test]
fn negative_value_added_pays_zero_wage() {
    // receipts[(firm, m)] = 100, outlays[(firm, m)] = 400 → wage == 0, kein Transfer
}

#[test]
fn actors_without_outlays_unchanged() {
    // receipts only → wage identisch zur bisherigen Berechnung (Regression)
}
```

- [ ] **Step 2:** Laufen lassen → FAIL (Signatur ohne outlays).
- [ ] **Step 3: Implementation.** In `wages.rs` neue Hilfsfunktion + Signaturerweiterung:

```rust
/// Value added for one (firm, market): revenue minus this tick's buyer outlays,
/// floored at zero (a buy-heavy tick pays zero wage, never a negative transfer).
pub(crate) fn value_added_for(
    revenue: Money,
    outlays: &BuyerOutlays,
    firm: EconomicActorId,
    market: MarketId,
) -> Result<Money, EconomyError> {
    let spent = outlays
        .0
        .get(&(firm, market))
        .copied()
        .unwrap_or(Money::ZERO);
    Ok(Money((revenue.0.checked_sub(spent.0).ok_or(EconomyError::Overflow)?).max(0)))
}
```

`run_pay_wages_at_tick` bekommt `outlays: &BuyerOutlays`; im FIRST-LEG-Loop ersetzt

```rust
let value_added = value_added_for(revenue, outlays, firm, market)?;
let wage = wage_for_revenue(value_added, labor_share)?;
```

das bisherige `wage_for_revenue(revenue, labor_share)`. WICHTIG: Doku-Kommentar der Funktion anpassen („wage on value added, not revenue"). Wrapper in `systems.rs` reicht `Res<BuyerOutlays>` durch.

- [ ] **Step 4:** Tests laufen lassen → PASS (inkl. ALLER bestehenden wages-Tests; bestehende Tests, die `run_pay_wages_at_tick` direkt rufen, bekommen `&BuyerOutlays::default()`).
- [ ] **Step 5:** Commit: `feat(economy): wages on value added (revenue − buyer outlays)`

---

### Task 3: `ProducerPolicies` + θ-Dividende mit Working-Capital-Kappung

**Files:**
- Create: `backend/crates/sim-core/src/economy/producers.rs`
- Modify: `backend/crates/sim-core/src/economy/mod.rs` (`pub mod producers;` + Re-Exports)
- Modify: `backend/crates/sim-core/src/economy/wages.rs` (`run_distribute_profit_at_tick`)
- Modify: `backend/crates/sim-core/src/economy/systems.rs` (Wrapper)

- [ ] **Step 1: Struktur anlegen** — `producers.rs`:

```rust
//! Firms-as-buyers: producer policies (θ-dividend + working-capital target) and
//! Leontief input pools. Spec: docs/superpowers/specs/2026-06-10-economy-production-chains-design.md
//! Grounding: Caiani et al. (2016) dividend share θ + liquidity buffer;
//! Carvalho & Tahbaz-Salehi (2019) fixed-coefficient input demand.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::economy::{EconomicActorId, GoodId, MarketId, Money, Quantity};

/// Authored payout policy per producer. NOT persisted — re-applied from the
/// markets layer at every start (the #83 lesson: config must not silently
/// revert to defaults on restart). Actors absent from this map keep the #75
/// behavior exactly: theta_bps = 10_000, wc_target = 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProducerPolicy {
    pub theta_bps: u16,      // validated 0..=10_000 at seed
    pub batches_target: u32, // validated >= 1 at seed; ONE knob: stock AND cash target
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ProducerPolicies(pub BTreeMap<EconomicActorId, ProducerPolicy>);

/// Leontief input pool: derived demand for one producer's input good at its home
/// market. `max_price` is the participation bound, rewritten every generation pass
/// (§5.4 of the spec); `last_generated_tick` is the only true state (persisted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InputPool {
    pub actor: EconomicActorId,
    pub market: MarketId,
    pub good: GoodId,
    pub in_qty: Quantity,  // recipe input per batch (denormalized from ProductionPools at seed)
    pub out_qty: Quantity, // recipe output per batch (for the participation bound)
    pub out_good: GoodId,  // whose reference price bounds the bid
    pub interval_ticks: u64,
    pub last_generated_tick: Option<u64>,
    pub max_price: Money, // last computed participation bound (telemetry + order price)
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct InputPools(pub BTreeMap<EconomicActorId, InputPool>);

/// Working-capital target in Money: expected input cost of `batches_target` batches
/// at the current participation bound. Deterministic i128 floor math.
pub(crate) fn wc_target(policy: ProducerPolicy, pool: &InputPool) -> Result<Money, crate::economy::EconomyError> {
    let raw = (policy.batches_target as i128) * (pool.in_qty.0 as i128) * (pool.max_price.0 as i128)
        / (crate::economy::ECONOMY_SCALE);
    Ok(Money(i64::try_from(raw).map_err(|_| crate::economy::EconomyError::Overflow)?))
}
```

ACHTUNG `wc_target`-Skalierung: prüfen, wie `checked_order_value(price, qty)` Money×Quantity verrechnet (`grep -n "fn checked_order_value" backend/crates/sim-core/src/economy`) und EXAKT dieselbe Skalenarithmetik verwenden — Kosten von `n·in_qty` Einheiten zu `max_price` müssen mit dem Settle-Wert übereinstimmen. Im Zweifel `checked_order_value(pool.max_price, Quantity(n * in_qty))` direkt wiederverwenden statt eigener Formel.

- [ ] **Step 2: Failing Tests** — in `wages.rs` `mod tests`:

```rust
#[test]
fn dividend_theta_caps_at_working_capital_target() {
    // policy θ=8000, batches=2; profit so, dass intended > cash − wc_target
    // → dividend == cash − wc_target (Kappung greift), Firma behält wc_target
}

#[test]
fn dividend_zero_when_cash_below_target() {
    // cash < wc_target → dividend == 0, kein Transfer, kein Event
}

#[test]
fn actors_without_policy_distribute_like_before() {
    // kein ProducerPolicies-Eintrag → θ=10000, wc=0 → byte-identisch zu #75 (Regression)
}
```

- [ ] **Step 3:** FAIL bestätigen.
- [ ] **Step 4: Implementation.** `run_distribute_profit_at_tick` erweitert um `outlays: &BuyerOutlays`, `policies: &ProducerPolicies`, `input_pools: &InputPools`. Pro `(firm, market)`:

```rust
let value_added = value_added_for(revenue, outlays, firm, market)?;
let wage = wage_for_revenue(value_added, labor_share)?;
let profit = value_added.checked_sub(wage)?; // >= 0 per labor_share <= 10_000

let (theta_bps, target) = match (policies.0.get(&firm), input_pools.0.get(&firm)) {
    (Some(policy), Some(pool)) => (policy.theta_bps as i128, wc_target(*policy, pool)?),
    _ => (dividend_share, Money::ZERO), // Bestandsverhalten: config-θ (10_000), kein Puffer
};
let intended = Money(i64::try_from((profit.0 as i128) * theta_bps / 10_000)
    .map_err(|_| EconomyError::Overflow)?);
let held = accounts.account(firm).available;
let distributable = Money((held.0 - target.0).max(0));
let covered = Money(intended.0.min(distributable.0));
```

Der bestehende Shortfall-Pfad (`MarketClearFailed`-Event bei `covered < intended`) gilt NUR noch, wenn `held < intended` UND kein Policy-Target die Differenz erklärt — bei Policy-Kappung ist das Zurückhalten GEWOLLT, kein Audit-Event. Konkret: Event nur pushen, wenn `covered.0 < intended.0.min((held.0).max(0))`. Beide-Legs-Transfer + Sentinel unverändert.

- [ ] **Step 5:** Tests → PASS (alle wages-Tests).
- [ ] **Step 6:** Commit: `feat(economy): θ-dividend with working-capital cap (ProducerPolicies)`

---

### Task 4: Leontief-Input-Orders + Teilnahme-Schranke

**Files:**
- Modify: `backend/crates/sim-core/src/economy/producers.rs`
- Modify: `backend/crates/sim-core/src/economy/systems.rs` (`generate_pool_orders_system` ruft die neue Fn nach `generate_pool_orders_at_tick`)

- [ ] **Step 1: Failing Tests** — in `producers.rs` `mod tests` (Setup-Vorbild: `pools.rs`-Order-Tests; Helfer für World mit `MarketGoodState` + EWMA kopieren):

```rust
#[test]
fn input_order_sizes_to_batches_target_minus_held() {
    // batches_target=2, in_qty=10, held=5 → desired=15; Bid über create_bid mit qty 15
}

#[test]
fn input_order_skipped_when_stocked() {
    // held=20 >= 2*10 → kein Bid, kein Event, Cursor trotzdem gestempelt
}

#[test]
fn participation_bound_formula() {
    // p_out_ref=1000, labor_share=6000, out_qty=10, in_qty=10
    // → max_price == floor(1000 * 4000/10_000 * 10/10) == 400
}

#[test]
fn input_order_rejected_without_funds() {
    // cash=0 → OrderRejected{reason: InsufficientFunds} im Ledger
}

#[test]
fn zero_reference_price_is_honest_error() {
    // ewma == 0 → Err(ZeroPrice), kein Default
}
```

- [ ] **Step 2:** FAIL bestätigen.
- [ ] **Step 3: Implementation** in `producers.rs`:

```rust
/// Participation bound (§5.4): never bid more per input unit than the expected
/// output covers AFTER the labor share — keeps the wage flow payable at any
/// accepted price. floor(p_out_ref * (10_000 − labor_share) / 10_000 * out_qty / in_qty).
pub(crate) fn participation_bound(
    p_out_ref: Money,
    labor_share_bps: i128,
    out_qty: Quantity,
    in_qty: Quantity,
) -> Result<Money, EconomyError> {
    if p_out_ref.0 <= 0 {
        return Err(EconomyError::ZeroPrice);
    }
    if in_qty.0 <= 0 || out_qty.0 <= 0 {
        return Err(EconomyError::InvalidOrder);
    }
    let raw = (p_out_ref.0 as i128) * (10_000 - labor_share_bps) / 10_000
        * (out_qty.0 as i128) / (in_qty.0 as i128);
    Ok(Money(i64::try_from(raw).map_err(|_| EconomyError::Overflow)?))
}

/// Leontief derived demand: per input pool (keys-first), rewrite max_price from the
/// participation bound, size desired = batches_target*in_qty − held (floor 0), cap by
/// affordability, place a bid via the SAME create_bid path as consumer pools. Mirrors
/// the structure of generate_pool_orders_at_tick (incl. dormant-market skip + cursor
/// stamping + OrderRejected on zero-capped).
#[allow(clippy::too_many_arguments)]
pub fn run_generate_input_orders_at_tick(
    accounts: &mut AccountBook,
    orders: &mut OrderBook,
    inventory: &InventoryBook,
    ledger: &mut TradeLedger,
    dirty: &mut DirtyMarketGoods,
    next: &mut NextOrderId,
    input_pools: &mut InputPools,
    policies: &ProducerPolicies,
    market_goods: &MarketGoods,
    config: &EconomyConfig,
    current_tick: u64,
    ttl_ticks: u64,
    dormant: &BTreeSet<MarketId>,
) -> Result<(), EconomyError> {
    let labor_share = config.validated_labor_share_bps()?;
    let actors: Vec<EconomicActorId> = input_pools.0.keys().copied().collect();
    for actor in actors {
        let mut pool = input_pools.0[&actor];
        if dormant.contains(&pool.market) {
            continue;
        }
        if !crate::economy::pools::interval_elapsed(pool.last_generated_tick, current_tick, pool.interval_ticks) {
            continue;
        }
        let policy = policies
            .0
            .get(&actor)
            .copied()
            .expect("input pool actor must have a producer policy (seeded together)");
        let p_out_ref = market_goods
            .0
            .get(&MarketGoodKey { market: pool.market, good: pool.out_good })
            .ok_or(EconomyError::ZeroPrice)?
            .ewma_reference_price;
        pool.max_price = participation_bound(p_out_ref, labor_share, pool.out_qty, pool.in_qty)?;
        let held = inventory.balance(actor, pool.good).available;
        let target = Quantity((policy.batches_target as i64).checked_mul(pool.in_qty.0).ok_or(EconomyError::Overflow)?);
        let desired = Quantity((target.0 - held.0).max(0));
        if desired.0 > 0 && pool.max_price.0 > 0 {
            let affordable = crate::economy::pools::affordable_qty(accounts.account(actor).available, pool.max_price)?;
            let capped = Quantity(desired.0.min(affordable.0));
            if capped.0 <= 0 {
                ledger.0.push(EconomyEvent::OrderRejected {
                    actor,
                    market: pool.market,
                    good: pool.good,
                    reason: EconomyError::InsufficientFunds,
                });
            } else {
                create_bid(accounts, orders, ledger, dirty, next, current_tick,
                           actor, pool.market, pool.good, capped, pool.max_price, ttl_ticks)?;
            }
        }
        pool.last_generated_tick = Some(current_tick);
        input_pools.0.insert(actor, pool);
    }
    Ok(())
}
```

(Sichtbarkeiten: `interval_elapsed`/`affordable_qty` sind `pub(crate)` in `pools.rs` — Importe entsprechend.) `interval_ticks` der InputPools = 1 beim Seed (jeden Tick nachbestellbar; das Lager-Target dämpft die Frequenz von selbst). In `systems.rs` ruft `generate_pool_orders_system` die neue Fn direkt nach `generate_pool_orders_at_tick` im selben `?`-Fluss.

- [ ] **Step 4:** Tests → PASS.
- [ ] **Step 5:** Commit: `feat(economy): Leontief input orders with participation bound`

---

### Task 5: `ProducerSpec` in markets.json + Seed + Validierung + Persistenz

**Files:**
- Modify: `backend/crates/sim-core/src/base_world.rs` (MarketLayer + ProducerSpec)
- Modify: `backend/crates/sim-core/src/economy/markets_layer.rs` (Seed + Validierung + Re-Apply)
- Modify: `backend/crates/sim-core/src/economy/production.rs` (Konstanten: `EXTRACTOR_TOOLS` → Doku anpassen, neuer `pub const EXTRACTOR_WOOD: EconomicActorId = EconomicActorId(8_041);`)
- Modify: `backend/crates/sim-core/src/economy/persist.rs` (`input_pools`-Feld, non-serde-default)
- Modify: `data/worlds/abutopia/layers/markets.json`
- Modify: `backend/crates/sim-core/tests/abutopia_bundle.rs`

- [ ] **Step 1: Schema** — `base_world.rs`, neben `ExtractorSpec`:

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProducerSpec {
    pub actor: u64,
    pub market: u32,
    pub in_good: u16,
    pub in_qty: i64,
    pub out_good: u16,
    pub out_qty: i64,
    pub qty: i64,        // sell-side offered_qty_per_tick (wie ExtractorSpec)
    pub min_price: i64,  // sell-side Reservationspreis
    pub theta_bps: u16,
    pub batches_target: u32,
    pub opening_cash: i64,
}
```

`MarketLayer` bekommt `#[serde(default)] pub producers: Vec<ProducerSpec>,` (Welt-Layer-Präzedenz `capita_baseline`; die Test-Fixtures in `base_world.rs:~835` und `seed.rs:~845` brauchen dadurch KEINE Änderung — verifizieren, sonst `producers: Vec::new()` ergänzen).

- [ ] **Step 2: Failing Seed-Tests** — in `markets_layer.rs` `mod tests` (Vorbild: bestehende Extractor-Seed-Tests):

```rust
#[test]
fn producer_seed_creates_all_five_pieces() {
    // ProducerSpec → ProductionPool(recipe in→out), InputPool, ProducerPolicy,
    // SupplyPool(out_good, qty, min_price), opening_cash auf dem Konto
}

#[test]
fn producer_validation_rejects_raw_input() {
    // in_good == 5 (GOOD_RAW) → seed panics/Err mit klarer Meldung
}

#[test]
fn producer_validation_rejects_zero_batches() {
    // batches_target == 0 → Err; theta_bps > 10_000 → Err; in_qty/out_qty <= 0 → Err
}

#[test]
fn producer_validation_requires_input_supply_path() {
    // kein Supply/Extractor mit out_good == in_good irgendwo → Err (Kette tot ab Seed)
}

#[test]
fn producer_policies_reapplied_over_persisted_state() {
    // Re-Apply-Pfad: Policies werden aus dem Layer neu gebaut, auch wenn ein Snapshot existiert
    // (Muster: capita_baseline-Re-Apply — gleiche Stelle, grep "re-apply"/"reapply" markets_layer.rs)
}
```

- [ ] **Step 3:** FAIL bestätigen.
- [ ] **Step 4: Implementation** in `markets_layer.rs::seed_from_markets_layer` (+ Validierungsblock am Anfang, Fehler über das vorhandene Fehler-/Panic-Muster der Datei — ansehen und exakt spiegeln):
  - Validierung: wie Tests in Step 2; zusätzlich `market` existiert in `layer.markets`.
  - Seed pro `ProducerSpec`: `ProductionPools.insert(actor, ProductionPool { recipe: Recipe { inputs: vec![(GoodId(in_good), Quantity(in_qty))], outputs: vec![(GoodId(out_good), Quantity(out_qty))] }, interval_ticks: 1, last_generated_tick: None })`; `InputPools.insert(actor, InputPool { actor, market, good: in_good, in_qty, out_qty, out_good, interval_ticks: 1, last_generated_tick: None, max_price: Money::ZERO })`; `ProducerPolicies.insert(...)`; SupplyPool wie beim Extractor-Seed; `accounts.deposit(actor, Money(opening_cash))` über das vorhandene Seed-Mint-Muster (ansehen: wie bekommen Demand-Actors ihr opening_cash? exakt gleich machen — Seed-Mint ist der EINZIGE erlaubte Nicht-Transfer).
  - Re-Apply: an der Stelle, die heute `capita_baseline`/Opening-Prices re-applied (Zeile ~237ff), zusätzlich `ProducerPolicies` aus dem Layer NEU bauen (unconditional overwrite — authored ist Wahrheit).
- [ ] **Step 5: Persistenz** — `persist.rs`: `pub input_pools: Vec<(EconomicActorId, InputPool)>,` in `EconomyPersistSnapshot` (KEIN serde-default), `extract_from_world`/`apply_into_world` symmetrisch zu `demand_pools`. ⚠️ Deploy-Hinweis ins PR-Template: einmaliges `DELETE FROM economy_snapshots`.
- [ ] **Step 6: Daten** — `data/worlds/abutopia/layers/markets.json`:
  - `extractors`: Eintrag `{ "actor": 8031, ... }` ENTFERNEN; neu `{ "actor": 8041, "market": 9003, "in_good": 5, "out_good": 2, "qty": 10, "min_price": 500 }`.
  - Neu `"producers": [ { "actor": 8031, "market": 9001, "in_good": 2, "in_qty": 10, "out_good": 4, "out_qty": 10, "qty": 10, "min_price": 500, "theta_bps": 8000, "batches_target": 2, "opening_cash": 1000000 } ]`.
  - `opening_prices`: `{ "market": 9003, "good": 2, "price": 500 }` und `{ "market": 9001, "good": 2, "price": 500 }` ergänzen (Teilnahme-Schranke bei p_tools_ref=1000, ls=6000: max_price=400 — Schranke liegt UNTER 500: absichtlich? NEIN → Opening-Preis WOOD auf 300 setzen, damit die Kette ab Tick 1 handeln kann und #85 den Preis frei findet. Im Plan-Review nachrechnen: `400 ≥ p_src(300) + rate·dist(9003→9001)`? `transport_cost_per_tile_unit` aus `EconomyConfig::default()` ablesen und die Distanz 9003→9001 aus `market_distances` — wenn die Schranke reißt, Opening-Preis weiter senken und im Commit dokumentieren).
  - `abutopia_bundle.rs`: Asserts für die neue Sektion (1 Producer, 8041 in extractors, WOOD-Preise vorhanden).
- [ ] **Step 7:** Alle sim-core-Tests → PASS. Commit: `feat(economy): ProducerSpec seed — 8031 buys WOOD from new extractor 8041`

---

### Task 6: Konservierungs-, Hydrate- und Stationaritäts-Tests (Kette end-to-end)

**Files:**
- Create: `backend/crates/sim-core/tests/economy_production_chain.rs`

- [ ] **Step 1: Konservierung über N Ticks**

```rust
#[test]
fn chain_conserves_money_byte_exact_over_200_ticks() {
    // World aus data/worlds/abutopia seeden (Pfad-Muster aus seed.rs::workspace_root()),
    // 200 Ticks schedule.run(); nach JEDEM Tick: total_money == baseline (das #78-Audit
    // feuert ohnehin — der Test beweist es auf dem echten Welt-Seed mit aktiver Kette).
    // Zusätzlich: HOUSEHOLD_SECTOR.available == 0 nach jedem Tick.
}

#[test]
fn chain_conserves_goods_ledger_identity() {
    // Σ Produced(WOOD)+Regenerated(WOOD) − Σ Consumed(WOOD) == Δ total WOOD inventory;
    // analog TOOLS mit FinalConsumed. Ledger-Tail über mehrere Ticks akkumulieren.
}
```

- [ ] **Step 2: Hydrate-Pfad (Lektion #86)**

```rust
#[test]
fn chain_survives_persistence_round_trip_mid_run() {
    // 50 Ticks → extract_from_world → frische World → apply_into_world → 50 weitere Ticks:
    // InputPool-Cursor erhalten, Produktion läuft weiter (Produced(TOOLS)-Events nach Resume),
    // ProducerPolicies NACH apply re-applied (Re-Apply-Pfad des Runtime-Konstruktors nutzen,
    // nicht nur das nackte apply_into_world — das war der #86-Fehler).
}
```

- [ ] **Step 3: Stationarität (Anti-Blocker-2)**

```rust
#[test]
fn wood_price_converges_and_firm_cash_is_bounded() {
    // 2_000 Ticks. Danach:
    // (a) ewma(WOOD @9001) in Band um p_src(WOOD @9003) + rate·dist (±20%, Muster aus
    //     dem #85-Band-Test — grep "band" in economy-Tests und exakt das Muster nutzen),
    // (b) cash(8031) <= wc_target + intended-Dividende eines Intervalls (kein monotoner Drift:
    //     max über die letzten 500 Ticks vergleichen, nicht nur Endwert),
    // (c) Σ FinalConsumed(TOOLS) der letzten 500 Ticks > 0 (Kette liefert stationär).
}
```

- [ ] **Step 4:** Tests → PASS (laufzeit-bewusst: 2_000 Ticks sim-core sind ok; KEIN `--workspace --all-targets` während Iteration).
- [ ] **Step 5:** Commit: `test(economy): chain conservation, hydrate-path resume, long-run stationarity`

---

### Task 7: Wire + Inspector (additiv) + Browser-Smoke

**Files:**
- Modify: `backend/crates/protocol/proto/abutown.proto`
- Modify: `backend/crates/protocol/src/lib.rs` (DTO) + `backend/crates/sim-server/src/app/proto_convert.rs` + die EconomySnapshot-Builder-Stelle (`grep -rn "EconomySnapshot" backend/crates/sim-server/src` → wo markets/goods befüllt werden)
- Modify: `src/backend/economyState.ts` (per `ls src/backend` verifizieren) + `src/render/inspectorPanelPainter.ts`
- Regenerate: `npm run generate:proto` + Rust-Build (prost via build.rs)

- [ ] **Step 1: Proto (additiv, frische Tags)**

```proto
message EconomySnapshot {
  uint32 protocol_version = 1;
  string world_id = 2;
  uint64 tick = 3;
  repeated EconomyMarket markets = 4;
  repeated EconomyMarketGood goods = 5;
  repeated EconomyProducer producers = 6;
}

message EconomyProducer {
  reserved 100 to max;
  uint64 actor_id = 1;
  uint32 market_id = 2;
  uint32 in_good = 3;
  uint32 out_good = 4;
  int64 retained_earnings = 5; // firm cash available; client divides by ECONOMY_SCALE
  int64 wc_target = 6;
  int64 max_bid = 7;           // current participation bound (telemetry)
}
```

`npx buf lint` + `npx buf breaking --against '.git#branch=origin/main,subdir=.'` → grün (rein additiv).
- [ ] **Step 2: Backend-Befüllung** — Builder iteriert `InputPools` (keys-first): `retained_earnings = accounts.account(actor).available`, `wc_target` über die Task-3-Fn, `max_bid = pool.max_price`. DTO + proto_convert symmetrisch zu `EconomyMarketGood` (bestehendes Muster kopieren). Test im bestehenden Snapshot-Builder-Testmodul: Producer erscheint mit korrekten Werten.
- [ ] **Step 3: Frontend** — `npm run generate:proto`; `economyState.ts`: producers-Array in den State decodieren (Muster `goods`); `inspectorPanelPainter.ts`: wenn der angeklickte Markt einen Producer hostet, drei Zeilen rendern: `Rezept: 10 WOOD → 10 TOOLS`, `Kasse/Ziel: <retained>/<target>`, `Max-Gebot: <max_bid>` (Formatierung wie bestehende Preiszeilen, ECONOMY_SCALE-Division übernehmen). vitest-Test neben den bestehenden economyState-Tests (`src/backend/economyState.test.ts`).
- [ ] **Step 4: Browser-Smoke (Pflicht, CLAUDE.md)** — Dev-Stack starten (Port 8080 vorher prüfen: `lsof -nP -iTCP:8080 -sTCP:LISTEN`), headless-chromium-Smoke nach dem Muster `scripts/smoke-7a.mjs`: verbinden, EconomySnapshot-Frames empfangen, assert `producers.length == 1` mit `actor_id == 8031` und `max_bid > 0`; zusätzlich nach ~30s: mindestens ein Flow-Trader-Agent zwischen 9003→9001 sichtbar (`sprite_key` mit `trader:`-Präfix in den Mobility-Frames). Render-smoke-Pins (`tests/e2e/render-smoke.spec.ts` pinnt exakte Agentenzahlen) prüfen und bewusst aktualisieren, falls WOOD-Trader die Counts ändern.
- [ ] **Step 5:** Commit: `feat(economy): producer telemetry on the wire + inspector lines`

---

### Task 8: Full CI-Gate + PR

- [ ] **Step 1:** `pgrep -f cargo` (Orphans klären), dann: `scripts/cargo-serial.sh fmt --all -- --check` && clippy `--workspace --all-targets -- -D warnings` && `test --workspace` (im Hintergrund, seriell).
- [ ] **Step 2:** Frontend: `npx tsc --noEmit -p tsconfig.typecheck.json` && `npx vitest run` && `npm run build` && `npm run test:e2e`. `npx buf lint` + breaking.
- [ ] **Step 3:** PR mit Deploy-Hinweis (⚠️ einmaliges `DELETE FROM economy_snapshots` vor Deploy; Frontend dist/ lokal bauen → Vercel static). Alle Checks GRÜN abwarten (`gh pr checks --watch`, nie bei pending mergen), squash-merge, Branch + Worktree aufräumen, Memory aktualisieren.
