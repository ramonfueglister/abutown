// Pure Auswahl-Logik über swisstopo-STAC-Seiten: pro Kachel (Suffix der
// Item-ID nach dem Jahrgang) das gewünschte Asset, neuester Jahrgang gewinnt.
// Das Netz (Pagination via links.rel=next) macht fetch-winterthur.mjs.
export function stacItemUrls({ pageJsonList, assetSuffix }) {
  const byTile = new Map(); // tileKey -> { vintage, url }
  for (const page of pageJsonList) {
    for (const item of page.features ?? []) {
      const m = /_(\d{4})_(\d+-\d+)$/.exec(item.id);
      if (!m) continue;
      const [, vintage, tile] = m;
      const asset = Object.values(item.assets ?? {}).find((a) => a.href.endsWith(assetSuffix));
      if (!asset) continue;
      const prev = byTile.get(tile);
      if (!prev || Number(vintage) > prev.vintage) byTile.set(tile, { vintage: Number(vintage), url: asset.href });
    }
  }
  return [...byTile.keys()].sort().map((k) => byTile.get(k).url);
}
