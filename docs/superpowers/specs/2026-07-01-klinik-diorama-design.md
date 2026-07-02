# Klinik-Diorama — cozy Clay-Aquarium mit ultrarealistischem Spitalbetrieb (Design)

**Datum:** 2026-07-01
**Status:** Design aus Brainstorming, bereit zur Review
**Zuhause:** abutown-Repo (die geplante „neue Simulation" auf dem gestrippten Fundament, origin/main `7d1ff07`)

## Ziel in einem Satz
Ein **lebendes Diorama** eines Schweizer Spitals (Vorbild Unispital): außen ein wunderschönes, cozy **Clay/Spielzeug-3D**-Miniaturmodell mit Wuselgefühl, innen ein **ultrarealistischer Klinikbetrieb** — es läuft von selbst, wächst von selbst, und man will nicht wegschauen.

## Kern-DNA
**Realismus lebt in der Simulation, Cozy lebt im Rendering.** Die zwei Schichten sind strikt getrennt und widersprechen sich nicht.

## Leitplanken (Nicht-Ziele v1)
- **Keine Karten, keine Spieler-Eingriffe** in v1 — nur Kamera (Zoom/Rotation) + Klick-Inspektion einer Figur/eines Raums. Erst wenn das Aquarium allein fesselt, verdient es Karten. (Karten-Aggregation „Drei-Schichten-Modell" liegt dokumentiert in der Schublade; MMO/Server später — der Kernel ist dafür verpflanzbar gebaut.)
- **Alles prozedural.** Keine Asset-Pipeline, keine gemodelten Meshes: Geometrie, Farben, Animation und Wachstum aus Code.
- **USK-12-diskret:** Diagnosen realistisch benannt, Darstellung mild (Gips, Infusionsständer, Rollstuhl — nie Blut/Leiden/Tod im Bild).
- **Client-only v1** (Browser). Kein Backend-Zwang für die erste Scheibe.
- Kein Audio in v1 (später).

## Recherche-Anker (Pflicht-Task vor dem Kernel-Bau)
„Ultrarealistisch" wird belegt, nicht behauptet (Projektprinzip: Mechaniken literatur-/quellenfundiert, APA7):
- **Triage:** das im Schweizer Notfall übliche 5-Stufen-System (ESI bzw. Manchester — verifizieren, was Unispitäler nutzen) mit Ziel-Wartezeiten je Stufe.
- **Patientenfluss:** Notfall- vs. elektiver Pfad, ambulant vs. stationär; Warteschlangen-/Patientenfluss-Literatur (ED crowding, Bettenmanagement, OP-Programm).
- **Kalibrier-Kennzahlen eines Unispitals** (z. B. USZ): Betten (~900+), Notfallkonsultationen/Jahr (~45k, verifizieren), Mitarbeitende, Schichtzeiten Früh/Spät/Nacht.
- Ergebnis: eine zitierte Referenztabelle in `docs/`, aus der die Sim-Parameter (Ankunftsraten, Behandlungsdauern, Personalschlüssel) abgeleitet werden. Platzhalterzahlen oben sind zu verifizieren.

## Architektur
Zwei Einheiten, harte Grenze, Events dazwischen:

**1 · Sim-Kernel** (`backend/crates/klinik-sim/`) — **Rust, ECS (bevy_ecs)**, kein Render-/Netz-Code im Kern.
- Deterministisch: fixed-timestep `tick()` (~10 Ticks/s Sim-Zeit), seeded RNG, keine Wanduhr im Kern; Determinismus-Hash als Test.
- **v1 als WASM im Browser** (wasm-bindgen): der Renderer liest pro Frame einen flachen **Zero-Copy-Snapshot** (Float32Array aus dem WASM-Speicher: Positionen, Rollen, Zustände) und interpoliert.
- **Später exakt dasselbe Crate serverseitig** (axum, MMO, Events über `/ws` + proto-Toolchain) — ein Kern, kein Port.
- **Crowd-Skalierung:** Flow-Field-/hierarchische Pfadplanung auf dem Wegegraph (HPA*/Flow-Field- und bevy_ecs-Muster aus der abutown-Git-Historie wiederverwenden). Ziel: **1000–10'000 Agenten** tragfähig.
- Modelliert: Agenten, Räume, Pfade, Warteschlangen, Schichten, Tag/Nacht, Wachstum. Vollständig unit-testbar (gleiche Seed → identischer Verlauf).

**2 · Diorama-Renderer** (`src/diorama/`) — three.js, konsumiert nur Sim-Events/State-Snapshots, rendert und interpoliert. Keine Spiellogik.

`src/main.ts` bindet beide. Die bestehende Kartenhand (`src/cardHand/`) bleibt unangetastet daneben.

## Sim-Scope v1 (erste Scheibe: das Notfallzentrum)
- **Bereiche:** Notfallzentrum (Anmeldung/Triage, Kojen, Schockraum diskret), **eine** Bettenstation, Diagnostik (Radiologie + Labor als Engpass-Ressourcen), Korridore/Wege, Eingang + Rettungszufahrt.
- **Patientenpfade (realistisch):** Ankunft zu Fuß oder per Rettungsdienst → Triage (5 Stufen, priorisiert die Warteschlange echt) → Koje/Wartezone → ggf. Diagnostik → Behandlung → Entlassung **oder** stationäre Aufnahme auf die Bettenstation. Elektive Patienten als kleiner Parallelstrom.
- **Rollen:** Pflegefachpersonen, **FaGe**, Assistenz-/Ober-/Kaderärzt:in, MPA/Empfang, Rettungsdienst, Hotellerie/Reinigung, Logistik. Jede Figur ist immer *erkennbar unterwegs zu etwas* (Wusel = viele kleine sichtbare Absichten).
- **Schichtsystem:** Früh/Spät/Nacht mit sichtbarem Schichtwechsel; Tag/Nacht-Zyklus steuert Ankunftsraten (nachts ruhig, Montagmorgen-Peak).
- **Diagnosen:** kleiner Katalog echter, häufiger Fälle (z. B. Fraktur, Appendizitis, Pneumonie, Rissquetschwunde) mit realistischen Stationen ihres Pfads — Darstellung diskret.
- **Autonomes Wachstum:** die Sim misst ihren Druck (Triage-Wartezeiten über Ziel, Bettenauslastung) → beschließt ein Bauprojekt → Bauphase mit Gerüst-Miniszene → neuer Raum/Trakt „ploppt" aus Clay-Bausteinen (Squash-and-Stretch). Wachstumspfad: Notfallzentrum → weitere Stationen/Departemente → Richtung Unispital-Maßstab über Wochen.
- **Skalierungsbudget:** v1 zeigt 20–60 gleichzeitige Agenten (kleines Notfallzentrum ist realistisch nicht voller); Kernel + Renderer sind auf **1000+ Agenten** ausgelegt (ECS/Flow-Fields sim-seitig, Compute/Instancing render-seitig) — der Wachstumspfad Richtung Unispital braucht sie.

## Diorama-Look (das Cozy — SOTA 2026)
- **Renderer-Unterbau:** **WebGPU-first** (three.js WebGPURenderer + TSL-Node-Materials), WebGL2-Fallback mit reduziertem Stack. Compute-Shader tragen die Menge: **GPU-getriebene Agenten** (Bewegung/Prozedur-Animation per Compute, InstancedMesh) — Wusel skaliert in die Hunderte ohne CPU-Last.
- **Licht (der Tiny-Glade-Kern):** kleines Diorama = Idealfall für **Echtzeit-GI** (Radiance-Cascades-artig oder Probe-basiert) — weiches Bounce-Licht, Farb-Bleed von Wänden. Dazu **PCSS-Kontaktschatten** (contact-hardening) und **GTAO**.
- **Clay-Material = SSS:** approximierte **Subsurface Scattering** in TSL (Licht dringt minimal ein) — erst das macht Knete statt bemaltem Plastik. RoundedBox/Kapsel-Geometrie überall, matte Pastell-Palette, dezenter Fresnel-Rim.
- **Post-Stack:** TAA, hochwertiges **Bokeh-Tilt-Shift** (Miniatur!), dezentes HDR-Bloom, hauchfeine Körnung, **AgX-Tonemapping** + kuratierte Farb-LUT pro Tageszeit; **volumetrische Morgensonne** durch die Eingangsfront; sanfter Tag/Nacht-Farbverlauf (warme Fenster bei Nacht).
- Bewusst ausgeschlossen: Gaussian Splatting (SOTA für eingescannte Szenen, nicht für prozedurale Sims).

## Art Direction (verbindlich — Schönheit ist reviewbar, kein Vibecoding)
Tech garantiert keine Schönheit; Disziplin tut es. Drei Mechanismen machen den Look **code-reviewbar**:

**1 · Design-Tokens als Single Source of Truth** (eine Datei, z. B. `src/diorama/designTokens.ts` — Muster: das frühere `designTokens.test.ts` aus der Git-Historie):
- **Palette:** exakt definierte Farbwerte — warme Creme-Basis, 1–2 gedeckte Sekundärtöne (Salbei/Mint), **ein** Akzent (Koralle), Dämmerungs-Blau für Nacht; 60-30-10-Verteilung. **Kein Hexwert außerhalb der Token-Datei** — im Review mechanisch prüfbar (Guard-Test greppt den Renderer).
- **Form-Sprache:** Rundungsradien aus fester Skala (chunky, keine dünnen Stäbe/scharfen Kanten); Bohnen-Proportionen fixiert (Kopf:Körper-Verhältnis); Requisiten aus dem gemeinsamen Baustein-Vokabular.
- **Licht-Rezept pro Tageszeit:** Key/Fill-Verhältnis, Farbtemperaturen, Schattenweichheit als Token-Presets (Morgen/Mittag/Abend/Nacht) — nicht ad hoc im Code gedreht.
- **Material-Bibliothek:** wenige benannte Clay-Materialien (Wand, Boden, Haut, Stoff, Metall-matt) mit festen Roughness-/SSS-Werten; Renderer darf nur aus der Bibliothek schöpfen.
- **Kamera-Contract:** feste Neigung, feste (virtuelle) Brennweite, Zoom-Grenzen — das Diorama hat *einen* Blick, wie eine gebaute Miniatur.

**2 · Referenz-Moodboard, eingecheckt:** kuratierte Referenzen (Tiny Glade, Townscaper, Monument Valley, Aardman-Claymation) + **3 „goldene" Ziel-Screenshots** (Morgen/Nacht/Totale) als verbindliche Messlatte im Repo.

**3 · Look-Gates im Prozess:** der Screenshot-Harness (Muster `capture-visuals`) rendert bei jedem Gate dieselben 3 Kamera-Positionen; Review vergleicht **Seite an Seite gegen die goldenen Screenshots** — bestanden/nicht bestanden wird am Bild entschieden, nicht am Gefühl. Externe Review (z. B. Codex) bekommt Moodboard + Golden Shots als Prüfgrundlage mitgeliefert.
- **Bohnen-Menschen:** Kapseln mit Augenpunkten, Rollen per Farbe/Accessoire (Stethoskop-Torus, Kittel-Farbe), federnder Watschelgang, Squash beim Anhalten — prozedurale Animation, keine Skelette.
- **Cozy-Mikrodetails:** Dampf über Kaffeebechern, Zzz über Schlafenden, Herzchen bei Entlassung, Topfpflanzen, Rettungswagen mit sanftem (stummem) Blinklicht.
- **Kamera:** feste Diorama-Neigung, Zoom + Rotation; Klick auf Figur/Raum → kleine Karte (Name, Rolle/Diagnose diskret, aktuelles Ziel).
- **Performance-Ziel:** 60 fps bei 100+ Agenten auf WebGPU (Budget-Architektur: mehrere Hundert). Gestufter Degraded Mode: WebGL2-Fallback ohne GI/SSS-Approximation, DOF/AO/Bloom einzeln abschaltbar — das Diorama bleibt überall hübsch, nur weniger magisch.

## Fehlerbehandlung / Robustheit
- Kernel wirft nie im Tick: ungültige Zustände werden als `warnings`-Events gemeldet, Sim läuft weiter (Aquarium darf nicht sterben).
- Renderer verliert nie den Kernel-Takt: bei Frame-Druck werden Sim-Ticks gebatcht, Interpolation überbrückt.
- Determinismus-Garantie als Test: N Ticks mit Seed X ⇒ byte-identischer State-Hash.

## Verifikation
1. **Kernel per TDD (Rust):** Triage-Priorisierung, Pfad-Korrektheit (kein Patient verschwindet: Erhaltungs-Invariante), Schichtwechsel, Wachstums-Trigger, Determinismus-Hash — **nativ und als WASM byte-identisch** (Paritäts-Test).
2. **Realismus-Review:** Sim-Kennzahlen (Wartezeiten je Triagestufe, Auslastung) gegen die zitierte Referenztabelle.
3. **Optik:** Browser-Smoke + Screenshot-Harness mit **Look-Gates** (3 feste Kamera-Positionen, Vergleich gegen die goldenen Ziel-Screenshots) + Token-Guard-Test (keine Farben/Materialien außerhalb der Design-Tokens).
4. **Der 5-Minuten-Test (Definition of done):** Diorama 5 Minuten laufen lassen — es wuselt lesbar, wächst nachvollziehbar, und man will nicht wegschauen.

## Später (explizit nicht v1)
Karten-Einfluss (Drei-Schichten-Aggregation), MMO/Server-Verpflanzung des Kernels (bis ~2000 Spieler), Audio/Ambience, Anbindung ans Lernspiel („Grünes Gold" / Swipe-Karten), Atlas-/Poster-Export.
