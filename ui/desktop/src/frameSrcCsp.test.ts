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
