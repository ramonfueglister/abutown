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

/** Must equal `CELL_SIZE_M` in backend/crates/winterthur-traffic/src/cells.rs. */
export const CELL_SIZE_M = 128;

/** Default gateway endpoint; overridable via ?trafficWs=… */
export const DEFAULT_TRAFFIC_WS = 'ws://localhost:8790/traffic';

/** The raw trafficnet.json document shape (fields we consume). */
interface TrafficNetDoc {
  lanes: RawLane[];
}

/** The statically-imported net document (see the module banner). Typed via a
 * cast, matching geoData.ts's `as { buildings: BakedBuilding[] }` pattern for
 * its JSON imports. */
const trafficNetDoc = trafficNetJson as unknown as TrafficNetDoc;

/** One `(sEnd, cell)` breakpoint along a lane: for arc positions `s` in
 * `(prevSEnd, sEnd]` the vehicle is in `cell`. Mirrors Rust's `LaneSegment`
 * (cells.rs lines ~32-36). The last segment's `sEnd` is `+Infinity` so any `s`
 * past the declared lane length still resolves. */
interface LaneSegment {
  sEnd: number;
  cell: number;
}

/** Row-major AOI cell grid over the lane-geometry plate. A faithful port of the
 * server's CellGrid so `cell = row*cols + col` ids line up on the wire, INCLUDING
 * the vertex-keyed per-lane `(sEnd, cell)` breakpoints that `cell_of_lane_s`
 * resolves against (cells.rs lines ~84-121/144-158). This is the ONE canonical
 * cell-classification path — used by keyframe ghost-healing and stale-vehicle
 * eviction alike (Task 9 review finding 2: two independent approximations
 * could disagree at a cell border and mis-evict). */
export class CellGrid {
  readonly minX: number;
  readonly minZ: number;
  readonly cols: number;
  readonly rows: number;
  /** lane id -> ordered breakpoints (same construction as cells.rs). */
  private readonly laneSegments: Map<number, LaneSegment[]>;

  private constructor(
    minX: number,
    minZ: number,
    cols: number,
    rows: number,
    laneSegments: Map<number, LaneSegment[]>,
  ) {
    this.minX = minX;
    this.minZ = minZ;
    this.cols = cols;
    this.rows = rows;
    this.laneSegments = laneSegments;
  }

  /** Build from the baked lanes. Mirrors `plate_bbox` (bbox over every lane
   * vertex, [x, z]) then `CellGrid::build`'s cols/rows derivation, and walks
   * each lane's vertices in arc order emitting a new breakpoint whenever the
   * cell changes — byte-for-byte the same rule as cells.rs `CellGrid::build`. */
  static build(lanes: RawLane[]): CellGrid {
    let minX = Infinity;
    let minZ = Infinity;
    let maxX = -Infinity;
    let maxZ = -Infinity;
    for (const lane of lanes) {
      for (const p of lane.pts) {
        if (p[0] < minX) minX = p[0];
        if (p[1] < minZ) minZ = p[1];
        if (p[0] > maxX) maxX = p[0];
        if (p[1] > maxZ) maxZ = p[1];
      }
    }
    if (!Number.isFinite(minX)) {
      // Empty net fallback — matches the Rust unit-box fallback.
      minX = 0;
      minZ = 0;
      maxX = 1;
      maxZ = 1;
    }
    const cols = Math.floor((maxX - minX) / CELL_SIZE_M) + 1;
    const rows = Math.floor((maxZ - minZ) / CELL_SIZE_M) + 1;

    const cellOfXZ = (x: number, z: number): number => {
      const col = clamp(Math.floor((x - minX) / CELL_SIZE_M), 0, cols - 1);
      const row = clamp(Math.floor((z - minZ) / CELL_SIZE_M), 0, rows - 1);
      return row * cols + col;
    };

    const laneSegments = new Map<number, LaneSegment[]>();
    for (const lane of lanes) {
      const segs: LaneSegment[] = [];
      let acc = 0;
      let curCell = cellOfXZ(lane.pts[0][0], lane.pts[0][1]);
      for (let i = 1; i < lane.pts.length; i++) {
        const a = lane.pts[i - 1];
        const b = lane.pts[i];
        const dx = b[0] - a[0];
        const dz = b[1] - a[1];
        acc += Math.sqrt(dx * dx + dz * dz);
        const c = cellOfXZ(b[0], b[1]);
        if (c !== curCell) {
          segs.push({ sEnd: acc, cell: curCell });
          curCell = c;
        }
      }
      // Final run extends to +inf so any s past the declared length still
      // resolves to the lane's terminal cell.
      segs.push({ sEnd: Infinity, cell: curCell });
      laneSegments.set(lane.id, segs);
    }

    return new CellGrid(minX, minZ, cols, rows, laneSegments);
  }

