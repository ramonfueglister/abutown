# Traffic SOTA S1 — Heterogene Flotte (vehicle_class end-to-end)

**Kontext:** SOTA-Audit 2026-07-06. Der Kern (IDM + MOBIL + Junction-Modell +
CH-Routing mit MSA-Live-Gewichten + Zensus-Gravity-Demand) ist SOTA-Praxis;
die grösste sichtbare Lücke ist die homogene Flotte: `trips.bin` reserviert
`vehicle_class u8`, aber demand-gen schreibt immer 0, der Kernel fährt eine
globale `IdmParams` und `VEHICLE_LEN = 4.5` für alle.

**Ziel:** Drei Fahrzeugklassen — 0 = PW, 1 = Lieferwagen, 2 = LKW — von der
Demand-Bake bis zur Silhouette im Browser, deterministisch, literaturfundiert.

## Nicht-Ziele

- Busse/ÖV (eigener Slice, braucht Linien/Haltestellen — User-Scoping).
- Velo/Fussgänger-Interaktion (Spec-Non-Goal v1).
- trips.bin-Formatänderung: Record-Layout bleibt 14 B, nur Byte 13 wird belebt.

## Klassenparameter (Quellen im Spec-§12-Stil, APA 7 ergänzen)

| Klasse | Anteil intern / A1-Durchgang | v0 | T | a | b | s0 | Länge |
|---|---|---|---|---|---|---|---|
| 0 PW | 89 % / 86 % | 13.9 | 1.5 | 1.4 | 2.0 | 2.0 | 4.5 m |
| 1 Lieferwagen | 9 % / 4 % | 13.9 | 1.6 | 1.1 | 2.0 | 2.5 | 6.5 m |
| 2 LKW | 2 % / 10 % | 12.5 | 1.7 | 0.7 | 2.0 | 3.0 | 12.0 m |

PW-Zeile = bestehende Treiber-et-al.-2000-Defaults. LKW-Parameter nach
Treiber & Kesting (2013), Kap. 11 (Lkw-Kalibrierung). Anteile nach
BFS Gütertransportstatistik / ASTRA SASVZ Fahrzeugkategorien (urban vs.
Transit-Autobahn); exakte Zahlen beim Implementieren zitieren und als
benannte Konstanten mit Quelle ablegen.

## Tasks (TDD, cargo via scripts/cargo-serial.sh, ein Rust-Agent aufs Mal)

1. **traffic-core: Klassen im Kernel.** `Fleet` bekommt `class: Vec<u8>`;
   `Core` hält `params: [IdmParams; N_CLASSES]` + `len: [f32; N_CLASSES]`
   statt globaler Werte; IDM/MOBIL/Junction lesen über den Klassenindex.
   Tests: Ring-Test mit gemischter Flotte bleibt deterministisch
   (state_hash threadcount-invariant), LKW hält grösseren Gleichgewichtsabstand.
2. **demand-gen: Klassenvergabe.** Pro Trip deterministisch via
   `u01(seed, index, CLASS_SALT)` gegen die Anteilstabelle des Segments
   (intern vs. SEGMENT_THROUGH). Writer-Doc + Tests (Anteile ±1 % auf 270k
   Trips, Byte-Stabilität des Goldens aktualisieren).
3. **winterthur-traffic: Durchreichen.** Loader validiert class < 3 (hart,
   kein Healing); Spawner übernimmt `TripRecord.vehicle_class` statt 0;
   `Core`-Spawn-API nimmt die Klasse. trips.bin neu baken (net_hash
   unverändert — Netz untouched; Hash-Gate beachten).
4. **Wire: class im VehicleState.** Proto-Feld `uint32 class` ergänzen
   (buf generate), Gateway schreibt es, trafficClient.ts liest es.
   Achtung Lektion #123: edge-id ≠ lane-id über den Wire — Feld nur
   additiv, keine Semantikänderung bestehender Felder.
5. **Frontend: Klasse → Silhouette.** carModels: Klasse 1 wählt van/pickup-
   Varianten, Klasse 2 bekommt einen neuen ~12-m-Truck-Loft (Kabine + Koffer,
   6 Räder) im CS-Stil; Klasse 0 wählt aus den 6 bestehenden Varianten.
   Farb-/Varianten-Hash bleibt id-stabil. wheelSpin: Radradius pro Variante.
6. **Smoke + Gate.** smoke-traffic.mjs erweitern: Klassenverteilung in den
   empfangenen Frames > 0 für Klasse 1 und 2; Screenshot-Capture
   (capture-traffic.mjs) mit sichtbarem LKW. Volles CI-Gate vor Push
   (Rust fmt/clippy/test + Frontend typecheck/vitest/build + e2e).

## Risiken

- trips.bin-Regeneration: Hash-Gate (memory: trips.bin-Hash-Gate) — die
  Bake muss über das Script laufen, nie von Hand.
- Kernel-Signaturänderung `Core::new`/Spawn: alle Testharnesse anfassen.
- Wire-Änderung ⇒ CLAUDE.md-Pflicht: echter Browser-Smoke, nicht nur Tests.

## Roadmap danach (Tasks #2–#5 in der Session-Task-Liste)

S2 Kalibrierung gegen Zählstellen → S3 actuated Signals → S4 Day-to-day-
Replanning (MATSim) → S5 wgpu-Port. ÖV/Fussgänger/Parken: Spec-Non-Goals,
braucht User-Entscheid.
