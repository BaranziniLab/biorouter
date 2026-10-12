// @vitest-environment node
/**
 * The boot splash, measured in Chromium with the real `index.html` and the real `main.css`
 * (first run vision round 3, FR-R1-7; R-29, R-34).
 *
 * `boot-splash.test.ts` runs the splash's own markup and script in jsdom, which can say what the
 * source contains and nothing about how it looks. This file loads the shipped `index.html` (its
 * inline style, its inline script, its theme-init script) in a real engine, with the compiled
 * stylesheet in place of the `<link>` and the React entry removed, and measures what a person on a
 * slow launch sees:
 *
 * - the splash is the Biorouter wordmark the app draws, not a look-alike: on every platform the
 *   component, given the splash's numbers, rasterises to the same pixels (same face, weight, inks
 *   and bars); on macOS, where the static numbers were read, they also match what
 *   `<BioRouterWordmark/>` measures and the two draw the same pixels unpinned (a platform gate,
 *   `SPLASH_MEASURED_ON`, says why Linux measures the text differently);
 * - its ground is the page's own canvas in every family and mode, so a launch that crosses the
 *   threshold fades onto the page with no grey flash;
 * - it appears at rest, whole in the first frame, with the brand face already loaded;
 * - its ink sits at the window's centre, with the sweep a fixed distance below;
 * - under forced colours and reduced motion it still reads and stays still;
 * - when the app mounts (the real `ThemeProvider`, then the dismissal `renderer.tsx` runs after
 *   it) the splash fades out over its 260 ms in light as well as in dark, because the page's mode
 *   is stamped before React so a mount is never a theme change (FR-R4-1).
 *
 * It needs a Chromium; with none installed it warns and passes, like the other `.browser.test`
 * files.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { fontScale } from '../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const DESKTOP = resolve(here, '..');
const STYLES = resolve(here, 'styles');
const requireFromDesktop = createRequire(resolve(DESKTOP, 'package.json'));

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

/** The in-app wordmark at the size the splash draws it (160 by 49), light or dark. */
const WORDMARK_ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { BioRouterWordmark } from './BioRouterWordmark';
const dark = new URLSearchParams(location.search).get('dark') === '1';
createRoot(document.getElementById('root')!).render(
  <BioRouterWordmark dark={dark} style={{ width: 160, height: 49, display: 'block' }} />
);
`;

/**
 * What `renderer.tsx` mounts, reduced to the two parts that matter to the hand-off: the app's real
 * `ThemeProvider`, and `DismissBootSplash` as its SIBLING, which React runs after the provider's
 * own effect. A copy of either would test the copy.
 */
const MOUNT_ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ThemeProvider } from './contexts/ThemeContext';
function DismissBootSplash() {
  React.useEffect(() => {
    window.__brBootSplash?.dismiss();
  }, []);
  return null;
}
createRoot(document.getElementById('root')!).render(
  <>
    <ThemeProvider>
      <div />
    </ThemeProvider>
    <DismissBootSplash />
  </>
);
`;

