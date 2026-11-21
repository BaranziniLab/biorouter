import dns from 'node:dns/promises';
import { isIP } from 'node:net';

const NETWORK_SCHEMES = new Set(['http:', 'https:', 'ws:', 'wss:']);

export type PublicNetworkTarget = {
  address: string;
  family: 4 | 6;
};

type LookupAll = (hostname: string) => Promise<ReadonlyArray<{ address: string; family: number }>>;

const NON_PUBLIC_IPV4_RANGES: ReadonlyArray<readonly [number, number]> = [
  [0x00000000, 8],
  [0x0a000000, 8],
  [0x64400000, 10],
  [0x7f000000, 8],
  [0xa9fe0000, 16],
  [0xac100000, 12],
  [0xc0000000, 24],
  [0xc0000200, 24],
  [0xc0586300, 24],
  [0xc0a80000, 16],
  [0xc6120000, 15],
  [0xc6336400, 24],
  [0xcb007100, 24],
  [0xe0000000, 4],
  [0xf0000000, 4],
];

function ipv4Value(address: string): number {
  return address
    .split('.')
    .map(Number)
    .reduce((value, octet) => (value * 256 + octet) >>> 0, 0);
}

function hasIpv4Prefix(address: number, prefix: number, bits: number): boolean {
  const mask = (0xffffffff << (32 - bits)) >>> 0;
  return (address & mask) >>> 0 === (prefix & mask) >>> 0;
}

function ipv4FromWords(high: number, low: number): string {
  return `${high >> 8}.${high & 0xff}.${low >> 8}.${low & 0xff}`;
}

function ipv6Words(address: string): number[] | null {
  let normalized = address.toLowerCase().split('%', 1)[0];
  if (normalized.includes('.')) {
    const tailStart = normalized.lastIndexOf(':') + 1;
    const ipv4 = normalized.slice(tailStart);
    if (tailStart === 0 || isIP(ipv4) !== 4) return null;
    const octets = ipv4.split('.').map(Number);
    const high = octets[0] * 256 + octets[1];
    const low = octets[2] * 256 + octets[3];
    normalized = `${normalized.slice(0, tailStart)}${high.toString(16)}:${low.toString(16)}`;
  }

  const halves = normalized.split('::');
  if (halves.length > 2) return null;
  const parseHalf = (half: string) =>
    half
      .split(':')
      .filter(Boolean)
      .map((word) => (/^[0-9a-f]{1,4}$/.test(word) ? Number.parseInt(word, 16) : Number.NaN));
  const left = parseHalf(halves[0]);
  const right = halves.length === 2 ? parseHalf(halves[1]) : [];
  if ([...left, ...right].some((word) => !Number.isFinite(word))) return null;

  if (halves.length === 1) return left.length === 8 ? left : null;
  const omitted = 8 - left.length - right.length;
  if (omitted < 1) return null;
  return [...left, ...Array<number>(omitted).fill(0), ...right];
}

function embeddedIpv4(words: readonly number[]): string | null {
  const firstFiveZero = words.slice(0, 5).every((word) => word === 0);
  if (firstFiveZero && words[5] === 0xffff) {
    return ipv4FromWords(words[6], words[7]);
  }
  if (words.slice(0, 4).every((word) => word === 0) && words[4] === 0xffff && words[5] === 0) {
    return ipv4FromWords(words[6], words[7]);
  }

  if (words[0] === 0x64 && words[1] === 0xff9b && words.slice(2, 6).every((word) => word === 0)) {
    return ipv4FromWords(words[6], words[7]);
  }
  return null;
}

/** True for addresses that a remote preview must never be allowed to reach. */
export function isPrivateNetworkAddress(address: string): boolean {
  const family = isIP(address);
  if (family === 4) {
    const value = ipv4Value(address);
    return NON_PUBLIC_IPV4_RANGES.some(([prefix, bits]) => hasIpv4Prefix(value, prefix, bits));
  }
  if (family !== 6) return false;

  const words = ipv6Words(address);
  if (!words) return true;
  const translated = embeddedIpv4(words);
  if (translated) return isPrivateNetworkAddress(translated);

  const first = words[0];
  return (
    (first & 0xfe00) === 0xfc00 ||
    (first & 0xffc0) === 0xfe80 ||
    (first & 0xffc0) === 0xfec0 ||
    (first & 0xff00) === 0xff00 ||
    (words[0] === 0x64 && words[1] === 0xff9b && words[2] === 1) ||
    (words[0] === 0x100 && words.slice(1, 4).every((word) => word === 0)) ||
    (words[0] === 0x2001 && words[1] === 0) ||
    (words[0] === 0x2001 && words[1] === 2 && words[2] === 0) ||
    (words[0] === 0x2001 && (words[1] & 0xfff0) === 0x10) ||
    (words[0] === 0x2001 && (words[1] & 0xfff0) === 0x20) ||
    (words[0] === 0x2001 && words[1] === 0x0db8) ||
    words[0] === 0x2002 ||
    (words[0] === 0x3fff && (words[1] & 0xf000) === 0) ||
    (first & 0xe000) !== 0x2000
  );
}

