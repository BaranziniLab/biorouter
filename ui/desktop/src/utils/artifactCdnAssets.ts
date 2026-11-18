/**
 * The CDN→inline rewriter for Auto Visualiser artifacts.
 *
 * Every artifact is displayed under `ARTIFACT_BROWSER_CSP`
 * (`default-src 'none'`), so the document itself can never reach the network:
 * a remote `<script src=…>` is blocked and the figure comes up blank or with
 * "… library failed to load". CDN mode still exists because it is the *stored*
 * blob that matters — `BIOROUTER_AUTOVIS_CDN=1` (the desktop default, set in
 * `biorouterd.ts`) shrinks a persisted figure from megabytes to a few KB, which
 * is what makes a large diagram survive a session reload. The main process
 * closes the gap by pre-fetching each URL below and splicing its source into
 * the document as an inline `<script>`, *before* the CSP is applied.
 *
 * Two invariants follow, and both are asserted from the Rust side in
 * `crates/biorouter-mcp/tests/autovis_cdn_desktop_contract.rs`:
 *
 * 1. Every URL the Auto Visualiser can emit in CDN mode appears in
 *    `ARTIFACT_CDN_ASSETS`. A library that is missing here is simply never
 *    inlined, and the figure fails every time in the packaged app while passing
 *    every Rust test — which is exactly how Mermaid shipped broken.
 * 2. Each is emitted as `<script src="URL" …></script>` (or a `<link>` for CSS),
 *    because that is the only shape the patterns below match. In particular a
 *    `<script type="module">import x from '…/+esm'</script>` never matches, so
 *    an ESM entrypoint cannot be used here however valid the URL is.
 *
 * ## Why the match is version-tolerant
 *
 * A figure's HTML is *persisted* in the session, so the tag a user has on disk
 * was written by whatever pin was current the day the figure was made. Matching
 * the pinned URL as an exact string meant every version bump silently broke
 * every figure already in someone's history: the stored tag no longer matched,
 * nothing was inlined, and the CSP stopped the remote script too, so the figure
 * came up blank. Moving Mermaid from `@11` to `@11.17.2` did exactly that.
 *
 * So a stored tag is recognised by its jsdelivr package and path, with the
 * version segment left free, and the bytes spliced in are today's pinned ones.
 * A figure therefore renders with the library BioRouter ships now rather than
 * with the one it shipped then, which is the same trade the offline path has
 * always made (the vendored copy is whatever is in the tree).
 */
export const ARTIFACT_CDN_ASSETS = [
  'https://cdn.jsdelivr.net/npm/d3@7.9.0/dist/d3.min.js',
  'https://cdn.jsdelivr.net/npm/d3-sankey@0.12.3/dist/d3-sankey.min.js',
  'https://cdn.jsdelivr.net/npm/chart.js@4.5.0/dist/chart.umd.min.js',
  'https://cdn.jsdelivr.net/npm/leaflet@1.9.4/dist/leaflet.js',
  'https://cdn.jsdelivr.net/npm/leaflet@1.9.4/dist/leaflet.css',
  'https://cdn.jsdelivr.net/npm/leaflet.markercluster@1.5.3/dist/leaflet.markercluster.js',
  // The classic (IIFE) bundle, which ends in `globalThis["mermaid"] = …`. Not
  // the `/+esm` transform: the replacement below produces a *classic* script,
  // so module source spliced into it would be a syntax error.
  'https://cdn.jsdelivr.net/npm/mermaid@11.17.2/dist/mermaid.min.js',
];

const escapeRegExp = (value: string): string => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

const JSDELIVR_NPM = 'https://cdn.jsdelivr.net/npm/';

/**
 * `https://cdn.jsdelivr.net/npm/<package>@<version>/<path>`, split into the
 * package (scope included), the version spec and the path after it.
 */
const JSDELIVR_NPM_URL =
  /^https:\/\/cdn\.jsdelivr\.net\/npm\/((?:@[^/@]+\/)?[^/@]+)@([^/]+)\/(.+)$/;

/**
 * A regex source matching `url`, with the version segment of a jsdelivr npm URL
 * left free so a tag stored under an older pin is still recognised.
 *
 * Everything but the version stays literal: the origin, the `npm/` prefix, the
 * package name and the whole path. The version itself cannot contain a slash, a
 * quote, an angle bracket or whitespace, so it can neither reach into the next
 * path segment nor escape the attribute it sits in. A URL that is not a
 * jsdelivr npm package URL is matched exactly, as before.
 */
export const artifactCdnUrlPattern = (url: string): string => {
  const parsed = JSDELIVR_NPM_URL.exec(url);
  if (!parsed) return escapeRegExp(url);
  const [, packageName, , path] = parsed;
  return `${escapeRegExp(JSDELIVR_NPM + packageName)}@[^/"'<>\\s]+/${escapeRegExp(path)}`;
};

/** The `<script src="…"></script>` shape the rewriter can replace. */
export const artifactCdnScriptPattern = (url: string): RegExp =>
  new RegExp(`<script\\b[^>]*src=["']${artifactCdnUrlPattern(url)}["'][^>]*>\\s*</script>`, 'g');

