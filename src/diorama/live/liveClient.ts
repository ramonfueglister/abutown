// src/diorama/live/liveClient.ts
//
// Browser WS client for the sim-server /live gateway (MMORPG M1 Task 14).
// Mirrors src/diorama/traffic/trafficClient.ts's structure:
//   * a pure, WS/DOM-free core (LiveClientCore) that decodes LiveServerMsg
//     binaries and maintains the per-cell citizen tables — exercised directly
//     by tests/live/liveClient.test.ts;
//   * a thin WebSocket wrapper (createLiveClient) with reconnect + AOI
//     (un)subscribe plumbing.
//
// Wire semantics honoured here (live.proto):
//   * cell ids are the SAME 128 m CellGrid ids as the traffic channel — the
//     caller derives them via the shared src/diorama/traffic/cellGrid.ts
//     (buildDefaultCellGrid / TrafficClient.grid) and passes them to
//     updateAoi(); an off-grid derivation silently produces empty
//     subscriptions (Phase-7a bug class);
//   * a keyframe CitizenCellFrame REPLACES the full membership of its cell;
//   * a delta upserts `citizens` and removes `departed` (authoritative);
//   * positions are x_dm/z_dm sint32 decimetres -> metres via /10;
//   * EconomyVitals arrives 1 Hz once subscribe_vitals was sent (we always
//     subscribe on (re)connect).

import { create, fromBinary, toBinary } from '@bufbuild/protobuf';
import {
  LiveClientMsgSchema,
  LiveServerMsgSchema,
  type LiveServerMsg,
} from '../../proto/live_pb.js';

/** Default gateway endpoint; overridable via ?liveWs=… / VITE_LIVE_WS. */
export const DEFAULT_LIVE_WS = 'ws://localhost:8080/live';

export interface LiveVitals {
  worldTick: bigint;
  sOfWorldDay: number;
  population: bigint;
  totalMoney: bigint;
  auditOk: boolean;
  prices: { marketId: number; goodId: number; ewmaPrice: bigint; marketName: string }[];
  tripsActive: bigint;
}

/** One live citizen, positions already decoded to metres. */
export interface LiveCitizen {
  id: number;
  x: number;
  z: number;
  /** 0=home 1=work 2=market 3=walking 4=driving (live.proto CitizenState). */
  activity: number;
}

export interface LiveClientCallbacks {
  onVitals: (v: LiveVitals) => void;
  onCitizens: (cell: number, citizens: LiveCitizen[], departed: number[], keyframe: boolean) => void;
}

/** The pure, WS/DOM-free core: binary decode, unit conversion, and the
 * per-cell citizen bookkeeping (keyframe replace / delta upsert+depart). */
export class LiveClientCore {
  /** cell id -> (citizen id -> citizen). Deterministic insertion order per
   * frame application; read by the citizens layer via citizensInCell(). */
  private readonly cells = new Map<number, Map<number, LiveCitizen>>();
  /** Newest world tick seen on any citizen frame. */
  worldTick = 0;

  private readonly cb: LiveClientCallbacks;

  constructor(cb: LiveClientCallbacks) {
    this.cb = cb;
  }

  /** All citizens currently tracked in `cell` (empty iterable if none). */
  citizensInCell(cell: number): Iterable<LiveCitizen> {
    return this.cells.get(cell)?.values() ?? [];
  }

  /** Total tracked citizens across all cells (debug/smoke surface). */
  get citizenCount(): number {
    let n = 0;
    for (const m of this.cells.values()) n += m.size;
    return n;
  }

  /** Drop the local state of cells we no longer subscribe to — their
   * `departed` frames stop arriving after unsubscribe, so keeping them
   * would leak ghosts (same failure mode as trafficClient Task 9 finding 3). */
  dropCells(cellIds: number[]): void {
    for (const c of cellIds) this.cells.delete(c);
  }

  /** Decode one LiveServerMsg binary and apply it (the WS message path). */
  handleBinary(bytes: Uint8Array): void {
    this.applyServerMsg(fromBinary(LiveServerMsgSchema, bytes));
  }

