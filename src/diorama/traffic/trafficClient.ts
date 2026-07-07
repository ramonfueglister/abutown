// src/diorama/traffic/trafficClient.ts
//
// Browser WS client for the winterthur-traffic gateway (Task 8 wire contract).
//   * imports data/winterthur/trafficnet.json at BUILD TIME (the SAME asset the
//     server bakes from — single source of truth for lane polylines + the AOI
//     grid), following the established static-import pattern in
//     src/diorama/ksw/geo/geoData.ts — no runtime fetch (data/ is not served by
//     vite; publicDir is public/, so a `fetch('data/...')` 200s on the SPA HTML
//     fallback and silently produces zero cars — Task 9 review finding 1);
//   * recomputes the AOI cell grid IDENTICALLY to the server so cell ids match
//     (see CellGrid below — replicates backend/crates/winterthur-traffic/
//     src/cells.rs `plate_bbox` + `CellGrid::build`, lines ~66-82/164-181;
//     an off-by-one here silently produces empty subscriptions);
//   * resolves a vehicle's cell via vertex-keyed per-lane (sEnd, cell)
//     breakpoints, a direct port of `cells.rs`'s `CellGrid::build` /
//     `cell_of_lane_s` (lines ~84-121/144-158) — ONE canonical path
//     (`CellGrid.cellOfLaneS`) used by keyframe ghost-healing and stale-vehicle
//     eviction alike, so the two never disagree at a cell border (Task 9
//     review finding 2);
//   * opens the WS, subscribes to the 3×3 cells around the camera target and
//     unsubscribes cells that leave the 5×5 neighbourhood (hysteresis);
//   * on every throttled camera update, evicts vehicles whose canonical cell
//     has fallen outside the 5×5 band — otherwise a long panning session grows
//     the vehicle table to the 4096 cap and silently drops new vehicles
//     (Task 9 review finding 3);
//   * applies CellFrames into a Map<vehId, VehKinematics> that carLayer reads.
//
// Wire semantics honoured here (traffic.proto):
//   * vehicle id is OPAQUE (slot | generation<<12) — used only as a Map key;
//   * `departed` lists are authoritative removals;
//   * a keyframe REPLACES the full membership of its cell (heals ghosts within
//     the ≤5 s keyframe cadence);
//   * a delta updates/adds `vehicles` and removes `departed`.
//
// Frame decoding uses the generated proto (src/proto/traffic_pb) + the
// @bufbuild/protobuf runtime, exactly as the build pipeline expects.

import { fromBinary, toBinary, create } from '@bufbuild/protobuf';
import {
  TrafficServerMsgSchema,
  TrafficClientMsgSchema,
} from '../../proto/traffic_pb.js';
import trafficNetJson from '../../../data/winterthur/trafficnet.json';
import { buildLaneNet, type RawLane, type TrafficNetGeom, type VehKinematics } from './deadReckon';
import { CellGrid } from './cellGrid';
import { beginLaneChange } from './laneBlend';

// The AOI cell grid lives in cellGrid.ts (shared with the live/citizens
// channel — Task 14 extraction); re-exported here so existing importers
// (tests, flowLayer) keep working unchanged.
export { CellGrid, CELL_SIZE_M } from './cellGrid';

/** Dev-default gateway endpoint (localhost); overridable via ?trafficWs=… */
export const DEFAULT_TRAFFIC_WS = 'ws://localhost:8790/traffic';

/** Production gateway: the deployed sim-server `/traffic` WS on Fly (fly.toml
 * app `abutown-abutopia`). Used as the default on any non-localhost host so a
 * static/Vercel deploy shows cars with zero config; override via
 * VITE_TRAFFIC_WS or ?trafficWs=… if the backend moves. */
export const PROD_TRAFFIC_WS = 'wss://abutown-abutopia.fly.dev/traffic';

/** The raw trafficnet.json document shape (fields we consume). */
interface TrafficNetDoc {
  lanes: RawLane[];
}

/** The statically-imported net document (see the module banner). Typed via a
 * cast, matching geoData.ts's `as { buildings: BakedBuilding[] }` pattern for
 * its JSON imports. */
