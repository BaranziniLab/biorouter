// @vitest-environment node
/**
 * The paste box's "Will fetch" chips rest on the page ground inside the filled card, in a real
 * layout engine (V3-R8-1; design.md 9.11 rule 3: a fill never sits on a fill).
 *
 * Paste text that contains a URL and the box lists each address as a toggle chip. The chip was a
 * neutral `Badge`, which rests on `--background-fill-strong`, and the box is a filled card on
 * `--background-fill`: the chip drew 1.09:1 off the card in light, a fill on a fill, while every
 * other control in the card (Title, the textarea, Stage) rests on `--background-default`. The
 * on and off states differed only by the strike-through and the muted ink.
 *
 * This lays the real `PasteTextBox` out in Chromium, with `styles/main.css` compiled by Tailwind
 * over the classes the bundle uses (so a utility that never generated reads as the old fill), in
 * light and dark, and measures the chip against the card and against its sibling controls:
 *  - at rest the chip is the page ground, the same as the Title field and Stage, in both states;
 *  - it steps to the strong fill under the pointer, as those controls do;
 *  - pressed (`on`) and not (`off`) differ by fill as well as strike-through: only `on` carries the
 *    selected wash;
 *  - a keyboard focus still shows (the grey focus surface and its 2px inset edge);
 *  - toggling moves nothing: the chip and the box keep their size and place.
 * Set `PASTE_CHIP_SHOTS` to a directory to also save the screenshots the review reads.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../../package.json'));
const SHOTS = process.env.PASTE_CHIP_SHOTS;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { PasteTextBox } from './PasteTextBox';

createRoot(document.getElementById('root')!).render(
  <div id="rail" style={{ width: 288, padding: 16, background: 'var(--background-default)' }}>
    <PasteTextBox onStage={() => {}} onCancel={() => {}} />
  </div>
);
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

const URL_TEXT =
  'See https://example.org/papers/fig-2 and https://doi.org/10.1000/xyz123 for more.';
const CARD = '#rail > div';
const AREA = '#rail textarea[aria-label="Text to stage"]';
const TITLE = '#rail input[aria-label="Title for the pasted source"]';
const STAGE = '#rail button.br-fill-rest';
const CHIPS = '#rail button[aria-pressed]';

type Look = {
  bg: string;
  image: string;
  ink: string;
  decoration: string;
  shadow: string;
  x: number;
  y: number;
  w: number;
  h: number;
};

describe.each(['light', 'dark'] as const)('the paste box "Will fetch" chips (%s)', (mode) => {
  let browser: Browser | null = null;
  let page: Page | null = null;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the paste chip tests.');
      return;
    }
    const built = await build({
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
    const script = built.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    const css = compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }]));

    page = await browser.newPage({
      viewport: { width: 420, height: 640 },
      reducedMotion: 'reduce',
    });
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div>` +
        `<div id="probe-default" style="background:var(--background-default)"></div>` +
        `<div id="probe-fill" style="background:var(--background-fill)"></div>` +
        `<div id="probe-strong" style="background:var(--background-fill-strong)"></div>` +
        `<div id="probe-focus" style="background:var(--background-focus)"></div></body></html>`
    );
    await page.addScriptTag({ content: script });
    await page.waitForSelector(AREA);
    await page.fill(AREA, URL_TEXT);
    await page.waitForSelector(CHIPS);
    await page.mouse.move(0, 0);
    expect(errors, 'page errors').toEqual([]);
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /**
   * One element's painted look, read after a beat: the wash is a transition on `--tint-ink`
   * (`tint-interactive`), so a read straight after a press or a hover sees the old value.
   */
  const look = async (selector: string, index = 0): Promise<Look> => {
    await page!.waitForTimeout(220);
    return page!.evaluate(
      ([sel, i]) => {
        const el = document.querySelectorAll(sel as string)[i as number] as HTMLElement;
        const cs = getComputedStyle(el);
        const box = el.getBoundingClientRect();
        return {
          bg: cs.backgroundColor,
          image: cs.backgroundImage,
          ink: cs.color,
          decoration: cs.textDecorationLine,
          shadow: cs.boxShadow,
          x: Math.round(box.x * 10) / 10,
          y: Math.round(box.y * 10) / 10,
          w: Math.round(box.width * 10) / 10,
          h: Math.round(box.height * 10) / 10,
        };
      },
      [selector, index]
    );
  };

  /** A token's painted colour, read off a probe element so `color-mix` and the like resolve. */
  const token = (probe: 'default' | 'fill' | 'strong' | 'focus') =>
    page!.evaluate(
      (id) => getComputedStyle(document.getElementById(`probe-${id}`)!).backgroundColor,
      probe
    );

  it('lists each detected address as a toggle chip inside the filled card', async (ctx) => {
    if (!page) return ctx.skip();
    expect(await page.locator(CHIPS).count()).toBe(2);
    const card = await look(CARD);
    expect(card.bg).toBe(await token('fill'));
  });

  it('rests on the page ground in both states, the same as the Title field and Stage', async (ctx) => {
    if (!page) return ctx.skip();
    const ground = await token('default');
    const card = await look(CARD);
    expect(card.bg, 'the card is the fill').not.toBe(ground);

    // Both chips start pressed ("on"); press the second to read an "off" one.
    const on = await look(CHIPS, 0);
    await page.locator(CHIPS).nth(1).click();
    await page.mouse.move(0, 0);
    const off = await look(CHIPS, 1);
    try {
      expect(on.bg, 'on chip').toBe(ground);
      expect(off.bg, 'off chip').toBe(ground);
      expect((await look(TITLE)).bg).toBe(ground);
      expect((await look(STAGE)).bg).toBe(ground);
      if (SHOTS) {
        mkdirSync(SHOTS, { recursive: true });
        await page
          .locator('#rail')
          .screenshot({ path: join(SHOTS, `paste-chip-rest-${mode}.png`) });
      }
    } finally {
      await page.locator(CHIPS).nth(1).click();
      await page.mouse.move(0, 0);
    }
  });

  it('differs between on and off by fill as well as by strike-through', async (ctx) => {
    if (!page) return ctx.skip();
    const on = await look(CHIPS, 0);
    await page.locator(CHIPS).nth(1).click();
    await page.mouse.move(0, 0);
    const off = await look(CHIPS, 1);
    try {
      // The selected wash is a background-image over the ground; the off chip has none.
      expect(on.image).toMatch(/linear-gradient/);
      expect(on.image).not.toBe(off.image);
      expect(off.decoration).toBe('line-through');
      expect(on.decoration).toBe('none');
      expect(on.ink).not.toBe(off.ink);
    } finally {
      await page.locator(CHIPS).nth(1).click();
      await page.mouse.move(0, 0);
    }
  });

  it('steps to the strong fill under the pointer', async (ctx) => {
    if (!page) return ctx.skip();
    await page.hover(CHIPS);
    try {
      expect((await look(CHIPS, 0)).bg).toBe(await token('strong'));
      // The second chip is not under the pointer and keeps resting.
      expect((await look(CHIPS, 1)).bg).toBe(await token('default'));
      if (SHOTS) {
        mkdirSync(SHOTS, { recursive: true });
        await page
          .locator('#rail')
          .screenshot({ path: join(SHOTS, `paste-chip-hover-${mode}.png`) });
      }
    } finally {
      await page.mouse.move(0, 0);
    }
  });

  it('still shows keyboard focus: the grey focus surface and its inset edge', async (ctx) => {
    if (!page) return ctx.skip();
    await page.focus(AREA);
    await page.keyboard.press('Tab');
    try {
      const active = await page.evaluate(
        () => (document.activeElement as HTMLElement | null)?.getAttribute('aria-pressed') ?? null
      );
      expect(active, 'focus reached the first chip').not.toBeNull();
      const chip = await look(CHIPS, 0);
      expect(chip.bg).toBe(await token('focus'));
      expect(chip.shadow).not.toBe('none');
      if (SHOTS) {
        mkdirSync(SHOTS, { recursive: true });
        await page
          .locator('#rail')
          .screenshot({ path: join(SHOTS, `paste-chip-focus-${mode}.png`) });
      }
    } finally {
      await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
    }
  });

  it('moves nothing when a chip is toggled or hovered', async (ctx) => {
    if (!page) return ctx.skip();
    const before = { chip: await look(CHIPS, 0), card: await look(CARD) };
    await page.hover(CHIPS);
    const hovered = { chip: await look(CHIPS, 0), card: await look(CARD) };
    await page.locator(CHIPS).nth(0).click();
    await page.mouse.move(0, 0);
    const toggled = { chip: await look(CHIPS, 0), card: await look(CARD) };
    try {
      for (const state of [hovered, toggled]) {
        expect([state.chip.x, state.chip.y, state.chip.w, state.chip.h]).toEqual([
          before.chip.x,
          before.chip.y,
          before.chip.w,
          before.chip.h,
        ]);
        expect([state.card.w, state.card.h]).toEqual([before.card.w, before.card.h]);
      }
    } finally {
      await page.locator(CHIPS).nth(0).click();
      await page.mouse.move(0, 0);
    }
  });
});
