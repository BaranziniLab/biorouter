import dns from 'node:dns/promises';
import { isIP } from 'node:net';
import { isPrivateNetworkAddress } from '../utils/embeddedBrowserPolicy';

/**
 * Which local destinations the Biorouter Browser may reach (BB-06).
 *
 * The browser's pinned-IP SOCKS proxy (`utils/embeddedBrowserProxy.ts`) used to refuse every
 * private and loopback address, which kept a public page away from a router, a local service and
 * the daemon, and also kept a person away from an intranet page or a Jupyter server on
 * `localhost`. The rule now depends on who asked:
 *
 * | Destination | The person navigates | A page requests it |
 * |---|---|---|
 * | Public | Allowed | Allowed |
 * | Private network or loopback | Allowed | Only from a page on that same local host |
 * | A reserved loopback port (the daemon, the dev renderer, the proxies) | Refused | Refused |
 *
 * The proxy cannot see which tab a connection is for, so admission happens one step earlier, in
 * the partition's `webRequest.onBeforeRequest`, which knows the web contents and so the tab. An
 * admitted local request records a GRANT for its host and port; the proxy connects to a private
 * or loopback address only while a grant for that exact host and port exists. A public page can
 * therefore never reach a local address, because nothing it does creates a grant, and a request it
 * makes to a host some other tab was granted is refused in `onBeforeRequest` before it reaches the
 * proxy.
 *
 * This module is pure (no Electron), so the whole decision is testable on its own.
 */

export type AddressClass = 'public' | 'private' | 'loopback';

/** How long a grant lasts after the last request that refreshed it. */
export const LOCAL_GRANT_TTL_MS = 10 * 60 * 1000;
/** How long a refusal is remembered, so a failed load can say why it failed. */
const REFUSAL_MEMORY_MS = 30 * 1000;
const MAX_GRANTS = 512;

type Clock = () => number;
let clock: Clock = () => Date.now();

/** Tests only: replace the clock. */
export function setLocalNetworkClockForTests(next: Clock | null): void {
  clock = next ?? (() => Date.now());
}

/** A host as the proxy and the request filter both spell it: no brackets, no trailing dot, lower case, IPv6 compressed. */
export function canonicalHost(hostname: string): string {
  const bare = hostname
    .trim()
    .replace(/^\[|\]$/g, '')
    .replace(/\.$/, '')
    .toLowerCase();
  if (isIP(bare) === 6) {
    try {
      return new URL(`http://[${bare.split('%', 1)[0]}]/`).hostname.replace(/^\[|\]$/g, '');
    } catch {
      return bare;
    }
  }
  return bare;
}

function isLocalhostName(host: string): boolean {
  return host === 'localhost' || host.endsWith('.localhost');
}

function embeddedIpv4(host: string): string | null {
  const dotted = /^(?:::ffff:|::|64:ff9b::)(\d+\.\d+\.\d+\.\d+)$/.exec(host);
  if (dotted) return dotted[1];
  const hex = /^(?:::ffff:|::|64:ff9b::)([0-9a-f]{1,4}):([0-9a-f]{1,4})$/.exec(host);
  if (!hex) return null;
  const high = Number.parseInt(hex[1], 16);
  const low = Number.parseInt(hex[2], 16);
  return `${high >> 8}.${high & 0xff}.${low >> 8}.${low & 0xff}`;
}

/** The class of a literal IP address. */
export function classifyAddress(address: string): AddressClass {
  const host = canonicalHost(address);
  const family = isIP(host);
  if (family === 4) {
    const first = Number(host.split('.', 1)[0]);
    // 0.0.0.0/8 reaches this machine's listeners on macOS and Linux, so it is loopback here.
    if (first === 127 || first === 0) return 'loopback';
    return isPrivateNetworkAddress(host) ? 'private' : 'public';
  }
  if (family === 6) {
    if (host === '::1' || host === '::') return 'loopback';
    // An IPv4 address carried inside IPv6 (mapped, compatible or NAT64) is that address: a
    // daemon port must not become reachable by spelling 127.0.0.1 as ::ffff:7f00:1.
    const embedded = embeddedIpv4(host);
    if (embedded) return classifyAddress(embedded);
    return isPrivateNetworkAddress(host) ? 'private' : 'public';
  }
  // Not an address at all: treat as the most restricted class.
  return 'private';
}

// ── Test host names (BIOROUTER_BROWSER_TEST_HOSTS) ─────────────────────────────

type TestHost = { address: string; cls: AddressClass };
let testHosts: ReadonlyMap<string, TestHost> = new Map();