const trafficNetDoc = trafficNetJson as unknown as TrafficNetDoc;

// (trafficnet doc typing above; CellGrid + CELL_SIZE_M now live in cellGrid.ts.)

/** Build the canonical AOI CellGrid from the build-time-imported
 * trafficnet.json lanes — the shared derivation for BOTH the traffic and the
 * live (citizens) channels (Task 14). */
export function buildDefaultCellGrid(): CellGrid {
  return CellGrid.build(trafficNetDoc.lanes);
}

/** The pure, WS/DOM-free core of the traffic client: subscription-set
 * bookkeeping, frame application (keyframe ghost-heal + delta), and
 * stale-vehicle eviction. Exercised directly by tests/traffic/trafficClient.
 * test.ts without a socket or a browser DOM. `TrafficClient` below wraps this
 * with the actual WebSocket plumbing. */
export class TrafficClientCore {
  readonly net: TrafficNetGeom;
  readonly grid: CellGrid;
  /** id -> last-known kinematics (units decoded to metres / m/s / ticks). */
  readonly vehicles = new Map<number, VehKinematics>();
  /** Newest sim tick seen on any frame — the dead-reckoning "now". */
  serverTick = 0;
  /** Latest decoded FlowFrame (Task 11/12): edge id -> aggregate {count,
   * v (m/s, decoded from the 0.25 m/s wire unit)}. Replaced wholesale on every
   * flow frame (self-contained, no deltas — see traffic.proto FlowFrame). */
  flow = new Map<number, { count: number; v: number }>();

  private subscribed = new Set<number>();
  private coordChecked = false;

  constructor(lanes: RawLane[], net: TrafficNetGeom) {
    this.net = net;
    this.grid = CellGrid.build(lanes);
  }

  /** The current subscription set (read-only view for tests/callers). */
  get subscribedCells(): ReadonlySet<number> {
    return this.subscribed;
  }

  /** Recompute the desired subscription (3×3 around the camera target),
   * unsubscribe cells outside the 5×5 hysteresis band, and evict any tracked
   * vehicle whose canonical cell (via `grid.cellOfLaneS` — the SAME path
   * `applyKeyframe` uses) has fallen outside that band. Without this, stale
   * vehicles from cells the camera panned away from linger forever (their
   * `departed` frames stop arriving once we unsubscribe), growing the table to
   * the 4096 cap over a long session and silently dropping new vehicles (Task
   * 9 review finding 3). O(vehicles) — fine at the ~2 Hz throttle this is
   * called at. Returns the `{ subscribe, unsubscribe }` delta to send over the
   * wire (empty arrays if nothing changed). */
  updateCamera(targetX: number, targetZ: number): { subscribe: number[]; unsubscribe: number[] } {
    const want = this.grid.cellsAround(targetX, targetZ, 1); // 3×3
    const keep = this.grid.cellsAround(targetX, targetZ, 2); // 5×5 hysteresis band

    const add: number[] = [];
    for (const c of want) if (!this.subscribed.has(c)) add.push(c);

    const remove: number[] = [];
    for (const c of this.subscribed) if (!keep.has(c)) remove.push(c);

    for (const c of add) this.subscribed.add(c);
    for (const c of remove) this.subscribed.delete(c);

    // Evict vehicles now outside the 5×5 band, regardless of subscription
    // churn this call — a vehicle can drift out of band via dead-reckoning
    // alone between camera moves.
    for (const [id, veh] of this.vehicles) {
      const cell = this.grid.cellOfLaneS(veh.lane, veh.s);
      if (!keep.has(cell)) this.vehicles.delete(id);
    }

    return { subscribe: add, unsubscribe: remove };
  }

