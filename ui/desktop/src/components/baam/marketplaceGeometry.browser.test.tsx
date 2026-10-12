// @vitest-environment node
/**
 * The BAAM browse dialogs' list, measured in a real layout engine (spec 3.11
 * acceptance, 5.7's pattern).
 *
 * jsdom lays nothing out, so `MarketplaceDialog.test.tsx` can say a row is built
 * from a clamped description and a full-bleed list, but not that the description
 * stays on one line, that a long name gives way before the action beside it, or
 * that the list keeps its floor while a search narrows it. This renders the real
 * `MarketplaceSection` and `MarketplaceRow` to markup and lays them out in
 * Chromium with `marketplace.css` and the shared list-row recipe cut from
 * `main.css` at its owner's markers, at the token values.
 *
 * The dialog shell itself (640px, `ModalShell size="lg"`) is Tailwind and a
 * Radix portal, which a static render cannot lay out; its size is asserted in
 * `MarketplaceDialog.test.tsx` and its width is `ModalShell`'s own to measure.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { MarketplaceRow, MarketplaceSection } from './MarketplaceDialog';

const here = dirname(fileURLToPath(import.meta.url));

/** `main.css`'s values for the tokens the list and its rows read. */
const TOKENS = `:root {
  --font-body: Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, sans-serif;
  --radius-md: 8px;
  --row-height: 40px;
  --border-subtle: #e5e3df;
  --border-focus: #6b6963;
  --overlay-hover: rgb(0 0 0 / 5%);
  --dur-fast-min: 95ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
}
* { box-sizing: border-box; }
body { margin: 0; font: 14px/20px var(--font-body); }
h3 { margin: 0; font: inherit; }
/* The utilities the rows and the section label use, at their Tailwind values. */
.flex { display: flex; } .flex-col { flex-direction: column; } .flex-1 { flex: 1 1 0%; }
.items-center { align-items: center; } .min-w-0 { min-width: 0; } .shrink-0 { flex-shrink: 0; }
.block { display: block; } .gap-2 { gap: 8px; } .gap-3 { gap: 12px; }
.px-3 { padding-inline: 12px; } .py-2 { padding-block: 8px; }
.truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.text-label { font-size: 14px; line-height: 20px; font-weight: 500; }
.text-supporting { font-size: 12px; line-height: 16px; }
.text-caps { font-size: 12px; line-height: 16px; font-weight: 500; letter-spacing: 0.04em;
  text-transform: uppercase; }
.tabular-nums { font-variant-numeric: tabular-nums; }`;

/** The shared list-row recipe and the list shell, cut from `main.css`. */
function listRowCss(): string {
  const main = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
  const begin = main.indexOf('/* @ws WS-PRIMITIVES begin: rows */');
  const end = main.indexOf('/* @ws WS-PRIMITIVES end: rows */');
  if (begin < 0 || end < begin) throw new Error('row block markers not found in main.css');
  const shell = '.biorouter-list-shell { display: flex; flex-direction: column; }\n';
  return shell + main.slice(begin, end);
}

const LONG_NAME = 'An extension whose name keeps going well past any width the row could give it';
const LONG_DESCRIPTION =
  'A description that runs on for far longer than one line of a 608 pixel list could ever hold, ' +
  'so a layout that wrapped it would turn a two line row into a three line one.';

