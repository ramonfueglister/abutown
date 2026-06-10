# Economy Slice: Mehrstufige Produktionsketten — Firms-as-Buyers (WOOD→TOOLS)

**Status:** Design (Brainstorming abgeschlossen, wartet auf Lead-Review)
**Datum:** 2026-06-10
**Vorgänger:** `2026-06-03-economy-self-sustaining-loop-design.md` (#75: Extraktoren + 100%-Profit-Ausschüttung — dessen §-„Kein Investitionssektor" ist der hier geschlossene Gap), `2026-06-04-economy-free-prices-design.md` (#77), `2026-06-05-economy-sfc-audit-design.md` (#78, der Safety-Canary dieser Slice), `2026-06-07-economy-onesided-price-convergence-design.md` (#85, räumliches Pricing der Inputflüsse).
**Lead-Entscheidungen (Brainstorm):** (1) Mechanik-Slice mit **einer** Kaufstufe — WOOD→TOOLS; IRON folgt als reiner Daten-Slice. (2) Sichtbarkeit über den bestehenden Macro-Flow/Flow-Trader-Pfad (räumlich getrennte Märkte), **kein** neues Rendering/Glyph in dieser Slice.

---

## 1. Ziel

Heute kaufen ausschließlich Konsumenten-Pools; Produzenten haben Inputkosten 0 (RAW-Faucet direkt an der Firma). Damit gibt es keine Zwischengüter, keine Kettenpropagation, und der in #75 dokumentierte latente Profit-Leak („Firmen reinvestieren nicht") bleibt unverlegt, weil Firmen nie Kasse brauchen.

Diese Slice führt die **Firms-as-Buyers-Mechanik** ein: die TOOLS-Firma kauft ihr Input-Gut WOOD auf einem räumlich entfernten Markt über den bestehenden Order-/Settle-/Macro-Flow-Pfad. Damit werden in einem Bogen drei gekoppelte Dinge real:

1. **Input-Kosten & Kettenpreise:** TOOLS-Stückkosten enthalten erstmals echte Inputpreise *inklusive* Transport (`rate·dist` aus #85) — die Kette ist räumlich.
2. **Value-added-Löhne:** Löhne auf Wertschöpfung statt Umsatz — beendet die Lohn-Doppelzählung entlang der Kette, bevor sie mit der ersten Kaufstufe entstünde.
3. **Profit-Leak-Recovery:** Firmen brauchen jetzt Working Capital → die 100%-Ausschüttung aus #75 wird durch eine **θ-Dividende mit Working-Capital-Kappung** ersetzt. Einbehaltenes ist bilanzierter Net Worth, alles oberhalb des Targets fließt strukturell an die Haushalte zurück.

`GOOD_WOOD(2)` wird live; `GOOD_IRON(3)` bleibt Platzhalter für den Folge-Daten-Slice. Die FOOD-Kette bleibt unangetastet (Kontrollgruppe).

## 2. Das Modell (auf echten Papers)

- **AB-SFC-Benchmark (Caiani et al., 2016):** Firmen sind Käufer auf Input-Märkten, finanzieren Käufe aus Kasse, schütten einen **Anteil θ der Profite als Dividende** an die Haushalte aus und halten den Rest als Liquidität/Net Worth; alle Flüsse sind doppelt gebucht. Genau dieses Muster wird hier übernommen: Matching über den vorhandenen Marktmechanismus, Dividendenquote `theta_bps`, Retained Earnings als Firmenkasse.
- **Leontief-Produktionsnetzwerk (Carvalho & Tahbaz-Salehi, 2019):** Produktion mit fixen Input-Koeffizienten (`in_qty` WOOD pro `out_qty` TOOLS, keine Substitution). Input-Nachfrage ist **abgeleitete Nachfrage** aus dem Produktionsplan — kein keynesianischer Konsum. Die Kettenpropagation von Preis-/Mengenschocks läuft über die Input-Output-Verknüpfung (hier: 1 Kante WOOD→TOOLS; die Leontief-Inverse wird mit IRON zur echten Kette).
- **Stock-Flow-Consistency (Godley & Lavoie, 2007):** jede Transaktion ist `AccountBook::transfer` (quadruple booking auf unserem zweispaltigen Konto-Modell); `total_money` bleibt byte-invariant; Sektorbilanzen schließen. Der per-Tick-Audit aus #78 bleibt unverändert der Fail-fast-Beweis.
- **Einseitige räumliche Konvergenz (intern, #85):** Der WOOD-Preis am Nachfrage-Markt konvergiert gegen `p_src + rate·dist`. Zusammen mit der Teilnahme-Schranke (§5.4) ist das die Anti-Spiralen-Garantie für Leontief-starre Inputnachfrage (Lektion aus Blocker-2, `2026-06-06-slice2b-followups.md`).

**Ehrliches Wording:** Es gibt weiterhin **keinen expliziten Arbeitsmarkt und keinen Investitions-/Kapitalgütersektor** — Löhne bleiben ein fixer Anteil der Wertschöpfung (labor_share), Working Capital ist reine Liquiditätshaltung, kein Kapitalstock. Das ist der bewusst nächste kleine Schritt Richtung Caiani-Benchmark, nicht der Benchmark selbst.

## 3. Architektur-Doktrin

Wie #69–#85: aggregate Autorität O(Sektoren), viewport-unabhängig; deterministisch (fixed-point i64/i128, keys-first BTreeMap, kein RNG/Float in der Autorität, floor-div, ties-by-ascending-index); NULL Fallbacks/silent-defaults; frozen-time-Persistenz; kein serde-default für neue Pflichtfelder.

## 4. Akteure & Datenfluss

```
Markt 9003 (Flow Demo A, [16,48])          Markt 9001 (Demo A, [2,3])            Markt 9002 (Korridor)
┌─────────────────────────────┐            ┌───────────────────────────┐          ┌──────────────────┐
│ NEU 8041: WOOD-Extraktor    │   WOOD     │ 8031 UMBAU: TOOLS-Firma   │  TOOLS   │ 8002: Consumer   │
│ RAW-Faucet → WOOD (qty 10), │ ─────────▶ │ kauft WOOD (InputPool),   │ ───────▶ │ kauft TOOLS      │
│ SupplyPool am 9003          │ macro_flow │ Rezept in_qty WOOD →      │ (wie     │ (unverändert)    │
└─────────────────────────────┘ + Flow-    │ out_qty TOOLS,            │  heute)  └──────────────────┘
                                  Trader   │ RAW-Faucet ENTFÄLLT       │
                                 sichtbar  └───────────────────────────┘
```

- **8041 (`EXTRACTOR_WOOD`, neu):** identische Faucet-Maschinerie wie 8031–8033 heute (`RawDeposit` + `ProductionPool` RAW→WOOD + `SupplyPool`), nur `out_good = GOOD_WOOD`. RAW bleibt die einzige nicht handelbare Urquelle.
- **8031 (Umbau, kein Parallelpfad):** `RawDeposit` und RAW→TOOLS-Rezept werden **ersatzlos entfernt**. Neu: `InputPool` (WOOD, Heimmarkt 9001) + Rezept `in_qty GOOD_WOOD → out_qty GOOD_TOOLS` über das **bestehende** Input-Gate von `run_production_at_tick` (kein WOOD im Lager → keine Charge → TOOLS-Angebot versiegt sichtbar).
- **WOOD 9003→9001** über den unveränderten `macro_flow`: #85-Pricing, Transportgebühr an `TRANSPORT_OPERATOR`, Flow-Trader automatisch materialisiert → Inputlogistik auf der Karte sichtbar, null neues Rendering.
- **Rezept-Verhältnis v0:** `in_qty = 10, out_qty = 10` (1:1) — minimiert Kalibrierrisiko; Verhältnis ist authorbar.

### 4.1 `InputPool` (neue, getrennte Pool-Art)

Bewusst **kein** Varianten-Flag in `DemandPool` (Isolation): eigener Struct + eigene Resource, spiegelt die `DemandPool`-Felder, die der Order-Pfad braucht (market, good, max_price, Kassenbindung), **ohne** mpc/autonomous/income:

```rust
InputPool { market, good: GoodId, /* abgeleitet, kein Keynes: */
            batches_target: u32,  // EIN Knopf: Soll-Lager in Chargen UND Working-Capital-Ziel (§5.3); Default 2
            max_price: Money }    // Teilnahme-Schranke, je Intervall neu gesetzt (§5.4)
InputPools(BTreeMap<EconomicActorId, InputPool>)  // Resource, persistiert
```

`desired_qty` wird **jedes Intervall** aus dem Rezept abgeleitet: `batches_target · in_qty − held(WOOD)` (geflort auf 0) — Leontief-abgeleitete Nachfrage, kein eigener Update-Pfad in `UpdateConsumption`. Der Consumption-Sink (`run_consumption_at_tick`) iteriert nur `DemandPools` und fasst Inputgüter strukturell nie an; `FinalConsumed` bleibt Konsumgütern vorbehalten.

## 5. Geldflüsse & Konservierung

Alle Bewegungen ausnahmslos `AccountBook::transfer`; #78-Audit unverändert gültig.

### 5.1 `BuyerOutlays` (Gegenstück zu `SellerReceipts`)
`BuyerOutlays(BTreeMap<(EconomicActorId, MarketId), Money>)` — sammelt beim Buyer-Settle (Auktion **und** macro_flow) die Käufer-Belastungen **aller** Käufer unconditional (kein Membership-Coupling im Settle; Konsumenten-Outlays sind eine harmlose, ungenutzte Statistik — nur der Join in PayWages wertet sie aus). Beim macro_flow enthält die Belastung den Transportanteil → Transport steckt damit real in den Inputkosten. Exakt dieselben Semantiken wie `SellerReceipts`: **per-Tick, ephemer, NIE persistiert**, Reset im selben `ResetReceipts`-Wrapper, Capture in der Scratch-Zone der Settle-Funktionen (ein verworfener Settle verwirft auch seine Outlays).

### 5.2 Value-added-Löhne
`value_added = SellerReceipts[(a, m)] − BuyerOutlays[(a, m)]` (fehlender Eintrag = 0); `wage = floor(labor_share_bps · max(0, value_added) / 10_000)`.
Für Extraktoren (Outlays 0) numerisch identisch zu heute — **kein Verhaltensdrift bei Bestandsakteuren**. Negative Wertschöpfung im Intervall (eingekauft, noch nichts verkauft) → Lohn 0, kein negativer Transfer.

### 5.3 θ-Dividende mit Working-Capital-Kappung (ersetzt 100%-Payout)
```
profit       = max(0, value_added − wage)
wc_target    = batches_target · in_qty · max_price     // erwartete Chargen-Inputkosten (ein Knopf, §4.1)
distributable = max(0, cash(a) − wc_target)
dividend     = min(floor(theta_bps · profit / 10_000), distributable)
```
Kadenz: dasselbe Ausschüttungs-Intervall wie die heutige Profit-Distribution (PayWages-Set), kein neuer Timer. Defaults: `theta_bps = 8_000`, `batches_target = 2`; beides per Producer in `markets.json` authorbar (Re-Apply-Regel §6.2). Für Akteure **ohne** `InputPool` (Extraktoren, Alt-Supplier) gilt weiterhin `wc_target = 0, theta_bps = 10_000` → exakt das heutige #75-Verhalten, kein Migrationseffekt. Transferweg unverändert Firma → `HOUSEHOLD_SECTOR` → Pools (`apportion_cash`); Sentinel `HOUSEHOLD_SECTOR == 0` nach Ausschüttung bleibt. Einbehaltenes = Firmen-Net-Worth (Telemetrie-Gauge `retained_earnings`, kein Hard-Sentinel — Kasse schwankt intra-Intervall legitim).

### 5.4 Teilnahme-Schranke (Anti-Spirale)
`max_price(WOOD) = floor(p_tools_ref · (10_000 − labor_share_bps)/10_000 · out_qty / in_qty)` mit `p_tools_ref` = bestehende EWMA-Referenz aus der Telemetrie. Interpretation: nie mehr für Inputs bieten, als der erwartete Output **nach** dem Lohnanteil deckt — so bleibt der Lohnfluss bei jedem akzeptierten Preis zahlbar. Zusammen mit #85 kann die Leontief-starre Nachfrage den WOOD-Preis nicht ins Ceiling treiben; liegt bereits `p_src + rate·dist` über der Schranke, ist die Kette strukturell unrentabel → Hungern sichtbar (§7.4), kein Maskieren.

### 5.5 Güter-Konservierung
Unverändert ledger-abgeleitet: `Σ Produced(WOOD) − Σ Consumed(WOOD als Produktionsinput) = ΔWOOD`; analog TOOLS mit `FinalConsumed` als Senke.

## 6. Schedule & Persistenz

### 6.1 Schedule (Reihenfolge unverändert, vier Erweiterungen)
```
ResetReceipts        → resettet auch InputOutlays (gleicher Wrapper)
Production           → 8041: RAW→WOOD (vorhandene Maschinerie); 8031: WOOD→TOOLS via Input-Gate
GeneratePoolOrders   → zusätzlich InputPool-Orders: desired = batches_target·in_qty − held, max_price = §5.4
ClearMarkets/MacroFlow → unverändert; Buyer-Settle eines InputPool-Akteurs bucht InputOutlays
PayWages             → Basis value_added (§5.2); Profit-Distribution wird zur θ-Dividende (§5.3, ersetzt)
UpdateConsumption    → nur DemandPools (Keynes); InputPools vollständig in GeneratePoolOrders abgeleitet
```
Produktion läuft vor dem Einkauf desselben Ticks → Ein-Tick-Produktionslag (Bestände aus Vor-Ticks), gewollt und standard.

### 6.2 Daten & Persistenz
- **`markets.json`:** neue Sektion `producers` (`actor, market, in_good, in_qty, out_good, out_qty, qty, min_price, theta_bps, batches_target, opening_cash` — `qty`/`min_price` sind die Sell-Side wie bei `ExtractorSpec`: der Producer verkauft seinen Output über einen normalen `SupplyPool`). `theta_bps`/`batches_target` leben in einer NICHT persistierten `ProducerPolicies`-Resource (Re-Apply aus dem Layer, §unten); nur `InputPools` (Order-Cursor) wird persistiert. 8031 wandert von `extractors` nach `producers` (opening_cash 1_000_000, konsistent mit Bestandsakteuren); 8041 neu in `extractors` mit `out_good: 2`; `opening_prices` für WOOD an 9003 und 9001. Extraktoren behalten Faucet-Semantik — kein Überladen des Schemas.
- **Seed-Validierung (fail-fast):** `producer.in_good ≠ GOOD_RAW` (RAW bleibt nicht handelbar), `in_qty/out_qty > 0`, `theta_bps ≤ 10_000`, `batches_target ≥ 1`, Producer-Markt existiert, Input-Gut hat mindestens einen Supply-Markt.
- **Config-Reapply (Lektion #83):** `theta_bps`/`batches_target` werden beim Start **vor dem Seed-Guard** aus dem authored Layer re-applied, sonst fällt die Ausschüttungsregel bei jedem Restart auf Default zurück.
- **Snapshot:** `InputPools` persistiert (non-serde-default) → ⚠️ **einmaliges `DELETE FROM economy_snapshots` vor dem Deploy** (wie #69/#73/#74/#75). `BuyerOutlays` ist per-Tick-ephemer (nie persistiert), `ProducerPolicies` wird re-applied. Kein Legacy-Shim.
- **Wire (`EconomySnapshot`, additiv):** `retained_earnings`-Gauge + Producer-Rezeptinfo für den Click-Inspector, **frische Tags** (4/5/2 bleiben reserviert, PR #92). `buf breaking` (WIRE_JSON) bleibt grün; Frontend zeigt die Felder im bestehenden Inspector.

## 7. Fehlerfälle (Konsequenz sichtbar, nichts heilen)

1. **Firma ohne Kasse:** Orders mangels `lock_cash` nicht platziert → Produktion hungert → TOOLS versiegt an 9002, Preis steigt sichtbar; Inspector zeigt `retained_earnings ≈ 0` + leeres Lager. Kein Bailout.
2. **Negative Wertschöpfung:** Lohn 0, Dividende 0 (geflort), keine negativen Transfers.
3. **Kein WOOD-Angebot/unerreichbar:** Orders expiren über `ExpireOrders`, `release_cash` gibt gelocktes Geld frei — Pfad ist #78-auditiert, kein neuer Leak möglich.
4. **`max_price` unter Marktpreis:** keine Trades → Hungern wie (1). Strukturelle Unrentabilität (`p_src + rate·dist` > Schranke) ist ein Authoring-Problem und bleibt sichtbar.
5. **Unsinnige Authoring-Werte:** fail-fast beim Seed (§6.2).
6. **Restart:** Frozen-Time-Restarts liegen an Tick-Grenzen; `BuyerOutlays` ist per-Tick (Settle und Lohn im selben Tick) und braucht keine Persistenz; der `InputPools`-Cursor ist persistiert → konsistentes Resume.
7. **Bug in neuen Transferpfaden:** der per-Tick-#78-Audit (byte-exakte `total_money`-Invarianz) bricht den Tick fail-fast.

## 8. Tests

- **Unit:** Value-added-Lohn (inkl. Negativ-Fenster → 0); θ-Dividende (Kasse < Target → 0; Teilausschüttung; θ=10_000/wc=0 ≡ #75-Verhalten); Teilnahme-Schranken-Formel; Leontief-Order-Sizing (Soll minus Bestand, Floor 0); `BuyerOutlays`-Capture an beiden Settle-Punkten (Auktion: actual_cost; macro_flow: Charge inkl. Transport) + Reset.
- **Konservierung:** N-Tick-Lauf mit voller Kette: `total_money` byte-invariant pro Tick; `HOUSEHOLD_SECTOR == 0` nach Lohn+Dividende+Rebate; Güter-Ledger-Identität WOOD und TOOLS.
- **Hydrate-Pfad (Lektion #86):** seed → extract → apply auf dem **Produktions**-Restore-Pfad, Resume mitten im Intervall, Kette läuft weiter.
- **Langlauf-Stationarität (Anti-Blocker-2):** M Ticks:
  - WOOD-Preis am 9001 settelt an der §5.4-Teilnahme-Schranke — abgeleitete-Nachfrage-/MRP-Preisbildung: die Schranke IST der Preis der Input-Leg, der einzige Käufer bietet sein Grenzwertprodukt netto Lohnanteil (Marshall'sche abgeleitete Nachfrage); der Handel bleibt dabei strikt profitabel (realisierte landed unit cost unter der Schranke), und das #85-LoOP-Band gilt weiterhin für KONSUMENTEN-Senken (abgedeckt durch den 9002-Langlauftest `abutopia_prices_stay_in_band_and_9002_consumes_over_long_run`).
  - (Bewusst korrigiert in Task 6: die ursprüngliche Fassung „konvergiert ins Band um `p_src + rate·dist`" übertrug Konsumenten-Senken-Semantik auf die Input-Leg — die #85-Flow-Margin-Rückkopplung iteriert nur Demand-/SupplyPools, InputPools sind strukturell außerhalb, und der dormant Demand-only-Bucket settelt AM Bid-Ceiling = der Schranke, die jede Kadenz aus der TOOLS-EWMA neu geschrieben wird. Fixpunkt-Trace: Schranke 274 vs idealisierte landed cost 345 vs realisierte ≈ 247.)
  - θ-Dividenden feuern im Langlauf-Tail und der Kassen-**Drift** der Firma bleibt im Tail klein relativ zur Skala (≤ ein `wc_target` über das Tail-Fenster) — das Seed-Endowment (`opening_cash`, §6) entwässert nie, weil die Ausschüttung pro Tick auf `θ·profit` gedeckelt ist (§5.3), und der (1−θ)-Einbehalt akkumuliert legitim als Firmen-Net-Worth (bewusst korrigiert: die frühere Fassung „Firmenkasse pendelt ≤ `wc_target + ε`" widersprach der eigenen Dividenden-Mechanik aus §5.3 und dem authored `opening_cash` aus §6 — ein absolutes Kassen-Level ist mit gedeckelter Ausschüttung unerreichbar, die Stationaritäts-Garantie ist der Drift).
  - TOOLS-Fluss an 9002 stationär > 0.
- **Browser-Smoke (Pflicht, CLAUDE.md):** Feature kreuzt die Wire (neue Snapshot-Felder + Inspector) → echter Browser-Lauf gegen den Dev-Stack; render-smoke-Pins (exakte Agentenzahlen) bewusst prüfen/aktualisieren, da WOOD-Flow-Trader hinzukommen.
- Volles CI-Gate (Rust fmt/clippy/test, buf lint+breaking, tsc src+tests+scripts, vitest, build, e2e) vor Push.

## 9. Out of Scope (bewusst)

IRON-Stufe (reiner Daten-Folge-Slice: `producers`-Eintrag + Markt-Zuordnung, null Mechanik); expliziter Arbeitsmarkt; Investitions-/Kapitalgütersektor; Multi-Input-/Multi-Output-Rezepte; Firmen-Glyph + Inspector-Ausbau (Option 3 aus dem Brainstorm); Insolvenz/Markteintritt; elastische Inputsubstitution.

## References (APA7)

- Caiani, A., Godin, A., Caverzasi, E., Gallegati, M., Kinsella, S., & Stiglitz, J. E. (2016). Agent based-stock flow consistent macroeconomics: Towards a benchmark model. *Journal of Economic Dynamics and Control, 69*, 375–408. https://doi.org/10.1016/j.jedc.2016.06.001
- Carvalho, V. M., & Tahbaz-Salehi, A. (2019). Production networks: A primer. *Annual Review of Economics, 11*, 635–663. https://doi.org/10.1146/annurev-economics-080218-030212
- Godley, W., & Lavoie, M. (2007). *Monetary economics: An integrated approach to credit, money, income, production and wealth*. Palgrave Macmillan. https://doi.org/10.1057/9780230626546
