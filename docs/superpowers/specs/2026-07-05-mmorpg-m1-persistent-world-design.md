# MMORPG Milestone 1 — Persistenter Welt-Server (Design)

Datum: 2026-07-05
Status: Entwurf, vom User im Brainstorming abschnittsweise freigegeben

## Vision (Gesamtbild, über M1 hinaus)

Abutown wird ein persistentes MMORPG: ein autonomes, lebendiges Winterthur auf
echtem swisstopo-Terrain, das kontinuierlich simuliert wird und dessen Zustand
dauerhaft überlebt. Spieler sind **Stadt-Götter/Verwalter** (Cities-Skylines-
Kamera bleibt die primäre Sicht), keine verkörperten Avatare. Die Simulation
läuft **vollständig autonom**; Spieler beeinflussen sie ausschliesslich
**indirekt über Karten** (Card-Hand = der geplante Einfluss-Mechanismus).
Kern-Inhalte: **lebende Wirtschaft** + **Karten-Events**. Die Stadt soll später
wachsen können (SOTA LOD-Streaming, 3D-Tiles-Klasse) — die Architektur darf
kein fixes Raster einbacken.

Meilenstein-Folge (jeder bekommt eigenen Spec → Plan → Implementierung):

- **M1 (dieser Spec):** Persistenter Welt-Server — die Welt LEBT und überlebt
  Neustarts. Bürger + Wirtschaft auf echten Gebäuden/Strassen, Persistenz in
  Supabase, Multiplayer-Zuschauen über WS. Noch keine Karten-Wirkung.
- **M2:** Karten-Wirkung E2E (Login → Karte → Server-Validierung → Sim-Effekt
  → alle Clients) + Welt-Event-Feed/Attribution (Sozial-Gefühl).
- **M3:** Netz-LOD-Streaming der Kachel-Pyramide (CDN, Priorisierung, Cache),
  Stadt-Wachstum.
- **M4+:** Neubau-/Abriss-Logik der Sim (Parzellen, Footprint-Generator),
  weitere Wirtschaftstiefe.

## Im Brainstorming getroffene Entscheidungen

1. Spielerrolle: Stadt-Gott/Verwalter (kein Avatar-Netcode).
2. Welt-Autorität: Rust-Sim-Server reaktivieren — Fly.io, 1 Maschine,
   Single-Writer. Supabase = Auth + Persistenz, Vercel = statisches Frontend.
3. Ansatz B: **neue Welt-Engine auf dem aktuellen Winterthur-Stack**
   (Meter-Welt, Graph, AOI), alte Domänen-Logik aus der Git-History
   **gezielt geerntet** (geometrie-freie Teile), kein Rückport des alten
   Tile-Chunk-Kerns.
4. Weltmodell: **Entitäten als Wahrheit, Kacheln als Index** (unten).
5. Zeitmodell: **Hybrid à la BitCraft** — 4h-Weltentag (unten).
6. Anonyme Besucher dürfen zuschauen (Spectator ohne Login); Login nur fürs
   Kartenspielen (M2).
7. Kein Welt-Wipe mehr: Snapshot-Versionierung + Migration ab Tag 1.

## Architektur

```
Vercel (statisches Frontend)          Fly.io (1 Maschine, Single-Writer)
┌─────────────────────────┐   wss    ┌──────────────────────────────────┐
│ Winterthur-App (WebGPU) │◄────────►│ sim-server (axum)                │
│  + Card-Hand (Supabase) │  proto   │  ├─ winterthur-traffic (besteht) │
└─────────────────────────┘          │  ├─ world-core (NEU)             │
         │ Auth (JWT)                │  │   Bürger + Wirtschaft auf     │
         ▼                           │  │   echtem Graph/Gebäuden       │
┌─────────────────────────┐          │  └─ persist (Snapshots+Events)   │
│ Supabase                │◄─────────┘
│  Auth + Postgres :5432  │  sqlx — NUR Session-Pooler :5432, NIE :6543
└─────────────────────────┘
```