  /** Apply one decoded server frame (see traffic.proto CellFrame semantics in
   * the module banner). */
  applyFrame(frame: {
    cell: number;
    tick: bigint | number;
    keyframe: boolean;
    vehicles: readonly WireVehicle[];
    departed: readonly number[];
  }): void {
    const tick = Number(frame.tick);
    if (tick > this.serverTick) this.serverTick = tick;

    if (frame.keyframe) {
      // A keyframe is the FULL membership of its cell. Upsert every listed
      // vehicle, then evict any tracked vehicle that currently resolves (via
      // the canonical grid.cellOfLaneS) to this cell but was not in the
      // keyframe (ghost heal).
      this.applyKeyframe(frame.cell, frame.vehicles, tick);
    } else {
      for (const v of frame.vehicles) this.upsert(v, tick);
      for (const id of frame.departed) this.vehicles.delete(id);
    }

    this.maybeCoordCheck(frame);
  }

  /** Apply one decoded FlowFrame (Task 11/12 wire): replace the whole flow
   * picture (self-contained, no deltas — traffic.proto FlowFrame comment).
   * Decodes v_q (0.25 m/s units) to m/s, matching WireVehicle.vQ's decode in
   * `upsert` above. */
  applyFlowFrame(frame: { tick: bigint | number; edges: readonly WireFlowEdge[] }): void {
    const tick = Number(frame.tick);
    if (tick > this.serverTick) this.serverTick = tick;
    const next = new Map<number, { count: number; v: number }>();
    for (const e of frame.edges) next.set(e.edge, { count: e.count, v: e.vQ / 4 });
    this.flow = next;
  }

  private applyKeyframe(cell: number, vehicles: readonly WireVehicle[], tick: number): void {
    const present = new Set<number>();
    for (const v of vehicles) {
      this.upsert(v, tick);
      present.add(v.id);
    }
    // Evict stale members of this cell, resolved via the canonical
    // vertex-keyed breakpoints (grid.cellOfLaneS) — the same path
    // updateCamera's eviction uses, so the two can never disagree at a
    // cell border (Task 9 review finding 2).
    for (const [id, veh] of this.vehicles) {
      if (present.has(id)) continue;
      const resolvedCell = this.grid.cellOfLaneS(veh.lane, veh.s);
      if (resolvedCell === cell) this.vehicles.delete(id);
    }
  }

  private upsert(v: WireVehicle, tick: number): void {
    const next: VehKinematics = {
      lane: v.lane,
      s: v.sQ / 10, // decimetres -> metres
      v: v.vQ / 4, // 0.25 m/s units -> m/s
      tickAt: tick,
    };
    // Motion-continuity blend (FIX-C2): when a KNOWN vehicle (same wire id, so
    // NOT a recycled-slot generation change) is re-seated onto a different
    // lane, attach a lateral (parallel) or bezier-sweep (junction) blend so the
    // rendered pose eases across instead of teleporting. A brand-new id has no
    // prior entry → snaps, which is the correct teleport-heal behaviour.
    const prev = this.vehicles.get(v.id);
    this.vehicles.set(v.id, prev ? beginLaneChange(this.net, next, prev, tick) : next);
  }

  /** One-time dev sanity check (CLAUDE.md Phase-7a: coordinate mismatch is this
   * repo's most expensive bug class). Log a lane vertex so it can be eyeballed
   * against a known road vertex — trafficnet pts and road pts share the frame,
   * so the numbers should sit in the same range. */
  private maybeCoordCheck(frame: { vehicles: readonly WireVehicle[] }): void {
    if (this.coordChecked || frame.vehicles.length === 0) return;
    this.coordChecked = true;
    const v = frame.vehicles[0];
    const pts = this.net.pts.get(v.lane);
    if (pts && pts.length > 0) {
      // eslint-disable-next-line no-console
      console.info(
        `[traffic] coord-check: lane ${v.lane} vertex0 = [${pts[0][0].toFixed(1)}, ${pts[0][1].toFixed(1)}] ` +
          `(same world frame as roads/buildings — no transform applied)`,
      );
    }
  }
}

/** The live traffic client: wraps TrafficClientCore with the actual WebSocket
 * connection. carLayer reads `net` + `vehicles` + `serverTick` (delegated). */
export class TrafficClient {
  private readonly core: TrafficClientCore;

  private ws: WebSocket | null = null;
  private readonly url: string;
  private closed = false;

  private constructor(url: string, core: TrafficClientCore) {
    this.url = url;
    this.core = core;
  }

