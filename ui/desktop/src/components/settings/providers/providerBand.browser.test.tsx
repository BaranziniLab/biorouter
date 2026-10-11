// @vitest-environment node
/**
 * The provider page's band in a real layout engine (vision round 2, V4-14; R-27's recessed band).
 *
 * `#/configure-providers` and `#/welcome` are outside the app shell, so they have no sidebar and
 * none of the shell's top-band bookkeeping. The band used to start at y=32 under a separate 32px
 * strip (76px of chrome where Settings has 44) and painted nothing. The claims are about layout
 * and the cascade, which jsdom does not run:
 *
 * - the band is 44px (`--chrome-height`) at y=0 and the page's body starts right under it;
 * - it paints the shared band ground (`--sidebar`), light and dark, not transparent;
 * - the 32px `.titlebar-drag-region` strip `App.tsx` paints (fixed, z-index 50) takes no clicks,
 *   so Back, the InfoTip and the tabs are reachable over their whole height, while the band's bar
 *   is the drag rect and every control in it is `no-drag`;
 * - the band's first control starts at the titlebar reserve on macOS (clear of the traffic
 *   lights) and at the ordinary 16px elsewhere.
 *
 * The real `ProviderSettingsPage` is bundled with esbuild (its config context, the catalog and
 * the commercial setup card are stubbed: the catalog has its own suites, and a stand-in draws the
 * tab list into the band's slot as the real one does), `styles/main.css` is compiled by Tailwind
 * with the classes the bundle uses, `providers.css` is appended as authored, and Chromium paints
 * it. Every rule is proved the way `Layout/viewsFill.browser.test.tsx` proves its own: the case
 * strips the one declaration it guards from the sheet and expects the defect, so the instrument is
 * shown to see what it claims to.
 *
 * Set `WS_SCREENSHOT_DIR` to also save a PNG of the page in light and dark.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../../package.json'));
const PROVIDERS_CSS = readFileSync(resolve(here, 'providers.css'), 'utf8');

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import ProviderSettings from './ProviderSettingsPage';

createRoot(document.getElementById('root')!).render(
  <MemoryRouter>
    <ProviderSettings onClose={() => {}} isOnboarding={false} />
  </MemoryRouter>
);
`;

/** What the page imports that is not the page's business: the config, the catalog, the setup card. */
const STUBS: Record<string, string> = {
  ConfigContext: `
    export const useConfig = () => ({
      getProviders: async () => [],
      read: async () => '',
      upsert: async () => {},
    });
  `,
  ProviderCatalog: `
    import { createPortal } from 'react-dom';
    export default function ProviderCatalog({ tabsHost }) {
      return (
        <>
          {tabsHost
            ? createPortal(
                <div className="br-provider-band-tabs" role="tablist">
                  <button role="tab" id="tab-local" type="button">Local</button>
                  <button role="tab" id="tab-institutional" type="button">Institutional</button>
                </div>,
                tabsHost
              )
            : null}
          <div id="catalog-body" style={{ height: 1600 }}>Catalog</div>
        </>
      );
    }
  `,
  CommercialSetupCard: `export const persistDetectedProviderSetup = async () => {};`,
};

const stubPlugin: Plugin = {
  name: 'provider-page-stubs',
  setup(b) {
    b.onResolve({ filter: /(ConfigContext|\/ProviderCatalog|CommercialSetupCard)$/ }, (args) => {
      const name = Object.keys(STUBS).find((key) => args.path.endsWith(key));
      return name ? { path: name, namespace: 'stub' } : undefined;
    });
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: STUBS[args.path],
      loader: 'tsx',
      resolveDir: here,
    }));
  },
};

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

type Measure = {
  header: { top: number; height: number };
  chromeHeight: number;
  bodyTop: number;
  headerGround: string;
  sidebarPaint: string;
  stripPointerEvents: string;
  topmostAtBack: string;
  topmostAboveTab: string;
  topmostAboveBar: string;
  backLeft: number;
  backRegion: string;
  tabRegion: string;
  barRegion: string;
  stripRegion: string;
};

