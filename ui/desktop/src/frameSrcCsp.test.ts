import { describe, it, expect } from 'vitest';
import { readdirSync, readFileSync, existsSync, statSync } from 'node:fs';
import { join } from 'node:path';

/**
 * `frame-src` in the two policies that govern the renderer document, and the
 * fact that makes `'self'` alone the right answer.
 *
 * Until September 2026 both carried `frame-src 'self' blob: https: http:`. The
 * `http:` was recorded — in `docs/desktop-ui/preview-panel/current-state.md` —
 * as load-bearing for the MCP-apps proxy iframe, which ran at the daemon's own
 * `http://127.0.0.1:<port>` origin. That feature was removed whole
 * (`docs/history/mcp-apps-removal/`), and `blob:`/`https:` were never traced to
 * a frame at all: they were inherited, not chosen.
 *
 * WHAT THESE ASSERTIONS ARE, AND ARE NOT. They read source text. A CSP cannot
 * be exercised in jsdom — it has no CSP engine — so this is a proxy for "the
 * emitted policy is this narrow", and the narrowing itself was measured in the
 * running dev app (an Auto Visualiser figure, an Agent Drafter preview card, a
 * PDF, a notebook and a workbook all rendered with zero
 * `securitypolicyviolation` reports). What these assertions add is the thing a
 * one-off measurement cannot: they fail when someone re-widens the directive,
 * or when someone adds the first frame that would need it.
 *
 * The second half is the load-bearing one, and it is deliberately a scan of the
 * EMITTERS rather than of the policy. A policy assertion on its own is satisfied
 * by a policy nobody can use: the day a component frames a URL instead of
 * `srcdoc`, the correct failure is here, at the frame, and not in a bug report
 * about a preview that renders blank.
 *
 * ⚠ AND A THIRD THING A STRING PIN CANNOT SEE: which of the two policies is
 * actually live. These assertions were written believing both applied to the
 * renderer. Only the `<meta>` did — both windows render in a `persist:`
 * partition and the header was installed on `session.defaultSession`, a
 * different `Session` object. The mechanism that makes "the pair" real is
 * asserted in `src/rendererSessionHooks.test.ts`; keep the two files together.
 */

function resolveFromPackage(relative: string): string {
  const candidates = [
    join(process.cwd(), relative),
    join(process.cwd(), 'ui', 'desktop', relative),
  ];
  const found = candidates.find((p) => existsSync(p));
  if (!found) throw new Error(`could not locate ${relative} from ${process.cwd()}`);
  return found;
}

/** The `frame-src …` run out of a full CSP string, without its trailing `;`. */
function frameSrcOf(policy: string): string {
  const match = policy.match(/frame-src([^;]*)/);
  if (!match) throw new Error(`no frame-src directive in policy: ${policy.slice(0, 200)}`);
  return match[1].trim();
}

/** A named directive's source run out of a full CSP string, without its `;`. */
function directiveOf(policy: string, directive: string): string {
  const match = policy.match(new RegExp(`${directive}([^;]*)`));
  if (!match) throw new Error(`no ${directive} directive in policy: ${policy.slice(0, 200)}`);
  return match[1].trim();
}

/** The sole `<directive> …` literal `main.ts` concatenates into the header. */
function mainProcessDirective(directive: string): string {
  const source = readFileSync(resolveFromPackage('src/main.ts'), 'utf-8');
  const code = source.replace(/^[ \t]*\/\/.*$/gm, '');
  const literals = [...code.matchAll(new RegExp(`"(${directive}[^"]*)"`, 'g'))].map((m) => m[1]);
  if (literals.length !== 1) {
    throw new Error(
      `expected exactly one ${directive} literal in main.ts, found ${literals.length}`
    );
  }
  return directiveOf(literals[0], directive);
}

function metaCsp(): string {
  const html = readFileSync(resolveFromPackage('index.html'), 'utf-8');
  const match = html.match(
    /<meta\s+http-equiv="Content-Security-Policy"\s+content="([^"]*)"\s*\/?>/i
  );
  if (!match) throw new Error('index.html has no Content-Security-Policy meta tag');
  return match[1];
}

/**
 * The `frame-src` literal `main.ts` concatenates into the header policy.
 *
 * Whole-line comments are stripped first, for the reason `workspaceChannelCsp`
 * strips them: the directive is discussed at length in the comment directly
 * above it, and a mention there must never be mistaken for the live value.
 */
function mainProcessFrameSrc(): string {
  const source = readFileSync(resolveFromPackage('src/main.ts'), 'utf-8');
  const code = source.replace(/^[ \t]*\/\/.*$/gm, '');
  const literals = [...code.matchAll(/"(frame-src[^"]*)"/g)].map((m) => m[1]);
  if (literals.length !== 1) {
    throw new Error(`expected exactly one frame-src literal in main.ts, found ${literals.length}`);
  }
  return frameSrcOf(literals[0]);
}

