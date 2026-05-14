import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, dirname, join } from 'node:path';
import { inflateSync } from 'node:zlib';
import net from 'node:net';

const CONTENT_HOST = 'content.openttd.org';
const CONTENT_PORT = 3978;
const PACKET_CONTENT_CLIENT_CONTENT = 5;
const PACKET_CONTENT_SERVER_CONTENT = 6;
const CONTENT_TYPE_SCENARIO = 5;
const CONTENT_TYPE_HEIGHTMAP = 6;
const SEND_TCP_COMPAT_MTU = 1460;

const TILE_CLEAR = 0;
const TILE_RAIL = 1;
const TILE_ROAD = 2;
const TILE_HOUSE = 3;
const TILE_TREES = 4;
const TILE_STATION = 5;
const TILE_WATER = 6;
const TILE_VOID = 7;
const TILE_INDUSTRY = 8;
const TILE_TUNNEL_BRIDGE = 9;
const TILE_OBJECT = 10;

const NORTH = 1;
const EAST = 2;
const SOUTH = 4;
const WEST = 8;

const DEFAULT_TERRAIN_KINDS = ['grass', 'water', 'riverbank', 'forest'];
const BUILDING_SHEETS = ['houses', 'oldhouses', 'cottages', 'townhouses', 'shops', 'flats', 'office', 'modern', 'tower', 'church'];
const DETAIL_CATEGORIES = ['tree', 'park', 'civic', 'industry', 'decor', 'station', 'dock', 'quai', 'field', 'yard'];
const DETAIL_ASSETS = ['industry', 'decor', 'station-roof', 'road-stop', 'rail-depot', 'road-depot', 'ship', 'dock', 'quay', 'factory', 'farm-field'];

export function normalizeOpenTtdMap(input) {
  const targetSize = input.targetSize ?? 512;
  const sourceWidth = input.sourceWidth;
  const sourceHeight = input.sourceHeight;
  const sampledTypes = new Uint8Array(targetSize * targetSize);
  const blockCounts = [];

  for (let y = 0; y < targetSize; y += 1) {
    for (let x = 0; x < targetSize; x += 1) {
      const counts = sampleBlockCounts(input.tileTypes, sourceWidth, sourceHeight, targetSize, x, y);
      blockCounts[y * targetSize + x] = counts;
      sampledTypes[y * targetSize + x] = blockTypeFromCounts(counts);
    }
  }

  const terrain = new Uint8Array(targetSize * targetSize);
  const roads = [];
  const rails = [];
  const buildings = [];
  const trees = [];
  const details = [];
  const roadKeys = new Set();
  const railKeys = new Set();
  const occupied = new Set();
  let sourceTreeTiles = 0;

  for (let y = 0; y < targetSize; y += 1) {
    for (let x = 0; x < targetSize; x += 1) {
      const tileType = sampledTypes[y * targetSize + x];
      const index = y * targetSize + x;
      terrain[index] = terrainKindIndex(tileType);
      if (tileType === TILE_ROAD || tileType === TILE_TUNNEL_BRIDGE) roadKeys.add(key(x, y));
      if (tileType === TILE_RAIL) railKeys.add(key(x, y));
    }
  }

  markRiverbanks(terrain, targetSize);

  for (const tileKey of [...roadKeys].sort(compareKeys)) {
    const [x, y] = parseKey(tileKey);
    const type = sampledTypes[y * targetSize + x];
    roads.push([x, y, maskFor(roadKeys, x, y), type === TILE_TUNNEL_BRIDGE ? 1 : 0]);
  }

  for (const tileKey of [...railKeys].sort(compareKeys)) {
    const [x, y] = parseKey(tileKey);
    rails.push([x, y, maskFor(railKeys, x, y)]);
  }

  for (const tileKey of [...roadKeys, ...railKeys]) occupied.add(tileKey);
  const detailKeys = new Set();
  preserveSourceCityObjects(input.tileTypes, sourceWidth, sourceHeight, targetSize, occupied, detailKeys, terrain, buildings, details);

  for (let y = 0; y < targetSize; y += 1) {
    for (let x = 0; x < targetSize; x += 1) {
      const counts = blockCounts[y * targetSize + x];
      const treeCount = counts.get(TILE_TREES) ?? 0;
      sourceTreeTiles += treeCount;
      if (treeCount > 0) {
        const coord = findFreePlacement({ x, y }, targetSize, occupied, terrain, ['forest', 'grass'], 1);
        if (coord) {
          trees.push([coord.x, coord.y]);
          occupied.add(key(coord.x, coord.y));
        }
      }
    }
  }

  if (sourceTreeTiles === 0) {
    fillFallbackForests(targetSize, occupied, terrain, trees);
  }

  return {
    id: input.id,
    source: input.source ?? '',
    sourceWidth,
    sourceHeight,
    width: targetSize,
    height: targetSize,
    terrainKinds: [...DEFAULT_TERRAIN_KINDS],
    buildingSheets: [...BUILDING_SHEETS],
    detailCategories: [...DETAIL_CATEGORIES],
    detailAssets: [...DETAIL_ASSETS],
    terrainRle: encodeRle(terrain),
    roads,
    rails,
    buildings: buildings.sort(compareCoordTuples),
    trees: trees.sort(compareCoordTuples),
    details: details.sort(compareCoordTuples),
  };
}