describe('the provider page band (V4-14)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the provider band tests.');
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
      plugins: [stubPlugin],
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

  /**
   * The page as `App.tsx` mounts it: the titlebar strip first, then the route. `platform` is what
   * the preload bridge reports; `sheet` is the compiled sheet, so a case can strip a rule from it.
   */
  async function open(
    mode: 'light' | 'dark',
    options: { platform?: string; sheet?: string } = {}
  ): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1440, height: 900 } });
    const platform = options.platform ?? 'darwin';
    const sheet = options.sheet ?? css + PROVIDERS_CSS;
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${sheet}</style></head>` +
        `<body style="margin:0"><div class="relative w-screen h-screen overflow-hidden bg-background-canvas flex flex-col">` +
        `<div class="titlebar-drag-region"></div><div id="root"></div></div></body></html>`
    );
    // Set in the document, not by `addInitScript`: `setContent` reuses the page's document, so an
    // init script would not run for it (and a page that never set it reads as macOS, which would
    // make the "elsewhere" case pass for the wrong reason).
    await tab.evaluate((p) => {
      (window as unknown as { electron: unknown }).electron = {
        platform: p,
        on: () => () => {},
      };
    }, platform);
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#catalog-body');
    await tab.waitForTimeout(200);
    return tab;
  }

  const measure = (tab: Page): Promise<Measure> =>
    tab.evaluate(() => {
      const q = (s: string) => document.querySelector(s) as HTMLElement;
      const header = q('[data-testid="page-header"]');
      const back = q('button[aria-label="Back"]');
      const tabEl = q('#tab-local');
      const bar = q('.biorouter-page-header-bar');
      const strip = q('.titlebar-drag-region');
      const hRect = header.getBoundingClientRect();
      const backRect = back.getBoundingClientRect();
      const tabRect = tabEl.getBoundingClientRect();
      const barRect = bar.getBoundingClientRect();
      const probe = document.createElement('div');
      probe.style.backgroundColor = 'var(--sidebar)';
      document.body.append(probe);
      const sidebarPaint = getComputedStyle(probe).backgroundColor;
      probe.remove();
      const label = (el: Element | null) =>
        el
          ? (el.className && typeof el.className === 'string' ? el.className.split(' ')[0] : '') ||
            el.tagName.toLowerCase()
          : 'none';
      const top = (x: number, y: number) => label(document.elementFromPoint(x, y));
      const region = (el: Element) =>
        getComputedStyle(el).getPropertyValue('-webkit-app-region').trim();
      // The band's own body: the first child after the header.
      const body = header.nextElementSibling as HTMLElement;
      return {
        header: { top: hRect.top, height: hRect.height },
        chromeHeight: parseFloat(
          getComputedStyle(document.documentElement).getPropertyValue('--chrome-height')
        ),
        bodyTop: body.getBoundingClientRect().top,
        headerGround: getComputedStyle(header).backgroundColor,
        sidebarPaint,
        stripPointerEvents: getComputedStyle(strip).pointerEvents,
        // The top 32px of the band is where the strip sits: probe inside it.
        topmostAtBack: top(backRect.left + backRect.width / 2, 8),
        topmostAboveTab: top(tabRect.left + tabRect.width / 2, 4),
        topmostAboveBar: top(Math.min(barRect.right - 40, 1000), 4),
        backLeft: backRect.left,
        backRegion: region(back),
        tabRegion: region(tabEl),
        barRegion: region(bar),
        stripRegion: region(strip),
      };
    });

  const shoot = async (tab: Page, name: string) => {
    const dir = process.env.WS_SCREENSHOT_DIR;
    if (!dir) return;
    mkdirSync(dir, { recursive: true });
    await tab.screenshot({ path: resolve(dir, `${name}.png`) });
  };

  it('is one 44px band at the top edge with the page body right under it', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const m = await measure(tab);
        expect(m.chromeHeight, 'the token this test reads').toBe(44);
        expect(m.header.top, `${mode} band top`).toBe(0);
        expect(m.header.height, `${mode} band height`).toBe(m.chromeHeight);
        // No spacer: the body starts where the band ends, so the page is 44px of chrome.
        expect(m.bodyTop, `${mode} body top`).toBe(m.chromeHeight);
        await shoot(tab, `provider-band-${mode}`);
      } finally {
        await tab.close();
      }
    }
  });

  it('paints the shared band ground, not nothing', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const m = await measure(tab);
        expect(m.headerGround, `${mode} band ground`).toBe(m.sidebarPaint);
        expect(m.headerGround, `${mode} not transparent`).not.toBe('rgba(0, 0, 0, 0)');
      } finally {
        await tab.close();
      }
    }
    // The instrument sees the defect: the old override paints nothing.
    const tab = await open('light', {
      sheet:
        css +
        PROVIDERS_CSS +
        '.br-provider-page .biorouter-page-header[data-band] { background: transparent; }',
    });
    try {
      expect((await measure(tab)).headerGround).toBe('rgba(0, 0, 0, 0)');
    } finally {
      await tab.close();
    }
  });

  it('lets the titlebar strip drag without taking the clicks meant for the band', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light');
    try {
      const m = await measure(tab);
      expect(m.stripPointerEvents).toBe('none');
      // Over Back's, a tab's and the empty bar's top edge, the thing under the pointer is the
      // band's own, never the strip.
      expect(m.topmostAtBack, 'over Back').not.toBe('titlebar-drag-region');
      expect(m.topmostAboveTab, 'over a tab').not.toBe('titlebar-drag-region');
      expect(m.topmostAboveBar, 'over the bar').not.toBe('titlebar-drag-region');
      // The band is the drag rect and its controls subtract from it; the strip still drags.
      expect(m.barRegion).toBe('drag');
      expect(m.backRegion).toBe('no-drag');
      expect(m.tabRegion).toBe('no-drag');
      expect(m.stripRegion).toBe('drag');
    } finally {
      await tab.close();
    }
    // The instrument sees the defect: without the rule the strip is on top of the band's top 32px.
    const without = PROVIDERS_CSS.replace(
      /body:has\(\.br-provider-page\) \.titlebar-drag-region \{[^}]*\}/,
      ''
    );
    expect(without).not.toBe(PROVIDERS_CSS);
    const broken = await open('light', { sheet: css + without });
    try {
      const m = await measure(broken);
      expect(m.stripPointerEvents).toBe('auto');
      expect(m.topmostAtBack).toBe('titlebar-drag-region');
      expect(m.topmostAboveTab).toBe('titlebar-drag-region');
    } finally {
      await broken.close();
    }
  });

  it('starts the first control past the traffic lights on macOS and at 16px elsewhere', async (ctx) => {
    if (!browser) return ctx.skip();
    const mac = await open('light', { platform: 'darwin' });
    try {
      // The window controls end near x=72 (`trafficLightPosition` x 20, three 12px buttons 8px
      // apart); the band's first control is the one the chat header's collapsed sidebar starts at.
      expect((await measure(mac)).backLeft).toBe(100);
    } finally {
      await mac.close();
    }
    const linux = await open('light', { platform: 'linux' });
    try {
      expect((await measure(linux)).backLeft).toBe(16);
    } finally {
      await linux.close();
    }
    // The instrument sees the defect: without the rule the band's padding is the ordinary 16px.
    const without = PROVIDERS_CSS.replace(
      /\.br-provider-page \.biorouter-page-header-bar \{[^}]*\}/,
      ''
    );
    expect(without).not.toBe(PROVIDERS_CSS);
    const broken = await open('light', { platform: 'darwin', sheet: css + without });
    try {
      expect((await measure(broken)).backLeft).toBe(16);
    } finally {
      await broken.close();
    }
  });
});