  applyServerMsg(msg: LiveServerMsg): void {
    for (const frame of msg.cells) {
      const tick = Number(frame.worldTick);
      if (tick > this.worldTick) this.worldTick = tick;

      const citizens: LiveCitizen[] = frame.citizens.map((c) => ({
        id: c.id,
        x: c.xDm / 10, // decimetres -> metres
        z: c.zDm / 10,
        activity: c.activity,
      }));

      let table = this.cells.get(frame.cell);
      if (frame.keyframe || !table) {
        // A keyframe is the FULL membership of its cell — replace wholesale.
        table = new Map();
        this.cells.set(frame.cell, table);
      }
      for (const c of citizens) table.set(c.id, c);
      if (!frame.keyframe) {
        for (const id of frame.departed) table.delete(id);
      }

      this.cb.onCitizens(frame.cell, citizens, [...frame.departed], frame.keyframe);
    }

    if (msg.vitals) {
      const v = msg.vitals;
      this.cb.onVitals({
        worldTick: v.worldTick,
        sOfWorldDay: v.sOfWorldDay,
        population: v.population,
        totalMoney: v.totalMoney,
        auditOk: v.auditOk === 1,
        prices: v.prices.map((p) => ({
          marketId: p.marketId,
          goodId: p.goodId,
          ewmaPrice: p.ewmaPrice,
          marketName: p.marketName,
        })),
        tripsActive: v.tripsActive,
      });
    }
    // msg.buildings (BuildingDelta) is not consumed by the M1 frontend yet.
  }
}

export interface LiveClient {
  /** Replace the subscribed AOI cell set; only the delta goes on the wire.
   * Cell ids MUST come from the shared traffic CellGrid derivation. */
  updateAoi(cells: number[]): void;
  close(): void;
}

export function createLiveClient(opts: {
  url: string;
  onVitals: (v: LiveVitals) => void;
  onCitizens: (cell: number, citizens: LiveCitizen[], departed: number[], keyframe: boolean) => void;
}): LiveClient & { core: LiveClientCore } {
  const core = new LiveClientCore({ onVitals: opts.onVitals, onCitizens: opts.onCitizens });

  const subscribed = new Set<number>();
  let ws: WebSocket | null = null;
  let closed = false;

  const send = (subscribe: number[], unsubscribe: number[], subscribeVitals?: boolean): void => {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const msg = create(LiveClientMsgSchema, {
      subscribeCells: subscribe,
      unsubscribeCells: unsubscribe,
      subscribeVitals,
    });
    ws.send(toBinary(LiveClientMsgSchema, msg));
  };

  const open = (): void => {
    const sock = new WebSocket(opts.url);
    sock.binaryType = 'arraybuffer';
    ws = sock;
    sock.addEventListener('open', () => {
      // Re-send the current subscription set on (re)connect; vitals always on.
      send([...subscribed], [], true);
    });
    sock.addEventListener('message', (ev: MessageEvent) => {
      if (!(ev.data instanceof ArrayBuffer)) return;
      core.handleBinary(new Uint8Array(ev.data));
    });
    sock.addEventListener('close', () => {
      ws = null;
      if (!closed) {
        // Simple reconnect: the subscription set is re-sent on the next open.
        setTimeout(() => { if (!closed) open(); }, 1000);
      }
    });
    sock.addEventListener('error', () => { /* close handler drives reconnect */ });
  };
  open();

  return {
    core,
    updateAoi(cells: number[]): void {
      const want = new Set(cells);
      const add: number[] = [];
      for (const c of want) if (!subscribed.has(c)) add.push(c);
      const remove: number[] = [];
      for (const c of subscribed) if (!want.has(c)) remove.push(c);
      if (add.length === 0 && remove.length === 0) return;
      for (const c of add) subscribed.add(c);
      for (const c of remove) subscribed.delete(c);
      core.dropCells(remove);
      send(add, remove);
    },
    close(): void {
      closed = true;
      ws?.close();
      ws = null;
    },
  };
}
