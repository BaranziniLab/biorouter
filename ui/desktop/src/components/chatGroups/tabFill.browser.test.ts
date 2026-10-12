// @vitest-environment node
/**
 * The document tabs are fills, not lines (owner message 13, R-27, design.md 9.11), measured in
 * a real engine on the real stylesheet.
 *
 * jsdom runs no CSS, so a component test can only read class names. The claims here are about
 * paint: an inactive tab is plain text on its band, the pointer gives it the ghost hover, the
 * selected tab is the solid fill step with its accent strip, no tab draws a line at rest, and the
 * steps can be SEEN in dark (the old ladder rose toward `--background-default`, which in dark is
 * #1b1b19 on a #171716 band, so its hover measured 1.02:1). `styles/main.css` is compiled by
 * Tailwind with the classes this markup uses, `chat-tabs.css` is appended as Vite would, and the
 * three strips are drawn with the classes their components render (the close and new-tab classes
 * are imported from the module both strips use). Colours are compared with probes that paint the
 * token itself, so a token value can change without this file changing.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { MENU_ROW_CLASS_NAME, MENU_SURFACE_CLASS_NAME } from '../ui/dropdown-menu';
import { TAB_CLOSE_CLASS, TAB_NEW_CLASS } from './tabClasses';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

const tab = (id: string, title: string, active: boolean) =>
  `<div class="br-tab group" id="${id}"${active ? ' data-active="true"' : ''}>` +
  `<button type="button" role="tab" class="flex min-w-0 flex-1 items-center gap-2 bg-transparent text-left">` +
  `<span class="br-tab__label">${title}</span></button>` +
  `<button type="button" class="${TAB_CLOSE_CLASS}" id="${id}-close" aria-label="Close">x</button>` +
  `</div>`;

/** The chat header band (BaseChat's 44px `bg-sidebar` row) holding the inline chat strip. */
const chatBand = (prefix: string, groupActive: boolean) =>
  `<div class="flex h-chrome items-center border-b border-border-subtle bg-sidebar" style="width:900px">` +
  `<div class="br-tabstrip br-tabstrip--inline" id="${prefix}-strip"` +
  `${groupActive ? '' : ' data-group-active="false"'} style="height:100%;flex:1">` +
  tab(`${prefix}-active`, 'Literature review', true) +
  tab(`${prefix}-idle`, 'Cohort query', false) +
  tab(`${prefix}-other`, 'Methods draft', false) +
  `<div class="flex-none"><button type="button" class="${TAB_NEW_CLASS}" id="${prefix}-new">+</button></div>` +
  `</div></div>`;

/** The preview panel's strip: `br-tabstrip` on its own `--sidebar` ground, a scroll box inside. */
const previewStrip =
  `<div class="br-tabstrip h-chrome" style="width:900px">` +
  `<div class="br-tabstrip__scroll flex min-w-0 flex-1 items-center gap-[3px]">` +
  tab('preview-active', 'figure.html', true) +
  tab('preview-idle', 'results.csv', false) +
  `</div><button type="button" class="br-tabstrip__overflow" id="preview-more">v</button></div>`;

/** The terminal dock's strip, on the surface, over the terminal ground. */
const dockStrip =
  `<div class="flex flex-col border-t border-border-subtle bg-background-default" style="width:900px">` +
  `<div class="br-tabstrip br-tabstrip--sm h-10"><div class="flex min-w-0 flex-1 items-center">` +
  tab('dock-active', 'biorouter', true) +
  tab('dock-idle', 'biorouter 2', false) +
  `</div></div><div class="bg-background-muted" style="height:24px"></div></div>`;

/** The overflow ▾ menu with its filter (from eight tabs), on the shared menu recipe. */
const overflowMenu =
  `<div class="${MENU_SURFACE_CLASS_NAME} br-tabstrip__menu w-56" id="menu">` +
  `<div class="br-tabstrip__menu-filter"><svg class="br-tabstrip__menu-filter-icon" id="filter-icon"></svg>` +
  `<input class="br-tabstrip__menu-filter-field" id="filter-field" placeholder="Filter tabs"></div>` +
  `<div class="${MENU_ROW_CLASS_NAME}" role="menuitem"><svg class="h-4 w-4" id="row-icon"></svg>` +
  `<span class="min-w-0 flex-1 truncate" id="row-label">Cohort query</span></div></div>`;

