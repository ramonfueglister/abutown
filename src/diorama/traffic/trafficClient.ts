// src/diorama/traffic/trafficClient.ts
//
// Browser WS client for the winterthur-traffic gateway (Task 8 wire contract).
//   * loads data/winterthur/trafficnet.json (the SAME asset the server bakes
//     from — single source of truth for lane polylines + the AOI grid);
//   * recomputes the AOI cell grid IDENTICALLY to the server so cell ids match
//     (see CellGrid below — replicates backend/crates/winterthur-traffic/
//     src/cells.rs `plate_bbox` + `CellGrid::build`, lines ~66-82/164-181;
//     an off-by-one here silently produces empty subscriptions);
//   * opens the WS, subscribes to the 3×3 cells around the camera target and
//     unsubscribes cells that leave the 5×5 neighbourhood (hysteresis);
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
import { buildLaneNet, type RawLane, type TrafficNetGeom, type VehKinematics } from './deadReckon';

/** Must equal `CELL_SIZE_M` in backend/crates/winterthur-traffic/src/cells.rs. */
export const CELL_SIZE_M = 128;

/** Default gateway endpoint; overridable via ?trafficWs=… */
export const DEFAULT_TRAFFIC_WS = 'ws://localhost:8790/traffic';

/** The raw trafficnet.json document shape (fields we consume). */
interface TrafficNetDoc {
  lanes: RawLane[];
}

/** Row-major AOI cell grid over the lane-geometry plate. A faithful port of the
 * server's CellGrid so `cell = row*cols + col` ids line up on the wire. */
export class CellGrid {
  readonly minX: number;
  readonly minZ: number;
  readonly cols: number;
  readonly rows: number;

  private constructor(minX: number, minZ: number, cols: number, rows: number) {
    this.minX = minX;
    this.minZ = minZ;
    this.cols = cols;
    this.rows = rows;
  }

  /** Build from the baked lanes. Mirrors `plate_bbox` (bbox over every lane
   * vertex, [x, z]) then `CellGrid::build`'s cols/rows derivation. */
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
    return new CellGrid(minX, minZ, cols, rows);
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

/** The live traffic client: WS connection, subscription state, and the
 * dead-reckoning vehicle table. carLayer reads `net` + `vehicles` + `serverTick`. */
export class TrafficClient {
  readonly net: TrafficNetGeom;
  readonly grid: CellGrid;
  /** id -> last-known kinematics (units decoded to metres / m/s / ticks). */
  readonly vehicles = new Map<number, VehKinematics>();
  /** Newest sim tick seen on any frame — the dead-reckoning "now". */
  serverTick = 0;

  private ws: WebSocket | null = null;
  private readonly url: string;
  private subscribed = new Set<number>();
  private coordChecked = false;
  private closed = false;

  private constructor(url: string, lanes: RawLane[], net: TrafficNetGeom) {
    this.url = url;
    this.net = net;
    this.grid = CellGrid.build(lanes);
  }

  /** Fetch trafficnet.json, build the net + grid, and open the socket. */
  static async connect(opts: { url?: string; netUrl?: string } = {}): Promise<TrafficClient> {
    const netUrl = opts.netUrl ?? 'data/winterthur/trafficnet.json';
    const res = await fetch(netUrl);
    if (!res.ok) throw new Error(`trafficnet fetch failed: ${res.status} ${netUrl}`);
    const doc = (await res.json()) as TrafficNetDoc;
    const net = buildLaneNet(doc.lanes);
    const client = new TrafficClient(opts.url ?? DEFAULT_TRAFFIC_WS, doc.lanes, net);
    client.open();
    return client;
  }

