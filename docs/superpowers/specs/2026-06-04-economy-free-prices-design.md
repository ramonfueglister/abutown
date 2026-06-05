# Economy: Free / Market-Clearing Prices — Design Spec

**Datum:** 2026-06-04
**Status:** Design (approved) → Plan
**Vorgänger:** #75 (self-sustaining loop, TOOLS) + #76 (FOOD self-sufficiency). Beide Güter sind nun selbsttragend & konserviert. Diese Slice macht **Preise scarcity-responsiv** — der explizit aufgeschobene `k_bps`-Tâtonnement-Nudge (#69-Spec Z. 68 & 228).

## 1. Problem

Preise werden heute **entdeckt, nicht gesetzt**. Es gibt bereits eine *lebende* Mengen-Rückkopplung (Settlement↑ → EWMA↑ → `desired_qty`↓ nächsten Tick). **Fixiert** sind aber die **Reservationspreise** der Pools: `SupplyPool.min_price` (500) und `DemandPool.max_price` (2000) — einmal geseedet, **für immer read-only**. Da jedes Settlement *innerhalb* `[min, max]` band-geklammert wird, bilden diese statischen Wände einen Korridor, der begrenzt, wie weit ein Preis je wandern kann.

**Folgen:**
1. **Einseitige Märkte** (reine Quelle = nur Supply, reine Senke = nur Demand) haben **gar keine** Preisentdeckung — `synthetic_price` (macro_flow.rs:38-47) pinnt Supply-only an den ask-floor, Demand-only an die bid-ceiling; ihr Preis-Gap verengt sich **nie** (Law of One Price gilt nur für beidseitige Märkte). In dieser Ökonomie sind **fast alle** Markt-Güter einseitig (Supply @ m_a / m_fa, Demand @ m_b / m_fb, gekoppelt über macro_flow).
2. Auch beidseitig: chronischer Mangel cleared für immer an der Decke (keine Höher-Zahlungsbereitschaft), chronischer Überschuss für immer am Boden (Verkäufer senken nie).

Das verfügbare-aber-ungenutzte Signal: `MarketGoodState.{unmet_demand_last_tick, unsold_supply_last_tick}` (market.rs:29-30) — jeden Tick frisch geschrieben (auction.rs:393/394/474/475, macro_flow.rs:786/787), persistiert, von **null** Produktionscode gelesen (totes Telemetrie). Die Spec hat genau das als „k_bps tâtonnement price-nudge policy" aufgeschoben (#69 Z. 228).

## 2. Ziel & ehrlicher Scope

**Garantiert** (was diese Slice liefert): Reservationspreise werden **scarcity-responsiv** — lokaler Excess-Demand treibt sie in die **korrekte Richtung** (Mangel→hoch, Überschuss→runter), **gedämpft & beschränkt**, **clearbar** (Band kollabiert nie unter Handelbarkeit), **konservierungs-exakt** (Preise sind Order-Parameter, kein Geld). Der tote Telemetrie-Vektor wird genutzt; der Korridor folgt nun der Knappheit statt statisch zu sein.

**Gemessen, nicht garantiert** (ehrlich): der *Grad* der räumlichen Law-of-One-Price-Konvergenz für einseitige Quelle↔Senke-Paare. Der lokale Excess-Demand-Nudge bewegt Preise korrekt-gerichtet und stabil, aber volle Gap→Transportkosten-Konvergenz für einseitige Paare hängt am Flow-Margin-Feedback und wird vom Steady-State-Test **gemessen** (verengt sich der Gap? bleibt er beschränkt?), nicht als Invariante behauptet. Kein Over-Claim.

## 3. Mechanismus: gedämpfter Tâtonnement-Reservationspreis-Nudge

Ein neues System liest nach `ClearMarkets`+`MacroFlow`+`Telemetry` das (dann finale) Excess-Demand-Signal jedes Pools und stößt dessen Reservationspreis Richtung Knappheit an — mit **vier Stabilitäts-Wächtern**.

Pro Pool (keys-first über `DemandPools` bzw. `SupplyPools`), nur auf dem Kadenz-Boundary-Tick:

1. **Signal** aus dem `(market, good)`-State des Pools:
   - `net = unmet_demand_last_tick.0 − unsold_supply_last_tick.0` (i64)
   - `scale = max(1, unmet_demand_last_tick.0 + unsold_supply_last_tick.0)`
   - `x_bps = (net as i128 * 10_000) / (scale as i128)` ∈ **[−10_000, +10_000]** (dimensionslose Intensität; floor; keine Division-durch-Traded-Überraschung).
   - `net == 0` (kein Ungleichgewicht) → `x_bps == 0` → kein Nudge (das System ist im Gleichgewicht **ruhig**).

