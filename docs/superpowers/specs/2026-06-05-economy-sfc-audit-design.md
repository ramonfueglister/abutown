# Economy: Release-Grade SFC Conservation Audit — Design Spec

**Datum:** 2026-06-05
**Status:** Design (approved) → Plan
**Kontext:** Nach #74/#75/#76/#77 ist der Geldkreislauf geschlossen und `total_money` byte-invariant — aber nur durch **Tests** bewiesen. Die Laufzeit-Wächter sind `debug_assert` (im Release elidiert), `total_money()` wird nur in Test-Code aufgerufen, und es gibt **kein** queryfähiges Konservierungs-Signal. Diese Slice macht die SFC-Konservierung **zur Laufzeit erzwungen + beobachtbar**, auch im Release — der Sicherheits-Canary, auf dem die kommende Multi-Stage-Slice (Firmen werden Käufer → der latente Profit-Leak geht live) aufbauen kann.

## 1. Problem

- **`HOUSEHOLD_SECTOR`-Net-Zero-Wächter sind `debug_assert_eq!`** (`wages.rs:147`, `:232`) → im `--release`-Build **elidiert**. Eine gestrandete-Cash-Regression im Sentinel bliebe in Produktion unentdeckt.
- **`total_money()`** (`accounts.rs:97`) existiert, wird aber **nur** in `tests/conservation.rs` aufgerufen. Kein Laufzeit-Check, dass Geld byte-invariant bleibt.
- **Kein `TickAudit`-Event / kein queryfähiges Konservierungs-Surface.** `EconomyEvent` (ledger.rs) hat Trade/WagePaid/ProfitDistributed/… aber nichts, das pro Tick „Geld ist konserviert: total=X" festhält.
- Konsequenz: ein geld-erzeugender/-vernichtender Bug (z. B. ein künftiger Käufer-Pfad, der den Profit-Leak triggert) würde in Produktion **still** korrumpieren.

## 2. Ziel & Scope

