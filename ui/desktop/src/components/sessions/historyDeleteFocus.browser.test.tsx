// @vitest-environment node
/**
 * Where focus goes when History's Delete dialog closes, in a real engine (vision V1-R8-1).
 *
 * Right-click a row (or press its ⋯), choose "Delete chat…" with the pointer, then press Escape:
 * focus fell to `<body>` and the next Tab began at the top of the window. The shared dialog rule
 * gives focus back after a menu pick only when the pick was a key; the row now says which of its
 * controls opened the menu, and a dialog closed with a key gives focus back to it. A dialog closed
 * with the pointer leaves focus alone.
 *
 * jsdom has no `:focus-visible` and no real input modality, so the real `SessionListView` is
 * bundled with esbuild and driven with Playwright's mouse and keyboard in Chromium, under
 * `main.css` compiled by Tailwind from the bundle's classes plus the authored `history.css`.
 * Set `HISTORY_SHOTS_DIR` to also save a screenshot of the ringed row.
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
const SHOTS = process.env.HISTORY_SHOTS_DIR;
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const ORIGIN = 'http://history.test';

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { TooltipProvider } from '../ui/Tooltip';
import SessionListView from './SessionListView';

createRoot(document.getElementById('root')!).render(
  <MemoryRouter initialEntries={['/sessions']}>
    <TooltipProvider>
      <div style={{ height: '100vh', display: 'flex', flexDirection: 'column' }}>
        <SessionListView onSelectSession={() => {}} />
      </div>
    </TooltipProvider>
  </MemoryRouter>
);
`;

/** The window globals the list reads (the preload bridge and the app config). */
const STUBS = `
window.electron = {
  platform: 'darwin',
  getUserActionKey: async () => 'test-key',
  createChatWindow: () => {},
  on: () => {},
  off: () => {},
};
window.appConfig = { get: (key) => (key === 'BIOROUTER_HOME_DIR' ? '/Users/me' : undefined) };
`;

const HOUR = 3_600_000;
function sessions() {
  const now = Date.now();
  const row = (i: number, ago: number, name: string) => ({
    id: `h${i}`,
    name,
    created_at: new Date(now - ago - 2 * HOUR).toISOString(),
    updated_at: new Date(now - ago).toISOString(),
    working_dir: '/Users/me/projects/cohort',
    message_count: 6,
    user_set_name: true,
    privacy_tier: 'public',
    extension_data: {},
  });
  return [
    row(1, 0.2 * HOUR, 'Pong game'),
    row(2, 1 * HOUR, 'Survival analysis'),
    row(3, 2 * HOUR, 'Volcano plot'),
  ];
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

describe('History’s Delete dialog gives focus back (V1-R8-1)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the History delete focus tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: {
        'process.env.NODE_ENV': '"production"',
        'process.env.ALPHA': 'false',
        'import.meta.env': JSON.stringify({ MODE: 'production', DEV: false, PROD: true }),
      },
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
    // The bundle drops CSS imports; History's own stylesheet is appended as the app's bundler does.
    css =
      compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }])) +
      '\n' +
      readFileSync(resolve(here, 'history.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(mode: 'light' | 'dark'): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1100, height: 800 } });
    const html =
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style></head>` +
      `<body style="margin:0"><div id="root"></div>` +
      `<script>${STUBS}</script><script>${script.replace(/<\/script/gi, '<\\/script')}</script>` +
      `</body></html>`;
    await tab.route(`${ORIGIN}/**`, async (route) => {
      const url = route.request().url();
      if (url === `${ORIGIN}/`) {
        await route.fulfill({ status: 200, contentType: 'text/html', body: html });
      } else if (new URL(url).pathname === '/sessions') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ sessions: sessions(), has_more: false }),
        });
      } else {
        await route.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
      }
    });
    await tab.goto(`${ORIGIN}/`);
    await tab.getByRole('button', { name: 'Open chat Pong game' }).waitFor({ timeout: 20_000 });
    await tab.waitForTimeout(400);
    return tab;
  }

  const focus = (tab: Page) =>
    tab.evaluate(() => {
      const el = document.activeElement as HTMLElement | null;
      return {
        label: el?.getAttribute('aria-label') ?? el?.tagName ?? null,
        ringed: el ? el.matches(':focus-visible') : false,
      };
    });
  const row = '[data-session-id="h1"]';
  const askDelete = async (tab: Page, from: 'right-click' | 'more') => {
    if (from === 'right-click') {
      await tab.click(`${row} .br-history-row-open`, { button: 'right' });
    } else {
      await tab.hover(row);
      await tab.waitForTimeout(250);
      await tab.getByRole('button', { name: 'More actions for Pong game' }).click();
    }
    await tab.getByRole('menuitem', { name: 'Delete chat…' }).click();
    await tab.getByRole('dialog').waitFor();
    await tab.waitForTimeout(250);
  };
  const dialogGone = async (tab: Page) => {
    await tab.getByRole('dialog').waitFor({ state: 'detached' });
    await tab.waitForTimeout(400);
  };

  it('returns focus to the control the menu came from after a key close, and not after a click', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      tab.setDefaultTimeout(8_000);
      try {
        // Right-click, Delete chat…, Escape: the row's open button, ringed.
        await askDelete(tab, 'right-click');
        await tab.keyboard.press('Escape');
        await dialogGone(tab);
        expect(await focus(tab), `${mode}: right-click, Escape`).toEqual({
          label: 'Open chat Pong game',
          ringed: true,
        });
        if (SHOTS) {
          await tab.screenshot({
            path: resolve(SHOTS, `history-delete-escape-${mode}.png`),
            clip: { x: 0, y: 0, width: 1100, height: 260 },
          });
        }

        // The row's ⋯, Delete chat…, Escape: back on the ⋯.
        await tab.mouse.click(1090, 790);
        await askDelete(tab, 'more');
        await tab.keyboard.press('Escape');
        await dialogGone(tab);
        expect(await focus(tab), `${mode}: ⋯, Escape`).toEqual({
          label: 'More actions for Pong game',
          ringed: true,
        });

        // Cancel clicked: focus is left alone.
        await tab.mouse.click(1090, 790);
        await askDelete(tab, 'right-click');
        await tab.getByRole('button', { name: 'Cancel' }).click();
        await dialogGone(tab);
        expect((await focus(tab)).label, `${mode}: right-click, Cancel clicked`).not.toBe(
          'Open chat Pong game'
        );
      } finally {
        await tab.close();
      }
    }
  });
});
