// @vitest-environment node
/**
 * The top of Settings and of the first-run setup page, at rest, measured in Chromium (owner,
 * 2026-10-12: "the user interface doesn't go all the way up even when the scroll is scrolled all
 * the way up ... leave plenty of headroom at the top").
 *
 * **The defect.** The `ScrollArea` root is `overflow: hidden` around its viewport, and its viewport
 * was not positioned, so every absolutely positioned box in the scrolled content without a
 * positioned box of its own above it was placed against the ROOT: an InfoTip's `sr-only`
 * description on every row with help. Those 1px spans sat past the viewport's clip and gave the
 * root up to 1,600px of overflow. A wheel cannot scroll a hidden box; `scrollIntoView` can, and
 * Settings calls it to open at a section (the update notice's "update", "usage", "project", and by
 * the 10px of scroll padding even "privacy"). The root scrolled with the viewport, the wheel
 * brought only the viewport back to 0, and the page's first lines stayed under the band, on every
 * tab, until Settings was reopened. Wheel scrolling alone never showed it, which is why it read
 * as "sometimes".
 *
 * **What holds it now.** `.br-scroll-viewport` is `position: relative` (`main.css`), so those
 * boxes are the viewport's and the root has nothing to scroll, and `ui/scroll-area.tsx` puts the
 * root back if anything ever scrolls it. Either one alone holds the top; the instrument at the end
 * removes both and watches the defect come back, and removes each in turn and watches it stay
 * gone. The page's first row then sits `--page-headroom` (24px) under the band, one value for
 * Settings and the setup page (`.br-page-headroom`), where Settings used to keep 20px of its own.
 *
 * **What is real.** `SettingsView` (its band, tabs, scroller, reading column and deep-link
 * scrolling) and `ProviderGuard`'s first-run page are bundled with esbuild and rendered by React
 * in Chromium, styled by `styles/main.css` compiled by Tailwind with the classes the bundle uses,
 * plus the component stylesheets they import. The Settings tabs' sections are stubbed with the real
 * `SettingSection` and `SettingRow` (every row has help, so every row has the InfoTip span that
 * escaped), and the provider catalog with a list of rows behind real InfoTips; the daemon, the
 * router state and the privacy notice are stubbed because a bare page has none.
 *
 * Set `PAGE_HEADROOM_SHOTS=<dir>` to save a screenshot of every state measured.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const COMPONENTS = resolve(here, '..');
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const SHOTS = process.env.PAGE_HEADROOM_SHOTS;

/** What the owner asked for, in pixels: the first row at least this far under the band. */
const MIN_HEADROOM = 24;
/** The band's height (`--chrome-height`), which the fix must not move. */
const BAND = 44;
/** The window sizes measured: the smallest the window allows (its width floor), and a laptop. */
const SIZES = [
  { width: 1048, height: 600 },
  { width: 1440, height: 900 },
] as const;

/** A Settings tab's sections: [DOM id, title, rows]. Every row carries help (an InfoTip). */
type Sections = Array<[string, string, number]>;
const MODELS: Sections = [
  ['settings-default-model', 'Default model', 8],
  ['settings-local-models', 'Local models', 8],
  ['settings-models-danger-zone', 'Danger zone', 3],
];
const CHAT: Sections = [
  ['settings-approvals', 'Approvals', 6],
  ['settings-display', 'Display', 6],
  ['settings-capabilities', 'Capabilities', 6],
  ['settings-memory', 'Memory', 5],
  ['settings-contexts', 'Contexts', 5],
  ['settings-app-sdk', 'App SDK', 4],
  ['settings-project', 'Project', 3],
];
const APP_TAIL: Sections = [
  ['settings-general', 'General', 6],
  ['settings-appearance', 'Appearance', 4],
  ['settings-usage', 'Usage', 4],
  ['settings-about', 'About', 3],
  ['settings-app-danger-zone', 'Danger zone', 2],
];

