import { describe, expect, it, vi } from 'vitest';
import {
  ARTIFACT_CDN_ASSETS,
  artifactCdnScriptPattern,
  artifactCdnStylePattern,
  createCachingCdnFetcher,
  inlineArtifactCdnAssets,
} from './artifactCdnAssets';

const MERMAID = 'https://cdn.jsdelivr.net/npm/mermaid@11.17.2/dist/mermaid.min.js';
const LEAFLET_CSS = 'https://cdn.jsdelivr.net/npm/leaflet@1.9.4/dist/leaflet.css';
const D3 = 'https://cdn.jsdelivr.net/npm/d3@7.9.0/dist/d3.min.js';
const CHART = 'https://cdn.jsdelivr.net/npm/chart.js@4.5.0/dist/chart.umd.min.js';

/** The marker a figure reads to learn which library never arrived. */
const missing = (name: string) =>
  `<script>(window.__biorouterUnavailableLibraries=window.__biorouterUnavailableLibraries||[]).push("${name}");</script>`;

/** A `fetch` answer carrying only what the fetcher reads. */
const answer = (body: string, status = 200) => ({
  ok: status >= 200 && status < 300,
  status,
  text: async () => body,
});

describe('artifactCdnAssets', () => {
  it('covers Mermaid, which the artifact CSP would otherwise block outright', () => {
    expect(ARTIFACT_CDN_ASSETS).toContain(MERMAID);
  });

  it('replaces a CDN script tag with the library source, inlined', async () => {
    const html = `<head><script src="${MERMAID}" crossorigin="anonymous"></script></head>`;
    const out = await inlineArtifactCdnAssets(html, async () => 'globalThis.mermaid={};');

    expect(out).toBe('<head><script>globalThis.mermaid={};</script></head>');
    expect(out).not.toContain('cdn.jsdelivr.net');
  });

  it('does not expand $-sequences in a minified bundle', async () => {
    const html = `<head><script src="${MERMAID}"></script></head>`;
    const source = `var a="$&",b="$'",c="$1",d="$$";`;
    expect(await inlineArtifactCdnAssets(html, async () => source)).toContain(source);
  });

  it('inlines a stylesheet as a style element', async () => {
    const html = `<head><link rel="stylesheet" href="${LEAFLET_CSS}"/></head>`;
    expect(await inlineArtifactCdnAssets(html, async () => '.leaflet{}')).toBe(
      '<head><style>.leaflet{}</style></head>'
    );
  });

  // A tag left in place is blocked by the artifact CSP and the figure then
  // fails with "mermaid is not defined", which reads as a bug in the figure. The
  // marker tells the figure what really happened.
  it('replaces the tag of a library it could not fetch with a marker, and reports it', async () => {
    const html = `<head><script src="${MERMAID}"></script></head>`;
    const onError = vi.fn();
    const out = await inlineArtifactCdnAssets(
      html,
      async () => {
        throw new Error('offline');
      },
      onError
    );

    expect(out).toBe(`<head>${missing('mermaid')}</head>`);
    expect(onError).toHaveBeenCalledWith(MERMAID, expect.any(Error));
  });

  it('an_unfetchable_library_is_recorded_for_the_figure', async () => {
    const html =
      `<head><script src="${D3}"></script><script src="${CHART}"></script>` +
      `<link rel="stylesheet" href="${LEAFLET_CSS}"/></head>`;
    const out = await inlineArtifactCdnAssets(html, async (url) => {
      if (url === CHART) return 'globalThis.Chart={};';
      throw new TypeError('fetch failed');
    });

    expect(out).toBe(
      `<head>${missing('d3')}<script>globalThis.Chart={};</script>${missing('leaflet')}</head>`
    );
    expect(out).not.toContain('cdn.jsdelivr.net');
  });

  // A figure's HTML is stored in the session, so a tag written under an older
  // pin outlives that pin. Matching the URL as an exact string meant a version
  // bump blanked every Mermaid diagram already in someone's history: the stored
  // tag stopped matching, nothing was inlined, and the artifact CSP blocked the
  // remote script as well.
  describe('a figure stored under an older pin', () => {
    const OLD_MERMAID = 'https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js';
    const OLDER_MERMAID = 'https://cdn.jsdelivr.net/npm/mermaid@10.9.0/dist/mermaid.min.js';

    it('is still recognised, and gets the currently pinned source', async () => {
      const html = `<head><script src="${OLD_MERMAID}" crossorigin="anonymous"></script></head>`;
      const fetched: string[] = [];
      const out = await inlineArtifactCdnAssets(html, async (url) => {
        fetched.push(url);
        return 'globalThis.mermaid={};';
      });

      expect(out).toBe('<head><script>globalThis.mermaid={};</script></head>');
      // The bytes come from today's pin, not from the version the tag names.
      expect(fetched).toEqual([MERMAID]);
    });

    it('is recognised whatever version the tag names', async () => {
      for (const url of [OLD_MERMAID, OLDER_MERMAID, MERMAID]) {
        const html = `<head><script src="${url}"></script></head>`;
        expect(await inlineArtifactCdnAssets(html, async () => 'x;')).toBe(
          '<head><script>x;</script></head>'
        );
      }
    });

    it('matches a stylesheet under an older pin too', async () => {
      const html = `<head><link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/leaflet@1.9.3/dist/leaflet.css"/></head>`;
      expect(await inlineArtifactCdnAssets(html, async () => '.leaflet{}')).toBe(
        '<head><style>.leaflet{}</style></head>'
      );
    });

    it('does not fetch for a document that references nothing known', async () => {
      const fetchAsset = vi.fn();
      const html = '<head><script src="https://example.com/app.js"></script></head>';
      expect(await inlineArtifactCdnAssets(html, fetchAsset)).toBe(html);
      expect(fetchAsset).not.toHaveBeenCalled();
    });
  });

  // The version segment is the only free part. Everything else stays literal,
  // so our source can never be spliced into a URL that merely looks similar.
  describe('stays anchored to the jsdelivr package path', () => {
    const leftAlone = [
      // A different origin that ends in the same path.
      'https://evil.example/npm/mermaid@11/dist/mermaid.min.js',
      // The jsdelivr URL as a query parameter on somebody else's host.
      'https://evil.example/?u=https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js',
      // A different package whose name merely contains ours.
      'https://cdn.jsdelivr.net/npm/mermaid-extras@1.0.0/dist/mermaid.min.js',
      // A different file under the same package.
      'https://cdn.jsdelivr.net/npm/mermaid@11.17.2/dist/mermaid.esm.min.mjs',
      // An extra path segment where the version goes.
      'https://cdn.jsdelivr.net/npm/mermaid@11/nested/dist/mermaid.min.js',
      // jsdelivr's gh endpoint, not npm.
      'https://cdn.jsdelivr.net/gh/mermaid-js/mermaid@11/dist/mermaid.min.js',
    ];

    for (const url of leftAlone) {
      it(`leaves ${url} untouched`, async () => {
        const html = `<head><script src="${url}"></script></head>`;
        const fetchAsset = vi.fn();
        expect(await inlineArtifactCdnAssets(html, fetchAsset)).toBe(html);
        expect(fetchAsset).not.toHaveBeenCalled();
      });
    }

    // `leaflet` and `leaflet.markercluster` are the pair that makes this worth
    // asserting: one package name is a prefix of the other.
    it('never matches another asset in the list', () => {
      for (const own of ARTIFACT_CDN_ASSETS) {
        const pattern = own.endsWith('.css')
          ? artifactCdnStylePattern(own)
          : artifactCdnScriptPattern(own);
        for (const other of ARTIFACT_CDN_ASSETS) {
          if (other === own) continue;
          const tag = other.endsWith('.css')
            ? `<link rel="stylesheet" href="${other}"/>`
            : `<script src="${other}"></script>`;
          expect([own, other, new RegExp(pattern.source).test(tag)]).toEqual([own, other, false]);
        }
      }
    });

    it('cannot let a version spec escape the attribute it sits in', () => {
      const attack = `<script src="https://cdn.jsdelivr.net/npm/mermaid@11"></script><script src="x/dist/mermaid.min.js"></script>`;
      expect(artifactCdnScriptPattern(MERMAID).test(attack)).toBe(false);
    });
  });

  it('cannot rewrite an ESM import, which is why assets must be emitted as src tags', async () => {
    // Not a wish for future support: the replacement produces a *classic*
    // script, so module source spliced into it would be a syntax error. The Rust
    // side is what has to emit a `src=` tag — asserted in
    // `crates/biorouter-mcp/tests/autovis_cdn_desktop_contract.rs`.
    const esm = `<head><script type="module">import mermaid from '${MERMAID}/+esm';</script></head>`;
    expect(artifactCdnScriptPattern(MERMAID).test(esm)).toBe(false);
    expect(await inlineArtifactCdnAssets(esm, async () => 'globalThis.mermaid={};')).toBe(esm);
  });
});

