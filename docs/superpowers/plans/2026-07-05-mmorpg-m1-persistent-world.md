# MMORPG M1 — Persistenter Welt-Server: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ein autonomes, persistentes Winterthur: Bürger mit echten Wohnungen/Arbeitsplätzen leben einen 4h-Weltentag, speisen den Verkehr, treiben eine geerntete SFC-auditierte Wirtschaft; alles überlebt Server-Neustarts (Supabase-Postgres), mehrere Browser schauen live zu.

**Architecture:** Neue Crate `world-core` (Bürger + Wirtschaft, Meter-Welt) wird als bevy_ecs-Systeme in den bestehenden `winterthur-traffic`-Loop eingehängt; `sim-server` wird das eine Binary (Traffic + Welt + Persistenz + Card-Hand-Routen + WS-Gateway). Wirtschaftskern wird aus Commit `bbd0159` geerntet (geometrie-freie Module 1:1, Rest neu auf Meter-Welt). Spec: `docs/superpowers/specs/2026-07-05-mmorpg-m1-persistent-world-design.md`.

**Tech Stack:** Rust (bevy_ecs 0.18, axum 0.8, sqlx 0.8, prost), Protobuf/buf, three.js WebGPU/TSL Frontend, Supabase Postgres, Fly.io.

## Global Constraints

- Cargo IMMER über `scripts/cargo-serial.sh` (z.B. `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p world-core`). Nie zwei cargo parallel. Vor Start `pgrep -f cargo`.
- Supabase-Postgres NUR über `:5432` (Session-Pooler). `:6543` crasht sqlx.
- **Kein Welt-Wipe:** jede Snapshot-Schema-Änderung nach dem ersten Deploy braucht eine Migration (`migrate_snapshot`) mit Test. Kein `DELETE FROM`-Ritual.
- Single-Writer: Fly `count=1`.
- Determinismus: kein `HashMap`-Iterieren in Sim-Pfaden (BTreeMap), Zufall nur via `traffic_core::u01` (splitmix64), keine Wanduhr in der Sim ausser über `WorldClock`.
- proto-Änderungen: `buf lint` + `npm run generate:proto` müssen grün bleiben; nur additive Felder.
- Geld = `Money(i64)` skaliert ×1000, wie geerntet. SFC-Audit-Verstoss = fail-fast, keine Heilungs-Guards (User-Regel: kein Legacy/Fallback-Cruft).
- Frontend-berührende Features sind erst fertig nach echtem Browser-Smoke (CLAUDE.md-Regel).
- Basis-Branch: `mmorpg/m1-spec` (ab origin/main). PRs gegen `main` auf GitHub.
- Vor jedem Push: volles lokales Gate (fmt-check, clippy -D warnings, cargo test, npm typecheck+test+build).

## Datei-Landkarte (Neu/Geändert)

```
backend/crates/world-core/            NEU — die Welt-Sim
  src/lib.rs                          Plugin-Export, Modul-Wiring
  src/clock.rs                        WorldClock (4h-Tag, frozen-time)
  src/model/mod.rs                    SimWorld (Gebäude, Zugänge), Loader
  src/model/building.rs               BuildingId, BuildingState (Lebenszyklus)
  src/econ/…                          geerntete Economy-Module (aus bbd0159)
  src/citizens/mod.rs                 Citizen-Komponenten + Seeding
  src/citizens/rhythm.rs              Tagesrhythmus-Systeme
  src/citizens/trips.rs               Brücke Bürger→Traffic-Core
  src/persist.rs                      WorldCoreSnapshot v1 + extract/apply + Migrationskette
  src/systems.rs                      Schedule-Installation (Reihenfolge)
backend/crates/sim-server/
  src/main.rs                         wird DER Prozess: ECS-Loop + Router
  src/world_store.rs                  Postgres world_core_snapshots Store
  migrations/202607050001_world_core_snapshots.sql
backend/crates/winterthur-traffic/    shell/gateway erweitert (World-Kanal)
backend/crates/protocol/proto/live.proto   NEU — Citizen/Vitals/WorldDelta wire
scripts/geo/bake-sim-world.mjs        NEU — kompaktes Sim-Welt-Artefakt
data/winterthur/simworld.json         NEU — committed (~5–8 MB)
data/winterthur/economy.json          NEU — authored Markt-/Firmen-Seeds
src/diorama/live/liveClient.ts        NEU — WS-Client für Live-Kanal
src/diorama/live/citizensLayer.ts     NEU — instanzierte Bürger-Renderer
src/diorama/live/vitalsHud.ts         NEU — Vitals-Karte
scripts/smoke-world.mjs               NEU — 2-Client- + Resume-Smoke
```

Ausführung in 7 Phasen (A–G). Innerhalb einer Phase sind Tasks sequentiell; jede Task endet grün + committed.

---

## Phase A — Fundament

### Task 1: Sim-Welt-Artefakt backen (`simworld.json`)

Der Sim-Server braucht Gebäude (ID, Nutzung, Position, Fläche, Strassen-Zugang) OHNE die 77-MB-Render-Pyramide. Wir backen ein kompaktes, committetes Artefakt aus denselben Quellen wie `bake-world.mjs`.

**Files:**
- Create: `scripts/geo/bake-sim-world.mjs`
- Create: `data/winterthur/simworld.json` (Output, committed)
- Test: `tests/geo/simworld.test.ts`

**Interfaces:**
- Produces: `data/winterthur/simworld.json` mit Schema:
```json
{
  "meta": { "anchor": {"lon": 8.7285, "lat": 47.5069}, "bake_version": 1, "source": "bake-world inputs" },
  "buildings": [
    { "id": "{UUID}", "usage": 1, "x": -123.4, "z": 567.8, "area_m2": 210.5,
      "height_m": 12.3, "access_edge": 4711, "access_offset": 12.5 }
  ]
}
```
  - `usage` = Usage-Enum aus world.proto (0–5), `x/z` = Footprint-Centroid in lokalen Metern (gleicher Anker), `access_edge/offset` = RoadGraph-Kante (graph.pb-Index) + Meter entlang der Kante.

- [x] **Step 1: Failing test schreiben** — `tests/geo/simworld.test.ts` (vitest, läuft nur wenn Datei existiert → Test prüft Existenz + Invarianten):

