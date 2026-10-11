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
/** The longest the first frame waits for the brand face (the script's FONT_WAIT_MS). */
const FONT_WAIT_MS = 250;

/** Installs the splash exactly as the browser would, with timers already faked. */
function mountSplash(): void {
  const html = readIndexHtml();
  document.body.innerHTML = extractMarkup(html);
  new Function(extractScript(html))();
}

function splashEl(): HTMLElement | null {
  return document.getElementById('br-boot');
}

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

describe('boot splash', () => {
  let animateCalls: number;

  beforeEach(() => {
    vi.useFakeTimers();
    animateCalls = 0;
    document.documentElement.className = '';
    document.documentElement.removeAttribute('data-theme');
    delete (window as { __brBootSplash?: unknown }).__brBootSplash;
  });

  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = '';
    delete (window as { __brBootSplash?: unknown }).__brBootSplash;
    delete (Element.prototype as { animate?: unknown }).animate;
    delete (document as { fonts?: unknown }).fonts;
  });

  /**
   * jsdom has no Web Animations API. The splash must never reach for one: it appears at rest, so
   * this stands one up that COUNTS, and the "at rest" cases below assert the count stays 0.
   */
  function stubAnimate() {
    (Element.prototype as unknown as { animate: unknown }).animate = function () {
      animateCalls += 1;
      return { cancel: () => undefined };
    };
  }

  /**
   * jsdom has no FontFaceSet. This stands one up whose `load` the test settles by hand, so the
   * wait for the brand face can be walked through (and a face that never arrives can be timed).
   */
  function stubFonts() {
    const calls: { font: string; text: string | undefined }[] = [];
    let settle: () => void = () => undefined;
    const pending = new Promise<unknown[]>((resolve) => {
      settle = () => resolve([]);
    });
    (document as unknown as { fonts: unknown }).fonts = {
      load: (font: string, text?: string) => {
        calls.push({ font, text });
        return pending;
      },
    };
    return { calls, arrive: settle };
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
   * Most of those literals are now GENERATED: each family's own
   * --background-canvas (the ground the page paints) and the base family's mark
   * inks (the splash draws the wordmark, which does not follow the family), so
   * only the base family's light values are hand-written, and those are pinned
   * to parchment.theme.mjs and to the wordmark's own constants by tests below.
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
      // `--background-canvas` by design and duplicates are the CORRECT outcome.
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

    /** The canvas hexes in parchment.theme.mjs, light then dark (file order). */
    function parchmentCanvas(): { light: string; dark: string; mutedLight: string } {
      const src = readFileSync(resolveFromPackage(join('themes', 'parchment.theme.mjs')), 'utf-8');
      const canvas = [...src.matchAll(/'background-canvas':\s*'(#[0-9a-fA-F]{6})'/g)].map(
        (m) => m[1]
      );
      const muted = [...src.matchAll(/'background-muted':\s*'(#[0-9a-fA-F]{6})'/g)].map(
        (m) => m[1]
      );
      expect(canvas, 'parchment.theme.mjs must define a canvas for light and dark').toHaveLength(2);
      return { light: canvas[0], dark: canvas[1], mutedLight: muted[0] };
    }

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
      // The ground is the PAGE's: --background-canvas, the ground the first-run
      // page and the loader paint (the loader's `bg-background-canvas` is pinned
      // below). It was --background-muted, a grey one step off that ground, so a
      // launch just over the threshold flashed grey before it faded onto white
      // (vision FR-R1-7).
      const canvas = parchmentCanvas();
      expect(read('bg'), '--br-bg must be parchment light --background-canvas').toBe(canvas.light);
      expect(read('bg'), '--br-bg must NOT be the muted grey').not.toBe(canvas.mutedLight);
    });

    it("paints each mode's page canvas in every family, never the muted grey", () => {
      const html = readIndexHtml();
      const canvas = parchmentCanvas();
      const rules = [...html.matchAll(/([^\n{}]*)#br-boot\s*\{[^}]*?--br-bg:\s*([^;]+);/g)].map(
        (m) => ({ dark: m[1].includes('.dark'), bg: m[2].trim() })
      );
      expect(rules.length).toBeGreaterThan(1);
      for (const r of rules) {
        expect(r.bg, r.dark ? 'dark ground' : 'light ground').toBe(
          r.dark ? canvas.dark : canvas.light
        );
      }
    });

    it("gives every family the wordmark's inks, because the wordmark does not follow the family", () => {
      // The splash draws icons/BioRouterWordmark.tsx's lockup, and that component
      // is DELIBERATELY fixed to the brand colours in every family (navy, lifting
      // to teal on a dark ground, over coral). A splash that took Alma Mater's
      // teal or Roche Limit's orange from the family's own `mark` would hand off
      // to a differently coloured logo one frame later. The constants are read
      // from the component's source, so the two cannot drift.
      const word = readFileSync(
        resolveFromPackage(join('src', 'components', 'icons', 'BioRouterWordmark.tsx')),
        'utf-8'
      );
      const ink = (name: string) =>
        word.match(new RegExp(`const ${name} = '(#[0-9a-fA-F]{6})'`))?.[1];
      const navy = ink('NAVY');
      const coral = ink('CORAL');
      const teal = ink('TEAL');
      expect([navy, coral, teal], 'the wordmark constants were not found').not.toContain(undefined);

      const html = readIndexHtml();
      const rules = [
        ...html.matchAll(
          /([^\n{}]*)#br-boot\s*\{[^}]*?--br-navy:\s*([^;]+);[^}]*?--br-coral:\s*([^;]+);/g
        ),
      ].map((m) => ({ dark: m[1].includes('.dark'), navy: m[2].trim(), coral: m[3].trim() }));
      // the hand-written base rule plus the five generated ones
      expect(rules).toHaveLength(themeFamilies().length * 2);
      for (const r of rules) {
        expect(r.navy, r.dark ? 'dark navy lifts to teal' : 'light navy').toBe(
          r.dark ? teal : navy
        );
        expect(r.coral, 'coral is the same in both modes').toBe(coral);
      }
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
    it('draws the wordmark, not the BR monogram (R-29: the text logo is the logo)', () => {
      mountSplash();
      const svg = document.querySelector('#br-boot svg.br-wordmark');
      expect(svg, 'the wordmark lockup is missing').not.toBeNull();
      // "Bio" and "Router" in two inks, over a two-tone bar drawn as two rects.
      const words = [...svg!.querySelectorAll('text tspan')];
      expect(words.map((w) => w.textContent)).toEqual(['Bio', 'Router']);
      expect(words.map((w) => w.getAttribute('class'))).toEqual(['br-navy', 'br-coral']);
      expect(
        [...svg!.querySelectorAll('rect')].map((r) => r.getAttribute('class')),
        'the bar is the navy rect with the coral one drawn over its right half'
      ).toEqual(['br-navy', 'br-coral']);
      // The monogram and its four animatable parts are gone from the launch frame.
      expect(document.querySelector('#br-boot .br-mark')).toBeNull();
      expect(document.querySelector('#br-boot .br-part')).toBeNull();
    });

    it("keeps the lockup's box and its two bar halves consistent with the size the CSS gives it", () => {
      // The numbers were read off a rendered <BioRouterWordmark/> (viewBox, the two rects, the
      // text's size). bootSplash.browser.test.ts renders the component in Chromium and compares
      // against them; this case keeps the shape (a viewBox in the CSS box's ratio, bar halves that
      // end together) honest in jsdom.
      mountSplash();
      const svg = document.querySelector('#br-boot svg.br-wordmark')!;
      const [x, y, w, h] = svg.getAttribute('viewBox')!.split(/\s+/).map(Number);
      expect(w / h, 'the box keeps the ratio the CSS sizes it by (160 x 49)').toBeCloseTo(
        160 / 49,
        1
      );
      expect(x).toBeLessThan(0);
      expect(y).toBeLessThan(0);
      const [navy, coral] = [...svg.querySelectorAll('rect')].map((r) => ({
        x: Number(r.getAttribute('x')),
        w: Number(r.getAttribute('width')),
      }));
      expect(coral.x + coral.w, 'both halves end at the same right edge').toBeCloseTo(
        navy.x + navy.w,
        2
      );
      expect(svg.querySelector('text')!.getAttribute('font-size')).toBe('100');
    });

    it('names the app once, in the brand spelling, and hides the lockup from assistive tech', () => {
      mountSplash();
      const live = document.querySelector('#br-boot .br-sr');
      expect(live?.getAttribute('role')).toBe('status');
      // "Biorouter", lowercase r (src/test/uiCopySpelling.test.ts). The lockup is the logo, and
      // its capital R is the logo's, so it is not announced a second time.
      expect(live?.textContent).toBe('Loading Biorouter…');
      const svg = document.querySelector('#br-boot svg.br-wordmark')!;
      expect(svg.getAttribute('aria-hidden')).toBe('true');
      expect(svg.hasAttribute('aria-label')).toBe(false);
      expect(svg.hasAttribute('role')).toBe(false);
    });

    it('starts hidden so a fast boot paints nothing at all', () => {
      mountSplash();
      expect(splashEl()?.hidden).toBe(true);
    });
  });

  /**
   * jsdom has NO layout engine — `getBoundingClientRect()` is all zeroes here —
   * so these cannot assert the wordmark's measured position. They pin the two CSS
   * facts that PRODUCE the position instead, which is where the bug actually
   * lived. Both were measured in a real Electron window before and after:
   * the old BR monogram's ink centre sat at y=447.59 on the splash but y=461.49
   * on the loader it hands off to (923px-tall viewport), so the logo visibly
   * jumped. bootSplash.browser.test.ts measures the wordmark's centre in
   * Chromium.
   */
  describe('wordmark placement (must not move when handing off to the app)', () => {
    it('centres the container on the WORDMARK, so no gap can offset it', () => {
      const body = ruleBody(splashStyle(), '#br-boot');
      // A `gap` only has an effect if something else shares the flow, and any
      // such sibling shifts the logo by half the extra height. This is the
      // regression: `gap: 26px` + an in-flow sweep pushed the mark 14px up.
      expect(body).toMatch(/justify-content:\s*center/);
      expect(body).not.toMatch(/(^|[\s;])gap\s*:/);
    });

    it('keeps the sweep out of the flex flow, so it cannot displace the wordmark', () => {
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

    it('takes the sweep down with the element, so nothing keeps running after the app is up', () => {
      // The sweep is a CSS animation on a child of #br-boot, so removing the element is what
      // stops it: there is no script-held animation left to cancel.
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS);
      expect(document.querySelector('#br-boot .br-sweep')).not.toBeNull();

      window.__brBootSplash?.dismiss();
      vi.advanceTimersByTime(400);
      expect(document.querySelector('#br-boot .br-sweep')).toBeNull();
      expect(document.querySelector('#br-boot')).toBeNull();
    });
  });

  /**
   * The splash is a launch surface, and a launch surface appears at REST (spec principle 9,
   * vision FR-R1-7): the wordmark is whole in the first frame, with no stagger, no blur and no
   * entrance. It used to cascade four parts in over 700 ms, which made the BR monogram the
   * first thing a new person saw for most of a second. The cascade, its keyframes and the
   * blur-in-user-units arithmetic are gone; what is left to pin is that nothing came back.
   */
  describe('appears at rest', () => {
    it('asks the Web Animations API for nothing, whatever the motion preference', () => {
      stubAnimate();
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS);

      expect(splashEl()?.hidden).toBe(false);
      expect(animateCalls).toBe(0);
    });

    it('sets no inline opacity, transform, filter or animation on anything it draws', () => {
      stubAnimate();
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS);

      const styled = [...document.querySelectorAll('#br-boot *')].filter((n) =>
        n.hasAttribute('style')
      );
      expect(
        styled.map((n) => `${n.tagName}.${n.getAttribute('class')}: ${n.getAttribute('style')}`)
      ).toEqual([]);
    });

    it('gives the wordmark no entrance in CSS either: no opacity ramp, filter, animation or transition', () => {
      const css = splashStyle();
      for (const selector of [
        '#br-boot .br-wordmark',
        '#br-boot .br-wordmark text',
        '#br-boot .br-navy',
        '#br-boot .br-coral',
      ]) {
        expect(ruleBody(css, selector), selector).not.toMatch(
          /(^|[\s;])(opacity|filter|animation|transition|transform)\s*:/
        );
      }
      // Nothing in the sheet fades or blurs a part in, by name.
      expect(css).not.toMatch(/\.br-part\b/);
      expect(css).not.toMatch(/blur\(/);
    });

    it('moves only the sweep, and only through transform', () => {
      const css = splashStyle();
      // The one keyframe set on the screen, and it moves the bar with `transform` alone.
      const keyframes = [...css.matchAll(/@keyframes\s+([\w-]+)\s*\{([\s\S]*?\})\s*\}/g)];
      expect(keyframes.map((k) => k[1])).toEqual(['br-boot-sweep']);
      expect(keyframes[0][2]).toMatch(/transform:\s*translateX\(100%\)/);
      expect(keyframes[0][2]).not.toMatch(/opacity|filter|width|height|left|top/);

      const bar = ruleBody(css, '#br-boot .br-sweep i');
      expect(bar).toMatch(/animation:\s*br-boot-sweep\b[^;]*\binfinite/);
      // The sweep is simply there from the first frame, not faded in behind the logo.
      expect(ruleBody(css, '#br-boot .br-sweep')).not.toMatch(/opacity/);
    });

    it('stops the sweep under reduced motion and leaves the logo exactly as it was', () => {
      const css = splashStyle();
      const reduced = css.match(
        /@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{([\s\S]*?\})\s*\}/
      )?.[1];
      expect(reduced, 'no reduced-motion rule').toBeDefined();
      expect(reduced).toMatch(/#br-boot \.br-sweep i\s*\{\s*animation:\s*none/);
      // Nothing else changes under reduced motion: the logo has no motion to remove.
      expect(reduced).not.toMatch(/\.br-wordmark|\.br-navy|\.br-coral|opacity/);
    });

    it('takes the whole lockup to the one ink forced colours guarantees', () => {
      // Chromium does not force an SVG fill authored as a colour, so without this the navy half
      // keeps its ink on a canvas the person chose (the in-app wordmark measured about 1.3:1).
      const forced = splashStyle().match(
        /@media\s*\(forced-colors:\s*active\)\s*\{([\s\S]*?\})\s*\}/
      )?.[1];
      expect(forced, 'no forced-colors rule').toBeDefined();
      expect(forced).toMatch(/\.br-navy/);
      expect(forced).toMatch(/\.br-coral/);
      expect(forced).toMatch(/fill:\s*CanvasText/);
    });
  });

  /**
   * Inter is `font-display: block` and is requested only when something paints with it, so a
   * splash shown at once drew the bars of the lockup first and its letters later (measured in
   * Chromium: `unloaded` when shown, `loaded` 10 to 20 ms after). The splash therefore loads the
   * face before it shows, and gives up after FONT_WAIT_MS so a slow or missing face never holds
   * the launch frame back.
   */
  describe('the first frame is the whole logo', () => {
    it('loads the brand face at the wordmark weight before it shows', () => {
      const fonts = stubFonts();
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS);

      expect(fonts.calls).toEqual([{ font: '600 100px Inter', text: 'BioRouter' }]);
      // Still hidden: the face has not arrived, so there is no first frame to show yet.
      expect(splashEl()?.hidden).toBe(true);
      expect(window.__brBootSplash?.isShown()).toBe(false);
    });

    it('shows the instant the face arrives', async () => {
      const fonts = stubFonts();
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS);

      fonts.arrive();
      await vi.advanceTimersByTimeAsync(0);
      expect(splashEl()?.hidden).toBe(false);
      expect(window.__brBootSplash?.isShown()).toBe(true);
    });

    it('never holds the splash past FONT_WAIT_MS for a face that does not arrive', () => {
      stubFonts();
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS + FONT_WAIT_MS - 1);
      expect(splashEl()?.hidden).toBe(true);

      vi.advanceTimersByTime(1);
      expect(splashEl()?.hidden).toBe(false);
    });

    it('shows at once when the document has no font loading API at all', () => {
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS);
      expect(splashEl()?.hidden).toBe(false);
    });

    it('shows nothing, and does not throw, when the app mounts while the face is still loading', async () => {
      const fonts = stubFonts();
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS);
      expect(splashEl()?.hidden).toBe(true);

      // The boot finished inside the wait: drop the splash without a flash.
      window.__brBootSplash?.dismiss();
      expect(splashEl()).toBeNull();

      fonts.arrive();
      await vi.advanceTimersByTimeAsync(FONT_WAIT_MS + 50);
      expect(splashEl()).toBeNull();
      expect(window.__brBootSplash?.isShown()).toBe(false);
    });

    it('treats a rejected load like an arrival rather than waiting out the clock', async () => {
      (document as unknown as { fonts: unknown }).fonts = {
        load: () => Promise.reject(new Error('no such face')),
      };
      mountSplash();
      vi.advanceTimersByTime(THRESHOLD_MS);
      await vi.advanceTimersByTimeAsync(0);
      expect(splashEl()?.hidden).toBe(false);
    });
  });
});
