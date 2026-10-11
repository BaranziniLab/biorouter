// @vitest-environment node
/**
 * The Extensions page's rows, measured in a real layout engine (spec 3.11 acceptance, 5.7's
 * pattern).
 *
 * jsdom lays nothing out, so `ExtensionItem.test.tsx` can say a row is a title line and one muted
 * line, but not that a long description stays on one line, that the section 13.5 provenance
 * sentence is never the part that gives way, that a long name ellipsizes before its badges are
 * pushed past the switch, or that the row's actions appear only when it is hovered. This renders
 * the real `ExtensionItem` and `ExtensionList`'s group label to markup and lays them out in
 * Chromium with `extension-row.css`, the shared list-row and row-actions recipes cut from
 * `main.css` at their owner's markers, and the token values.
 *
 * The page band (44px, hairline, title position) is `Layout/pageBandGeometry.browser.test.tsx`'s,
 * and the dialog (640px, top anchor) is `ModalShell`'s to measure.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';
import type { FixedExtensionEntry } from '../../../ConfigContext';
import { loadRegistry, type RegistryLoad } from '../../../baam/registry';
import ExtensionItem from './ExtensionItem';

// `PrivacyBadge` reads the master switch off the config context; a static render has no provider.
vi.mock('../../../ConfigContext', () => ({ usePrivacyTiersEnabled: () => true }));

const here = dirname(fileURLToPath(import.meta.url));
const main = readFileSync(resolve(here, '../../../../styles/main.css'), 'utf8');

/** `main.css`'s values for the tokens the rows read, and the utilities the markup uses. */
const TOKENS = `:root {
  --font-body: Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, sans-serif;
  --radius-md: 8px;
  --radius-inner: 4px;
  --row-height: 40px;
  --icon-row: 16px;
  --control-md: 32px;
  --border-subtle: #e5e3df;
  --border-focus: #6b6963;
  --text-muted: #6b6963;
  --text-subtle: #8a867f;
  --text-default: #1d1c1a;
  --overlay-hover: rgb(0 0 0 / 5%);
  --dur-fast-min: 95ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
}
* { box-sizing: border-box; }
body { margin: 0; font: 14px/20px var(--font-body); }
h2, p { margin: 0; }
.flex { display: flex; } .inline-flex { display: inline-flex; } .flex-1 { flex: 1 1 0%; }
.items-center { align-items: center; } .min-w-0 { min-width: 0; }
.gap-3 { gap: 12px; } .px-3 { padding-inline: 12px; } .py-2\\.5 { padding-block: 10px; }
.flex-col { flex-direction: column; } .gap-6 { gap: 24px; } .truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.flex-shrink-0 { flex-shrink: 0; } .rounded-inner { border-radius: 4px; }
.h-5 { height: 20px; } .px-1\\.5 { padding-inline: 6px; } .gap-1 { gap: 4px; }
.text-label { font-size: 14px; line-height: 20px; font-weight: 500; }
.text-supporting { font-size: 12px; line-height: 16px; }
.text-chip { font-size: 12px; line-height: 16px; font-weight: 500; }
.text-caps { font-size: 12px; line-height: 16px; font-weight: 500; letter-spacing: 0.04em; text-transform: uppercase; }
.tabular-nums { font-variant-numeric: tabular-nums; }
.text-text-muted { color: var(--text-muted); } .text-text-default { color: var(--text-default); }
.bg-background-medium { background: #ebe9e5; } .bg-background-muted { background: #f3f1ed; }
.h-control-md { height: 32px; } .w-control-md { width: 32px; padding: 0; }
.br-switch { flex: none; width: 32px; height: 20px; border: 0; border-radius: 10px; background: #bbb; padding: 0; }
.br-info-tip { display: inline-flex; width: 24px; height: 24px; margin-block: -5px; margin-inline: -1px -5px; border: 0; padding: 0; background: none; }
.br-info-tip > svg { width: 14px; height: 14px; }
svg { flex: none; }
.sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }`;

/** The shared list-row recipe and the row-actions cluster, cut from `main.css`. */
function sharedCss(): string {
  const rowsBegin = main.indexOf('/* @ws WS-PRIMITIVES begin: rows */');
  const rowsEnd = main.indexOf('/* @ws WS-PRIMITIVES end: rows */');
  if (rowsBegin < 0 || rowsEnd < rowsBegin) throw new Error('row block markers not found');
  const actionsBegin = main.indexOf('.br-row-actions-slot {');
  const actionsEnd = main.indexOf('/* Spinner: one glyph');
  if (actionsBegin < 0 || actionsEnd < actionsBegin) throw new Error('row-actions block not found');
  return (
    '.biorouter-list-shell { display: flex; flex-direction: column; }\n' +
    main.slice(rowsBegin, rowsEnd) +
    '\n' +
    main.slice(actionsBegin, actionsEnd)
  );
}

