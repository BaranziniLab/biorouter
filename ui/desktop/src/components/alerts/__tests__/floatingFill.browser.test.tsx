// @vitest-environment node
/**
 * Find in chat, the toast and the inline notice under R-27 (owner message 13, design.md 9.11),
 * in a real engine on the real stylesheet.
 *
 * Two kinds of surface, two rules. The find bar and a toast FLOAT, so they keep a hairline and a
 * shadow, and in dark they take the lifted ground every menu takes (`.biorouter-popover-surface`,
 * V1-M5): a #1b1b19 box over the dark transcript was told apart by its hairline alone. The inline
 * notice sits IN the page, so it is the subtle fill with a transparent edge. And whatever paints
 * the pressed or hovered state inside them must still be seen on the ground they now have: the
 * "Aa" toggle and the toast's close hover were `--background-medium`, which IS the lifted dark
 * ground. The real components are bundled with esbuild and `styles/main.css` is compiled by
 * Tailwind with the classes the bundle uses (plus `search.css`, which the bar imports).
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../../package.json'));

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import SearchBar from '../../conversation/SearchBar';
import {
  NotificationContent,
  NotificationSurface,
  TOAST_SURFACE_CLASS_NAME,
} from '../NotificationSurface';
import { Button } from '../../ui/button';

function App() {
  return (
    <div style={{ display: 'grid', gap: 24, padding: 24, width: 520 }}>
      <div id="find-cell" style={{ position: 'relative', height: 72 }}>
        <SearchBar onSearch={() => {}} onClose={() => {}} initialSearchTerm="agent" />
      </div>
      <div className="Toastify__toast-container" style={{ position: 'static', width: 420 }}>
        <div id="toast" className={TOAST_SURFACE_CLASS_NAME}>
          <NotificationContent
            status="loading"
            title="Loading extensions"
            actions={<Button id="toast-action" variant="secondary" size="sm">View details</Button>}
          />
          <button id="toast-close" type="button" className="Toastify__close-button">x</button>
        </div>
      </div>
      <NotificationSurface
        status="warning"
        message="Your choice was not recorded."
        onClose={() => {}}
        dismissLabel="Dismiss banner"
      />
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

describe('floating surfaces keep their line, the in-page notice takes the fill (R-27)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the floating surface tests.');
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
    css =
      compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }])) +
      '\n' +
      readFileSync(resolve(STYLES, 'search.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(mode: 'light' | 'dark'): Promise<Page> {
    const page = await browser!.newPage({ viewport: { width: 600, height: 520 } });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-canvas)"><div id="root"></div></body></html>`
    );
    await page.addScriptTag({ content: script });
    await page.waitForSelector('[data-testid="conversation-search-bar"]');
    await page.waitForTimeout(300);
    return page;
  }

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
  const edgeOf = (page: Page, value: string) =>
    page.evaluate((v) => {
      const probe = document.createElement('div');
      probe.style.borderTop = `1px solid ${v}`;
      document.body.append(probe);
      const out = getComputedStyle(probe).borderTopColor;
      probe.remove();
      return out;
    }, value);
  const look = (page: Page, selector: string) =>
    page.evaluate((s) => {
      const cs = getComputedStyle(document.querySelector(s)!);
      return {
        ground: cs.backgroundColor,
        edge: cs.borderTopColor,
        edgeWidth: cs.borderTopWidth,
        shadow: cs.boxShadow,
      };
    }, selector);
  /** `top` over `under`, as sRGB bytes, painted on a canvas (oklab mixes read as oklab). */
  const over = (page: Page, top: string, under: string) =>
    page.evaluate(
      ([t, u]) => {
        const ctx = document.createElement('canvas').getContext('2d')!;
        ctx.fillStyle = u;
        ctx.fillRect(0, 0, 1, 1);
        ctx.fillStyle = t;
        ctx.fillRect(0, 0, 1, 1);
        return Array.from(ctx.getImageData(0, 0, 1, 1).data.slice(0, 3)).join(',');
      },
      [top, under] as const
    );

  const BAR = '[data-testid="conversation-search-bar"]';

  it('floats the find bar and the toast on the one floating recipe, lifted in dark', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const page = await open(mode);
      try {
        const lifted = await paint(
          page,
          mode === 'dark' ? 'var(--background-medium)' : 'var(--background-default)'
        );
        const edge = await edgeOf(
          page,
          mode === 'dark' ? 'var(--border-strong)' : 'var(--border-subtle)'
        );
        for (const id of [BAR, '#toast']) {
          const s = await look(page, id);
          expect(s.ground, `${mode} ${id} ground`).toBe(lifted);
          expect(s.edgeWidth, `${mode} ${id} keeps its hairline`).toBe('1px');
          expect(s.edge, `${mode} ${id} hairline`).toBe(edge);
          expect(s.shadow, `${mode} ${id} keeps its shadow`).not.toBe('none');
        }
      } finally {
        await page.close();
      }
    }
  });

  it('shows the "Aa" toggle, the toast close hover and the loading chip on the surface they sit on', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const page = await open(mode);
      try {
        const bar = (await look(page, BAR)).ground;
        await page.click(`${BAR} button[aria-pressed]`);
        await page.mouse.move(590, 510);
        await page.waitForTimeout(300);
        const aa = await look(page, `${BAR} button[aria-pressed="true"]`);
        expect(aa.ground, `${mode} Aa on`).toBe(
          await paint(page, 'var(--background-fill-strong)', BAR)
        );
        expect(await over(page, aa.ground, bar), `${mode} Aa on is not the bar`).not.toBe(
          await over(page, bar, bar)
        );

        const toast = (await look(page, '#toast')).ground;
        await page.hover('#toast-close');
        await page.waitForTimeout(300);
        const close = (await look(page, '#toast-close')).ground;
        expect(await over(page, close, toast), `${mode} toast close hover`).not.toBe(
          await over(page, toast, toast)
        );
        const chip = (await look(page, '#toast [data-status="loading"]')).ground;
        expect(await over(page, chip, toast), `${mode} loading chip`).not.toBe(
          await over(page, toast, toast)
        );
      } finally {
        await page.close();
      }
    }
  });

  it('draws the inline notice as the subtle fill with a transparent edge, its controls a step off it', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const page = await open(mode);
      try {
        const banner = await look(page, '[data-testid="notification-surface"]');
        expect(banner.ground, `${mode} the notice is filled`).toBe(
          await paint(page, 'var(--background-fill)')
        );
        expect(banner.edgeWidth).toBe('1px');
        expect(banner.edge, `${mode} no outline`).toBe(await paint(page, 'transparent'));
        expect(banner.shadow).toBe('none');
        await page.hover('button[aria-label="Dismiss banner"]');
        await page.waitForTimeout(300);
        const x = (await look(page, 'button[aria-label="Dismiss banner"]')).ground;
        expect(await over(page, x, banner.ground), `${mode} dismiss hover`).not.toBe(
          await over(page, banner.ground, banner.ground)
        );
      } finally {
        await page.close();
      }
    }
  });
});
