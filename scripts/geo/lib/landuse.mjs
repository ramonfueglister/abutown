// Landuse-Ringe aus OSM: Landcover-Enum-Kind + Ring in lokalen Metern
// (gerundet wie transformRoads). Unbekannte landuse/natural-Tags → weglassen.
const KIND_MAP = {
  meadow: 1, grass: 1,
  forest: 2, wood: 2,
  farmland: 3,
  residential: 4,
  industrial: 5, commercial: 5,
  basin: 6, reservoir: 6,
};

export function transformLanduse({ osmLanduse, projector }) {
  const out = [];
  for (const el of osmLanduse.elements ?? []) {
    if (el.type !== 'way' || !el.geometry || el.geometry.length < 3) continue;
    const t = el.tags ?? {};
    const tag = t.landuse ?? t.natural;
    const kind = KIND_MAP[tag];
    if (!kind) continue;
    const ring = el.geometry.map(({ lon, lat }) => {
      const [x, z] = projector.toLocal(lon, lat);
      return [Math.round(x * 100) / 100, Math.round(z * 100) / 100];
    });
    out.push({ kind, ring });
  }
  return out;
}
