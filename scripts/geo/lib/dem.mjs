// scripts/geo/lib/dem.mjs
// ESRI-AAIGrid (aus gdal_translate) → Höhen-Sampler in lokalen Metern.
// Zeile 0 = Nord. Bilinear; Anfragen außerhalb clampen auf den Rand —
// der Dörfer-Ring fragt bewusst über die Gemeindegrenze hinaus.
export function parseAAIGrid(text) {
  const lines = text.split('\n');
  const head = {};
  let i = 0;
  for (; i < lines.length; i++) {
    const m = /^(\w+)\s+(-?[\d.]+)/.exec(lines[i]);
    if (!m || !/^(ncols|nrows|xllcorner|yllcorner|cellsize|dx|dy|NODATA_value)$/i.test(m[1])) break;
    head[m[1].toLowerCase()] = Number(m[2]);
  }
  const ncols = head.ncols, nrows = head.nrows;
  const data = new Float32Array(ncols * nrows);
  let k = 0;
  for (; i < lines.length; i++) {
    if (!lines[i].trim()) continue;
    for (const v of lines[i].trim().split(/\s+/)) data[k++] = Number(v);
  }
  if (k !== ncols * nrows) throw new Error(`AAIGrid: ${k} values, expected ${ncols * nrows}`);
  // Some AAIGrid exports (this bake's dem.asc, gdal_translate over a
  // geographic-degree source) use `dx`/`dy` instead of a single `cellsize`,
  // and dx != dy here (6e-5 vs 4e-5 deg — the source raster isn't square in
  // lon/lat degree-space). Support both: cellsize when present/isotropic,
  // otherwise separate celldx/celldy.
  const celldx = head.cellsize ?? head.dx;
  const celldy = head.cellsize ?? head.dy;
  return {
    ncols, nrows, xll: head.xllcorner, yll: head.yllcorner,
    celldx, celldy, nodata: head.nodata_value ?? -9999, data,
  };
}

export function makeDemSampler(grid, projector) {
  // lokale Meter → lon/lat invers zur equirect-Projektion (project.mjs):
  // toLocal: x = (lon-anchorLon)*rad*R*cos(anchorLat), z = -(lat-anchorLat)*rad*R
  // Inverse: lon = anchorLon + x/(R*rad*cos(anchorLat)), lat = anchorLat - z/(R*rad)
  const R = 6371008.8, rad = Math.PI / 180;
  return {
    heightAt(x, z) {
      const lat = projector.anchorLat - z / (R * rad);
      const lon = projector.anchorLon + x / (R * rad * Math.cos(projector.anchorLat * rad));
      // grid.xll/yll are corner coords of the outer edge of the corner cell;
      // cell centers are offset by +0.5 cell from the corner. Columns step by
      // celldx (lon), rows step by celldy (lat) — independent when the grid
      // is non-square in degree-space.
      const col = (lon - grid.xll) / grid.celldx - 0.5;
      const rowFromS = (lat - grid.yll) / grid.celldy - 0.5; // row index counted from south (row 0 = south edge)
      const row = grid.nrows - 1 - rowFromS; // flip: row 0 = north (data layout)
      const c0 = Math.max(0, Math.min(grid.ncols - 2, Math.floor(col)));
      const r0 = Math.max(0, Math.min(grid.nrows - 2, Math.floor(row)));
      const fc = Math.max(0, Math.min(1, col - c0));
      const fr = Math.max(0, Math.min(1, row - r0));
      const at = (r, c) => grid.data[r * grid.ncols + c];
      const h = at(r0, c0) * (1 - fc) * (1 - fr) + at(r0, c0 + 1) * fc * (1 - fr)
        + at(r0 + 1, c0) * (1 - fc) * fr + at(r0 + 1, c0 + 1) * fc * fr;
      return h;
    },
  };
}

// Resample the geographic AAIGrid sampler onto a regular LOCAL-METRE grid so
// the terrain-grading kernel (grading.mjs) can mutate it in place. That kernel
// speaks a different grid convention than parseAAIGrid: row-major
// `data[j*ncols+i]` with i↔world-x (east), j↔world-z (south), cell (0,0)'s
// centre at (xll,yll) in local metres, isotropic `cellsize`, and NO north/south
// flip (row index grows with z). parseAAIGrid is the opposite (lon/lat degrees,
// row 0 = north, non-square dx/dy). So we sample the geographic `sampler` at
// each local-grid cell centre and hand the resulting flat grid to gradeDem.
//
// `cellsize` (default 10 m) is finer than the finest tile sample step (L2 ≈
// 15 m), so nothing the tiles read is lost to resampling. Returns
// { grid, sampler } where the returned sampler's heightAt(x,z) bilinearly
// reads grid.data — call gradeDem on `grid`, then pass this sampler to every
// later stage so they see the graded heights (the grid is mutated in place, and
// heightAt reads it lazily, so the same sampler object reflects the mutation).
export function makeLocalGrid(geoSampler, { minX, minZ, maxX, maxZ }, cellsize = 10) {
  if (!(cellsize > 0)) throw new Error('makeLocalGrid: cellsize must be > 0');
  const ncols = Math.max(2, Math.ceil((maxX - minX) / cellsize) + 1);
  const nrows = Math.max(2, Math.ceil((maxZ - minZ) / cellsize) + 1);
  const data = new Float64Array(ncols * nrows);
  for (let j = 0; j < nrows; j++) {
    const z = minZ + j * cellsize;
    for (let i = 0; i < ncols; i++) {
      const x = minX + i * cellsize;
      data[j * ncols + i] = geoSampler.heightAt(x, z);
    }
  }
  const grid = { ncols, nrows, xll: minX, yll: minZ, cellsize, data };
  const sampler = {
    heightAt(x, z) {
      const col = (x - grid.xll) / grid.cellsize;
      const row = (z - grid.yll) / grid.cellsize;
      const c0 = Math.max(0, Math.min(grid.ncols - 2, Math.floor(col)));
      const r0 = Math.max(0, Math.min(grid.nrows - 2, Math.floor(row)));
      const fc = Math.max(0, Math.min(1, col - c0));
      const fr = Math.max(0, Math.min(1, row - r0));
      const at = (r, c) => grid.data[r * grid.ncols + c];
      return at(r0, c0) * (1 - fc) * (1 - fr) + at(r0, c0 + 1) * fc * (1 - fr)
        + at(r0 + 1, c0) * (1 - fc) * fr + at(r0 + 1, c0 + 1) * fc * fr;
    },
  };
  return { grid, sampler };
}

// `snapMarginM` (default 0) is forwarded as a third arg to sampler.heightAt.
// The corridor-snap sampler (scripts/geo/lib/corridorsnap.mjs) uses it to widen
// its hard-clamp region to this tile's cell-diagonal, so a road bench is
// clamped across the whole tile cell it crosses, not just the vertices
// physically inside the corridor (the coarse lattice otherwise interpolates the
// bench away — see corridorsnap.mjs). Plain samplers ignore the extra arg.
export function extractPatch(sampler, { originX, originZ, gridN, cellSize }, snapMarginM = 0) {
  const out = new Float32Array(gridN * gridN);
  for (let j = 0; j < gridN; j++)
    for (let i = 0; i < gridN; i++)
      out[j * gridN + i] = sampler.heightAt(originX + i * cellSize, originZ + j * cellSize, snapMarginM);
  return out;
}
