// @vitest-environment node
/**
 * InfoTip's "hover peeks, click pins" look, under the whole stylesheet in Chromium.
 *
 * A hover or a Tab PREVIEWS the tip and leaves the trigger unmarked once the pointer is elsewhere;
 * a click, tap, Enter or Space KEEPS the same tip and marks the trigger (`data-pinned`): the glyph
 * takes the strong fill on its 20px mark box, an `asChild` target a pressed plate laid over its
 * own fill and 2px past it. Keyboard focus wins over every state, and forced colours, which repaint
 * every fill as Canvas, get a 1px CanvasText edge for the kept mark instead.
 *
 * jsdom runs no cascade, has no `::before` and paints nothing, so the real components are bundled
 * with esbuild, `styles/main.css` is compiled by Tailwind with the classes the bundle uses, and
 * Chromium paints them. Colours are read off the painted page (a 1px screenshot) and compared with
 * probes that paint the token itself in the same place.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { InfoTip } from './info-tip';
import { Badge } from './badge';
import { PrivacyBadge } from './PrivacyBadge';
import { AffiliationBadge } from './AffiliationBadge';

const UCSF = { kind: 'institutions', institutions: [{ id: 'ucsf', display_name: 'UCSF' }] };

function App() {
  return (
    <div style={{ display: 'grid', gap: 32, padding: 40, justifyItems: 'start', width: 520 }}>
      <button type="button" id="before">Before</button>
      <p id="glyph-line" style={{ margin: 0 }}>
        Spellcheck<InfoTip label="Spellcheck" help="Checks spelling in the chat input." data-testid="glyph" />
      </p>
      <p style={{ margin: 0 }}>
        <InfoTip label="Legacy" help="Made before the format existed." asChild>
          <Badge id="badge">Legacy format</Badge>
        </InfoTip>
      </p>
      <p id="padlock-alone" style={{ margin: 0 }}>
        <PrivacyBadge tier="private" dense enforcementOff={false} />
      </p>
      <button type="button" id="chip" style={{ display: 'inline-flex', gap: 6, alignItems: 'center' }}>
        Model
        <PrivacyBadge tier="private" dense enforcementOff={false} />
        <AffiliationBadge affiliation={UCSF} dense />
      </button>
      <p id="affiliation-alone" style={{ margin: 0 }}>
        <AffiliationBadge affiliation={UCSF} />
      </p>
      <div id="scroller" style={{ height: 80, width: 300, overflow: 'auto' }}>
        <div style={{ height: 400, paddingTop: 20 }}>
          In a scroller<InfoTip label="Scrolled" help="Sits in a scroller." data-testid="scrolled" />
        </div>
      </div>
      <button type="button" id="after">After</button>
      <div id="empty" style={{ width: 200, height: 60 }} />
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<App />);
`;

const launchChromium = async (): Promise<Browser | null> => {
  for (const options of [{}, { channel: 'chromium' }, { channel: 'chrome' }] as const) {
    try {
      return await chromium.launch(options);
    } catch {
      // try the next build
    }
  }
  return null;
};

type Rgb = [number, number, number];

/** WCAG relative luminance of a gamma-encoded sRGB colour on 0..255. */
const luminance = ([r, g, b]: Rgb) => {
  const lin = (c: number) => {
    const v = c / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
};
const ratio = (a: Rgb, b: Rgb) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};
/** Two painted colours that are the same up to rounding. */
const same = (a: Rgb, b: Rgb) => a.every((v, i) => Math.abs(v - b[i]) <= 2);