/**
 * Map fixture host names to loopback for the Electron tests, each with the class it stands for:
 * `site-a.test=127.0.0.1:public,intranet.test=127.0.0.1:private`.
 *
 * ⚠ Honoured only when `enabled` is true, which main passes as `!app.isPackaged &&
 * BIOROUTER_E2E === '1'`. A packaged app never reads the variable, because a host mapped as
 * `public` reaches loopback without a grant.
 */
export function configureBrowserTestHosts(spec: string | undefined, enabled: boolean): void {
  const parsed = new Map<string, TestHost>();
  if (enabled && spec) {
    for (const item of spec.split(',')) {
      const match = /^\s*([a-z0-9.-]+)=([0-9a-f.:]+):(public|private|loopback)\s*$/i.exec(item);
      if (!match) continue;
      const address = canonicalHost(match[2]);
      if (!isIP(address)) continue;
      parsed.set(canonicalHost(match[1]), { address, cls: match[3].toLowerCase() as AddressClass });
    }
  }
  testHosts = parsed;
}

export function testHostOverride(hostname: string): TestHost | null {
  return testHosts.get(canonicalHost(hostname)) ?? null;
}

/** The class a host has without a DNS lookup, or `null` when only resolving can tell. */
export function literalHostClass(hostname: string): AddressClass | null {
  const host = canonicalHost(hostname);
  const override = testHostOverride(host);
  if (override) return override.cls;
  if (isLocalhostName(host)) return 'loopback';
  if (isIP(host)) return classifyAddress(host);
  return null;
}

// ── Reserved loopback ports ────────────────────────────────────────────────────

const reservedPorts = new Set<number>();

/**
 * A loopback port no browser tab may reach, for anyone: the daemon (it holds the agent's tools),
 * the dev renderer's Vite server, and the browser's own proxies. Main reserves each one as it
 * learns it. A reservation is never lifted, because a port the daemon held may be reused by the
 * next daemon.
 */
export function reserveLocalPort(port: number | string | null | undefined): void {
  const value = typeof port === 'string' ? Number(port) : port;
  if (typeof value === 'number' && Number.isInteger(value) && value > 0 && value < 65536) {
    reservedPorts.add(value);
  }
}

export function isReservedLocalPort(port: number): boolean {
  return reservedPorts.has(port);
}

/** Tests only. */
export function clearReservedLocalPortsForTests(): void {
  reservedPorts.clear();
}

// ── Grants ──────────────────────────────────────────────────────────────────────

const grants = new Map<string, number>();
const refusals = new Map<string, number>();

function grantKey(host: string, port: number): string {
  return `${canonicalHost(host)}|${port}`;
}

function prune(map: Map<string, number>, ttl: number, now: number): void {
  for (const [key, at] of map) {
    if (now - at > ttl) map.delete(key);
  }
}

/** Record that `host:port` may be reached as a local destination. */
export function grantLocalDestination(host: string, port: number): void {
  const now = clock();
  prune(grants, LOCAL_GRANT_TTL_MS, now);
  if (grants.size >= MAX_GRANTS) {
    const oldest = [...grants.entries()].sort((a, b) => a[1] - b[1])[0];
    if (oldest) grants.delete(oldest[0]);
  }
  grants.set(grantKey(host, port), now);
}

export function hasLocalGrant(host: string, port: number): boolean {
  const at = grants.get(grantKey(host, port));
  return at !== undefined && clock() - at <= LOCAL_GRANT_TTL_MS;
}

/** Tests only. */
export function clearLocalGrantsForTests(): void {
  grants.clear();
  refusals.clear();
}

/** The proxy remembers what it refused, so a failed load can tell the person why. */
export function noteLocalRefusal(host: string, port: number): void {
  const now = clock();
  prune(refusals, REFUSAL_MEMORY_MS, now);
  refusals.set(grantKey(host, port), now);
}

export function wasRecentlyRefused(host: string, port: number): boolean {
  const at = refusals.get(grantKey(host, port));
  return at !== undefined && clock() - at <= REFUSAL_MEMORY_MS;
}

// ── The decision ────────────────────────────────────────────────────────────────

export function defaultPort(url: URL): number {
  if (url.port) return Number(url.port);
  return url.protocol === 'https:' || url.protocol === 'wss:' ? 443 : 80;
}

export type LocalRequest = {
  url: URL;
  /** A main frame load the person started: the address field, a chat link, a toolbar control. */
  person: boolean;
  /** The host of the tab's committed page when that page was itself admitted as local. */
  localPageHost: string | null;
  /**
   * The host of the frame that made a subresource request, when known. A public frame inside a
   * local page (an embedded widget, an ad) is not that page, so it gets no local access.
   */
  requesterHost?: string | null;
};