const MARKUP =
  `<div style="display:grid;gap:16px;padding:16px">` +
  chatBand('chat', true) +
  chatBand('unfocused', false) +
  previewStrip +
  dockStrip +
  overflowMenu +
  `</div>`;

const STRIPS = ['chat', 'preview', 'dock'] as const;

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

/** sRGB channels on 0..255 (gamma encoded, not rounded) and alpha on 0..1. */
type Rgba = [number, number, number, number];

/** OKLab to gamma-encoded sRGB on 0..255 (Ottosson's matrices, the sRGB transfer curve). */
const oklabToSrgb = (L: number, a: number, b: number): [number, number, number] => {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  const linear = [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
  return linear.map((v) => {
    const c = Math.min(1, Math.max(0, v));
    return 255 * (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055);
  }) as [number, number, number];
};

/**
 * A computed colour as the engine serialises it: `rgb()`/`rgba()` for a plain colour,
 * `oklab(L a b / α)` for the hover ink, a `color-mix(in oklab, …)`, and `color(srgb …)` for the
 * engine's own conversion of that ink (the check in the hover test). Any other shape throws, so
 * a token that starts computing to a new syntax fails here by name instead of measuring nonsense.
 */
const parseColour = (value: string): Rgba => {
  const num = (s: string, percentOf = 1) =>
    s.endsWith('%') ? (parseFloat(s) / 100) * percentOf : s === 'none' ? 0 : parseFloat(s);
  const rgb = /^rgba?\(([^)]+)\)$/.exec(value);
  if (rgb) {
    const [r, g, b, a = '1'] = rgb[1].split(/[\s,/]+/).filter(Boolean);
    return [num(r, 255), num(g, 255), num(b, 255), num(a)];
  }
  const srgb = /^color\(srgb ([^)]+)\)$/.exec(value);
  if (srgb) {
    const [r, g, b, a = '1'] = srgb[1].split(/[\s/]+/).filter(Boolean);
    return [255 * num(r), 255 * num(g), 255 * num(b), num(a)];
  }
  const oklab = /^oklab\(([^)]+)\)$/.exec(value);
  if (oklab) {
    const [L, a, b, alpha = '1'] = oklab[1].split(/[\s/]+/).filter(Boolean);
    return [...oklabToSrgb(num(L), num(a, 0.4), num(b, 0.4)), num(alpha)];
  }
  throw new Error(`tabFill: no parser for the computed colour ${JSON.stringify(value)}`);
};

/**
 * `top` composited over `under` (source-over), computed in float in gamma-encoded sRGB, the
 * space browsers blend in. Computed rather than painted on a canvas and read back, because
 * 8-bit raster rounding depends on the canvas backend, and the light hover-to-selected bound
 * sits inside one 8-bit step. Measured in Chrome 155 for the 5% hover ink over the light band
 * #f7f7f5: the GPU canvas read back [237,236,234] (1.0217:1 against the selected #eaeae6), the
 * CPU canvas of CI's GPU-less Linux runner [236,236,234] (1.0197:1, failing 1.02), and the
 * blend itself is [236.75,236.5,234.35] (1.0247:1). The bounds are claims about the tokens, so
 * they are measured against the colour the tokens define, which is the same on every runner.
 */
const over = (top: string, under: string): Rgba => {
  const [tr, tg, tb, ta] = parseColour(top);
  const [ur, ug, ub, ua] = parseColour(under);
  const alpha = ta + ua * (1 - ta);
  if (alpha === 0) return [0, 0, 0, 0];
  const mix = (t: number, u: number) => (t * ta + u * ua * (1 - ta)) / alpha;
  return [mix(tr, ur), mix(tg, ug), mix(tb, ub), alpha];
};
const luminance = ([r, g, b]: Rgba) => {
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
};
const ratio = (a: Rgba, b: Rgba) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};

