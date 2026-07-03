# S3: Das echte KSW — reale Hülle, lebendes Innenleben, Dollhouse-Cutaway

**Datum:** 2026-07-03 · **Branch:** `geo/s3-real-ksw` (Basis main@838fb44) · Folge-Spec zu `2026-07-02-winterthur-geodata-design.md` (Variante A, user-approved) und dem Style-/Debug-Stand.

## Ziel

Das stilisierte 60×38-Hero-Hospital wird durch den **echten KSW-Komplex** ersetzt
(26 gebakte Campus-Bauten; Hauptstruktur 17 936 m², 70 m, 113-Punkte-Footprint).
Das handgebaute Innenleben (Korridore, OP/Notaufnahme/…, Agenten) zieht ins
**Erdgeschoss der echten Hülle** um. Einblick per **Dollhouse-Cutaway**
(user-approved): beim Reinzoomen faden die oberen Stockwerke aus, übrig bleibt
das lebende Erdgeschoss im echten Umriss.

**Harte Regeln:** Geodäsie (Hülle = gebakte swisstopo-Geometrie, unverändert);
Look-Vokabular unverändert (Clay-Tokens, Materialrezepte); Stadt-Slice bleibt
unangetastet; Screenshot-Gate je Task (inkl. night); Browser-Smoke Pflicht.

## Etappen (je eigener Plan-Block, strikt in Reihenfolge)

### S3a — Reale Hülle steht

- `kswBuildings` (bisher vom Rendering ausgeschlossen) rendern über die
  **bestehende City-Pipeline** (Massing + Shader-Fassaden + Sockel/Trauf), als
  eigene Gruppe `kswCampus` (nicht in `cityRoot`-LOD-Fernring — Hero-Zone ist
  immer „near": Fassaden-Detail an, volle Bäume).
- Das stilisierte Hospital (buildHospital-Shell + Plaza-Platte 72×56) wird
  **entfernt**; die Agenten-/Nav-/Props-Module bleiben im Code (S3b nutzt sie).
  Übergangszustand nach S3a: Kamera-Presets zeigen den echten Campus, Agenten
  deaktiviert (Flag), Smoke-Erwartungen angepasst (dokumentierter Zwischenstand
  auf dem Branch — gemergt wird erst nach S3d).
- Kamera: `overview`-Preset neu gerahmt auf den echten Komplex; roofFade
  vorerst AUS für den Campus (kommt als Cutaway in S3c).

### S3b — Innenraum-Generator (Zonen-Ladder)

- **Zonen-Zerlegung** des Erdgeschoss-Footprints der Hauptstruktur:
  rektilineare Zerlegung des 113-Punkte-Polygons in ≤ 8 achsparallele
  Rechteck-Zonen (größte-Rechteck-Heuristik, deterministisch, getestet;
  Restflächen < 6 m Breite werden Korridor-/Nebenraumstreifen).
- Pro Zone der **bewährte Ladder-Grundriss** (2 Korridore + Raumzeilen +
  Endverbinder), skaliert auf Zonenmaße; Raum-Vokabular/Props/Rollen aus dem
  bestehenden authored `floorPlan.ts`-Katalog (Abteilungs-Zuordnung pro Zone:
  Notaufnahme an der Zone mit der echten Tür zur Strasse, OP zentral, etc. —
  authored Mapping-Tabelle, kein Zufall).
- Zonen verbunden durch Querkorridore an angrenzenden Kanten; **ein**
  Nav-Graph über alle Zonen (nav.ts wird um Zonen-Verbinder erweitert — die
  Ladder-Logik pro Zone bleibt).
- Innenwände/Böden/Props über die bestehenden Builder (`building.ts`-Vokabular)
  innerhalb der echten Aussenhülle (Innenwand-Offset 0.45 m zur echten Wand).
- Tests: Zerlegungs-Invarianten (Abdeckung ≥ 85 % der Polygonfläche, keine
  Überlappung, alle Zonen ≥ 6×6 m), Plan-Invarianten wie
  `tests/diorama/floorPlan.test.ts`, Nav-Erreichbarkeit (jeder Raum ↔ jede
  Zone ↔ Aussentür).

### S3c — Dollhouse-Cutaway

- Höhenschnitt statt Dachfade: Uniform `cutH` im Campus-Fassaden-/Wand-Material
  (TSL): Fragmente mit `worldY > cutH` werden discarded + Schnittkante als
  heller Trim-Saum (2. Uniform für Kantenband); Dächer/obere Stockwerke faden
  als Ganzes (Opacity) bevor der Schnitt greift — Ablauf beim Reinzoomen:
  radius < roofFadeNear → Dach+Obergeschosse faden (wie heute), dann
  `cutH → 3.2 m` (Erdgeschoss + Schnittsaum). Rauszoomen umgekehrt.
- GI-/Shadow-Refresh-Politik wie roofFadePolicy (markDirty an den Schwellen);
  Agenten-/Innen-Meshes rendern nur bei aktivem Cutaway (LOD-artig).
- Hero-Guard-Analogie: Presets `er`/`ops` zielen auf konkrete Zonen
  (Ziel + radius so, dass Cutaway aktiv).

### S3d — Agenten & Aussenraum

- Spawn/Walker aus dem generierten Plan (S3b) statt `kswPlan`; Rollenverteilung
  wie heute; Aussen-Plaza minimal neu: Vorplatz an der **echten Haupttür**
  (Bake-`door` der Hauptstruktur), Ambulanz an der Notaufnahme-Zone,
  Original-Props (Bänke/Lampen/Bäume) entlang der echten Campus-Wege
  (OSM-Fusswege im KSW-Radius). Helipad: auf dem echten Flachdach-Teil der
  Hauptstruktur (real existiert der KSW-Dachlandeplatz).
- Smoke-ksw wieder voll grün (Agenten-Checks reaktiviert, an neue Geometrie
  angepasst); Voll-Gate + alle Captures (morning/night, alle Presets).

## Risiken / bewusste Entscheidungen

- Das Original-Diorama-Innenleben ist **authored Vokabular in generierter
  Anordnung** — Raumformen bleiben plausibel, nicht raumgenau real (KSW-interne
  Pläne sind nicht öffentlich; verifiziert in der Basis-Spec).
- Merge-Gate: der Branch wird erst nach S3d gemergt (S3a/b/c sind sichtbare
  Zwischenzustände).
- Die 25 Nebenbauten bleiben geschlossene Shader-Fassaden-Hüllen (kein
  Innenleben) — Fokus aufs Hauptgebäude.