export type LocalVerdict =
  | { allow: true; cls: AddressClass | null }
  | { allow: false; reason: 'reserved' | 'local'; cls: AddressClass };

export type ResolveHostClass = (hostname: string) => Promise<{
  cls: AddressClass;
  addresses: string[];
} | null>;

/**
 * Decide one request of the browser partition.
 *
 * A request is ADMISSIBLE as local when the person started it, or when the tab already shows a
 * local page on the same host (its own scripts, styles and API calls). Everything else may reach
 * public addresses only. A name is resolved here only when the answer matters: the request is
 * admissible (to know whether to grant), or some other tab holds a grant for the same host and
 * port (to refuse this one if the host is local). Otherwise the proxy's own check is enough,
 * because without a grant it refuses every local address.
 */
export async function decideLocalRequest(
  request: LocalRequest,
  resolve: ResolveHostClass
): Promise<LocalVerdict> {
  const host = canonicalHost(request.url.hostname);
  const port = defaultPort(request.url);
  const requester = request.requesterHost ? canonicalHost(request.requesterHost) : null;
  const admissible =
    request.person ||
    (request.localPageHost !== null &&
      request.localPageHost === host &&
      (requester === null || requester === host));

  let cls = literalHostClass(host);
  let addresses: string[] = [];
  if (cls === null && (admissible || hasLocalGrant(host, port))) {
    const resolved = await resolve(host).catch(() => null);
    if (resolved) {
      cls = resolved.cls;
      addresses = resolved.addresses;
    }
  }
  if (cls === null || cls === 'public') return { allow: true, cls };

  if (
    isReservedLocalPort(port) &&
    (cls === 'loopback' || addresses.some((a) => classifyAddress(a) === 'loopback'))
  ) {
    return { allow: false, reason: 'reserved', cls };
  }
  if (!admissible) return { allow: false, reason: 'local', cls };

  grantLocalDestination(host, port);
  // The proxy may be handed the address rather than the name (Chromium resolves `localhost`
  // itself), so the addresses the name resolved to are granted for this port too.
  for (const address of addresses) grantLocalDestination(address, port);
  if (isLocalhostName(host)) {
    grantLocalDestination('127.0.0.1', port);
    grantLocalDestination('::1', port);
  }
  return { allow: true, cls };
}

/**
 * The proxy's check for a resolved destination. `cls` is the class of everything the name
 * resolved to (any local answer makes it local).
 */
export function proxyMayConnect(host: string, port: number, cls: AddressClass): boolean {
  if (cls === 'public') return true;
  if (cls === 'loopback' && isReservedLocalPort(port)) return false;
  return hasLocalGrant(host, port);
}

type LookupAll = (hostname: string) => Promise<ReadonlyArray<{ address: string; family: number }>>;

const defaultLookup: LookupAll = (host) => dns.lookup(host, { all: true, verbatim: true });

/** The most restricted class among a set of addresses: one local answer makes the name local. */
export function classOfAddresses(addresses: readonly string[]): AddressClass {
  let result: AddressClass = 'public';
  for (const address of addresses) {
    const cls = classifyAddress(address);
    if (cls === 'loopback') return 'loopback';
    if (cls === 'private') result = 'private';
  }
  return result;
}

/**
 * Resolve a host for the request filter and the proxy alike. A fixture host name maps to its
 * declared address and class; `localhost` is loopback without asking DNS; a literal is its own
 * answer.
 */
export async function resolveHostClass(
  hostname: string,
  lookupAll: LookupAll = defaultLookup
): Promise<{ cls: AddressClass; addresses: string[]; family: 4 | 6 } | null> {
  const host = canonicalHost(hostname);
  if (!host) return null;
  const override = testHostOverride(host);
  if (override) {
    return {
      cls: override.cls,
      addresses: [override.address],
      family: isIP(override.address) as 4 | 6,
    };
  }
  if (isLocalhostName(host)) return { cls: 'loopback', addresses: ['127.0.0.1'], family: 4 };
  const literal = isIP(host);
  if (literal) return { cls: classifyAddress(host), addresses: [host], family: literal as 4 | 6 };
  const resolved = await lookupAll(host);
  if (resolved.length === 0 || resolved.some(({ address, family }) => isIP(address) !== family)) {
    return null;
  }
  const addresses = resolved.map(({ address }) => address);
  return { cls: classOfAddresses(addresses), addresses, family: resolved[0].family as 4 | 6 };
}
