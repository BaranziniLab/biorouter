/**
 * The name OpenSSH files a host key under, read back into a host and a port: `host` on port 22,
 * `[host]:port` elsewhere (the daemon's `host_keys::lookup_name`). Only a plain name comes back,
 * as the daemon takes it: letters, digits and `.-_:`, not starting with `-` or `.`.
 */
export interface HostKeyLookup {
  host: string;
  port: number;
}

const PLAIN_HOST = /^[A-Za-z0-9_:][A-Za-z0-9._:-]{0,252}$/;

/** A lookup name as a host and port, or `null` when it is not one. */
export function parseHostKeyLookup(text: string | null | undefined): HostKeyLookup | null {
  if (typeof text !== 'string') return null;
  const trimmed = text.trim().replace(/[.,;]+$/, '');
  const bracketed = /^\[([^[\]]+)\]:(\d{1,5})$/.exec(trimmed);
  if (bracketed) {
    const port = Number(bracketed[2]);
    return PLAIN_HOST.test(bracketed[1]) && port > 0 && port <= 65535
      ? { host: bracketed[1], port }
      : null;
  }
  return PLAIN_HOST.test(trimmed) ? { host: trimmed, port: 22 } : null;
}

const UNKNOWN_LINE = / host key is known for (\S+)/i;

/**
 * The host OpenSSH named in an unknown key's deciding line ("No ED25519 host key is known for
 * H and you have requested strict checking."), the last such line winning as the daemon reads
 * it. `null` when the words name none.
 */
export function unknownKeyLookup(detail: string | null | undefined): HostKeyLookup | null {
  if (typeof detail !== 'string') return null;
  const lines = detail.split('\n').reverse();
  for (const line of lines) {
    const match = UNKNOWN_LINE.exec(line);
    if (match) return parseHostKeyLookup(match[1]);
  }
  return null;
}
