// @vitest-environment node
/**
 * The summary popover keeps its distance from the band line and from the window's bottom edge
 * (vision V2-R4-2), measured in Chromium with the whole stylesheet.
 *
 * Measured in the app at 1300x900 with a long to-do list: `.br-summary-popover` at 1004,44,
 * 280x857, so its top edge sat on the header band's hairline (the button is centred in the 44px
 * band and the primitive's offset is 6px) and its bottom edge at 901, past the window's, with the
 * bottom corners and hairline cut. Beside a preview it sat at top 44 as well.
 *
 * jsdom lays nothing out and positions no popper, so this file bundles the real
 * `SummaryHeaderButton` (and `ChatSummary` inside it) with esbuild, mounts it in a band shaped
 * like BaseChat's header row (`data-preview-area="header"`, `h-chrome`, `border-b`,
 * `items-center`, `pr-4`, the actions group), compiles `styles/main.css` with Tailwind over the
 * classes the bundle uses, adds the summary's own stylesheet, and measures the opened popover. The
 * first case is the instrument: the old wiring (anchored to the button, the primitive's 6px
 * offset, no collision padding) given the same page lands on the band line and past the bottom.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type BrowserServer, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { fontScale } from '../../../vite-plugins/fontScale.mjs';
import { SUMMARY_POPOVER_BAND_GAP, SUMMARY_POPOVER_WINDOW_GAP } from './SummaryHeaderButton';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const SUMMARY_CSS = readFileSync(resolve(here, 'chatSummary.css'), 'utf8');

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Button } from '../ui/button';
import { Popover, PopoverAnchor, PopoverContent } from '../ui/popover';
import { ChatSummary } from '../ChatSummary';
import { SummaryHeaderButton } from './SummaryHeaderButton';

const params = window.PARAMS;
const items = Array.from({ length: params.items }, (_, i) => ({
  id: 'todo-' + i,
  text: 'Step ' + (i + 1) + ': filter the cohort and write the table for the next model run',
  status: i < params.items / 2 ? 'completed' : 'pending',
}));
const summary = {
  toolCalls: '42',
  billedTokens: '211k',
  artifacts: '3',
  codeDelta: { added: 120, removed: 14 },
  todos: { items, loading: false, error: false, refresh: () => {} },
  hasWorkflow: false,
  onWorkflow: () => {},
  onDiagnostics: () => {},
};

/** The wiring before V2-R4-2: anchored to the button, the primitive's offset, no padding. */
function Legacy({ open, setOpen }) {
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverAnchor asChild>
        <span className="inline-flex">
          <Button type="button" variant="ghost" shape="round" aria-label="Chat summary"
            onClick={() => setOpen(!open)}>S</Button>
        </span>
      </PopoverAnchor>
      <PopoverContent role="dialog" aria-label="Chat summary" side="bottom" align="end"
        className="br-summary-popover">
        <ChatSummary {...summary} />
      </PopoverContent>
    </Popover>
  );
}

function Page() {
  const [open, setOpen] = React.useState(false);
  return (
    <div style={{ display: 'flex', height: '100vh' }}>
      <div style={{ flex: '1 1 0%', minWidth: 0, display: 'flex', flexDirection: 'column' }}>
        <div
          data-preview-area="header"
          className="relative z-[var(--z-sticky)] flex h-chrome flex-shrink-0 items-center gap-3 border-b border-border-subtle bg-sidebar pr-4"
        >
          <span className="pl-4 text-label">Build kdps R package</span>
          <div className="ml-auto flex flex-shrink-0 items-center gap-1">
            <Button type="button" variant="ghost" shape="round" aria-label="Open in-app terminal">T</Button>
            {params.legacy ? (
              <Legacy open={open} setOpen={setOpen} />
            ) : (
              <SummaryHeaderButton
                mode="popover"
                railShown={false}
                railRendered={false}
                railId="summary-rail"
                railStill
                onToggleRail={() => {}}
                popoverOpen={open}
                onPopoverOpenChange={setOpen}
                progress={null}
                summary={summary}
              />
            )}
          </div>
        </div>
        <div className="flex-1 bg-background-canvas" />
      </div>
      {params.preview > 0 && (
        <div id="preview" style={{ width: params.preview + 'px', flexShrink: 0 }} className="bg-background-default" />
      )}
    </div>
  );
}

