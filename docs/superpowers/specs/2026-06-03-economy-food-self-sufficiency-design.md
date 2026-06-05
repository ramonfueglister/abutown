# Economy: FOOD Self-Sufficiency — Design Spec

**Datum:** 2026-06-03
**Status:** Design (approved) → Plan
**Vorgänger:** PR #75 „self-sustaining loop" (kontinuierliche TOOLS-Quelle + 100 % Profit-Ausschüttung + Transport-Rebate). Diese Slice schließt die zweite Hälfte derselben Lücke.

## 1. Problem

PR #75 trägt den Titel „self-sustaining loop", aber der Kreislauf schließt sich **nur für TOOLS**. **FOOD läuft weiterhin aus.** FOOD sitzt auf endlichen Eröffnungs-Endowments ohne jede erneuerbare Quelle:

- `food_supplier 8_011`: `Quantity(1_000_000)` GOOD_FOOD, bietet 10/Tick @ `m_a` (`seed.rs:192-213`).
- `flow_supplier 8_021`: `Quantity(1_000_000)` GOOD_FOOD, bietet 10/Tick @ `m_fa` (`seed.rs:289-310`).

Bei aggregierter FOOD-Nachfrage von 20/Tick (zwei Konsumenten à 10) sind die 2 M Endowment in ~50–100 k Ticks erschöpft. Danach treffen die FOOD-Konsumenten-Pools `InsufficientFunds`, das stehende FOOD-Ungleichgewicht (Supply@FA → Demand@FB), auf dem die ganze MacroFlow-Demo beruht, kollabiert, und die „lebende Wirtschaft" degeneriert zu einem Ein-Gut-System. Vier von sieben Subsystem-Surveys haben dies unabhängig als **correctness-critical** markiert.

Das ist exakt derselbe Defekt, den #75 für TOOLS behoben hat — also eine saubere, kohärente Ein-Konzept-Slice, kein Sammelsurium.

## 2. FOOD-Topologie heute (zwei unabhängige Supply→Demand-Paare)

FOOD ist **kein** einzelner Versorgungspunkt, sondern zwei separate Paare auf je eigenem Markt:

| Paar | Supplier | Supply-Markt | Consumer | Demand-Markt | Zweck |
|------|----------|-------------|----------|-------------|-------|
| 1 (Multi-Gut-Auktion) | `8_011` | `m_a` (9_001) | `8_012` | `m_b` (9_002) | FOOD handelt auf denselben Märkten wie TOOLS |
| 2 (Cross-Market-MacroFlow) | `8_021` | `m_fa` (9_003) | `8_022` | `m_fb` (9_004) | stehendes räumliches Ungleichgewicht → wiederkehrender MacroFlow |

**Schlüssel-Fakt:** Ein `SupplyPool` ist an genau ein `(actor, market, good)` gebunden, und es gibt **einen Pool pro Actor**. Ein einzelner zentraler Extractor könnte also nur **einen** Markt versorgen — der andere FOOD-Markt liefe weiter aus (und die MacroFlow-Demo stürbe). FOOD-Selbstversorgung erfordert strukturell **eine erneuerbare Quelle pro FOOD-Supply-Markt**.

## 3. Design: zwei co-lokalisierte FOOD-Extractors

Spiegelt den bewährten #75-TOOLS-Extractor an **jedem** der zwei FOOD-Supply-Märkte:

- **`EXTRACTOR_FOOD_A = EconomicActorId(8_032)`** @ `m_a` — übernimmt, während Endlich-Supplier `8_011` drainiert (hält die Multi-Gut-Auktion lebendig).
- **`EXTRACTOR_FOOD_FA = EconomicActorId(8_033)`** @ `m_fa` — übernimmt, während Endlich-Supplier `8_021` drainiert (hält den MacroFlow-Showcase lebendig).

`8_032`/`8_033` sind **neue, freie** Actor-IDs, distinkt vom TOOLS-Extractor `8_031` und allen geseedeten Aktoren (`8_001`/`8_002`, `8_011`/`8_012`, `8_021`/`8_022`) — keine Map-Key-Kollision in `RawDeposits`/`ProductionPools`/`SupplyPools` (alle `BTreeMap<EconomicActorId, _>`, ein Pool pro Actor).