function normalizedHost(hostname: string): string {
  return hostname
    .replace(/^\[|\]$/g, '')
    .replace(/\.$/, '')
    .toLowerCase();
}

/** Chromium's built in PDF viewer, which Electron ships as a component extension. */
export const PDF_VIEWER_EXTENSION_ID = 'mhjfbmdgcfjbbpaeojofohoefgiehjai';

/**
 * The PDF viewer's own resources: its extension page and the shared `chrome://resources` it
 * loads. Measured on Electron 39.8.10, opening a PDF in the browser requests exactly these two
 * schemes besides the PDF itself. A web page cannot load either (Chromium refuses web origins
 * access to them), so admitting them here adds nothing a page can reach.
 */
export function isPdfViewerResourceUrl(candidate: string): boolean {
  try {
    const parsed = new URL(candidate);
    return (
      (parsed.protocol === 'chrome-extension:' && parsed.hostname === PDF_VIEWER_EXTENSION_ID) ||
      (parsed.protocol === 'chrome:' && parsed.hostname === 'resources')
    );
  } catch {
    return false;
  }
}

export function isAllowedEmbeddedRequestUrl(candidate: string): boolean {
  try {
    const parsed = new URL(candidate);
    return NETWORK_SCHEMES.has(parsed.protocol) && !parsed.username && !parsed.password;
  } catch {
    return false;
  }
}

/** Resolve once for the socket that will be opened to the returned literal IP. */
export async function resolvePublicEmbeddedHost(
  hostname: string,
  lookupAll: LookupAll = (host) => dns.lookup(host, { all: true, verbatim: true })
): Promise<PublicNetworkTarget> {
  const host = normalizedHost(hostname);
  if (!host || host === 'localhost' || host.endsWith('.localhost')) {
    throw new Error('Local addresses are not available to embedded pages.');
  }

  const family = isIP(host);
  if (family) {
    if (isPrivateNetworkAddress(host)) {
      throw new Error('Local addresses are not available to embedded pages.');
    }
    return { address: host, family: family as 4 | 6 };
  }

  const resolved = await lookupAll(host);
  if (
    resolved.length === 0 ||
    resolved.some(({ address, family: resolvedFamily }) => {
      const actualFamily = isIP(address);
      return actualFamily !== resolvedFamily || isPrivateNetworkAddress(address);
    })
  ) {
    throw new Error('Local addresses are not available to embedded pages.');
  }
  const target = resolved[0];
  return { address: target.address, family: target.family as 4 | 6 };
}

/** Validate a URL immediately before offering it to the system browser. */
export async function assertPublicEmbeddedUrl(candidate: string): Promise<URL> {
  const parsed = new URL(candidate);
  if (!isAllowedEmbeddedRequestUrl(candidate)) {
    throw new Error('Only public HTTP(S) and WebSocket URLs are allowed.');
  }
  await resolvePublicEmbeddedHost(parsed.hostname);
  return parsed;
}

/**
 * Whether a URL looks like a sign-in page. Used ONLY for the wording of the confirmation shown
 * before the default browser opens a link ("Open this sign-in page in your browser?").
 *
 * ⚠ It no longer keeps any page out of the Biorouter Browser (BB-03): login pages load in the
 * panel like any other page. It used to block every navigation whose host or path contained
 * `auth`, `login`, `sso` and similar words, which made login gated sites unusable and caught
 * ordinary URLs too. What protects a login now is that the agent can never type into a password
 * or verification code field.
 */
export function isAuthenticationNavigation(candidate: string): boolean {
  try {
    const url = new URL(candidate);
    const haystack = `${url.hostname}${url.pathname}`.toLowerCase();
    return (
      /(^|[./_-])(auth|authorize|login|log-in|oauth|saml|signin|sign-in|sso)([./_-]|$)/.test(
        haystack
      ) ||
      url.searchParams.has('client_id') ||
      url.searchParams.has('redirect_uri') ||
      url.searchParams.has('response_type')
    );
  } catch {
    return false;
  }
}
