// Declarative floor plan of the Kantonsspital Winterthur diorama.
// One level, ladder circulation: three room rows, two east-west corridors,
// two end connectors, entrance plaza + ambulance apron to the south.
// All positions in world meters, y = 0 ground plane. Geometry invariants
// are enforced by tests/diorama/floorPlan.test.ts.

import { palette } from '../designTokens';

export type PropPlacement = { kind: string; x: number; z: number; rotY?: number; scale?: number };

export type PersonRole =
  | 'nurse'
  | 'doctor'
  | 'surgeon'
  | 'patient'
  | 'child'
  | 'visitor'
  | 'labtech'
  | 'paramedic';

export type PersonPlacement = { role: PersonRole; x: number; z: number; yaw: number };

export type WallSide = 'n' | 's' | 'e' | 'w';

export type Room = {
  id: string;
  label: string;
  accent: number;
  // rect: center x/z, outer width (x extent) and depth (z extent)
  rect: { x: number; z: number; w: number; d: number };
  // center = offset along the wall from the wall's midpoint
  doors: Array<{ wall: WallSide; center: number; width: number }>;
  windows: Array<{ wall: WallSide; center: number; width: number }>;
  props: PropPlacement[];
  people: PersonPlacement[];
};

export type FloorPlan = {
  plate: { w: number; d: number };
  building: { x: number; z: number; w: number; d: number };
  corridors: Array<{ x: number; z: number; w: number; d: number }>;
  rooms: Room[];
  outdoorProps: PropPlacement[];
  outdoorPeople: PersonPlacement[];
};

const win = (wall: WallSide, center: number, width = 1.4) => ({ wall, center, width });
const door = (wall: WallSide, center: number, width = 1.6) => ({ wall, center, width });

