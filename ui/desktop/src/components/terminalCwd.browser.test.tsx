// @vitest-environment node
/**
 * The terminal dock's working folder (`TerminalCwd`), laid out in Chromium.
 *
 * It sits beside the pane tabs at most 38% of the strip wide. It used to end-truncate, so a
 * short strip cut the folder's own name first (`~/Desktop/Bio…`). Now the leading part shrinks
 * behind its own ellipsis and the name keeps its width; the name gives way only when it alone is
 * wider than the space. jsdom lays nothing out, so this is measured here, with the component's
 * own stylesheet (`terminal-dock.css`).
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { TerminalCwd } from './TerminalCwd';

const here = dirname(fileURLToPath(import.meta.url));

const PAGE = `:root { --font-body: Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, sans-serif; }
body { margin: 0; font: 14px/20px var(--font-body); }
* { box-sizing: border-box; }
.text-supporting { font-size: 12px; line-height: 16px; }
.text-text-muted { color: #635c54; }
/* The dock's strip: the tab list takes what is left, the folder sits beside it. */
.strip { display: flex; align-items: center; gap: 4px; height: 40px; }
.tabs { display: flex; flex: 1 1 0%; min-width: 0; }
.tab { flex-shrink: 0; width: 120px; }`;

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

describe('the terminal dock’s working folder in a real layout engine', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the terminal folder ' +
          'layout tests. Run `npx playwright install chromium` to enable them.'
      );
    }
    css = readFileSync(resolve(here, 'terminal-dock.css'), 'utf8');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** A strip `width` px wide, two tabs and the folder, in a window `viewport` px wide. */
  async function open(cwd: string, width: number, viewport = 1000): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: viewport, height: 300 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${PAGE}${css}</style></head><body>` +
        `<div class="strip" id="strip" style="width:${width}px">` +
        `<div class="tabs"><div class="tab">BioRouter</div><div class="tab">BioRouter 2</div></div>` +
        renderToStaticMarkup(<TerminalCwd cwd={cwd} />) +
        `</div></body></html>`
    );
    return tab;
  }

  const measure = (tab: Page) =>
    tab.evaluate(() => {
      const label = document.querySelector<HTMLElement>('.br-terminal-cwd')!;
      const piece = (selector: string) => {
        const node = label.querySelector<HTMLElement>(selector);
        if (!node) return null;
        return {
          text: node.textContent,
          width: node.getBoundingClientRect().width,
          // Wider than its box: something in it is hidden behind the ellipsis.
          cut: node.scrollWidth > node.clientWidth + 0.5,
          overflow: getComputedStyle(node).textOverflow,
        };
      };
      const strip = document.getElementById('strip')!.getBoundingClientRect();
      const box = label.getBoundingClientRect();
      return {
        display: getComputedStyle(label).display,
        width: box.width,
        stripWidth: strip.width,
        insideStrip: box.right <= strip.right + 0.5,
        lead: piece('.br-terminal-cwd__lead'),
        name: piece('.br-terminal-cwd__name'),
      };
    });

  const LONG = '/Volumes/shared/research/projects/cohorts/2026/BioRouter';

  it('shows the whole folder when the strip has room for it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('/Volumes/lab/BioRouter', 1000);
    try {
      const m = await measure(tab);
      expect(m.display).toBe('flex');
      expect(m.lead).toMatchObject({ text: '/Volumes/lab', cut: false });
      expect(m.name).toMatchObject({ text: '/BioRouter', cut: false });
    } finally {
      await tab.close();
    }
  });

  it('shrinks the leading part behind an ellipsis and keeps the folder’s own name whole', async (ctx) => {
    if (!browser) return ctx.skip();
    // 38% of 420px is 159.6px, far less than the path.
    const tab = await open(LONG, 420);
    try {
      const m = await measure(tab);
      expect(m.width).toBeLessThanOrEqual(m.stripWidth * 0.38 + 0.5);
      expect(m.insideStrip).toBe(true);
      expect(m.name, 'the name is drawn in full').toMatchObject({
        text: '/BioRouter',
        cut: false,
      });
      expect(m.lead, 'the leading part gave way').toMatchObject({
        cut: true,
        overflow: 'ellipsis',
      });
      expect(m.lead!.width).toBeGreaterThan(0);
    } finally {
      await tab.close();
    }
  });

  it('cuts the name itself only when it alone is wider than the space', async (ctx) => {
    if (!browser) return ctx.skip();
    const name = 'a-folder-whose-own-name-is-far-wider-than-the-space-beside-the-tabs';
    const tab = await open(`/Volumes/lab/${name}`, 420);
    try {
      const m = await measure(tab);
      expect(m.width).toBeLessThanOrEqual(m.stripWidth * 0.38 + 0.5);
      expect(m.insideStrip).toBe(true);
      expect(m.lead!.width, 'nothing of the leading part is left').toBeLessThan(1);
      expect(m.name).toMatchObject({ cut: true, overflow: 'ellipsis' });
    } finally {
      await tab.close();
    }
  });

  it('stays hidden below the sm breakpoint, as it always was', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(LONG, 600, 600);
    try {
      expect((await measure(tab)).display).toBe('none');
    } finally {
      await tab.close();
    }
  });
});
