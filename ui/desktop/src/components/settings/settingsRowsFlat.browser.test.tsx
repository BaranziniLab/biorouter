// @vitest-environment node
/**
 * Settings rows draw no hairline between them, draw one again under forced colours, and keep a box
 * on every control while the pointer is on their row, in a real cascade (owner message 13, R-27;
 * spec 3.13; the WS-VIEWS-A note on `.br-rows-flat`; WS-PRIMITIVES-4).
 *
 * Settings owns no row rule. The line is switched off by `.br-rows-flat` in `styles/main.css`, with
 * the line given back in a `forced-colors` media block there, and a washed row's controls step back
 * to the page ground in the shared rows block there. `settings.css` is imported by `SettingsView`,
 * so it loads AFTER main.css; a second `content: none` rule of the same specificity in that file
 * would therefore win the forced-colours cascade and leave every Settings row with no separator at
 * all. jsdom runs no stylesheet and cannot see that, and neither can a source grep, because the
 * defect is the ORDER of two correct-looking rules. So the class strings are read out of the real
 * components (`SettingsView`'s reading column and the three lists a Settings dialog draws in a
 * portal), the authored blocks of `main.css` and all of `settings.css` are put on a page in the
 * order the app loads them, and Chromium reports the `::after` each row ends up with and the
 * colours a hovered row and the control in it end up with.
 *
 * Proved the way the neighbouring browser tests are: each case also runs against a sheet with the
 * one thing it guards put back (a duplicate `content: none` rule, the missing class, the control
 * ground removed from the shared block) and expects the defect, so the instrument is shown to see
 * what it claims to.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const read = (path: string) => readFileSync(resolve(here, path), 'utf8');
const MAIN_CSS = read('../../styles/main.css');
const SETTINGS_CSS = read('settings.css');

/** The class string of the first element the pattern matches in a component's source. */
function classesIn(file: string, pattern: RegExp): string {
  const match = read(file).match(pattern);
  if (!match) throw new Error(`${file} no longer has an element matching ${pattern}`);
  return match[1];
}

const PAGE_CLASSES = classesIn(
  'SettingsView.tsx',
  /<ReadableContent size="chat" className="([^"]+)"/
);
const LIST_CLASSES = [
  classesIn('app/ResetPanel.tsx', /<ul className="([^"]*biorouter-settings-list[^"]*)"/),
  classesIn(
    'permission/PermissionModal.tsx',
    /<div className="([^"]*biorouter-settings-list[^"]*)"/
  ),
  classesIn(
    'permission/PermissionRulesModal.tsx',
    /className="([^"]*biorouter-settings-list[^"]*)"\s+role="group"/
  ),
];

const TOKENS = [
  '--radius-md',
  '--row-height',
  '--border-subtle',
  '--border-focus',
  '--overlay-hover',
  '--dur-fast-min',
  '--ease-out',
  '--background-fill',
  '--background-fill-strong',
  '--background-default',
  '--background-canvas',
];

/** main.css's `.dark { ... }` block: the Parchment dark values every family starts from. */
const DARK_BLOCK = (() => {
  const match = MAIN_CSS.match(/\n\.dark \{([\s\S]*?)\n\}/);
  if (!match) throw new Error('main.css no longer has a bare `.dark { ... }` block');
  return match[1];
})();

type Mode = 'light' | 'dark';

/**
 * The declaration of each token in main.css, and of every token those values name in turn
 * (`--radius-md` is `var(--radius-element)`), so a token change reaches this test and no
 * reference is left dangling, which would compute to nothing. Light is the first declaration
 * anywhere in the file; dark is the first one in the `.dark` block, then the file's.
 */