const tabStub = (sections: Sections, wrap?: string) => `
  import * as React from 'react';
  import { Sections } from 'stub:sections';
  export default function Stub() {
    const body = React.createElement(Sections, { sections: ${JSON.stringify(sections)} });
    return ${wrap ? `React.createElement('div', { className: '${wrap}' }, body)` : 'body'};
  }
`;

/** Modules the bundle takes from here instead of the app, by import specifier. */
const STUBS: Array<{ filter: RegExp; contents: string }> = [
  {
    filter: /^stub:sections$/,
    contents: `
      import * as React from 'react';
      import { SettingSection, SettingRow } from '${COMPONENTS}/ui/setting-row';
      import { Switch } from '${COMPONENTS}/ui/switch';
      const h = React.createElement;
      export function Sections({ sections }) {
        return h(React.Fragment, null, sections.map(([id, title, rows]) =>
          h(SettingSection, { key: id, id, title, help: 'What the ' + title + ' section holds.' },
            Array.from({ length: rows }, (_, i) =>
              h(SettingRow, {
                key: i,
                label: title + ' setting ' + (i + 1),
                help: 'What ' + title + ' setting ' + (i + 1) + ' changes.',
                controlId: id + '-control-' + i,
              }, h(Switch, { checked: i % 2 === 0, onCheckedChange: () => {} }))))));
      }
    `,
  },
  // Settings: the five sections components, and what Settings warms while it opens.
  { filter: /\/models\/ModelsSection$/, contents: tabStub(MODELS) },
  { filter: /\/chat\/ChatSettingsSection$/, contents: tabStub(CHAT, 'pb-8') },
  {
    filter: /\/config\/ConfigSettings$/,
    contents: tabStub([['settings-configuration', 'Configuration', 1]]),
  },
  { filter: /\/privacy\/PrivacyPanel$/, contents: tabStub([['settings-privacy', 'Privacy', 2]]) },
  { filter: /\/app\/AppSettingsSection$/, contents: tabStub(APP_TAIL) },
  {
    filter: /\/disclosureCopy$/,
    contents: `export const readDisclosureAcknowledgement = async () => null;`,
  },
  { filter: /\/warmSettings$/, contents: `export const useWarmSettings = () => {};` },
  // The first-run page: a daemon with no provider and no skip, and no day-one notice.
  {
    filter: /^\.\/ConfigContext$/,
    contents: `
      const api = {
        read: async (key) => (key === 'BIOROUTER_PROVIDER' ? '' : false),
        upsert: async () => {},
        getProviders: async () => [],
      };
      export const useConfig = () => api;
    `,
  },
  {
    filter: /\/settings\/providers\/ProviderCatalog$/,
    contents: `
      import * as React from 'react';
      import { createPortal } from 'react-dom';
      import { InfoTip } from '${COMPONENTS}/ui/info-tip';
      import '${COMPONENTS}/settings/providers/providers.css';
      const h = React.createElement;
      export default function ProviderCatalog({ tabsHost }) {
        const tabs = h('div', { role: 'tablist', 'aria-label': 'Provider kinds', className: 'flex' },
          ['Local', 'Institutional', 'Public'].map((name) =>
            h('button', { key: name, role: 'tab', type: 'button' }, name)));
        return h(React.Fragment, null,
          tabsHost ? createPortal(tabs, tabsHost) : null,
          h('p', { 'data-first-row': '', className: 'text-supporting text-text-muted' },
            'Private because inference runs on this machine. Nothing leaves it.'),
          Array.from({ length: 24 }, (_, i) =>
            h('div', { key: i, className: 'br-first-run-card', style: { marginTop: 16 } },
              h('div', { className: 'flex items-center gap-2' },
                h('span', { className: 'text-label' }, 'Provider ' + (i + 1)),
                h(InfoTip, { label: 'Provider ' + (i + 1), help: 'Where provider ' + (i + 1) + ' runs.' })),
              h('p', { className: 'text-supporting text-text-muted' }, 'A line about provider ' + (i + 1) + '.'))));
      }
    `,
  },
  { filter: /^\.\.\/toasts$/, contents: `export const toastService = { error() {} };` },
  {
    filter: /\/onboarding\/CommercialSetupCard$/,
    contents: `export const persistDetectedProviderSetup = async () => {};`,
  },
  {
    filter: /\/utils\/navigationUtils$/,
    contents: `export const createNavigationHandler = () => () => {};`,
  },
  { filter: /\/utils\/surface$/, contents: `export const isBrowserSurface = () => false;` },
  {
    filter: /\/privacy\/HostManagedModelPanel$/,
    contents: `export const HostManagedModelPanel = () => null;`,
  },
  {
    filter: /\/privacy\/firstRunNoticeDecision$/,
    contents: `export const useFirstRunNoticeHold = () => false;`,
  },
  {
    filter: /\/privacy\/firstRunNoticeHandoff$/,
    contents: `export const markFirstPageDrawn = () => {};`,
  },
  {
    filter: /\/Layout\/TitlebarControls$/,
    contents: `export const getTitlebarControlInset = (mac) => (mac ? 100 : 16);`,
  },
  {
    filter: /\/SecureStorageNotice$/,
    contents: `export const KEY_STORAGE_NOTICE = 'Keys stay in your system keychain.';`,
  },
];