Jeder ist eine exakte Kopie des #75-Musters, neben dem Endlich-Supplier geseedet (drain-then-handover):

```
Eröffnungsbestand:  InventoryBook.deposit(actor, GOOD_RAW, REGEN_QTY_FOOD)
Faucet:             RawDeposit { good: GOOD_RAW, qty_per_interval: REGEN_QTY_FOOD, interval_ticks: 1, last_regen_tick: None }
Rezept:             ProductionPool { recipe: Recipe { inputs: [(GOOD_RAW, REGEN_QTY_FOOD)], outputs: [(GOOD_FOOD, REGEN_QTY_FOOD)] }, interval_ticks: 1, last_generated_tick: None }
Angebot:            SupplyPool { market: <m_a | m_fa>, good: GOOD_FOOD, offered_qty_per_tick: REGEN_QTY_FOOD, min_price: Money(500), interval_ticks: 1, last_generated_tick: None }
```

mit `REGEN_QTY_FOOD = Quantity(10)` pro Extractor (jeder Markt bedient genau 10/Tick Nachfrage — siehe §6).

**Wichtig — Supply und Demand sind NICHT co-lokalisiert.** Jedes Paar ist *cross-market* (siehe §2): der Supplier sitzt am Supply-Markt (`m_a` bzw. `m_fa`), der zugehörige Konsument fragt am **gepaarten** Demand-Markt nach (`m_b` bzw. `m_fb`). Die Auktion cleared strikt pro `(market, good)` — Angebot @ `m_a` trifft in der Auktion **nie** auf Nachfrage @ `m_b`. Der einzige Querverbindungs-Pfad ist **`macro_flow`** (für ruhende Märkte) bzw. der S3-Residual-Sourcing-Pfad (für aktive Märkte), gestützt auf die in `MarketDistances` hinterlegten Paare `(m_a,m_b)` und `(m_fa,m_fb)` — es gibt **keine** Distanz `m_a↔m_fb`, das Routing-Pairing ist also strukturell fest: `m_a` bedient `m_b`, `m_fa` bedient `m_fb`.

**Drain-then-handover (identisch zu TOOLS).** Während der Drain-Phase stehen an jedem Supply-Markt 20/Tick FOOD bereit (Endlich-Supplier 10 + Extractor 10); davon fließen 10/Tick über `macro_flow`/Residual zum gepaarten Demand-Markt (= dessen 10/Tick-Nachfrage), der Rest bleibt unverkauft beim Verkäufer. Sobald das 1 M-Endowment des Endlich-Suppliers erschöpft ist, bleibt exakt der Extractor-Output (10/Tick), der die geroutete Nachfrage genau deckt. Die Endlich-Supplier `8_011`/`8_021` **bleiben** geseedet (kein Entfernen — minimaler Eingriff, exakte Symmetrie zu TOOLS-Supplier `8_001`).

**Geld-Handover (kein Leck, siehe §5).** Sowohl der Endlich-Supplier als auch der Extractor sind Verkäufer; ihr FOOD-Verkaufserlös fließt durch die bestehende #75-Maschinerie (Lohn + 100 % Profit → Haushalte). Während des Drains teilt sich das aus FOOD stammende Haushaltseinkommen zwischen beiden Verkäufern auf; nach Erschöpfung zahlt nur noch der Extractor. Das ist eine **einmalige Gleichgewichts-Verschiebung der Einkommensquelle**, kein Verlust — `total_money` bleibt byte-invariant (alles `AccountBook::transfer`).

### Abgelehnte Alternativen

- **(B) Ein zentraler Extractor:** kann nur einen Markt versorgen → der andere FOOD-Markt läuft weiter aus, MacroFlow-Showcase stirbt. Verworfen.
- **(C) Dediziertes `GOOD_RAW_FOOD` / geteilter Roh-Input mit Kontention:** ökonomisch interessanter (echte Input-Konkurrenz), aber ändert das bewährte TOOLS-Gleichgewicht und ist faktisch die Multi-Stage-Production-Slice. Aufgeschoben.