function fillFallbackForests(targetSize, occupied, terrain, trees) {
  const grass = DEFAULT_TERRAIN_KINDS.indexOf('grass');
  const riverbank = DEFAULT_TERRAIN_KINDS.indexOf('riverbank');
  for (let y = 0; y < targetSize; y += 1) {
    for (let x = 0; x < targetSize; x += 1) {
      const tileKey = key(x, y);
      if (occupied.has(tileKey)) continue;
      const terrainKind = terrain[y * targetSize + x];
      if (terrainKind !== grass && terrainKind !== riverbank) continue;
      const cellX = Math.floor(x / 16);
      const cellY = Math.floor(y / 16);
      if (hash(`fallback-forest-cell:${cellX}:${cellY}`) % 5 !== 0) continue;
      if (hash(`fallback-forest-tile:${x}:${y}`) % 4 !== 0) continue;
      trees.push([x, y]);
      occupied.add(tileKey);
    }
  }
}

function preserveSourceCityObjects(tileTypes, sourceWidth, sourceHeight, targetSize, occupied, detailKeys, terrain, buildings, details) {
  for (let sourceY = 0; sourceY < sourceHeight; sourceY += 1) {
    const row = sourceY * sourceWidth;
    for (let sourceX = 0; sourceX < sourceWidth; sourceX += 1) {
      const tileType = tileTypes[row + sourceX];
      if (tileType !== TILE_HOUSE && tileType !== TILE_INDUSTRY && tileType !== TILE_STATION && tileType !== TILE_OBJECT) continue;
      const target = {
        x: Math.floor((sourceX * targetSize) / sourceWidth),
        y: Math.floor((sourceY * targetSize) / sourceHeight),
      };

      if (tileType === TILE_HOUSE) {
        const coord = findFreePlacement(
          target,
          targetSize,
          occupied,
          terrain,
          ['grass'],
          8,
          (candidate) => hasWaterClearance(candidate, targetSize, terrain, 2)
        );
        if (!coord) continue;
        buildings.push([coord.x, coord.y, buildingSheetIndex(coord.x, coord.y), frameFor(coord.x, coord.y)]);
        occupied.add(key(coord.x, coord.y));
        continue;
      }

      const detail = detailForTileType(tileType, tileTypes, sourceWidth, sourceHeight, sourceX, sourceY);
      if (!detail) continue;
      const coord = findFreePlacement(target, targetSize, detailKeys, terrain, detail.allowedTerrainKinds, 3);
      if (!coord) continue;
      details.push([coord.x, coord.y, detail.category, detail.asset]);
      detailKeys.add(key(coord.x, coord.y));
    }
  }
}