**Garantiert:** pro Tick, **auch im Release**, (a) `total_money` ist byte-invariant gegenüber dem Vortick — bei Abweichung **fail-fast** (Lead-Entscheid: unrecoverable Invariant-Verletzung → laut halten); (b) die `HOUSEHOLD_SECTOR`-Net-Zero-Wächter sind release-grade; (c) ein `TickAudit`-Event hält den Konservierungs-Trace fest (queryfähig via #68-Audit-Store). **Reine Beobachtung/Erzwingung — ändert KEIN ökonomisches Verhalten** (kein Geld bewegt, kein Preis/Menge berührt). **Kein neues persistiertes Feld → keine `DELETE FROM economy_snapshots`-Migration.**

**Aufgeschoben (nicht in dieser Slice):** Per-Gut-Ledger-Reconciliation zur Laufzeit (Güter sind ein Fluss `Δtotal_good == Σ(Produced+Regenerated)−Σ(Consumed+FinalConsumed)`, #73 — teurer, pro-Tick-Event-Akkumulation nötig; bleibt Test-only via `conservation_full_plugin_multi_tick`); die Profit-Leak-**Recovery** (gehört zur Multi-Stage-Slice, wo Käufer den Leak live machen); ein `/economy/events`-Read-API über dem `TickAudit`-Event.

## 3. Mechanismus

Ein neues System läuft als **letztes** der Tick-Kette (nach `UpdateConsumption`, also nachdem ALLE Geldbewegungen des Ticks settled sind), `.before(tick_increment_system)`.

**Pure-Kern** (`audit.rs` oder `accounts.rs` — Plan entscheidet, eigene fokussierte Stelle):
```
run_tick_audit_at_tick(accounts: &AccountBook, ledger: &mut TradeLedger,
                       last: &mut LastTickMoney, current_tick: u64) -> Result<(), EconomyError>
  total = accounts.total_money()?            // Err = Overflow beim Summieren = selbst ein Fault
  if let Some(prev) = last.0 {
      if total != prev { return Err(EconomyError::ConservationViolation) }   // DRIFT
  }
  ledger.0.push(EconomyEvent::TickAudit { tick: current_tick, total_money: total })  // Trace
  last.0 = Some(total)
  Ok(())
```

**System-Wrapper (fail-fast):**
```
run_tick_audit_system(tick, accounts: Res<AccountBook>, mut ledger: ResMut<TradeLedger>, mut last: ResMut<LastTickMoney>)
  run_tick_audit_at_tick(&accounts, &mut ledger, &mut last, tick.0)
      .expect("CONSERVATION VIOLATION: total_money changed between ticks (money minted/destroyed) — the SFC byte-invariant is broken; halting the tick. This must never happen.")
```
Das `.expect` ist der **fail-fast**: ein Drift panikt den Schedule-Run → der Server-Tick-Loop bricht laut ab (kein stilles Weiterlaufen mit korrumpiertem State). Konsistent mit den bestehenden „dies ist unmöglich"-`.expect`s der Codebase (z. B. der Regen-Cursor-`.expect`).

**Baseline ohne Persistenz (kein DELETE):** `LastTickMoney(pub Option<Money>)` ist eine **ephemere** Resource (Default `None`, NICHT in `EconomyPersistSnapshot`). Erster Audit-Tick: `None` → initialisieren, kein Check. Danach: `total == prev`. Da Geld nur beim **Seed** geprägt wird (einmalig, vor dem ersten Tick) und danach ausschließlich via `transfer` bewegt wird, ist `total_money` post-Seed konstant — der Tick-über-Tick-Vergleich ist exakt. Auf einer **hydratisierten** World läuft kein Seed; `total_money` ist der restaurierte (konservierte) Wert; der erste Audit-Tick re-initialisiert die Baseline → über Restarts konsistent OHNE Persistenz.

**Sentinel-Upgrade (release-grade):** in `run_pay_wages_at_tick` (`wages.rs:147`) und `run_distribute_profit_at_tick` (`:232`) das `debug_assert_eq!(account(HOUSEHOLD_SECTOR).available, Money::ZERO)` ersetzen durch
```
if accounts.account(HOUSEHOLD_SECTOR).available != Money::ZERO {
    return Err(EconomyError::ConservationViolation);
}
```
**Wrapper-Asymmetrie (wichtig, vom Review verifiziert):** `run_pay_wages_system` nutzt `.expect` (systems.rs:508) → ein Sentinel-`Err` paniert dort **direkt** fail-fast. ABER `run_distribute_profit_system` (systems.rs:525-538) macht **bewusst KEIN** `.expect` — es degradiert jeden `Err` zu einem `MarketClearFailed`-Event und läuft weiter (#75-Design: die Profit-Verteilung ist *genuin* fallible — der underfunded-firm-Shortfall ist ein recoverbarer `InsufficientFunds`-Fault). Ein naives `return Err(ConservationViolation)` am Profit-Sentinel würde dort also **soft geschluckt**, nicht paniken. **Fix (bewahrt den fail-fast-Entscheid):** der Profit-Wrapper unterscheidet die Reasons — bei `EconomyError::ConservationViolation` (ein unrecoverbarer Invariant-Bruch) **`.expect`/panic** (fail-fast), bei den genuin-fallible Reasons (`InsufficientFunds` etc.) bleibt der weiche `MarketClearFailed`-Audit-Pfad. So ist der Net-Zero-Wächter auf BEIDEN Pfaden release-grade fail-fast, ohne #75s legitimes Soft-Handling des recoverbaren Shortfalls zu brechen. **Unabhängig davon** ist das End-of-Tick-`total_money`-Audit (§3 oben) der globale Backstop: jeder echte Geld-Drift paniert dort, egal welcher lokale Pfad ihn verursacht.

## 4. Komponenten / Berührte Files (alle unter `backend/crates/sim-core/src/economy/`)

- `money.rs`: neue `EconomyError::ConservationViolation`-Variante (ehrlich benannt).
- `ledger.rs`: neue `EconomyEvent::TickAudit { tick: u64, total_money: Money }` + `event_type()`-Arm `"tick_audit"`.
- `audit.rs` (oder `accounts.rs`): `run_tick_audit_at_tick` (Pure-Kern) + `LastTickMoney(pub Option<Money>)`-Resource.
- `systems.rs`: `EconomySet::TickAudit` ans Ende der `configure_sets(...).chain()` (nach `UpdateConsumption`); `run_tick_audit_system`-Wrapper (`.expect`, fail-fast); Registrierung `.before(tick_increment_system)`.
- `wages.rs`: zwei Sentinel-`debug_assert_eq!` → `if … != ZERO { return Err(ConservationViolation) }` (release-grade).
- `mod.rs`: `LastTickMoney` in `EconomyPlugin::install` registrieren (Default `None`).
- `persist.rs`: **kein Change** (kein neues persistiertes Feld; `LastTickMoney` ephemer).
- `tests/{audit,conservation,wages,persist}.rs`: §6.

## 5. Konservierung, Determinismus, No-Fallback, Persistenz

- **Konservierung:** das Audit-System bewegt **kein** Geld/Güter — es liest `total_money()` und schreibt nur ein Event + die ephemere Baseline. `total_money` trivial unberührt.
- **Determinismus:** `total_money()` summiert keys-first über `AccountBook` (BTreeMap) **`available + locked` pro Konto** (accounts.rs:97-104, `checked_add` → `Overflow`), i64/i128, kein float/RNG. (Dass der locked-Anteil mitgezählt wird, ist genau, WARUM `lock_cash`/`release_cash`/`debit_locked` konservierungs-neutral sind — die gesperrte Leg bleibt im Total.) Der `TickAudit`-Event-Inhalt ist deterministisch (tick + total). Gleiche Inputs → byte-identisch.
- **NO-FALLBACK / ehrliche Errors:** ein Drift ist **kein** zu tolerierender Zustand — `Err(ConservationViolation)` → `.expect`-panic (fail-fast). Kein `unwrap_or`, kein stiller Default, kein `let _`. Die `total_money()?`-Overflow-Variante wird ehrlich propagiert.
- **Persistenz:** KEIN neues persistiertes **Feld** (`LastTickMoney` ephemer, NICHT in `EconomyPersistSnapshot`; `EconomyConfig` unberührt; `persist.rs`-Code unverändert). ABER: der persistierte `ledger_tail: Vec<EconomyEvent>` (persist.rs:54, #61-bounded) **trägt** nun additiv `TickAudit`-Events, sobald welche im Tail zum Flush-Zeitpunkt stehen. Das bricht **nicht** die Deserialisierung alter Snapshots: serde-extern-getaggte Enums lesen alte Daten fehlerfrei (alte `ledger_tail` enthält die Variante schlicht nicht), kein `schema_version`-Bump, **kein DELETE**. Der §6-Round-Trip-Test (ein `TickAudit` im Tail) sichert das ab.
- **1M-Skalierung:** O(|accounts|) pro Tick (ein `total_money()`-Sweep, ~Sektoren-Anzahl), viewport-unabhängig. `TickAudit` ein Event/Tick (Volumen vom #68-Drain + #61-Bounded-Tail getragen; ein Cadence-Gate wäre ein trivialer Follow-on falls nötig — bewusst NICHT vorgezogen, der Per-Tick-Trace ist das wertvollere Audit-Surface).

## 6. Tests (TDD)

1. **TickAudit-Event:** nach einem Tick (voller Plugin) enthält der Ledger genau ein `TickAudit { tick, total_money }` mit `total_money == accounts.total_money()`; `event_type()=="tick_audit"`.
2. **Konserviert → kein Panic:** über N Ticks (voller Plugin, der bestehende self-sustaining Aufbau) feuert das Audit jeden Tick, kein Panic, N `TickAudit`-Events; `LastTickMoney` trackt den konstanten Wert.
3. **Drift → fail-fast:** der Pure-Kern `run_tick_audit_at_tick` mit `last=Some(X)` und einem `AccountBook`, dessen `total_money()=Y≠X`, liefert `Err(ConservationViolation)` (Pure-Test). Zusätzlich ein `#[should_panic]`-Test, der mitten im Lauf Geld in ein Konto injiziert (`accounts.deposit(...)`) und beweist, dass der nächste Audit-Tick paniert.
4. **Sentinel release-grade:** `run_pay_wages_at_tick` / `run_distribute_profit_at_tick` mit einem künstlich non-zero gelassenen `HOUSEHOLD_SECTOR` (oder einem Setup, das den Sentinel bräche) liefert `Err(ConservationViolation)` — und im Normalfall (Sentinel netto null) weiterhin `Ok`. (Bestehende wage/profit-Tests bleiben grün, weil der Sentinel dort immer null ist.)
4. **Determinismus:** zweimal derselbe Tick-Input → identischer `TickAudit` + identische Baseline.
5. **Persist-Round-Trip unberührt:** ein Snapshot round-trippt verlustfrei OHNE `LastTickMoney`-Feld (kein neues Feld); der bestehende `snapshot_without_…`-Stil bleibt; ein `TickAudit`-Event im `ledger_tail` round-trippt verlustfrei.
6. **Nicht-Destabilisierung:** `conservation_full_plugin_multi_tick` + `steady_state_multi_tick` bleiben grün mit aktivem Audit (das Audit bestätigt dieselbe Geld-Invariante, die die Tests schon prüfen — konsistent, kein Konflikt).

## 7. Sub-Slice-Dekomposition (ein PR)

- **A (Fehler/Event/Resource):** `EconomyError::ConservationViolation`; `EconomyEvent::TickAudit` + event_type; `LastTickMoney`-Resource + mod.rs-Registrierung. Trivial-Tests.
- **B (Audit-System + Schedule):** `run_tick_audit_at_tick` Pure-Kern + Pure-Tests (Konserviert/Drift); `EconomySet::TickAudit` + Wrapper + Chain-Registrierung; voller-Plugin-Event-Test + `#[should_panic]`-Drift-Test.
- **C (Sentinel-Upgrade + Konservierungs-Nicht-Destabilisierung):** wages.rs-Sentinels release-grade; Sentinel-Err-Test; bestehende multi-tick-Tests grün; Persist-Round-Trip; voller lokaler Gate.

## 8. Offene Entscheidungen / Aufgeschoben

1. **Fail-fast bei Drift** (bestätigt vom Lead — `.expect`-panic, unrecoverable).
2. **Per-Tick TickAudit-Event** (bestätigt — kein Cadence-Gate v0; Volumen via #68/#61 getragen).
3. **Aufgeschoben:** Per-Gut-Laufzeit-Reconciliation; Profit-Leak-Recovery (Multi-Stage-Slice); `/economy/events` Read-API; ein per-Markt/Sektor-Konservierungs-Breakdown im `TickAudit`.