const rowCss = readFileSync(resolve(here, 'extension-row.css'), 'utf8');

const LONG_NAME =
  'a-very-long-extension-name-that-keeps-going-well-past-any-width-the-row-could-give';
const LONG_DESCRIPTION =
  'A description that runs on for far longer than one line of a 688 pixel row could ever hold, ' +
  'so a layout that wrapped it would turn a two line row into a three line one and push the ' +
  'sentence the privacy design requires off the screen.';

const external = (name: string, description?: string): FixedExtensionEntry =>
  ({
    type: 'stdio',
    name,
    description: description ?? '',
    cmd: 'uv',
    args: [],
    enabled: true,
  }) as FixedExtensionEntry;

const noop = () => {};

function page(catalog: RegistryLoad, columnWidth = 712): string {
  const row = (extension: FixedExtensionEntry, extra: { refuse?: boolean } = {}) =>
    renderToStaticMarkup(
      <ExtensionItem
        extension={extension}
        onToggle={noop}
        onConfigure={noop}
        onRemove={noop}
        catalog={catalog}
        defaultProvider={extra.refuse ? { name: 'Anthropic', tier: 'public' } : null}
      />
    );
  const rows = [
    // 0: description + a marketplace provenance sentence
    row(external('spokeagent', 'SPOKE graph')),
    // 1: a built-in with a description and no visible provenance
    row({
      type: 'builtin',
      name: 'example',
      display_name: 'Example',
      description: 'Example extension',
      enabled: true,
    } as FixedExtensionEntry),
    // 2: a built-in with nothing to say is the only row that is the title alone (an external
    // one always has provenance: a file install says so)
    row({
      type: 'builtin',
      name: 'bare',
      display_name: 'Bare',
      enabled: true,
    } as FixedExtensionEntry),
    // 3: a long description AND the longest provenance sentence (a file install)
    row(external('medcp', LONG_DESCRIPTION)),
    // 4: a long name beside the Built-in and Public badges, and a long description
    row({
      type: 'builtin',
      name: 'longname',
      display_name: LONG_NAME,
      description: LONG_DESCRIPTION,
      enabled: true,
    } as FixedExtensionEntry),
    // 5: a Private marketplace row refused under a public model: the warning badge too
    row(external('ucsfomopagent', 'OMOP'), { refuse: true }),
  ].join('');
  return `<!doctype html><html><head><style>${TOKENS}\n${sharedCss()}\n${rowCss}</style></head><body>
    <div id="column" style="width:${columnWidth}px;padding:0">
      <section>
        <div class="br-ext-group-label"><h2 id="label" class="truncate text-caps text-text-muted">Enabled <span class="tabular-nums">6</span></h2></div>
        <div class="biorouter-list-shell br-rows-flat" id="list">${rows}</div>
      </section>
      <section>
        <!-- The control: the shared row recipe on a shell that did not opt out, so a test can tell a
             line the page's shell removed from a recipe the sheet never drew. -->
        <div class="biorouter-list-shell" id="plain"><div class="biorouter-list-row" id="plain-first">A</div><div class="biorouter-list-row">B</div></div>
      </section>
    </div>
  </body></html>`;
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

// 90s a test: each one launches a page in a Chromium that other browser suites (and other
// agents' runs) are loading at the same time; alone, a test takes about 400ms.
describe('Extension rows in a real layout engine', { timeout: 90_000 }, () => {
  let browser: Browser | null = null;
  let html = '';
  let narrowHtml = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the extension row geometry ' +
          'tests. Run `npx playwright install chromium` to enable them.'
      );
    }
    const catalog = await loadRegistry();
    html = page(catalog);
    // A pane the page can end up in when the window is split or squeezed.
    narrowHtml = page(catalog, 420);
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(narrow = false): Promise<Page> {
    const tab = await browser!.newPage({
      viewport: { width: 1000, height: 1000 },
      reducedMotion: 'reduce',
    });
    await tab.setContent(narrow ? narrowHtml : html);
    return tab;
  }

  /** The box of `selector` inside the nth ROW (or of the nth row itself, or of `#label`). */
  const box = (tab: Page, selector: string, index = 0) =>
    tab.evaluate(
      ([sel, i]) => {
        const rows = document.querySelectorAll('[data-extension-row]');
        const el = (
          sel === '[data-extension-row]'
            ? rows[i as number]
            : sel.startsWith('#')
              ? document.querySelector(sel as string)
              : rows[i as number].querySelector(sel as string)
        ) as HTMLElement;
        const r = el.getBoundingClientRect();
        return {
          left: r.left,
          right: r.right,
          top: r.top,
          bottom: r.bottom,
          width: r.width,
          height: r.height,
          overflows: el.scrollWidth > el.clientWidth,
        };
      },
      [selector, index] as const
    );

  it('draws a row as at most two lines: 57px with a muted line, 53px without', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const withLine = await box(tab, '[data-extension-row]', 0);
      const bare = await box(tab, '[data-extension-row]', 2);
      // 10 + 36 (the name and its muted line) + 10 + the row recipe's 1px hairline room.
      expect(withLine.height).toBe(57);
      // The 32px `⋯` button is taller than a lone title, so it sets a one-line row: 10 + 32 + 10 + 1.
      expect(bare.height).toBe(53);
    } finally {
      await tab.close();
    }
  });

  it('keeps a long description and the provenance sentence on ONE muted line', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      // Row 3 holds the longest description and a file-install provenance sentence.
      const row = await box(tab, '[data-extension-row]', 3);
      const secondary = await box(tab, '.br-ext-secondary', 3);
      const description = await box(tab, '.br-ext-description', 3);
      const provenance = await box(tab, '.br-ext-provenance', 3);

      expect(row.height).toBe(57); // two lines: nothing wrapped to a third
      expect(secondary.height).toBe(16);
      // The part that gives way is the description…
      expect(description.overflows).toBe(true);
      // …never the sentence the privacy design requires: it is whole, on the same line, inside the row.
      expect(provenance.overflows).toBe(false);
      expect(provenance.height).toBe(16);
      expect(provenance.right).toBeLessThanOrEqual(secondary.right + 0.5);
      expect(Math.abs(provenance.top - description.top)).toBeLessThan(2);
    } finally {
      await tab.close();
    }
  });

  it('ellipsizes a long name before its badges are pushed past the switch', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      // Row 4: a long name beside the Built-in and Public badges.
      const row = await box(tab, '[data-extension-row]', 4);
      const title = await box(tab, '.br-ext-title', 4);
      const switchBox = await box(tab, '.br-switch', 4);
      const badges = await tab.evaluate(() =>
        [
          ...document
            .querySelectorAll('[data-extension-row]')[4]
            .querySelectorAll('.br-ext-title-line > *'),
        ]
          .slice(1)
          .map((el) => el.getBoundingClientRect().right)
      );

      expect(title.overflows).toBe(true);
      expect(row.height).toBe(57);
      expect(badges.length).toBeGreaterThanOrEqual(2);
      for (const right of badges) expect(right).toBeLessThanOrEqual(switchBox.left);

      // And the refused-pairing row keeps its warning badge on the title line, inside the row.
      const refused = await tab.evaluate(() => {
        const rowEl = document.querySelectorAll('[data-extension-row]')[5] as HTMLElement;
        const warning = [...rowEl.querySelectorAll('.br-ext-title-line *')].find((el) =>
          el.textContent?.includes('Unavailable in new chats')
        ) as HTMLElement;
        const toggle = rowEl.querySelector('.br-switch') as HTMLElement;
        return {
          warningRight: warning.getBoundingClientRect().right,
          toggleLeft: toggle.getBoundingClientRect().left,
          height: rowEl.getBoundingClientRect().height,
        };
      });
      expect(refused.warningRight).toBeLessThanOrEqual(refused.toggleLeft);
      expect(refused.height).toBe(57);
    } finally {
      await tab.close();
    }
  });

  it('puts the switch at the trailing edge, centred on the row, and the glyph at the leading one', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const row = await box(tab, '[data-extension-row]', 0);
      const toggle = await box(tab, '.br-switch', 0);
      const glyph = await box(tab, '.br-ext-glyph', 0);
      const label = await box(tab, '#label', 0);

      expect(toggle.width).toBe(32);
      expect(row.right - toggle.right).toBe(12);
      expect(Math.abs((toggle.top + toggle.bottom) / 2 - (row.top + row.bottom) / 2)).toBeLessThan(
        1
      );
      expect(glyph.width).toBe(16);
      expect(glyph.left - row.left).toBe(12);
      // The group label shares the glyph's left edge, so the heading and the column read as one.
      expect(label.left).toBe(glyph.left);
    } finally {
      await tab.close();
    }
  });

  it('stacks the provenance sentence under the description in a pane too narrow for both, never under the switch', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(true);
    try {
      const rowCount = await tab.evaluate(
        () => document.querySelectorAll('[data-extension-row]').length
      );
      for (let index = 0; index < rowCount; index += 1) {
        const hasProvenance = await tab.evaluate((i) => {
          const rows = document.querySelectorAll('[data-extension-row]');
          return rows[i].querySelector('.br-ext-provenance') !== null;
        }, index);
        if (!hasProvenance) continue;
        const row = await box(tab, '[data-extension-row]', index);
        const provenance = await box(tab, '.br-ext-provenance', index);
        const toggle = await box(tab, '.br-switch', index);
        // The sentence wraps inside the text column: it ends before the switch and inside the row.
        expect(provenance.right).toBeLessThanOrEqual(toggle.left);
        expect(provenance.right).toBeLessThanOrEqual(row.right);
        expect(provenance.overflows).toBe(false);
      }
      // A row with a description puts the sentence BELOW it, not beside it.
      const description = await box(tab, '.br-ext-description', 0);
      const provenance = await box(tab, '.br-ext-provenance', 0);
      expect(provenance.top).toBeGreaterThanOrEqual(description.top + 16);
    } finally {
      await tab.close();
    }
  });

  it('shows the row menu only when the row is hovered or holds focus', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const opacity = () =>
        tab.evaluate(
          () => getComputedStyle(document.querySelector('.br-row-actions') as HTMLElement).opacity
        );
      expect(await opacity()).toBe('0');
      await tab.hover('[data-extension-row]');
      await tab.waitForTimeout(200);
      expect(await opacity()).toBe('1');
      await tab.mouse.move(900, 900);
      await tab.waitForTimeout(200);
      expect(await opacity()).toBe('0');
      // Keyboard focus reveals it too: the actions stay in the tab order.
      await tab.focus('.br-row-actions button');
      await tab.waitForTimeout(200);
      expect(await opacity()).toBe('1');
    } finally {
      await tab.close();
    }
  });

  it('draws no line under a row: rows are told apart by space and the hover wash (R-27)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      // The shell carries `.br-rows-flat`, the one shared rule (main.css, rows block); this folder
      // has no rule of its own for it.
      const rows = await tab.evaluate(() =>
        [...document.querySelectorAll('[data-extension-row]')].map((row) => {
          const style = getComputedStyle(row);
          return {
            content: getComputedStyle(row, '::after').content,
            // The recipe's 1px of transparent border stays: no row moves for losing its line.
            border: style.borderBottomWidth,
            borderColor: style.borderBottomColor,
          };
        })
      );
      expect(rows.length).toBeGreaterThan(1);
      for (const row of rows) {
        expect(row.content).toBe('none');
        expect(row.border).toBe('1px');
        expect(row.borderColor).toBe('rgba(0, 0, 0, 0)');
      }
      // Two rows touch with nothing between them, so the 20px of padding is the whole gap.
      const [first, second] = await tab.evaluate(() =>
        [...document.querySelectorAll('[data-extension-row]')]
          .slice(0, 2)
          .map((row) => row.getBoundingClientRect())
          .map((rect) => ({ top: rect.top, bottom: rect.bottom }))
      );
      expect(second.top - first.bottom).toBe(0);
    } finally {
      await tab.close();
    }
  });

  it('draws the line back in forced colours, except under the last row', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      // Forced colours repaint every fill as Canvas, so the hover wash that tells a row from the
      // next is gone and the line is all that separates them. The shared rule gives it back; a
      // private `content: none` on the row (which this folder used to carry) would have kept it
      // off, at the same specificity and loaded later.
      await tab.emulateMedia({ forcedColors: 'active' });
      const contents = await tab.evaluate(() =>
        [...document.querySelectorAll('[data-extension-row]')].map(
          (row) => getComputedStyle(row, '::after').content
        )
      );
      expect(contents.length).toBeGreaterThan(2);
      for (const content of contents.slice(0, -1)) expect(content).toBe('""');
      // The shared `:last-child` rule still leaves the last row without one.
      expect(contents[contents.length - 1]).toBe('none');
      await tab.emulateMedia({ forcedColors: 'none' });
      const rest = await tab.evaluate(
        () => getComputedStyle(document.querySelector('[data-extension-row]')!, '::after').content
      );
      expect(rest).toBe('none');
    } finally {
      await tab.close();
    }
  });

  it('leaves the shared row recipe its straight, inset hairline everywhere else', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      // The control is a row of a list that did not opt out: the sheet draws its line, so the
      // page's `none` above is `.br-rows-flat` at work and not a recipe that stopped drawing one.
      const line = await tab.evaluate(() => {
        const row = document.getElementById('plain-first') as HTMLElement;
        const after = getComputedStyle(row, '::after');
        return {
          content: after.content,
          inset: after.insetInlineStart,
          radius: getComputedStyle(row).borderBottomLeftRadius,
        };
      });
      expect(line.content).not.toBe('none');
      // Inset by the row radius, so the line ends where the hover wash starts to curve.
      expect(line.inset).toBe('8px');
      expect(line.radius).toBe('8px');
    } finally {
      await tab.close();
    }
  });
});
