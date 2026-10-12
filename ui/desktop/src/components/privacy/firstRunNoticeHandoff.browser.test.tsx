// @vitest-environment node
/**
 * The day-one notice opens when the launch has handed off, measured in Chromium on a throttled CPU
 * (first run vision round 5, FR-R5-1; R-34).
 *
 * Since FR-R4-4 the notice's count starts at boot, so on a slower launch the answer is in before
 * the boot splash has finished fading and before the guard has drawn the first-run band. A gate
 * that opened the moment the answer was `show` put the dialog under the fading wordmark (the
 * wordmark drawn across the dialog's body text for 100 to 200 ms) and over an empty ground, with
 * the band and its skeleton popping in behind the blur. Measured at 3x and 4x CPU in light and
 * dark; full speed never showed it, which is why this file throttles.
 *
 * jsdom has no splash that fades and no paint order, so the unit tests can say what the hook
 * decides and nothing about what a person sees. This file loads the SHIPPED `index.html` (its
 * splash, its inline controller, its theme script, the compiled `main.css`) and, in place of the
 * renderer entry, a bundle of the REAL `ProviderGuard`, the REAL `FirstRunPrivacyNoticeGate`, the
 * real count and the real hand-off, mounted the way `renderer.tsx` mounts them: the count and the
 * hold declared first, React rendered later (so the splash has been drawn), the splash dismissed
 * by a sibling in the same commit. Only the daemon is stubbed (the config reads, the provider
 * list, the session list, each with a latency the URL sets) and the catalog, which is not the
 * subject. A recorder samples the page on every DOM mutation and every frame.
 *
 * What it asserts is the finding's two lines: the dialog never exists while `#br-boot` is in the
 * document or while the guard still shows its loader, and it does open (a run in which it never
 * did would pass the first line for the wrong reason).
 *
 * It needs a Chromium; with none installed it warns and skips, like the other `.browser.test`
 * files.
 */
import { appendFileSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { fontScale } from '../../../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const COMPONENTS = resolve(here, '..');
const DESKTOP = resolve(here, '../../..');
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(DESKTOP, 'package.json'));

/**
 * What the stub config answers, every latency read from the page's query string. The session list
 * the count reads is not stubbed here: the real generated client asks `https://daemon.test` and
 * the test's route answers (see `launch`), so the count under test is the real one end to end.
 */
const CONFIG_STUB = `
const q = new URLSearchParams(location.search);
const later = (ms, value) => new Promise((resolve) => setTimeout(() => resolve(value), ms));
const value = {
  extensionsList: [],
  read: async (key) => {
    await later(Number(q.get('read') ?? 250));
    return key === 'BIOROUTER_ONBOARDING_SKIPPED' ? false : (q.get('provider') ?? '');
  },
  upsert: async () => {},
  getProviders: async () => later(Number(q.get('providers') ?? 100), []),
};
export const useConfig = () => value;
`;

const CATALOG_STUB = `
import { createElement } from 'react';
export default function ProviderCatalog({ loading }) {
  return createElement('div', { 'data-testid': 'catalog-stub', 'data-loading': String(Boolean(loading)) }, 'CATALOG');
}
`;

const STUBS: { filter: RegExp; contents: string }[] = [
  { filter: /(^|\/)ConfigContext$/, contents: CONFIG_STUB },
  { filter: /\/ProviderCatalog$/, contents: CATALOG_STUB },
  {
    filter: /\/CommercialSetupCard$/,
    contents: `export const persistDetectedProviderSetup = async () => {};`,
  },
];

const stubPlugin: Plugin = {
  name: 'stubs',
  setup(b) {
    for (const { filter, contents } of STUBS) {
      b.onResolve({ filter }, (args) =>
        // Only the files under test reach for these; a stub never replaces a module a third
        // party (react-router, radix) resolves for itself.
        args.importer.startsWith(COMPONENTS) || args.importer.includes('/src/')
          ? { path: args.path, namespace: 'stub', pluginData: contents }
          : undefined
      );
    }
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: args.pluginData as string,
      resolveDir: COMPONENTS,
      loader: 'js',
    }));
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

/**
 * What `renderer.tsx` does, reduced to the parts that matter to the hand-off, with the real
 * components in the places it puts them: the count and the hold are started before React renders;
 * React renders after `mountAfter` ms (so a splash that crossed its 400 ms threshold is on screen);
 * the guard and the gate mount in one commit (as `App` mounts them) with the dismissal as their
 * sibling. A copy of `DismissBootSplash` is the one stand-in, because `renderer.tsx` runs the whole
 * boot at import.
 */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { client } from '../api/client.gen';