/** A defect put back into one source file (relative to `components/`). Must match, or throw. */
type Mutation = { file: string; from: string; to: string };

function stubPlugin(mutations: Mutation[]): Plugin {
  return {
    name: 'page-headroom-stubs',
    setup(b) {
      for (const { filter, contents } of STUBS) {
        b.onResolve({ filter }, (args) => ({
          path: args.path,
          namespace: 'stub',
          pluginData: contents,
        }));
      }
      b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
        contents: args.pluginData as string,
        loader: 'js',
        resolveDir: here,
      }));
      const hits = new Set<string>();
      for (const mutation of mutations) {
        const path = resolve(COMPONENTS, mutation.file);
        b.onLoad({ filter: /\.tsx?$/ }, (args) => {
          if (args.path !== path) return undefined;
          const source = readFileSync(path, 'utf8');
          if (!source.includes(mutation.from)) {
            throw new Error(`the mutation of ${mutation.file} matches nothing`);
          }
          hits.add(mutation.file);
          return {
            contents: source.replace(mutation.from, mutation.to),
            loader: path.endsWith('.tsx') ? 'tsx' : 'ts',
          };
        });
      }
      b.onEnd(() => {
        for (const { file } of mutations) {
          if (!hits.has(file)) throw new Error(`the mutated ${file} was never loaded`);
        }
      });
    },
  };
}

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import SettingsView from './SettingsView';
import ProviderGuard from '../ProviderGuard';

const surface = (window as any).__surface;
const section = (window as any).__section;