2. **Speed-Limit** (die tragende Anti-Oszillations-Garantie):
   - `step_bps = clamp((k_bps as i128 * x_bps) / 10_000, −max_step_bps, +max_step_bps)`
   - Defaults `k_bps = 500` (5 %), `max_step_bps = 100` (harte **1 %/Intervall**-Decke).

3. **Translation beider Wände in dieselbe Richtung** (User-Entscheid): jeder Pool skaliert seinen eigenen Reservationspreis:
   - `new = price.0 + (price.0 as i128 * step_bps) / 10_000` (i128, checked → `Overflow`).
   - Mangel (`net>0`) → Preis **hoch**; Überschuss (`net<0`) → Preis **runter**. Da `step_bps ∈ [−100,+100]` ⇒ Faktor `(1 + step/10000) ∈ [0.99, 1.01] > 0`, bleibt die Ordnung `min < max` bei proportionaler Skalierung erhalten (beide Pools desselben beidseitigen Marktes sehen dasselbe Signal → Band **transliert**; einseitige Märkte: jede Seite reagiert auf ihr lokales Signal).

4. **Absolute Guardrails:** `new = clamp(new, price_floor, price_ceiling)` mit `price_floor > 0` (verhindert `ZeroPrice` in `spend_to_qty`/`affordable_qty`) und großzügiger `price_ceiling`. Die Klammerung ist monoton ⇒ `min ≤ max` bleibt erhalten (selbst `min == max == bound` cleart noch zum Einzelpreis — **nie unhandelbar**).

**Kadenz (zustandslos, KEINE Migration):** das System feuert nur, wenn `tick.is_multiple_of(config.macro_flow_interval_ticks)` (Default 10) — exakt das Muster des #75-Transport-Rebate. **Kein Cursor-Feld** → keine Snapshot-Schema-Änderung. Damit absorbiert die schnelle EWMA-Mengen-Schleife (α=2000, ~10-Tick-Halbwertszeit) jeden Nudge, bevor der nächste kommt → Zeitskalen entkoppelt → keine Limit-Cycles. Auf dem Boundary-Tick läuft macro_flow ohnehin (gleiches Intervall), also ist das Signal frisch-post-flow.

## 4. Einfügepunkt

- **Feld(er) mutiert:** `DemandPool.max_price` + `SupplyPool.min_price` (pools.rs — beide `Money`/i64, `Copy`, **bereits** in `DemandPools`/`SupplyPools` serde-persistiert). **Kein neues Feld, keine `DELETE FROM economy_snapshots`-Migration** (erste Economy-Slice ohne DELETE).
- **Schedule:** neues `EconomySet::AdjustReservationPrices`, **nach `Telemetry`, vor `UpdateConsumption`**, `.before(tick_increment_system)`. Die angepassten Preise wirken automatisch beim **nächsten** `GeneratePoolOrders` und macro_flow-Band-Build (beide lesen `pool.max_price`/`min_price` LIVE — macro_flow.rs:138-139/153-154).
- **System:** `run_adjust_reservation_prices_at_tick(demand: &mut DemandPools, supply: &mut SupplyPools, market_goods: &MarketGoods, config: &EconomyConfig, current_tick: u64) -> Result<(), EconomyError>` — pur über Refs, keys-first, kein `World`. **Error-Modell (Codebase-Präzedenz wages/profit):** Config einmal oben validieren (`validated_*` → `.expect` auf die genuin-infallible, einkompilierte Config); der checked-arithmetic-Pfad (`price·step/10_000` → `i64::try_from`) liefert `EconomyError::Overflow` als `Result`; der System-Wrapper surface't ein genuines `Err` ehrlich als `MarketClearFailed`-Audit-Event — **kein `let _`, kein stiller Default**. (Mit `price_ceiling ≤ 100_000` ist Overflow praktisch unerreichbar — `100_000·100/10_000 = 1_000` — aber das `Result` bleibt ehrlich statt eines `as`-Wrap.)
- **Config (NICHT persistiert → keine Migration):** `EconomyConfig.price_adjust_k_bps: u16 = 500`, `price_adjust_max_step_bps: u16 = 100`, `price_floor: Money`, `price_ceiling: Money`, mit `validated_*`-Gettern (`k_bps ≤ 10_000`, `max_step_bps ≤ 10_000`, `0 < floor < ceiling`). `EconomyConfig` ist **nach dem Seeding eingefroren** (einkompilierte Defaults, nicht persistiert, kein Mid-Run-Reload) — daher kann die zustandslose `macro_flow_interval_ticks`-Kadenz-Phase nie driften, und die No-Cursor/No-DELETE-Eigenschaft hält.

## 5. Konservierung & Invarianten