import { daemonClientConfig } from '../utils/daemonClient';
import ProviderGuard from './ProviderGuard';
import { FirstRunPrivacyNoticeGate } from './privacy/FirstRunPrivacyNoticeGate';
import { startFirstRunNoticeCount } from './privacy/firstRunNoticeDecision';
import { holdFirstRunNoticeForFirstPage } from './privacy/firstRunNoticeHandoff';

function DismissBootSplash() {
  React.useEffect(() => {
    window.__brBootSplash?.dismiss();
  }, []);
  return null;
}

const q = new URLSearchParams(location.search);
client.setConfig(daemonClientConfig('https://daemon.test', 'secret'));
startFirstRunNoticeCount();
holdFirstRunNoticeForFirstPage();

setTimeout(() => {
  createRoot(document.getElementById('root')).render(
    <MemoryRouter>
      <ProviderGuard didSelectProvider={false}>
        <div data-testid="app-stub">Application</div>
      </ProviderGuard>
      <FirstRunPrivacyNoticeGate />
      <DismissBootSplash />
    </MemoryRouter>
  );
}, Number(q.get('mountAfter') ?? 900));
`;

/**
 * The recorder, installed before any page script. It samples on every DOM mutation and on every
 * frame, keeping only changes, so the first moment the dialog exists is in the list with the state
 * of everything else at that moment.
 */
const RECORDER = `
(() => {
  // When the session list came back: the moment the notice became due.
  const realFetch = window.fetch.bind(window);
  window.fetch = async (...args) => {
    const response = await realFetch(...args);
    if (String((args[0] && args[0].url) || args[0]).includes('daemon.test/sessions')) {
      window.__countAnsweredAt = performance.now();
    }
    return response;
  };
  const samples = [];
  let last = '';
  const take = (why) => {
    const splash = window.__brBootSplash;
    const sample = {
      t: performance.now(),
      why,
      splashInDom: !!document.getElementById('br-boot'),
      splashShown: splash ? splash.isShown() : null,
      splashGone: splash ? splash.isGone() : null,
      loader: !!document.querySelector('[data-testid="provider-guard-loading"]'),
      band: !!document.querySelector('[data-testid="first-run-band"]'),
      app: !!document.querySelector('[data-testid="app-stub"]'),
      dialog: !!document.querySelector('[data-testid="first-run-privacy-notice"]'),
    };
    const key = JSON.stringify({ ...sample, t: 0, why: '' });
    if (key === last) return;
    last = key;
    samples.push(sample);
  };
  new MutationObserver(() => take('mutation')).observe(document, { childList: true, subtree: true });
  const frame = () => {
    take('frame');
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
  window.__samples = samples;
})();
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

type Family = 'parchment' | 'alma-mater' | 'roche-limit';

type Sample = {
  t: number;
  why: string;
  splashInDom: boolean;
  splashShown: boolean | null;
  splashGone: boolean | null;
  loader: boolean;
  band: boolean;
  app: boolean;
  dialog: boolean;
};

type Run = { samples: Sample[]; countAnsweredAt: number };

describe('The day-one notice opens after the launch has handed off (FR-R5-1)', () => {
  let browser: Browser | null = null;
  let script = '';
  let indexHtml = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the notice hand-off browser tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: COMPONENTS, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: { 'process.env.NODE_ENV': '"production"' },
      loader: { '.svg': 'dataurl', '.png': 'dataurl' },
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
    const tailwind = compiler.build(
      new Scanner({}).scanFiles([{ content: script, extension: 'js' }])
    );
    // The two stylesheets the first-run page imports alongside main.css (the esbuild plugin blanks
    // every `.css` import), so the page behind the dialog is drawn as the app draws it.
    const pageCss = ['onboarding/onboarding.css', 'settings/providers/providers.css']
      .map((file) => readFileSync(resolve(COMPONENTS, file), 'utf8'))
      .join('\n');
    const css = (
      await postcss([fontScale()]).process(`${tailwind}\n${pageCss}`, { from: undefined })
    ).css;

    // The shipped page, with the stylesheet inlined where the <link> was and the renderer entry
    // replaced by the bundle above. Each swap is asserted, so a markup change cannot leave this
    // testing a stub.
    const source = readFileSync(resolve(DESKTOP, 'index.html'), 'utf8');
    const link = /<link href="\.\/src\/styles\/main\.css" rel="stylesheet"\/>/;
    const entry = /<script type="module" src="\.\/src\/renderer\.tsx"><\/script>/;
    if (!link.test(source) || !entry.test(source)) {
      throw new Error('index.html no longer has the stylesheet link or the renderer entry');
    }
    indexHtml = source
      .replace(link, () => `<style>${css}</style>`)
      .replace(entry, () => '<script src="/entry.js"></script>');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /**
   * One launch, recorded until the dialog has been up for a moment. `query` sets the stub daemon's
   * latencies; `cpu` is the throttling rate the finding was measured at.
   */
  async function launch(
    query: Record<string, string | number>,
    options: { dark?: boolean; family?: Family; cpu?: number } = {}
  ): Promise<{ tab: Page; run: Run }> {
    const { dark = false, family = 'parchment', cpu = 4 } = options;
    const ctx = await browser!.newContext({
      viewport: { width: 1440, height: 900 },
      colorScheme: dark ? 'dark' : 'light',
    });
    const tab = await ctx.newPage();
    await tab.addInitScript(
      ([fam, isDark]) => {
        localStorage.setItem('theme_family', fam as string);
        localStorage.setItem('theme', isDark ? 'dark' : 'light');
        localStorage.setItem('use_system_theme', 'false');
      },
      [family, dark] as const
    );
    await tab.addInitScript({ content: RECORDER });
    await tab.route(
      (url) => url.hostname === 'handoff.test',
      (route) =>
        new URL(route.request().url()).pathname === '/entry.js'
          ? route.fulfill({ body: script, contentType: 'text/javascript' })
          : route.fulfill({ body: indexHtml, contentType: 'text/html' })
    );
    // The daemon: two chats the migration marked private and one public, after `count` ms.
    const cors = {
      'access-control-allow-origin': '*',
      'access-control-allow-headers': '*',
      'access-control-allow-methods': 'GET, POST, OPTIONS',
    };
    const row = (id: string, tier: string, reason: string | null, provider: string) => ({
      id,
      name: id,
      description: '',
      working_dir: '/tmp',
      created_at: '2026-01-01T00:00:00Z',
      updated_at: '2026-01-01T00:00:00Z',
      extension_data: {},
      message_count: 1,
      privacy_tier: tier,
      privacy_reason: reason,
      provider_name: provider,
    });
    await tab.route(
      (url) => url.hostname === 'daemon.test',
      async (route) => {
        if (route.request().method() === 'OPTIONS') {
          return route.fulfill({ status: 204, headers: cors });
        }
        await new Promise((resolve) => setTimeout(resolve, Number(query.count ?? 50)));
        return route.fulfill({
          status: 200,
          headers: { ...cors, 'content-type': 'application/json' },
          body: JSON.stringify({
            sessions: [
              row('s1', 'private', 'backfill:versa_azure', 'versa_azure'),
              row('s2', 'private', 'backfill:ollama', 'ollama'),
              row('s3', 'public', null, 'anthropic'),
            ],
          }),
        });
      }
    );
    const cdp = await ctx.newCDPSession(tab);
    await cdp.send('Emulation.setCPUThrottlingRate', { rate: cpu });
    const params = new URLSearchParams(
      Object.entries(query).map(([key, value]) => [key, String(value)])
    );
    await tab.goto(`http://handoff.test/?${params.toString()}`);
    await tab.waitForSelector('[data-testid="first-run-privacy-notice"]', { timeout: 25_000 });
    // A beat of frames after it opens, so anything that arrives behind it is on the record.
    await tab.waitForTimeout(500);
    const run = await tab.evaluate(() => ({
      samples: (window as unknown as { __samples: Sample[] }).__samples,
      countAnsweredAt: (window as unknown as { __countAnsweredAt: number }).__countAnsweredAt,
    }));
    if (process.env.HANDOFF_TRACE) {
      const lines = [`${JSON.stringify(query)} countAnsweredAt=${run.countAnsweredAt.toFixed(0)}`];
      for (const sample of run.samples) {
        lines.push(
          [
            sample.t.toFixed(0).padStart(6),
            sample.why.padEnd(8),
            ...['splashInDom', 'splashShown', 'splashGone', 'loader', 'band', 'app', 'dialog'].map(
              (key) => `${key}=${Number(sample[key as keyof Sample])}`
            ),
          ].join(' ')
        );
      }
      appendFileSync(process.env.HANDOFF_TRACE, `${lines.join('\n')}\n\n`);
    }
    return { tab, run };
  }

  /** The two lines of the finding, plus the facts that make them mean something. */
  function expectHandedOff(run: Run) {
    const { samples } = run;
    const opened = samples.find((s) => s.dialog);
    expect(opened, 'the dialog opened').toBeDefined();

    // Never under the splash, never on the empty loader: at the first moment it exists...
    expect(opened!.splashInDom).toBe(false);
    expect(opened!.splashGone).toBe(true);
    expect(opened!.loader).toBe(false);
    // ...on a page that has drawn (the first-run band, or the application let through)...
    expect(opened!.band || opened!.app).toBe(true);
    // ...and at every sample after it.
    expect(samples.filter((s) => s.dialog && (s.splashInDom || s.loader))).toEqual([]);
    // The page the dialog stands on never goes away under it.
    expect(samples.filter((s) => s.dialog && !s.band && !s.app)).toEqual([]);
    return opened!;
  }

  /**
   * The dialog came because the launch handed off, not because the gate's bound ran out. The bound
   * is three seconds from the moment the gate mounts, so a guard that never said it had drawn would
   * still pass every line above (late, and on a page that had drawn long since). The dialog follows
   * the later of the two hand-offs by tens of milliseconds on an idle machine; the 1.5 s allowed
   * here is for a busy one, and stays clear of the bound it tells apart (a missing mark measured
   * 2.5 s).
   */
  function expectPromptHandOff(run: Run, opened: Sample) {
    const page = run.samples.find((s) => s.band || s.app)!;
    const gone = run.samples.find((s) => s.splashGone)!;
    expect(opened.t - Math.max(page.t, gone.t)).toBeLessThan(1500);
  }

  it('light, 4x CPU: the count wins the race, the dialog still waits for the splash and the band', async (ctx) => {
    if (!browser) return ctx.skip();
    const { tab, run } = await launch({ count: 50, read: 250, mountAfter: 900 }, { cpu: 4 });
    try {
      const opened = expectHandedOff(run);

      // Anti-vacuity: this is the launch the finding measured. The splash had been drawn before
      // React rendered, the notice was due (the count answered) before the page had drawn, and the
      // guard showed its empty loader for a while with the answer already in.
      const firstBand = run.samples.find((s) => s.band)!;
      expect(run.samples.some((s) => s.splashShown && !s.loader && !s.band && !s.dialog)).toBe(
        true
      );
      expect(run.countAnsweredAt).toBeLessThan(firstBand.t);
      expect(run.samples.some((s) => s.loader && s.splashInDom)).toBe(true);
      // And the order the hand-off puts them in: the band, then the splash gone, then the dialog.
      const gone = run.samples.find((s) => s.splashGone)!;
      expect(firstBand.t).toBeLessThanOrEqual(opened.t);
      expect(gone.t).toBeLessThanOrEqual(opened.t);
      expectPromptHandOff(run, opened);
    } finally {
      await tab.context().close();
    }
  });

  it('dark Alma Mater, 3x CPU: the same, on the other family and mode', async (ctx) => {
    if (!browser) return ctx.skip();
    const { tab, run } = await launch(
      { count: 50, read: 250, mountAfter: 900 },
      { dark: true, family: 'alma-mater', cpu: 3 }
    );
    try {
      expectHandedOff(run);
      expect(run.countAnsweredAt).toBeLessThan(run.samples.find((s) => s.band)!.t);
    } finally {
      await tab.context().close();
    }
  });

  it('a slow count: the page draws first and the dialog opens the moment the answer lands', async (ctx) => {
    if (!browser) return ctx.skip();
    const { tab, run } = await launch(
      { count: 1800, read: 150, mountAfter: 100 },
      { cpu: 2, family: 'roche-limit' }
    );
    try {
      const opened = expectHandedOff(run);
      // The other order of the race: the band was up well before the answer, and the dialog came
      // with the answer, not after any further wait.
      const firstBand = run.samples.find((s) => s.band)!;
      expect(firstBand.t).toBeLessThan(run.countAnsweredAt);
      expect(opened.t - run.countAnsweredAt).toBeLessThan(1500);
    } finally {
      await tab.context().close();
    }
  });

  it('an install with a provider: the dialog opens over the application, not before it', async (ctx) => {
    if (!browser) return ctx.skip();
    const { tab, run } = await launch(
      { count: 50, read: 250, mountAfter: 900, provider: 'versa_azure' },
      { cpu: 4 }
    );
    try {
      const opened = expectHandedOff(run);
      expect(opened.app).toBe(true);
      expect(run.samples.some((s) => s.band)).toBe(false);
      expectPromptHandOff(run, opened);
    } finally {
      await tab.context().close();
    }
  });

  it('a page that draws before the fade is over: the dialog waits for the splash alone', async (ctx) => {
    if (!browser) return ctx.skip();
    // The guard's read answers in 10 ms, so the band is committed long before the splash's 260 ms
    // fade ends. Nothing but the splash is holding the dialog back here.
    const { tab, run } = await launch(
      { count: 50, read: 10, providers: 10, mountAfter: 900 },
      { cpu: 2 }
    );
    try {
      const opened = expectHandedOff(run);
      const band = run.samples.find((s) => s.band)!;
      const gone = run.samples.find((s) => s.splashGone)!;
      // Anti-vacuity: the band really was up, with the splash still in the document, before the
      // splash left. If it were not, the splash would not be what this case tests.
      expect(band.splashInDom).toBe(true);
      expect(gone.t).toBeGreaterThan(band.t);
      expect(opened.t).toBeGreaterThanOrEqual(gone.t);
    } finally {
      await tab.context().close();
    }
  });
});
