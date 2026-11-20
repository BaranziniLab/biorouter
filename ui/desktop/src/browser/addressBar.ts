/**
 * What the Biorouter Browser's address field loads for what a person typed.
 *
 * Pure and free of Electron and the DOM: the renderer's address field calls it, and so does the
 * main process for a request that arrives from elsewhere. The rules are the ones people expect
 * from Chrome's omnibox, kept small:
 *
 * - An `http:` or `https:` URL loads as typed.
 * - A host, with or without a port and path, loads over `https:`, except a local one
 *   (`localhost`, an IP literal, a single label with a port such as `jupyter:8888`), which loads
 *   over `http:`, the scheme local development servers speak.
 * - Anything else is a search with the search engine, Google by default.
 *
 * - An address with a scheme a web page must never be opened with (`file:`, `javascript:`,
 *   `chrome:`, `about:`, `data:`, `view-source:` and the browser's own internal schemes) is
 *   refused with one line saying why (L2). Sending it to the search engine instead would hand a
 *   local path or a script to a third party as a query.
 * - Any other scheme (`mailto:`) is searched for as text, which is harmless.
 */

export type SearchEngine = {
  name: string;
  /** A URL template with `{searchTerms}` where the encoded query goes. */
  template: string;
};

export const DEFAULT_SEARCH_ENGINE: SearchEngine = {
  name: 'Google',
  template: 'https://www.google.com/search?q={searchTerms}',
};

export type AddressTarget =
  | { kind: 'url' | 'search'; url: string }
  | { kind: 'refused'; scheme: string; reason: string };

/**
 * Schemes the address field refuses (L2). `javascript:` and `vbscript:` are refused however they
 * are spaced; the rest only when the scheme runs straight into its address (`about:blank`), so a
 * search such as `data: science jobs` still searches.
 */
const REFUSED_SCHEMES = new Set([
  'file',
  'javascript',
  'vbscript',
  'chrome',
  'chrome-extension',
  'chrome-untrusted',
  'devtools',
  'about',
  'data',
  'blob',
  'filesystem',
  'view-source',
]);
const SCHEME_PREFIX = /^([a-z][a-z0-9+.-]*):(\s?)/i;

function refusedScheme(text: string): string | null {
  const match = SCHEME_PREFIX.exec(text);
  if (!match) return null;
  const scheme = match[1].toLowerCase();
  if (!REFUSED_SCHEMES.has(scheme)) return null;
  const spaced = match[2] !== '';
  return spaced && scheme !== 'javascript' && scheme !== 'vbscript' ? null : scheme;
}

const MAX_ADDRESS_CHARS = 8 * 1024;
const HOST_PATTERN =
  /^(\[[0-9a-f:.]+\]|[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)*)(:\d{1,5})?([/?#].*)?$/i;
const IPV4 = /^\d{1,3}(?:\.\d{1,3}){3}$/;

function searchUrl(text: string, engine: SearchEngine): string {
  const template = engine.template.includes('{searchTerms}')
    ? engine.template
    : DEFAULT_SEARCH_ENGINE.template;
  return template.replace('{searchTerms}', encodeURIComponent(text));
}

function validHttpUrl(candidate: string): string | null {
  try {
    const url = new URL(candidate);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return null;
    if (url.username || url.password || !url.hostname) return null;
    return url.href;
  } catch {
    return null;
  }
}

function isLocalHost(host: string): boolean {
  const bare = host.replace(/^\[|\]$/g, '').toLowerCase();
  return (
    bare === 'localhost' || bare.endsWith('.localhost') || IPV4.test(bare) || bare.includes(':')
  );
}

/**
 * The page the address field opens for `input`, or `null` for empty input.
 */
export function addressFieldTarget(
  input: string,
  engine: SearchEngine = DEFAULT_SEARCH_ENGINE
): AddressTarget | null {
  const text = input.trim();
  if (!text) return null;
  const refused = refusedScheme(text);
  if (refused) {
    return {
      kind: 'refused',
      scheme: refused,
      reason: `${refused}: addresses do not open in the Biorouter Browser. It opens web pages (http and https) only.`,
    };
  }
  if (text.length > MAX_ADDRESS_CHARS)
    return { kind: 'search', url: searchUrl(text.slice(0, 2048), engine) };

  if (/^https?:\/\//i.test(text)) {
    const url = validHttpUrl(text);
    return url ? { kind: 'url', url } : { kind: 'search', url: searchUrl(text, engine) };
  }

  if (!/\s/.test(text)) {
    const match = HOST_PATTERN.exec(text);
    if (match) {
      const [, host, port] = match;
      const local = isLocalHost(host);
      const dotted = host.includes('.') && !local;
      const hasAlphaTld = dotted && /\.[a-z][a-z0-9-]*[a-z0-9]$/i.test(host);
      const singleLabelWithPort = !host.includes('.') && Boolean(port);
      if (local || hasAlphaTld || singleLabelWithPort) {
        const scheme = local || singleLabelWithPort ? 'http' : 'https';
        const url = validHttpUrl(`${scheme}://${text}`);
        if (url) return { kind: 'url', url };
      }
    }
  }

  return { kind: 'search', url: searchUrl(text, engine) };
}