- **SFC unberührt:** Preise sind Order-PARAMETER, kein Geld. Der Nudge schreibt nur i64-Preisfelder und liest Telemetrie — ruft **nie** `transfer`/`lock`/`consume`. Präzise: `total_money`-Konservierung folgt aus der **Settlement-Atomizität** (`auction.rs` lockt/debitiert atomar + konserviert), NICHT aus Preis-Unveränderlichkeit — ein geänderter Reservationspreis re-tuned nur, WELCHE Orders nächsten Tick matchen und zu welchem Stückpreis, nie die bewegte Gesamt-Money/Quantity. `total_money` bleibt daher byte-invariant.
- **Determinismus / Fixed-Point:** i64-Felder, i128-Zwischenwerte, checked, floor, ECONOMY_SCALE=1000; kein float/RNG; keys-first BTreeMap, Ties-by-ascending-key. Gleiche Inputs → byte-identisch über Runs + persist/restore.
- **NO-FALLBACK / ehrliche Errors:** nie still einen Preis defaulten; `price_floor > 0` erzwingt strikt positive Preise (kein `ZeroPrice`); `Overflow` via `Result`, nicht wrappen. Config-Validierung fail-loud.
- **Band/Executable-Bounds (v0-Spec Z. 636):** jedes Settlement bleibt in `[marginal_ask, marginal_bid]`. Anchored-Settlement garantiert das; der Nudge hält das Band nicht-degeneriert (proportionale Skalierung + monotone Klammerung ⇒ `min ≤ max`, immer clearbar).
- **Koexistenz EWMA:** `ewma_reference_price` bleibt die Konsum-Referenz; der Nudge schreibt **weder** `ewma_reference_price` **noch** `last_settlement_price` (kein Double-Count) — nur Pool-Reservationspreise. EWMA low-passt die resultierenden Settlement-Bewegungen.
- **Koexistenz MacroFlow / LoOP:** macro_flow liest `pool.max_price`/`min_price` LIVE für die Band-Extreme und ehrt `preserve_price` (aktive Endpoints behalten Auktionspreis). Nudge läuft nach `Telemetry` (Signal final) und auf derselben langsamen Kadenz (= macro_flow_interval) → kämpft nicht gegen die räumliche Konvergenz.

## 6. Skalierung auf 1.000.000

O(|DemandPools| + |SupplyPools|), ein State-Lookup pro Pool, läuft nur jeden N-ten Tick. Viewport-unabhängig, kein O(Population). Identische mean-field-Disziplin wie #69-#76.

## 7. Berührte Files (alle unter `backend/crates/sim-core/src/economy/`)