function detailForTileType(tileType, tileTypes, sourceWidth, sourceHeight, sourceX, sourceY) {
  if (tileType === TILE_INDUSTRY) {
    return {
      category: DETAIL_CATEGORIES.indexOf('industry'),
      asset: DETAIL_ASSETS.indexOf('factory'),
      allowedTerrainKinds: ['grass', 'riverbank'],
    };
  }
  if (tileType === TILE_STATION) {
    return {
      category: DETAIL_CATEGORIES.indexOf('station'),
      asset: DETAIL_ASSETS.indexOf('station-roof'),
      allowedTerrainKinds: ['grass', 'riverbank'],
    };
  }
  if (tileType === TILE_OBJECT) {
    if (hasSourceNeighbor(tileTypes, sourceWidth, sourceHeight, sourceX, sourceY, [TILE_WATER], 2)) {
      const waterAssets = ['ship', 'dock', 'quay'];
      return {
        category: DETAIL_CATEGORIES.indexOf('dock'),
        asset: DETAIL_ASSETS.indexOf(waterAssets[hash(`${sourceX}:${sourceY}:water-object`) % waterAssets.length]),
        allowedTerrainKinds: ['water', 'riverbank', 'grass'],
      };
    }
    if (hasSourceNeighbor(tileTypes, sourceWidth, sourceHeight, sourceX, sourceY, [TILE_ROAD, TILE_TUNNEL_BRIDGE], 2)) {
      const roadDetails = [
        { category: 'station', asset: 'road-stop' },
        { category: 'yard', asset: 'road-depot' },
      ];
      const selected = roadDetails[hash(`${sourceX}:${sourceY}:road-object`) % roadDetails.length];
      return {
        category: DETAIL_CATEGORIES.indexOf(selected.category),
        asset: DETAIL_ASSETS.indexOf(selected.asset),
        allowedTerrainKinds: ['grass', 'riverbank'],
      };
    }
    return {
      category: DETAIL_CATEGORIES.indexOf('decor'),
      asset: DETAIL_ASSETS.indexOf('decor'),
      allowedTerrainKinds: ['grass', 'riverbank', 'forest'],
    };
  }
  return undefined;
}

function hasSourceNeighbor(tileTypes, sourceWidth, sourceHeight, sourceX, sourceY, wantedTypes, radius) {
  const wanted = new Set(wantedTypes);
  for (let y = Math.max(0, sourceY - radius); y <= Math.min(sourceHeight - 1, sourceY + radius); y += 1) {
    const row = y * sourceWidth;
    for (let x = Math.max(0, sourceX - radius); x <= Math.min(sourceWidth - 1, sourceX + radius); x += 1) {
      if (x === sourceX && y === sourceY) continue;
      if (wanted.has(tileTypes[row + x])) return true;
    }
  }
  return false;
}

function hasWaterClearance(coord, size, terrain, radius) {
  const water = DEFAULT_TERRAIN_KINDS.indexOf('water');
  for (let y = Math.max(0, coord.y - radius); y <= Math.min(size - 1, coord.y + radius); y += 1) {
    for (let x = Math.max(0, coord.x - radius); x <= Math.min(size - 1, coord.x + radius); x += 1) {
      if (terrain[y * size + x] === water) return false;
    }
  }
  return true;
}

