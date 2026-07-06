// scripts/geo/lib/roadprofiles.mjs
//
// Shared plumbing for road-owned longitudinal profiles (spec §5 "A+B"
// amendment). bake-world.mjs grades the DEM and writes each road/rail way's
// smoothed 10 m-station profile to scratch/geo/grading-profiles.json, keyed by
// a HASH of the exact road/rail geometry it graded. bake-winterthur.mjs
// recomputes the same roads/rails (same osm-roads.json + same projector, so
// byte-identical arrays) and MUST match that hash before it may attach the
// profiles to data/winterthur/roads.json — a hard gate, no fallback: if the
// geometry ever diverges (different fetch, different transform), the two
// artifacts would silently disagree, so we fail loudly instead.
//
// Profile heights are stored RELATIVE to the shared world anchor (the graded
// ground height at world origin 0,0). The runtime's groundYAt(x,z) =
// worldHeightAt(x,z) − anchorGroundHeight(world); a road-owned profile height
// is likewise anchor-relative, so the runtime adds it directly into its
// y≈0 cityRoot space (see src/diorama/ksw/geo/worldData.ts). Any residual
// between the bake's bilinear anchor and the runtime's nearest-vertex anchor
// is sub-decimetre and documented; it is NOT a fallback path.

/**
 * Deterministic geometry key for one road/rail way: class + width + the 0.01 m
 * -quantized centreline (the same precision roads.json already stores). Two ways
 * with the same key ARE the same physical way, so profiles can be matched by
 * key across two bakes without relying on array index/order. This is what lets
 * bake-winterthur attach bake-world's grading profiles even when the two scripts
 * enumerate a different SUBSET of ways (bake-world grades the whole Gemeinde;
 * the committed roads.json is a clipped subset with byte-identical geometry).
 */
export function wayKey(kind, r) {
  const w = (Math.round(r.width * 100) / 100).toFixed(2);
  const pts = r.pts.map(([x, z]) => `${(Math.round(x * 100) / 100).toFixed(2)},${(Math.round(z * 100) / 100).toFixed(2)}`).join(';');
  return `${kind}|${r.class}|${w}|${pts}`;
}

/**
 * Shift an absolute-DEM-metre profile to be anchor-relative by subtracting the
 * shared anchor ground height. The runtime adds these back into its y≈0 space.
 */
export function shiftProfile(profile, anchorGround) {
  return {
    stepM: profile.stepM,
    ys: profile.ys.map((y) => y - anchorGround),
  };
}

/**
 * Attach grading profiles (from a byKey map, see bake-world.mjs) onto each
 * road/rail by geometry key. HARD GATE — every way must have a matching
 * profile; a miss throws (no fallback). Mutates each way, adding `.profile`.
 * Shared by bake-winterthur.mjs and scripts/geo/attach-road-profiles.mjs so
 * there is ONE matching implementation.
 */
export function attachProfilesByKey(byKey, roads, rails) {
  if (!byKey || typeof byKey !== 'object') {
    throw new Error('attachProfilesByKey: byKey map missing — stale grading-profiles.json, re-run `npm run geo:bake-world`');
  }
  const miss = [];
  const take = (kind, arr) => {
    for (const r of arr) {
      const p = byKey[wayKey(kind, r)];
      if (!p) { miss.push(`${kind} ${r.class} ${r.pts[0]?.join(',')}`); continue; }
      r.profile = p;
    }
  };
  take('road', roads);
  take('rail', rails);
  if (miss.length > 0) {
    throw new Error(`attachProfilesByKey: ${miss.length} way(s) have no grading profile (e.g. ${miss.slice(0, 3).join(' | ')}) — profiles graded against different geometry; re-run \`npm run geo:bake-world\``);
  }
}

/** Quantize a profile's ys to 0.01 m (deterministic byte output). */
export function quantizeProfile(profile) {
  return {
    stepM: profile.stepM,
    ys: profile.ys.map((y) => Math.round(y * 100) / 100),
  };
}
