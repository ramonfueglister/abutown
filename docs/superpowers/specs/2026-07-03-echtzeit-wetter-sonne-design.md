# Echtzeit-Wetter & echte Sonnenzeiten Winterthur (Design)

**Datum:** 2026-07-03
**Status:** Design aus Brainstorming, bereit zur Review
**Zielbranch:** aufsetzend auf `klinik/look-prototype` (Look-Prototyp mit SkyMesh, Sonnen-Tagesbogen, volumetrischen Wolken, Presets morning/dusk/night)

## Ziel in einem Satz

Das Klinik-Diorama zeigt zu jedem Zeitpunkt die **echte Winterthur-Stimmung**: echter Sonnen- und Mondstand (astronomisch berechnet, strikt Echtzeit) und echtes Live-Wetter (Bewölkung, Niederschlag, Wind, Nebel via Open-Meteo) — ohne den kuratierten Clay-Look aufzugeben.

## Entscheidungen aus dem Brainstorming

| Frage | Entscheid |
|---|---|
| Echtzeit-Härte | **Strikt Echtzeit** — nachts ist das Diorama dunkel (Mondlicht, warme Fenster). Debug-Override nur per URL-Param. |
| Wetter-Scope | **Alle vier Aspekte**: Bewölkung, Niederschlag (Regen/Schnee), Wind, Nebel. |
| Datenquelle | **Open-Meteo** (gratis, kein Key, CORS-fähig, nutzt u. a. MeteoSchweiz-ICON-CH-Modelle). Sonne/Mond brauchen keine API. |
| Nachthimmel | **Echter Mond + Sterne**: Mondposition/-phase astronomisch, prozedurales Sternenfeld mit echter Rotation. |
| Ansatz | **A — Keyframed Art-Direction**: echte Physik steuert, kuratierte Keyframes rendern. |

## Kernprinzip

**Physikalische Wahrheit steuert, kuratierte Schönheit rendert** (Übertragung der Projekt-DNA «Realismus in der Sim, Cozy im Rendering» auf die Umgebung). Die echte Sonnen-Elevation und das echte Wetter sind das *Steuersignal*; gerendert wird durch Interpolation zwischen **art-directed Keyframes** — nicht durch rohe physikalische Werte. Der DREDGE-Dusk-Moment bleibt exakt erhalten, er passiert nur neu zur echten Winterthurer Abenddämmerung.

## Architektur

Neues Modul `src/diorama/environment/`, strikt getrennt in reine Berechnung (unit-testbar, kein three.js) und Anwendung auf die Szene:

### 1 · `solar.ts` — Astronomie

- Sonnen- und Mondposition (Azimut/Elevation) + Mondphase aus UTC-Zeit + Winterthur-Koordinaten (47.499° N, 8.724° O).
- Bibliothek: **`suncalc`** (~3 KB, MIT, Meeus-basierte Formeln), abgesichert durch Golden-Tests gegen bekannte Winterthur-Ephemeriden.
- Zeitzonen sind für die Berechnung irrelevant (läuft auf UTC); «Winterthur-Zeit» ergibt sich aus den Koordinaten. Keine Zeitzonen-Library nötig.

### 2 · `weather.ts` — Open-Meteo-Client

- Abgefragte Felder für Winterthur: `cloud_cover` (gesamt + low/mid/high), `precipitation`, `rain`, `snowfall`, `wind_speed_10m`, `wind_direction_10m`, `visibility`, `weather_code`, `temperature_2m` (stündlich + current).
- Fetch alle **15 Minuten**; zwischen den Stundenwerten wird **linear interpoliert** — kein sichtbarer Sprung beim Refresh.

### 3 · `environment.ts` — der pure Kern

```
computeEnvironment(utcNow: Date, weather: WeatherState): EnvironmentState
```

Pur, deterministisch, vollständig unit-testbar. `EnvironmentState` enthält: Sonnenrichtung/-farbe/-intensität, SkyMesh-Parameter (turbidity/rayleigh/mie), Wolken-Coverage/Drift-Vektor, Fog (color/near/far), Niederschlag (Typ + Intensität 0..1), Mondrichtung + Phasenwinkel + Intensität, Sternen-Sichtbarkeit, Grade-Blend.

### 4 · `applyEnvironment.ts` — Szenen-Anwendung

Pro Frame: EnvironmentState auf Licht-Uniforms, SkyMesh, Wolken-Uniforms, Fog, Grade, Partikelsystem, Sterne/Mond anwenden. **Nur Uniform-Updates, kein Geometrie-Rebuild** → perf-neutral.

## Keyframe-System (das Herzstück)

Die bestehenden Presets werden zu **Keyframes über der Sonnen-Elevation**:

| Keyframe | Elevations-Band | Quelle |
|---|---|---|
| Nacht | < −6° | bestehendes `night`-Preset |
| Dämmerung → Golden Hour (Morgen) | −6°…10°, Sonne aufsteigend | bestehendes `morning`-Preset |
| Dämmerung → Golden Hour (Abend) | −6°…10°, Sonne absteigend | bestehendes `dusk`-Preset (DREDGE-Moment) |
| Tag | > 25° | **neu zu kuratieren** (heller, neutraler, flacherer Kontrast) |

