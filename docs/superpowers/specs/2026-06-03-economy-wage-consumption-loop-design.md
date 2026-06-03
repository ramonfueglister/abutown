# Economy Slice: SFC-Lohn-/Konsum-Kreislauf — den Geldkreislauf schließen

**Status:** Design (Brainstorming abgeschlossen, wartet auf Lead-Review)
**Datum:** 2026-06-03
**Vorgänger:** `2026-06-02-economy-consumption-design.md` (Konsum-Senke, §6 nennt die Einkommens-/Lohnquelle als „the next economic-realism gap")
**Branch:** `plan/economy-wage-loop` (auf `origin/main` 3274806)

---

## 1. Ziel

Der Geldkreislauf ist auf der Nachfrageseite offen. Geld wird **genau einmal** erzeugt (Bootstrap-Seed `Money(1_000_000)` je Consumer-Pool); danach ist jeder Fluss konservativ, und die drei Consumer-Aktoren (8_002 Tools, 8_012 Food, 8_022 Flow-Food) treten **nur als Käufer** auf — sie verdienen nirgends. Folge: Consumer-Cash sinkt monoton bis `OrderRejected{InsufficientFunds}`, der Handel friert ein, alles Geld sammelt sich in Verkäuferkonten und im nie-debitierten `TRANSPORT_OPERATOR`.

Dieser Slice schließt den Kreislauf **vollständig in einem Stück**: Firmen zahlen einen Lohnanteil ihres Umsatzes an den Haushaltssektor (Einkommensseite), und Haushalte konsumieren aus ihrem laufenden Einkommen (Konsumfunktion). Money bleibt **byte-invariant** (konservativ, kein Faucet), die Autorität ist **O(Sektoren)** und skaliert auf **1.000.000 Agenten**, sichtbare Pendler projizieren das Geschehen — analog zur etablierten Doktrin (#69–#72).

## 2. Das Modell (auf echten Papers)

- **Stock-Flow-Consistent accounting** (Godley & Lavoie 2007, *Monetary Economics*): jede Transaktion ist doppelte Buchung; Geld ist ein konservierter Bestand, kein exogener Zufluss. Die bestehende `AccountBook` (transfer/lock/release, `total_money` invariant) ist **bereits implizit SFC** — Löhne darauf zu bauen ist der no-cruft-Fit.
- **Lengnick (2013), *Agent-based macroeconomics: a baseline model* (JEBO)**: Haushalte sind bei Firmen angestellt, verdienen **Lohn**, und konsumieren **aus laufendem Einkommen**; der 1-Tick-Einkommen→Konsum-Lag ist die kanonische Periodenstruktur.
- **Labor share of value added** (Kaldor stylized fact; Karabarbounis-Neiman 2014): Firmen zahlen ~60% ihrer Wertschöpfung als Lohn.
- **Keynesianische Konsumfunktion** `C = a + b·Y`: autonomer Konsum `a` (Subsistenz, aus Vermögen finanziert) + marginale Konsumneigung `b` (MPC) mal Einkommen `Y`. Der positive Achsenabschnitt `a` bricht die Null-Falle.

**Bewusste v0-Vereinfachungen (mean-field, repräsentativer Haushalt):** kein individueller Arbeitsmarkt / Arbeitslosigkeit (Lohnbudget ist ein deterministischer Anteil, kein gematchter Job-Count); kein Profit-/Dividenden-Kanal (die nicht-Lohn-40% bleiben als einbehaltenes Firmen-Cash — späterer Slice); Wertschöpfung = Umsatz, weil die geseedeten Firmen keine Zwischengüter kaufen; Population skaliert das Sektor-Budget arithmetisch statt über Pro-Person-Heterogenität.

## 3. Architektur-Doktrin

Genau wie #69 (macro flow), #70 (flow-traders), #71 (shoppers), #73 (consumption sink):

- **Aggregate Autorität, viewport-unabhängig**: läuft jeden Tick für ALLE Sektoren, egal wer zuschaut. **O(Sektoren), nie O(Agenten).**
- **Sichtbare Agenten sind reine Projektionen**: render-only, nicht persistiert, regeneriert bei Neustart, **viewport-begrenzt**.
- **Determinismus**: fixed-point i64/i128, keys-first BTreeMap-Iteration, kein RNG, kein Float, floor-division, ties-by-ascending-index.
- **Frozen-time-Persistenz**: persistierte Cursor/Felder; kein offline catch-up; kein serde-default.

---

## 4. Teil A — Die Lohnseite (Einkommen rein)

### 4.1 SellerReceipts — Per-Tick-Umsatzerfassung (ephemer)

Neue Resource in `economy/wages.rs`:

```rust
/// Pro (Firma, Markt) der Brutto-Verkaufsumsatz, der DIESEN Tick gutgeschrieben
/// wurde. Eine nicht-monetäre Lauf-Statistik (KEIN Geldspeicher), jeden Tick
/// genullt, NIE persistiert. Key (actor, market) erhält die Markt-Dimension für
/// die Pendler-Attribution und vermeidet die Firma→Markt-Rückrechnung.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct SellerReceipts(pub BTreeMap<(EconomicActorId, MarketId), Money>);
```

**Schreibstellen** (die zwei bestätigten Settle-Punkte, an denen Verkäufer-Id *und* Markt *und* Betrag im Scope sind):

1. **Auktion** — `auction.rs`, unmittelbar nach `next_accounts.deposit(ask.owner, actual_cost)` (~:400): in einen **Scratch**-Akkumulator, der im **selben** `*accounts = next_accounts`-Commit-Block (~:432) übernommen wird → ein Fault, der den Clone verwirft, verwirft auch die Receipts (kohärent mit dem Geld). Key `(ask.owner, key.market)`.
2. **MacroFlow** — `settle_flow`, nach `accounts.deposit(*actor, receipt)` (~:646): `settle_flow` läuft gegen `scratch_accounts` und faltet nur im Ok-Zweig (~:984) zurück; den Receipts-Akkumulator im selben Ok-Zweig falten. Key `(src_seller, flow.src)`.

**Reset-Kadenz:** `.clear()` ganz am **Tick-Anfang** (neuer `EconomySet::ResetReceipts` vor `ExpireOrders`), erfasst exakt einen Tick Umsatz — spiegelt `run_consumption_at_tick`, das `consumed_qty_last_tick` reset-all-then-accumulate behandelt.

**Restore-sicher:** ephemer, jeden Tick genullt, **nie** in `EconomyPersistSnapshot`. Wir replayen NICHT den Audit-Ledger (dessen `MacroFlow`-Events tragen keinen Seller) — wir erfassen am Settle-Punkt, wo der Verkäufer bekannt ist. Nach Restore startet `SellerReceipts` leer und wird im ersten Post-Restore-Tick vor dem Lohn-System korrekt neu befüllt.

### 4.2 Lohnregel — labor share of value added

Neue **benannte** Konstante (keine magische 0.6) in `EconomyConfig`:

```rust
pub labor_share_bps: u16, // default 6_000 = 0.60; VALIDIERT 0..=10_000
```

**Wertschöpfung je Firma** = `SellerReceipts[(firm, market)]` (Brutto-Umsatz), weil die geseedeten Firmen keine Zwischengüter kaufen (Input-Kosten = 0). Die allgemeine Definition `value_added = revenue − input_cost` ist dokumentiert; der Input-Term ist in v0 null.

```rust
// keys-first über SellerReceipts; i128-Zwischenrechnung, floor, try_from
let wage_f = i64::try_from(
    (revenue.0 as i128) * (labor_share_bps as i128) / 10_000
).map_err(|_| EconomyError::Overflow)?;
```

`labor_share_bps ≤ 10_000` (validiert) ⇒ `wage_f ≤ revenue_f` ⇒ kein Overdraft (die Firma zahlt aus dem Cash, das sie gerade erhielt). Floor lässt den Rundungsrest bei der Firma (nie geprägt).

### 4.3 Konservativer Zwei-Bein-Transfer

Reservierter Sentinel `HOUSEHOLD_SECTOR = EconomicActorId(u64::MAX - 1)` (neben `TRANSPORT_OPERATOR = u64::MAX`) als Klärungskonto.

1. **Firma → HOUSEHOLD_SECTOR**: pro Firma `transfer(firm, HOUSEHOLD_SECTOR, Money(wage_f))`. Vor dem Commit jeder Firma: Affordability-Pre-Check (`available ≥ wage_f`); **`wage_bill` wird nur aus tatsächlich erfolgten Transfers aufsummiert** — schlägt ein Transfer fehl (darf per Invariante nicht passieren, da `wage_f ≤ gerade erhaltener Umsatz`), wird er auditiert (`EconomyEvent`), nicht stillschweigend verschluckt, und sein Anteil fließt nicht in `wage_bill`.
2. **HOUSEHOLD_SECTOR → Consumer-Pools**: `apportion_cash(&weights, wage_bill.0)` (largest-remainder, summenerhaltend, ties-by-ascending-index), dann pro Pool `transfer(HOUSEHOLD_SECTOR, consumer_i, Money(splits[i]))`. Da `apportion_cash` exakt summenerhaltend ist (`Σ splits == wage_bill` wenn `Σ weights > 0`), **nettet HOUSEHOLD_SECTOR jeden Tick auf null**.

**Guard gegen gestrandetes Cash:** wenn `Σ weights == 0` gibt `apportion_cash` `[0,…]` zurück und würde `wage_bill` im Sentinel stranden. Daher: **Seed-Assert** mindestens ein positives Gewicht, und das Lohn-System überspringt das erste Bein wenn `Σ weights == 0`. Zusätzlich ein **`debug_assert!(HOUSEHOLD_SECTOR.available == 0)`** nach PayWages — der einzige Fang für Sentinel-gestrandetes Cash, das `total_money` *nicht* auslöst.

### 4.4 Einkommens-Erfassung pro Haushalt

Neues Feld auf `DemandPool` (nach `last_consumed_tick`):

```rust
/// Lohn-Money, das dieser Haushalts-Pool im VORIGEN Tick erhielt (ein Perioden-
/// FLUSS, nicht der Kontostand). Jeden Tick auf ZERO genullt, bevor der Lohn-
/// Split akkumuliert. Treibt die Konsumfunktion (Teil B). `Money: Copy` hält
/// DemandPool `Copy`; persistiert frei im demand_pools-Vec.
pub income_last_tick: Money,
```

Geschrieben vom Lohn-System: Reset-all (keys-first `demand.0.values_mut()`), dann je erfolgreichem `transfer(HOUSEHOLD_SECTOR, consumer_i, share)` `pool.income_last_tick = pool.income_last_tick.checked_add(share)?`. **Konservierungs-Vertrag:** Einkommen wird NUR aus der `to`-Seite eines *abgeschlossenen* `transfer` gutgeschrieben — nie aus `src_revenue` vor dem Transfer, nie als `deposit` eines geprägten Anteils.

---

## 5. Teil B — Die Konsumfunktion (Lohn treibt Konsum)

### 5.1 Funktionsform (fixed-point)

```rust
// C_target = autonomous + (mpc_bps * income_last_tick) / 10_000
pub(crate) fn target_spend(autonomous: Money, mpc_bps: i32, income_last_tick: Money)
    -> Result<Money, EconomyError>
{
    if !(0..=10_000).contains(&mpc_bps) { return Err(EconomyError::InvalidOrder); }
    let induced = i64::try_from(
        (income_last_tick.0 as i128) * (mpc_bps as i128) / 10_000  // floor
    ).map_err(|_| EconomyError::Overflow)?;
    autonomous.checked_add(Money(induced))
}
```

Der **autonome Term `> 0`** bricht die Null-Falle (`income=0 → C=0 → keine Bids → kein Umsatz → kein Lohn → income=0`, absorbierend): er hält bei Nulleinkommen einen Boden-Bid lebendig, finanziert aus dem Vermögen (Subsistenz/Entsparen — Lengnick „consume even at zero current income").

### 5.2 Mengen-Abbildung

Ziel-Ausgabe (Money) → `desired_qty_per_tick` (Quantity) über einen Referenzpreis, invertiert zu `affordable_qty`s SCALE-Mathematik:

```rust
fn spend_to_qty(spend: Money, p_ref: Money) -> Result<Quantity, EconomyError> {
    if p_ref.0 <= 0 { return Err(EconomyError::ZeroPrice); }
    let raw = (spend.0 as i128) * ECONOMY_SCALE / p_ref.0 as i128; // floor
    Ok(Quantity(i64::try_from(raw).map_err(|_| EconomyError::Overflow)?))
}
```

**Referenzpreis = `ewma_reference_price`** (geglättet, `systems.rs` Telemetrie), Fallback `trader_default_ref_price` wenn 0. **Begründung (adversarialer Fund, gegen Code verifiziert):** die Consumer bieten an **Demand-only-Märkten** (m_b=9_002, m_fb=9_004); deren `last_settlement_price` wird nur vom Macro-Flow-Write-back auf `macro_flow_interval_ticks=10`-Intervallen geschrieben und ist dazwischen auf `bid_ceiling = max_price` gepinnt — eine **10-periodische Stufenfunktion**, die `desired_qty` oszillieren ließe. `ewma_reference_price` entfernt diesen Forcing-Term, sodass die 1-Tick-Lag-Rekurrenz eine echte Kontraktion erster Ordnung ist.

`desired_qty` ist nur ein **Ziel**; `generate_pool_orders_at_tick` cappt unverändert downstream: `min(desired_qty_per_tick, affordable_qty(available, max_price))`. Da wir am `p_ref` dimensionieren, aber am `max_price ≥ p_ref` locken, ist der Affordability-Cap konservativ — der Haushalt lockt nie mehr Cash als vorhanden.

### 5.3 Bootstrap (Tick 0 .. erster Lohn)

`income_last_tick` seedet auf `Money::ZERO` (explizit auf allen 3 Pool-Literalen, kein serde-default). Tick 0: `C_target = autonomous + MPC·0 = autonomous > 0` → Boden-Bid landet (cash-reich durch 1M-Seed) → clear gegen geseedete Supplier-Asks → Firmen-Umsatz → 60%-Lohn → `income_last_tick > 0` → Tick 1 springt `C_target` hoch. Kein Sonderfall-Code — der autonome Achsenabschnitt macht den Kaltstart zur normalen Auswertung bei `income=0`.

### 5.4 Timing (1-Tick-Lag, Lengnick-Periodenstruktur)

Neues `run_consumption_update_system` (`EconomySet::UpdateConsumption`): liest je Pool `income_last_tick` (gerade in Tick T gebucht) + `p_ref`, schreibt `pool.desired_qty_per_tick = spend_to_qty(target_spend(...), p_ref)`. Weil `GeneratePoolOrders` früher in Tick T lief, wird das neue `desired_qty` erst in **Tick T+1** zum Bid — der explizite 1-Tick-Einkommen→Konsum-Lag (vermeidet Same-Tick-Zirkularität).

---

## 6. Sichtbare Pendler-Projektion

`CommuterTrips` (Twin von `ShopperVisits`/`FlowShipments`) in `economy/commuters.rs`: Arbeiter, die Firma-Knoten ↔ Heim-Knoten entlang des Footway-Graphen laufen. **PURE VIEW** — kein Wirtschaftsstate, nicht persistiert, regeneriert bei Neustart. Reservierter Offset `COMMUTER_ACTOR_OFFSET = 3 << 32` (Shopper 2<<32, Flow-Trader 1<<32). Ephemeres `NextCommuterId` (reset 0 bei Restore).

**Telemetrie-Treiber:** ephemere `WageTelemetry`-Resource (NICHT auf `MarketGoodState` — vermeidet die Konstruktor-Fan-out + ein zusätzliches `DELETE`), pro Markt `wage_paid_last_tick: Money`, vom Lohn-System reset-all-then-accumulate geschrieben. `capture_commuter_trips` (pure, World-frei, unit-testbar) iteriert beobachtete Märkte mit `wage_paid_last_tick > 0`, `target = min(wage_paid / commuters_per_wage_unit, max_commuters_per_market)`, füllt den Shortfall aus einem deterministisch (NodeId) sortierten Heim-Knoten-Provider — exakt wie `capture_shopper_visits`.

**Cap absolut, nie aus Lohnmagnitude abgeleitet** (`max_commuters_per_market: usize`, z.B. 4 wie Shopper) — sonst koppelt die „viewport-bounded"-Garantie über die Lohnmagnitude an die 1M-Population.

---

## 7. Schedule (hart gepinnt)

Aktuelle Live-Kette (`systems.rs`): `RefreshLod → ExpireOrders → Production → GeneratePoolOrders → ClearMarkets → MacroFlow → Consume → ShopperCapture(excl) → Materialize(excl) → Telemetry`.

Einfügungen (unbedingt, nicht „später klären"):

| Set | Ordering | Begründung |
|---|---|---|
| `ResetReceipts` | `.before(ExpireOrders)` (Tick-Anfang) | SellerReceipts genullt, bevor Settles akkumulieren |
| `PayWages` | `.after(ClearMarkets).after(MacroFlow).before(Consume)` | beide Settle-Pfade haben Receipts gebucht; Sink draint Güter nach Lohn |
| `UpdateConsumption` | `.after(PayWages).after(Telemetry)` | liest `income_last_tick` (von PayWages) + das **finale** `ewma_reference_price` (von Telemetry geschrieben, daher zwingend danach) |

`PayWages` ist ein normales paralleles System (liest SellerReceipts + AccountBook + DemandPools, schreibt WageTelemetry + `income_last_tick`). `CommuterCapture` läuft `.after(PayWages)` (liest die frische WageTelemetry) und ist — wie `ShopperCapture` — ein **exklusives** System (Graph + NodeSpatialIndex + beobachtete Chunks). `UpdateConsumption` ist damit das letzte Wirtschafts-System vor `tick_increment_system`; sein neu gesetztes `desired_qty_per_tick` wird erst in `GeneratePoolOrders` von Tick T+1 zum Bid (der 1-Tick-Lag). Alle `.before(tick_increment_system)`.

## 8. Konservierung & Invarianten

`total_money` ist über den gesamten Loop **byte-invariant**: jede Geldbewegung ist `AccountBook::transfer` (paired `checked_sub`/`checked_add` auf `available`), nie mint/burn. Der Zwei-Bein-Pfad nettet HOUSEHOLD_SECTOR auf null. `SellerReceipts`/`WageTelemetry`/`income_last_tick` sind nicht-monetäre Statistiken (von `total_money()` ignoriert).

Die Konsumfunktion berührt **kein** Geldfeld — sie liest Einkommen/Preis und schreibt eine Quantity; Geld bewegt sich nur über die bestehenden, konservierungs-bewiesenen Pfade (lock_cash/release_cash/debit_locked/transfer).

**Tests müssen asserten** (im vollen Plugin-Tick, beide Settle-Pfade): (1) `total_money()` byte-gleich vor/nach PayWages und über den ganzen Tick; (2) `HOUSEHOLD_SECTOR.available == 0` nach PayWages; (3) keine Firma negativ; (4) `Σ income_last_tick eines Ticks == Σ Firma→Haushalt-Transferbeträge desselben Ticks`. Property-Test über zufällige `labor_share_bps ∈ [0,10_000]`, `mpc_bps ∈ [0,10_000]` und pathologische Receipts (null; Einzelfirma; `wage_bill < #Pools` → manche Splits runden auf 0; `labor_share_bps == 10_000`; All-Zero-Weights; population=1_000_000 mit max Seeds für Overflow).

## 9. Determinismus

Alle Iteration keys-first über BTreeMaps (`SellerReceipts`, `DemandPools`, `MarketGoods`). Fixed-point i64-Money/Quantity, i128-Zwischenrechnung, floor-division (einzige Rundungsregel), `try_from`→`Overflow`. Kein Float, kein RNG. `apportion_cash` ties-by-ascending-index. Pendler-Origin-Auswahl sortiert `NodeSpatialIndex`-Ergebnisse `sort_unstable_by_key(|n| n.0)` und nimmt First-N. Lohn-Ledger-Events (`WagePaid`) in aufsteigender Firmen-Id-Reihenfolge. **Determinismus-Regressionstest:** gleicher persistierter Snapshot + gleicher Tick, zweimal (und über serialize/deserialize-Roundtrip) ⇒ byte-identisches `desired_qty_per_tick` je Pool.

## 10. Persistenz & Migration

**Persistiert** (reitet bestehende `Vec<(K,V)>`-Snapshots, KEIN serde-default):
- `income_last_tick`, `mpc_bps`, `autonomous` auf `DemandPool` (im `demand_pools`-Vec).
- `HouseholdSector { population: u64, pool_weights: Vec<(EconomicActorId, i64)> }` als neues Top-Level-Snapshot-Feld.

**Ephemer** (nicht persistiert, default-leer bei Restore): `SellerReceipts`, `WageTelemetry`, `CommuterTrips`, `NextCommuterId`. Kein neuer Tick-Cursor (das Lohn-System ist tick-stateless über die ephemere SellerReceipts; die bestehenden `last_generated_tick`/`last_consumed_tick` gaten den Rest) — vermeidet die `LastProcessedMonth`-Replay-Bug-Klasse by design.

**Migration:** drei neue non-default `DemandPool`-Felder + ein neues Snapshot-Feld ⇒ alte `economy_snapshots`-Zeilen scheitern beim Deserialisieren (genau der #69-`market_distances`-Präzedenzfall). **Einmalig `DELETE FROM economy_snapshots` vor Deploy.** Alle Felder landen im **selben PR**, sodass eine einzige Migration alle abdeckt (kein zweiter destruktiver Wipe).

## 11. Skalierung auf 1.000.000

Die Lohn-Autorität iteriert nur `SellerReceipts` (O(verkaufende Firmen × Markt)) + die Handvoll Haushalts-Pools — **nie Pro-Agent-Entities** (kein `AgentIdIndex`, keine `FlowCells`). Die Million ist ein einzelnes `u64` in `HouseholdSector`, das das Sektor-Budget *arithmetisch* parametrisiert (i64-Mathematik, O(1) in der Zahl) und nie 1M Konten materialisiert — `AccountBook` bleibt O(Firmen + Pools + 2 Sentinels). Sichtbare Pendler werden nur in beobachteten Chunks materialisiert und absolut gecappt → eine Off-Screen-Million kostet null Strukturen.

*Ehrlich benannt:* der reset-all Telemetrie-Pass ist O(Märkte), und der Tick insgesamt skaliert mit der Marktzahl (wie macro flow) — nicht mit der Population. Nur die *Magnitude* der Zahlen skaliert mit 1M, nicht die Datenstrukturen.

## 12. Ehrliche Grenzen

- **Endowment-Wind-down:** v0-Firmen liquidieren ein endliches 1M-Güter-Endowment, produzieren nicht kontinuierlich. Mit sinkendem Bestand: Umsatz↓ → Lohn↓ → Einkommen↓ → Konsum→autonomer Boden, finanziert aus dem schrumpfenden Vermögen, bis `InsufficientFunds` (sauberer, auditierter Halt). Ein **dauerhaft selbsterhaltender** Loop braucht **kontinuierliche Produktion** — späterer Slice.
- **Stabilität:** weil Geld konservativ ist (`total_money` invariant) und `MPC·labor_share < 1` (Default 0.8·0.6 = 0.48), ist der Multiplikator `1/(1−0.48) ≈ 1.92` endlich; die 1-Tick-Lag-Rekurrenz mit Gain < 1 ist eine Kontraktion → monotone Konvergenz, kein Limit-Cycle, keine Geldexplosion (monetär inhärent beschränkt durch `total_money`).
- **Dormant-Kopplung:** `GeneratePoolOrders` überspringt dormant Märkte; der Bid-Pfad des Loops gilt nur für nicht-dormant Märkte (dormant werden von macro flow bedient). Die Lohn-/Konsum-*Autorität* läuft trotzdem für alle Pools.

## 13. Berührte Files

| File | Änderung |
|---|---|
| `economy/wages.rs` (NEU) | `SellerReceipts`, `HouseholdSector`, `HOUSEHOLD_SECTOR`, `run_pay_wages_at_tick` (pure core), labor-share fixed-point, apportion-Split |
| `economy/commuters.rs` (NEU) | `CommuterTrip(s)`, `NextCommuterId`, `COMMUTER_ACTOR_OFFSET`, `capture_commuter_trips`, `WageTelemetry` |
| `economy/pools.rs` | `target_spend`, `spend_to_qty`; `income_last_tick`/`mpc_bps`/`autonomous` auf `DemandPool`; `run_consumption_update_system`-Core |
| `economy/systems.rs` | `EconomySet::{ResetReceipts,PayWages,UpdateConsumption,CommuterCapture}`; Chain-Ordering; Wrapper-Systeme; `EconomyConfig::{labor_share_bps,commuters_per_wage_unit,max_commuters_per_market}` |
| `economy/auction.rs` | SellerReceipts an `:400` schreiben, im `next_accounts`-Block committen |
| `economy/macro_flow.rs` | SellerReceipts in `settle_flow` threaden, an `:646` akkumulieren, im Ok-Zweig falten |
| `economy/ledger.rs` | `EconomyEvent::WagePaid { firm, market, amount }` (+ event_type-Match) |
| `economy/persist.rs` | `household_sector`-Feld; extract/apply; KEIN serde-default |
| `economy/seed.rs` | `HouseholdSector` (population + weights); `income_last_tick`/`mpc_bps`/`autonomous` auf den 3 Pools |
| `economy/materialize.rs` | `CommuterTrips` als render-only walking agents zeichnen (ghost-free despawn) |
| `economy/mod.rs` | `pub mod wages/commuters`; Resources in `EconomyPlugin::install` |
| Tests | Konservierung (total_money byte-invariant, sector-nets-zero, Σincome==Σtransfers), Determinismus-Roundtrip, Stabilitäts-/Bootstrap-Smoke |

## 14. Offene Entscheidungen / empfohlene Defaults

Diese sind mit sinnvollen Defaults belegt; bitte beim Spec-Review bestätigen oder anpassen:

1. **MPC** `mpc_bps = 8_000` (0.8) — mit labor_share 0.6 Round-trip-Gain 0.48, Multiplikator ≈1.92, komfortabel stabil.
2. **labor_share** `labor_share_bps = 6_000` (0.60) — stylized fact.
3. **autonomer Konsum** `autonomous ≈ Money(5_000)` je Pool (≥ Money(1_000), damit `spend_to_qty(autonomous, default_ref=1_000)` ≥ 1 ganze Einheit ergibt) — gegen gewünschtes Demo-Aktivitätsniveau tunen.
4. **Parameter-Ort:** `mpc_bps` + `autonomous` als **Pro-Pool-Felder auf `DemandPool`** (Lengnick-treue Heterogenität, Symmetrie zu `urgency_bps`/`elasticity_bps`) vs. uniforme `EconomyConfig`-Konstanten. Empfehlung: Pro-Pool.
5. **Nicht-Lohn-40% (Dividenden-Kanal):** v0 lässt sie als einbehaltenes Firmen-Cash (Firmen akkumulieren — akzeptabel, da das Endowment ohnehin endlich ist). Dividenden-/Kapitalisten-Haushalt = späterer Slice. Bestätigen.
6. **Pool-Weights & population:** Gewichtung des Lohn-Splits über die 3 Pools (gleich / nach `desired_qty` / geseedete Allokation) + `HouseholdSector.population` (1_000_000 vs. aktueller geseedeter Agent-Count). Empfehlung: gleiche Gewichte v0, population = 1_000_000.
7. **`HOUSEHOLD_SECTOR = u64::MAX - 1`** (neben `TRANSPORT_OPERATOR = u64::MAX`) — Kollisions-Assert gegen geseedete Ids (8_001..8_022) und die Offset-Bänder.

## 15. Deferred (spätere Slices)

- **Kontinuierliche Produktion** (Firmen produzieren statt Endowment zu liquidieren) → dauerhaft selbsterhaltender Loop.
- **Expliziter Arbeitsmarkt** (Diamond-Mortensen-Pissarides Matching, Einstellen/Entlassen, Arbeitslosigkeit).
- **Profit-/Dividenden-Kanal** (Kapitalisten-Haushalt für die nicht-Lohn-Wertschöpfung).
- **Mehr Firmentypen** über Trader/Supplier hinaus.
- **Allgemeine Wertschöpfung** `value_added = revenue − input_cost` für Input-kaufende Produzenten.
- **Konsum-Preis-Elastizität** (Einkommen treibt auch `max_price`, derzeit fix).