  get net(): TrafficNetGeom {
    return this.core.net;
  }

  get grid(): CellGrid {
    return this.core.grid;
  }

  /** The current subscription set (3×3 cells around the last camera target).
   * Used by flowLayer.ts to suppress impostors inside the AOI where real
   * cars already render (see the Task 12 exclusion invariant). */
  get subscribedCells(): ReadonlySet<number> {
    return this.core.subscribedCells;
  }

  get vehicles(): Map<number, VehKinematics> {
    return this.core.vehicles;
  }

  get serverTick(): number {
    return this.core.serverTick;
  }

  /** Latest decoded FlowFrame edge -> {count, v (m/s)} (Task 11/12). */
  get flow(): Map<number, { count: number; v: number }> {
    return this.core.flow;
  }

  /** Build the net + grid from the build-time-imported trafficnet.json and
   * open the socket. */
  static async connect(opts: { url?: string } = {}): Promise<TrafficClient> {
    const net = buildLaneNet(trafficNetDoc.lanes);
    const core = new TrafficClientCore(trafficNetDoc.lanes, net);
    const client = new TrafficClient(opts.url ?? DEFAULT_TRAFFIC_WS, core);
    client.open();
    return client;
  }

  private open(): void {
    const ws = new WebSocket(this.url);
    ws.binaryType = 'arraybuffer';
    this.ws = ws;
    ws.addEventListener('open', () => {
      // Re-send the current subscription set on (re)connect, plus subscribe
      // to the aggregate flow channel ALWAYS-ON (Task 12 deviation-log
      // decision: simpler than a zoom-gated toggle, and the far-LOD impostor
      // layer only renders it where visible — outside the subscribed AOI
      // minus the fade ring — so the always-on ~30 KB/s flow stream is spent
      // whether or not impostors are currently drawn).
      const subscribed = [...this.core.subscribedCells];
      this.send(subscribed, [], true);
    });
    ws.addEventListener('message', (ev) => this.onMessage(ev));
    ws.addEventListener('close', () => {
      this.ws = null;
      if (!this.closed) {
        // Simple reconnect: the subscription set is re-sent on the next open.
        setTimeout(() => { if (!this.closed) this.open(); }, 1000);
      }
    });
    ws.addEventListener('error', () => { /* close handler drives reconnect */ });
  }

  /** Recompute the desired subscription (3×3 around the camera target) and send
   * the delta; unsubscribe only cells outside the 5×5 neighbourhood (hysteresis
   * so a target hovering on a cell border doesn't thrash); evict vehicles that
   * fell outside the band. Call throttled (~2 Hz) from the render loop. */
  updateCamera(targetX: number, targetZ: number): void {
    const { subscribe, unsubscribe } = this.core.updateCamera(targetX, targetZ);
    if (subscribe.length === 0 && unsubscribe.length === 0) return;
    this.send(subscribe, unsubscribe);
  }

  private send(subscribe: number[], unsubscribe: number[], subscribeFlow?: boolean): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
    const msg = create(TrafficClientMsgSchema, {
      subscribeCells: subscribe,
      unsubscribeCells: unsubscribe,
      subscribeFlow,
    });
    this.ws.send(toBinary(TrafficClientMsgSchema, msg));
  }

  private onMessage(ev: MessageEvent): void {
    if (!(ev.data instanceof ArrayBuffer)) return;
    const server = fromBinary(TrafficServerMsgSchema, new Uint8Array(ev.data));
    for (const frame of server.cells) this.core.applyFrame(frame);
    if (server.flow) this.core.applyFlowFrame(server.flow);
  }

  close(): void {
    this.closed = true;
    this.ws?.close();
    this.ws = null;
  }
}

/** Minimal structural view of a decoded VehicleState (avoids importing the
 * generated type name at every call site). */
interface WireVehicle {
  id: number;
  lane: number;
  sQ: number;
  vQ: number;
}

/** Minimal structural view of a decoded FlowState (Task 11/12). */
interface WireFlowEdge {
  edge: number;
  count: number;
  vQ: number;
}