export function decodeOpenTtdSavegame(filePath) {
  const buffer = readFileSync(filePath);
  const compression = buffer.subarray(0, 4).toString('ascii');
  const savegameVersion = buffer.readUInt16BE(4);
  const payload = buffer.subarray(8);
  const data = decompressOpenTtdPayload(compression, payload, filePath);
  const chunks = extractChunks(data);
  const maps = chunks.MAPS?.records?.[0];
  if (!maps) throw new Error(`Missing MAPS chunk in ${filePath}`);
  const sourceWidth = maps.dim_x;
  const sourceHeight = maps.dim_y;
  const tileBytes = chunks.MAPT?.data;
  if (!tileBytes) throw new Error(`Missing MAPT chunk in ${filePath}`);
  if (tileBytes.length !== sourceWidth * sourceHeight) {
    throw new Error(`MAPT size ${tileBytes.length} does not match ${sourceWidth}x${sourceHeight}`);
  }
  return {
    savegameVersion,
    sourceWidth,
    sourceHeight,
    tileTypes: Uint8Array.from(tileBytes, (value) => value >> 4),
  };
}

export async function downloadBananasContent(contentId, outputDir) {
  mkdirSync(outputDir, { recursive: true });
  const packet = makeClientContentPacket(contentId);
  const response = await requestContent(packet);
  const archivePath = join(outputDir, `${response.filename}.tar.gz`);
  writeFileSync(archivePath, response.body);
  const extractDir = join(outputDir, response.filename);
  mkdirSync(extractDir, { recursive: true });
  const extract = spawnSync('tar', ['-xzf', archivePath, '-C', extractDir], { encoding: 'utf8' });
  if (extract.status !== 0) throw new Error(`tar extraction failed: ${extract.stderr}`);
  const scenarioPath = findFirstScenario(extractDir);
  return { archivePath, extractDir, scenarioPath, contentType: response.contentType, contentId: response.contentId };
}

export function generateTypeScriptModule(map, exportName = 'openTtdImportedMap') {
  return [
    '/* This file is generated by scripts/import-openttd-map.mjs. Do not edit by hand. */',
    '',
    `export const ${exportName} = JSON.parse(String.raw\`${JSON.stringify(map)}\`);`,
    '',
  ].join('\n');
}

export function writeTypeScriptModule(map, outputPath, exportName = 'openTtdImportedMap') {
  mkdirSync(dirname(outputPath), { recursive: true });
  writeFileSync(outputPath, generateTypeScriptModule(map, exportName));
}

function sampleBlockCounts(tileTypes, sourceWidth, sourceHeight, targetSize, targetX, targetY) {
  const startX = Math.floor((targetX * sourceWidth) / targetSize);
  const endX = Math.max(startX + 1, Math.floor(((targetX + 1) * sourceWidth) / targetSize));
  const startY = Math.floor((targetY * sourceHeight) / targetSize);
  const endY = Math.max(startY + 1, Math.floor(((targetY + 1) * sourceHeight) / targetSize));
  const counts = new Map();

  for (let y = startY; y < endY; y += 1) {
    const row = y * sourceWidth;
    for (let x = startX; x < endX; x += 1) {
      const tileType = tileTypes[row + x];
      counts.set(tileType, (counts.get(tileType) ?? 0) + 1);
    }
  }
  return counts;
}

function blockTypeFromCounts(counts) {
  for (const priority of [TILE_WATER, TILE_RAIL, TILE_ROAD, TILE_TUNNEL_BRIDGE, TILE_STATION, TILE_HOUSE, TILE_INDUSTRY, TILE_OBJECT, TILE_TREES]) {
    if ((counts.get(priority) ?? 0) > 0) return priority;
  }
  return TILE_CLEAR;
}

function findFreePlacement(origin, size, occupied, terrain, allowedTerrainKinds, radius = 1, isSafe = () => true) {
  const allowed = new Set(allowedTerrainKinds.map((kind) => DEFAULT_TERRAIN_KINDS.indexOf(kind)));
  for (const offset of placementOffsets(radius)) {
    const x = origin.x + offset.x;
    const y = origin.y + offset.y;
    if (x < 0 || y < 0 || x >= size || y >= size) continue;
    if (occupied.has(key(x, y))) continue;
    if (!allowed.has(terrain[y * size + x])) continue;
    if (!isSafe({ x, y })) continue;
    return { x, y };
  }
  return undefined;
}

