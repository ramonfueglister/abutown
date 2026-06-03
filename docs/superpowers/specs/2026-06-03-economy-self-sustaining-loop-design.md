# Economy Slice: Selbsttragender Wirtschaftskreislauf

**Status:** Design (Brainstorming abgeschlossen, wartet auf Lead-Review)
**Datum:** 2026-06-03
**Vorgänger:** `2026-06-03-economy-wage-consumption-loop-design.md` (#74 schloss den Geld-Kreis Lohn→Konsum); dessen §12 nennt „continuous production" als den nächsten Gap.
**Branch:** `plan/economy-self-sustaining` (frisch auf `origin/main` 5a32336)

---

## 1. Ziel

Der Geld-Kreis ist geschlossen (#74), **aber der Loop wickelt sich ab** — durch zwei *unabhängige* Lecks:

1. **Güter-Leck:** Supplier liquidieren ein endliches `Quantity(1_000_000)`-Endowment, die Senke (`FinalConsumed`) zerstört Güter, `ProductionPools` ist ungeseedet/inert → Güter erschöpft in ~100k Ticks → Trades verhungern.
2. **Geld-Leck:** nur `labor_share` (60%) des Umsatzes fließt als Lohn zurück; die restlichen **40% (Profit) stranden in den Verkäuferkonten** (Verkäufer haben nur `SupplyPool`, nie `DemandPool` → bieten nie, geben nie aus); zusätzlich wird `TRANSPORT_OPERATOR` Gebühren gutgeschrieben, aber **nie belastet** (ein Geld-Sink). Haushaltsbargeld drainiert pro Zyklus zum Autonomous-Floor bis `InsufficientFunds` — **egal wie viele Güter da sind.**

Diese Slice schließt **beide** Lecks in einem Bogen (3 Sub-Slices, ein PR), sodass der Loop **wirklich selbsttragend** wird: kontinuierliche Güter-Quelle + 100% des Umsatzes zurück an konsumierende Haushalte + Recycling der Transport-Gebühren. `total_money` bleibt **byte-invariant**, Güter werden ledger-auditierbar, die Autorität ist O(Sektoren) und viewport-unabhängig.

## 2. Das Modell (auf echten Papers)

- **Circular flow of income (geschlossen):** in einem geschlossenen Kreislauf mit fixem Geldbestand `M` muss alles, was Haushalte ausgeben, vollständig zu ihnen zurückkehren. Firmen dürfen kein Geld dauerhaft einbehalten → **100% des Umsatzes** wird ausgeschüttet (Lohn + Profit), Firmen netten pro Tick auf null. (Lead-Entscheidung: Profit geht an die *bestehenden* Haushalte — **keine Kapitalisten-Klasse** in v0; bewusste Vereinfachung.)
- **Stock-Flow-Consistent (Godley & Lavoie 2007):** jeder Geldzug ist doppelte Buchung (`AccountBook::transfer`), `total_money` per Konstruktion invariant.
- **Kontinuierliche Produktion (primär/extraktiv):** eine *stehende* Güterzufuhr ersetzt die einmalige Endowment. Eine nicht-handelbare Ressource `GOOD_RAW` wird periodisch deponiert (flow-capped) und über eine `RAW→Handelsgut`-Rezeptur input-gated in handelbare Güter umgesetzt — ehrliche, gedeckelte Quelle, kein unbeschränkter Faucet, Preissignal bleibt lebendig.
- **Keynesianischer Multiplikator < 1:** mit Voll-Ausschüttung (payout share = 1.0) und `MPC = 0.8` ist die Per-Runde-Verstärkung `1.0·0.8 = 0.8 < 1` → die 1-Tick-Lag-Rekurrenz kontrahiert, Gleichgewicht `Y* = autonomous/(1−MPC)`, Netto-Sparen → 0 im steady state. **Der Multiplikator ist Heuristik; die Garantie ist die Konservierungs-Invariante + ein empirischer steady-state-Test (§8).**

## 3. Architektur-Doktrin

Wie #69–#74: aggregate Autorität O(Sektoren), viewport-unabhängig (läuft für ALLE, egal wer zuschaut); deterministisch (fixed-point i64/i128, keys-first BTreeMap, kein RNG/Float, floor-div, ties-by-ascending-index); **NULL Fallbacks/silent-defaults — ehrliche Error-Codes / `.expect` statt `let _`** wenn ein Invariantenbruch auftritt; frozen-time-Persistenz; kein serde-default.

---

## 4. Teil A — Güter-Quelle (kontinuierliche Produktion)

### 4.1 GOOD_RAW (strukturell nicht-handelbar)
`pub const GOOD_RAW: GoodId = GoodId(5);` in `goods.rs` (nächste freie u16, auto-re-exportiert). RAW wird **strukturell nie** in einen `SupplyPool`/`DemandPool`/Market-Seed konstruiert — es gibt keinen Listing-Pfad, also kann es nie in `OrderBook`/`MarketGoods` gelangen. Nicht-Handelbarkeit ist *erzwungen* (kein Laufzeit-Guard).

### 4.2 EXTRACTOR + RawDeposits (flow-capped faucet)
- `pub const EXTRACTOR: EconomicActorId = EconomicActorId(8_031);` — **ein** benannter Förderer (nicht N verstreute Faucets).
- `RawDeposit { good: GoodId, qty_per_interval: Quantity, interval_ticks: u64, last_regen_tick: Option<u64> }` + `RawDeposits(BTreeMap<EconomicActorId, RawDeposit>)` (Resource, persistiert; spiegelt `ProductionPool`).
- `run_regen_at_tick(inventory, ledger, deposits, current_tick) -> Result<(), EconomyError>`: keys-first, `interval_elapsed`-Gate (aus `pools` wiederverwendet), bei Fire `inventory.deposit(actor, good, qty_per_interval)` + `ledger.push(Regenerated { actor, good, qty })`, stempelt `last_regen_tick`.

**Ehrliches Wording:** Das ist ein **flow-capped faucet** — `run_regen_at_tick` deponiert `qty_per_interval` *ohne* den RAW-Bestand zu lesen. RAW bleibt nur beschränkt (`≤ 2·qty_per_interval`), weil die Rezeptur es konsumiert, sobald `≥ qty` vorhanden ist. (Kein „capacity-capped stock" behaupten, was der Code nicht tut.)

### 4.3 RAW→Handelsgut über das BESTEHENDE `run_production_at_tick` (unverändert)
EXTRACTOR erhält eine `ProductionPool { recipe: { inputs: [(GOOD_RAW, q)], outputs: [(GOOD_TOOLS, q)] }, interval_ticks: 1 }`. Der atomare all-inputs-covered-Check von `run_production_at_tick` heißt: bei RAW-Knappheit drosselt der Output **ehrlich auf null** in diesem Tick (input-gated, kein Minting). Das Handelsgut fließt über den bestehenden `SupplyPool`-Pfad (`generate_pool_orders_at_tick` bietet `min(offered, available)`).

### 4.4 Schedule
Neuer `EconomySet::Regenerate` **zwischen `ExpireOrders` und `Production`** (RAW dieses Ticks ist sofort konsumierbar). `Regenerated`-Event + `event_type()`-Arm `"regenerated"`.

---

## 5. Teil B — Profit-Verteilung an die Haushalte (kein Kapitalist)

Damit Firmen **nichts** einbehalten, wird der Profit (Umsatz − Lohn) **vollständig** an die drei bestehenden Konsumenten-Haushalte (8_002/8_012/8_022) ausgeschüttet — dieselben `pool_weights` + `apportion_cash` wie der Lohn. Effektiv: 100% des Umsatzes zurück an Haushalte, Firmen netten pro Tick auf null. **Kein OWNER-Pool, keine `pool_weights`-Änderung.**

- `dividend_share_bps: u16` auf `EconomyConfig`, **default `10_000` (Voll-Ausschüttung)**, validiert `0..=10_000` via `validated_dividend_share_bps()` (analog `validated_labor_share_bps`). (Lead-Default = voll; ein kleinerer Wert ließe Profit stranden und wäre dann NICHT selbsttragend — §15.)
- `run_distribute_profit_at_tick(accounts, receipts: &SellerReceipts, demand, household, ledger, config) -> Result<(), EconomyError>`: keys-first über `SellerReceipts`; je `(firm, market)`: `wage = wage_for_revenue(revenue, labor_share)` (rekomputiert, gleiches Flooring), `profit = revenue − wage`, `dividend = floor(profit · dividend_share_bps / 10_000)` (i128, `try_from → Overflow`). Bei `dividend > 0`: **fallibler** Zwei-Bein-Transfer `firm → HOUSEHOLD_SECTOR`, dann `apportion_cash` über `pool_weights` → `HOUSEHOLD_SECTOR → Haushalte`, `income_last_tick += share`, `ProfitDistributed { firm, market, amount }`-Event.

**Fallible, nicht `.expect`-Panic (Fix D1):** Eine Firma, die im selben Tick verkauft UND (via macro_flow) kauft, kann zum Verteilungs-Zeitpunkt einen Saldo `< profit` haben. Statt eines latenten Prozess-Panics: die Funktion propagiert `Result`; bei `InsufficientFunds` wird **nur der gedeckte Betrag** gebucht und der Fehlbetrag **laut über ein audited Ledger-Event** (`MarketClearFailed`-artig, wie der macro_flow-Wrapper in `systems.rs`) surfaced — kein stiller Drop, kein Panic. Der Bevy-Wrapper schluckt **nicht** mit `let _`, sondern surfaced den Fehler.

---

## 6. Teil C — Transport-Rebate

`run_transport_rebate_at_tick(accounts, demand, household, ledger) -> Result<(), EconomyError>`: entleert den **gesamten** `TRANSPORT_OPERATOR`-Saldo → `HOUSEHOLD_SECTOR`, dann `apportion_cash` über **dieselben `pool_weights` wie der Lohn** (die Käufer-Haushalte zahlten die Gebühr) → `income_last_tick += share`, `TransportRebate { amount }`-Event.

**Gating ohne Cursor (Fix D3):** Der Rebate wird auf **dasselbe** `current_tick.is_multiple_of(macro_flow_interval_ticks)`-Modulo gegated wie der `TRANSPORT_OPERATOR`-Credit in `macro_flow` (stateless, kein `TransportRebateCursor`). Credit und Drain bleiben phasen-gesperrt über Save/Restore, und es entfällt ein persistiertes Feld. Mid-Intervall darf der Operator-Saldo `> 0` sein; an jeder Intervallgrenze geht er auf null.

---

## 7. Konservierung & Invarianten

**Geld byte-invariant:** jeder neue Geldzug ist `AccountBook::transfer` (Profit: firm→HOUSEHOLD_SECTOR→Haushalte; Rebate: TRANSPORT_OPERATOR→HOUSEHOLD_SECTOR→Haushalte). Regeneration + Produktion sind **güter-only** (`inventory.deposit`/recipe). `total_money()` byte-gleich über den ganzen Tick.

**Drei separate Net-Zero-Asserts (Fix):** `HOUSEHOLD_SECTOR.available == 0` wird am Ende von **je** `run_pay_wages_at_tick`, `run_distribute_profit_at_tick`, `run_transport_rebate_at_tick` `debug_assert`-geprüft — drei unabhängige Asserts, NICHT eines „über drei Legs" (das Wage-Assert feuert vor dem `.after`-Profit-System).

**Güter-Bilanz (auditierbar):** `total_good(g)_after − total_good(g)_before == Σ(Regenerated_g + Produced_g) − Σ(Consumed_g + FinalConsumed_g)`. Für `GOOD_RAW`: nur `Regenerated` (Quelle) − `Consumed` (Rezeptur-Input). Für `GOOD_TOOLS`: `Produced` − `FinalConsumed` (+ trade-interne Moves, die auf null netten).

## 8. Stabilität — ehrlich

**Garantie = Konservierung + empirischer Test, nicht Algebra.** Der Multiplikator `(labor_share + dividend·(1−labor_share))·MPC = 1.0·0.8 = 0.8 < 1` ist eine **Heuristik**. Da Profit voll an *gefloorte, ausgebende* Haushalte geht (kein nicht-ausgebender Verkäufer, kein Horten-Owner), zirkuliert `M` vollständig; das Keynes-Gleichgewicht `Y* = autonomous/(1−MPC)` hat **Netto-Sparen 0** (Konsum = Einkommen), also kein unbeschränktes Horten. Güter erreichen steady state bei der Regen-Rate (Überschuss → EWMA-Preis steigt → `spend_to_qty` senkt Menge — negative Rückkopplung, im geklemmten Preisband).

**Ehrliche Vorbehalte (müssen in die Prosa + den Test):**
- Der Preis-Regulator ist **gedeckelt**: Order-Preise sind statisch (`max_price`/`min_price` im Seed), `settlement_price` klemmt nur in `[marginal_ask, marginal_bid]`; bei reiner Knappheit (kein Clear) wird `last_settlement_price` nicht geschrieben — der Preis kann nie über `max_price` steigen. Chronische Knappheit korrigiert sich NICHT über den Preis → Regen-Sizing (§15) muss die Nachfrage decken.
- Der autonome Floor ist nur nachhaltig, wenn der Loop *tatsächlich* geschlossen ist (Voll-Ausschüttung) — sonst ziehen die Floors den endlichen Seed-Bestand auf null.

**Nicht-vakuöser steady-state-Test (Fix S2) — der eigentliche Beweis:** `steady_state_multi_tick`:
- **EXTRACTOR als EINZIGER Supplier im Test-World** (die endlichen 1M-Endowments im Test droppen — sonst maskiert die Endowment den steady state ~100k Ticks lang und der Test wäre grün aus dem falschen Grund).
- `N ≥ 200` Ticks; vorab gepinnte `K` (z.B. letzte 50), Ratio `r`, `ε`.
- **Harte Asserts:** (a) `total_money()` jeden Tick exakt konstant; (b) **Verkäufer-Salden beschränkt** (`max−min` über die letzten K `< ε` — fängt das Unbounded-Retained-Earnings-Versagen; bei Voll-Ausschüttung netten sie ~0); (c) ein repräsentativer Konsumenten-**Konto-Saldo** (nicht nur `income_last_tick`) UND ein Markt-`traded_qty_last_tick` je in `[lo, hi]` mit `hi/lo < r` UND **`lo > 0`** (lebender, nicht eingefrorener steady state); (d) `total_good(GOOD_TOOLS)` beschränkt (nicht monoton wachsend/kollabierend).
- Schlägt fehl, wenn `total_money` driftet (Leck offen), eine Bilanz divergiert (Regen mis-sized), oder Salden zum Floor/Null kollabieren (Loop nicht geschlossen).

## 9. Determinismus & No-Fallback

Fixed-point i64/i128, floor, `try_from→Overflow`; keys-first über alle BTreeMaps (`RawDeposits`, `SellerReceipts`, `pool_weights`, `DemandPools`); `apportion_cash` (largest-remainder, ties-by-ascending-index). Neue `EconomyEvent`-Varianten erzwingen exhaustive `event_type()`-Arme. **Null Fallbacks:** `validated_dividend_share_bps()` → `InvalidOrder` bei `>10_000`; Profit-Verteilung **fallible mit auditiertem Surfacing** (kein `.expect`-Panic, kein stiller Drop — Fix D1); fehlende Pools/Preise → ehrlicher Error.

**Intra-Set-Ordering erzwungen (Fix D2):** `run_distribute_profit_system` läuft im PayWages-Set mit **expliziter `.after(run_pay_wages_system)`-Kante**, der Rebate `.after` beidem; ein **System-Ambiguity-Check** (Bevy) sichert, dass die drei Schreiber von `DemandPools.income_last_tick`/`AccountBook` eine deterministische Reihenfolge haben (nicht nur Set-Membership). Reihenfolge: `wage-reset → wage-credit → profit-credit → rebate-credit`.

## 10. Schedule (vollständige neue Kette)
`ResetReceipts → RefreshLod → ExpireOrders → Regenerate → Production → GeneratePoolOrders → ClearMarkets → MacroFlow → PayWages(wage→profit) → TransportRebate → Consume → ShopperCapture → CommuterCapture → Materialize → Telemetry → UpdateConsumption`. Profit ist KEIN neues Set (im PayWages-Set, `.after` wage). Rebate ist ein neues Set `.after(PayWages).before(Consume)` — Rebate-Einkommen ist vor `UpdateConsumption` (letztes) gebucht, gleicher 1-Tick-Lag wie Lohn.

## 11. Persistenz & Migration
- **Frei persistiert:** `ProductionPools` (EXTRACTOR-Rezeptur), `DemandPools`, `AccountBook` (EXTRACTOR-Saldo), `HouseholdSector`.
- **Neu persistiert:** nur **`raw_deposits: Vec<(EconomicActorId, RawDeposit)>`** in `EconomyPersistSnapshot` (spiegelt `production_pools`). **Kein** `transport_rebate_cursor` (Fix D3), **kein** Owner.
- `EconomyConfig` ist NICHT persistiert → `dividend_share_bps`/Regen-Params brauchen kein Snapshot-Feld; das steady-state-Band gilt nur für die einkompilierten Defaults (der Band-Test pinnt sie).
- **Eine** Snapshot-Änderung → **einmalig `DELETE FROM economy_snapshots`** vor Deploy (kein serde-default; #69/#73/#74-Disziplin). Seed (EXTRACTOR + RawDeposit) läuft nur auf frischen Worlds.

## 12. Skalierung auf 1.000.000
`run_regen_at_tick` O(|RawDeposits|) = ein EXTRACTOR; Profit-Verteilung O(|SellerReceipts|) (dieselbe Schleife wie Lohn); Rebate O(|pool_weights|). Nie O(Population), viewport-unabhängig (läuft für alle Sektoren jeden Tick). `HouseholdSector.population` bleibt inert.

## 13. Sub-Slice-Dekomposition (ein PR)
- **A (Güter-Quelle):** `GOOD_RAW`, `EXTRACTOR`, `RawDeposit`/`RawDeposits`, `run_regen_at_tick` + `Regenerated`, `EconomySet::Regenerate` vor Production, EXTRACTOR-Seed (RAW-Inventory + RawDeposit + RAW→TOOLS-ProductionPool + SupplyPool), `RawDeposits` in `mod.rs`. Tests: Output throttelt auf RAW-Verfügbarkeit; RAW nie gelistet/gehandelt. **Sizing-Sim hier**, bevor Konstanten fixiert werden.
- **B (Geld-Lecks):** `dividend_share_bps`(=10_000) + Validator; `run_distribute_profit_at_tick` (fallible) + `ProfitDistributed`; `run_transport_rebate_at_tick` (macro_flow-Modulo) + `TransportRebate`; `EconomySet::TransportRebate` + Profit-in-PayWages mit `.after`-Kante + Ambiguity-Check. Tests: Profit konserviert + drainiert Firmen-Cash auf null; Rebate entleert Operator + konserviert; **drei separate Net-Zero-Asserts**.
- **C (Persistenz + Konservierung + Stabilität):** `raw_deposits` in `EconomyPersistSnapshot` (ein DELETE); Persist-Round-Trip; `conservation_full_plugin_multi_tick` (Geld byte-invariant + Per-Gut-Bilanz); `steady_state_multi_tick` (§8, EXTRACTOR-only, Verkäufer-Bound, lebende Bänder).

## 14. Berührte Files (alle unter `backend/crates/sim-core/src/economy/`)
`goods.rs` (GOOD_RAW), `production.rs` (EXTRACTOR/RawDeposit/RawDeposits/run_regen_at_tick), `wages.rs` (dividend_share_bps-Validierung), `transport.rs` (run_transport_rebate_at_tick), `ledger.rs` (Regenerated/ProfitDistributed/TransportRebate + event_type-Arme), `systems.rs` (Regenerate + TransportRebate Sets, Profit-in-PayWages mit .after + Ambiguity-Check), `seed.rs` (EXTRACTOR-Seed), `persist.rs` (raw_deposits), `mod.rs` (RawDeposits registrieren), `tests/{conservation,production,persist}.rs`.

## 15. Offene Entscheidungen / Defaults
1. **`dividend_share_bps = 10_000`** (Voll-Ausschüttung) — bestätigt durch die Lead-Wahl „Profit voll an Haushalte". (Ein kleinerer Wert wäre NICHT selbsttragend.)
2. **Regen-Sizing** (`regen_qty_per_interval`/`interval_ticks`): muss die aggregierte Nachfrage bei Seed-Preisen decken (statische Preise self-korrigieren chronische Knappheit nicht). Sizing-Sim in Sub-Slice A; entscheiden, ob ein zweiter RAW→FOOD-Extractor nötig ist oder FOOD bewusst auf der versiegenden Endowment bleibt. Vorschlag: `interval_ticks=1`, `regen_qty` an der Summe der TOOLS-Nachfrage; FOOD ggf. zweiter Extractor.
3. **Transport-Rebate an die Lohn-Haushalte** (vorgeschlagen, da die Käufer die Gebühr zahlten).
4. **EXTRACTOR neben den finiten Suppliern** (kleinerer Diff; die Endowments versiegen, EXTRACTOR wird die stehende Quelle) vs. die finiten Endowments gleich ersetzen. Vorschlag: daneben — **aber im steady-state-Test EXTRACTOR-only** (sonst Endowment-Maskierung).

## 16. Ehrliche Grenzen / Deferred
- **Keine Kapitalisten-Klasse** in v0 (Profit geht an die Arbeiter-Haushalte). Ein echter Dividenden-/Kapitaleinkommens-Kanal (Owner-Haushalt mit Anti-Horten-Konsumregel) ist ein späterer Bogen.
- **Kein Kapitalgüter-/Investitions-Sektor** (Firmen reinvestieren nicht; sie schütten voll aus). Späterer Bogen.
- **Statische Preise** (Order-`max_price`/`min_price` fix) — der Regulator ist gedeckelt; freie Preissetzung ist ein späterer Bogen.
- **Eine Primär-Ressource/ein Extractor** in v0; mehrstufige Produktionsketten (raw→intermediate→final) später.
