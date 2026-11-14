export type RemoteHttpBackend = { url: string; host: string };

/** Fragment changes do not change the backend request; query changes do. */
export function effectiveBackendUrl(raw: string): string | null {
  try {
    const url = new URL(raw);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return null;
    url.hash = '';
    return url.href;
  } catch {
    return null;
  }
}

/** The URL stays in memory; only a SHA-256 identity goes into ackHosts. */
export async function remoteConsentKey(
  raw: string,
  kind: 'http' | 'disclosure' = 'http'
): Promise<string | null> {
  const canonical = effectiveBackendUrl(raw);
  if (!canonical) return null;
  const digest = await globalThis.crypto.subtle.digest(
    'SHA-256',
    new TextEncoder().encode(canonical)
  );
  const hex = Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join(
    ''
  );
  return `remote-${kind}:sha256:${hex}`;
}

/** Match a confirmation to the complete backend URL, including its query. */
export function remoteHttpBackendUrl(raw: string): RemoteHttpBackend | null {
  const canonical = effectiveBackendUrl(raw);
  if (!canonical) return null;
  const url = new URL(canonical);
  const loopback =
    url.hostname === 'localhost' ||
    url.hostname === 'localhost.' ||
    url.hostname === '[::1]' ||
    /^127\.(?:\d{1,3}\.){2}\d{1,3}$/.test(url.hostname);
  return url.protocol === 'http:' && !loopback ? { url: canonical, host: url.host } : null;
}

export function hasRemoteHttpAcknowledgement(ackHosts: unknown, url: string): boolean {
  return Array.isArray(ackHosts) && ackHosts.some((entry) => entry === url);
}

export function addRemoteHttpAcknowledgement(ackHosts: unknown, url: string): string[] {
  if (!/^remote-(?:http|disclosure):sha256:[0-9a-f]{64}$/.test(url)) return [];
  const previous = Array.isArray(ackHosts)
    ? ackHosts.filter(
        (entry): entry is string =>
          typeof entry === 'string' && entry !== url && !entry.includes('?') && !entry.includes('@')
      )
    : [];
  return [...previous, url].slice(-64);
}