/** Every `.ts`/`.tsx` under `src/`, excluding tests and the generated client. */
function rendererSources(): string[] {
  const root = resolveFromPackage('src');
  const out: string[] = [];
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir)) {
      const full = join(dir, entry);
      if (statSync(full).isDirectory()) {
        if (entry === 'api' || entry === '__fixtures__' || entry === 'node_modules') continue;
        walk(full);
        continue;
      }
      if (!/\.tsx?$/.test(entry)) continue;
      if (/\.test\.tsx?$/.test(entry)) continue;
      out.push(full);
    }
  };
  walk(root);
  return out;
}

describe('frame-src', () => {
  it('is `self` alone in the index.html meta policy', () => {
    expect(frameSrcOf(metaCsp())).toBe("'self'");
  });

  it('is `self` alone in the main-process header policy', () => {
    expect(mainProcessFrameSrc()).toBe("'self'");
  });

  it('admits no remote, blob or data frame in either policy', () => {
    for (const frameSrc of [frameSrcOf(metaCsp()), mainProcessFrameSrc()]) {
      const sources = frameSrc.split(/\s+/).filter(Boolean);
      for (const dead of ['http:', 'https:', 'blob:', 'data:', '*']) {
        expect(sources).not.toContain(dead);
      }
    }
  });

  /**
   * The emitter scan. Every `<iframe>` the renderer writes must carry `srcDoc`
   * (or `srcdoc`, for the one built as an HTML string) and no `src`, because
   * `frame-src 'self'` is only the right policy while that holds.
   *
   * `about:srcdoc` is what `permissionPolicy.ts`'s
   * `isAllowedArtifactFrameNavigation` admits on `will-frame-navigate`, so the
   * two guards describe the same closed world from opposite ends.
   */
  it('is not contradicted by any frame the renderer creates', () => {
    const offenders: string[] = [];
    let framesSeen = 0;

    for (const file of rendererSources()) {
      // Comments first, or the scan reports prose. `embeddedBrowser.ts`'s doc
      // block explains why the live browser is a `WebContentsView` and not an
      // `<iframe>`, and an unstripped scan reads that sentence as a frame.
      const source = readFileSync(file, 'utf-8')
        .replace(/\/\*[\s\S]*?\*\//g, '')
        .replace(/^[ \t]*\/\/.*$/gm, '');
      for (const match of source.matchAll(/<iframe\b[\s\S]*?>/g)) {
        framesSeen += 1;
        const attributes = match[0];
        const hasSrcDoc = /\bsrcDoc\b|\bsrcdoc\s*=/.test(attributes);
        const hasSrc = /\bsrc\s*=/.test(attributes);
        if (!hasSrcDoc || hasSrc) {
          offenders.push(`${file}: ${attributes.replace(/\s+/g, ' ').slice(0, 120)}`);
        }
      }
    }

    // Non-vacuous: a scan that found nothing would pass while saying nothing.
    // Four in components plus the one `artifactSecurity` writes as HTML.
    expect(framesSeen).toBeGreaterThanOrEqual(5);
    expect(offenders).toEqual([]);
  });
});

/**
 * `script-src`, and the reason the two policies have to agree on it.
 *
 * `index.html` opens with an inline `<script>` — the pre-hydration theme-family
 * boot that sets `data-theme` before first paint. The `<meta>` policy has always
 * granted `'unsafe-inline'` for it. The header policy did not, and for as long as
 * the header reached no window that cost nothing. The moment it reached the
 * renderer, the two policies INTERSECT and the missing token becomes the binding
 * one: measured on Electron 39.8.10 with this header on the renderer's own
 * session, the boot script does not run in either the packaged `file://`
 * renderer or the dev `http://localhost:517x` one, and vite's react-refresh
 * preamble is inline too.
 *
 * ⚠ This is not permission to widen the effective policy. The meta already
 * permits inline script and already enforces, so nothing about what the renderer
 * may execute changed. Tightening it for real means removing the token from BOTH
 * files and giving the boot script a hash — a separate change, with its own
 * measurement.
 */
describe('script-src', () => {
  it('grants `unsafe-inline` in both policies, for the index.html boot script', () => {
    for (const scriptSrc of [
      directiveOf(metaCsp(), 'script-src'),
      mainProcessDirective('script-src'),
    ]) {
      expect(scriptSrc.split(/\s+/)).toContain("'unsafe-inline'");
    }
  });

  it('is not contradicted by index.html, which really does open with an inline script', () => {
    const html = readFileSync(resolveFromPackage('index.html'), 'utf-8');
    // Non-vacuous: the assertion above is only justified while this is true.
    // An inline `<script>` with no `src`, before the module bundle.
    expect(html).toMatch(/<script>[\s\S]*initializeTheme[\s\S]*<\/script>/);
  });

  it('grants the two policies the same script sources', () => {
    const meta = directiveOf(metaCsp(), 'script-src').split(/\s+/).filter(Boolean).sort();
    const header = mainProcessDirective('script-src').split(/\s+/).filter(Boolean).sort();
    expect(header).toEqual(meta);
  });
});