- **Neue Crate `backend/crates/world-core`** im bestehenden Workspace,
  bewusst getrennt von `traffic-core`. Enthält Weltmodell, Bürger, Wirtschaft,
  Weltuhr.
- **`sim-server`** bleibt der eine Prozess: tickt Verkehr + Welt, bedient
  `/ws` (binäres proto), `/health`, validiert Supabase-JWTs (Card-Hand-Code
  existiert). Pro Verbindung eine AOI — Multiplayer-Zuschauen ist in M1
  inklusive.
- Frontend bleibt auf Vercel (Build lokal → statisches `dist/`);
  Live-Build braucht die bisher fehlenden `VITE_`-Supabase-Vars.

## Weltmodell: Entitäten als Wahrheit, Kacheln als Index

Winterthur ist eine Meter-Welt (echte Footprints, Strassengraph aus
`traffic-net`) — kein Tile-Raster als Wahrheit. Stattdessen:

- **Wahrheit = Entitäten** mit stabilen IDs und sim-eigenem Zustand:
  Gebäude, Firmen, Bürger, Graph-Kanten. Die Sim darf sie mutieren und
  erschaffen.
- **Gebäude-Lebenszyklus ab M1 im Datenmodell** (Logik teils erst später):
  `Bewohnt/Genutzt → Leerstehend (z.B. Firma bankrott) → Verfallend →
  Abgerissen`, plus `Im Bau → Fertig` für spätere Neubauten. Zustandswechsel
  sind WorldEvents (unten) und werden an alle Clients gefunkt; der Renderer
  reagiert (dunkle Fenster, Verfall, Entfernen aus dem BatchedMesh).
  Neubauten rendern über denselben prozeduralen Footprint→Massing-Pfad wie
  gebackene Gebäude.
- **Kacheln = abgeleitete Indizes**: die bestehende Render-Kachel-Pyramide
  und AOI-Rechtecke bleiben die räumliche Partitionierung für Streaming,
  Interest Management und Dirty-Tracking. Stadt wächst = mehr Kacheln +
  mehr Graph, kein Rasterumbau.

## Simulation

- **Bürger:** persistente Agenten mit Zuhause/Arbeitsplatz in echten
  Gebäuden (Nutzungs-Ableitung aus Landuse + Census, die `demand-gen`
  bereits lädt). Tagesrhythmus an der Weltuhr: Heim → Arbeit → Markt → Heim.
  Die Verkehrs-Demand wird aus diesen Bürgerwegen gespeist statt aus rein
  statistischem Sampling — die Autos bekommen einen Grund.
- **Wirtschaft (aus der Git-History geerntet, geometrie-frei):** Güter,
  Auktions-Preisbildung, Firmen mit Produktionsrezepten, Löhne, und das
  **SFC-Konservierungs-Audit** (Geld byte-genau konserviert, Verstoss =
  fail-fast) als Sicherheitsnetz. Märkte/Firmen sitzen an echten Adressen.
  Wirtschaftsmechanismen bleiben literatur-fundiert (APA7-Präzedenz des
  Projekts).
- **Takte:** Verkehr behält seinen schnellen Takt; Bürger/Wirtschaft laufen
  gröber (Sekunden-Takt, Tages-Ereignisse an der Weltuhr). Tick-Budget wird
  mit dem bewährten Profiling-Harness-Muster überwacht.
- **Start-Umfang:** ein Stadtteil-Ausschnitt zuerst (Kalibrierungs-Risiko
  der Gebäude-Nutzung begrenzen), dann auf die Gemeinde ausweiten.

## Zeitmodell: 4h-Weltentag (Hybrid à la BitCraft)

- **Weltentag = 4 echte Stunden** (6× Zeitraffer): Sonne/Mond/Sterne laufen
  auf der Weltuhr — jede Session sieht Morgenrot und Nacht.
