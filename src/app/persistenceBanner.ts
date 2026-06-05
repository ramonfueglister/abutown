/**
 * Non-blocking persistence banner — shown when persistence is degraded
 * (transient failures, map stays rendered) and cleared on recovery.
 * Idempotent: calling it multiple times with the same status does not
 * duplicate the banner.
 */

export type PersistenceBannerStatus = 'healthy' | 'degraded' | 'starting' | 'stale' | 'down';

export function setPersistenceBanner(doc: Document, status: PersistenceBannerStatus): void {
  const existing = doc.querySelector('[data-persistence-banner]');
  if (status === 'healthy' || status === 'starting') {
    existing?.remove();
    return;
  }
  const el = (existing as HTMLElement) ?? doc.createElement('div');
  el.setAttribute('data-persistence-banner', 'true');
  el.className = 'persistence-banner';
  el.textContent =
    status === 'degraded'
      ? 'Persistenz vorübergehend verzögert — Welt läuft, letzte Schreibvorgänge werden wiederholt.'
      : 'Persistenz offline — Daten werden derzeit nicht gespeichert.';
  if (!existing) doc.body.appendChild(el);
}
