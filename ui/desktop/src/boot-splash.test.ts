import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { GENERATED_THEMES, THEME_FAMILY_IDS } from './styles/themes.generated';

/**
 * These tests run the boot splash's REAL source, extracted out of
 * `index.html`, rather than a copy kept in the test file. The splash has to be
 * inline pre-React markup (see the comment in index.html), which means there is
 * no module to import — so we lift the markup and the script straight out of
 * the shipped page. If either block is renamed or removed, extraction throws
 * instead of quietly testing nothing.
 */

// `import.meta.url` is not a file: URL under Vitest's jsdom transform, so
// resolve from the working directory instead — vitest may be invoked from the
// package dir or from the repo root.
function resolveFromPackage(relative: string): string {
  const candidates = [
    join(process.cwd(), relative),
    join(process.cwd(), 'ui', 'desktop', relative),
  ];
  const found = candidates.find((p) => existsSync(p));
  if (!found) throw new Error(`could not locate ${relative} from ${process.cwd()}`);
  return found;
}

function readIndexHtml(): string {
  return readFileSync(resolveFromPackage('index.html'), 'utf-8');
}

function extractMarkup(html: string): string {
  const match = html.match(/<!-- br-boot:start -->([\s\S]*?)<!-- br-boot:end -->/);
  if (!match) throw new Error('boot splash markup delimiters not found in index.html');
  return match[1];
}

function extractScript(html: string): string {
  const match = html.match(/<script id="br-boot-splash-script">([\s\S]*?)<\/script>/);
  if (!match) throw new Error('boot splash script not found in index.html');
  return match[1];
}

const THRESHOLD_MS = 400;
const DURATION_MS = 420;
const STAGGER_MS = 70;

/** Only the fields the splash actually sets — avoids depending on DOM lib globals. */
type SplashKeyframe = { opacity?: number; transform?: string; filter?: string };
type SplashAnimOptions = {
  duration?: number;
  easing?: string;
  delay?: number;
  fill?: string;
};
type AnimateCall = { keyframes: SplashKeyframe[]; options: SplashAnimOptions };

/** Installs the splash exactly as the browser would, with timers already faked. */
function mountSplash(): void {
  const html = readIndexHtml();
  document.body.innerHTML = extractMarkup(html);
  new Function(extractScript(html))();
}

function splashEl(): HTMLElement | null {
  return document.getElementById('br-boot');
}