- **Datum/Saison = real:** der Kalender bleibt der echte (keine 6×-Drift der
  Jahreszeiten). Pro echtem Tag vergehen 6 Weltentage desselben Datums.
- **Wetter bleibt echt (und wird dadurch natürlich „komprimiert"):** die
  Live-Wetter-Kopplung an das echte Winterthur bleibt bestehen; innerhalb
  eines 4h-Weltentags erlebt man die echte Wetterentwicklung dieser 4
  Stunden. Der Zwillings-Charakter (echtes Wetter über echter Stadt) bleibt
  erhalten, nur der Tag/Nacht-Zyklus ist Spielzeit.
- **Wirtschaftsprozesse** takten in Weltentagen (Produktion, Löhne,
  Tagesrhythmen) — Welt-Fortschritt im „mehrmals täglich reinschauen"-Tempo.
- Es gilt weiter das **Frozen-Time-Modell**: Server läuft = Weltzeit läuft;
  Server down = Welt friert ein; Resume vom letzten Snapshot, kein
  Offline-Nachholen.

## Persistenz (Supabase Postgres, dreischichtig)

1. **Welt-Basis (unveränderlich):** gebackene Artefakte (Terrain, Graph,
   Original-Gebäude), versioniert als Dateien/Storage, nicht in Tabellen.
2. **Welt-Abweichungen (append-only `world_events`):** jede bauliche
   Veränderung durch die Sim (Leerstand, Verfall, Abriss, Neubau) ist ein
   WorldEvent. Aktuelle Stadt = Basis + Events; die Welt hat eine
   *Geschichte*. Bauliche Events werden **nie** gepruned (Stadtgeschichte);
   neue Clients laden Basis + kompakten Abweichungs-Snapshot.
3. **Sim-Zustand (Snapshots, ~5s-Zyklus):** Bürger, Firmen/Konten/Preise,
   Weltuhr-Tick als versionierte Snapshots. Resume beim Boot; SFC-Audit
   verifiziert nach jedem Resume die Geld-Konservierung. Resume-Beweis ist
   die Boot-Log-Zeile, nicht der DB-Tick.

**Harte Prinzipien:**

- **Kein Welt-Wipe, je.** Snapshots tragen eine Schema-Version; jede
  Schema-Änderung liefert eine Migration (v_n lesen → v_n+1 schreiben) mit
  Test. Das alte `DELETE FROM …_snapshots`-Ritual ist verboten.
- **Retention/Kompaktierung:** hochfrequente Sim-Events werden periodisch
  zusammengefasst/gepruned (Lehre: 200k-Prune-Loop der alten
  `economy_events`); bauliche WorldEvents sind davon ausgenommen.
- **Backups:** Supabase-Backups/PITR aktivieren, sobald die Welt Geschichte
  angesammelt hat (spätestens beim öffentlichen Launch).
- Verbindung ausschliesslich über `:5432` (Session-Pooler); `:6543`
  (Transaction-Pooler) crasht sqlx.
- Single-Writer: Fly `count=1`, niemals skalieren.
- Gesundheits-/Validitäts-Guards dürfen nie an statischen Seed-Zahlen
  hängen (Population ist dynamisch).

## Wire-Protokoll (Erweiterung des bestehenden proto/buf)

Neue Messages neben dem bestehenden Verkehrs-AOI-Protokoll:

- `CitizenDelta` — Bürger innerhalb der AOI, interpoliert wie heute die
  Fahrzeuge.
- `WorldDelta` — Gebäude-Zustandsänderungen (an alle Verbindungen).
- `EconomyVitals` — Kennzahlen fürs HUD (Population, Geld, Preise, Routing),
  niederfrequent.
- `WorldEventFeed` — menschenlesbare Ereignisse („Schreinerei Müller ist
  bankrott") — Basis für den M2-Feed.

Alles binär (protobuf über WS), versioniert, `buf breaking` bewacht die
Kompatibilität. WebSocket bleibt bewusst der Transport (Spectator+Karten ist
nicht latenz-kritisch); ein späterer WebTransport-Swap bleibt eine isolierte
Transportschicht-Änderung.

## Welt-Gärtner-Werkzeuge (M1 klein, wächst mit)

Eine autonome Live-Wirtschaft driftet. Ab M1:

- **Vitals-Dashboard** im Frontend (Population, Geld, Preise, Audit-Status —
  HUD-Muster existierte im alten Stack).
- **Runtime-Tuning:** zentrale Sim-Parameter zur Laufzeit änderbar
  (autorisierter Admin-Endpoint), ohne Neustart/Redeploy.
- **Alarme:** SFC-Audit-Verstoss, Persistenz-Stau, Tick-Budget-Überschreitung
  → sichtbar in `/health` + Log; Eskalation (Notification) folgt später.

## Sichtbares Ergebnis von M1 (Abnahme)

1. Zwei Browser öffnen die Live-URL → beide sehen dasselbe Winterthur:
   Bürger mit echten Wohnungen/Arbeitsplätzen leben ihren Weltentag, der
   Verkehr entsteht aus ihren Wegen, Firmen produzieren, Preise bilden sich
   (Vitals-HUD).
2. Server-Neustart → Welt macht exakt dort weiter (Boot-Log-Beweis).
3. Anonymes Zuschauen funktioniert ohne Login.
4. 4h-Weltentag sichtbar (Sonnenlauf), echtes Wetter weiterhin live.
5. Geräte ohne WebGPU erhalten eine saubere „nicht unterstützt"-Meldung.
6. Geodaten-Attribution (swisstopo/OSM) ist im UI sichtbar.

Explizit NICHT in M1: Karten-Wirkung, Event-Feed-UI, Chat, Neubau-/Abriss-
*Logik* (nur Datenmodell), Netz-LOD-Streaming.

## Tests

- Rust: Unit/Integration inkl. **Resume-Regressionstest auf dem echten
  Hydrate-Pfad** und SFC-Audit; **Snapshot-Migrations-Test** (v_n → v_n+1
  ohne Datenverlust). Cargo immer über `scripts/cargo-serial.sh`.
- Frontend: vitest + typecheck wie gehabt.
- **Browser-Smoke mit zwei parallelen Clients** gegen den echten Stack ist
  Pflicht vor „fertig" (Phase-7a-Lehre: grüne Tests beweisen die
  Frontend↔Backend-Naht nicht). Smoke prüft: beide Clients sehen dieselben
  Bürger/Vitals; Restart-Resume.
- CI: bestehendes Gate (fmt/clippy/test + Frontend + e2e) erweitert um die
  neuen Smokes.

## Risiken

- **(a) Gebäude-Nutzungs-Kalibrierung:** falsch abgeleitete Wohn-/
  Arbeitsnutzung wirkt tot oder absurd → Start mit Stadtteil-Ausschnitt,
  Dichte-Proben gegen Census.
- **(b) Tick-Budget:** Wirtschaft + Verkehr + Persistenz auf 1 Fly-vCPU →
  Profiling-Harness von Anfang an, Lehren aus der Tick-Starvation-Outage
  (yield, MissedTickBehavior::Delay) übernehmen.
- **(c) Scope-Sog:** M1 endet hart vor Karten-Wirkung und Neubau-Logik.
- **(d) Schema-Disziplin:** Migrations-Pflicht kostet pro Slice etwas Zeit —
  sie ist der Preis dafür, die Welt nie wieder zu wipen.

## Offene Fragen (vor Implementierungs-Feinschliff)

- Exakte Wirtschafts-Taktung in Weltentagen (Lohnzyklus, Produktionsdauern)
  — im Implementierungsplan festlegen.
- Umfang des Start-Stadtteils (welcher Ausschnitt Winterthurs).
- Admin-Autorisierung fürs Runtime-Tuning (Supabase-Rolle vs. statischer
  Token) — im Plan entscheiden.
