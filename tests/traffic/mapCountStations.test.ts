// tests/traffic/mapCountStations.test.ts
//
// S2 calibration Task 2: station→edge mapping. Synthetic cross of two
// streets pins the load-bearing rules: alignment beats proximity at a
// junction, antiparallel edge pairs resolve to the matching travel
// direction, and unmappable inputs fail LOUD (no silent guess — a wrong
// cross-section poisons the calibration).

import { describe, expect, it } from 'vitest';
import { ANCHOR, makeProjector } from '../../scripts/geo/lib/project.mjs';
import { DESTINATIONS, destinationOf, mapStations } from '../../scripts/traffic/map-count-stations.mjs';

// Synthetic net around a junction placed AT a real-ish station coordinate:
// an E-W street (edge pair 1/2) and a N-S street (edge 3), crossing at the
// station. World frame: +x east, +z south.
const proj = makeProjector(ANCHOR);
const ST = { lon: 8.7171, lat: 47.4903 };
const [SX, SZ] = proj.toLocal(ST.lon, ST.lat);

function lane(id: number, edge: number, pts: [number, number][]) {
  return { id, edge, index: 0, lengthM: 100, pts };
}

const NET = {
  edges: [
    { id: 1, from: 0, to: 1, lanes: [11] }, // eastbound
    { id: 2, from: 1, to: 0, lanes: [12] }, // westbound
    { id: 3, from: 2, to: 3, lanes: [13] }, // northbound (toward -z)
  ],
  lanes: [
    lane(11, 1, [
      [SX - 100, SZ + 3],
      [SX + 100, SZ + 3],
    ]),
    lane(12, 2, [
      [SX + 100, SZ - 3],
      [SX - 100, SZ - 3],
    ]),
    // N-S street passes CLOSER to the station than the E-W lanes (1 m vs 3 m)
    // so a proximity-first pick would grab it wrongly for E/W directions.
    lane(13, 3, [
      [SX + 1, SZ + 100],
      [SX + 1, SZ - 100],
    ]),
  ],
};

function station(richtungName: string) {
  return {
    anlageName: 'K999 MIV Teststrasse',
    richtung: 'X',
    richtungName,
    lat: ST.lat,
    lon: ST.lon,
  };
}

describe('mapStations', () => {
  it('resolves antiparallel pair + crossing street by destination bearing', () => {
    // Seen is ~ESE of the station, A1-Töss ~WNW, Zentrum ~NNE (real
    // geography via DESTINATIONS) — so eastbound→Seen picks edge 1,
    // westbound→A1 picks edge 2, northbound→Zentrum picks edge 3 even
    // though lane 13 is nearest to the station for all three.
    const mapped = mapStations(
      {
        stations: [
          station('Richtung Seen'),
          station('Richtung Autobahnanschluss A1'),
          station('Richtung Winterthur Zentrum'),
        ],
      },
      NET,
    );
    const byDir = Object.fromEntries(mapped.stations.map((s) => [s.richtungName, s.edge]));
    expect(byDir['Richtung Seen']).toBe(1);
    expect(byDir['Richtung Autobahnanschluss A1']).toBe(2);
    expect(byDir['Richtung Winterthur Zentrum']).toBe(3);
  });

  it('fails loud on unknown destinations and unmappable stations', () => {
    expect(() => destinationOf('Richtung Nirgendwo')).toThrow(/no authored destination/);
    // Station far from any lane → no candidate in radius.
    const far = { ...station('Richtung Seen'), lat: ST.lat + 0.02 };
    expect(() => mapStations({ stations: [far] }, NET)).toThrow(/no lane within/);
  });

  it('authored destinations cover the live Winterthur feed', () => {
    for (const name of [
      'Seen',
      'Oberwinterthur',
      'Grüze',
      'Winterthur Zentrum',
      'Autobahnanschluss A1',
      'Bassersdorf',
    ]) {
      expect((DESTINATIONS as Record<string, unknown>)[name]).toBeDefined();
    }
  });
});