/** The `<link href="…">` shape the rewriter can replace. */
export const artifactCdnStylePattern = (url: string): RegExp =>
  new RegExp(`<link\\b[^>]*href=["']${artifactCdnUrlPattern(url)}["'][^>]*>`, 'g');

export type ArtifactCdnAssetFetcher = (url: string) => Promise<string>;

/**
 * The global a figure reads to learn which of its libraries never arrived.
 * `templates/_common.js` in the Auto Visualiser reads the same name, and
 * `the_common_script_names_a_missing_library_plainly` in that crate's tests
 * holds the two together.
 */
export const UNAVAILABLE_LIBRARIES_GLOBAL = '__biorouterUnavailableLibraries';

/** The npm package a CDN URL names (`d3`, `leaflet.markercluster`), else the URL. */
const libraryName = (url: string): string => JSDELIVR_NPM_URL.exec(url)?.[1] ?? url;

/**
 * The inline script that takes the place of a library tag the main process
 * could not fill. It records the library for the figure's error card, which then
 * says the library could not be downloaded instead of "d3 is not defined", and
 * tags its render error so the panel does not ask the agent to repair a figure
 * that is not broken. The name comes from `ARTIFACT_CDN_ASSETS`, never from the
 * document, and `<` is escaped so it can never close the script.
 */
const unavailableLibraryMarker = (url: string): string => {
  const name = JSON.stringify(libraryName(url)).replace(/</g, '\\u003c');
  const list = `window.${UNAVAILABLE_LIBRARIES_GLOBAL}`;
  return `<script>(${list}=${list}||[]).push(${name});</script>`;
};

/**
 * Replace every known CDN reference in `rawHtml` with the library's source,
 * inlined. Unknown references are left untouched. A known one that cannot be
 * fetched is replaced with a marker naming the library (the artifact CSP would
 * block the remote tag anyway), so the figure fails visibly and says why rather
 * than silently rendering half a chart.
 */
export const inlineArtifactCdnAssets = async (
  rawHtml: string,
  fetchAsset: ArtifactCdnAssetFetcher,
  onError: (url: string, error: unknown) => void = () => {}
): Promise<string> => {
  let html = rawHtml;
  for (const url of ARTIFACT_CDN_ASSETS) {
    const isStylesheet = url.endsWith('.css');
    const pattern = isStylesheet ? artifactCdnStylePattern(url) : artifactCdnScriptPattern(url);
    // A fresh, non-global copy for the test: `lastIndex` makes `RegExp.test`
    // stateful on a `/g/` pattern, so reusing this one would skip every other
    // document.
    if (!new RegExp(pattern.source).test(html)) continue;
    try {
      const asset = await fetchAsset(url);
      // Replacement must be a function: minified bundles contain `$&`, `$'`,
      // `$1`, `$$` sequences, which String.replace would expand as match
      // references and corrupt the inlined script.
      if (isStylesheet) {
        html = html.replace(pattern, () => `<style>${asset}</style>`);
      } else {
        html = html.replace(pattern, () => `<script>${asset}</script>`);
      }
    } catch (error) {
      html = html.replace(pattern, () => unavailableLibraryMarker(url));
      onError(url, error);
    }
  }
  return html;
};

/** How long one library download may take, body included, before the open gives up. */
export const ARTIFACT_CDN_FETCH_TIMEOUT_MS = 20_000;

/** The part of `fetch` the CDN fetcher uses, so a test can stand in for it. */
export type CdnFetch = (
  url: string,
  init: { signal: AbortSignal }
) => Promise<{ ok: boolean; status: number; text(): Promise<string> }>;

/**
 * An `ArtifactCdnAssetFetcher` that downloads each library once per app session.
 *
 * What it keeps is a library that arrived. A download that fails (offline, an
 * HTTP error, a proxy that resets) is dropped from the cache as it settles, so
 * the next figure that needs the library tries again. It used to keep the
 * rejected promise, and one open while offline then broke every later figure
 * that needed that library until the app was restarted. Figures opened while a
 * download is in flight share it.
 *
 * A download that has not finished within `timeoutMs` is aborted and fails, so
 * an open never waits forever on a silent network. The timeout also settles the
 * open when `fetchImpl` ignores its signal.
 */
export const createCachingCdnFetcher = (
  fetchImpl: CdnFetch,
  { timeoutMs = ARTIFACT_CDN_FETCH_TIMEOUT_MS }: { timeoutMs?: number } = {}
): ArtifactCdnAssetFetcher => {
  const cache = new Map<string, Promise<string>>();

  const download = (url: string): Promise<string> => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timedOut = new Promise<never>((_, reject) => {
      timer = setTimeout(() => {
        const error = new Error(`timed out after ${timeoutMs} ms`);
        controller.abort(error);
        reject(error);
      }, timeoutMs);
    });
    const fetched = (async () => {
      const response = await fetchImpl(url, { signal: controller.signal });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      return response.text();
    })();
    return Promise.race([fetched, timedOut]).finally(() => clearTimeout(timer));
  };

  return (url) => {
    const cached = cache.get(url);
    if (cached) return cached;
    const pending = download(url);
    cache.set(url, pending);
    pending.catch(() => {
      if (cache.get(url) === pending) cache.delete(url);
    });
    return pending;
  };
};