/** The list as a dialog body holds it: 16px of inline padding around a column. */
function page(css: string, { rows = 3, height }: { rows?: number; height?: number } = {}) {
  const bare = renderToStaticMarkup(
    <MarketplaceSection label="Core skills" count={rows}>
      <MarketplaceRow title="Plain" description="One short line" />
      <MarketplaceRow title="No description" />
      <MarketplaceRow
        title={LONG_NAME}
        meta="UCSF · 1.0.0"
        description={LONG_DESCRIPTION}
        trailing={
          <button type="button" id="long-action" style={{ width: 64, height: 28 }}>
            Add
          </button>
        }
      />
      <MarketplaceRow
        title="Moderately long extension name"
        meta="A long organization name · 12.34.56 · and a considerably longer tag line that keeps going on and on and on"
        description="Fits"
        trailing={
          <button type="button" id="meta-action" style={{ width: 64, height: 28 }}>
            Add
          </button>
        }
      />
    </MarketplaceSection>
  );
  const filler = Array.from({ length: Math.max(0, rows - 4) }, (_, i) =>
    renderToStaticMarkup(<MarketplaceRow key={i} title={`Filler ${i}`} description="Filler row" />)
  ).join('');
  return `<!doctype html><html><head><style>${TOKENS}\n${css}</style></head><body>
    <div id="body" style="width:640px;padding:0 16px;display:flex;flex-direction:column;${
      height ? `height:${height}px;` : ''
    }">
      <div id="search" style="flex:none;height:32px;margin-bottom:12px"></div>
      <div class="br-marketplace-list" id="list">${bare}${filler}</div>
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

describe('Marketplace dialog list in a real layout engine', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the marketplace geometry ' +
          'tests. Run `npx playwright install chromium` to enable them.'
      );
    }
    css = listRowCss() + readFileSync(resolve(here, 'marketplace.css'), 'utf8');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(options?: Parameters<typeof page>[1]): Promise<Page> {
    const tab = await browser!.newPage({
      viewport: { width: 1000, height: 1000 },
      reducedMotion: 'reduce',
    });
    await tab.setContent(page(css, options));
    return tab;
  }

  /** The box of the nth match, relative to the viewport. */
  const boxes = (tab: Page, selector: string) =>
    tab.evaluate((sel) => {
      return [...document.querySelectorAll(sel)].map((el) => {
        const r = el.getBoundingClientRect();
        return {
          left: r.left,
          right: r.right,
          top: r.top,
          height: r.height,
          width: r.width,
          overflows: el.scrollWidth > el.clientWidth,
        };
      });
    }, selector);

  it('draws a row as at most two lines: 53px with a description, the 40px floor without', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const rows = await boxes(tab, '[data-marketplace-row]');
      // 8 + 20 (name) + 16 (description) + 8 + the row recipe's 1px hairline room.
      expect(rows[0].height).toBe(53);
      expect(rows[1].height).toBe(40);
    } finally {
      await tab.close();
    }
  });

  it('keeps a long description on one line and ellipsizes a long name', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const rows = await boxes(tab, '[data-marketplace-row]');
      const titles = await boxes(tab, '[data-marketplace-title]');
      const [description] = await boxes(tab, '[data-marketplace-row]:nth-of-type(3) span.block');
      // The long row is still the two-line height; nothing wrapped to a third line.
      expect(rows[2].height).toBe(53);
      expect(titles[2].height).toBe(20);
      expect(titles[2].overflows).toBe(true);
      expect(description.height).toBe(16);
      expect(description.overflows).toBe(true);
    } finally {
      await tab.close();
    }
  });

  it('keeps the action inside the row at its padding, whatever the name', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const rows = await boxes(tab, '[data-marketplace-row]');
      const [action] = await boxes(tab, '#long-action');
      expect(action.width).toBe(64);
      expect(rows[2].right - action.right).toBe(12);
    } finally {
      await tab.close();
    }
  });

  it('lets the muted fact give way before the name does', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      // The fourth row's name fits on its own; its fact is long. Squeezed, the
      // fact keeps a smaller share of its natural width than the name does.
      const share = await tab.evaluate(() => {
        const row = document.querySelectorAll('[data-marketplace-row]')[3];
        const title = row.querySelector('[data-marketplace-title]') as HTMLElement;
        const meta = row.querySelector('[data-marketplace-meta]') as HTMLElement;
        return {
          title: title.clientWidth / title.scrollWidth,
          meta: meta.clientWidth / meta.scrollWidth,
        };
      });
      expect(share.meta).toBeLessThan(0.75);
      expect(share.title).toBeGreaterThan(share.meta + 0.2);
    } finally {
      await tab.close();
    }
  });

  it('runs the list to the dialog edges while its text keeps the body inset', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const [list] = await boxes(tab, '#list');
      const [body] = await boxes(tab, '#body');
      // The body carries 16px of inline padding; the list's hairlines do not.
      expect(list.left).toBe(body.left);
      expect(list.right).toBe(body.right);
      const [label] = await boxes(tab, '.br-marketplace-section-label');
      expect(label.height).toBe(16 + 8 + 4);
    } finally {
      await tab.close();
    }
  });

  it('scrolls inside a fixed body and never grows it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ rows: 20, height: 480 });
    try {
      const [body] = await boxes(tab, '#body');
      expect(body.height).toBe(480);
      const scroll = await tab.evaluate(() => {
        const list = document.getElementById('list')!;
        return {
          scrolls: list.scrollHeight > list.clientHeight,
          overflowY: getComputedStyle(list).overflowY,
        };
      });
      expect(scroll).toEqual({ scrolls: true, overflowY: 'auto' });
    } finally {
      await tab.close();
    }
  });

  it('keeps a floor height so a narrowing search does not shrink the dialog', async (ctx) => {
    if (!browser) return ctx.skip();
    // One row left after a search, in a body that sizes to its content.
    const tab = await browser!.newPage({
      viewport: { width: 1000, height: 1000 },
      reducedMotion: 'reduce',
    });
    try {
      await tab.setContent(page(css));
      await tab.evaluate(() => {
        const rows = document.querySelectorAll('[data-marketplace-row]');
        rows.forEach((row, i) => i > 0 && row.remove());
      });
      const [list] = await boxes(tab, '#list');
      // min(45vh, 420px) at a 1000px window.
      expect(list.height).toBe(420);
    } finally {
      await tab.close();
    }
  });
});