```ts
import { describe, it, expect } from 'vitest';
import { readFileSync, existsSync } from 'node:fs';

const PATH = 'data/winterthur/simworld.json';

describe('simworld artifact', () => {
  it('exists and is committed', () => {
    expect(existsSync(PATH)).toBe(true);
  });
  it('has plausible building inventory', () => {
    const w = JSON.parse(readFileSync(PATH, 'utf8'));
    expect(w.meta.anchor.lon).toBeCloseTo(8.7285, 4);
    expect(w.buildings.length).toBeGreaterThan(20000);
    expect(w.buildings.length).toBeLessThan(40000);
    const usages = new Set(w.buildings.map((b: any) => b.usage));
    expect(usages.has(1)).toBe(true); // residential
    expect(usages.has(2) || usages.has(3)).toBe(true); // work
    const withAccess = w.buildings.filter((b: any) => b.access_edge >= 0);
    expect(withAccess.length / w.buildings.length).toBeGreaterThan(0.9);
    for (const b of w.buildings.slice(0, 500)) {
      expect(typeof b.id).toBe('string');
      expect(Number.isFinite(b.x) && Number.isFinite(b.z)).toBe(true);
      expect(b.area_m2).toBeGreaterThan(0);
    }
  });
});
```

- [x] **Step 2: Test laufen lassen** — `npx vitest run tests/geo/simworld.test.ts` → FAIL (Datei fehlt).
- [x] **Step 3: `scripts/geo/bake-sim-world.mjs` schreiben.** Kein neuer Fetch: das Script liest die GLEICHEN Zwischenprodukte wie `bake-world.mjs` (scratch/geo/…). Wenn `scratch/` fehlt, bricht es mit klarer Meldung ab (`npm run geo:fetch` zuerst). Struktur: kopiere aus `scripts/geo/bake-world.mjs` die Schritte Boundary→lokal, GDB-Gebäude-Extraktion, Usage-Klassifikation (die `usageNum()`-Regex-Zuordnung), Road-Graph-Bau und Access-Point-Berechnung (`scripts/geo/lib/access.mjs`) — aber statt Tiles zu enkodieren, schreibe pro Gebäude Centroid + Shoelace-Fläche:

```js
function centroidAndArea(ring) {
  let a = 0, cx = 0, cz = 0;
  for (let i = 0; i < ring.length; i++) {
    const [x1, z1] = ring[i], [x2, z2] = ring[(i + 1) % ring.length];
    const w = x1 * z2 - x2 * z1;
    a += w; cx += (x1 + x2) * w; cz += (z1 + z2) * w;
  }
  a /= 2;
  return { x: cx / (6 * a), z: cz / (6 * a), area: Math.abs(a) };
}
```

Output deterministisch: Gebäude nach `id` sortiert, Zahlen auf 2 Dezimalen gerundet (`Math.round(v*100)/100`), `JSON.stringify` mit stabiler Feldreihenfolge. Gate im Script: 20k–40k Gebäude, ≥90% mit access_edge, sonst `process.exit(1)`.
- [x] **Step 4: Backen + Test grün** — `npm run geo:fetch` falls scratch fehlt (dauert; siehe Memory diorama-smoke-needs-world-bake), dann `node scripts/geo/bake-sim-world.mjs`, dann `npx vitest run tests/geo/simworld.test.ts` → PASS. Zweiter Bake-Lauf → `git diff --stat data/winterthur/simworld.json` leer (Determinismus-Beweis).
- [x] **Step 5: package.json-Script `geo:bake-sim` ergänzen, committen** — `git add scripts/geo/bake-sim-world.mjs data/winterthur/simworld.json tests/geo/simworld.test.ts package.json && git commit -m "feat(geo): bake compact simworld artifact for world-core"`.

### Task 2: Crate `world-core` + `SimWorld`-Loader

**Files:**
- Create: `backend/crates/world-core/Cargo.toml`, `src/lib.rs`, `src/model/mod.rs`, `src/model/building.rs`
- Modify: `backend/Cargo.toml` (workspace member)

**Interfaces:**
- Produces:
```rust
pub struct BuildingId(pub u32);              // dichter Index in SimWorld.buildings (stabil pro Bake via Sortierung nach UUID)
pub enum BuildingLifecycle { Occupied, Vacant, Decaying, Demolished, UnderConstruction }
pub struct SimBuilding { pub uuid: String, pub usage: Usage, pub x: f32, pub z: f32,
    pub area_m2: f32, pub height_m: f32, pub access_edge: i64, pub access_offset: f32 }
pub enum Usage { Unknown, Residential, Commercial, Industrial, Public, Agriculture } // = world.proto Zahlenwerte
pub struct SimWorld { pub buildings: Vec<SimBuilding>, /* usage-Indizes */ }
impl SimWorld {
    pub fn load(json: &str) -> Result<SimWorld, WorldError>;
    pub fn residential(&self) -> &[u32];   // BuildingId-Indizes
    pub fn workplaces(&self) -> &[u32];    // Commercial+Industrial+Public
    pub fn within_radius(&self, cx: f32, cz: f32, r: f32) -> Vec<u32>;
}
```
- Consumes: `data/winterthur/simworld.json` (Task 1).

- [x] **Step 1: Crate anlegen** — `backend/crates/world-core/Cargo.toml`:

```toml
[package]
name = "world-core"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
thiserror = { workspace = true }
bevy_ecs = { workspace = true }
tracing = { workspace = true }
traffic-core = { path = "../traffic-core" }
traffic-net = { path = "../traffic-net" }
```

