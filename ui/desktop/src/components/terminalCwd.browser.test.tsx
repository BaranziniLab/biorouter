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
import { build } from 'esbuild';
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

/**
 * Where the ellipsis ends and the name begins, as drawn. `text-overflow: ellipsis` hides whole
 * glyphs and draws the ellipsis right after the last one that fits, so the rest of the box the
 * strip gave the leading part stayed empty, and the dock read `~/bi… /home`: 2 to 8px between the
 * ellipsis and the name at 12px, varying with the width. `fitCwdLead` cuts the box to the glyphs it
 * keeps. Only painted pixels can show the gap (no DOM node holds the ellipsis), so the page is
 * captured and read back, and the REAL component runs, bundled with esbuild, so its layout effect
 * and its ResizeObserver are what fit the box.
 */
describe('the terminal folder’s ellipsis against the name, as painted', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  const ENTRY = `
    import * as React from 'react';
    import { createRoot } from 'react-dom/client';
    import { TerminalCwd } from './TerminalCwd';
    const widths = (window as any).__WIDTHS__ as number[];
    const cwd = (window as any).__CWD__ as string;
    createRoot(document.getElementById('root')!).render(
      <>
        {widths.map((width) => (
          <div key={width} className="strip" data-width={width} style={{ width }}>
            <div className="tabs"><div className="tab">BioRouter</div></div>
            <TerminalCwd cwd={cwd} />
          </div>
        ))}
      </>
    );
  `;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) return;
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      loader: { '.css': 'empty' },
      define: { 'process.env.NODE_ENV': '"production"' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    css = readFileSync(resolve(here, 'terminal-dock.css'), 'utf8');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** Every strip width from 300 to 440px in 7px steps: the gap varies with the width. */
  const WIDTHS = Array.from({ length: 21 }, (_, i) => 300 + i * 7);

  async function mount(cwd: string): Promise<Page> {
    const tab = await browser!.newPage({
      viewport: { width: 1000, height: 1000 },
      deviceScaleFactor: 1,
    });
    await tab.setContent(
      `<!doctype html><html><head><style>${PAGE}${css}
        .strip { margin: 0 0 4px; background: #fff; }</style></head>` +
        `<body><div id="root"></div></body></html>`
    );
    await tab.evaluate(
      ({ widths, cwd }) => Object.assign(window, { __WIDTHS__: widths, __CWD__: cwd }),
      { widths: WIDTHS, cwd }
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('.br-terminal-cwd__lead');
    return tab;
  }

  /**
   * For each strip: whether the leading part is cut, and the empty columns between the last ink
   * left of the name's box (the ellipsis) and the first ink in it. Ink is any pixel darker than
   * the white ground.
   */
  async function gaps(tab: Page) {
    const shot = (await tab.screenshot()).toString('base64');
    return tab.evaluate(async (png) => {
      const image = new Image();
      image.src = `data:image/png;base64,${png}`;
      await image.decode();
      const canvas = document.createElement('canvas');
      canvas.width = image.width;
      canvas.height = image.height;
      const context = canvas.getContext('2d')!;
      context.drawImage(image, 0, 0);
      return [...document.querySelectorAll<HTMLElement>('.strip')].map((strip) => {
        const lead = strip.querySelector<HTMLElement>('.br-terminal-cwd__lead')!;
        const name = strip.querySelector<HTMLElement>('.br-terminal-cwd__name')!;
        const leadBox = lead.getBoundingClientRect();
        const nameBox = name.getBoundingClientRect();
        const x0 = Math.floor(leadBox.left);
        const x1 = Math.ceil(nameBox.right);
        const y0 = Math.floor(nameBox.top);
        const height = Math.ceil(nameBox.height);
        const data = context.getImageData(x0, y0, x1 - x0, height).data;
        const split = Math.round(nameBox.left) - x0;
        let leadInk = -1;
        let nameInk = Infinity;
        for (let x = 0; x < x1 - x0; x++) {
          for (let y = 0; y < height; y++) {
            const at = (y * (x1 - x0) + x) * 4;
            if (data[at] + data[at + 1] + data[at + 2] > 600) continue;
            if (x < split) leadInk = Math.max(leadInk, x);
            else nameInk = Math.min(nameInk, x);
          }
        }
        return {
          width: Number(strip.dataset.width),
          cut: lead.scrollWidth > lead.clientWidth || lead.style.maxWidth !== '',
          gap: nameInk - leadInk - 1,
        };
      });
    }, shot);
  }

  const LONG_CWD = '/Volumes/shared/research/projects/cohorts/2026/BioRouter';

  it('draws the ellipsis against the name at every width, as glyphs sit in a word', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await mount(LONG_CWD);
    try {
      // The space between two glyphs of the path itself, drawn whole: the measure of "touching".
      const measured = await gaps(tab);
      const cut = measured.filter((strip) => strip.cut);
      expect(cut.length, 'the leading part is cut at every width here').toBe(WIDTHS.length);
      for (const strip of cut) {
        expect(
          strip.gap,
          `the space after the ellipsis in a ${strip.width}px strip`
        ).toBeLessThanOrEqual(2);
      }
    } finally {
      await tab.close();
    }
  });

  it('fits again when the strip is resized, wider and narrower', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await mount(LONG_CWD);
    try {
      for (const delta of [5, -9]) {
        await tab.evaluate((by) => {
          for (const strip of document.querySelectorAll<HTMLElement>('.strip')) {
            strip.style.width = `${Number(strip.dataset.width) + by}px`;
            strip.dataset.width = String(Number(strip.dataset.width) + by);
          }
        }, delta);
        // The ResizeObserver's callback runs before the next paint.
        await tab.evaluate(() => new Promise((done) => requestAnimationFrame(() => done(null))));
        for (const strip of await gaps(tab)) {
          expect(strip.gap, `after a resize, a ${strip.width}px strip`).toBeLessThanOrEqual(2);
        }
      }
    } finally {
      await tab.close();
    }
  });
});