describe('boot splash', () => {
  let animateCalls: AnimateCall[];
  let cancelled: number;

  beforeEach(() => {
    vi.useFakeTimers();
    animateCalls = [];
    cancelled = 0;
    document.documentElement.className = '';
    document.documentElement.removeAttribute('data-theme');
    delete (window as { __brBootSplash?: unknown }).__brBootSplash;
  });

  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = '';
    delete (window as { __brBootSplash?: unknown }).__brBootSplash;
    delete (Element.prototype as { animate?: unknown }).animate;
  });

  /** jsdom has no Web Animations API; stand one up so the animated path runs. */
  function stubAnimate() {
    (Element.prototype as unknown as { animate: unknown }).animate = function (
      keyframes: SplashKeyframe[],
      options: SplashAnimOptions
    ) {
      animateCalls.push({ keyframes, options });
      return {
        cancel: () => {
          cancelled += 1;
        },
      };
    };
  }

  /**
   * The splash paints before React, so it cannot import THEME_FAMILIES — it has
   * to restate every family in plain CSS, and the theme-init script has to
   * restate them again in plain JS. That is three lists that can drift.
   *
   * These tests are the lockstep enforcement: add a family to ThemeContext and
   * they fail until the splash and the init script both cover it. Without them
   * a new theme silently boots on Parchment cream — including, at its worst, a
   * cream flash on a dark ground.
   *
   * The splash uses literal hexes rather than var() because it paints before the
   * stylesheet is applied — in dev, Vite injects main.css via JS after first
   * paint, so a var() would resolve to its fallback during exactly the window
   * the splash exists to fill. (An earlier version of this comment, and of the
   * one in index.html, blamed `@theme inline` for compiling the tokens away.
   * That is false: they are ordinary custom properties, verified by probe.)
   *
   * Most of those literals are now GENERATED from each family's
   * --background-muted, so only the base family's light values are hand-written
   * — and those are pinned to parchment.theme.mjs by a test below.
   */
  describe('theme coverage', () => {
    /**
     * The canonical family list. It is GENERATED from themes/*.theme.mjs, so
     * this imports it rather than scraping a source file — the list used to be
     * hand-maintained in three places and this test existed to keep them in
     * step. Now there is one list; the test's job is to prove index.html's
     * pre-React copy still matches it.
     */
    function themeFamilies(): string[] {
      return [...THEME_FAMILY_IDS];
    }

    it('finds more than one family, so the checks below are not vacuous', () => {
      expect(themeFamilies().length).toBeGreaterThan(1);
    });

    it('styles every theme family in both light and dark', () => {
      const html = readIndexHtml();
      const missing: string[] = [];

      for (const family of themeFamilies()) {
        // Parchment is the default and is styled by the bare #br-boot /
        // html.dark rules rather than a [data-theme] selector.
        if (family === 'parchment') {
          if (!/#br-boot\s*\{/.test(html)) missing.push('parchment (light)');
          if (!/html\.dark\s+#br-boot\s*\{/.test(html)) missing.push('parchment (dark)');
          continue;
        }
        const light = new RegExp(`html\\[data-theme='${family}'\\]\\s+#br-boot\\s*\\{`);
        const dark = new RegExp(`html\\.dark\\[data-theme='${family}'\\]\\s+#br-boot\\s*\\{`);
        if (!light.test(html)) missing.push(`${family} (light)`);
        if (!dark.test(html)) missing.push(`${family} (dark)`);
      }

      expect(missing, `boot splash has no rule for: ${missing.join(', ')}`).toEqual([]);
    });

    it('gives every family the ONE shared ground per mode, and never the wrong mode', () => {
      // This assertion used to demand that every family+mode ground be DISTINCT,
      // on the reasoning that a duplicate meant one family was wearing another's
      // background. That reasoning no longer holds: neutrals are now shared
      // infrastructure, so all three families paint the same
      // `--background-muted` by design and duplicates are the CORRECT outcome.
      //
      // The regression it actually protected against survives, though, and is
      // asserted here instead: a family+mode whose rule is missing or malformed
      // falls back to the OTHER mode's ground — which is how a cream splash once
      // flashed on a dark ground. So: exactly one distinct ground per mode, and
      // the two modes must differ. That is strictly stronger than distinctness
      // was, because it also catches a single family drifting off the shared set.
      const html = readIndexHtml();
      const rules = [...html.matchAll(/([^\n{}]*)#br-boot\s*\{[^}]*?--br-bg:\s*([^;]+);/g)].map(
        (m) => ({ dark: m[1].includes('.dark'), bg: m[2].trim() })
      );

      expect(rules.length, 'no #br-boot rules found — did the markers move?').toBeGreaterThan(1);

      const light = new Set(rules.filter((r) => !r.dark).map((r) => r.bg));
      const dark = new Set(rules.filter((r) => r.dark).map((r) => r.bg));

      expect([...light], 'every family must share ONE light splash ground').toHaveLength(1);
      expect([...dark], 'every family must share ONE dark splash ground').toHaveLength(1);
      expect([...light][0], 'the dark ground must not equal the light one').not.toBe([...dark][0]);
    });

    it("pins the base family's hand-written splash values to its definition", () => {
      // The generator deliberately skips the base family's LIGHT rule: this
      // `#br-boot` block IS that rule, and emitting a second copy would give one
      // ground two rules. That makes these four values the one splash cell no
      // generator gate covers — so it is covered here instead, or it drifts
      // silently.
      const html = readIndexHtml();
      const base = html.match(/\n\s*#br-boot\s*\{([\s\S]*?)\n\s*\}/)?.[1] ?? '';
      const read = (name: string) =>
        base.match(new RegExp(`--br-${name}\\s*:\\s*([^;]+);`))?.[1].trim();

      const parchment = GENERATED_THEMES.parchment.light;
      expect(read('navy'), '--br-navy must match parchment.light mark.navy').toBe(
        parchment.mark.navy
      );
      expect(read('coral'), '--br-coral must match parchment.light mark.coral').toBe(
        parchment.mark.coral
      );
      // The ground is --background-muted, which is what makes the hand-off to
      // the booted app free of a colour jump. It is now the SHARED light
      // neutral rather than Parchment's own cream #faf8f3 — the families
      // differ in ink and accent (--br-navy / --br-coral above), not ground.
      expect(read('bg'), '--br-bg must be the shared light --background-muted').toBe('#f4f4f2');
    });

    it('uses CSS comments inside <style>, never HTML comments', () => {
      // `<!--` is not a comment to the CSS parser — it is a CDO token, and a
      // `<!-- ... -->` marker mid-stylesheet makes the parser DISCARD the rule
      // that follows it. Generated-region markers written as HTML comments
      // silently deleted the `html.dark #br-boot` rule, so Parchment dark had no
      // splash rule at all and fell back to the light ground. The rule text was
      // still present in the file, which is why a regex over the source (like
      // the assertions above) could not see the problem — only parsing could.
      const html = readIndexHtml();
      const styleBlocks = [...html.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((m) => m[1]);
      expect(styleBlocks.length).toBeGreaterThan(0);
      for (const block of styleBlocks) {
        expect(block, 'HTML comment inside <style> will eat the next CSS rule').not.toContain(
          '<!--'
        );
      }
    });

    it('gives every family a splash rule the CSS parser actually keeps', () => {
      // Guards the same bug from the other side: each family+mode must have a
      // rule, and each must set its own --br-bg.
      const html = readIndexHtml();
      const style = html.match(/<style[^>]*>([\s\S]*?)<\/style>/)?.[1] ?? '';
      for (const family of themeFamilies()) {
        const isBase = family === 'parchment';
        const light = isBase
          ? /(?:^|\n)\s*#br-boot\s*\{/
          : new RegExp(`html\\[data-theme='${family}'\\]\\s*#br-boot`);
        const dark = isBase
          ? /html\.dark\s*#br-boot/
          : new RegExp(`html\\.dark\\[data-theme='${family}'\\]\\s*#br-boot`);
        expect(style, `${family} light splash rule`).toMatch(light);
        expect(style, `${family} dark splash rule`).toMatch(dark);
      }
    });

    it('keeps the pre-React theme script in lockstep with ThemeContext', () => {
      const html = readIndexHtml();
      const families = themeFamilies();
      const declared = html.match(/const FAMILIES\s*=\s*\[([^\]]+)\]/);

      if (declared) {
        // The script keeps an explicit allow-list: it must match exactly, or a
        // family is either unreachable or settable-but-unstyled.
        const inScript = [...declared[1].matchAll(/'([^']+)'/g)].map((m) => m[1]);
        expect(inScript).toEqual(families);
        return;
      }
      // Older shape: the script branches on family names inline. Each family
      // must at least be named there, or it can never be applied before paint.
      const missing = families.filter((f) => f !== 'parchment' && !html.includes(`'${f}'`));
      expect(missing, `theme script never mentions: ${missing.join(', ')}`).toEqual([]);
    });
  });

  describe('markup', () => {
    it('ships four independently animatable mark parts', () => {
      mountSplash();
      expect(document.querySelectorAll('#br-boot .br-part')).toHaveLength(4);
    });

    it('announces the wait to screen readers, since nothing on screen is text', () => {
      mountSplash();
      const live = document.querySelector('#br-boot .br-sr');
      expect(live?.getAttribute('role')).toBe('status');
      expect(live?.textContent).toContain('Loading BioRouter');
    });

    it('starts hidden so a fast boot paints nothing at all', () => {
      mountSplash();
      expect(splashEl()?.hidden).toBe(true);
    });
  });

  /**
   * jsdom has NO layout engine — `getBoundingClientRect()` is all zeroes here —
   * so these cannot assert the mark's measured position. They pin the two CSS
   * facts that PRODUCE the position instead, which is where the bug actually
   * lived. Both were measured in a real Electron window before and after:
   * the mark's ink centre sat at y=447.59 on the splash but y=461.49 on the
   * loader it hands off to (923px-tall viewport) — a 14px drop, plus a 4px
   * size change, i.e. the logo visibly jumped.
   */
  describe('mark placement (must not move when handing off to the app)', () => {
    /** The splash stylesheet with CSS comments removed, so rule matching is
     *  not thrown off by the (long) explanatory comments between rules. */
    function splashStyle(): string {
      const match = readIndexHtml().match(/<style id="br-boot-splash-style">([\s\S]*?)<\/style>/);
      if (!match) throw new Error('boot splash style block not found in index.html');
      return match[1].replace(/\/\*[\s\S]*?\*\//g, '');
    }

    /**
     * The declaration body of one rule. Anchored on `}` or start-of-sheet so
     * `#br-boot` matches only the BASE rule, never `html.dark #br-boot`.
     */
    function ruleBody(css: string, selector: string): string {
      const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      const match = css.match(new RegExp(`(?:^|\\})\\s*${escaped}\\s*\\{([^}]*)\\}`));
      if (!match) throw new Error(`rule not found: ${selector}`);
      return match[1];
    }

    it('centres the container on the MARK — no gap can offset it', () => {
      const body = ruleBody(splashStyle(), '#br-boot');
      // A `gap` only has an effect if something else shares the flow, and any
      // such sibling shifts the mark by half the extra height. This is the
      // regression: `gap: 26px` + an in-flow sweep pushed the mark 14px up.
      expect(body).toMatch(/justify-content:\s*center/);
      expect(body).not.toMatch(/(^|[\s;])gap\s*:/);
    });

    it('keeps the sweep out of the flex flow, so it cannot displace the mark', () => {
      const body = ruleBody(splashStyle(), '#br-boot .br-sweep');
      expect(body).toMatch(/position:\s*absolute/);
    });

    it('hands off to a loader that draws no logo of its own, so nothing can resize or jump', () => {
      // ProviderGuard's `isChecking` branch is the screen the splash uncovers. It used to draw the
      // same BR monogram (first at Tailwind's h-20 against the splash's 84px, so the logo resized
      // mid-boot), which made the monogram a second logo before the wordmark (vision FR-R1-7,
      // R-29). It now draws nothing on the page ground, so whatever the splash shows fades out
      // over an empty ground and no mark underneath it can sit at another size or centre.
      const guard = readFileSync(
        resolveFromPackage(join('src', 'components', 'ProviderGuard.tsx')),
        'utf-8'
      );
      const loader = guard.match(/if \(isChecking\) \{([\s\S]*?)\n {2}\}\n/);
      expect(loader, "ProviderGuard's isChecking branch not found").not.toBeNull();
      const markup = loader![1].replace(/\/\*[\s\S]*?\*\//g, '');
      expect(markup).not.toMatch(/<BioRouter(Mark|Wordmark)\b/);
      expect(markup).toMatch(/bg-background-canvas/);
    });
  });

  describe('threshold gating', () => {
    it('stays hidden right up to the threshold', () => {
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS - 1);
      expect(splashEl()?.hidden).toBe(true);
      expect(window.__brBootSplash?.isShown()).toBe(false);
    });

    it('appears once the boot is slow enough to warrant it', () => {
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS);
      expect(splashEl()?.hidden).toBe(false);
      expect(window.__brBootSplash?.isShown()).toBe(true);
    });
  });

  describe('dismiss', () => {
    it('removes the splash outright when the boot beat the threshold', () => {
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS - 100);
      window.__brBootSplash?.dismiss();

      expect(splashEl()).toBeNull();
      // The pending show timer must not resurrect it after removal.
      vi.advanceTimersByTime(1000);
      expect(splashEl()).toBeNull();
      expect(window.__brBootSplash?.isShown()).toBe(false);
    });

    it('fades out, then removes, when the splash was already on screen', () => {
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS);
      expect(splashEl()).not.toBeNull();

      window.__brBootSplash?.dismiss();
      // Still present mid-fade so the handoff is not a hard cut.
      expect(splashEl()?.classList.contains('br-out')).toBe(true);

      vi.advanceTimersByTime(400);
      expect(splashEl()).toBeNull();
    });

    it('is idempotent — a second call cannot throw or double-remove', () => {
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS);
      window.__brBootSplash?.dismiss();
      vi.advanceTimersByTime(400);
      expect(() => window.__brBootSplash?.dismiss()).not.toThrow();
      expect(splashEl()).toBeNull();
    });

    it('cancels in-flight animations so they cannot outlive the element', () => {
      stubAnimate();
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS);
      expect(animateCalls.length).toBeGreaterThan(0);

      window.__brBootSplash?.dismiss();
      vi.advanceTimersByTime(400);
      expect(cancelled).toBe(animateCalls.length);
    });
  });

  describe('cascade timing', () => {
    it('staggers the four parts left to right on the agreed keyframes', () => {
      stubAnimate();
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS);

      // Four parts plus one sweep-bar fade.
      const partAnims = animateCalls.slice(0, 4);
      expect(partAnims).toHaveLength(4);

      partAnims.forEach((call, i) => {
        expect(call.options.delay).toBe(i * STAGGER_MS);
        expect(call.options.duration).toBe(DURATION_MS);
        expect(call.options.easing).toBe('cubic-bezier(0.22, 1, 0.36, 1)');
        expect(call.options.fill).toBe('forwards');
      });

      // Each part resolves out of blur, from transparent to opaque.
      const first = partAnims[0];
      expect(first.keyframes[0].opacity).toBe(0);
      expect(first.keyframes[first.keyframes.length - 1].opacity).toBe(1);
      expect(String(first.keyframes[0].filter)).toMatch(/^blur\(/);
      expect(first.keyframes[first.keyframes.length - 1].filter).toBe('blur(0px)');
    });

    it('converts blur into SVG user units, not raw device pixels', () => {
      stubAnimate();
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS);

      // 2px at a 165.672-unit viewBox rendered 84px wide ≈ 3.94 user units.
      // A literal blur(2px) here would mean the blur scaled with the mark.
      const startBlur = String(animateCalls[0].keyframes[0].filter);
      const value = parseFloat(startBlur.replace(/[^0-9.]/g, ''));
      expect(value).toBeCloseTo(2 * (165.672 / 84), 1);
      expect(value).not.toBeCloseTo(2, 1);
    });
  });

  describe('reduced motion', () => {
    it('renders the mark settled instead of cascading it', () => {
      stubAnimate();
      const original = window.matchMedia;
      window.matchMedia = ((query: string) =>
        ({
          matches: query.includes('prefers-reduced-motion'),
          media: query,
          addEventListener: vi.fn(),
          removeEventListener: vi.fn(),
          addListener: vi.fn(),
          removeListener: vi.fn(),
          onchange: null,
          dispatchEvent: vi.fn(),
        }) as unknown as MediaQueryList) as typeof window.matchMedia;

      try {
        mountSplash();
        vi.advanceTimersByTime(THRESHOLD_MS);

        expect(splashEl()?.hidden).toBe(false);
        expect(animateCalls).toHaveLength(0);
        document.querySelectorAll<HTMLElement>('#br-boot .br-part').forEach((part) => {
          expect(part.style.opacity).toBe('1');
        });
      } finally {
        window.matchMedia = original;
      }
    });
  });
});