const noCss: Plugin = {
  name: 'nocss',
  setup(b) {
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

const FAMILIES = ['parchment', 'alma-mater', 'roche-limit'] as const;

/**
 * The platform the splash's static viewBox and rects were read on (FR-R1-7, Chrome on macOS). The
 * case that holds them to the component's own measurement runs only here; the comment on that case
 * says what Linux measured instead.
 */
const SPLASH_MEASURED_ON = 'darwin';

type Ink = { left: number; right: number; top: number; bottom: number };

describe('The boot splash draws the wordmark at rest on the page ground (FR-R1-7)', () => {
  let browser: Browser | null = null;
  let css = '';
  let wordmarkScript = '';
  let mountScript = '';
  let indexHtml = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the boot splash browser tests.');
      return;
    }
    const bundle = await build({
      stdin: {
        contents: WORDMARK_ENTRY,
        loader: 'tsx',
        resolveDir: resolve(here, 'components', 'icons'),
        sourcefile: 'entry.tsx',
      },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: { 'process.env.NODE_ENV': '"production"' },
      plugins: [noCss],
      logLevel: 'silent',
    });
    wordmarkScript = bundle.outputFiles[0].text;
    const mount = await build({
      stdin: {
        contents: MOUNT_ENTRY,
        loader: 'tsx',
        resolveDir: here,
        sourcefile: 'mount.tsx',
      },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: { 'process.env.NODE_ENV': '"production"' },
      plugins: [noCss],
      logLevel: 'silent',
    });
    mountScript = mount.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    const tailwind = compiler.build(
      new Scanner({}).scanFiles([{ content: wordmarkScript, extension: 'js' }])
    );
    css = (await postcss([fontScale()]).process(tailwind, { from: undefined })).css;

    // The shipped page, with the stylesheet inlined where the <link> was and the React entry
    // dropped. Each swap is asserted, so a markup change cannot leave this testing a stub.
    const source = readFileSync(resolve(DESKTOP, 'index.html'), 'utf8');
    const link = /<link href="\.\/src\/styles\/main\.css" rel="stylesheet"\/>/;
    const entry = /<script type="module" src="\.\/src\/renderer\.tsx"><\/script>/;
    if (!link.test(source) || !entry.test(source)) {
      throw new Error('index.html no longer has the stylesheet link or the renderer entry');
    }
    indexHtml = source.replace(link, () => `<style>${css}</style>`).replace(entry, '');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** The real page at an http origin (so localStorage works), theme chosen the way the app does. */
  async function openSplash(
    options: {
      width?: number;
      height?: number;
      dark?: boolean;
      family?: (typeof FAMILIES)[number];
      media?: { forcedColors?: 'active'; reducedMotion?: 'reduce' };
      scale?: number;
      /** `stored`: a saved choice (the default). `none`: a fresh install, so the OS decides.
       *  `blocked`: storage that throws on every read. */
      storage?: 'stored' | 'none' | 'blocked';
    } = {}
  ): Promise<Page> {
    const {
      width = 1440,
      height = 900,
      dark = false,
      family = 'parchment',
      media,
      scale,
      storage = 'stored',
    } = options;
    const ctx = await browser!.newContext({
      viewport: { width, height },
      colorScheme: dark ? 'dark' : 'light',
      deviceScaleFactor: scale ?? 1,
      ...(media?.forcedColors ? { forcedColors: media.forcedColors } : {}),
      ...(media?.reducedMotion ? { reducedMotion: media.reducedMotion } : {}),
    });
    const tab = await ctx.newPage();
    if (storage === 'stored') {
      await tab.addInitScript(
        ([fam, isDark]) => {
          localStorage.setItem('theme_family', fam as string);
          localStorage.setItem('theme', isDark ? 'dark' : 'light');
          localStorage.setItem('use_system_theme', 'false');
        },
        [family, dark] as const
      );
    } else if (storage === 'blocked') {
      await tab.addInitScript(() => {
        Storage.prototype.getItem = () => {
          throw new Error('storage is blocked');
        };
      });
    }
    await tab.route('http://splash.test/', (route) =>
      route.fulfill({ body: indexHtml, contentType: 'text/html' })
    );
    await tab.goto('http://splash.test/');
    return tab;
  }

  /** Resolves in the first animation frame the splash is on screen, with what that frame holds. */
  const firstFrame = (tab: Page) =>
    tab.evaluate(
      () =>
        new Promise<{
          interLoaded: boolean;
          animations: string[];
          partOpacities: string[];
          partFilters: string[];
          bioLeft: number;
        }>((res) => {
          const tick = () => {
            const splash = window.__brBootSplash;
            if (splash && splash.isShown()) {
              const parts = [...document.querySelectorAll('#br-boot .br-wordmark > *')];
              res({
                interLoaded: document.fonts.check('600 100px Inter'),
                animations: document
                  .getAnimations()
                  .map(
                    (a) => (a as unknown as { animationName?: string }).animationName ?? 'other'
                  ),
                partOpacities: parts.map((p) => getComputedStyle(p).opacity),
                partFilters: parts.map((p) => getComputedStyle(p).filter),
                bioLeft: document
                  .querySelector('#br-boot .br-wordmark tspan')!
                  .getBoundingClientRect().left,
              });
              return;
            }
            requestAnimationFrame(tick);
          };
          requestAnimationFrame(tick);
        })
    );

  /** The ink of whatever is drawn above `limitY` (CSS px), read from a real screenshot. */
  async function inkAbove(tab: Page, limitY: number, scale: number): Promise<Ink> {
    const png = await tab.screenshot();
    const scratch = await browser!.newPage();
    try {
      return await scratch.evaluate(
        async ([b64, limit, dpr]) => {
          const img = new Image();
          img.src = `data:image/png;base64,${b64}`;
          await img.decode();
          const canvas = document.createElement('canvas');
          canvas.width = img.width;
          canvas.height = img.height;
          const g = canvas.getContext('2d')!;
          g.drawImage(img, 0, 0);
          const { data } = g.getImageData(0, 0, img.width, img.height);
          const ground = [data[0], data[1], data[2]];
          let left = img.width;
          let right = -1;
          let top = img.height;
          let bottom = -1;
          const rows = Math.min(img.height, Math.floor((limit as number) * (dpr as number)));
          for (let y = 0; y < rows; y++) {
            for (let x = 0; x < img.width; x++) {
              const i = (y * img.width + x) * 4;
              const d =
                Math.abs(data[i] - ground[0]) +
                Math.abs(data[i + 1] - ground[1]) +
                Math.abs(data[i + 2] - ground[2]);
              if (d > 90) {
                if (x < left) left = x;
                if (x > right) right = x;
                if (y < top) top = y;
                if (y > bottom) bottom = y;
              }
            }
          }
          const s = dpr as number;
          return { left: left / s, right: (right + 1) / s, top: top / s, bottom: (bottom + 1) / s };
        },
        [png.toString('base64'), limitY, scale] as const
      );
    } finally {
      await scratch.close();
    }
  }

  type Drawn = {
    viewBox: number[];
    rects: { x: number; y: number; w: number; h: number }[];
    fontSize: string | null;
    weight: string;
    family: string;
  };

  /** The real component, in the real engine, at the size the splash draws it, with what it measured. */
  async function drawApp(): Promise<{ app: Page; live: Drawn }> {
    const app = await browser!.newPage({ viewport: { width: 400, height: 200 } });
    await app.setContent(
      `<!doctype html><html><head><style>${css}</style></head>` +
        `<body style="margin:0;background:#fff"><div id="root"></div></body></html>`
    );
    await app.addScriptTag({ content: wordmarkScript });
    await app.waitForSelector('#root svg rect');
    await app.evaluate(() => document.fonts.ready);
    await app.waitForTimeout(300);
    const live = await app.evaluate(() => {
      const svg = document.querySelector('#root svg')!;
      const rects = [...svg.querySelectorAll('rect')].map((r) => ({
        x: Number(r.getAttribute('x')),
        y: Number(r.getAttribute('y')),
        w: Number(r.getAttribute('width')),
        h: Number(r.getAttribute('height')),
      }));
      const text = svg.querySelector('text')!;
      return {
        viewBox: svg.getAttribute('viewBox')!.split(' ').map(Number),
        rects,
        fontSize: text.getAttribute('font-size'),
        weight: getComputedStyle(text).fontWeight,
        family: getComputedStyle(text).fontFamily,
      };
    });
    return { app, live };
  }

  /** The splash page, shown, light, with its static numbers and the box it is drawn in. */
  async function drawSplash(): Promise<{ tab: Page; splash: Drawn & { size: number[] } }> {
    const tab = await openSplash();
    await firstFrame(tab);
    const splash = await tab.evaluate(() => {
      const svg = document.querySelector('#br-boot .br-wordmark')!;
      const rects = [...svg.querySelectorAll('rect')].map((r) => ({
        x: Number(r.getAttribute('x')),
        y: Number(r.getAttribute('y')),
        w: Number(r.getAttribute('width')),
        h: Number(r.getAttribute('height')),
      }));
      const text = svg.querySelector('text')!;
      const box = svg.getBoundingClientRect();
      return {
        viewBox: svg.getAttribute('viewBox')!.split(/\s+/).map(Number),
        rects,
        fontSize: text.getAttribute('font-size'),
        weight: getComputedStyle(text).fontWeight,
        family: getComputedStyle(text).fontFamily,
        size: [box.width, box.height],
      };
    });
    return { tab, splash };
  }

  /** How many pixels of the splash's box differ visibly from the component's 160 by 49. */
  async function differingPixels(app: Page, tab: Page) {
    const clip = { x: 0, y: 0, width: 160, height: 49 };
    const appPng = await app.screenshot({ clip });
    const box = await tab.evaluate(() => {
      const r = document.querySelector('#br-boot .br-wordmark')!.getBoundingClientRect();
      return { x: r.left, y: r.top, width: r.width, height: r.height };
    });
    // The splash sits on the page ground, the component on white: both are white in light mode.
    const splashPng = await tab.screenshot({ clip: box });
    const scratch = await browser!.newPage();
    try {
      return await scratch.evaluate(
        async ([a, b]) => {
          const read = async (b64: string) => {
            const img = new Image();
            img.src = `data:image/png;base64,${b64}`;
            await img.decode();
            const c = document.createElement('canvas');
            c.width = img.width;
            c.height = img.height;
            const g = c.getContext('2d')!;
            g.drawImage(img, 0, 0);
            return g.getImageData(0, 0, img.width, img.height).data;
          };
          const [da, db] = [await read(a), await read(b)];
          let n = 0;
          for (let i = 0; i < da.length; i += 4) {
            const d =
              Math.abs(da[i] - db[i]) +
              Math.abs(da[i + 1] - db[i + 1]) +
              Math.abs(da[i + 2] - db[i + 2]);
            if (d > 48) n += 1;
          }
          return { n, total: da.length / 4 };
        },
        [appPng.toString('base64'), splashPng.toString('base64')] as const
      );
    } finally {
      await scratch.close();
    }
  }

  it('is the wordmark the app draws: the same face, inks and bars, the same pixels at the same numbers', async () => {
    if (!browser) return;
    const { app, live } = await drawApp();
    const { tab, splash } = await drawSplash();

    expect(splash.rects).toHaveLength(2);
    expect(live.rects).toHaveLength(2);
    expect(splash.fontSize).toBe(live.fontSize);
    expect(splash.weight).toBe(live.weight);
    expect(splash.family).toBe(live.family);
    expect(splash.size).toEqual([160, 49]);

    // Everything but the measurement: the component is given the splash's numbers (its viewBox
    // and its two rects; React has settled, so nothing re-measures over them) and the two must
    // then rasterise to the same pixels. Its own text, face, weight and inks stay its own, so a
    // letter-spacing change (1109 pixels) or a swapped ink (769) in the splash still fails here on
    // every platform, as a wrong weight fails the line above. A bar moved or resized in the markup
    // does not, because its numbers are the ones pinned: whether the numbers are the ones the
    // component measures is the next case, which can only hold on the platform they were read on.
    const pinned = await app.evaluate(
      ([viewBox, rects]) => {
        const svg = document.querySelector('#root svg')!;
        svg.setAttribute('viewBox', viewBox.join(' '));
        [...svg.querySelectorAll('rect')].forEach((r, i) => {
          r.setAttribute('x', String(rects[i].x));
          r.setAttribute('y', String(rects[i].y));
          r.setAttribute('width', String(rects[i].w));
          r.setAttribute('height', String(rects[i].h));
        });
        return new Promise<string>((res) =>
          requestAnimationFrame(() =>
            requestAnimationFrame(() => res(svg.getAttribute('viewBox')!))
          )
        );
      },
      [splash.viewBox, splash.rects] as const
    );
    expect(pinned.split(' ').map(Number)).toEqual(splash.viewBox);

    // Same engine, same face, same numbers: the two rasterise to the same pixels.
    const differing = await differingPixels(app, tab);
    await app.close();
    await tab.context().close();
    expect(
      differing.n,
      `${differing.n} of ${differing.total} pixels differ from the app wordmark at the splash's numbers`
    ).toBeLessThan(40);
  }, 60_000);

  /*
   * PLATFORM GATE: macOS only, where the static numbers in index.html were read (FR-R1-7).
   *
   * The splash's viewBox and rects are what <BioRouterWordmark/> measured (getBBox and the
   * character positions) in Chrome on macOS, which measures text through CoreText. Chrome on
   * Linux measures it through FreeType and Skia, and the same bundled Inter does not come out the
   * same: on CI's ubuntu runner with Google Chrome stable (Frontend run 38204107261, job
   * 114663666236, at a7d069304) the component's viewBox x was 0.12 user units from the markup's
   * -7.21 (about 0.04 CSS px at the 160 px drawn width; the residue of the reported difference,
   * 0.1200000000000001, places it at the first of the four numbers, which the component derives
   * from the text's measured height). Windows' DirectWrite is a third text stack and has not been
   * measured. The measured drawing cannot match the static one there either: on macOS, moving the
   * splash's x by +0.12 or -0.12 alone made 167 and 48 pixels differ (the bound is 40), measured
   * 2026-10-12. So neither the two-decimal geometry nor the unpinned pixel comparison is a
   * property of the splash off macOS; the case above holds everything else on every platform.
   * Regenerate the numbers (index.html says how) on macOS, or move SPLASH_MEASURED_ON to the
   * platform they were read on.
   */
  it.runIf(process.platform === SPLASH_MEASURED_ON)(
    'carries the numbers the app measures, on the platform they were read on: the same geometry and the same pixels unpinned',
    async () => {
      if (!browser) return;
      const { app, live } = await drawApp();
      const { tab, splash } = await drawSplash();

      // The markup keeps two decimals; the component measures more. Half a hundredth is the rounding.
      const close = (a: number[], b: number[]) =>
        a.forEach((v, i) => expect(Math.abs(v - b[i])).toBeLessThanOrEqual(0.006));
      close(splash.viewBox, live.viewBox);
      expect(splash.rects).toHaveLength(2);
      splash.rects.forEach((r, i) =>
        close(
          [r.x, r.y, r.w, r.h],
          [live.rects[i].x, live.rects[i].y, live.rects[i].w, live.rects[i].h]
        )
      );

      // The component as it measured itself, not pinned: the two rasterise to the same pixels. A
      // sub-pixel of slack covers the two-decimal rounding of the box; anything visible (a weight,
      // a face, a bar position, an ink) moves far more than that.
      const differing = await differingPixels(app, tab);
      await app.close();
      await tab.context().close();
      expect(
        differing.n,
        `${differing.n} of ${differing.total} pixels differ from the app wordmark`
      ).toBeLessThan(40);
    },
    60_000
  );

  it("paints the page's own canvas in every family and mode, and the window ground under it", async () => {
    if (!browser) return;
    const tab = await openSplash();
    await firstFrame(tab);
    const grounds = await tab.evaluate(
      (families) => {
        const root = document.documentElement;
        const splash = document.getElementById('br-boot')!;
        const probe = document.createElement('i');
        document.body.appendChild(probe);
        const read = (token: string) => {
          probe.style.background = `var(${token})`;
          return getComputedStyle(probe).backgroundColor;
        };
        const out: { scope: string; splash: string; canvas: string; app: string; muted: string }[] =
          [];
        for (const family of families) {
          for (const dark of [false, true]) {
            root.setAttribute('data-theme', family);
            root.classList.toggle('dark', dark);
            out.push({
              scope: `${family} ${dark ? 'dark' : 'light'}`,
              splash: getComputedStyle(splash).backgroundColor,
              canvas: read('--background-canvas'),
              app: read('--background-app'),
              muted: read('--background-muted'),
            });
          }
        }
        return out;
      },
      [...FAMILIES]
    );
    expect(grounds).toHaveLength(6);
    for (const g of grounds) {
      expect(g.splash, `${g.scope}: the splash ground is the page canvas`).toBe(g.canvas);
      expect(g.splash, `${g.scope}: and the window ground behind it`).toBe(g.app);
      expect(g.splash, `${g.scope}: never the muted grey it used to be`).not.toBe(g.muted);
    }
    await tab.context().close();
  }, 60_000);

  it('appears whole in its first frame: the brand face loaded, nothing fading, sliding or blurred', async () => {
    if (!browser) return;
    for (const dark of [false, true]) {
      const tab = await openSplash({ dark });
      const first = await firstFrame(tab);
      expect(
        first.interLoaded,
        `${dark ? 'dark' : 'light'}: Inter is loaded when the splash shows`
      ).toBe(true);
      // The only animation on the screen is the sweep.
      expect(first.animations).toEqual(['br-boot-sweep']);
      expect(new Set(first.partOpacities)).toEqual(new Set(['1']));
      expect(new Set(first.partFilters)).toEqual(new Set(['none']));
      // And the letters do not move over the frames after it (no slide-in).
      const later = await tab.evaluate(
        () =>
          new Promise<number[]>((res) => {
            const lefts: number[] = [];
            const tick = () => {
              lefts.push(
                document.querySelector('#br-boot .br-wordmark tspan')!.getBoundingClientRect().left
              );
              if (lefts.length < 20) requestAnimationFrame(tick);
              else res(lefts);
            };
            requestAnimationFrame(tick);
          })
      );
      expect(new Set(later.map((n) => n.toFixed(2))).size, 'the lockup moved between frames').toBe(
        1
      );
      expect(later[0].toFixed(2)).toBe(first.bioLeft.toFixed(2));
      await tab.context().close();
    }
  }, 60_000);

  it("sits the wordmark's ink at the window's centre, with the sweep a fixed distance below", async () => {
    if (!browser) return;
    // 1440 x 900 is the usual window; 1048 x 700 is the smallest the app opens at.
    for (const [width, height] of [
      [1440, 900],
      [1048, 700],
    ] as const) {
      const tab = await openSplash({ width, height, scale: 2 });
      await firstFrame(tab);
      const sweep = await tab.evaluate(() => {
        const r = document.querySelector('#br-boot .br-sweep')!.getBoundingClientRect();
        return { top: r.top, left: r.left, width: r.width, height: r.height };
      });
      const ink = await inkAbove(tab, sweep.top - 4, 2);
      const label = `${width}x${height}`;
      expect(
        Math.abs((ink.left + ink.right) / 2 - width / 2),
        `${label}: ink centre x`
      ).toBeLessThanOrEqual(1);
      expect(
        Math.abs((ink.top + ink.bottom) / 2 - height / 2),
        `${label}: ink centre y`
      ).toBeLessThanOrEqual(1);
      // The sweep is centred under it, a small fixed distance from the underline.
      expect(
        Math.abs(sweep.left + sweep.width / 2 - width / 2),
        `${label}: sweep centre x`
      ).toBeLessThanOrEqual(0.5);
      expect(
        sweep.top - ink.bottom,
        `${label}: gap from the underline to the sweep`
      ).toBeGreaterThanOrEqual(20);
      expect(
        sweep.top - ink.bottom,
        `${label}: gap from the underline to the sweep`
      ).toBeLessThanOrEqual(28);
      await tab.context().close();
    }
  }, 90_000);

  it('keeps the wordmark in the brand inks in every family, with navy lifting to teal in the dark', async () => {
    if (!browser) return;
    const inks: Record<string, string[]> = {};
    for (const family of FAMILIES) {
      for (const dark of [false, true]) {
        const tab = await openSplash({ family, dark });
        await firstFrame(tab);
        inks[`${family} ${dark ? 'dark' : 'light'}`] = await tab.evaluate(() => [
          getComputedStyle(document.querySelector('#br-boot .br-wordmark tspan.br-navy')!).fill,
          getComputedStyle(document.querySelector('#br-boot .br-wordmark tspan.br-coral')!).fill,
          getComputedStyle(document.querySelector('#br-boot .br-wordmark rect.br-navy')!).fill,
        ]);
        await tab.context().close();
      }
    }
    const light = ['rgb(5, 32, 73)', 'rgb(184, 90, 50)', 'rgb(5, 32, 73)'];
    const dark = ['rgb(24, 163, 172)', 'rgb(184, 90, 50)', 'rgb(24, 163, 172)'];
    for (const [scope, got] of Object.entries(inks)) {
      expect(got, scope).toEqual(scope.endsWith('dark') ? dark : light);
    }
  }, 120_000);

  it('reads in the one ink forced colours guarantees, instead of navy on a canvas it did not choose', async () => {
    if (!browser) return;
    const tab = await openSplash({ media: { forcedColors: 'active' } });
    await firstFrame(tab);
    const fills = await tab.evaluate(() => {
      const probe = document.createElement('i');
      probe.style.color = 'CanvasText';
      document.body.appendChild(probe);
      const canvasText = getComputedStyle(probe).color;
      return {
        canvasText,
        bio: getComputedStyle(document.querySelector('#br-boot tspan.br-navy')!).fill,
        router: getComputedStyle(document.querySelector('#br-boot tspan.br-coral')!).fill,
        bar: getComputedStyle(document.querySelector('#br-boot rect.br-navy')!).fill,
        barRight: getComputedStyle(document.querySelector('#br-boot rect.br-coral')!).fill,
      };
    });
    expect(fills.bio).toBe(fills.canvasText);
    expect(fills.router).toBe(fills.canvasText);
    expect(fills.bar).toBe(fills.canvasText);
    expect(fills.barRight).toBe(fills.canvasText);
    await tab.context().close();
  }, 60_000);

  it('stands still under reduced motion: the sweep stops and the logo is exactly where it was', async () => {
    if (!browser) return;
    const calm = await openSplash({ media: { reducedMotion: 'reduce' } });
    const first = await firstFrame(calm);
    expect(first.animations, 'nothing animates under reduced motion').toEqual([]);
    const calmBox = await calm.evaluate(() => {
      const r = document.querySelector('#br-boot .br-wordmark')!.getBoundingClientRect();
      return [r.left, r.top, r.width, r.height];
    });
    await calm.context().close();

    const moving = await openSplash();
    await firstFrame(moving);
    const movingBox = await moving.evaluate(() => {
      const r = document.querySelector('#br-boot .br-wordmark')!.getBoundingClientRect();
      return [r.left, r.top, r.width, r.height];
    });
    await moving.context().close();
    expect(calmBox).toEqual(movingBox);
  }, 60_000);

  /** The classes <html> carries once the page's own theme script has run, nothing else added. */
  const modeClasses = (tab: Page) =>
    tab.evaluate(() => [...document.documentElement.classList].sort());

  it('stamps the mode on <html> before React, light as well as dark, whatever storage holds (FR-R4-1)', async () => {
    if (!browser) return;
    // The theme provider decides "did the mode change?" from this class. A light page used to
    // carry no class at all, so every light mount looked like a change from nothing to light.
    const cases = [
      { storage: 'stored', dark: false, want: ['light'] },
      { storage: 'stored', dark: true, want: ['dark'] },
      { storage: 'none', dark: false, want: ['light'] },
      { storage: 'none', dark: true, want: ['dark'] },
      { storage: 'blocked', dark: false, want: ['light'] },
      { storage: 'blocked', dark: true, want: ['dark'] },
    ] as const;
    for (const { storage, dark, want } of cases) {
      const tab = await openSplash({ storage, dark });
      expect(
        await modeClasses(tab),
        `${storage} storage, ${dark ? 'dark' : 'light'} system`
      ).toEqual(want);
      // The theme script retries once the document is parsed; that second run changes nothing.
      await tab.waitForTimeout(120);
      expect(await modeClasses(tab), `${storage} storage, after the retry`).toEqual(want);
      await tab.context().close();
    }
  }, 90_000);

  it('fades out over its 260ms when the app mounts, in light and in dark, with the mode saved or not (FR-R4-1)', async () => {
    if (!browser) return;
    // Every launch path: a saved choice, a fresh install following the OS, in both modes.
    const cases = [
      { storage: 'stored', dark: false },
      { storage: 'stored', dark: true },
      { storage: 'none', dark: false },
      { storage: 'none', dark: true },
    ] as const;
    for (const { storage, dark } of cases) {
      const label = `${storage} storage, ${dark ? 'dark' : 'light'}`;
      const tab = await openSplash({ storage, dark });
      await firstFrame(tab);

      // Read in the same task as the dismissal, before any frame: what the style engine makes of
      // `br-out` is decided by what <html> carries at that moment, so this is the whole question.
      await tab.evaluate(() => {
        const splash = window.__brBootSplash!;
        const real = splash.dismiss;
        splash.dismiss = () => {
          real();
          const el = document.getElementById('br-boot')!;
          const root = document.documentElement;
          (window as unknown as { __dismissProbe: unknown }).__dismissProbe = {
            switching: root.classList.contains('biorouter-theme-switching'),
            transitionDuration: getComputedStyle(el).transitionDuration,
            fades: el
              .getAnimations()
              .filter(
                (a) =>
                  (a as unknown as { transitionProperty?: string }).transitionProperty === 'opacity'
              )
              .map((a) => Number(a.effect!.getTiming().duration)),
          };
        };
      });
      await tab.addScriptTag({ content: mountScript });
      // React commits on its own schedule; the probe exists once the dismissal has run.
      await tab.waitForFunction(
        () => (window as unknown as { __dismissProbe?: unknown }).__dismissProbe !== undefined,
        null,
        { timeout: 10_000 }
      );
      const probe = await tab.evaluate(
        () =>
          (
            window as unknown as {
              __dismissProbe: { switching: boolean; transitionDuration: string; fades: number[] };
            }
          ).__dismissProbe
      );

      expect(probe.switching, `${label}: the mount is not a theme change`).toBe(false);
      expect(probe.transitionDuration, `${label}: the fade keeps its duration`).toBe('0.26s');
      expect(probe.fades, `${label}: an opacity transition is running on the splash`).toEqual([
        260,
      ]);

      // And it still takes itself down once the fade has run.
      await tab.waitForFunction(() => !document.getElementById('br-boot'), null, {
        timeout: 10_000,
      });
      await tab.context().close();
    }
  }, 120_000);
});
