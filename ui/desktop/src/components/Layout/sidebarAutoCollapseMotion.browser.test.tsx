// @vitest-environment node
/**
 * The sidebar's automatic collapse and restore at the 1120 rung do not animate while the window
 * is being resized (spec principle 9, checklist 12; vision V1-R4-1).
 *
 * jsdom has no layout engine, no CSS transitions and no `resize` dispatch order, and all three
 * are the subject. So the real `AppLayout` (its provider, sidebar, inset, titlebar toggle and
 * width watcher) is bundled with esbuild, the app's own `styles/main.css` is compiled by
 * Tailwind, and Chromium resizes its viewport step by step, the way a window dragged or set to
 * a size by a script does: one resize event per step, the steps further apart than the 180ms
 * the layout holds still for.
 *
 * Three facts decide the outcome, and the cases pin each:
 *
 * 1. Chromium fires a window `resize` listener in REGISTRATION order, capture flag or not. After
 *    a load the width watcher's listener is registered ahead of the provider's marking listener
 *    and `AppLayout`'s own (a child's effect runs before its parent's). React treats `resize` as
 *    a discrete event and commits the watcher's `setOpen` in the microtask right after that
 *    listener returns, so a chat (which reads layout in its commit) resolved styles with the
 *    sidebar collapsed and `<body>` not yet marked, and the gap's width and the panel's
 *    translate began to ease. Only the FIRST crossing after a load did: a change of `open` gives
 *    `setOpen` a new identity and re-runs the watcher's effect, which re-registers its listener
 *    last, so every later crossing found the marker first. That is why a reload, then 1300,
 *    1200, 1100 slid in about half the trials on a chat and a second crossing never did. The
 *    watcher now holds the window still itself, before it moves anything.
 * 2. The hold ends `MOTION.resizeSettleMs` after the last event, so a crossing that lands on a
 *    step later than that finds the class gone. The cases wait 300ms between steps for that.
 * 3. A click on the toggle at a narrow width is NOT a resize, and must still slide.
 *
 * Every claim is proved the same way: the case runs against a build with the one call that
 * guards it removed, and expects the defect, so the instrument is shown to see what it claims to.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

/** The line where the watcher holds the window still before it moves the sidebar (AppLayout.tsx). */
const HOLD_LINE = /^.*@hold-before-move.*$/m;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { AppLayout } from './AppLayout';
import { useSidebar } from '../ui/sidebar';

/**
 * A chat reads layout in its commit (the transcript measures, the column glides, the tab strip
 * reveals the active tab). Any such read resolves styles on the spot, which is what turns a
 * missed hold into a running transition, so the stand-in does one whenever the sidebar changes.
 */
function Chat() {
  const { state } = useSidebar();
  React.useLayoutEffect(() => {
    document.querySelector('[data-slot="sidebar-gap"]')?.getBoundingClientRect();
  }, [state]);
  return <div id="chat">chat</div>;
}