(Workspace-Dependency-Namen exakt aus `backend/Cargo.toml` übernehmen; fehlt eine im Workspace-`[workspace.dependencies]`, dort ergänzen.) Member in `backend/Cargo.toml` eintragen.
- [x] **Step 2: Failing test** — in `src/model/mod.rs` `#[cfg(test)]`-Modul; Fixture INLINE (kein Filesystem im Unit-Test):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &str = r#"{
      "meta": {"anchor": {"lon": 8.7285, "lat": 47.5069}, "bake_version": 1},
      "buildings": [
        {"id":"{B1}","usage":1,"x":0.0,"z":0.0,"area_m2":200.0,"height_m":9.0,"access_edge":5,"access_offset":2.0},
        {"id":"{A2}","usage":2,"x":100.0,"z":0.0,"area_m2":400.0,"height_m":12.0,"access_edge":7,"access_offset":1.0},
        {"id":"{C3}","usage":0,"x":500.0,"z":500.0,"area_m2":50.0,"height_m":4.0,"access_edge":-1,"access_offset":0.0}
      ]}"#;

    #[test]
    fn loads_and_indexes_by_usage() {
        let w = SimWorld::load(FIXTURE).unwrap();
        assert_eq!(w.buildings.len(), 3);
        // sortiert nach uuid: {A2},{B1},{C3}
        assert_eq!(w.buildings[0].uuid, "{A2}");
        assert_eq!(w.residential(), &[1]);
        assert_eq!(w.workplaces(), &[0]);
        assert_eq!(w.within_radius(0.0, 0.0, 150.0), vec![0, 1]);
    }
}
```

- [x] **Step 3:** `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p world-core` → FAIL (Typen fehlen).
- [x] **Step 4: Implementieren.** `SimWorld::load` parst JSON (serde), **sortiert Gebäude nach `uuid`** (macht BuildingId=Index bake-stabil), baut `residential`/`workplaces`-Indizes einmalig. `within_radius` = linearer Scan (29k Einträge, nur beim Seeding benutzt — kein Hot-Path). `usage` ausserhalb 0–5 → `WorldError::BadUsage`.
- [x] **Step 5:** Test grün → PASS. fmt+clippy: `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml` und `clippy -p world-core -- -D warnings`.
- [x] **Step 6: Commit** — `git commit -m "feat(world-core): crate skeleton + SimWorld loader"`.

### Task 3: `WorldClock` — 4h-Weltentag, frozen-time

**Files:**
- Create: `backend/crates/world-core/src/clock.rs`

**Interfaces:**
- Produces:
```rust
pub const WORLD_TIME_SCALE: u64 = 6;          // 24h Welt in 4h real
pub const TICKS_PER_SECOND: u64 = 10;         // = Traffic-Loop (DT 0.1s)
#[derive(bevy_ecs::prelude::Resource, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct WorldClock { pub world_tick: u64 } // persistiert; NUR Ticks, keine Wanduhr
impl WorldClock {
    pub fn advance(&mut self) { self.world_tick += 1 }
    pub fn world_seconds(&self) -> u64;        // world_tick / 10 * 6
    pub fn s_of_world_day(&self) -> u32;       // world_seconds % 86_400
    pub fn world_day(&self) -> u64;            // world_seconds / 86_400
}
```
- Später konsumiert von: Rhythmus-Systemen (Task 8), Traffic-Spawner-Umhängung (Task 9), Persistenz (Task 10).

- [x] **Step 1: Failing tests** (im selben File):

```rust
#[test]
fn four_real_hours_is_one_world_day() {
    let mut c = WorldClock { world_tick: 0 };
    for _ in 0..(4 * 3600 * TICKS_PER_SECOND) { c.advance(); }
    assert_eq!(c.world_day(), 1);
    assert_eq!(c.s_of_world_day(), 0);
}
#[test]
fn resume_continues_exactly() {
    let c = WorldClock { world_tick: 987_654 };
    let s = serde_json::to_string(&c).unwrap();
    let r: WorldClock = serde_json::from_str(&s).unwrap();
    assert_eq!(r.world_tick, 987_654);
    assert_eq!(r.s_of_world_day(), c.s_of_world_day());
}
```

- [x] **Step 2:** Test FAIL → implementieren (reine Ganzzahl-Arithmetik: `world_seconds = world_tick * WORLD_TIME_SCALE / TICKS_PER_SECOND`) → PASS.
- [x] **Step 3: Commit** — `git commit -m "feat(world-core): WorldClock — 4h world day, tick-based frozen time"`.

---

## Phase B — Economy-Ernte

### Task 4: Geometrie-freie Economy-Module ernten

Quelle: Commit `bbd0159`, Pfad `backend/crates/sim-core/src/economy/`. Der Harvest-Report (unten „Ernte-Tabelle") listet Modul→Status. Ernte heisst: `git show bbd0159:backend/crates/sim-core/src/economy/<f>.rs > backend/crates/world-core/src/econ/<f>.rs`, dann Pfade/Imports anpassen. **Inklusive der vorhandenen Unit-Tests in den Dateien.**

**Files:**
- Create: `backend/crates/world-core/src/econ/{mod,ids,money,accounts,inventory,goods,orders,auction,ledger,pools,production,producers,market_goods,pricing,wages,flow_shipments,flow_telemetry,audit}.rs`

**Interfaces:**
- Produces (unverändert aus Ernte, zentral):
```rust
pub struct Money(pub i64); pub struct Quantity(pub i64);
pub struct AccountBook(BTreeMap<EconomicActorId, MoneyAccount>);
impl AccountBook { pub fn total_money(&self) -> Result<Money, EconomyError>; /* deposit/lock/transfer… */ }
pub fn build_clearing_plan(key: MarketGoodKey, bids: &[…], asks: &[…], last_price: Money) -> Result<ClearingPlan, EconomyError>;
pub fn run_tick_audit_at_tick(accounts: &AccountBook, ledger: &mut TradeLedger, last: &mut LastTickMoney, tick: u64) -> Result<(), EconomyError>;
```

**Ernte-Tabelle (Modul → Aktion):**

| Datei | Aktion |
|---|---|
| ids, money, accounts, inventory, goods, orders, auction, ledger, pools, production, producers, market_good*, pricing, wages, flow_shipments, flow_telemetry, audit | 1:1 ernten, Imports auf `crate::econ::…` umschreiben |
| market.rs | NUR `MarketGoodKey`-freie Teile; `MarketSite` NEU (Task 5) — altes `node_id`-Feld NICHT übernehmen |
| transport.rs | NICHT ernten; Distanz = `fn euclid_m(a:(f32,f32), b:(f32,f32)) -> i64` (Meter, gerundet) neu in econ/mod.rs |
| macro_flow.rs | Kern ernten; `DormantMarkets`-Gate ERSETZEN durch „nie dormant" (M1: alle Märkte immer aktiv — kleine Marktzahl) |
| capita.rs | Arithmetik ernten; `live_agent_count` liest künftig `CitizenRegistry` (Task 7) statt `AgentMarker` |
| attribution.rs, materialize.rs, trader_render.rs | NICHT ernten (M1 ohne Attribution/Trader-Renderer) |
| systems.rs | NICHT ernten — Neuaufbau in Task 6 (schlankere Kette ohne Attribution/Materialize/LOD) |

- [x] **Step 1:** Module per `git show` extrahieren (Reihenfolge: ids→money→accounts→inventory→goods→orders→auction→ledger→market_goods→pools→production→producers→pricing→wages→flow_shipments→flow_telemetry→audit), `econ/mod.rs` mit `pub mod`-Liste anlegen. Bei jedem Modul: Imports fixen, auskompilieren lassen.
- [x] **Step 2:** `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p world-core` → die geernteten Unit-Tests (Dutzende: Auktion, EWMA, Konservierung, prorata…) PASSEN. Scheitert ein geernteter Test, ist die Ernte falsch — nicht den Test ändern.
- [x] **Step 3:** clippy -D warnings + fmt grün.
- [x] **Step 4: Commit** — `git commit -m "feat(world-core): harvest geometry-free economy core from bbd0159 (accounts, auction, pricing, wages, SFC audit)"`.

### Task 5: Märkte/Firmen auf echten Orten seeden (`economy.json`)

**Files:**
- Create: `data/winterthur/economy.json` (authored, committed)
- Create: `backend/crates/world-core/src/econ/seed.rs`

**Interfaces:**
- Produces:
```rust
pub struct MarketSite { pub id: MarketId, pub name: String, pub x: f32, pub z: f32 }
pub struct EconomySeed { /* geparst aus economy.json */ }
pub fn seed_economy(world: &mut bevy_ecs::world::World, seed: &EconomySeed, sim: &SimWorld) -> Result<(), EconomyError>;
```
- `seed_economy` ist idempotent (zweiter Aufruf no-op wenn `Markets` nicht leer) — Guard fürs Resume (Lehre aus PR #86: NIE vor apply_into_world seeden).

**economy.json (authored — echte Winterthur-Orte, lokale Meter via bekannter Anker-Transform):**

```json
{
  "capita_baseline": 1000000,
  "markets": [
    { "id": 1, "name": "Marktgasse",  "lon": 8.7296, "lat": 47.4996 },
    { "id": 2, "name": "Neumarkt",    "lon": 8.7280, "lat": 47.4989 },
    { "id": 3, "name": "Grüze Industrie", "lon": 8.7594, "lat": 47.4923 },
    { "id": 4, "name": "Töss",        "lon": 8.7075, "lat": 47.4886 }
  ],
  "goods": { "food": 1, "wood": 2, "tools": 4 },
  "firms": [
    { "actor": 9001, "market": 3, "recipe": {"in": [["wood", 2]], "out": [["tools", 1]]}, "interval_ticks": 600 },
    { "actor": 9002, "market": 4, "raw": ["wood", 5], "interval_ticks": 600 },
    { "actor": 9003, "market": 1, "raw": ["food", 8], "interval_ticks": 600 }
  ],
  "initial_cash": { "9001": 500000, "9002": 200000, "9003": 200000, "household": 2000000 }
}
```

(lon/lat → lokale Meter im Loader über die Anker-Formel aus `scripts/geo/lib/project.mjs`, in Rust nachgebaut: `x=(lon-8.7285)*rad*R*cos(47.5069°)`, `z=-(lat-47.5069)*rad*R`, R=6371008.8.)

- [x] **Step 1: Failing test** in `seed.rs`: `seed_economy` auf frischer `World` mit dem 3-Gebäude-Fixture aus Task 2 → danach `Markets.len()==4`, `AccountBook.total_money()` == Summe aus initial_cash, zweiter Aufruf ändert nichts (`total_money` identisch, Markets weiterhin 4).
- [x] **Step 2:** FAIL → implementieren: JSON parsen, lon/lat→Meter, Resources befüllen (`Markets`, `AccountBook` via deposit, `RawDeposit`/`InputPool`/`ProductionPool` gemäss firms, `MarketDistances` = euclid_m paarweise, `HouseholdSector { population: 0, … }`). → PASS.
- [x] **Step 3: Commit** — `git commit -m "feat(world-core): authored economy seed on real Winterthur locations"`.

### Task 6: Economy-Tick-Kette (`systems.rs`) + Konservierungs-Beweis

**Files:**
- Create: `backend/crates/world-core/src/systems.rs`
- Modify: `backend/crates/world-core/src/lib.rs`

**Interfaces:**
- Produces:
```rust
pub struct WorldCorePlugin { pub seed: EconomySeed, pub sim_world: std::sync::Arc<SimWorld> }
pub fn install_world_systems(world: &mut World, schedule: &mut Schedule, plugin: &WorldCorePlugin);
// Economy-Kette läuft NUR jeden 10. Tick (1 Hz): EconomyCadence-Guard
```
- Kette (an geernteten `run_*_at_tick`-Funktionen, in dieser Reihenfolge): `expire_orders → regen → production → generate_pool_orders → clear_markets → macro_flow → pay_wages → distribute_profit → consume → adjust_reservation_prices → consumption_update → tick_audit`. Audit-`Err` ⇒ `panic!` mit Kontext (fail-fast, Spec).

- [x] **Step 1: Failing Integrationstest** `backend/crates/world-core/tests/econ_loop.rs`:

```rust
use bevy_ecs::prelude::*;
use world_core::*;