describe('the document tabs are fills, not lines (R-27)', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the tab fill tests.');
      return;
    }
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    css =
      compiler.build(new Scanner({}).scanFiles([{ content: MARKUP, extension: 'html' }])) +
      '\n' +
      readFileSync(resolve(here, 'chat-tabs.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /**
   * Wait until every finite animation and transition on the page has ended, so a
   * measurement reads the tab at rest. A fixed 250 ms was not enough at a load
   * average of 168: the selected tab's settle had not run yet, and the tab measured
   * 0.97 of its 32px.
   */
  const settle = (page: Page) =>
    page.evaluate(() =>
      Promise.all(
        document
          .getAnimations()
          .filter((a) => Number.isFinite(a.effect?.getComputedTiming().endTime))
          .map((a) => a.finished.catch(() => undefined))
      ).then(() => undefined)
    );

  async function open(mode: 'light' | 'dark', forcedColors = false): Promise<Page> {
    const page = await browser!.newPage({ viewport: { width: 960, height: 400 } });
    if (forcedColors) await page.emulateMedia({ forcedColors: 'active' });
    await page.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)">${MARKUP}</body></html>`
    );
    await settle(page);
    return page;
  }

  /** The computed colour a probe inside `within` paints for `value`. */
  const paint = (page: Page, value: string, within = 'body') =>
    page.evaluate(
      ([v, w]) => {
        const probe = document.createElement('div');
        probe.style.backgroundColor = v;
        document.querySelector(w)!.append(probe);
        const out = getComputedStyle(probe).backgroundColor;
        probe.remove();
        return out;
      },
      [value, within] as const
    );

  const look = (page: Page, selector: string) =>
    page.evaluate((s) => {
      const el = document.querySelector(s)!;
      const cs = getComputedStyle(el);
      const box = el.getBoundingClientRect();
      return {
        ground: cs.backgroundColor,
        image: cs.backgroundImage,
        edge: [cs.borderTopColor, cs.borderRightColor, cs.borderBottomColor, cs.borderLeftColor],
        edgeWidth: cs.borderTopWidth,
        radius: cs.borderTopLeftRadius,
        weight: cs.fontWeight,
        shadow: cs.boxShadow,
        before: getComputedStyle(el, '::before').content,
        height: box.height,
        width: box.width,
      };
    }, selector);

  /** The band a strip's tabs sit on, as painted. */
  const band = (page: Page, strip: (typeof STRIPS)[number]) =>
    page.evaluate((s) => {
      const tabEl = document.getElementById(`${s}-idle`)!;
      let el: HTMLElement | null = tabEl.parentElement;
      while (el) {
        const bg = getComputedStyle(el).backgroundColor;
        if (!/rgba\(.*,\s*0\)$/.test(bg) && bg !== 'transparent') return bg;
        el = el.parentElement;
      }
      return getComputedStyle(document.body).backgroundColor;
    }, strip);

  it('draws no line on any tab at rest, and keeps the 32px tab on an 8px corner', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const page = await open(mode);
      try {
        const clear = await paint(page, 'transparent');
        for (const strip of [...STRIPS, 'unfocused']) {
          for (const which of ['active', 'idle']) {
            const t = await look(page, `#${strip}-${which}`);
            const where = `${mode} ${strip} ${which}`;
            expect(t.edgeWidth, `${where} keeps its 1px edge as room`).toBe('1px');
            for (const side of t.edge) expect(side, `${where} draws no edge`).toBe(clear);
            expect(t.shadow, `${where} is flat`).toBe('none');
            expect(t.height, `${where} is --tab-height`).toBe(32);
            expect(t.radius, `${where} takes the element radius`).toBe('8px');
            expect(t.before, `${where} hangs no divider`).toBe('none');
          }
        }
      } finally {
        await page.close();
      }
    }
  });

  it('rests an inactive tab as plain text and the selected one on the fill step with its strip', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const page = await open(mode);
      try {
        const clear = await paint(page, 'transparent');
        const strong = await paint(page, 'var(--background-fill-strong)');
        for (const strip of STRIPS) {
          const idle = await look(page, `#${strip}-idle`);
          const active = await look(page, `#${strip}-active`);
          const ground = await band(page, strip);
          expect(idle.ground, `${mode} ${strip}: an inactive tab is the band`).toBe(clear);
          expect(active.ground, `${mode} ${strip}: the selected fill`).toBe(strong);
          expect(active.image, `${mode} ${strip}: the accent strip`).toMatch(/linear-gradient/);
          expect(active.weight).toBe('600');
          expect(
            ratio(over(active.ground, ground), over(ground, ground)),
            `${mode} ${strip}: the selected fill can be seen on its band`
          ).toBeGreaterThanOrEqual(1.08);
        }
      } finally {
        await page.close();
      }
    }
  });

  it('gives a hovered tab a fill that can be seen in both modes, short of the selected step', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const page = await open(mode);
      try {
        const hover = await paint(page, 'var(--overlay-hover)');
        const selected = await paint(page, 'var(--background-fill-strong)');
        // over() converts the hover ink itself, so it must agree with the engine's own
        // conversion of it to sRGB (a computed value, so no raster rounding enters).
        const engine = parseColour(
          await paint(page, 'color-mix(in srgb, var(--overlay-hover) 100%, transparent 0%)')
        );
        parseColour(hover).forEach((c, i) =>
          expect(
            Math.abs(c - engine[i]),
            `${mode}: the hover ink, channel ${i}, as the engine converts it`
          ).toBeLessThan(i < 3 ? 0.5 : 0.001)
        );
        for (const strip of STRIPS) {
          await page.hover(`#${strip}-idle`);
          await settle(page);
          const t = await look(page, `#${strip}-idle`);
          const ground = await band(page, strip);
          expect(t.ground, `${mode} ${strip}: the ghost hover`).toBe(hover);
          expect(
            ratio(over(t.ground, ground), over(ground, ground)),
            `${mode} ${strip}: the hover is visible on the band (the old ladder was 1.02:1 in dark)`
          ).toBeGreaterThanOrEqual(1.04);
          expect(
            ratio(over(t.ground, ground), over(selected, ground)),
            `${mode} ${strip}: a hovered tab does not read as the selected one`
          ).toBeGreaterThanOrEqual(1.02);
          // The close control shows on hover and is a further step on the hovered tab.
          const close = `#${strip}-idle-close`;
          expect(await page.$eval(close, (el) => getComputedStyle(el).opacity)).toBe('1');
          await page.hover(close);
          await settle(page);
          const x = await look(page, close);
          expect(x.ground, `${mode} ${strip}: the close hover`).toBe(hover);
          await page.mouse.move(950, 390);
        }
        // On the selected fill too: the close hover is a step, not the fill again.
        await page.hover('#chat-active-close');
        await settle(page);
        const strong = await paint(page, 'var(--background-fill-strong)');
        const x = await look(page, '#chat-active-close');
        expect(
          ratio(over(x.ground, strong), over(strong, strong)),
          `${mode}: the close hover on the selected fill`
        ).toBeGreaterThanOrEqual(1.04);
      } finally {
        await page.close();
      }
    }
  });

  it('dims an unfocused group’s selected tab to the quiet fill, without the strip or the weight', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const page = await open(mode);
      try {
        const t = await look(page, '#unfocused-active');
        expect(t.ground).toBe(await paint(page, 'var(--overlay-hover)'));
        expect(t.image).toBe('none');
        expect(t.weight).toBe('400');
        await page.hover('#unfocused-active');
        await settle(page);
        expect((await look(page, '#unfocused-active')).ground).toBe(
          await paint(page, 'var(--background-fill-strong)')
        );
      } finally {
        await page.close();
      }
    }
  });

  it('hovers the "+" and the ▾ on the band’s fill step', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const page = await open(mode);
      try {
        const strong = await paint(page, 'var(--background-fill-strong)');
        for (const id of ['#chat-new', '#preview-more']) {
          await page.hover(id);
          await settle(page);
          expect((await look(page, id)).ground, `${mode} ${id}`).toBe(strong);
        }
      } finally {
        await page.close();
      }
    }
  });

  it('fades only the chat strip ends that have tabs scrolled past them', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open('light');
    try {
      const mask = (attrs: string[]) =>
        page.evaluate((names) => {
          const strip = document.getElementById('chat-strip')!;
          for (const name of ['data-scrolled-start', 'data-scrolled-end']) {
            strip.removeAttribute(name);
          }
          for (const name of names) strip.setAttribute(name, '');
          return getComputedStyle(strip).maskImage;
        }, attrs);
      expect(await mask([]), 'a strip that fits has no mask').toBe('none');
      expect(await mask(['data-scrolled-end'])).toMatch(
        /^linear-gradient\(to left, rgba\(0, 0, 0, 0\)/
      );
      expect(await mask(['data-scrolled-start'])).toMatch(
        /^linear-gradient\(to right, rgba\(0, 0, 0, 0\)/
      );
      const both = await mask(['data-scrolled-start', 'data-scrolled-end']);
      expect(both).toMatch(/32px/);
      expect(both).toMatch(/calc\(100% - 32px\)/);
    } finally {
      await page.close();
    }
  });

  it('starts the overflow menu’s filter glyph and text on the rows’ glyph and label edges', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open('light');
    try {
      const x = await page.evaluate(() => {
        const left = (id: string) => document.getElementById(id)!.getBoundingClientRect().left;
        return {
          filterIcon: left('filter-icon'),
          rowIcon: left('row-icon'),
          filterText: left('filter-field'),
          rowText: left('row-label'),
        };
      });
      // At 12px of padding both sat 4px left of the rows.
      expect(x.filterIcon).toBeCloseTo(x.rowIcon, 0);
      expect(x.filterText).toBeCloseTo(x.rowText, 0);
    } finally {
      await page.close();
    }
  });

  it('rings the whole tab for keyboard focus, clear of its accent strip, and never after a click', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const page = await open(mode);
      try {
        const ring = (id: string) =>
          page.evaluate((s) => {
            const pill = getComputedStyle(document.getElementById(s)!);
            const button = getComputedStyle(document.querySelector(`#${s} > [role='tab']`)!);
            return {
              pill: [pill.outlineStyle, pill.outlineWidth, pill.outlineOffset],
              button: button.outlineStyle,
            };
          }, id);
        await page.keyboard.press('Tab');
        expect(
          await page.evaluate(() => document.activeElement?.closest('.br-tab')?.id),
          'Tab reaches the first tab’s label button'
        ).toBe('chat-active');
        const focused = await ring('chat-active');
        // The ring is on the pill, 4px in: the strip is the pill's bottom 2px.
        expect(focused.pill, `${mode} keyboard focus rings the pill`).toEqual([
          'solid',
          '2px',
          '-4px',
        ]);
        expect(focused.button, `${mode} the label button draws no ring of its own`).toBe('none');
        await page.click('#chat-idle > [role="tab"]');
        await page.waitForTimeout(100);
        expect((await ring('chat-idle')).pill[0], `${mode} no ring after a click`).toBe('none');
        expect((await ring('chat-active')).pill[0]).toBe('none');
      } finally {
        await page.close();
      }
    }
  });

  it('draws every tab’s edge in forced colours and the selected one in Highlight, at the same size', async (ctx) => {
    if (!browser) return ctx.skip();
    const rest = await open('light');
    const forced = await open('light', true);
    try {
      const highlight = await forced.evaluate(() => {
        const probe = document.createElement('div');
        probe.style.backgroundColor = 'Highlight';
        document.body.append(probe);
        return getComputedStyle(probe).backgroundColor;
      });
      for (const strip of STRIPS) {
        const idle = await look(forced, `#${strip}-idle`);
        const active = await look(forced, `#${strip}-active`);
        expect(idle.edge[0], `${strip}: an inactive tab's edge is drawn`).not.toMatch(
          /rgba\(.*,\s*0\)$/
        );
        expect(active.edge[0], `${strip}: the selected tab's edge is Highlight`).toBe(highlight);
        expect(active.edge[0]).not.toBe(idle.edge[0]);
        const before = await look(rest, `#${strip}-active`);
        expect([active.width, active.height], `${strip}: no geometry moves`).toEqual([
          before.width,
          before.height,
        ]);
      }
    } finally {
      await rest.close();
      await forced.close();
    }
  });
});
