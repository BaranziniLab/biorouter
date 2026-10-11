// @vitest-environment node
/**
 * Keyboard focus survives a toggle, in a real browser (vision round 2, V3-R2-1).
 *
 * jsdom cannot see this defect: it never moves focus off a button that turns `disabled`, while
 * Chromium hands focus to `<body>` the moment it does. The Scheduler's Pause, Resume and Stop (on
 * a list row and in the detail band), the list's Refresh and the Built apps row's Launch were all
 * `disabled` while their request ran, so a keyboard user who pressed Enter on one of them lost
 * their place and had to Tab in from the top. Measured in the running app: `document.activeElement`
 * was BODY after Enter on Resume and after Enter on Pause, on the row and in the band.
 *
 * The REAL `SchedulesView`, `ScheduleDetailView` and `ApplicationItem` are bundled with esbuild
 * and rendered by React in Chromium. Only their data edges are stubbed (the Scheduler's API
 * module, the toast module, the run-history view and the create dialog), with a request that
 * takes 400ms so there is a window to sample. The case that matters is read twice: once in the
 * state the person is in WHILE the request is out, and once after it lands, when Pause has
 * become Resume in the same element.
 *
 * "The instrument" proves each case can fail: the same bundle with the new code swapped back for
 * the old `disabled` attribute, which must leave focus on BODY. A check that cannot fail is not
 * a check (and a mutation that matches nothing throws, so a refactor cannot turn it into a
 * no-op).
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');

/** The `@ws WS-VIEWS-A begin: busy action` block, the only stylesheet these cases need. */
function busyActionRule(): string {
  const match = MAIN_CSS.match(
    /\/\* @ws WS-VIEWS-A begin: busy action \*\/([\s\S]*?)\/\* @ws WS-VIEWS-A end: busy action \*\//
  );
  if (!match) throw new Error('main.css has no `@ws WS-VIEWS-A begin: busy action` block');
  return match[1];
}

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import SchedulesView from './SchedulesView';
import ScheduleDetailView from './ScheduleDetailView';
import { ApplicationItem } from '../applications/ApplicationsView';

const view = (window as unknown as { __view: string }).__view;

/** The launch flow of ApplicationsView, in miniature: a press while one is out starts nothing. */
function LaunchRow() {
  const [launching, setLaunching] = React.useState(false);
  const launch = async () => {
    if (launching) return;
    setLaunching(true);
    (window as unknown as { __sched: { calls: unknown[][] } }).__sched.calls.push(['launch']);
    await new Promise((resolve) => setTimeout(resolve, 400));
    setLaunching(false);
  };
  return (
    <ApplicationItem
      app={{ id: 'cohort-explorer', title: 'Cohort Explorer', kind: 'agentic' } as never}
      isLaunching={launching}
      onLaunch={launch}
      onOpenConversation={() => {}}
      onExport={() => {}}
      onDelete={() => {}}
    />
  );
}

