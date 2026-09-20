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
 * Replace every known CDN reference in `rawHtml` with the library's source,
 * inlined. Unknown or unfetchable references are left untouched — the figure
 * then fails visibly rather than silently rendering half a chart.
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
      onError(url, error);
    }
  }
  return html;
};