## 4. `GOOD_RAW` wiederverwenden (keine neue GoodId)

Beide FOOD-Extractors konsumieren das **bestehende** `GOOD_RAW`. Faucets sind **pro Actor** (`RawDeposits: BTreeMap<EconomicActorId, RawDeposit>`), also regeneriert und konsumiert jeder der drei Extractors sein **eigenes** RAW im selben Tick — **keine Kontention, nichts zu allozieren**. Eine dedizierte `GOOD_RAW_FOOD` brächte keinen funktionalen Gewinn (kein geteilter Pool → keine Konkurrenz) und nur eine inerte GoodId mehr. YAGNI → `GOOD_RAW` wiederverwenden. `GOOD_RAW` bleibt strukturell nicht-handelbar (nie auf einem SupplyPool/DemandPool/Markt).

**Annahme (dokumentiert):** Der Faucet deponiert RAW ungedeckelt (`run_regen_at_tick` hat kein Level-Cap); RAW bleibt nur beschränkt, weil das 1:1-Rezept (`inputs == outputs == REGEN_QTY_FOOD`, `interval_ticks == 1`) jeden Tick exakt so viel RAW konsumiert wie regeneriert wird, und jeder Extractor der **einzige** Konsument seines eigenen RAW ist. Das gilt für diese Slice (fixe Rate, ein RAW-Konsument pro Actor). **YAGNI-Caveat:** ein künftiger Extractor mit variabler Rate oder mehreren RAW-konsumierenden Rezepten, die sich `GOOD_RAW` teilen, bräuchte entweder ein RAW-Level-Cap oder ein dediziertes Roh-Gut — das ist die Multi-Stage-Slice, nicht diese.

## 5. Konservierung

- **Güter-only.** Beide neuen Extractors laufen durch die **bestehenden** auditierten `run_regen_at_tick` (EconomySet::Regenerate) + `run_production_at_tick` (EconomySet::Production). Diese Systeme iterieren keys-first über **alle** `RawDeposits`/`ProductionPools` — die zwei neuen Einträge werden automatisch verarbeitet. **Keine Schedule-Änderung, kein neues System.**
- **Keine NEUEN Geld-Pfade.** Diese Slice fügt **keinen** Geld-handhabenden Code hinzu. Die FOOD-Extractors sind **Firmen** (Verkäufer): wenn ihr FOOD cleared, erzeugen sie `SellerReceipts`, die durch die **bestehende** #75-Maschinerie laufen — Lohn (`run_pay_wages_at_tick`) + 100 % Profit (`run_distribute_profit_at_tick`) → Haushalte. Das **schließt FOODs Geld-Kreislauf identisch zu TOOLS**: perpetuelle FOOD-Verkäufe → perpetuelles Haushaltseinkommen (statt bisher endlichem Einkommen aus den drainierenden Endlich-Suppliern). Die Extractors erscheinen **nicht** in `HouseholdSector.pool_weights` (das wird nur aus Konsumenten-`DemandPools` gebaut, `seed.rs:329-352`). `total_money` bleibt byte-invariant (ausschließlich `AccountBook::transfer`).
- **Per-Gut-Bilanz FOOD.** Wo FOOD bisher nur drainierte, balanciert die FOOD-Ledger-Bilanz nun im Steady State: `Δtotal_good(FOOD) == Σ(Produced_FOOD) − Σ(Consumed_FOOD+FinalConsumed_FOOD)`. RAW balanciert **pro Extractor** (`Regenerated(actor, RAW) == Consumed(actor, RAW)` je Produktions-Tick, RAW-Bestand je Actor beschränkt) — das macht eine künftige Shared-RAW-Regression sichtbar.

## 6. Faucet-Sizing als routing-bewusste Invariante (Härtung)