- Interpolation zwischen benachbarten Keyframes ist **stetig** (keine Sprünge, testbar).
- Morgen und Abend bleiben *unterschiedliche* Keyframes, unterschieden per Auf-/Absteigend-Flag der Sonne.
- Alle Keyframe-Werte leben in `designTokens.ts` (Single Source of Truth, Art-Direction-Prinzip aus dem Klinik-Diorama-Spec).

## Wetter → Szene

- **Bewölkung:** `cloud_cover` → Coverage der Raymarch-Wolken (gemappt auf ~0.15–0.85) **und** Diffus-Umschaltung des Lichts: Directional-Intensität runter, Hemisphären-Anteil rauf, Schatten weicher. Bedeckter Tag fühlt sich diffus an, nicht nur «mehr Wolken».
- **Wind:** Geschwindigkeit + Richtung → Wolken-Drift als 2D-Vektor-Uniform (heute feste Achse).
- **Nebel:** `visibility` + Weather-Code → Fog-Verdichtung (near/far). Winterthurer Herbst-Hochnebel wird sichtbar: grauweisse Suppe, gedämpftes Licht.
- **Niederschlag:** neues GPU-Partikelsystem — instanzierte Quads in einer kamerafolgenden Box über dem Diorama, **vertex-animiert** (kein Compute in v1). Regen = gestreckte Fäden mit Fallgeschwindigkeit + Windversatz; Schnee = langsam taumelnde Flocken. Typwahl aus `snowfall` vs. `rain` (Temperatur als Tiebreaker), Dichte skaliert mit Intensität (~2–4k Instanzen max).
- **Bewusst NICHT in v1:** nasse Oberflächen, Pfützen, akkumulierende Schneedecke — eigene spätere Slice.

## Nachthimmel (echt)

- **Mond:** echte Position via suncalc; das bestehende `moonLight` folgt ihr. Mondscheibe als Shader-Sprite mit korrekt beleuchteter **Phase** (Phasenwinkel aus `getMoonIllumination`). Mondlicht-Intensität skaliert mit Phase — Vollmond sichtbar heller als Neumond.
- **Sterne:** prozedurales Punktfeld auf einer Kuppel (hash-gestreut, einige tausend Punkte), rotiert um den echten Himmelspol (Polhöhe = geographische Breite 47.5°, siderische Rate). Einblenden ab Sonnen-Elevation < −6°, gedimmt durch Bewölkung.
- `nightGlow` (warme Fenster) bleibt wie gehabt.

## Zeit-Treiber & Debug-Overrides

- Standard: echte Uhr, pro Frame ausgewertet.
- `?at=2026-07-03T06:30` — Zeit einfrieren/setzen (Entwicklung, Screenshots, Visual-Harness).
- `?wx=clear|overcast|rain|snow|fog` — Wetter übersteuern (kuratierte Testzustände statt Live-Daten).
- Die bisherigen `?preset=`-Parameter werden durch diese beiden ersetzt (die Presets leben als Keyframes weiter).

## Fehlerbehandlung

Kein Fallback-Cruft, aber eine bewusste Offline-Policy (Wetter ändert sich langsam):

- Fetch-Fehler → **letzter bekannter Zustand gilt weiter** + Retry mit Backoff.
- Letzte erfolgreiche Antwort wird in `localStorage` gecacht → Reload ohne Netz startet nicht kahl.
- Waren *noch nie* Daten da: dokumentierter Klarhimmel-Default + Konsolen-Warnung.
- Sonne/Mond/Sterne sind API-unabhängig und laufen immer.

## Testing

- **Unit (vitest):**
  - `solar.ts` golden gegen bekannte Winterthur-Ephemeriden (Sonnenauf-/-untergang an den Solstitien, Mittagselevation, Mondphase an fixen Daten; Referenzwerte bei Implementation aus NOAA/ephemeriden-Quelle verifizieren).
  - `computeEnvironment`: Stetigkeit über Keyframe-Grenzen (keine Sprünge), monotone Blends, Morgen-≠-Abend-Unterscheidung.
  - Open-Meteo-Parsing gegen ein echtes Response-Fixture; Wetter-Mappings als reine Funktionen.
- **Browser-Smoke (Pflicht per CLAUDE.md — Feature kreuzt die Wire):** headless Chromium; prüft, dass der echte Open-Meteo-Fetch rausgeht und parst; probt via `?at=`/`?wx=` mehrere Zustände (06:00, Mittag, Dusk, 23:00, Regen, Schnee, Hochnebel) auf erwartete Uniform-Werte.
- **Look-Review:** capture-visuals-Harness rendert dieselbe Zustandsmatrix als Screenshots — Schönheit bleibt reviewbar.

## Performance

Pro Frame nur Uniform-Updates + eine Astronomie-Berechnung (Mikrosekunden). Partikel ~2–4k Instanzen, Sternenfeld statische Geometrie mit Rotationsmatrix. Kein Risiko fürs 100–120-fps-Budget der Perf-Pipeline (#111).

## Nicht-Ziele v1

- Keine nassen Oberflächen / Schneedecke (spätere Slice).
- Keine Wettervorhersage-UI, keine Anzeige von Messwerten.
- Keine Kopplung ans Sim-Schichtsystem (die Sim hat ihren eigenen Tag/Nacht-Zyklus für Ankunftsraten; Kopplung ist eine spätere Design-Entscheidung).
- Kein MeteoSchweiz-Direktanschluss.