function tokens(mode: Mode = 'light'): string {
  const found = new Map<string, string>();
  const queue = [...TOKENS];
  const declaration = (source: string, name: string) =>
    source.match(new RegExp(`\\n\\s*${name.replace(/-/g, '\\-')}:\\s*([^;]+);`));
  while (queue.length > 0) {
    const name = queue.shift()!;
    if (found.has(name)) continue;
    const match =
      (mode === 'dark' ? declaration(DARK_BLOCK, name) : null) ?? declaration(MAIN_CSS, name);
    if (!match) throw new Error(`main.css does not declare ${name}`);
    const value = match[1].trim();
    found.set(name, value);
    for (const ref of value.matchAll(/var\((--[\w-]+)/g)) queue.push(ref[1]);
  }
  const lines = [...found].map(([name, value]) => `  ${name}: ${value};`);
  return `:root {\n${lines.join('\n')}\n}`;
}

/** One authored block of main.css, between its `@ws` markers. */
function block(owner: string, name: string): string {
  const match = MAIN_CSS.match(
    new RegExp(
      `/\\* @ws ${owner} begin: ${name} \\*/([\\s\\S]*?)/\\* @ws ${owner} end: ${name} \\*/`
    )
  );
  if (!match) throw new Error(`main.css has no "@ws ${owner} begin: ${name}" block`);
  return match[1];
}

/**
 * main.css's own order. `.br-rows-flat` (with its forced-colours half) and the washed row's control
 * grounds are ONE definition, in the shared rows block; WS-VIEWS-A's copy of the flat rule was
 * deleted with it, so this reads no other block.
 */
const MAIN_ROWS = [
  block('WS-PRIMITIVES', 'rows'),
  block('WS-PRIMITIVES', 'settings row hover'),
].join('\n');

/** What a person on the old stylesheet had: this file repeating the rule main.css owns. */
const DUPLICATE_RULE = `
.br-rows-flat :is(.biorouter-settings-row, .biorouter-list-row)::after {
  content: none;
}`;

/**
 * A control as a row holds it: it rests on `--br-control-ground` when a ground names one and on the
 * rest fill otherwise, which is what the Select trigger, a field and a secondary button all do.
 */
const CONTROL = `<span class="probe-control" style="display:inline-block;width:40px;height:16px;background-color:var(--br-control-ground, var(--background-fill))"></span>`;

const rows = (n: number, tag: string, prefix: string) =>
  Array.from(
    { length: n },
    (_, i) =>
      `<${tag} class="biorouter-settings-row" id="${prefix}-${i + 1}">row ${CONTROL}</${tag}>`
  ).join('');

function page(options: { pageClasses?: string } = {}): string {
  const pageClasses = options.pageClasses ?? PAGE_CLASSES;
  return `<div class="${pageClasses}" id="page">${rows(3, 'div', 'page')}</div>
    ${LIST_CLASSES.map(
      (classes, i) =>
        `<ul class="${classes}" id="list-${i}" style="list-style:none;margin:0;padding:0">${rows(3, 'li', `list-${i}`)}</ul>`
    ).join('')}
    <div class="biorouter-settings-list" id="plain">${rows(3, 'div', 'plain')}</div>`;
}

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

describe('Settings rows and the hairline, in a real cascade', () => {
  let browser: Browser | null = null;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) console.warn('No Chromium found (Playwright’s or Google Chrome): skipping.');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(
    css: string,
    options: { forced?: boolean; pageClasses?: string; mode?: Mode } = {}
  ): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 800, height: 800 } });
    await tab.emulateMedia({ forcedColors: options.forced ? 'active' : 'none' });
    await tab.setContent(
      `<!doctype html><meta charset="utf-8"><style>${tokens(options.mode)}\nbody { margin: 0; background: var(--background-canvas); }\n${css}</style><body>${page(options)}</body>`
    );
    return tab;
  }

  const lines = (tab: Page, scope: string) =>
    tab.evaluate(
      (selector) =>
        [...document.querySelectorAll(`${selector} .biorouter-settings-row`)].map(
          (row) => getComputedStyle(row, '::after').content
        ),
      scope
    );

  it('carries the class on the reading column and on all three dialog lists', () => {
    expect(PAGE_CLASSES).toMatch(/\bbr-rows-flat\b/);
    for (const classes of LIST_CLASSES) expect(classes).toMatch(/\bbr-rows-flat\b/);
  });

  it('draws no line between Settings rows, on the page or in a dialog list', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(`${MAIN_ROWS}\n${SETTINGS_CSS}`);
    try {
      expect(await lines(tab, '#page')).toEqual(['none', 'none', 'none']);
      for (let i = 0; i < LIST_CLASSES.length; i++) {
        expect(await lines(tab, `#list-${i}`)).toEqual(['none', 'none', 'none']);
      }
      // A list that has not decided keeps the shared hairline (and no line under its last row).
      expect(await lines(tab, '#plain')).toEqual(['""', '""', 'none']);
    } finally {
      await tab.close();
    }

    const ruled = await open(`${MAIN_ROWS}\n${SETTINGS_CSS}`, {
      pageClasses: PAGE_CLASSES.replace(/\bbr-rows-flat\b/, '').trim(),
    });
    try {
      expect(
        await lines(ruled, '#page'),
        'without the class on the column the page draws its lines again'
      ).toEqual(['""', '""', 'none']);
    } finally {
      await ruled.close();
    }
  }, 120_000);

  it('gives the line back under forced colours, where the hover wash and every fill are gone', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(`${MAIN_ROWS}\n${SETTINGS_CSS}`, { forced: true });
    try {
      expect(await lines(tab, '#page')).toEqual(['""', '""', 'none']);
      for (let i = 0; i < LIST_CLASSES.length; i++) {
        expect(await lines(tab, `#list-${i}`)).toEqual(['""', '""', 'none']);
      }
    } finally {
      await tab.close();
    }

    const duplicated = await open(`${MAIN_ROWS}\n${SETTINGS_CSS}\n${DUPLICATE_RULE}`, {
      forced: true,
    });
    try {
      expect(
        await lines(duplicated, '#page'),
        'a copy of the rule in settings.css beats main.css’s forced-colours rule: no separator at all'
      ).toEqual(['none', 'none', 'none']);
      expect(await lines(duplicated, '#list-0')).toEqual(['none', 'none', 'none']);
    } finally {
      await duplicated.close();
    }
  }, 120_000);

  /** The sRGB bytes a colour lands on when painted over the page ground (alpha included). */
  const painted = (tab: Page, selector: string) =>
    tab.evaluate((sel) => {
      const canvas = document.createElement('canvas');
      canvas.width = canvas.height = 1;
      const context = canvas.getContext('2d')!;
      const layer = (color: string) => {
        context.fillStyle = color;
        context.fillRect(0, 0, 1, 1);
      };
      layer(getComputedStyle(document.body).backgroundColor);
      layer(getComputedStyle(document.querySelector(sel)!).backgroundColor);
      const [r, g, b] = context.getImageData(0, 0, 1, 1).data;
      return [r, g, b];
    }, selector);

  const channel = (value: number) => {
    const c = value / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  const luminance = ([r, g, b]: number[]) =>
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
  const contrast = (a: number[], b: number[]) => {
    const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
  };

  /** What a person sees with the pointer on the middle row of the page: the row and its control. */
  async function underPointer(css: string, mode: Mode) {
    const tab = await open(css, { mode });
    try {
      await tab.hover('#page-2');
      // The row eases into its wash; a read mid-transition is still the resting transparent.
      await tab.evaluate(() =>
        Promise.all(document.getAnimations().map((animation) => animation.finished))
      );
      return {
        row: await painted(tab, '#page-2'),
        control: await painted(tab, '#page-2 .probe-control'),
        ground: await painted(tab, 'body'),
      };
    } finally {
      await tab.close();
    }
  }

  // The page is the canvas (`MainPanelLayout`), not the default ground. The shared wash is the page
  // ink at 5%, and on the canvas in dark it lands one step off the page ground a washed row's
  // controls step back to: a control is 1.03:1 off its row and has no box. Settings paints the
  // card fill there instead. A box is 1.06:1 or more; the cases below sit at about 1.10 and 1.03.
  const A_BOX = 1.06;
  const SHARED_GROUND = /--br-control-ground:\s*var\(--background-default\);/;
  const SETTINGS_PAINT = /\.br-rows-flat\s+\.biorouter-settings-row:not\([^{]*\{[^}]*\}/;

  for (const mode of ['light', 'dark'] as const) {
    it(`keeps a control's box while the pointer is on its row, ${mode}`, async (ctx) => {
      if (!browser) return ctx.skip();
      const settings = SETTINGS_CSS.match(SETTINGS_PAINT);
      expect(settings, 'settings.css paints the hovered row').not.toBeNull();

      const real = await underPointer(`${MAIN_ROWS}\n${SETTINGS_CSS}`, mode);
      expect(real.row, 'the row under the pointer is washed').not.toEqual(real.ground);
      expect(
        contrast(real.control, real.row),
        'the control reads as a box on the washed row'
      ).toBeGreaterThanOrEqual(A_BOX);

      const unstepped = await underPointer(
        `${MAIN_ROWS.replace(SHARED_GROUND, '')}\n${SETTINGS_CSS}`,
        mode
      );
      expect(
        contrast(unstepped.control, unstepped.row),
        'without the shared block’s control ground the box is gone'
      ).toBeLessThan(A_BOX);

      const washed = await underPointer(
        `${MAIN_ROWS}\n${SETTINGS_CSS.replace(SETTINGS_PAINT, '')}`,
        mode
      );
      const alone = contrast(washed.control, washed.row);
      if (mode === 'dark') {
        expect(alone, 'on the canvas in dark the shared wash alone leaves no box').toBeLessThan(
          A_BOX
        );
      } else {
        expect(alone, 'in light the shared wash alone is enough').toBeGreaterThanOrEqual(A_BOX);
      }
    }, 120_000);
  }
});