Der Survey markierte `REGEN_QTY` als handgrößt mit nur einem Einmaltest (`regen_rate_covers_aggregate_tools_demand_at_seed`, `tests/production.rs:265-326`), der **aggregat pro Gut** prüft (summiert ALLE TOOLS-Demand-Pools, keine Markt-Gruppierung) — ausreichend solange ein Gut nur eine Supply-Quelle hat, aber **blind gegen Ortsbindung**: läge bei zwei FOOD-Extractors beide an `m_a`, bliebe der Aggregat-Check 20 ≥ 20 grün, während `m_fb`s Nachfrage (8_022, geroutet aus `m_fa`) unversorgt verhungert. Da wir zwei ortsgebundene Faucets hinzufügen, brauchen wir eine **routing-bewusste** Invariante.

**Korrekte Form (NICHT supply=demand co-lokalisiert):** Supply ist ortsgebunden und erreicht Nachfrage nur über `MarketDistances`-Paare (§3). Die Invariante ist daher **pro Konsumenten-Pool**, gekeyt auf seinen *bedienenden Supply-Markt*:

> Für jeden Konsumenten-`DemandPool` (Gut g, Demand-Markt d, Nachfrage q) muss die Summe der kontinuierlichen Faucet-Raten für g an allen Supply-Märkten s, die d über `MarketDistances` erreichen (plus same-market-Supply), ≥ q sein.

Gegen die feste Seed-Topologie (`m_a↔m_b`, `m_fa↔m_fb`; **keine** `m_a↔m_fb`-Distanz): TOOLS-Pool `8_002` @ `m_b` ← Faucet @ `m_a` = 10 ≥ 10; FOOD-Pool `8_012` @ `m_b` ← Faucet @ `m_a` = 10 ≥ 10; FOOD-Pool `8_022` @ `m_fb` ← Faucet @ `m_fa` = 10 ≥ 10. Das ist **kein** Aggregat-Check und **nicht** vacuous (es keyt auf Demand-Pools, die real existieren, gegen den per `MarketDistances` erreichbaren Supply — nicht auf den Supply-Markt, der null Demand-Pools trägt).

**Implementierung (§12 Item 4):** ein **neuer** Test (bzw. eine routing-bewusste Generalisierung), der NICHT der bestehende Aggregat-TOOLS-Test ist; dieser bleibt als Spezialfall bestehen oder wird durch den allgemeineren ersetzt — der Plan entscheidet, aber der neue Test MUSS die `MarketDistances`-Erreichbarkeit modellieren, sonst ist er entweder vacuous (Key auf Supply-Markt) oder falsch (Co-Lokalisierungs-Assert).

## 7. Namensgebung (Aufräumen)

