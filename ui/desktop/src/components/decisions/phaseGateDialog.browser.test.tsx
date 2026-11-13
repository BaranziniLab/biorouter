// @vitest-environment node
/**
 * The phase gate dialog in Chromium, with the whole stylesheet (C-7, C-8).
 *
 * C-8: the open, not-passed gate's focus ring sat on "Pass this phase", the first button,
 * which overrides a gate, so one Enter passed the phase. The dialog now rests EVERY open on
 * its own box: after a pointer open or a key open, Enter does nothing and no button wears a
 * ring. jsdom has no `:focus-visible` and lays nothing out, so the real `PhaseGateDialog` is
 * bundled with esbuild (the `ui/dialogPointerOpen.browser.test.tsx` method) and driven with
 * Playwright's mouse and keyboard; its requests go through the generated client's own `fetch`
 * hook, recorded, never to a network.
 *
 * C-7: the footer is ModalShell's wrapping row. The `⋯` holds the start of the first line and
 * Skip and Pass stay one group: at a narrow window they move to the next line together,
 * right-aligned, both inside the dialog box.
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

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const SHEETS = ['decisionsPanel.css', 'probabilityBar.css']
  .map((name) => readFileSync(resolve(here, name), 'utf8'))
  .join('\n');

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { client } from '../../api/client.gen';
import { parseSessionSubagents } from '../../utils/sessionSubagents';
import { wirePhase, wireSubagents } from '../../utils/sessionSubagents.fixture';
import { PhaseGateDialog } from './PhaseGateDialog';

window.REQUESTS = [];
client.setConfig({
  baseUrl: 'http://gates.test',
  fetch: async (request) => {
    window.REQUESTS.push(request.method + ' ' + new URL(request.url).pathname);
    return new Response(JSON.stringify({ message: 'ok', applied: true, plan: null }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  },
});

const TITLE = window.LONG_TITLE
  ? 'Probe the write path of the experiment tracker against every staging database replica'
  : 'Probe write test';
const plan = parseSessionSubagents(
  wireSubagents([], {
    title: 'Plan',
    goal: null,
    revision: 1,
    phases: [
      wirePhase({
        id: 'probe',
        title: TITLE,
        status: 'in_progress',
        gate: {
          threshold: 0.9,
          questionCount: 1,
          attempts: 1,
          maxAttempts: 2,
          lastPassed: false,
          lastMinP: 0.03,
          blocked: false,
          owner: 'agent',
          questions: { mars: 'Did the subagent report launching a rocket to Mars?' },
          holds: ['review'],
        },
      }),
      wirePhase({ id: 'review', title: 'Review the probe', heldBy: 'probe' }),
    ],
  })
).plan;

function App() {
  const [open, setOpen] = React.useState(false);
  return (
    <div>
      <button id="open-gate" type="button" onClick={() => setOpen(true)}>Probe gate</button>
      <PhaseGateDialog
        open={open}
        onOpenChange={setOpen}
        sessionId="s1"
        phase={plan.phases[0]}
        phases={plan.phases}
        models={[]}
        onOpenHistory={() => {}}
      />
    </div>
  );
}
createRoot(document.getElementById('root')).render(<App />);
`;

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
    }
  }
  return null;
};

describe('the phase gate dialog, in a real browser', () => {
  let browser: Browser | null = null;
  let server: BrowserServer | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    const launched = await launchChromium();
    browser = launched?.browser ?? null;
    server = launched?.server ?? null;
    if (!browser) {
      console.warn('No Chromium found: skipping the phase gate dialog tests.');
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
    css = (await postcss([fontScale()]).process(`${tailwind}\n${SHEETS}`, { from: undefined })).css;
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

  async function openPage(viewport = { width: 1200, height: 900 }, longTitle = false) {
    const tab = await browser!.newPage({ viewport });
    tab.setDefaultTimeout(10_000);
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div>` +
        `<script>window.LONG_TITLE = ${longTitle};</script></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#open-gate');
    return tab;
  }

  /** Where focus is, and whether any button in the dialog draws a keyboard ring. */
  const focusState = (tab: Page) =>
    tab.evaluate(() => {
      const dialog = document.querySelector('[role="dialog"]');
      const active = document.activeElement;
      return {
        onBox: active === dialog,
        ringed: Array.from(dialog?.querySelectorAll('button') ?? []).some((button) =>
          button.matches(':focus-visible')
        ),
        requests: (window as unknown as { REQUESTS: string[] }).REQUESTS.length,
      };
    });

  it('rests a pointer open on the box: Enter passes nothing and no button is ringed', async () => {
    if (!browser) return;
    const tab = await openPage();
    await tab.click('#open-gate');
    await tab.waitForSelector('[data-testid="phase-gate-pass"]');
    expect(await focusState(tab)).toEqual({ onBox: true, ringed: false, requests: 0 });
    await tab.keyboard.press('Enter');
    await tab.waitForTimeout(150);
    const after = await focusState(tab);
    await tab.close();
    expect(after).toEqual({ onBox: true, ringed: false, requests: 0 });
  }, 60_000);

  it('rests a key open on the box too', async () => {
    if (!browser) return;
    const tab = await openPage();
    await tab.focus('#open-gate');
    await tab.keyboard.press('Enter');
    await tab.waitForSelector('[data-testid="phase-gate-pass"]');
    expect(await focusState(tab)).toEqual({ onBox: true, ringed: false, requests: 0 });
    await tab.keyboard.press('Enter');
    await tab.waitForTimeout(150);
    const after = await focusState(tab);
    await tab.close();
    expect(after).toEqual({ onBox: true, ringed: false, requests: 0 });
  }, 60_000);

  for (const width of [1200, 360]) {
    it(`keeps the footer's ⋯ first and Skip and Pass together, inside the box, at ${width}px`, async () => {
      if (!browser) return;
      const tab = await openPage({ width, height: 900 }, true);
      await tab.click('#open-gate');
      await tab.waitForSelector('[data-testid="phase-gate-pass"]');
      // A picture for a person reviewing the layout: `DECISIONS_SCREENSHOT_DIR=/tmp/x`.
      const shots = process.env.DECISIONS_SCREENSHOT_DIR;
      if (shots) {
        await tab.waitForTimeout(400);
        await tab.screenshot({ path: `${shots}/phase-gate-${width}.png` });
      }
      const box = await tab.evaluate(() => {
        const rect = (selector: string) =>
          document.querySelector(selector)!.getBoundingClientRect().toJSON() as DOMRect;
        return {
          dialog: rect('[role="dialog"]'),
          more: rect('[data-testid="phase-gate-more"]'),
          skip: rect('[data-testid="phase-gate-skip"]'),
          pass: rect('[data-testid="phase-gate-pass"]'),
        };
      });
      await tab.close();
      expect(box.dialog.width).toBeLessThanOrEqual(480);
      // The ⋯ is the first thing in the footer, on its start edge.
      expect(box.more.left).toBeLessThan(box.skip.left);
      expect(box.more.top).toBeLessThanOrEqual(box.skip.top);
      // Skip and Pass share a line, Pass last, flush to the end.
      expect(box.skip.top).toBe(box.pass.top);
      expect(box.skip.right).toBeLessThanOrEqual(box.pass.left);
      for (const button of [box.more, box.skip, box.pass]) {
        expect(button.left).toBeGreaterThanOrEqual(box.dialog.left);
        expect(button.right).toBeLessThanOrEqual(box.dialog.right + 0.5);
        expect(button.bottom).toBeLessThanOrEqual(box.dialog.bottom + 0.5);
      }
      expect(box.dialog.right - box.pass.right).toBeLessThanOrEqual(17);
    }, 60_000);
  }
});