  private open(): void {
    const ws = new WebSocket(this.url);
    ws.binaryType = 'arraybuffer';
    this.ws = ws;
    ws.addEventListener('open', () => {
      // Re-send the current subscription set on (re)connect.
      if (this.subscribed.size > 0) this.send([...this.subscribed], []);
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
   * so a target hovering on a cell border doesn't thrash). Call throttled (~2 Hz)
   * from the render loop. */
  updateCamera(targetX: number, targetZ: number): void {
    const want = this.grid.cellsAround(targetX, targetZ, 1); // 3×3
    const keep = this.grid.cellsAround(targetX, targetZ, 2); // 5×5 hysteresis band

    const add: number[] = [];
    for (const c of want) if (!this.subscribed.has(c)) add.push(c);

    const remove: number[] = [];
    for (const c of this.subscribed) if (!keep.has(c)) remove.push(c);

    if (add.length === 0 && remove.length === 0) return;
    for (const c of add) this.subscribed.add(c);
    for (const c of remove) {
      this.subscribed.delete(c);
      // Drop vehicles that lived only in a now-unsubscribed cell? We cannot know
      // a vehicle's cell cheaply here (it would need pos_at), so we leave the
      // table as-is; dead-reckoning keeps them coasting until a future keyframe
      // or the render layer culls them by distance. Departed lists from still-
      // subscribed cells remain authoritative.
    }
    this.send(add, remove);
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
    for (const frame of server.cells) {
      const tick = Number(frame.tick);
      if (tick > this.serverTick) this.serverTick = tick;

      if (frame.keyframe) {
        // A keyframe is the FULL membership of its cell. Remove any prior member
        // of this cell that isn't present now (heals ghosts). We identify a
        // cell's members lazily: the frame's own vehicle list is authoritative,
        // so we simply upsert them and let departed/keyframe handling from other
        // cells manage the rest. To honour "keyframe replaces cell membership"
        // without tracking per-cell sets on the client, we drop stale members of
        // THIS cell by recomputing each current member's cell on ingest below.
        this.applyKeyframe(frame.cell, frame.vehicles, tick);
      } else {
        for (const v of frame.vehicles) this.upsert(v, tick);
        for (const id of frame.departed) this.vehicles.delete(id);
      }

      this.maybeCoordCheck(frame);
    }
  }

  /** Keyframe = full membership of `cell`. Upsert every listed vehicle, then
   * evict any tracked vehicle that currently resolves to this cell but was not
   * in the keyframe (ghost heal). Cell membership is recomputed from the current
   * dead-reckoned position via the grid — cheap and exact for the wire's cell
   * definition (cell_of_lane_s uses pos_at; we use the same grid on pos). */
  private applyKeyframe(cell: number, vehicles: readonly WireVehicle[], tick: number): void {
    const present = new Set<number>();
    for (const v of vehicles) {
      this.upsert(v, tick);
      present.add(v.id);
    }
    // Evict stale members of this cell.
    for (const [id, veh] of this.vehicles) {
      if (present.has(id)) continue;
      // Resolve the vehicle's cell from its last-known position.
      const p = this.resolveCell(veh);
      if (p === cell) this.vehicles.delete(id);
    }
  }

  private resolveCell(veh: VehKinematics): number {
    // Use the base position (no forward dead-reckon) to classify the cell, to
    // match how the server buckets by the vehicle's actual s at publish.
    const pts = this.net.pts.get(veh.lane);
    if (!pts) return -1;
    // Cheap: interpolate to veh.s via the grid's cellOf on the pos. We avoid a
    // full posAt import cycle by inlining a coarse lookup: clamp s and find the
    // vertex-nearest point. For cell classification the vertex approximation
    // matches the server's own vertex-keyed lane_segments approach.
    const lut = this.net.arcLut.get(veh.lane);
    if (!lut) return -1;
    const sClamped = Math.min(Math.max(veh.s, 0), lut[lut.length - 1]);
    let seg = 0;
    while (seg < lut.length - 2 && lut[seg + 1] < sClamped) seg++;
    const a = pts[seg];
    const b = pts[seg + 1];
    const segLen = lut[seg + 1] - lut[seg] || 1;
    const t = (sClamped - lut[seg]) / segLen;
    const x = a[0] + (b[0] - a[0]) * t;
    const z = a[1] + (b[1] - a[1]) * t;
    return this.grid.cellOf(x, z);
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