  get cellCount(): number {
    return this.cols * this.rows;
  }

  /** Row-major cell id for a world (x, z). Clamps to the grid (matches Rust). */
  cellOf(x: number, z: number): number {
    const col = clamp(Math.floor((x - this.minX) / CELL_SIZE_M), 0, this.cols - 1);
    const row = clamp(Math.floor((z - this.minZ) / CELL_SIZE_M), 0, this.rows - 1);
    return row * this.cols + col;
  }

  /** The cell a vehicle at arc position `s` on `lane` (a lane id) occupies.
   * Direct port of `cells.rs::cell_of_lane_s`: a short linear scan over the
   * lane's precomputed breakpoints, no position interpolation. Returns -1 for
   * an unknown lane id (never happens for live vehicles). */
  cellOfLaneS(lane: number, s: number): number {
    const segs = this.laneSegments.get(lane);
    if (!segs) return -1;
    for (const seg of segs) {
      if (s <= seg.sEnd) return seg.cell;
    }
    // Unreachable — the last breakpoint's sEnd is +Infinity.
    return segs[segs.length - 1]?.cell ?? -1;
  }

  /** The (col, row) of a world position, clamped to the grid. */
  colRowOf(x: number, z: number): { col: number; row: number } {
    const col = clamp(Math.floor((x - this.minX) / CELL_SIZE_M), 0, this.cols - 1);
    const row = clamp(Math.floor((z - this.minZ) / CELL_SIZE_M), 0, this.rows - 1);
    return { col, row };
  }

  /** All valid cell ids within ±radius cells of the cell containing (x, z)
   * (a (2r+1)×(2r+1) block, clipped to the grid). radius 1 => 3×3, 2 => 5×5. */
  cellsAround(x: number, z: number, radius: number): Set<number> {
    const { col, row } = this.colRowOf(x, z);
    const out = new Set<number>();
    for (let dr = -radius; dr <= radius; dr++) {
      const r = row + dr;
      if (r < 0 || r >= this.rows) continue;
      for (let dc = -radius; dc <= radius; dc++) {
        const c = col + dc;
        if (c < 0 || c >= this.cols) continue;
        out.add(r * this.cols + c);
      }
    }
    return out;
  }
}

function clamp(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
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
    this.vehicles.set(v.id, {
      lane: v.lane,
      s: v.sQ / 10, // decimetres -> metres
      v: v.vQ / 4, // 0.25 m/s units -> m/s
      tickAt: tick,
    });
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

  get vehicles(): Map<number, VehKinematics> {
    return this.core.vehicles;
  }

  get serverTick(): number {
    return this.core.serverTick;
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
      // Re-send the current subscription set on (re)connect.
      const subscribed = [...this.core.subscribedCells];
      if (subscribed.length > 0) this.send(subscribed, []);
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

  private send(subscribe: number[], unsubscribe: number[]): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
    const msg = create(TrafficClientMsgSchema, {
      subscribeCells: subscribe,
      unsubscribeCells: unsubscribe,
    });
    this.ws.send(toBinary(TrafficClientMsgSchema, msg));
  }

  private onMessage(ev: MessageEvent): void {
    if (!(ev.data instanceof ArrayBuffer)) return;
    const server = fromBinary(TrafficServerMsgSchema, new Uint8Array(ev.data));
    for (const frame of server.cells) this.core.applyFrame(frame);
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