function placementOffsets(radius) {
  const offsets = [{ x: 0, y: 0 }];
  for (let distance = 1; distance <= radius; distance += 1) {
    for (let y = -distance; y <= distance; y += 1) {
      for (let x = -distance; x <= distance; x += 1) {
        if (Math.max(Math.abs(x), Math.abs(y)) !== distance) continue;
        offsets.push({ x, y });
      }
    }
  }
  return offsets;
}

function terrainKindIndex(tileType) {
  if (tileType === TILE_WATER) return DEFAULT_TERRAIN_KINDS.indexOf('water');
  if (tileType === TILE_TREES) return DEFAULT_TERRAIN_KINDS.indexOf('forest');
  return DEFAULT_TERRAIN_KINDS.indexOf('grass');
}

function markRiverbanks(terrain, size) {
  const water = DEFAULT_TERRAIN_KINDS.indexOf('water');
  const grass = DEFAULT_TERRAIN_KINDS.indexOf('grass');
  const riverbank = DEFAULT_TERRAIN_KINDS.indexOf('riverbank');
  const updates = [];
  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      const index = y * size + x;
      if (terrain[index] !== grass) continue;
      if (terrain[(y - 1) * size + x] === water || terrain[(y + 1) * size + x] === water || terrain[y * size + x - 1] === water || terrain[y * size + x + 1] === water) {
        updates.push(index);
      }
    }
  }
  for (const index of updates) terrain[index] = riverbank;
}

function encodeRle(values) {
  const result = [];
  if (values.length === 0) return result;
  let current = values[0];
  let length = 1;
  for (let index = 1; index < values.length; index += 1) {
    if (values[index] === current) {
      length += 1;
    } else {
      result.push([current, length]);
      current = values[index];
      length = 1;
    }
  }
  result.push([current, length]);
  return result;
}

function maskFor(keys, x, y) {
  return (
    (keys.has(key(x, y - 1)) ? NORTH : 0) |
    (keys.has(key(x + 1, y)) ? EAST : 0) |
    (keys.has(key(x, y + 1)) ? SOUTH : 0) |
    (keys.has(key(x - 1, y)) ? WEST : 0)
  );
}

function buildingSheetIndex(x, y) {
  const sheet = x % 11 < 2 || y % 13 < 2 ? 'oldhouses' : (x + y) % 9 < 3 ? 'townhouses' : (x + y) % 9 < 6 ? 'shops' : 'houses';
  return BUILDING_SHEETS.indexOf(sheet);
}

function frameFor(x, y) {
  return hash(`${x}:${y}`) % 4;
}

function hash(value) {
  return createHash('sha1').update(value).digest()[0];
}

function key(x, y) {
  return `${x}:${y}`;
}

function parseKey(tileKey) {
  return tileKey.split(':').map(Number);
}

function compareKeys(a, b) {
  const [ax, ay] = parseKey(a);
  const [bx, by] = parseKey(b);
  return ay - by || ax - bx;
}

function compareCoordTuples(a, b) {
  return a[1] - b[1] || a[0] - b[0];
}

function decompressOpenTtdPayload(compression, payload, filePath) {
  if (compression === 'OTTN') return payload;
  if (compression === 'OTTZ') return inflateSync(payload);
  if (compression === 'OTTX') {
    const result = spawnSync('xz', ['-dc'], { input: payload, encoding: 'buffer', maxBuffer: 512 * 1024 * 1024 });
    if (result.status !== 0) throw new Error(`xz failed: ${result.stderr.toString()}`);
    return result.stdout;
  }
  throw new Error(`Unsupported OpenTTD savegame compression ${compression}`);
}