describe('createCachingCdnFetcher', () => {
  // One offline open used to cache the rejection for the whole app session, so
  // every later figure needing that library failed until the app restarted.
  it('a_failed_fetch_is_retried_on_the_next_open', async () => {
    const fetchImpl = vi
      .fn()
      .mockRejectedValueOnce(new TypeError('fetch failed'))
      .mockResolvedValueOnce(answer('', 503))
      .mockResolvedValue(answer('globalThis.d3={};'));
    const fetchAsset = createCachingCdnFetcher(fetchImpl, { timeoutMs: 1_000 });

    await expect(fetchAsset(D3)).rejects.toThrow('fetch failed');
    await expect(fetchAsset(D3)).rejects.toThrow('HTTP 503');
    await expect(fetchAsset(D3)).resolves.toBe('globalThis.d3={};');
    // What is kept is the library, once it has arrived.
    await expect(fetchAsset(D3)).resolves.toBe('globalThis.d3={};');
    expect(fetchImpl).toHaveBeenCalledTimes(3);
  });

  it('shares one download between figures opened at the same time', async () => {
    let finish: (value: ReturnType<typeof answer>) => void = () => {};
    const fetchImpl = vi.fn(
      () =>
        new Promise<ReturnType<typeof answer>>((resolve) => {
          finish = resolve;
        })
    );
    const fetchAsset = createCachingCdnFetcher(fetchImpl, { timeoutMs: 1_000 });

    const first = fetchAsset(D3);
    const second = fetchAsset(D3);
    finish(answer('globalThis.d3={};'));

    await expect(Promise.all([first, second])).resolves.toEqual([
      'globalThis.d3={};',
      'globalThis.d3={};',
    ]);
    expect(fetchImpl).toHaveBeenCalledTimes(1);
  });

  it('a_fetch_that_hangs_times_out', async () => {
    const signals: AbortSignal[] = [];
    const fetchImpl = vi
      .fn()
      // A request that never answers and ignores its signal: the timeout must
      // still settle the open rather than leave the panel loading forever.
      .mockImplementationOnce((_url: string, init: { signal: AbortSignal }) => {
        signals.push(init.signal);
        return new Promise(() => {});
      })
      .mockResolvedValue(answer('globalThis.d3={};'));
    const fetchAsset = createCachingCdnFetcher(fetchImpl, { timeoutMs: 20 });

    await expect(fetchAsset(D3)).rejects.toThrow('timed out after 20 ms');
    // The request it gave up on is cancelled, not left running.
    expect(signals[0].aborted).toBe(true);
    // And the next open tries again.
    await expect(fetchAsset(D3)).resolves.toBe('globalThis.d3={};');
    expect(fetchImpl).toHaveBeenCalledTimes(2);
  });
});