`EXTRACTOR` (singular, implizit TOOLS) wird umbenannt zu **`EXTRACTOR_TOOLS`**; die zwei FOOD-Konstanten heißen `EXTRACTOR_FOOD_A` / `EXTRACTOR_FOOD_FA`. Mechanisches Rename; keine Verhaltensänderung. (Saubere Simulation: kein implizit-typisierter „EXTRACTOR" neben expliziten FOOD-Konstanten.)

**Blast-Radius:** das `pub const` + alle Verwendungen — `production.rs` (Definition), `seed.rs`, `systems.rs` und die Test-Files (`tests/production.rs`, `tests/seed.rs`, `tests/systems.rs`, `tests/conservation.rs`) sowie Doc-Kommentare, die „EXTRACTOR" nennen (`ledger.rs`/`persist.rs`/`goods.rs`). **Verifikationsschritt:** nach dem Rename muss `grep -rw EXTRACTOR backend/crates/sim-core/src/economy` **null** Treffer liefern und `grep -rE 'EXTRACTOR_(TOOLS|FOOD_A|FOOD_FA)'` genau die drei Konstanten + Verwendungen — kein verwaister Bezeichner.

## 8. Schedule

**Unverändert.** `ResetReceipts → … → Regenerate → Production → GeneratePoolOrders → ClearMarkets → MacroFlow → PayWages(wage→profit) → TransportRebate → Consume → …`. Die neuen FOOD-Extractors sind reine **Daten** (Seed-Einträge); die generischen Regenerate-/Production-/Order-Systeme verarbeiten sie ohne neue Registrierung. Die FOOD-`SupplyPool`s werden von `GeneratePoolOrders` → ClearMarkets/MacroFlow wie jeder andere SupplyPool aufgenommen.

## 9. Persistenz & Migration

- **Keine neue Schema-Änderung.** Wiederverwendet die in #75 bereits persistierten Maps `raw_deposits: Vec<(EconomicActorId, RawDeposit)>` + `ProductionPools` + `SupplyPools` — nur **mehr Einträge**, kein neues Feld. Alte Snapshots **deserialisieren weiterhin** (kein Boot-Fehler).
- **Ehrliche Folge (kein heal-on-restore):** Da `seed_demo_economy` nur auf frischen Worlds läuft, erscheinen die neuen FOOD-Extractors **nur auf frischen Worlds**. Eine bereits persistierte World gewinnt sie erst durch ein Re-Seed (`DELETE FROM economy_snapshots`) — dieselbe Story wie #75, **kein** Hydrate-Zeit-Inject-Shim.
- **Praktisch:** #75 ist noch nicht deployed (PR offen). #75 + diese Slice landen gemeinsam auf einer frischen World mit **einem** DELETE (der bereits durch #75s `raw_deposits`-Feld erzwungen wird). Es entsteht **kein zusätzlicher** zwingender DELETE über #75 hinaus.

## 10. Skalierung auf 1.000.000

`run_regen_at_tick` ist O(|RawDeposits|) = 3 Extractors; `run_production_at_tick` O(|ProductionPools|) = 3; Order-Generierung O(|SupplyPools|). Alles viewport-unabhängig, läuft für alle Sektoren jeden Tick. Kein O(Population). `HouseholdSector.population` bleibt inert (wie zuvor).

## 11. Berührte Files (alle unter `backend/crates/sim-core/src/economy/`)

- `production.rs`: `EXTRACTOR` → `EXTRACTOR_TOOLS` umbenennen; `EXTRACTOR_FOOD_A = EconomicActorId(8_032)`, `EXTRACTOR_FOOD_FA = EconomicActorId(8_033)` hinzufügen.
- `seed.rs`: zwei FOOD-Extractor-Tripel (RAW-Eröffnungsbestand + RawDeposit + RAW→FOOD ProductionPool + FOOD SupplyPool @ m_a bzw. m_fa); `REGEN_QTY_FOOD = Quantity(10)`. **Den veralteten Kommentar bei `seed.rs:130-131`** („FOOD is intentionally left on the draining 1M endowment (no RAW→FOOD extractor this slice — recorded decision)") **ersetzen** — FOOD hat jetzt Extractors. `EXTRACTOR`-Referenzen umbenennen.
- `systems.rs`: nur `EXTRACTOR`→`EXTRACTOR_TOOLS`-Rename (keine Logikänderung — die Systeme sind bereits generisch über alle Deposits/Pools).
- `tests/production.rs`: FOOD-Analoga zu den Regen-/Throttle-Tests; die routing-bewusste Sizing-Invariante aus §6 als **neuen** Test (der bestehende `regen_rate_covers_aggregate_tools_demand_at_seed` wird ersetzt oder bleibt als Spezialfall); `EXTRACTOR`-Rename.
- `tests/seed.rs`: `seed_installs_extractor` → prüft jetzt **alle drei** Extractors (TOOLS @ m_a, FOOD @ m_a, FOOD @ m_fa) mit korrekten Märkten/Rezept-Inputs/Outputs/Faucet-Raten; `EXTRACTOR`-Rename.
- `tests/conservation.rs`: `steady_state_multi_tick` auf FOOD erweitern (Extractor-only FOOD-Supplier, lebende+beschränkte FOOD-Bänder — siehe §12 Item 5); `conservation_full_plugin_multi_tick` deckt FOOD-Per-Gut-Bilanz + per-Actor-RAW-Bilanz mit ab; `EXTRACTOR`-Rename.
- `tests/systems.rs`: nur `EXTRACTOR`→`EXTRACTOR_TOOLS`-Rename (falls referenziert).
- `persist.rs` / `tests/persist.rs`: kein Code-Change in `persist.rs` nötig; ein Round-Trip-Test, der explizit **alle drei** `raw_deposits`-Einträge seedet und verlustfrei round-trippt (der #75-Test deckt evtl. nur einen ab).

## 12. Tests (TDD)

1. **Regen feeds FOOD:** beide FOOD-Extractors regenerieren GOOD_RAW; `run_production_at_tick` wandelt RAW→FOOD; FOOD-Bestand steigt; Throttle: ohne RAW kein FOOD (input-gated).
2. **RAW nie gelistet/gehandelt:** kein FOOD-Extractor und kein Markt führt GOOD_RAW (strukturell nicht-handelbar) — auch mit drei Extractors.
3. **Seed installiert drei Extractors:** `seed.rs` legt TOOLS @ m_a, FOOD @ m_a, FOOD @ m_fa korrekt an (Rezept-Inputs/Outputs, Märkte, Faucet-Raten).
4. **Routing-bewusste Sizing-Invariante (§6):** für jeden Konsumenten-Pool ist die Faucet-Summe seines Guts an den über `MarketDistances` erreichbaren Supply-Märkten ≥ seine Nachfrage. Konkret nicht-vacuous gegen die Seed-Topologie: 8_002@m_b←m_a 10≥10, 8_012@m_b←m_a 10≥10, 8_022@m_fb←m_fa 10≥10. **Negativ-Kontrolle:** ein Test-Setup, das beide FOOD-Faucets an `m_a` legt, MUSS die Invariante für 8_022@m_fb **fehlschlagen** lassen (beweist Nicht-Vacuität).
5. **Steady-State multi-tick (erweitert):** mit Extractor-only-Suppliern (Endlich-Endowment entfernt) lebt **auch FOOD** über N Ticks — FOOD-Konsumenten-Saldo `lo > 0` und beschränkt, FOOD-`traded_qty lo > 0`, FOOD-Bestand beschränkt, Geld konstant.
6. **Per-Actor-RAW-Bilanz:** je Produktions-Tick gilt für jeden der drei Extractors `Regenerated(actor, RAW) == Consumed(actor, RAW)`, RAW-Bestand je Actor beschränkt (fängt eine künftige Shared-RAW-Regression).
7. **Persist Round-Trip:** ein Snapshot mit **allen drei** `raw_deposits`-Einträgen (explizit geseedet) round-trippt verlustfrei.

## 13. Sub-Slice-Dekomposition (ein PR)

- **A (Konstanten + Rename):** `EXTRACTOR`→`EXTRACTOR_TOOLS`, zwei FOOD-Extractor-IDs. Mechanisch.
- **B (Seed + Tests):** zwei FOOD-Extractor-Tripel seeden; Seed-Test auf drei Extractors; Regen/Throttle-FOOD-Tests; routing-bewusste Sizing-Invariante (§6) + Negativ-Kontrolle.
- **C (Konservierung + Stabilität):** `steady_state_multi_tick` + `conservation_full_plugin_multi_tick` auf FOOD erweitern; Per-Actor-RAW-Bilanz; 3-Einträge-Persist-Round-Trip-Test.

## 14. Offene Entscheidungen / Aufgeschoben

1. **`REGEN_QTY_FOOD = 10` pro Extractor** (bestätigt — deckt 10/Tick je Markt exakt, spiegelt TOOLS).
2. **`GOOD_RAW` wiederverwenden** (bestätigt — keine Kontention bei Per-Actor-Faucets).
3. **Zwei co-lokalisierte Extractors** (bestätigt — Datenmodell + Showcase-Erhalt erzwingen es).
4. **Aufgeschoben (künftige Slices, unverändert):** dediziertes `GOOD_RAW_FOOD` mit geteilter Input-Kontention; Multi-Stage-Chains (WOOD→IRON→TOOLS); freie/markträumende Preise (der eigentliche SOTA-Realismus-Sprung, Runner-up dieser Runde); Profit-Leak-Recovery + release-grade SFC-Audit; Per-Capita-Konsum-Skalierung; expliziter Arbeitsmarkt. Faucet-Sizing bleibt statisch-handgrößt (durch die routing-bewusste Invariante aus §6 getestet, nicht zur Laufzeit selbstkorrigierend — Letzteres hängt an der freie-Preise-Slice).