export const kswPlan: FloorPlan = {
  plate: { w: 72, d: 56 },
  building: { x: 0, z: -2, w: 60, d: 38 }, // x -30..30, z -21..17
  corridors: [
    { x: 0, z: -9, w: 60, d: 4 }, // corridor A (north), z -11..-7
    { x: 0, z: 5, w: 60, d: 4 }, // corridor B (south), z 3..7
    { x: -28, z: -2, w: 4, d: 10 }, // west connector, z -7..3
    { x: 28, z: -2, w: 4, d: 10 }, // east connector, z -7..3
  ],
  rooms: [
    // ── Row N (z -21..-11): surgery, intensive care, imaging, lab ──
    {
      id: 'op1',
      label: 'Zentral-OP Saal 1',
      accent: palette.mint,
      rect: { x: -26, z: -16, w: 8, d: 10 },
      doors: [door('s', 0, 1.8)],
      windows: [win('n', -1.8), win('n', 1.8), win('w', 0)],
      props: [
        { kind: 'opTable', x: -26, z: -16.4 },
        { kind: 'opLightDouble', x: -26, z: -16.4 },
        { kind: 'anesthesiaMachine', x: -28.4, z: -17.6, rotY: 0.5 },
        { kind: 'instrumentTable', x: -24.2, z: -17.8 },
        { kind: 'vitalsMonitor', x: -27.9, z: -15.2, rotY: 1.2 },
        { kind: 'scrubSink', x: -23.3, z: -13.2, rotY: -Math.PI / 2 },
      ],
      people: [
        { role: 'surgeon', x: -26.9, z: -16.3, yaw: 1.55 },
        { role: 'surgeon', x: -25.1, z: -16.5, yaw: -1.55 },
        { role: 'nurse', x: -24.6, z: -15.1, yaw: -2.4 },
      ],
    },
    {
      id: 'op2',
      label: 'Zentral-OP Saal 2',
      accent: palette.mint,
      rect: { x: -18, z: -16, w: 8, d: 10 },
      doors: [door('s', 0, 1.8)],
      windows: [win('n', -1.8), win('n', 1.8)],
      props: [
        { kind: 'opTable', x: -18, z: -16.2, rotY: Math.PI / 2 },
        { kind: 'opLightDouble', x: -18, z: -16.2, rotY: Math.PI / 2 },
        { kind: 'anesthesiaMachine', x: -20.6, z: -17.4, rotY: 0.8 },
        { kind: 'instrumentTable', x: -15.9, z: -17.5 },
        { kind: 'careCart', x: -15.7, z: -14.4, rotY: 0.4 },
      ],
      people: [
        { role: 'surgeon', x: -18.1, z: -17.6, yaw: 0.0 },
        { role: 'nurse', x: -16.6, z: -16.1, yaw: -1.3 },
      ],
    },
    {
      id: 'opPrep',
      label: 'OP-Einleitung',
      accent: palette.sage,
      rect: { x: -11.5, z: -16, w: 5, d: 10 },
      doors: [door('s', 0)],
      windows: [win('n', 0)],
      props: [
        { kind: 'stretcher', x: -11.6, z: -16.6, rotY: Math.PI / 2 },
        { kind: 'ivStand', x: -10.2, z: -17.6 },
        { kind: 'careCart', x: -13.1, z: -17.7, rotY: -0.3 },
        { kind: 'handSanitizer', x: -9.8, z: -13.0 },
      ],
      people: [
        { role: 'nurse', x: -12.9, z: -15.0, yaw: 2.6 },
        { role: 'patient', x: -11.6, z: -15.4, yaw: 3.1 },
      ],
    },
    {
      id: 'ips',
      label: 'Intensivstation IPS',
      accent: palette.coralSoft,
      rect: { x: -4, z: -16, w: 10, d: 10 },
      doors: [door('s', 0, 2.0)],
      windows: [win('n', -3), win('n', 0), win('n', 3)],
      props: [
        { kind: 'icuBed', x: -7.2, z: -17.4 },
        { kind: 'icuBed', x: -4.0, z: -17.4 },
        { kind: 'icuBed', x: -0.8, z: -17.4 },
        { kind: 'ventilator', x: -6.0, z: -18.6 },
        { kind: 'ventilator', x: 0.4, z: -18.6 },
        { kind: 'vitalsMonitor', x: -2.8, z: -18.5, rotY: 0.4 },
        { kind: 'careCart', x: -7.9, z: -13.4, rotY: 0.9 },
        { kind: 'handSanitizer', x: -0.2, z: -13.0 },
      ],
      people: [
        { role: 'nurse', x: -5.6, z: -15.6, yaw: 2.9 },
        { role: 'nurse', x: -0.9, z: -15.8, yaw: -2.7 },
        { role: 'doctor', x: -3.2, z: -14.6, yaw: 2.2 },
      ],
    },
    {
      id: 'xray',
      label: 'Radiologie Röntgen',
      accent: palette.glass,
      rect: { x: 4, z: -16, w: 6, d: 10 },
      doors: [door('s', -1.2)],
      windows: [win('n', 0)],
      props: [
        { kind: 'xrayMachine', x: 4.2, z: -17.0 },
        { kind: 'leadShieldWindow', x: 2.0, z: -13.6, rotY: 0.5 },
        { kind: 'radiologyConsole', x: 1.9, z: -14.6, rotY: 0.5 },
      ],
      people: [
        { role: 'labtech', x: 2.6, z: -15.3, yaw: 0.8 },
        { role: 'patient', x: 5.4, z: -15.0, yaw: -2.6 },
      ],
    },
    {
      id: 'ct',
      label: 'Computertomographie CT',
      accent: palette.glass,
      rect: { x: 10.5, z: -16, w: 7, d: 10 },
      doors: [door('s', 1.4)],
      windows: [],
      props: [
        { kind: 'ctScanner', x: 10.5, z: -16.8 },
        { kind: 'leadShieldWindow', x: 8.2, z: -13.5, rotY: -0.4 },
        { kind: 'radiologyConsole', x: 8.3, z: -14.5, rotY: -0.4 },
      ],
      people: [{ role: 'doctor', x: 8.9, z: -15.4, yaw: 0.6 }],
    },
    {
      id: 'mri',
      label: 'MRI Magnetresonanz',
      accent: palette.glass,
      rect: { x: 18, z: -16, w: 8, d: 10 },
      doors: [door('s', -1.6)],
      windows: [],
      props: [
        { kind: 'mriScanner', x: 18.2, z: -16.6 },
        { kind: 'leadShieldWindow', x: 15.3, z: -13.6, rotY: -0.5 },
        { kind: 'radiologyConsole', x: 15.4, z: -14.7, rotY: -0.5 },
        { kind: 'handSanitizer', x: 21.4, z: -13.2 },
      ],
      people: [{ role: 'labtech', x: 16.2, z: -15.1, yaw: 0.7 }],
    },
    {
      id: 'lab',
      label: 'Zentrallabor',
      accent: palette.plantGreen,
      rect: { x: 26, z: -16, w: 8, d: 10 },
      doors: [door('s', 0)],
      windows: [win('n', -1.8), win('n', 1.8), win('e', 0)],
      props: [
        { kind: 'labBench', x: 24.4, z: -17.6 },
        { kind: 'labBench', x: 27.8, z: -17.6 },
        { kind: 'microscope', x: 24.4, z: -17.5 },
        { kind: 'centrifuge', x: 27.8, z: -17.5 },
        { kind: 'sampleRack', x: 26.1, z: -17.5 },
        { kind: 'labBench', x: 28.2, z: -14.2, rotY: Math.PI / 2 },
        { kind: 'plant', x: 22.6, z: -13.2 },
      ],
      people: [
        { role: 'labtech', x: 24.5, z: -16.2, yaw: 3.1 },
        { role: 'labtech', x: 27.2, z: -15.6, yaw: 2.6 },
      ],
    },

    // ── Row M (z -7..3): cardio, endoscopy, wards, physio, admin ──
    {
      id: 'cardio',
      label: 'Kardiologie Herzkatheter',
      accent: palette.coral,
      rect: { x: -22, z: -2, w: 8, d: 10 },
      doors: [door('n', 0, 1.8)],
      windows: [],
      props: [
        { kind: 'opTable', x: -22, z: -2.4 },
        { kind: 'cathLabArm', x: -22, z: -2.4 },
        { kind: 'radiologyConsole', x: -24.6, z: 0.8, rotY: 2.6 },
        { kind: 'vitalsMonitor', x: -19.6, z: -3.6, rotY: -0.6 },
      ],
      people: [
        { role: 'doctor', x: -23.2, z: -2.5, yaw: 1.5 },
        { role: 'nurse', x: -20.8, z: -1.2, yaw: -2.2 },
      ],
    },
    {
      id: 'endo',
      label: 'Endoskopie',
      accent: palette.honey,
      rect: { x: -14.5, z: -2, w: 7, d: 10 },
      doors: [door('n', 0), door('s', 0)],
      windows: [],
      props: [
        { kind: 'stretcher', x: -14.6, z: -2.4, rotY: Math.PI / 2 },
        { kind: 'endoscopyTower', x: -16.9, z: -3.6 },
        { kind: 'careCart', x: -12.3, z: -3.8, rotY: -0.4 },
        { kind: 'handSanitizer', x: -11.8, z: 1.6 },
      ],
      people: [
        { role: 'doctor', x: -15.9, z: -1.6, yaw: 1.9 },
        { role: 'nurse', x: -13.2, z: -1.0, yaw: -2.0 },
      ],
    },
    {
      id: 'wardChirurgie',
      label: 'Bettenstation Chirurgie',
      accent: palette.sage,
      rect: { x: -6, z: -2, w: 10, d: 10 },
      doors: [door('n', 0, 1.8), door('s', 0, 1.8)],
      windows: [],
      props: [
        { kind: 'hospitalBed', x: -8.6, z: -4.2 },
        { kind: 'hospitalBed', x: -8.6, z: -1.4 },
        { kind: 'hospitalBed', x: -3.2, z: -4.2, rotY: Math.PI },
        { kind: 'hospitalBed', x: -3.2, z: -1.4, rotY: Math.PI },
        { kind: 'sideTable', x: -6.8, z: -4.4 },
        { kind: 'sideTable', x: -5.0, z: -1.2 },
        { kind: 'ivStand', x: -8.9, z: -2.8 },
        { kind: 'wheelchair', x: -2.2, z: 1.4, rotY: 2.4 },
        { kind: 'plant', x: -9.9, z: 1.6 },
      ],
      people: [
        { role: 'nurse', x: -6.1, z: -2.9, yaw: 2.1 },
        { role: 'patient', x: -4.4, z: 0.6, yaw: -2.6 },
        { role: 'visitor', x: -7.4, z: 0.9, yaw: 1.2 },
      ],
    },
    {
      id: 'wardMedizin',
      label: 'Bettenstation Medizin',
      accent: palette.mint,
      rect: { x: 4, z: -2, w: 10, d: 10 },
      doors: [door('n', 0, 1.8), door('s', 0, 1.8)],
      windows: [],
      props: [
        { kind: 'hospitalBed', x: 1.4, z: -4.2 },
        { kind: 'hospitalBed', x: 1.4, z: -1.4 },
        { kind: 'hospitalBed', x: 6.8, z: -4.2, rotY: Math.PI },
        { kind: 'hospitalBed', x: 6.8, z: -1.4, rotY: Math.PI },
        { kind: 'sideTable', x: 3.2, z: -4.4 },
        { kind: 'vitalsMonitor', x: 5.2, z: -1.1, rotY: 2.4 },
        { kind: 'linenCart', x: 8.0, z: 1.4, rotY: 0.6 },
        { kind: 'plant', x: 0.2, z: 1.6 },
      ],
      people: [
        { role: 'nurse', x: 3.9, z: -2.9, yaw: 1.0 },
        { role: 'patient', x: 5.7, z: 0.7, yaw: -2.9 },
        { role: 'visitor', x: 2.4, z: 0.9, yaw: 2.0 },
      ],
    },
    {
      id: 'physio',
      label: 'Physiotherapie',
      accent: palette.honey,
      rect: { x: 13.5, z: -2, w: 9, d: 10 },
      doors: [door('n', 0), door('s', 0)],
      windows: [],
      props: [
        { kind: 'physioTable', x: 11.2, z: -4.0 },
        { kind: 'physioTable', x: 11.2, z: -1.2 },
        { kind: 'exerciseBike', x: 16.2, z: -4.2, rotY: -0.5 },
        { kind: 'parallelBars', x: 15.4, z: 0.4, rotY: Math.PI / 2 },
        { kind: 'gymBall', x: 13.4, z: 1.5 },
        { kind: 'gymBall', x: 17.2, z: -1.6 },
        { kind: 'plant', x: 9.7, z: 1.7 },
      ],
      people: [
        { role: 'nurse', x: 13.0, z: -2.5, yaw: 1.9 },
        { role: 'patient', x: 15.4, z: -0.6, yaw: 3.0 },
      ],
    },
    {
      id: 'admin',
      label: 'Verwaltung',
      accent: palette.woodSoft,
      rect: { x: 22, z: -2, w: 8, d: 10 },
      doors: [door('n', 0), door('s', 0)],
      windows: [],
      props: [
        { kind: 'officeDesk', x: 20.2, z: -4.0 },
        { kind: 'officeChair', x: 20.2, z: -3.0, rotY: Math.PI },
        { kind: 'officeDesk', x: 24.0, z: -4.0 },
        { kind: 'officeChair', x: 24.0, z: -3.0, rotY: Math.PI },
        { kind: 'filingCabinet', x: 25.3, z: -0.4, rotY: -Math.PI / 2 },
        { kind: 'filingCabinet', x: 25.3, z: 1.0, rotY: -Math.PI / 2 },
        { kind: 'infoBoard', x: 19.0, z: 1.9, rotY: Math.PI },
        { kind: 'deskPlant', x: 21.2, z: -4.1 },
      ],
      people: [
        { role: 'visitor', x: 20.3, z: -2.2, yaw: 3.1 },
        { role: 'doctor', x: 23.2, z: 0.6, yaw: -1.2 },
      ],
    },

    // ── Row S (z 7..17): emergency, birth, entrance, day clinics, cafeteria ──
    {
      id: 'notfall',
      label: 'Interdisziplinäres Notfallzentrum',
      accent: palette.coral,
      rect: { x: -23.5, z: 12, w: 13, d: 10 },
      doors: [door('n', 2.0, 1.8), door('s', -3.0, 2.4)],
      windows: [win('w', -2.0), win('w', 2.0)],
      props: [
        { kind: 'triageDesk', x: -20.0, z: 9.2, rotY: Math.PI },
        { kind: 'waitingBench', x: -18.2, z: 12.4, rotY: Math.PI / 2 },
        { kind: 'waitingBench', x: -18.2, z: 14.6, rotY: Math.PI / 2 },
        { kind: 'stretcher', x: -27.8, z: 9.6, rotY: Math.PI / 2 },
        { kind: 'stretcher', x: -27.8, z: 12.2, rotY: Math.PI / 2 },
        { kind: 'shockroomLight', x: -27.8, z: 9.6 },
        { kind: 'defibrillator', x: -29.0, z: 13.6 },
        { kind: 'careCart', x: -26.4, z: 13.8, rotY: 0.5 },
        { kind: 'ivStand', x: -26.6, z: 10.6 },
        { kind: 'vitalsMonitor', x: -28.8, z: 10.9, rotY: 1.2 },
        { kind: 'wheelchair', x: -21.6, z: 15.2, rotY: -0.8 },
        { kind: 'handSanitizer', x: -17.6, z: 8.0 },
      ],
      people: [
        { role: 'nurse', x: -20.6, z: 10.6, yaw: 0.4 },
        { role: 'nurse', x: -26.9, z: 11.4, yaw: 1.7 },
        { role: 'doctor', x: -25.2, z: 9.9, yaw: 2.4 },
        { role: 'patient', x: -18.9, z: 12.4, yaw: -1.5 },
        { role: 'patient', x: -18.9, z: 14.6, yaw: -1.5 },
        { role: 'paramedic', x: -23.4, z: 14.9, yaw: 0.2 },
      ],
    },
    {
      id: 'geburt',
      label: 'Gebärsaal',
      accent: palette.coralSoft,
      rect: { x: -14.25, z: 12, w: 5.5, d: 10 },
      doors: [door('n', 0)],
      windows: [win('s', 0)],
      props: [
        { kind: 'birthingBed', x: -14.3, z: 12.6 },
        { kind: 'ivStand', x: -12.6, z: 11.4 },
        { kind: 'vitalsMonitor', x: -16.0, z: 11.2, rotY: 0.9 },
        { kind: 'babyCrib', x: -12.5, z: 14.6, rotY: 0.4 },
      ],
      people: [
        { role: 'nurse', x: -15.6, z: 13.2, yaw: 1.2 },
        { role: 'patient', x: -14.3, z: 12.0, yaw: 3.1 },
      ],
    },
    {
      id: 'neo',
      label: 'Neonatologie',
      accent: palette.honey,
      rect: { x: -9.25, z: 12, w: 4.5, d: 10 },
      doors: [door('n', 0)],
      windows: [win('s', 0)],
      props: [
        { kind: 'incubator', x: -10.2, z: 12.6 },
        { kind: 'incubator', x: -8.2, z: 12.6 },
        { kind: 'babyCrib', x: -10.2, z: 15.0, rotY: -0.3 },
        { kind: 'vitalsMonitor', x: -7.8, z: 14.8, rotY: -2.2 },
      ],
      people: [{ role: 'nurse', x: -9.2, z: 10.6, yaw: 3.0 }],
    },
    {
      id: 'empfang',
      label: 'Eingangshalle Empfang',
      accent: palette.mint,
      rect: { x: -2.5, z: 12, w: 9, d: 10 },
      doors: [door('n', 0, 1.8), door('s', 0, 2.6)],
      windows: [win('s', -2.8, 1.5), win('s', 2.8, 1.5)],
      props: [
        { kind: 'receptionDesk', x: -2.5, z: 9.4, rotY: Math.PI },
        { kind: 'infoBoard', x: -6.3, z: 9.0, rotY: 0.6 },
        { kind: 'waitingBench', x: -5.6, z: 13.8, rotY: Math.PI / 2 },
        { kind: 'waitingBench', x: 0.6, z: 13.8, rotY: -Math.PI / 2 },
        { kind: 'plant', x: -6.2, z: 15.6 },
        { kind: 'plant', x: 1.2, z: 15.6 },
        { kind: 'wheelchair', x: 0.9, z: 9.4, rotY: 1.9 },
        { kind: 'handSanitizer', x: -0.2, z: 15.9 },
      ],
      people: [
        { role: 'nurse', x: -2.5, z: 8.8, yaw: 3.14 },
        { role: 'visitor', x: -4.9, z: 13.6, yaw: -1.4 },
        { role: 'visitor', x: -1.3, z: 12.2, yaw: 0.8 },
        { role: 'child', x: -0.1, z: 13.5, yaw: -0.9 },
      ],
    },
    {
      id: 'kinder',
      label: 'Kinderklinik',
      accent: palette.honey,
      rect: { x: 4.75, z: 12, w: 5.5, d: 10 },
      doors: [door('n', 0)],
      windows: [win('s', 0)],
      props: [
        { kind: 'hospitalBed', x: 4.0, z: 12.8, rotY: Math.PI / 2, scale: 0.85 },
        { kind: 'cafeTable', x: 6.2, z: 14.4, scale: 0.8 },
        { kind: 'gymBall', x: 6.4, z: 11.2, scale: 0.7 },
        { kind: 'plant', x: 2.8, z: 15.4, scale: 0.9 },
        { kind: 'sideTable', x: 6.6, z: 9.4 },
      ],
      people: [
        { role: 'child', x: 5.8, z: 13.4, yaw: -0.7 },
        { role: 'child', x: 4.1, z: 10.8, yaw: 1.3 },
        { role: 'nurse', x: 3.3, z: 12.1, yaw: 1.8 },
      ],
    },
    {
      id: 'onko',
      label: 'Onkologie Tagesklinik',
      accent: palette.sage,
      rect: { x: 10.25, z: 12, w: 5.5, d: 10 },
      doors: [door('n', 0)],
      windows: [win('s', 0)],
      props: [
        { kind: 'infusionChair', x: 8.8, z: 12.4, rotY: 0.9 },
        { kind: 'infusionChair', x: 10.4, z: 13.2, rotY: 0.3 },
        { kind: 'infusionChair', x: 12.0, z: 12.4, rotY: -0.6 },
        { kind: 'ivStand', x: 9.6, z: 13.6 },
        { kind: 'ivStand', x: 11.4, z: 13.6 },
        { kind: 'plant', x: 12.2, z: 15.2 },
      ],
      people: [
        { role: 'nurse', x: 10.2, z: 10.4, yaw: 2.9 },
        { role: 'patient', x: 8.8, z: 12.2, yaw: 0.9 },
        { role: 'patient', x: 12.0, z: 12.2, yaw: -0.6 },
      ],
    },
    {
      id: 'dialyse',
      label: 'Dialyse',
      accent: palette.glass,
      rect: { x: 15.75, z: 12, w: 5.5, d: 10 },
      doors: [door('n', 0)],
      windows: [win('s', 0)],
      props: [
        { kind: 'infusionChair', x: 14.4, z: 12.6, rotY: 0.6 },
        { kind: 'infusionChair', x: 15.9, z: 13.2 },
        { kind: 'infusionChair', x: 17.4, z: 12.6, rotY: -0.6 },
        { kind: 'dialysisMachine', x: 14.0, z: 14.2 },
        { kind: 'dialysisMachine', x: 15.9, z: 14.6 },
        { kind: 'dialysisMachine', x: 17.8, z: 14.2 },
      ],
      people: [
        { role: 'nurse', x: 15.7, z: 10.4, yaw: 3.0 },
        { role: 'patient', x: 15.9, z: 13.0, yaw: 0.0 },
      ],
    },
    {
      id: 'apotheke',
      label: 'Spitalapotheke',
      accent: palette.plantGreen,
      rect: { x: 20.75, z: 12, w: 4.5, d: 10 },
      doors: [door('n', 0)],
      windows: [win('s', 0)],
      props: [
        { kind: 'pharmacyShelf', x: 19.4, z: 13.8, rotY: Math.PI / 2 },
        { kind: 'pharmacyShelf', x: 22.1, z: 13.8, rotY: -Math.PI / 2 },
        { kind: 'counterDesk', x: 20.7, z: 9.6, rotY: Math.PI },
      ],
      people: [{ role: 'labtech', x: 20.7, z: 11.6, yaw: 0.0 }],
    },
    {
      id: 'cafeteria',
      label: 'Cafeteria',
      accent: palette.woodSoft,
      rect: { x: 26.5, z: 12, w: 7, d: 10 },
      doors: [door('n', 0, 1.8)],
      windows: [win('s', -1.6, 1.5), win('s', 1.6, 1.5), win('e', 0)],
      props: [
        { kind: 'counterBar', x: 26.5, z: 9.2, rotY: Math.PI },
        { kind: 'espressoMachine', x: 27.4, z: 9.2 },
        { kind: 'cafeTable', x: 24.8, z: 12.6 },
        { kind: 'cafeChair', x: 24.0, z: 12.6, rotY: Math.PI / 2 },
        { kind: 'cafeChair', x: 25.6, z: 12.6, rotY: -Math.PI / 2 },
        { kind: 'cafeTable', x: 28.2, z: 12.6 },
        { kind: 'cafeChair', x: 27.4, z: 12.6, rotY: Math.PI / 2 },
        { kind: 'cafeChair', x: 29.0, z: 12.6, rotY: -Math.PI / 2 },
        { kind: 'cafeTable', x: 26.5, z: 15.0 },
        { kind: 'cafeChair', x: 25.7, z: 15.0, rotY: Math.PI / 2 },
        { kind: 'cafeChair', x: 27.3, z: 15.0, rotY: -Math.PI / 2 },
        { kind: 'plant', x: 23.6, z: 15.4 },
      ],
      people: [
        { role: 'visitor', x: 24.0, z: 12.9, yaw: 1.6 },
        { role: 'doctor', x: 27.4, z: 13.0, yaw: 1.5 },
        { role: 'child', x: 25.7, z: 15.3, yaw: 1.6 },
        { role: 'visitor', x: 26.4, z: 10.4, yaw: 3.1 },
      ],
    },
  ],
  outdoorProps: [
    { kind: 'ambulance', x: -24.5, z: 20.5, rotY: 0.15 },
    { kind: 'waitingBench', x: -6.4, z: 19.6, rotY: Math.PI },
    { kind: 'waitingBench', x: 1.4, z: 19.6, rotY: Math.PI },
    { kind: 'plant', x: -8.6, z: 19.4, scale: 1.3 },
    { kind: 'plant', x: 3.6, z: 19.4, scale: 1.3 },
    { kind: 'plant', x: -14.0, z: 18.6, scale: 1.1 },
    { kind: 'plant', x: 9.0, z: 18.6, scale: 1.1 },
    { kind: 'wheelchair', x: -19.8, z: 18.4, rotY: 2.6 },
  ],
  outdoorPeople: [
    { role: 'paramedic', x: -23.0, z: 18.3, yaw: 2.6 },
    { role: 'paramedic', x: -25.8, z: 18.6, yaw: -2.4 },
    { role: 'visitor', x: -2.4, z: 20.8, yaw: 0.3 },
    { role: 'child', x: -1.2, z: 21.2, yaw: -0.4 },
    { role: 'visitor', x: 5.2, z: 20.2, yaw: 2.2 },
    { role: 'doctor', x: -10.8, z: 19.8, yaw: 1.1 },
  ],
};
