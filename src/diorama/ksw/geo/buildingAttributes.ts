// Single accessor for per-building enrichment (Bauzone erlaubt / GWR ist).
// TODAY: resolves from the baked static artifact already in the bundle —
// zero new wire. LATER (Vercel+Fly cutover): swap this body to fetch+cache
// GET /building-attributes from VITE_ABUTOWN_BACKEND_URL; the return shape
// is identical, so callers never change.
import { type BakedBuilding, cityBuildings, kswBuildings } from './geoData';

export type BuildingHoverInfo = {
  name?: string;
  gwrCategory: string | null;
  bauzone: string | null;
  bauzoneCode: string | null;
};

const byId = new Map<string, BakedBuilding>();
for (const b of [...cityBuildings, ...kswBuildings]) byId.set(b.id, b);

export function getBuildingHoverInfo(id: string): BuildingHoverInfo | undefined {
  const b = byId.get(id);
  if (!b) return undefined;
  return {
    name: b.name,
    gwrCategory: b.gwrCategory ?? null,
    bauzone: b.bauzone ?? null,
    bauzoneCode: b.bauzoneCode ?? null,
  };
}