createRoot(document.getElementById('root')!).render(
  surface === 'setup' ? (
    <MemoryRouter>
      <ProviderGuard didSelectProvider={false}>
        <p>The application</p>
      </ProviderGuard>
    </MemoryRouter>
  ) : (
    <div id="shell">
      <SettingsView onClose={() => {}} setView={() => {}} viewOptions={section ? { section } : {}} />
    </div>
  )
);
`;

/** The app's stylesheet edited before Tailwind compiles it. Must match, or throw. */
type CssEdit = { from: string; to: string };

type Variant = { mutations?: Mutation[]; css?: CssEdit[] };
type Prepared = { script: string; css: string };

async function prepare(variant: Variant = {}): Promise<Prepared> {
  const out = await build({
    stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
    bundle: true,
    write: false,
    outdir: '/page-headroom',
    format: 'iife',
    jsx: 'automatic',
    platform: 'browser',
    plugins: [stubPlugin(variant.mutations ?? [])],
    define: { 'process.env.NODE_ENV': '"production"' },
    loader: { '.svg': 'dataurl', '.png': 'dataurl' },
    logLevel: 'silent',
  });
  const script = out.outputFiles.find((file) => file.path.endsWith('.js'))!.text;
  // The stylesheets the real components import (settings.css, onboarding.css, providers.css).
  const componentCss = out.outputFiles.find((file) => file.path.endsWith('.css'))?.text ?? '';
  let main = readFileSync(resolve(STYLES, 'main.css'), 'utf8');
  for (const edit of variant.css ?? []) {
    if (!main.includes(edit.from))
      throw new Error(`the main.css edit matches nothing: ${edit.from}`);
    main = main.replace(edit.from, edit.to);
  }
  const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
  const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
  const compiler = await compile(main, { base: STYLES, onDependency() {} });
  const css = compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }]));
  return { script, css: `${css}\n${componentCss}` };
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

/** Everything the assertions read, from one frame. */
type Measure = {
  bandBottom: number;
  viewportTop: number;
  viewportBottom: number;
  viewportScrollTop: number;
  rootScrollTop: number;
  rootOverflow: number;
  scrolled: string | null;
  mask: string;
  headroomToken: string;
  rowClearance: number;
  textClearance: number;
  textBottom: number;
  textHit: boolean;
  text: string;
};

/**
 * Read the page in one frame. The first ROW is the first Settings section box (its top is where
 * the column's own padding ends) or the setup page's first line; the first TEXT is the first
 * glyph run a person sees (not an `sr-only` description), and it is "visible" only when the
 * element under its middle is the text's own, not the band or anything else.
 */
const measure = (tab: Page): Promise<Measure> =>
  tab.evaluate(() => {
    const band = document.querySelector('header[data-band]')!;
    const root = document.querySelector<HTMLElement>('.br-scroll-root')!;
    const viewport = root.querySelector<HTMLElement>(':scope > [data-radix-scroll-area-viewport]')!;
    const row = viewport.querySelector('.biorouter-settings-section, [data-first-row]')!;
    const walker = document.createTreeWalker(viewport, NodeFilter.SHOW_TEXT);
    let text: { rect: DOMRect; el: Element } | null = null;
    while (!text && walker.nextNode()) {
      const node = walker.currentNode;
      const el = node.parentElement;
      if (!el || !node.textContent?.trim() || el.closest('.sr-only')) continue;
      const range = document.createRange();
      range.selectNodeContents(node);
      const rect = range.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) text = { rect, el };
    }
    if (!text) throw new Error('no visible text in the scroller');
    const bandBottom = band.getBoundingClientRect().bottom;
    const box = viewport.getBoundingClientRect();
    const { rect } = text;
    const hit = document.elementFromPoint(
      rect.left + Math.min(4, rect.width / 2),
      rect.top + rect.height / 2
    );
    const style = getComputedStyle(viewport);
    return {
      bandBottom,
      viewportTop: box.top,
      viewportBottom: box.bottom,
      viewportScrollTop: viewport.scrollTop,
      rootScrollTop: root.scrollTop,
      rootOverflow: root.scrollHeight - root.clientHeight,
      scrolled: root.getAttribute('data-scrolled'),
      mask: style.maskImage || style.getPropertyValue('-webkit-mask-image') || 'none',
      headroomToken: getComputedStyle(document.documentElement)
        .getPropertyValue('--page-headroom')
        .trim(),
      rowClearance: row.getBoundingClientRect().top - bandBottom,
      textClearance: rect.top - bandBottom,
      textBottom: rect.bottom,
      textHit: Boolean(hit && (hit === text.el || text.el.contains(hit) || hit.contains(text.el))),
      text: (text.el.textContent ?? '').trim().slice(0, 60),
    };
  });

/** Wait until neither the viewport nor the root has moved for 400ms (smooth scrolls included). */
const settle = (tab: Page) =>
  tab.evaluate(
    () =>
      new Promise<void>((done) => {
        const root = document.querySelector<HTMLElement>('.br-scroll-root')!;
        const viewport = root.querySelector<HTMLElement>(
          ':scope > [data-radix-scroll-area-viewport]'
        )!;
        let last = '';
        let since = performance.now();
        const tick = () => {
          const now = `${viewport.scrollTop}|${root.scrollTop}`;
          if (now !== last) {
            last = now;
            since = performance.now();
          }
          if (performance.now() - since > 400) done();
          else requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
      })
  );

async function pointAtScroller(tab: Page) {
  const box = await tab
    .locator('.br-scroll-root > [data-radix-scroll-area-viewport]')
    .first()
    .boundingBox();
  if (!box) throw new Error('the scroller has no box');
  await tab.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
}

/** The wheel, as a mouse sends it: notched steps. */
async function wheel(tab: Page, deltaY: number, steps: number) {
  await pointAtScroller(tab);
  for (let i = 0; i < steps; i++) {
    await tab.mouse.wheel(0, deltaY);
    await tab.waitForTimeout(16);
  }
  await settle(tab);
}

/** A trackpad fling back to the top: many small deltas with no pause between them. */
async function fling(tab: Page) {
  await pointAtScroller(tab);
  for (let i = 0; i < 160; i++) await tab.mouse.wheel(0, -24);
  await settle(tab);
}

/** Wheel up until the scroller reports its top, the way a person keeps turning the wheel. */
async function wheelToTop(tab: Page) {
  for (let round = 0; round < 6; round++) {
    await wheel(tab, -400, 8);
    const top = await tab.evaluate(
      () => document.querySelector('.br-scroll-root > [data-radix-scroll-area-viewport]')!.scrollTop
    );
    if (top === 0) return;
  }
}

/**
 * The owner's acceptance, at the top: the viewport and the root at 0, the fade off, and the
 * first row and its first line fully visible with at least 24px of clear space under the band.
 */
function expectAtTop(m: Measure, label: string) {
  const at = `${label}: ${JSON.stringify(m)}`;
  expect(m.bandBottom, at).toBe(BAND);
  expect(m.viewportTop, at).toBe(BAND);
  expect(m.viewportScrollTop, at).toBe(0);
  expect(m.rootScrollTop, at).toBe(0);
  expect(m.scrolled, at).toBe('false');
  expect(m.mask, at).toBe('none');
  expect(m.rowClearance, at).toBeGreaterThanOrEqual(MIN_HEADROOM);
  expect(m.textClearance, at).toBeGreaterThanOrEqual(MIN_HEADROOM);
  expect(m.textBottom, at).toBeLessThanOrEqual(m.viewportBottom);
  expect(m.textHit, at).toBe(true);
  // The clear space is the shared token, not a per-page number that happens to be large enough.
  expect(m.rowClearance, at).toBeCloseTo(parseFloat(m.headroomToken), 1);
}

describe('the top of Settings and the setup page', () => {
  let browser: Browser | null = null;
  let real: Prepared = { script: '', css: '' };

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the page headroom tests.');
      return;
    }
    real = await prepare();
    if (SHOTS) mkdirSync(SHOTS, { recursive: true });
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(
    prepared: Prepared,
    options: {
      surface: 'settings' | 'setup';
      section?: string;
      size: { width: number; height: number };
    }
  ): Promise<Page> {
    const tab = await browser!.newPage({ viewport: options.size, deviceScaleFactor: 2 });
    await tab.setContent(
      `<!doctype html><html><head><meta charset="utf-8"><style>${prepared.css}
        #shell { height: 100vh; display: flex; flex-direction: column; }</style></head>` +
        `<body style="margin:0;background:var(--background-canvas)"><div id="root"></div></body></html>`
    );
    await tab.evaluate(
      ([surface, section]) => {
        const w = window as unknown as Record<string, unknown>;
        w.__surface = surface;
        w.__section = section;
        w.electron = { platform: 'darwin', on: () => () => {} };
      },
      [options.surface, options.section ?? null] as const
    );
    await tab.addScriptTag({ content: prepared.script });
    await tab.waitForSelector(
      options.surface === 'setup' ? '[data-first-row]' : '.biorouter-settings-section'
    );
    await settle(tab);
    return tab;
  }

  async function shot(tab: Page, name: string) {
    if (SHOTS) await tab.screenshot({ path: join(SHOTS, `${name}.png`) });
  }

  async function check(tab: Page, label: string) {
    const m = await measure(tab);
    await shot(tab, label);
    expectAtTop(m, label);
    return m;
  }

  for (const size of SIZES) {
    const at = `${size.width}x${size.height}`;

    it(`Settings: every tab, wheeled down and back to the top, at ${at}`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(real, { surface: 'settings', size });
      try {
        await check(tab, `settings-${at}-models-open`);
        for (const name of ['models', 'chat', 'app'] as const) {
          if (name !== 'models') {
            await tab.click(`[data-testid="settings-${name}-tab"]`);
            await settle(tab);
            await check(tab, `settings-${at}-${name}-open`);
          }
          await wheel(tab, 300, 4);
          const down = await measure(tab);
          expect(down.viewportScrollTop, `${name} scrolled`).toBeGreaterThan(0);
          expect(down.scrolled, `${name} scrolled`).toBe('true');
          if (name === 'app') await fling(tab);
          else await wheelToTop(tab);
          await check(tab, `settings-${at}-${name}-back-at-top`);
        }
      } finally {
        await tab.close();
      }
    }, 120_000);

    it(`Settings opened at a section, then wheeled back to the top, at ${at}`, async (ctx) => {
      if (!browser) return ctx.skip();
      // `update` is the update notice's key (the About section, near the App tab's end), `project`
      // the last section of Chat, and `privacy` a section near the top, which shortchanged the top
      // by the viewport's 10px scroll padding alone.
      for (const section of ['update', 'project', 'privacy']) {
        const tab = await open(real, { surface: 'settings', section, size });
        try {
          const landed = await measure(tab);
          expect(landed.viewportScrollTop, `${section} landed`).toBeGreaterThan(0);
          expect(landed.rootScrollTop, `${section} landed`).toBe(0);
          await shot(tab, `settings-${at}-${section}-landed`);
          await wheelToTop(tab);
          await check(tab, `settings-${at}-${section}-back-at-top`);
          // Changing tab afterwards starts each tab at its top, with nothing left under the band.
          for (const name of ['models', 'chat', 'app'] as const) {
            await tab.click(`[data-testid="settings-${name}-tab"]`);
            await settle(tab);
            await check(tab, `settings-${at}-${section}-then-${name}`);
          }
        } finally {
          await tab.close();
        }
      }
    }, 180_000);

    it(`the setup page, wheeled down and flung back to the top, at ${at}`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(real, { surface: 'setup', size });
      try {
        await check(tab, `setup-${at}-open`);
        await wheel(tab, 300, 4);
        expect((await measure(tab)).viewportScrollTop).toBeGreaterThan(0);
        await wheelToTop(tab);
        await check(tab, `setup-${at}-wheeled-back`);
        await wheel(tab, 300, 4);
        await fling(tab);
        await check(tab, `setup-${at}-flung-back`);
      } finally {
        await tab.close();
      }
    }, 120_000);
  }

  /**
   * Within a pixel of the top. A rest a fraction short of 0 is the top (`data-scrolled` counts a
   * whole pixel), so the fade is off; and the smallest offset that does turn it on fades only the
   * empty headroom, never the first row. Measured as the engine stores each offset: the Chromium
   * this was written against stored a programmatic 0.5 as 1 at 2x, so the fractional branch is
   * asserted only when the engine keeps a fraction, and the whole-pixel branch always.
   */
  it('fades nothing but the headroom within a pixel of the top', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(real, { surface: 'settings', size: SIZES[0] });
    try {
      for (const target of [0.5, 1, 2]) {
        await wheel(tab, 300, 2);
        const top = await tab.evaluate((value) => {
          const viewport = document.querySelector<HTMLElement>(
            '.br-scroll-root > [data-radix-scroll-area-viewport]'
          )!;
          viewport.scrollTop = value;
          return viewport.scrollTop;
        }, target);
        await settle(tab);
        const m = await measure(tab);
        const fade = await tab.evaluate(() =>
          parseFloat(
            getComputedStyle(document.documentElement).getPropertyValue('--scroll-fade-top')
          )
        );
        const at = JSON.stringify({ target, top, fade, ...m });
        expect(fade, at).toBeGreaterThan(0);
        // An engine that rounds the fraction down has put the scroller at its top, measured above.
        if (top === 0) continue;
        if (top < 1) {
          expect(m.scrolled, at).toBe('false');
          expect(m.mask, at).toBe('none');
        } else {
          expect(m.scrolled, at).toBe('true');
        }
        // The fade's whole height ends above the first row, so the row is drawn at full ink.
        expect(m.rowClearance - (m.viewportTop - m.bandBottom), at).toBeGreaterThan(fade);
        expect(m.rowClearance, at).toBeGreaterThan(MIN_HEADROOM - 3);
        expect(m.textHit, at).toBe(true);
      }
    } finally {
      await tab.close();
    }
  }, 60_000);

  /**
   * THE INSTRUMENT. A harness that cannot show the defect would pass anything, so the defect is put
   * back: the viewport unpositioned (`main.css`) and the root's guard gone (`ui/scroll-area.tsx`).
   * Opening Settings at `update` must then leave the root scrolled and the first row under the
   * band after the wheel reaches the top, exactly as the owner saw it. Each half alone holds.
   */
  const UNPOSITIONED: CssEdit = {
    from: '.br-scroll-viewport {\n  position: relative;\n',
    to: '.br-scroll-viewport {\n',
  };
  const UNGUARDED: Mutation = {
    file: 'ui/scroll-area.tsx',
    from: "root.addEventListener('scroll', pin, { passive: true });",
    to: '',
  };

  async function updateThenTop(prepared: Prepared): Promise<Measure> {
    const tab = await open(prepared, { surface: 'settings', section: 'update', size: SIZES[1] });
    try {
      await wheelToTop(tab);
      return await measure(tab);
    } finally {
      await tab.close();
    }
  }

  it('the instrument: with both halves removed, the top stays under the band', async (ctx) => {
    if (!browser) return ctx.skip();
    const broken = await updateThenTop(
      await prepare({ css: [UNPOSITIONED], mutations: [UNGUARDED] })
    );
    expect(broken.rootOverflow, JSON.stringify(broken)).toBeGreaterThan(0);
    expect(broken.viewportScrollTop, JSON.stringify(broken)).toBe(0);
    expect(broken.rootScrollTop, JSON.stringify(broken)).toBeGreaterThan(0);
    expect(broken.rowClearance, JSON.stringify(broken)).toBeLessThan(MIN_HEADROOM);
    expect(broken.textHit, JSON.stringify(broken)).toBe(false);
  }, 180_000);

  it('the instrument: either half alone keeps the top in view', async (ctx) => {
    if (!browser) return ctx.skip();
    const positionedOnly = await updateThenTop(await prepare({ mutations: [UNGUARDED] }));
    expect(positionedOnly.rootOverflow, JSON.stringify(positionedOnly)).toBe(0);
    expectAtTop(positionedOnly, 'viewport positioned, no guard');
    const guardOnly = await updateThenTop(await prepare({ css: [UNPOSITIONED] }));
    expect(guardOnly.rootOverflow, JSON.stringify(guardOnly)).toBeGreaterThan(0);
    expectAtTop(guardOnly, 'guard, viewport unpositioned');
  }, 180_000);
});