#[test]
fn thousand_econ_ticks_conserve_money_and_trade() {
    let (mut world, mut schedule) = build_test_sim(); // Helper: Fixture-SimWorld + echter economy.json-Seed
    let start = world.resource::<econ::AccountBook>().total_money().unwrap();
    for _ in 0..10_000 { schedule.run(&mut world); }   // 10k Ticks = 1000 Econ-Zyklen
    let end = world.resource::<econ::AccountBook>().total_money().unwrap();
    assert_eq!(start, end, "SFC conservation violated");
    let goods = world.resource::<econ::MarketGoods>();
    assert!(goods.iter().any(|(_, s)| s.traded_qty_last_tick.0 > 0 || s.ewma_reference_price.0 > 0),
        "economy is dead: nothing ever traded");
}
```

- [x] **Step 2:** FAIL → `systems.rs` implementieren (EconomyCadence: `if clock.world_tick % 10 != 0 { return }` als erste Bedingung in jedem System-Wrapper; Systeme via `.chain()`).
- [x] **Step 3:** PASS + clippy/fmt. Läuft der Test >30s, Cadence-Systeme profilieren — nicht den Test kürzen.
- [x] **Step 4: Commit** — `git commit -m "feat(world-core): economy tick chain at 1 Hz with fail-fast SFC audit"`.

---

## Phase C — Bürger

### Task 7: Citizen-Seeding (deterministisch, Stadtteil-Radius)

**Files:**
- Create: `backend/crates/world-core/src/citizens/mod.rs`

**Interfaces:**
- Produces:
```rust
#[derive(Component)] pub struct Citizen { pub id: u32, pub home: u32, pub work: u32 } // BuildingId-Indizes
#[derive(Component)] pub enum CitizenState { AtHome, AtWork, AtMarket { market: MarketId }, Commuting { trip: TripKind } }
#[derive(Resource, Default)] pub struct CitizenRegistry { pub count: u64, pub by_id: BTreeMap<u32, Entity> }
pub struct SeedParams { pub center: (f32, f32), pub radius_m: f32, pub residents_per_40m2: f32, pub seed: u64 }
pub fn seed_citizens(world: &mut World, sim: &SimWorld, p: &SeedParams) -> u64; // returns count; idempotent (no-op wenn Registry nicht leer)
```
- Regeln: Wohnkapazität = `floor(area_m2 * floors / 40)` mit `floors = max(1, round(height_m / 3))`; Arbeitsplatz = nächstgelegenes Workplace-Gebäude gewichtet mit `u01(seed, citizen_id, 0xW0RK)` über die 8 nächsten (Distanz-Rang-Gewichte 8..1); alle Draws via `traffic_core::u01`.

- [x] **Step 1: Failing tests**: (a) Fixture aus Task 2 (1 Wohnhaus 200m²/3 Geschosse → 15 Bürger, alle home=B1, work=A2); (b) Determinismus: zweimal seeden auf frischen Welten → identische `(id, home, work)`-Tripel; (c) Idempotenz: zweiter `seed_citizens`-Aufruf auf derselben Welt → count unverändert.
- [x] **Step 2:** FAIL → implementieren → PASS. `HouseholdSector.population` = count setzen (Wirtschafts-Kopplung).
- [x] **Step 3: Commit** — `git commit -m "feat(world-core): deterministic citizen seeding into real buildings"`.

### Task 8: Tagesrhythmus auf der WorldClock

**Files:**
- Create: `backend/crates/world-core/src/citizens/rhythm.rs`
- Modify: `backend/crates/world-core/src/systems.rs` (System einhängen, vor Economy-Kette)

**Interfaces:**
- Produces:
```rust
#[derive(Resource, Default)] pub struct TripRequests(pub Vec<TripRequest>);
pub struct TripRequest { pub citizen: u32, pub from_building: u32, pub to_building: u32, pub kind: TripKind }
pub enum TripKind { ToWork, ToMarket, ToHome }
pub fn rhythm_system(/* liest WorldClock, Citizen, CitizenState; schreibt TripRequests + CitizenState::Commuting */);
```
- Fahrplan (s_of_world_day, pro Bürger gejittert mit `u01(seed, citizen_id, world_day)` ± 45 Welt-Minuten): 07:30 → ToWork, 12:00 ± Streuung 20% der Bürger → ToMarket (nächster Markt zum Arbeitsplatz) und 13:00 zurück (ToWork-Trip mit Ziel work), 17:30 → ToHome. Ein Bürger emittiert pro Zustand genau EINEN Request (State-Wechsel auf Commuting verhindert Doppel-Emission).

- [x] **Step 1: Failing test**: Welt mit 15 Fixture-Bürgern; WorldClock auf 07:29 Weltzeit stellen (`world_tick` passend), 2 Welt-Stunden ticken → alle 15 haben genau einen ToWork-Request emittiert (nicht mehr), Zustände = Commuting.
- [x] **Step 2:** FAIL → implementieren → PASS.
- [x] **Step 3: Commit** — `git commit -m "feat(world-core): citizen daily rhythm on the 4h world day"`.

### Task 9: Trip-Brücke Bürger↔Verkehr + Ankunft

Bürger-Trips werden echte Autos (lange Wege) oder „Teleport nach Dauer" (kurze Wege, M1 ohne Fussgänger-Routing — Fussweg-Dauer = Distanz / 1.4 m/s, Ankunft per Timer). Damit entsteht der Verkehr aus Bürgern. Census-Demand bleibt als Hintergrund aktiv, aber mit `DEMAND_SCALE` 0.5 (Bürger ersetzen einen Teil).

**Files:**
- Create: `backend/crates/world-core/src/citizens/trips.rs`
- Modify: `backend/crates/winterthur-traffic/src/shell.rs` (Systeme einhängen: `world_rhythm → world_trips` NACH `spawn_trips`, VOR `core_tick`; `WorldClock.advance` als allererstes System)
- Modify: `backend/crates/winterthur-traffic/src/clock.rs` + `spawner.rs`: `s_of_day()` liest **WorldClock** statt Wanduhr (`s_of_world_day()`); `day_kind` bleibt am realen Kalender (Spec: Datum/Saison real)

**Interfaces:**
- Consumes: `TripRequests` (Task 8), `Core::spawn/despawn/vehicle_view` (traffic-core), `TrafficNet` (lane-Lookup über edge), `SimBuilding.access_edge/access_offset`, CH-`router` aus winterthur-traffic.
- Produces:
```rust
#[derive(Resource, Default)] pub struct ActiveTrips(pub BTreeMap<u32 /*citizen*/, ActiveTrip>);
pub enum ActiveTrip { Driving { veh: VehId, dest_building: u32 }, WalkingUntil { arrive_tick: u64, dest_building: u32 } }
pub fn dispatch_trips_system(/* TripRequests → Core.spawn oder WalkingUntil */);
pub fn arrivals_system(/* Driving: vehicle_view none/route-Ende → Ankunft; Walking: arrive_tick erreicht → Ankunft; CitizenState setzen */);
```
- Modus-Wahl: Distanz (euklidisch home→ziel) > 800 m UND beide Gebäude mit access_edge ⇒ Auto, sonst Walk. Auto-Route: CH-Router von access_edge(from) zu access_edge(to); scheitert Routing ⇒ Walk-Fallback (einziger erlaubter Fallback — Netz-Inseln sind real).

- [x] **Step 1: Failing test** `backend/crates/world-core/tests/trips.rs` mit dem Test-Fixture-Netz `backend/crates/traffic-core/tests/fixtures/diamond-gateway.json`: 1 Bürger, home/work an zwei Netz-Kanten >800m, ToWork-Request → nach `dispatch` existiert 1 Fahrzeug im Core; Route bis Ende ticken → `arrivals_system` setzt `CitizenState::AtWork`, Fahrzeug despawnt.
- [x] **Step 2:** FAIL → implementieren → PASS.
- [x] **Step 3: Zweiter Test:** kurzer Weg (<800m) → kein Fahrzeug, `WalkingUntil` korrekt (`dauer = dist/1.4*10 ticks`), nach Ablauf AtWork.
- [x] **Step 4:** Shell-Integration: Systeme in `shell::build_sim`-Schedule einhängen (Signatur um `Option<WorldCorePlugin>` erweitern; Traffic-only-Betrieb bleibt für bestehende Tests/Binary möglich). Bestehende winterthur-traffic-Tests bleiben grün (`scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p winterthur-traffic`).
- [x] **Step 5: Commit** — `git commit -m "feat: citizens drive real trips through the traffic core"`.

---

## Phase D — Persistenz

### Task 10: `WorldCoreSnapshot` v1 + Migrationskette

**Files:**
- Create: `backend/crates/world-core/src/persist.rs`

**Interfaces:**
- Produces:
```rust
pub const WORLD_SNAPSHOT_VERSION: u32 = 1;
#[derive(serde::Serialize, serde::Deserialize)]
pub struct WorldCoreSnapshot {
    pub version: u32,
    pub clock: WorldClock,
    pub citizens: Vec<CitizenSnap>,          // (id, home, work, state, active_trip ohne VehId — Fahrzeuge werden beim Resume als Walk-Rest fortgesetzt)
    pub building_states: Vec<(u32, BuildingLifecycle)>, // nur Abweichungen von Occupied
    pub econ: EconSnap,                       // Felder wie geerntetes EconomyPersistSnapshot minus market_chunks/market_distances(neu berechnet)
}
pub fn extract(world: &World) -> WorldCoreSnapshot;
pub fn apply(world: &mut World, snap: WorldCoreSnapshot);   // Reihenfolge: apply VOR seed-Guards (Lehre #86)
pub fn migrate_snapshot(raw: serde_json::Value) -> Result<WorldCoreSnapshot, MigrateError>;
// migrate_snapshot: liest raw["version"]; v==1 → direkt; unbekannt → Err. Jede künftige Version fügt hier einen Arm hinzu.
```
- Fahrzeug-Policy beim Resume (bewusst, dokumentiert im Code): laufende Auto-Trips werden als `WalkingUntil` mit Rest-Distanz/1.4 m/s fortgesetzt (traffic-core-Fleet ist nicht persistiert — akzeptierter M1-Schnitt; kein Bürger geht verloren, Konservierung unberührt).

- [x] **Step 1: Failing tests:** (a) Roundtrip: Sim 500 Ticks laufen lassen → extract → frische Welt (Seeds via Guards übersprungen) → apply → weitere 500 Ticks → `total_money` identisch mit Durchlauf ohne Unterbrechung? NEIN — zu strikt wegen Trip-Policy; stattdessen: total_money konserviert UND CitizenRegistry.count gleich UND WorldClock fortgesetzt. (b) `migrate_snapshot` mit `{"version": 99}` → Err. (c) `migrate_snapshot(serde_json::to_value(extract(...)))` → Ok.
- [x] **Step 2:** FAIL → implementieren → PASS.
- [x] **Step 3: Commit** — `git commit -m "feat(world-core): versioned snapshot with migration chain (no-wipe principle)"`.

### Task 11: Postgres-Store + Boot-Resume im sim-server

**Files:**
- Create: `backend/crates/sim-server/src/world_store.rs`
- Create: `backend/crates/sim-server/migrations/202607050001_world_core_snapshots.sql`
- Test: `backend/crates/sim-server/tests/world_store.rs` (opt-in via `ABUTOWN_TEST_DATABASE_URL`, Muster der bestehenden opt-in-Tests)

**SQL:**

```sql
CREATE TABLE IF NOT EXISTS world_core_snapshots (
    world_id TEXT PRIMARY KEY,
    tick BIGINT NOT NULL CHECK (tick >= 0),
    schema_version INTEGER NOT NULL,
    payload JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

**Interfaces:**
- Produces:
```rust
pub struct WorldStore { pool: sqlx::PgPool }
impl WorldStore {
    pub async fn with_pool(pool: PgPool) -> Result<Self, StoreError>;    // führt Migration aus (include_str!-Muster)
    pub async fn write(&self, world_id: &str, tick: u64, snap: &WorldCoreSnapshot) -> Result<(), StoreError>; // Upsert ON CONFLICT (world_id)
    pub async fn read(&self, world_id: &str) -> Result<Option<WorldCoreSnapshot>, StoreError>; // via migrate_snapshot
}
```
- Boot-Log-Vertrag (der Resume-Beweis, exakt): `tracing::info!(tick, "resuming world-core from persisted snapshot")` bzw. `"seeding fresh world-core state"`.

- [x] **Step 1: Failing opt-in-Test:** write→read Roundtrip gegen lokale PG (Version bleibt, tick stimmt); read auf fremder world_id → None; write mit höherem tick überschreibt.
- [x] **Step 2:** FAIL (gegen lokale Postgres laufen lassen) → implementieren (SQL-Muster exakt wie `postgres_economy.rs` aus bbd0159, siehe Harvest-Report §7) → PASS.
- [x] **Step 3: Commit** — `git commit -m "feat(sim-server): world_core_snapshots store with boot resume"`.

---

## Phase E — Ein Prozess + Wire

### Task 12: `live.proto` + Codegen beidseitig

**Files:**
- Create: `backend/crates/protocol/proto/live.proto`
- Modify: `backend/crates/protocol/build.rs` (live.proto + world.proto in compile_protos aufnehmen), `backend/crates/protocol/src/lib.rs` (Modul exportieren)
- Run: `npm run generate:proto` (buf erzeugt `src/proto/live_pb.ts`)

**live.proto (vollständig):**

```protobuf
syntax = "proto3";
package live.v1;

message LiveClientMsg {
  repeated uint32 subscribe_cells = 1;    // gleiche 128m-CellGrid-IDs wie traffic
  repeated uint32 unsubscribe_cells = 2;
  optional bool subscribe_vitals = 3;
}

message CitizenState {
  uint32 id = 1;
  sint32 x_dm = 2;                        // Position in Dezimetern (lokale Meter ×10)
  sint32 z_dm = 3;
  uint32 activity = 4;                    // 0=home 1=work 2=market 3=walking 4=driving
}

message CitizenCellFrame {
  uint32 cell = 1;
  uint64 world_tick = 2;
  bool keyframe = 3;
  repeated CitizenState citizens = 4;
  repeated uint32 departed = 5;
}

message EconomyVitals {
  uint64 world_tick = 1;
  uint32 s_of_world_day = 2;
  uint64 population = 3;
  int64 total_money = 4;                  // raw ×1000
  uint32 audit_ok = 5;                    // 1 = letzte Audit-Prüfung ok
  repeated MarketPrice prices = 6;
  uint64 trips_active = 7;
}
message MarketPrice { uint32 market_id = 1; uint32 good_id = 2; int64 ewma_price = 3; string market_name = 4; }

message BuildingDelta {
  string building_uuid = 1;
  uint32 lifecycle = 2;                   // BuildingLifecycle als Zahl
  uint64 world_tick = 3;
}

message LiveServerMsg {
  repeated CitizenCellFrame cells = 1;
  optional EconomyVitals vitals = 2;      // 1× pro Sekunde an vitals-Subscriber
  repeated BuildingDelta buildings = 3;   // an alle
}
```

- [x] **Step 1:** proto schreiben, `build.rs` erweitern, `buf lint` grün, `scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p abutown-protocol` grün, `npm run generate:proto` erzeugt `src/proto/live_pb.ts`, `npm run typecheck` grün.
- [x] **Step 2: Commit** — `git commit -m "feat(protocol): live.proto — citizen AOI frames, vitals, building deltas"`.

### Task 13: Gateway-Erweiterung + sim-server wird DER Prozess

**Files:**
- Modify: `backend/crates/winterthur-traffic/src/gateway.rs` (WS-Route `/live` neben `/traffic`: eigene Session-Tabelle, CitizenCellFrames 1 Hz — Diff-Muster von CellFrame kopieren; Vitals 1 Hz; Keyframe alle 5 s)
- Modify: `backend/crates/winterthur-traffic/src/shell.rs` (`publish_live`-System nach `publish_snapshot`; Citizen-Positionen: Driving = `vehicle_view` Pos, Walking = Linear-Interpolation from→to über Trip-Dauer, sonst Gebäude-Centroid)
- Modify: `backend/crates/sim-server/src/main.rs`: statt nur axum-Serve → lädt SimWorld+TrafficNet+trips, baut ECS mit `WorldCorePlugin`, Resume via `WorldStore.read` (apply VOR Seed-Guards; Log-Vertrag Task 11), startet `run_loop_with_router(world, schedule, port, card_hand_router)`, Persist-Flush alle 50 Ticks (5 s) via `WorldStore.write` (tokio spawn, nie im Tick blockieren — extract synchron, write async).
- Modify: `fly.toml`/`Dockerfile`: Binary bleibt `sim-server`; `data/winterthur/{trafficnet.json,trips.bin,simworld.json,economy.json}` ins Image kopieren. Env `ABUTOWN_WORLD_ID=winterthur`.

**Interfaces:**
- Consumes: alles Vorherige. Produces: EIN Prozess auf EINEM Port (8080): `/health`, `/cards`, `/card-hand`, `/traffic` (WS), `/live` (WS).

- [x] **Step 1: Failing Rust-Integrationstest** `backend/crates/sim-server/tests/live_ws.rs` (in-memory, ohne PG): Server mit Fixture-Netz + Fixture-SimWorld starten (tokio), WS auf `/live` verbinden, `subscribe_vitals` senden → innerhalb 3 s kommt `LiveServerMsg` mit `vitals.population > 0`.
- [x] **Step 2:** FAIL → implementieren → PASS.
- [x] **Step 3:** Restart-Test (opt-in PG): Server starten, 20 s laufen, töten, neu starten → Boot-Log enthält `resuming world-core from persisted snapshot`, Vitals-world_tick > vorherigem Wert.
- [x] **Step 4:** Volles Backend-Gate: fmt, clippy, `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace`.
- [x] **Step 5: Commit** — `git commit -m "feat(sim-server): one process — traffic + world-core + persistence + card-hand + /live gateway"`.

---

## Phase F — Frontend

### Task 14: `liveClient.ts` — Live-Kanal-Client

**Files:**
- Create: `src/diorama/live/liveClient.ts`
- Test: `tests/live/liveClient.test.ts`

**Interfaces:**
- Produces:
```ts
export interface LiveVitals { worldTick: bigint; sOfWorldDay: number; population: bigint;
  totalMoney: bigint; auditOk: boolean; prices: { marketId: number; goodId: number; ewmaPrice: bigint; marketName: string }[];
  tripsActive: bigint }
export interface LiveCitizen { id: number; x: number; z: number; activity: number }
export function createLiveClient(opts: {
  url: string;
  onVitals: (v: LiveVitals) => void;
  onCitizens: (cell: number, citizens: LiveCitizen[], departed: number[], keyframe: boolean) => void;
}): { updateAoi(cells: number[]): void; close(): void }
```
- Muster: Spiegel von `trafficClient.ts` (gleiche CellGrid-Ableitung aus trafficnet.json wiederverwenden — Funktion daraus exportieren statt kopieren), decode via `src/proto/live_pb.ts`.

- [x] **Step 1: Failing vitest:** Frame-Handling pur testen (decode-Funktion mit handkonstruiertem `LiveServerMsg`-Binary via `@bufbuild/protobuf` create+toBinary): Vitals-Callback bekommt konvertierte Werte (x_dm→Meter /10), keyframe ersetzt Zell-Bestand, delta entfernt departed.
- [x] **Step 2:** FAIL → implementieren → PASS. `npm run typecheck` grün (tsconfig.typecheck.json deckt tests ab — CLAUDE.md-Falle beachten).
- [x] **Step 3: Commit** — `git commit -m "feat(frontend): live channel client (citizens AOI + vitals)"`.

### Task 15: Bürger-Rendering + Vitals-HUD + Attribution + WebGPU-Meldung

**Files:**
- Create: `src/diorama/live/citizensLayer.ts` (InstancedMesh-Kapseln, Muster/Material aus `src/diorama/ksw/agentMeshes.ts` wiederverwenden; Höhe via vorhandenem `makeHeightSampler`; Kapazität 4096, Dead-Reckoning linear 1.4 m/s Richtung letzter Bewegung, Snap bei Frame)
- Create: `src/diorama/live/vitalsHud.ts` (DOM-Overlay-Karte: Weltzeit HH:MM, Population, Geld, „Audit ✓/✗", 4 Marktpreise, aktive Trips; designTokens.ts-Stile)
- Modify: `src/diorama/ksw/main.ts` (Layer + HUD mounten wenn `?live=1` ODER `VITE_LIVE_WS` gesetzt; AOI-Update an Kamera-Throttle des trafficClient anhängen)
- Modify: `index.html`/App-Boot: WebGPU-Check — `if (!navigator.gpu) { zeige zentriertes Overlay „Abutown braucht WebGPU (Chrome/Edge 121+, Safari 26+)…" und stoppe Boot }`
- Modify: Attribution-Footer (`© swisstopo, © OpenStreetMap contributors` aus manifest.attribution) als fixes DOM-Element unten rechts.

- [x] **Step 1:** Implementieren (kein sinnvoller Unit-Test für GPU-Layer; HUD-Formatierung als vitest: `formatWorldClock(sOfWorldDay)` → „07:30", Money-Formatierung ÷1000 mit Tausender-Trennung).
- [x] **Step 2:** `npm run typecheck && npm test && npm run build` grün.
- [x] **Step 3: Browser-Verifikation (Pflicht):** Stack lokal starten (sim-server + vite), `?live=1`, prüfen: Bürger-Kapseln sichtbar in Strassennähe tagsüber, HUD zählt Weltzeit 6× schneller als real, Attribution sichtbar. (Smoke 13/13: WS /live 18 Frames, population=139647, Weltzeit-Rate 6.00×, 414 Kapseln instanziert, Attribution aus manifest, 0 Console-Errors.)
- [x] **Step 4: Commit** — `git commit -m "feat(frontend): citizens layer, vitals HUD, attribution, WebGPU gate"`.

---

## Phase G — Abnahme & Betrieb

### Task 16: `smoke-world.mjs` — 2 Clients + Restart-Resume

**Files:**
- Create: `scripts/smoke-world.mjs` (Muster: `scripts/smoke-traffic.mjs` + `scripts/lib/traffic-stack.mjs`; Stack-Launcher um sim-server-Variante erweitern statt duplizieren)

**Asserts (alle müssen halten):**
1. Client A und B (zwei Playwright-Kontexte) verbinden auf `/live`; beide erhalten Vitals mit identischer `population` und `world_tick`-Differenz < 100.
2. Beide sehen ≥1 CitizenCellFrame mit citizens > 0 (Kamera auf Altstadt).
3. Vitals `audit_ok == 1` durchgehend über 30 s.
4. `s_of_world_day` wächst ~6× Realzeit (Messung über 20 s, Toleranz ±20%).
5. Restart-Teil (nur mit lokalem PG, `ABUTOWN_TEST_DATABASE_URL`): sim-server killen + neu starten (restartTraffic-Muster) → Boot-Log enthält `resuming world-core from persisted snapshot`, Client reconnected, `world_tick` monoton weiter, `population` unverändert.

- [x] **Step 1:** Script schreiben, lokal grün laufen lassen (mit lokalem Postgres).
- [x] **Step 2:** In CI e2e-Job aufnehmen (`.github/workflows/ci.yml`). CI-Entscheid: e2e hat den 77-MB-World-Bake nicht (gitignored, Bake in CI zu schwer) → Smoke läuft dort im `--no-render`-Modus (2 Node-WS-Clients gegen /live+/health, gleiche generierte Proto-Decodes + geteilte CellGrid-Ableitung) mit postgres:16-Service + `ABUTOWN_TEST_DATABASE_URL`, damit auch der Restart-Teil in CI läuft; Browser-Modus bleibt das lokale Gate.
- [x] **Step 3: Commit** — `git commit -m "test: two-client world smoke with restart-resume proof"`.

### Task 17: PR, Deploy-Runbook, Vercel-Envs

- [ ] **Step 1:** `progress.md`-Eintrag (Konvention: neue Einträge oben ab Zeile 19). `deploy/README.md` aktualisieren: sim-server ersetzt traffic-Binary auf Fly; Secrets `DATABASE_URL` (Supabase :5432!), `SUPABASE_URL`, `CORS_ALLOWED_ORIGINS`, `ABUTOWN_WORLD_ID`; Vercel braucht neu `VITE_SUPABASE_URL`, `VITE_SUPABASE_ANON_KEY`, `VITE_LIVE_WS`, `VITE_TRAFFIC_WS`.
- [ ] **Step 2:** Volles lokales Gate (Global Constraints) + `scripts/smoke-world.mjs` + `scripts/smoke-traffic.mjs` (Regressionsschutz) + `scripts/smoke-cardhand.mjs`.
- [ ] **Step 3:** PR gegen `main` öffnen (Titel „MMORPG M1: persistenter Welt-Server"), CI ABWARTEN bis alle Checks PASS (Memory-Regel: nie auf UNSTABLE mergen), mergen, Branch aufräumen.
- [ ] **Step 4:** Deploy NICHT automatisch — dem User melden: bereit für `fly deploy` + Vercel-Env-Setup (Netzwerk-Fragilität + Single-Writer-Kollisionsgefahr sind dokumentierte Betriebsrisiken; User entscheidet Zeitpunkt).

---

## Offene Punkte aus dem Spec → hier entschieden

- Wirtschafts-Taktung: Economy-Kette 1 Hz (jeder 10. Tick), Produktions-/Regen-Intervalle 600 Ticks = 1 Welt-Zehn-Minuten-Äquivalent; Löhne jede Econ-Runde (geerntetes Verhalten).
- Start-Stadtteil: `SeedParams { center: Anker (KSW), radius_m: 2500 }` — Altstadt + Umfeld; Ausweitung = Parameter.
- Admin-Tuning (Spec „Welt-Gärtner"): **auf M1.5 verschoben** — M1 liefert Vitals (HUD + /health erweitert um `world_tick`/`audit_ok`); Runtime-Tuning-Endpoint kommt als erster Folge-Slice, damit M1 nicht wächst.

## Selbstreview-Notizen (erledigt)

- Spec-Abdeckung: Welt lebt+überlebt (T10–13,16), 2-Browser (T16), 4h-Tag (T3,9,16#4), Entity-Lebenszyklus im Datenmodell (T2,T10,BuildingDelta T12), No-Wipe (T10 Migrationskette + Test), Retention: world_core_snapshots ist Upsert (wächst nicht); world_events-Feed kommt in M2 — im Spec M1-Schnitt abgedeckt („Logik später").
- Bewusste M1-Schnitte, im Code zu dokumentieren: Fahrzeuge nicht persistiert (Resume-Policy T10), Märkte nie dormant (T4), Fussgänger ohne Routing (T9).
