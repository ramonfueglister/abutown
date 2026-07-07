# Traffic SOTA S2 — Kalibrierung gegen reale Zählstellen

**Kontext:** Spec-Follow-up 3 (2026-07-03-winterthur-traffic-sim-design.md §11).
Nach S1 (heterogene Flotte, PR #147) ist die Demand-Bake literaturfundiert,
aber nie gegen echte Messungen geprüft. Recherche 2026-07-06 (verifizierte
Quellen):

## Datenquellen (verifiziert, offen lizenziert)

1. **Stadt Winterthur MIV-Zähldaten** (primär) — CSV, stündlich, pro Spur +
   Richtung, volle SWISS10-Klassen (pw, lief, lw, sattelzug, …), 2025-03 →
   heute, täglich aktualisiert, Koordinaten WGS84:
   `https://daten.statistik.zh.ch/ogd/daten/ressourcen/KTZH_00003042_00006323.csv`
   (Doku: `…/KTZH_00003042_00006324.txt`). Live-Stationen:
   * K501 Seenerstrasse / Rudolf-Diesel-Strasse (47.4934, 8.7605)
   * K606 Untere Vogelsangstrasse / Breitestrasse (47.4903, 8.7171)
   * K611 Steigstrasse (47.4762, 8.7025)
2. **ASTRA SASVZ Jahresbulletin 2025** (sekundär, Korridor-Niveau) — DTV/DWV +
   SWISS10-Klassenanteile monatlich für A1-Stationen 093 (Umfahrung
   Winterthur, DTV ~78–90k), 639 (Töss), 284 (Kemptthal), 856 (Oberohringen):
   `https://www.astra.admin.ch/dam/de/sd-web/iS7VWufuwjEJ/Bulletin_2025_de.xlsx`
   Stundenwerte nur auf Anfrage (verkehrsdaten@astra.admin.ch) — v1 nutzt
   die Monats-DTV als Niveau-Anker, keine A1-Stundenganglinie.

## Ziel

Simulierte Werktags-Stundenganglinien (Fahrzeuge/h, pro Klasse) an den
gemappten Zählquerschnitten vs. gemessene Di–Do-Mittelwerte; Güte per
**GEH-Statistik** (UK DMRB: GEH < 5 für ≥85 % der Messpunkte = akzeptabel).
Level-Anker A1: simulierter Tagesdurchfluss an 093/639 vs. Bulletin-DWV.

## Tasks

1. **Profil-Extraktor** `scripts/traffic/fetch-count-profiles.mjs`: lädt das
   Stadt-CSV (Cache unter scratch/calibration/), filtert Di–Do (echte
   Werktage, keine Feiertage — Liste aus clock.rs spiegeln), mittelt pro
   Station×Richtung×Stunde×Klassenbucket (pw+pw_plus+mr+bus → car;
   lief+lief_plus+lief_aufl → delivery; lw+lw_plus+sattelzug → truck) und
   schreibt `scratch/calibration/observed-profiles.json`. Vitest-Test auf
   Fixture-CSV.
2. **Stations→Edge-Mapping** `scripts/traffic/map-count-stations.mjs`:
   WGS84 → Weltframe (bestehende Transform aus scripts/geo/lib), nächste
   trafficnet-Edge-Paare (beide Richtungen) je Station; schreibt
   `data/winterthur/count-stations.json` (committet, klein). Achtung
   Lektion: edge-id ≠ lane-id.
3. **Mess-Harness** (Rust, `winterthur-traffic` Beispiel-Bin oder Test
   `--ignored`): headless Lauf über einen vollen Welttag (144k Ticks @ 6×),
   zählt Edge-Querungen pro Weltstunde × Klasse an den gemappten Edges
   (Kernel-Seam: `VehicleView`-Publish oder Despawn/Boundary-Hook), schreibt
   `scratch/calibration/simulated-profiles.json`. Läuft NICHT in CI
   (Laufzeit ~Stunde) — `--ignored` + Doku, wie die Criterion-Benches.
4. **Vergleichs-Report** `scripts/traffic/calibration-report.mjs`: GEH pro
   Station×Stunde×Klasse, Zusammenfassung als Markdown-Tabelle nach
   `docs/superpowers/specs/…-calibration-report.md`; Gate-Vorschlag
   (GEH<5-Quote) NICHT als CI-Gate, sondern als Bericht (Vakuitäts-Lektion
   aus swiss-roads: Metriken zuerst auf Aussagekraft prüfen).
5. **Justierung**: je nach Befund `trips_scale`, A1-Durchgangsvolumen
   (demand-authored.json) und ggf. Klassenanteile nachziehen — jede Änderung
   mit Quelle im Commit begründet, trips.bin rebaken (Hash-Gate).

## Nicht-Ziele

- Kein Echtzeit-Abgleich (Kanton-ZH-API), keine A1-Stundenprofile per
  E-Mail-Anfrage in v1, keine automatische Optimierungsschleife (das wäre
  S4-Territorium).