describe('InfoTip: hover peeks, click pins, in Chromium', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the InfoTip state tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: { 'process.env.NODE_ENV': '"production"' },
      loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    css = compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }]));
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(mode: 'light' | 'dark', forcedColors = false): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 700, height: 900 } });
    if (forcedColors) await tab.emulateMedia({ forcedColors: 'active' });
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style>` +
        // Transitions off, so a sample reads the state, not a frame of the way to it.
        `<style>*, *::before, *::after { transition: none !important; }</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#after');
    await tab.waitForTimeout(100);
    return tab;
  }

  /** The colour the page paints at one CSS pixel. */
  const pixel = async (tab: Page, x: number, y: number): Promise<Rgb> => {
    const shot = await tab.screenshot({
      clip: { x: Math.round(x), y: Math.round(y), width: 1, height: 1 },
    });
    return (await tab.evaluate(async (b64) => {
      const img = new Image();
      img.src = 'data:image/png;base64,' + b64;
      await img.decode();
      const c = document.createElement('canvas');
      c.width = c.height = 1;
      const g = c.getContext('2d')!;
      g.drawImage(img, 0, 0);
      return Array.from(g.getImageData(0, 0, 1, 1).data.slice(0, 3));
    }, shot.toString('base64'))) as Rgb;
  };

  /**
   * What a token paints on the page ground, composited the way the page composites it: a probe
   * the size of a few pixels, painted and read back.
   */
  const paint = async (tab: Page, value: string): Promise<Rgb> => {
    await tab.evaluate((v) => {
      const probe = document.createElement('div');
      probe.id = 'probe';
      probe.style.cssText = `position:fixed;left:650px;top:850px;width:8px;height:8px;background:${v}`;
      document.body.append(probe);
    }, value);
    const out = await pixel(tab, 654, 854);
    await tab.evaluate(() => document.getElementById('probe')?.remove());
    return out;
  };

  const box = (tab: Page, selector: string) =>
    tab.evaluate((s) => {
      const r = document.querySelector(s)!.getBoundingClientRect();
      return {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
        width: r.width,
        height: r.height,
      };
    }, selector);
  const tipOpen = (tab: Page) =>
    tab.evaluate(
      () =>
        document.querySelector('[data-slot="info-tip-content"]')?.getAttribute('data-state') ===
        'open'
    );
  const pinned = (tab: Page, selector: string) =>
    tab.evaluate((s) => document.querySelector(s)!.hasAttribute('data-pinned'), selector);
  /** The background a pseudo-element computes to, against what a probe computes for `value`. */
  const markGround = (tab: Page, selector: string) =>
    tab.evaluate(
      (s) => getComputedStyle(document.querySelector(s)!, '::before').backgroundColor,
      selector
    );
  const groundOf = (tab: Page, value: string) =>
    tab.evaluate((v) => {
      const probe = document.createElement('div');
      probe.style.backgroundColor = v;
      document.body.append(probe);
      const out = getComputedStyle(probe).backgroundColor;
      probe.remove();
      return out;
    }, value);
  const ink = (tab: Page, selector: string) =>
    tab.evaluate((s) => getComputedStyle(document.querySelector(s)!).color, selector);
  const inkOf = (tab: Page, value: string) =>
    tab.evaluate((v) => {
      const probe = document.createElement('span');
      probe.style.color = v;
      document.body.append(probe);
      const out = getComputedStyle(probe).color;
      probe.remove();
      return out;
    }, value);

  it('marks the glyph only when kept: rest, hover, preview elsewhere, kept, press, focus', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const g = await box(tab, '[data-testid="glyph"]');
        // A point on the 20px mark box (2px inside the 24px hit box) clear of the 14px glyph.
        const mark = () => pixel(tab, g.left + 3, g.top + g.height / 2);
        const ground = await paint(tab, 'var(--background-default)');
        const hoverWash = await paint(tab, 'var(--overlay-hover)');
        const kept = await paint(tab, 'var(--background-fill-strong)');
        const pressed = await paint(tab, 'var(--overlay-pressed)');
        // Focus draws a 2px inset edge over the pixel `mark` reads, so it is read as computed.
        const focus = await groundOf(tab, 'var(--background-focus)');
        const subtle = await inkOf(tab, 'var(--text-subtle)');
        const strongInk = await inkOf(tab, 'var(--text-default)');

        expect(same(await mark(), ground), `${mode} rest`).toBe(true);
        expect(await ink(tab, '[data-testid="glyph"]'), `${mode} rest ink`).toBe(subtle);

        // Pointer on it, before the preview opens: the hover wash and the default ink.
        await tab.mouse.move(g.left + g.width / 2, g.top + g.height / 2);
        await tab.waitForTimeout(60);
        expect(await tipOpen(tab), 'not open before 200ms').toBe(false);
        expect(same(await mark(), hoverWash), `${mode} hover`).toBe(true);
        expect(await ink(tab, '[data-testid="glyph"]')).toBe(strongInk);

        // The same wash while a preview shows: the pointer, not the preview, is what it marks.
        await tab.waitForTimeout(300);
        expect(await tipOpen(tab)).toBe(true);
        expect(same(await mark(), hoverWash), `${mode} hover with the preview`).toBe(true);

        // A preview with the pointer elsewhere (on the tip): the rest look, no mark.
        const tip = await box(tab, '[data-slot="info-tip-content"]');
        await tab.mouse.move(tip.left + 6, tip.top + 6);
        await tab.waitForTimeout(250);
        expect(await tipOpen(tab), 'the pointer on the tip keeps the preview').toBe(true);
        expect(await pinned(tab, '[data-testid="glyph"]')).toBe(false);
        expect(same(await mark(), ground), `${mode} preview, pointer elsewhere`).toBe(true);
        expect(await ink(tab, '[data-testid="glyph"]')).toBe(subtle);

        // A press shows the pressed wash at once.
        await tab.mouse.move(g.left + g.width / 2, g.top + g.height / 2);
        await tab.mouse.down();
        expect(same(await mark(), pressed), `${mode} press`).toBe(true);
        await tab.mouse.up();

        // That click kept the previewed tip: the strong fill, held with the pointer on it...
        expect(await pinned(tab, '[data-testid="glyph"]')).toBe(true);
        expect(same(await mark(), kept), `${mode} kept, pointer on it`).toBe(true);
        // ...and away from it, where pointer leave no longer closes anything.
        await tab.mouse.move(600, 860);
        await tab.waitForTimeout(300);
        expect(await tipOpen(tab), 'kept: pointer leave leaves it open').toBe(true);
        expect(same(await mark(), kept), `${mode} kept, pointer elsewhere`).toBe(true);
        expect(await ink(tab, '[data-testid="glyph"]')).toBe(strongInk);

        // The step is honest: subtle against hover, more against rest (the docs quote both).
        const keptVsHover = ratio(kept, hoverWash);
        const keptVsRest = ratio(kept, ground);
        expect(keptVsRest, `${mode} kept against rest`).toBeGreaterThan(keptVsHover);
        expect(keptVsRest, `${mode} kept against rest`).toBeGreaterThanOrEqual(1.1);
        expect(keptVsHover, `${mode} kept against hover`).toBeGreaterThanOrEqual(1.03);
        console.info(
          `InfoTip ${mode}: kept/rest ${keptVsRest.toFixed(2)}:1, kept/hover ${keptVsHover.toFixed(2)}:1`
        );

        // A second click closes it, and the mark goes with it.
        await tab.mouse.click(g.left + g.width / 2, g.top + g.height / 2);
        await tab.mouse.move(600, 860);
        await tab.waitForTimeout(250);
        expect(await tipOpen(tab)).toBe(false);
        expect(same(await mark(), ground), `${mode} closed`).toBe(true);

        // Keyboard focus wins over every state, kept included.
        await tab.focus('#before');
        await tab.keyboard.press('Tab');
        await tab.waitForTimeout(30);
        expect(await tipOpen(tab), 'Tab previews it at once').toBe(true);
        expect(await markGround(tab, '[data-testid="glyph"]'), `${mode} focus`).toBe(focus);
        await tab.keyboard.press('Enter');
        expect(await pinned(tab, '[data-testid="glyph"]')).toBe(true);
        expect(await markGround(tab, '[data-testid="glyph"]'), `${mode} focus wins over kept`).toBe(
          focus
        );
        await tab.keyboard.press('Escape');
        expect(await tipOpen(tab)).toBe(false);
      } finally {
        await tab.close();
      }
    }
  });

  it('keeps a kept tip through a blur and a scroll; a preview closes on its ancestor scroll', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light');
    try {
      const s = await box(tab, '[data-testid="scrolled"]');
      const centre = { x: s.left + s.width / 2, y: s.top + s.height / 2 };
      await tab.mouse.move(centre.x, centre.y);
      await tab.waitForTimeout(300);
      expect(await tipOpen(tab)).toBe(true);
      await tab.evaluate(() => {
        document.getElementById('scroller')!.scrollTop = 30;
      });
      await tab.waitForTimeout(150);
      expect(await tipOpen(tab), 'a preview closes when its scroller moves it').toBe(false);

      await tab.evaluate(() => {
        document.getElementById('scroller')!.scrollTop = 0;
      });
      await tab.mouse.click(centre.x, centre.y);
      expect(await tipOpen(tab)).toBe(true);
      expect(await pinned(tab, '[data-testid="scrolled"]')).toBe(true);
      await tab.evaluate(() => {
        document.getElementById('scroller')!.scrollTop = 30;
      });
      await tab.waitForTimeout(150);
      expect(await tipOpen(tab), 'a kept tip ignores the scroll').toBe(true);

      // A blur with nowhere for focus to go, which is what a window switch does to it.
      await tab.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
      await tab.waitForTimeout(100);
      expect(await tipOpen(tab), 'a kept tip outlives the blur').toBe(true);

      // A press outside closes it.
      const empty = await box(tab, '#empty');
      await tab.mouse.click(empty.left + 20, empty.top + 20);
      await tab.waitForTimeout(150);
      expect(await tipOpen(tab)).toBe(false);

      // Focus that really moves elsewhere in the page closes a kept tip too.
      await tab.evaluate(() => {
        document.getElementById('scroller')!.scrollTop = 0;
      });
      const again = await box(tab, '[data-testid="scrolled"]');
      await tab.mouse.click(again.left + again.width / 2, again.top + again.height / 2);
      expect(await tipOpen(tab)).toBe(true);
      await tab.keyboard.press('Tab');
      await tab.waitForTimeout(100);
      expect(await tab.evaluate(() => document.activeElement?.id)).toBe('after');
      expect(await tipOpen(tab), 'focus moved on: the kept tip closes').toBe(false);
    } finally {
      await tab.close();
    }
  });

  it('lays a pressed plate over a kept asChild badge and 2px past it, its own fill untouched at rest', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const b = await box(tab, '#badge');
        const y = b.top + b.height / 2;
        const inside = () => pixel(tab, b.left + 2, y); // the badge's own padding
        const rim = () => pixel(tab, b.left - 1, y); // 1px outside it
        const ground = await paint(tab, 'var(--background-default)');
        const badgeFill = await paint(tab, 'var(--background-fill-strong)');

        expect(same(await inside(), badgeFill), `${mode} rest fill`).toBe(true);
        expect(same(await rim(), ground), `${mode} rest rim`).toBe(true);
        const restInk = await ink(tab, '#badge');

        // Hover: the hover wash over the badge and on the rim.
        await tab.mouse.move(b.left + b.width / 2, y);
        await tab.waitForTimeout(60);
        const hoverInside = await inside();
        const hoverRim = await rim();
        expect(same(hoverInside, badgeFill), `${mode} hover changes the badge`).toBe(false);
        expect(same(hoverRim, ground), `${mode} hover reaches the rim`).toBe(false);

        // Kept: a deeper plate, held with the pointer away, and the badge's ink is its own.
        await tab.mouse.click(b.left + b.width / 2, y);
        await tab.mouse.move(600, 860);
        await tab.waitForTimeout(300);
        expect(await tipOpen(tab)).toBe(true);
        expect(await pinned(tab, '#badge')).toBe(true);
        const keptInside = await inside();
        const keptRim = await rim();
        expect(ratio(keptRim, ground), `${mode} kept plate off the page`).toBeGreaterThanOrEqual(
          1.1
        );
        expect(ratio(keptInside, badgeFill), `${mode} kept over the badge's fill`).toBeGreaterThan(
          ratio(hoverInside, badgeFill)
        );
        expect(await ink(tab, '#badge'), 'the ink is the badge’s meaning').toBe(restInk);
        console.info(
          `InfoTip asChild ${mode}: kept plate/page ${ratio(keptRim, ground).toFixed(2)}:1, ` +
            `kept over the badge/badge ${ratio(keptInside, badgeFill).toFixed(2)}:1, ` +
            `hover over the badge/badge ${ratio(hoverInside, badgeFill).toFixed(2)}:1`
        );
        const shadow = await tab.evaluate(
          () => getComputedStyle(document.getElementById('badge')!).boxShadow
        );
        expect(shadow, 'the plate reaches 2px out').toMatch(/0px 0px 0px 2px/);

        // A second click closes it and the plate goes.
        await tab.mouse.click(b.left + b.width / 2, y);
        await tab.mouse.move(600, 860);
        await tab.waitForTimeout(250);
        expect(same(await rim(), ground), `${mode} closed`).toBe(true);

        // Keyboard focus is the one mark while it is there: its outline, no plate.
        await tab.focus('#glyph-line button');
        await tab.keyboard.press('Tab');
        expect(await tab.evaluate(() => document.activeElement?.id)).toBe('badge');
        await tab.keyboard.press('Enter');
        expect(await pinned(tab, '#badge'), 'Enter keeps an asChild tip').toBe(true);
        const focused = await tab.evaluate(() => {
          const st = getComputedStyle(document.getElementById('badge')!);
          return { outline: `${st.outlineStyle} ${st.outlineWidth}`, shadow: st.boxShadow };
        });
        expect(focused.outline).toBe('solid 2px');
        expect(focused.shadow).toBe('none');
        await tab.keyboard.press(' ');
        expect(await pinned(tab, '#badge'), 'Space closes it').toBe(false);
      } finally {
        await tab.close();
      }
    }
  });

  it('draws a kept mark as a 1px CanvasText edge in forced colours', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light', true);
    try {
      const outlineOf = (selector: string, pseudo?: string) =>
        tab.evaluate(
          ([s, p]) => {
            const st = getComputedStyle(document.querySelector(s)!, p ?? null);
            return `${st.outlineStyle} ${st.outlineWidth} ${st.outlineOffset}`;
          },
          [selector, pseudo] as const
        );
      const g = await box(tab, '[data-testid="glyph"]');
      const edge = () => pixel(tab, g.left + 2, g.top + g.height / 2); // the mark box's outer pixel
      const canvas = await edge();

      expect(await outlineOf('[data-testid="glyph"]', '::before')).toMatch(/^none /);
      await tab.mouse.click(g.left + g.width / 2, g.top + g.height / 2);
      await tab.mouse.move(600, 860);
      await tab.waitForTimeout(250);
      expect(await pinned(tab, '[data-testid="glyph"]')).toBe(true);
      expect(await outlineOf('[data-testid="glyph"]', '::before')).toBe('solid 1px -1px');
      // Painted, not only computed: the edge stands off the forced Canvas.
      expect(ratio(await edge(), canvas), 'the kept edge is visible').toBeGreaterThanOrEqual(3);
      await tab.keyboard.press('Escape');

      const b = await box(tab, '#badge');
      await tab.mouse.click(b.left + b.width / 2, b.top + b.height / 2);
      await tab.mouse.move(600, 860);
      await tab.waitForTimeout(250);
      expect(await pinned(tab, '#badge')).toBe(true);
      expect(await outlineOf('#badge')).toBe('solid 1px 1px');
      // The edge is 1px wide, 1px out: the pixel 2px before the badge's left side.
      const badgeEdge = await pixel(tab, b.left - 2, b.top + b.height / 2);
      expect(ratio(badgeEdge, canvas), 'the kept badge edge is visible').toBeGreaterThanOrEqual(3);
    } finally {
      await tab.close();
    }
  });

  it('makes a standalone dense badge its own InfoTip, and leaves one inside a button plain', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light');
    try {
      const facts = await tab.evaluate(() => {
        const read = (el: Element) => ({
          tabindex: el.getAttribute('tabindex'),
          title: el.getAttribute('title'),
          describedBy: el.getAttribute('aria-describedby'),
          description: el.getAttribute('aria-describedby')
            ? document.getElementById(el.getAttribute('aria-describedby')!)?.textContent
            : null,
        });
        return {
          padlock: read(document.querySelector('#padlock-alone [data-testid="privacy-badge"]')!),
          affiliation: read(
            document.querySelector('#affiliation-alone [data-testid="affiliation-badge"]')!
          ),
          chipPadlock: read(document.querySelector('#chip [data-testid="privacy-badge"]')!),
          chipAffiliation: read(document.querySelector('#chip [data-testid="affiliation-badge"]')!),
        };
      });
      expect(facts.padlock).toMatchObject({
        tabindex: '0',
        title: null,
        description:
          'Private. Biorouter only lets private models, decision models included, read this chat.',
      });
      expect(facts.affiliation.tabindex).toBe('0');
      expect(facts.affiliation.title).toBeNull();
      expect(facts.affiliation.description).toMatch(/Compliance does not transfer/);
      for (const inChip of [facts.chipPadlock, facts.chipAffiliation]) {
        expect(inChip).toEqual({
          tabindex: null,
          title: null,
          describedBy: null,
          description: null,
        });
      }

      // A hover on the padlock in the chip opens nothing of its own.
      const p = await box(tab, '#chip [data-testid="privacy-badge"]');
      await tab.mouse.move(p.left + p.width / 2, p.top + p.height / 2);
      await tab.waitForTimeout(350);
      expect(await tab.locator('[data-slot="info-tip-content"]').count()).toBe(0);

      // The standalone padlock opens its words on hover.
      const alone = await box(tab, '#padlock-alone [data-testid="privacy-badge"]');
      await tab.mouse.move(alone.left + alone.width / 2, alone.top + alone.height / 2);
      await tab.waitForTimeout(350);
      expect(await tipOpen(tab)).toBe(true);
      expect(await tab.locator('[data-slot="info-tip-content"]').textContent()).toBe(
        'Private. Biorouter only lets private models, decision models included, read this chat.'
      );
    } finally {
      await tab.close();
    }
  });
});