createRoot(document.getElementById('root')!).render(
  <MemoryRouter>
    {view === 'detail' ? (
      <ScheduleDetailView scheduleId="nightly-cohort" onNavigateBack={() => {}} />
    ) : view === 'apps' ? (
      <LaunchRow />
    ) : (
      <SchedulesView />
    )}
  </MemoryRouter>
);
`;

/** Every module the views reach for data, replaced by a stub that answers after `delay` ms. */
const STUBS: Array<[RegExp, string]> = [
  [
    /^\.\.\/\.\.\/schedule$/,
    `
    const st = () => (window as any).__sched;
    const wait = () => new Promise((r) => setTimeout(r, st().delay));
    const log = (...a: unknown[]) => st().calls.push(a);
    const find = (id: string) => st().jobs.find((j: any) => j.id === id);
    export const listSchedules = async () => { log('list'); await wait(); return st().jobs.map((j: any) => ({ ...j })); };
    export const getScheduleSessions = async () => [];
    export const pauseSchedule = async (id: string) => { log('pause', id); await wait(); find(id).paused = true; };
    export const unpauseSchedule = async (id: string) => { log('unpause', id); await wait(); find(id).paused = false; };
    export const killRunningJob = async (id: string) => { log('kill', id); await wait(); find(id).currently_running = false; return { message: 'Killed' }; };
    export const runScheduleNow = async (id: string) => { log('run', id); await wait(); return 'run-session'; };
    export const inspectRunningJob = async () => ({});
    export const deleteSchedule = async () => {};
    export const updateSchedule = async () => {};
    export const createSchedule = async () => {};
  `,
  ],
  [
    /^\.\.\/sessions\/SessionHistoryView$/,
    `export default function SessionHistoryView() { return null; }`,
  ],
  [/^\.\/ScheduleModal$/, `export const ScheduleModal = () => null;`],
  [/^\.\.\/privacy\/useSessionListTiers$/, `export const useSessionListTiers = () => ({});`],
  [/^\.\.\/\.\.\/api$/, `export const getSession = async () => ({ data: null });`],
  [
    /^\.\.\/\.\.\/toasts$/,
    `export const toastError = () => {}; export const toastSuccess = () => {};`,
  ],
  [/^\.\.\/\.\.\/utils\/userAction$/, `export const userActionHeaders = async () => ({});`],
];

/**
 * The new code swapped back for the old `disabled` attribute, file by file. Each entry must
 * match, or the build throws: a mutation that finds nothing would pass every case for the wrong
 * reason.
 */
const OLD_DISABLED: Array<[RegExp, Array<[string, string]>]> = [
  [
    /schedule\/SchedulesView\.tsx$/,
    [
      ['className={busyClass}', 'disabled={actionInProgress}'],
      ['busy={isRefreshing || isLoading}', 'disabled={isRefreshing || isLoading}'],
    ],
  ],
  [
    /schedule\/ScheduleDetailView\.tsx$/,
    [['busy={isActionLoading}', 'disabled={isActionLoading}']],
  ],
  [
    /applications\/ApplicationsView\.tsx$/,
    [["className={isLaunching ? 'br-action-busy' : undefined}", 'disabled={isLaunching}']],
  ],
];

async function bundle(restoreDisabled: boolean): Promise<string> {
  const stubs: Plugin = {
    name: 'stubs',
    setup(b) {
      for (const [filter] of STUBS) {
        b.onResolve({ filter }, (args) => ({ path: args.path, namespace: 'stub' }));
      }
      b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => {
        const hit = STUBS.find(([filter]) => filter.test(args.path));
        return { contents: hit![1], loader: 'ts', resolveDir: here };
      });
      if (restoreDisabled) {
        b.onLoad(
          { filter: /\/(SchedulesView|ScheduleDetailView|ApplicationsView)\.tsx$/ },
          (args) => {
            let source = readFileSync(args.path, 'utf8');
            for (const [file, swaps] of OLD_DISABLED) {
              if (!file.test(args.path)) continue;
              for (const [from, to] of swaps) {
                if (!source.includes(from))
                  throw new Error(`mutation target missing in ${args.path}: ${from}`);
                source = source.split(from).join(to);
              }
            }
            return { contents: source, loader: 'tsx', resolveDir: dirname(args.path) };
          }
        );
      }
    },
  };
  const out = await build({
    stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
    bundle: true,
    write: false,
    format: 'iife',
    jsx: 'automatic',
    platform: 'browser',
    define: { 'process.env.NODE_ENV': '"production"' },
    loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
    logLevel: 'silent',
    plugins: [stubs],
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

type Sched = {
  jobs: Array<Record<string, unknown>>;
  delay: number;
  calls: unknown[][];
};
type Probe = { tag: string; label: string | null; marked: boolean };

const JOBS: Sched['jobs'] = [
  { id: 'nightly-cohort', source: '/x/a.yaml', cron: '0 0 1 * * *', paused: false },
  { id: 'weekly-lit', source: '/x/b.yaml', cron: '0 0 9 * * 1', currently_running: true },
];

describe('keyboard focus survives a toggle (V3-R2-1)', () => {
  let browser: Browser | null = null;
  let fixed = '';
  let old = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found (Playwright’s or Google Chrome): skipping the focus cases.');
      return;
    }
    [fixed, old] = await Promise.all([bundle(false), bundle(true)]);
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(
    view: 'list' | 'detail' | 'apps',
    script: string,
    jobs = JOBS
  ): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1100, height: 700 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${busyActionRule()}</style></head><body><div id="root"></div></body></html>`
    );
    // After setContent, which opens a fresh document: an init script would have run before it.
    await tab.evaluate(
      ({ view, jobs }) => {
        const w = window as unknown as { __view: string; __sched: Sched };
        w.__view = view;
        w.__sched = { jobs: JSON.parse(JSON.stringify(jobs)), delay: 400, calls: [] };
      },
      { view, jobs }
    );
    await tab.addScriptTag({ content: script });
    return tab;
  }

  const probe = (tab: Page): Promise<Probe> =>
    tab.evaluate(() => {
      const el = document.activeElement as (HTMLElement & { __probe?: boolean }) | null;
      return {
        tag: el?.tagName ?? 'none',
        label: el?.getAttribute('aria-label') ?? null,
        marked: Boolean(el?.__probe),
      };
    });

  /** Focus the control, mark its DOM node, press Enter, and wait until its request is out. */
  async function pressEnterOn(tab: Page, selector: string, call: string) {
    const control = tab.locator(selector).first();
    await control.waitFor();
    await control.focus();
    await control.evaluate((node) => {
      (node as HTMLElement & { __probe?: boolean }).__probe = true;
    });
    const before = await tab.evaluate(
      (name) =>
        (window as unknown as { __sched: Sched }).__sched.calls.filter((c) => c[0] === name).length,
      call
    );
    await tab.keyboard.press('Enter');
    await tab.waitForFunction(
      ([name, count]) =>
        (window as unknown as { __sched: Sched }).__sched.calls.filter((c) => c[0] === name)
          .length > (count as number),
      [call, before] as const
    );
    // Blink drops the focus of a control that stopped being focusable in a task of its own, not
    // inside the render that disabled it, so a sample taken in the same turn can still see the
    // button focused (measured: the list row fell to BODY at once, the band and Launch did not).
    // The request takes 400ms, so this is still well inside it.
    await tab.waitForTimeout(120);
  }

  /** The request landed: nothing is busy any more. */
  const settled = (tab: Page) =>
    tab.waitForFunction(() => !document.querySelector('[aria-busy="true"], .br-action-busy'));

  const stillTheSameFocusedControl = (p: Probe, label: RegExp) => {
    expect(p.tag, 'focus must not fall to the page').toBe('BUTTON');
    expect(p.marked, 'the focused element must be the one that was pressed').toBe(true);
    expect(p.label).toMatch(label);
  };

  it('keeps focus on a list row from Pause through Resume and back', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('list', fixed);
    try {
      await pressEnterOn(tab, 'button[aria-label^="Pause "]', 'pause');
      stillTheSameFocusedControl(await probe(tab), /^Pause /);

      await tab.waitForSelector('button[aria-label^="Resume "]');
      await settled(tab);
      stillTheSameFocusedControl(await probe(tab), /^Resume /);

      await tab.keyboard.press('Enter');
      await tab.waitForFunction(() =>
        (window as unknown as { __sched: Sched }).__sched.calls.some((c) => c[0] === 'unpause')
      );
      stillTheSameFocusedControl(await probe(tab), /^Resume /);
      await tab.waitForSelector('button[aria-label^="Pause "]');
      await settled(tab);
      stillTheSameFocusedControl(await probe(tab), /^Pause /);
    } finally {
      await tab.close();
    }
  });

  it('keeps focus on a running list row’s Stop, which becomes Pause', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('list', fixed);
    try {
      await pressEnterOn(tab, 'button[aria-label^="Stop "]', 'kill');
      stillTheSameFocusedControl(await probe(tab), /^Stop /);
      await settled(tab);
      // The row's job is no longer running, so the control the person pressed is now its Pause.
      stillTheSameFocusedControl(await probe(tab), /^Pause weekly-lit/);
    } finally {
      await tab.close();
    }
  });

  it('keeps focus on Refresh in the list band while a refresh is out', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('list', fixed);
    try {
      await tab.waitForSelector('button[aria-label^="Pause "]');
      await settled(tab);
      await pressEnterOn(tab, 'button[aria-label="Refresh schedules"]', 'list');
      stillTheSameFocusedControl(await probe(tab), /^Refresh schedules$/);
      await settled(tab);
      stillTheSameFocusedControl(await probe(tab), /^Refresh schedules$/);
    } finally {
      await tab.close();
    }
  });

  it('keeps focus on the detail band’s one control from Pause through Resume', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('detail', fixed);
    try {
      await pressEnterOn(tab, 'button[aria-label="Pause"]', 'pause');
      stillTheSameFocusedControl(await probe(tab), /^Pause$/);

      await tab.waitForSelector('button[aria-label="Resume"]');
      await settled(tab);
      stillTheSameFocusedControl(await probe(tab), /^Resume$/);

      await tab.keyboard.press('Enter');
      await tab.waitForFunction(() =>
        (window as unknown as { __sched: Sched }).__sched.calls.some((c) => c[0] === 'unpause')
      );
      stillTheSameFocusedControl(await probe(tab), /^Resume$/);
      await tab.waitForSelector('button[aria-label="Pause"]');
      await settled(tab);
      stillTheSameFocusedControl(await probe(tab), /^Pause$/);
    } finally {
      await tab.close();
    }
  });

  it('keeps focus on the detail band’s Stop, which becomes Pause', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('detail', fixed, [{ ...JOBS[0], currently_running: true }]);
    try {
      await pressEnterOn(tab, 'button[aria-label="Stop run"]', 'kill');
      stillTheSameFocusedControl(await probe(tab), /^Stop run$/);
      await tab.waitForSelector('button[aria-label="Pause"]');
      await settled(tab);
      stillTheSameFocusedControl(await probe(tab), /^Pause$/);
    } finally {
      await tab.close();
    }
  });

  it('keeps focus on a Built apps row’s Launch while the launch is out', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('apps', fixed);
    try {
      await pressEnterOn(tab, 'button[aria-label^="Launch "]', 'launch');
      stillTheSameFocusedControl(await probe(tab), /^Launch /);
      await settled(tab);
      stillTheSameFocusedControl(await probe(tab), /^Launch /);
    } finally {
      await tab.close();
    }
  });

  it('dims the working action and restores it, without disabling it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('detail', fixed);
    try {
      const opacity = () =>
        tab.evaluate(() => {
          const el = document.querySelector(
            'button[aria-label="Pause"], button[aria-label="Resume"]'
          )!;
          return {
            opacity: getComputedStyle(el).opacity,
            disabled: (el as HTMLButtonElement).disabled,
          };
        });
      await tab.waitForSelector('button[aria-label="Pause"]');
      expect((await opacity()).opacity).toBe('1');
      await pressEnterOn(tab, 'button[aria-label="Pause"]', 'pause');
      expect(await opacity()).toEqual({ opacity: '0.5', disabled: false });
      await settled(tab);
      expect(await opacity()).toEqual({ opacity: '1', disabled: false });
    } finally {
      await tab.close();
    }
  });

  /**
   * The instrument. The same bundle with the old `disabled` attribute put back must drop focus on
   * BODY in every one of the surfaces above, or the cases above prove nothing.
   */
  describe('the instrument: with the old `disabled` attribute focus falls to the page', () => {
    const fallsToBody = async (
      view: 'list' | 'detail' | 'apps',
      selector: string,
      call: string,
      jobs = JOBS
    ) => {
      const tab = await open(view, old, jobs);
      try {
        await pressEnterOn(tab, selector, call);
        const during = await probe(tab);
        expect(during.tag, 'a focused button that turns disabled must lose focus').toBe('BODY');
      } finally {
        await tab.close();
      }
    };

    it('on a list row', async (ctx) => {
      if (!browser) return ctx.skip();
      await fallsToBody('list', 'button[aria-label^="Pause "]', 'pause');
    });

    it('on Refresh in the list band', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open('list', old);
      try {
        await tab.waitForSelector('button[aria-label^="Pause "]');
        await settled(tab);
        await pressEnterOn(tab, 'button[aria-label="Refresh schedules"]', 'list');
        expect((await probe(tab)).tag).toBe('BODY');
      } finally {
        await tab.close();
      }
    });

    it('in the detail band', async (ctx) => {
      if (!browser) return ctx.skip();
      await fallsToBody('detail', 'button[aria-label="Pause"]', 'pause');
    });

    it('on a Built apps row’s Launch', async (ctx) => {
      if (!browser) return ctx.skip();
      await fallsToBody('apps', 'button[aria-label^="Launch "]', 'launch');
    });
  });
});