- `systems.rs`: `EconomyConfig`-Knobs (`price_adjust_k_bps`/`price_adjust_max_step_bps`/`price_floor`/`price_ceiling` + `validated_*`); `EconomySet::AdjustReservationPrices` zwischen `Telemetry` und `UpdateConsumption`; `run_adjust_reservation_prices_system`-Wrapper (Err ehrlich gesurfaced).
- **`pricing.rs` (NEU — bestätigt, nicht „Plan entscheidet"):** `run_adjust_reservation_prices_at_tick` (der pure Kern: Signal → step → translate → clamp). Eigene fokussierte Datei (Single-Responsibility), hält `pools.rs` schlank.
- **Audit-Event: bestehendes `EconomyEvent::MarketClearFailed` wiederverwenden** (bestätigt — KEIN neues Event), analog dem `systems.rs`-Präzedenzfall für Faults; der Wrapper pusht es bei einem genuinen `Err`. Kein `ledger.rs`-Change.
- `mod.rs`: System registrieren.
- `tests/{pricing|pools}.rs`, `tests/systems.rs`, `tests/conservation.rs`: siehe §9.
- **Kein** `persist.rs`-Change, **kein** neues persistiertes Feld.

## 8. Schedule (vollständig, neue Kette)
`ResetReceipts → … → ClearMarkets → MacroFlow → PayWages → TransportRebate → Consume → ShopperCapture → CommuterCapture → Materialize → Telemetry → **AdjustReservationPrices** → UpdateConsumption`. Neu nur das eine Set, an genau einer Stelle (eigener `add_systems`-Aufruf mit `.after(<Telemetry-System>).before(<UpdateConsumption-System>)` — die Bevy-0.18-Gotcha aus #75 beachten: explizite Kante in separatem `add_systems`).

## 9. Tests (TDD)

**Pure-Kern (`run_adjust_reservation_prices_at_tick`):**
1. **Signal-Intensität:** `x_bps` = clamp/saturierend in [−10000,10000]; `net==0` → 0; reine Mangel → +10000; reiner Überschuss → −10000; gemischt → korrekt skaliert (floor).
2. **Speed-Limit:** bei riesigem Ungleichgewicht ist `|step_bps| ≤ max_step_bps` (Decke greift unbedingt, unabhängig von der Signalstärke).
3. **Richtung + Translation:** Mangel → beide Wände hoch, Überschuss → beide runter, Balance → unverändert; nach dem Nudge gilt stets `min < max` (Band transliert, kollabiert nicht).
4. **Guardrails:** clamp in `[floor, ceiling]`; ein Preis fällt nie ≤0; ein an die Decke getriebenes Band bleibt `min ≤ max` (clearbar).
5. **Determinismus:** zweimal derselbe Input → byte-identische Preisanpassung; keys-first.
6. **NO-FALLBACK:** Config mit `floor ≤ 0` oder `floor ≥ ceiling` oder `k_bps > 10_000` → ehrlicher `Err`, kein still-Default.

**Schedule/Plugin:**
7. **Ordering:** ein Recorder-Test beweist `AdjustReservationPrices` läuft nach dem Telemetrie-System und vor dem Consumption-Update; Kadenz-Gate feuert nur auf `tick % macro_flow_interval_ticks == 0`.

**Konservierung & Verhalten (multi-tick, voller Plugin):**
8. **Konservierung:** `total_money` byte-invariant über N Ticks mit aktivem Nudge (das System bewegt kein Geld) — erweitert/spiegelt `conservation_full_plugin_multi_tick`.
9. **Scarcity-Response (das Kern-Verhalten):** ein Markt mit **anhaltend** `unmet_demand > 0` über **≥ 5 Kadenz-Boundary-Ticks** zeigt einen **monotonen, beschränkten Anstieg** der Reservationspreise (und `unsold_supply > 0` → monotones, beschränktes Fallen), geklammert durch die Guardrails — beweist, dass die tote Telemetrie nun den Preis treibt. (Über mehrere Boundaries gemessen, damit ein oszillierendes — nicht anhaltendes — Ungleichgewicht den Test nicht flattert; der Anstieg ist durch `[floor,ceiling]` + `max_step_bps` ohnehin gedeckelt.)
10. **Stabilität / Nicht-Destabilisierung:** der bestehende selbsttragende Steady-State (analog #75/#76 `steady_state_multi_tick`) bleibt mit aktivem Nudge **lebend & beschränkt** — Reservationspreise oszillieren nicht unbeschränkt (Tail-Band beschränkt), Geld konstant, Mengen-Bänder weiterhin `lo>0`, Band stets clearbar (`min ≤ max`). **Cross-Market-Gap = NUR gemessen/protokolliert, NIEMALS ein harter Assert** (No-Over-Claim-Regel, §2): der Test loggt, ob sich der einseitige Quelle↔Senke-Preis-Gap gegenüber dem statischen Korridor verengt, asserted aber nur Beschränktheit/Clearbarkeit — volle LoOP-Konvergenz ist nicht garantiert.

## 10. Sub-Slice-Dekomposition (ein PR)

- **A (Config + purer Nudge-Kern):** `EconomyConfig`-Knobs + `validated_*`; `run_adjust_reservation_prices_at_tick` (Signal/step/translate/clamp) + Pure-Tests 1-6. NICHT verdrahtet.
- **B (Schedule-Verdrahtung):** `EconomySet::AdjustReservationPrices` + System-Registrierung (separater `add_systems`, Kadenz-Gate) + Ordering/Kadenz-Test 7.
- **C (Konservierung + Verhalten + Stabilität):** Tests 8-10 (Konservierung multi-tick, Scarcity-Response, Steady-State-Nicht-Destabilisierung + Gap-Messung).

## 11. Offene Entscheidungen / Aufgeschoben

1. **Tâtonnement-Reservationspreis-Nudge** (bestätigt — die spec-benannte aufgeschobene Mechanik).
2. **Scope: ALLE Märkte** (bestätigt vom Lead).
3. **Band transliert (beide Wände, gleiche Richtung wie Knappheit)** (bestätigt vom Lead — bewahrt clearbare Überlappung).
4. **Signal = Order-Residual `unmet−unsold`** (die spec-earmarkte tote Telemetrie); **Defaults** k_bps=500, max_step_bps=100, Kadenz=macro_flow_interval_ticks=10, Guardrails `[price_floor, price_ceiling]` großzügig um die Seeds (500/2000) — die Headline-Tuning-Knobs, im Plan mit konkreten Werten zu fixieren (z. B. floor=1, ceiling=100_000) und vom Steady-State-Test validiert.
5. **Aufgeschoben:** Elastizitäts-geformte Nachfrage (`elasticity_bps`/`urgency_bps` aktivieren — komplementäre Demand-seitige Verfeinerung, Mechanismus 3 der Grundlage); inventar-/coverage-basiertes Pricing (Mechanismus 2); volle räumliche LoOP-Konvergenz für einseitige Paare via Flow-Margin-Feedback (diese Slice liefert die lokale Scarcity-Response, misst die Konvergenz); Profit-Leak-Recovery + release-grade SFC-Audit; Multi-Stage-Chains; expliziter Arbeitsmarkt; per-capita-Konsum.