createRoot(document.getElementById('root')!).render(
  <MemoryRouter initialEntries={['/']}>
    <Routes>
      <Route element={<AppLayout />}>
        <Route path="*" element={<Chat />} />
      </Route>
    </Routes>
  </MemoryRouter>
);
`;

/** The heavy children `AppLayout` renders are not under test; each has its own suite. */
const STUBS: Record<string, string> = {
  AppSidebar: 'export default () => <nav data-testid="app-sidebar">sidebar</nav>;',
  DependencySetupModal: 'export default () => null;',
  ExtensionUpdateReporter: 'export default () => null;',
  useNavigation: 'export const useNavigation = () => () => {};',
};

function stubsPlugin(mutate?: (source: string) => string): Plugin {
  return {
    name: 'stubs',
    setup(b) {
      b.onResolve(
        {
          filter: /(AppSidebar|DependencySetupModal|ExtensionUpdateReporter|hooks\/useNavigation)$/,
        },
        (args) => ({
          path: args.path,
          namespace: 'stub',
        })
      );
      b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => {
        const key = Object.keys(STUBS).find((name) => args.path.endsWith(name))!;
        return { contents: STUBS[key], loader: 'tsx', resolveDir: here };
      });
      if (mutate) {
        b.onLoad({ filter: /Layout[\\/]AppLayout\.tsx$/ }, (args) => ({
          contents: mutate(readFileSync(args.path, 'utf8')),
          loader: 'tsx',
          resolveDir: here,
        }));
      }
    },
  };
}

async function bundle(mutate?: (source: string) => string): Promise<string> {
  const out = await build({
    stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
    bundle: true,
    write: false,
    format: 'iife',
    jsx: 'automatic',
    platform: 'browser',
    define: { 'process.env.NODE_ENV': '"production"' },
    loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
    plugins: [stubsPlugin(mutate)],
    logLevel: 'silent',
  });
  return out.outputFiles[0].text;
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

type Run = { property: string; slot: string; resizing: boolean };

describe('the automatic collapse at the 1120 rung stays still while the window moves (V1-R4-1)', () => {
  let browser: Browser | null = null;
  let css = '';
  let script = '';
  let withoutHold = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the sidebar auto-collapse motion tests.');
      return;
    }
    script = await bundle();
    // The mutation must hit: a build that silently matched nothing would "prove" nothing.
    let hit = false;
    withoutHold = await bundle((source) => {
      const next = source.replace(HOLD_LINE, '');
      hit = next !== source;
      return next;
    });
    expect(hit, 'the hold call to remove was found in AppLayout.tsx').toBe(true);
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

  /** The layout at `width`, mounted and settled, with a recorder for every transition that starts. */
  async function open(js: string, width: number): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width, height: 800 } });
    // A real origin, not `setContent` on about:blank: the sidebar writes a cookie when it opens or
    // closes, and a blank document refuses every cookie write.
    await tab.route('http://harness.test/', (route) =>
      route.fulfill({
        contentType: 'text/html',
        body:
          `<!doctype html><html><head><style>${css}</style></head>` +
          `<body style="margin:0"><div id="root"></div></body></html>`,
      })
    );
    await tab.goto('http://harness.test/');
    await tab.evaluate(() => {
      const w = window as unknown as { __runs: Run[] };
      w.__runs = [];
      document.addEventListener(
        'transitionrun',
        (event) => {
          const target = event.target as HTMLElement;
          const slot = target.closest?.('[data-slot]')?.getAttribute('data-slot') ?? '';
          if (!slot.startsWith('sidebar')) return;
          w.__runs.push({
            property: event.propertyName,
            slot,
            resizing: document.body.classList.contains('biorouter-window-resizing'),
          });
        },
        true
      );
    });
    await tab.addScriptTag({ content: js });
    await tab.waitForSelector('#chat');
    await tab.waitForTimeout(500);
    return tab;
  }

  const runs = (tab: Page) =>
    tab.evaluate(() => (window as unknown as { __runs: Run[] }).__runs.slice());
  const clearRuns = (tab: Page) =>
    tab.evaluate(() => {
      (window as unknown as { __runs: Run[] }).__runs.length = 0;
    });
  const sidebarState = (tab: Page) =>
    tab.evaluate(() => document.querySelector('[data-slot="sidebar"]')!.getAttribute('data-state'));
  const holdingStill = (tab: Page) =>
    tab.evaluate(() => document.body.classList.contains('biorouter-window-resizing'));
  const moved = (list: Run[]) =>
    list.filter(
      (run) =>
        (run.slot === 'sidebar-gap' && run.property === 'width') ||
        (run.slot === 'sidebar-container' && ['translate', 'transform'].includes(run.property)) ||
        (run.slot === 'sidebar-inset' && run.property === 'margin-left')
    );

  /** One resize step, then the wait that outlasts the settle, so the next step finds no hold. */
  async function step(tab: Page, width: number) {
    await tab.setViewportSize({ width, height: 800 });
    await tab.waitForTimeout(300);
  }

  it('collapses past the rung in one frame, on a step later than the settle after the last', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(script, 1300);
    try {
      await step(tab, 1200);
      expect(await holdingStill(tab), 'the hold from the 1200 step has ended').toBe(false);
      await clearRuns(tab);
      await tab.setViewportSize({ width: 1100, height: 800 });
      await tab.waitForTimeout(120);
      expect(await sidebarState(tab)).toBe('collapsed');
      expect(await moved(await runs(tab))).toEqual([]);
      await tab.waitForTimeout(400);
      expect(await moved(await runs(tab))).toEqual([]);
    } finally {
      await tab.close();
    }
  });

  it('restores past the rung in one frame, too', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(script, 1300);
    try {
      await step(tab, 1100);
      expect(await sidebarState(tab)).toBe('collapsed');
      await clearRuns(tab);
      await tab.setViewportSize({ width: 1200, height: 800 });
      await tab.waitForTimeout(120);
      expect(await sidebarState(tab)).toBe('expanded');
      expect(await moved(await runs(tab))).toEqual([]);
      await tab.waitForTimeout(400);
      expect(await moved(await runs(tab))).toEqual([]);
    } finally {
      await tab.close();
    }
  });

  it('instrument: without the hold in the watcher the same step slides the gap and the panel', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(withoutHold, 1300);
    try {
      await step(tab, 1200);
      expect(await holdingStill(tab)).toBe(false);
      await clearRuns(tab);
      await tab.setViewportSize({ width: 1100, height: 800 });
      await tab.waitForTimeout(120);
      const slid = moved(await runs(tab));
      expect(slid.map((run) => `${run.slot}:${run.property}`).sort()).toEqual(
        expect.arrayContaining(['sidebar-container:translate', 'sidebar-gap:width'])
      );
    } finally {
      await tab.close();
    }
  });

  it('still slides when a person toggles the overlay at a narrow width', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(script, 1300);
    try {
      await step(tab, 1100);
      expect(await holdingStill(tab)).toBe(false);
      await clearRuns(tab);
      await tab.click('[data-sidebar="trigger"]');
      await tab.waitForTimeout(120);
      expect(await sidebarState(tab)).toBe('expanded');
      const slid = moved(await runs(tab));
      expect(slid.map((run) => `${run.slot}:${run.property}`)).toContain(
        'sidebar-container:translate'
      );
    } finally {
      await tab.close();
    }
  });

  it('opens already collapsed in a window narrower than the rung, without sliding shut', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(script, 1060);
    try {
      expect(await sidebarState(tab)).toBe('collapsed');
      expect(await moved(await runs(tab))).toEqual([]);
    } finally {
      await tab.close();
    }
  });

  it('instrument: without the hold, a window that opens narrower than the rung slides shut', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(withoutHold, 1060);
    try {
      const slid = moved(await runs(tab)).map((run) => `${run.slot}:${run.property}`);
      expect(slid).toEqual(
        expect.arrayContaining(['sidebar-container:translate', 'sidebar-gap:width'])
      );
    } finally {
      await tab.close();
    }
  });
});
