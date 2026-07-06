// src/diorama/ksw/geo/corridorMask.ts
//
// Runtime decoder for the bake's corridor mask (Task 5e, spec §5
// "Terrain-discard"). The bake (scripts/geo/lib/corridormask.mjs) rasterizes
// every road/rail corridor into a packed 1-bit-per-cell world-space raster and
// writes it to data/winterthur/world/mask.bin. The terrain fragment shader
// samples it and DISCARDS fragments inside corridors so rendered terrain never
// pierces a road surface (ribbon skirts close the hole). This module fetches +
// decodes that file and exposes it both as a CPU reader (`covers`) and as a GPU
// DataTexture (`corridorMaskDataTexture`) for the shader.
//
// ── MIRROR of scripts/geo/lib/corridormask.mjs ─────────────────────────────
// The header layout (magic/version/originX/originZ/cellSizeM/cols/rows +
// packed little-endian bitfield) MUST match the encoder byte-for-byte. The
// parity test (tests/diorama/corridorMask.test.ts) encodes with the bake lib
// and decodes here to keep the two locked together. No fallback: a magic or
// version mismatch is a hard error (a mismatched/absent mask must surface
// loudly, not silently skip the discard).
import * as THREE from 'three/webgpu';

const MAGIC = 0x434d5330; // "CMS0"
const HEADER_BYTES = 4 + 4 + 8 + 8 + 4 + 4 + 4;
const VERSION = 1;

export interface CorridorMask {
  originX: number;
  originZ: number;
  cellSizeM: number;
  cols: number;
  rows: number;
  bits: Uint8Array;
  /** true iff the corridor cell covering world (x,z) is set (nearest cell). */
  covers(x: number, z: number): boolean;
}

/** Decode a mask.bin buffer. Hard-errors on magic/version/length mismatch. */
export function decodeCorridorMask(bin: Uint8Array | ArrayBuffer): CorridorMask {
  const u8 = bin instanceof Uint8Array ? bin : new Uint8Array(bin);
  if (u8.byteLength < HEADER_BYTES) {
    throw new Error(`decodeCorridorMask: buffer too small (${u8.byteLength} < ${HEADER_BYTES})`);
  }
  const dv = new DataView(u8.buffer, u8.byteOffset, u8.byteLength);
  let o = 0;
  const magic = dv.getUint32(o, true); o += 4;
  if (magic !== MAGIC) throw new Error(`decodeCorridorMask: bad magic 0x${magic.toString(16)} (expected 0x${MAGIC.toString(16)})`);
  const version = dv.getUint32(o, true); o += 4;
  if (version !== VERSION) throw new Error(`decodeCorridorMask: unsupported version ${version} (expected ${VERSION})`);
  const originX = dv.getFloat64(o, true); o += 8;
  const originZ = dv.getFloat64(o, true); o += 8;
  const cellSizeM = dv.getFloat32(o, true); o += 4;
  const cols = dv.getUint32(o, true); o += 4;
  const rows = dv.getUint32(o, true); o += 4;
  const expectBytes = Math.ceil((cols * rows) / 8);
  const bits = u8.subarray(HEADER_BYTES, HEADER_BYTES + expectBytes);
  if (bits.length !== expectBytes) {
    throw new Error(`decodeCorridorMask: truncated bitfield (${bits.length} < ${expectBytes} for ${cols}×${rows})`);
  }
  const bitsCopy = new Uint8Array(bits);
  return {
    originX, originZ, cellSizeM, cols, rows, bits: bitsCopy,
    covers(x: number, z: number): boolean {
      const i = Math.round((x - originX) / cellSizeM);
      const j = Math.round((z - originZ) / cellSizeM);
      if (i < 0 || j < 0 || i >= cols || j >= rows) return false;
      const n = j * cols + i;
      return (bitsCopy[n >> 3] & (1 << (n & 7))) !== 0;
    },
  };
}

/**
 * Expand the packed mask into a single-channel (R8, red) DataTexture, one texel
 * per cell (255 = corridor, 0 = open terrain), with NEAREST filtering so the
 * shader reads the exact cell without interpolating a soft edge. `flipY` is
 * disabled so texel (i,j) maps to cell (i,j) directly (the shader computes the
 * uv from world coords, not three's default bottom-up convention).
 */
export function corridorMaskDataTexture(mask: CorridorMask): THREE.DataTexture {
  const { cols, rows, bits } = mask;
  const data = new Uint8Array(cols * rows);
  for (let n = 0; n < cols * rows; n++) {
    data[n] = (bits[n >> 3] & (1 << (n & 7))) !== 0 ? 255 : 0;
  }
  const tex = new THREE.DataTexture(data, cols, rows, THREE.RedFormat, THREE.UnsignedByteType);
  tex.magFilter = THREE.NearestFilter;
  tex.minFilter = THREE.NearestFilter;
  tex.wrapS = THREE.ClampToEdgeWrapping;
  tex.wrapT = THREE.ClampToEdgeWrapping;
  tex.flipY = false;
  tex.needsUpdate = true;
  return tex;
}

/** Fetch + decode mask.bin from the world artifact base URL. Hard-errors (no
 * fallback) if the file is missing/unreadable — a corridor mask that fails to
 * load must stop the boot loudly, not silently render terrain through roads. */
export async function loadCorridorMask(baseUrl = '/winterthur-world/'): Promise<CorridorMask> {
  const url = `${baseUrl}mask.bin`;
  const res = await fetch(url);
  if (!res.ok) {
    throw new Error(`loadCorridorMask: failed to fetch ${url}: ${res.status} ${res.statusText} — re-run the bake (writes mask.bin) and ensure the public/winterthur-world symlink exists`);
  }
  return decodeCorridorMask(new Uint8Array(await res.arrayBuffer()));
}