function extractChunks(data) {
  let offset = 0;
  const chunks = {};
  while (data.subarray(offset, offset + 4).toString('ascii') !== '\0\0\0\0') {
    const tag = data.subarray(offset, offset + 4).toString('ascii');
    offset += 4;
    const marker = data[offset];
    offset += 1;
    const chunkType = marker & 0xf;
    if (chunkType === 0) {
      const size = ((marker >> 4) << 24) | data.readUIntBE(offset, 3);
      offset += 3;
      chunks[tag] = { data: data.subarray(offset, offset + size) };
      offset += size;
    } else if (chunkType === 1 || chunkType === 2) {
      const parts = [];
      while (true) {
        const sizeResult = readGamma(data, offset);
        offset = sizeResult.offset;
        if (sizeResult.value === 0) break;
        const size = sizeResult.value - 1;
        parts.push(data.subarray(offset, offset + size));
        offset += size;
      }
      chunks[tag] = { data: Buffer.concat(parts) };
    } else if (chunkType === 3 || chunkType === 4) {
      const headerSizeResult = readGamma(data, offset);
      offset = headerSizeResult.offset;
      const headerStart = offset;
      const headers = readTableHeaders(data, offset);
      offset = headerStart + headerSizeResult.value - 1;
      const records = [];
      while (true) {
        const sizeResult = readGamma(data, offset);
        offset = sizeResult.offset;
        if (sizeResult.value === 0) break;
        const recordStart = offset;
        if (chunkType === 4) {
          const indexResult = readGamma(data, offset);
          offset = indexResult.offset;
        }
        const size = sizeResult.value - 1 - (offset - recordStart);
        if (tag === 'MAPS') {
          const record = readTableRecord(data, offset, headers);
          records.push(record.value);
        }
        offset += size;
      }
      chunks[tag] = { headers, records };
    } else {
      throw new Error(`Unsupported chunk type ${chunkType} for ${tag}`);
    }
  }
  return chunks;
}

function readTableHeaders(data, offset) {
  const root = [];
  while (data[offset] !== 0) {
    const type = data[offset];
    offset += 1;
    const nameResult = readGammaString(data, offset);
    offset = nameResult.offset;
    root.push({ type: type & 0xf, hasLength: Boolean(type & 0x10), name: nameResult.value });
  }
  return { root };
}

function readTableRecord(data, offset, headers) {
  const record = {};
  for (const field of headers.root) {
    const valueResult = readField(data, offset, field);
    offset = valueResult.offset;
    record[field.name] = valueResult.value;
  }
  return { value: record, offset };
}

function readField(data, offset, field) {
  if (field.hasLength && field.type !== 10) {
    const lengthResult = readGamma(data, offset);
    offset = lengthResult.offset;
    const values = [];
    for (let index = 0; index < lengthResult.value; index += 1) {
      const valueResult = readSingleField(data, offset, field.type);
      offset = valueResult.offset;
      values.push(valueResult.value);
    }
    return { value: values, offset };
  }
  return readSingleField(data, offset, field.type);
}

function readSingleField(data, offset, type) {
  if (type === 1) return { value: data.readInt8(offset), offset: offset + 1 };
  if (type === 2) return { value: data.readUInt8(offset), offset: offset + 1 };
  if (type === 3) return { value: data.readInt16BE(offset), offset: offset + 2 };
  if (type === 4) return { value: data.readUInt16BE(offset), offset: offset + 2 };
  if (type === 5) return { value: data.readInt32BE(offset), offset: offset + 4 };
  if (type === 6) return { value: data.readUInt32BE(offset), offset: offset + 4 };
  if (type === 10) return readGammaString(data, offset);
  throw new Error(`Unsupported table field type ${type}`);
}

function readGammaString(data, offset) {
  const lengthResult = readGamma(data, offset);
  offset = lengthResult.offset;
  const value = data.subarray(offset, offset + lengthResult.value).toString('utf8');
  return { value, offset: offset + lengthResult.value };
}