createRoot(document.getElementById('root')).render(<Page />);
`;

/**
 * Every teardown step gets this long and no longer, so the afterAll (disconnect, close, kill and
 * reap) ends in bounded time. Same recipe as `styles/themeSwitch.browser.test.ts`.
 */
const TEARDOWN_STEP_MS = 5_000;

const settledWithin = async (work: Promise<unknown>, ms: number): Promise<boolean> => {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const expired = new Promise<false>((done) => {
    timer = setTimeout(() => done(false), ms);
    (timer as unknown as { unref?: () => void }).unref?.();
  });
  try {
    return (
      (await Promise.race([
        work.then(
          () => true,
          () => true
        ),
        expired,
      ])) !== false
    );
  } finally {
    if (timer) clearTimeout(timer);
  }
};

const launchChromium = async (): Promise<{ server: BrowserServer; browser: Browser } | null> => {
  for (const options of [{}, { channel: 'chromium' }, { channel: 'chrome' }] as const) {
    let server: BrowserServer | null = null;
    try {
      server = await chromium.launchServer(options);
      return { server, browser: await chromium.connect(server.wsEndpoint()) };
    } catch {
      if (server) await settledWithin(server.close(), TEARDOWN_STEP_MS);
      // try the next build
    }
  }
  return null;
};

type Box = { left: number; top: number; right: number; bottom: number; height: number };

interface Placed {
  popover: Box;
  band: Box;
  button: Box;
  /** The popover's border, read from the computed style (a cut hairline would still be 1). */
  border: number;
  /** The summary's body (`.br-summary__body`) holds more than it shows. */
  bodyScrolls: boolean;
}

interface Open {
  width: number;
  height: number;
  items: number;
  legacy?: boolean;
  preview?: number;
  scale?: number;
}

describe('the summary popover keeps off the band line and the window edge (V2-R4-2)', () => {
  let browser: Browser | null = null;
  let server: BrowserServer | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    const launched = await launchChromium();
    browser = launched?.browser ?? null;
    server = launched?.server ?? null;
    if (!browser) {
      console.warn('No Chromium found: skipping the summary popover tests.');
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
    const tailwind = compiler.build(
      new Scanner({}).scanFiles([{ content: script, extension: 'js' }])
    );
    // The summary's own sheet is imported by ChatSummary.tsx in the app; unlayered, as there.
    css = (await postcss([fontScale()]).process(`${tailwind}\n${SUMMARY_CSS}`, { from: undefined }))
      .css;
  }, 180_000);

  afterAll(async () => {
    const connection = browser;
    const launched = server;
    browser = null;
    server = null;
    if (connection) await settledWithin(connection.close(), TEARDOWN_STEP_MS);
    if (!launched) return;
    if (await settledWithin(launched.close(), TEARDOWN_STEP_MS)) return;
    launched.process().kill('SIGKILL');
    await settledWithin(launched.close(), TEARDOWN_STEP_MS);
  }, 30_000);

  async function openPopover(options: Open): Promise<{ tab: Page; placed: Placed }> {
    const tab = await browser!.newPage({
      viewport: { width: options.width, height: options.height },
      deviceScaleFactor: options.scale ?? 1,
    });
    tab.setDefaultTimeout(8000);
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.evaluate(
      (params) => {
        (window as unknown as { PARAMS: unknown }).PARAMS = params;
      },
      {
        items: options.items,
        legacy: options.legacy === true,
        preview: options.preview ?? 0,
      }
    );
    await tab.addScriptTag({ content: script });
    await tab.click('[data-preview-area="header"] button[aria-label="Chat summary"]');
    await tab.waitForSelector('.br-summary-popover');
    // The open motion and any repositioning settle before anything is read. Only the clock's
    // animations: the body's scroll-edge fades run on a scroll timeline and never finish.
    await tab.evaluate(async () => {
      const popover = document.querySelector('.br-summary-popover')!;
      for (let pass = 0; pass < 2; pass++) {
        await Promise.all(
          popover
            .getAnimations({ subtree: true })
            .filter(
              (a) =>
                a.timeline === document.timeline && a.effect?.getTiming().iterations !== Infinity
            )
            .map((a) => a.finished.catch(() => undefined))
        );
        await new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
      }
    });
    const placed = await tab.evaluate(() => {
      const box = (el: Element) => {
        const r = el.getBoundingClientRect();
        return { left: r.left, top: r.top, right: r.right, bottom: r.bottom, height: r.height };
      };
      const popover = document.querySelector('.br-summary-popover')!;
      const band = document.querySelector('[data-preview-area="header"]')!;
      const button = band.querySelector('button[aria-label="Chat summary"]')!;
      const body = popover.querySelector<HTMLElement>('.br-summary__body')!;
      return {
        popover: box(popover),
        band: box(band),
        button: box(button),
        border: parseFloat(getComputedStyle(popover).borderBottomWidth),
        bodyScrolls: body.scrollHeight > body.clientHeight + 1,
      };
    });
    return { tab, placed };
  }

  it('the old wiring, on the same page, lands on the band line and past the window (the instrument)', async (ctx) => {
    if (!browser) return ctx.skip();
    const { tab, placed } = await openPopover({
      width: 1300,
      height: 900,
      items: 24,
      legacy: true,
    });
    try {
      expect(placed.band.bottom).toBe(44);
      // Flush on the band's hairline, and cut by the window's bottom edge.
      expect(placed.popover.top).toBeLessThanOrEqual(placed.band.bottom);
      expect(placed.popover.bottom).toBeGreaterThan(900 - SUMMARY_POPOVER_WINDOW_GAP);
    } finally {
      await tab.close();
    }
  });

  it('hangs 4px below the band line and stops 8px above the window bottom, at 1300x900', async (ctx) => {
    if (!browser) return ctx.skip();
    const { tab, placed } = await openPopover({ width: 1300, height: 900, items: 24 });
    try {
      expect(placed.band.bottom).toBe(44);
      expect(placed.popover.top).toBe(placed.band.bottom + SUMMARY_POPOVER_BAND_GAP);
      expect(placed.popover.bottom).toBe(900 - SUMMARY_POPOVER_WINDOW_GAP);
      // Its right edge on the button's, as before.
      expect(placed.popover.right).toBe(placed.button.right);
      expect(placed.border).toBe(1);
      // The list scrolls inside the popover rather than running off the window.
      expect(placed.bodyScrolls).toBe(true);
    } finally {
      await tab.close();
    }
  });

  it('keeps the same gaps beside a side preview', async (ctx) => {
    if (!browser) return ctx.skip();
    const { tab, placed } = await openPopover({
      width: 1440,
      height: 900,
      items: 24,
      preview: 520,
    });
    try {
      expect(placed.button.right).toBeLessThan(1440 - 520);
      expect(placed.popover.top).toBe(placed.band.bottom + SUMMARY_POPOVER_BAND_GAP);
      expect(placed.popover.bottom).toBe(900 - SUMMARY_POPOVER_WINDOW_GAP);
      expect(placed.popover.right).toBe(placed.button.right);
    } finally {
      await tab.close();
    }
  });

  it('holds the gaps at a device pixel ratio of 2 and in a short window', async (ctx) => {
    if (!browser) return ctx.skip();
    const retina = await openPopover({ width: 1300, height: 900, items: 24, scale: 2 });
    try {
      expect(retina.placed.popover.top).toBe(48);
      expect(retina.placed.popover.bottom).toBe(892);
    } finally {
      await retina.tab.close();
    }
    const short = await openPopover({ width: 1300, height: 600, items: 24 });
    try {
      expect(short.placed.popover.top).toBe(48);
      expect(short.placed.popover.bottom).toBe(600 - SUMMARY_POPOVER_WINDOW_GAP);
    } finally {
      await short.tab.close();
    }
  });

  it('a short list keeps its natural height under the band line', async (ctx) => {
    if (!browser) return ctx.skip();
    const { tab, placed } = await openPopover({ width: 1300, height: 900, items: 3 });
    try {
      expect(placed.popover.top).toBe(48);
      expect(placed.popover.bottom).toBeLessThan(900 - SUMMARY_POPOVER_WINDOW_GAP);
      expect(placed.bodyScrolls).toBe(false);
    } finally {
      await tab.close();
    }
  });
});
