// scripts/geo/attach-road-profiles.mjs
//
// Attach the road-owned longitudinal profiles from a `geo:bake-world` grading
// run onto the committed data/winterthur/roads.json IN PLACE, matched by way
// geometry (spec §5 "A+B"). This exists because bake-world grades the whole
// Gemeinde while this branch's committed roads.json is a clipped SUBSET with
// byte-identical geometry (the #133 reconciliation is a separate task): running
// the full bake-winterthur.mjs would also regenerate nature.json/buildings.json
// to municipality scale, which is out of scope here. So we surgically add the
// `profile` field to the existing roads.json, reusing bake-winterthur.mjs's
// exact matching logic (attachProfilesByKey — one shared implementation).
//
// HARD GATE: every committed road/rail must have a matching profile in the
// grading run, else this throws (no fallback). Deterministic: same inputs →
// byte-identical roads.json.
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { attachProfilesByKey } from './lib/roadprofiles.mjs';

const SCRATCH = 'scratch/geo';
const OUT = 'data/winterthur';
const profPath = `${SCRATCH}/grading-profiles.json`;
const roadsPath = `${OUT}/roads.json`;

if (!existsSync(profPath)) {
  console.error(`attach-road-profiles: ${profPath} missing — run \`npm run geo:bake-world\` first`);
  process.exit(1);
}
if (!existsSync(roadsPath)) {
  console.error(`attach-road-profiles: ${roadsPath} missing`);
  process.exit(1);
}

const prof = JSON.parse(readFileSync(profPath, 'utf8'));
const { roads, rails } = JSON.parse(readFileSync(roadsPath, 'utf8'));

attachProfilesByKey(prof.byKey, roads, rails);

writeFileSync(roadsPath, JSON.stringify({ roads, rails }));
console.log(
  `attach-road-profiles: attached ${roads.length} road + ${rails.length} rail profiles by geometry `
  + `(anchor ${prof.anchorGroundHeight} m, ${Object.keys(prof.byKey).length} keyed) → ${roadsPath}`,
);