function readGamma(data, offset) {
  const byte = data[offset];
  offset += 1;
  if ((byte & 0x80) === 0) return { value: byte & 0x7f, offset };
  if ((byte & 0xc0) === 0x80) return { value: ((byte & 0x3f) << 8) | data[offset], offset: offset + 1 };
  if ((byte & 0xe0) === 0xc0) return { value: ((byte & 0x1f) << 16) | data.readUInt16BE(offset), offset: offset + 2 };
  if ((byte & 0xf0) === 0xe0) return { value: ((byte & 0x0f) << 24) | (data.readUInt16BE(offset) << 8) | data[offset + 2], offset: offset + 3 };
  if ((byte & 0xf8) === 0xf0) return { value: ((byte & 0x07) << 32) | data.readUInt32BE(offset), offset: offset + 4 };
  throw new Error('Invalid OpenTTD gamma value');
}

function makeClientContentPacket(contentId) {
  const packet = Buffer.alloc(2 + 1 + 2 + 4);
  packet.writeUInt16LE(packet.length, 0);
  packet.writeUInt8(PACKET_CONTENT_CLIENT_CONTENT, 2);
  packet.writeUInt16LE(1, 3);
  packet.writeUInt32LE(contentId, 5);
  if (packet.length > SEND_TCP_COMPAT_MTU) throw new Error('Content request packet exceeds MTU');
  return packet;
}

function requestContent(packet) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host: CONTENT_HOST, port: CONTENT_PORT });
    const chunks = [];
    let buffer = Buffer.alloc(0);
    let header = undefined;
    let settled = false;

    socket.setTimeout(30000);
    socket.on('connect', () => socket.write(packet));
    socket.on('timeout', () => {
      socket.destroy();
      reject(new Error('Timed out downloading OpenTTD content'));
    });
    socket.on('error', reject);
    socket.on('data', (data) => {
      buffer = Buffer.concat([buffer, data]);
      while (buffer.length >= 2) {
        const length = buffer.readUInt16LE(0);
        if (buffer.length < length) break;
        const packetData = buffer.subarray(0, length);
        buffer = buffer.subarray(length);
        if (packetData[2] !== PACKET_CONTENT_SERVER_CONTENT) continue;
        const payload = packetData.subarray(3);
        if (!header) {
          header = readContentHeader(payload);
          if (header.contentType !== CONTENT_TYPE_SCENARIO && header.contentType !== CONTENT_TYPE_HEIGHTMAP) {
            socket.destroy();
            reject(new Error(`Unexpected content type ${header.contentType}`));
            return;
          }
          if (header.body.length > 0) chunks.push(header.body);
        } else if (payload.length === 0) {
          const body = Buffer.concat(chunks);
          settled = true;
          socket.end();
          resolve({ ...header, body });
        } else {
          chunks.push(payload);
        }
      }
    });
    socket.on('close', () => {
      if (!settled && header) reject(new Error('OpenTTD content connection closed before transfer completed'));
    });
  });
}

function readContentHeader(payload) {
  let offset = 0;
  const contentType = payload.readUInt8(offset);
  offset += 1;
  const contentId = payload.readUInt32LE(offset);
  offset += 4;
  const filesize = payload.readUInt32LE(offset);
  offset += 4;
  const nul = payload.indexOf(0, offset);
  const filename = payload.subarray(offset, nul).toString('utf8');
  offset = nul + 1;
  return { contentType, contentId, filesize, filename, body: payload.subarray(offset) };
}

function findFirstScenario(root) {
  const result = spawnSync('find', [root, '-type', 'f', '-name', '*.scn', '-print', '-quit'], { encoding: 'utf8' });
  const scenarioPath = result.stdout.trim();
  if (!scenarioPath) throw new Error(`No .scn file found under ${root}`);
  return scenarioPath;
}
