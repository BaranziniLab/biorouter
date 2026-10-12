// @vitest-environment node
/**
 * Built apps and its export dialog, painted by Chromium: the new UI the built-apps work (#392)
 * brought into the redesign, measured where jsdom cannot see.
 *
 * The real `ApplicationsView` is bundled with esbuild and served from a real origin
 * (`http://harness.test/`, route-fulfilled), so the daemon's `GET /apps` and the build-chat check
 * (`GET /sessions/{id}?metadata_only=true`) are answered at the network, the way the view meets
 * them, and `styles/main.css` is compiled by Tailwind with the classes the bundle uses. The main
 * process is a stub on `window.electron`: `exportApp` answers what the case needs.
 *
 * What is pinned here, and why only here:
 * - an unreadable app's row is two lines, the height of every other row (the consequence moved
 *   into the badge's help; a third line of prose made it the one tall row in the list);
 * - the export dialog's body scrolls at the app's minimum window (1048x600) instead of clipping
 *   its last rows, which the shell's `overflow: hidden` used to do without a scrollbar;
 * - the app check's findings sit in a warning Note, the shell body is the one scroller, the Note's
 *   own ceiling folds a long list behind "Show more", and focus lands on the findings;
 * - every size in the view and the dialog is on the five-step scale.
 *
 * Set `VIEWS_A_SHOTS` to a directory to also write a screenshot of each state, light and dark.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const SHOTS = process.env.VIEWS_A_SHOTS;
const ORIGIN = 'http://harness.test';

/** The type scale (spec 2.1): the only sizes a visible run of text may take. */
const SCALE = [12, 13, 14, 17, 24];

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import ApplicationsView from './ApplicationsView';

createRoot(document.getElementById('root')!).render(
  <div style={{ height: '100vh' }}>
    <MemoryRouter>
      <ApplicationsView />
    </MemoryRouter>
  </div>
);
`;

const APP_CREATED = 1750000000;
const iso = (secs: number) => new Date(secs * 1000).toISOString();

/** Reaches knowledge bases four ways, a skill and an extension: the tallest full export. */
const COHORT = {
  id: 'cohort-explorer',
  title: 'Cohort Explorer',
  description: 'Explore a paired cohort.',
  kind: 'agentic',
  session_id: 'session-1',
  created_at: APP_CREATED,
  updated_at: APP_CREATED + 600,
  surface: { actions: [{ name: 'filter' }, { name: 'plot' }], signals: [{ name: 'ready' }] },
  agent: {
    model: { provider: 'anthropic', model: 'claude-sonnet-4-5' },
    knowledge_base: 'ms-cohort',
    skills: ['ggplot-visualization'],
    extensions: ['spokeagent'],
    capabilities: {
      data: { sources: [{ name: 'trials', kind: 'knowledge', ids: ['trials-kb'] }] },
      memory: { kb: 'app-memory', shared_kb: 'lab-memory' },
    },
  },
};
const APPS = [
  COHORT,
  {
    id: 'trial-dashboard',
    title: 'Trial Dashboard',
    kind: 'static',
    session_id: 'session-gone',
    created_at: APP_CREATED,
    updated_at: APP_CREATED + 300,
    agent: { model: { provider: 'openai', model: 'gpt-5.5' } },
  },
  {
    id: 'half-written',
    title: 'half-written',
    kind: 'static',
    broken: 'manifest.json: expected value at line 1 column 1',
  },
];

/**
 * Points at a deleted build chat and names neither model nor knowledge base, so its meta line is
 * its description, long enough to pass the row's measure at any window. The note used to sit
 * after it in one truncated line, past the ellipsis: out of sight at rest.
 */
const RENAL = {
  id: 'renal-dosing',
  title: 'Renal Dosing Assistant',
  description:
    'Adjusts renally cleared drug doses from a creatinine clearance estimate, flags agents to ' +
    'avoid below a threshold, and drafts the dosing note for the chart with its references.',
  kind: 'static',
  session_id: 'session-renal-gone',
  created_at: APP_CREATED,
  updated_at: APP_CREATED + 200,
};

/** A build chat that spans the app's creation, so the row leads to it. */
const SESSION_ROW = {
  id: 'session-1',
  name: 'Cohort build',
  created_at: iso(APP_CREATED - 3600),
  updated_at: iso(APP_CREATED + 3600),
  session_type: 'user',
  working_dir: '/tmp',
  message_count: 4,
};

const FINDINGS = [
  'index.html: #results is not in the page, but the agent prompt renders into @region:results',
  'inline script is blocked by the app page’s content security policy',
  'src/main.ts never calls br.ready(), so the agent never learns the page loaded',
  ...Array.from({ length: 8 }, (_, n) => `src/main.ts: action "step-${n + 1}" is declared twice`),
];

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

describe('Built apps and its export dialog, in Chromium', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Built apps export tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      alias: { '@': resolve(here, '../..') },
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
    css = compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }]));
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /**
   * The view at a window size and mode, the daemon answered at the network and the main process
   * stubbed. `lint` makes the first export answer the app check's findings; `apps` replaces the
   * list `GET /apps` answers.
   */
  async function open(
    options: {
      mode?: 'light' | 'dark';
      size?: { width: number; height: number };
      lint?: boolean;
      apps?: unknown[];
    } = {}
  ): Promise<Page> {
    const {
      mode = 'light',
      size = { width: 1440, height: 900 },
      lint = false,
      apps = APPS,
    } = options;
    const tab = await browser!.newPage({ viewport: size });
    await tab.route(`${ORIGIN}/**`, (route) => {
      const url = new URL(route.request().url());
      const json = (body: unknown, status = 200) =>
        route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
      if (url.pathname === '/') {
        return route.fulfill({
          contentType: 'text/html',
          body:
            `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head>` +
            `<style>${css}</style></head>` +
            `<body style="margin:0;background:var(--background-canvas)"><div id="root"></div>` +
            `</body></html>`,
        });
      }
      if (url.pathname === '/apps') return json(apps);
      if (url.pathname === '/sessions/session-1') return json(SESSION_ROW);
      if (url.pathname.startsWith('/sessions/')) return json({ message: 'not found' }, 404);
      return json({}, 404);
    });
    await tab.goto(`${ORIGIN}/`);
    await tab.evaluate(
      ([findings, answerLint]) => {
        let asked = 0;
        (window as unknown as { electron: unknown }).electron = {
          platform: 'darwin',
          getSecretKey: async () => 'test-secret',
          getUserActionKey: async () => 'test-proof',
          openExternal: async () => undefined,
          openAppInBrowser: async () => undefined,
          exportApp: async () => {
            asked += 1;
            if (answerLint && asked === 1) {
              return {
                status: 'lint',
                message: 'harness ERRORs block export',
                findings,
                retry: 't',
              };
            }
            // Never answers otherwise, so the dialog can be looked at while it works.
            return new Promise(() => {});
          },
        };
      },
      [FINDINGS, lint] as const
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[data-testid="app-row"]');
    // The deleted build chat is checked and said on its row.
    await tab.waitForSelector('[data-testid="app-build-chat-note"]');
    return tab;
  }

  async function shot(tab: Page, name: string) {
    if (!SHOTS) return;
    mkdirSync(SHOTS, { recursive: true });
    await tab.screenshot({ path: resolve(SHOTS, `built-apps-${name}.png`) });
  }

  /** Every distinct font size of a visible text run under `selector`. */
  const textSizes = (tab: Page, selector: string) =>
    tab.evaluate((root) => {
      const sizes = new Set<number>();
      const walker = document.createTreeWalker(document.querySelector(root)!, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        const parent = node.parentElement;
        if (!parent || !node.textContent?.trim()) continue;
        const box = parent.getBoundingClientRect();
        const style = getComputedStyle(parent);
        // Visually hidden help (sr-only) is not a visible run.
        if (box.width <= 1 || box.height <= 1 || style.visibility === 'hidden') continue;
        sizes.add(Number.parseFloat(style.fontSize));
      }
      return [...sizes].sort((a, b) => a - b);
    }, selector);

  /** Opens Cohort Explorer's export dialog from its row's ⋯ menu. */
  async function openExport(tab: Page) {
    await tab.getByRole('button', { name: 'More actions for Cohort Explorer' }).click();
    await tab.getByRole('menuitem', { name: 'Export…' }).click();
    await tab.waitForSelector('[role="dialog"]');
    await tab.waitForTimeout(350);
  }

  it('draws an unreadable app as a two-line row, the height of the others, light and dark', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open({ mode });
      try {
        await tab.waitForTimeout(200);
        const heights = await tab.$$eval('[data-testid="app-row"]', (rows) =>
          rows.map((row) => row.getBoundingClientRect().height)
        );
        expect(heights, mode).toHaveLength(3);
        // Cohort Explorer (model · kb), Trial Dashboard (model · Build chat deleted), half-written.
        expect(
          Math.abs(heights[2] - heights[0]),
          `${mode} broken row vs a row`
        ).toBeLessThanOrEqual(1);
        expect(Math.abs(heights[1] - heights[0]), `${mode} deleted-chat row`).toBeLessThanOrEqual(
          1
        );
        expect(await tab.getByTestId('app-build-chat-note').textContent()).toBe(
          'Build chat deleted'
        );
        for (const size of await textSizes(tab, '.biorouter-list-shell')) {
          expect(SCALE, `${mode} row text ${size}px`).toContain(size);
        }
        await shot(tab, `rows-${mode}`);
      } finally {
        await tab.close();
      }
    }
  }, 180_000);

  it('keeps a deleted build chat’s note on the line at rest when the description is long', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      for (const size of [
        { width: 1048, height: 600 },
        { width: 1440, height: 900 },
      ]) {
        const label = `${mode} ${size.width}x${size.height}`;
        const tab = await open({ mode, size, apps: [COHORT, RENAL] });
        try {
          await tab.waitForTimeout(200);
          // At rest: the pointer is away from every row, and nothing has focus.
          await tab.mouse.move(0, 0);
          const read = await tab.evaluate(() => {
            const rows = [...document.querySelectorAll<HTMLElement>('[data-testid="app-row"]')];
            const row = rows.find((r) => r.textContent?.includes('Renal Dosing Assistant'))!;
            const line = row.querySelector<HTMLElement>('[data-testid="app-row-meta"]')!;
            const note = row.querySelector<HTMLElement>('[data-testid="app-build-chat-note"]')!;
            const description = line.firstElementChild as HTMLElement;
            const box = (el: Element) => el.getBoundingClientRect();
            const n = box(note);
            const hit = document.elementFromPoint(n.left + n.width / 2, n.top + n.height / 2);
            return {
              heights: rows.map((r) => box(r).height),
              line: { left: box(line).left, right: box(line).right },
              note: { left: n.left, right: n.right },
              noteText: note.textContent,
              noteWhole: note.scrollWidth <= Math.ceil(n.width),
              descriptionRight: box(description).right,
              descriptionTruncated: description.scrollWidth > description.clientWidth,
              descriptionEllipsis: getComputedStyle(description).textOverflow,
              hitIsNote: hit === note || note.contains(hit),
              lineText: line.textContent,
            };
          });
          expect(read.noteText, label).toBe('Build chat deleted');
          // Inside the line it belongs to, whole, and the thing painted at its own centre.
          expect(read.note.left, `${label} note starts inside the line`).toBeGreaterThanOrEqual(
            read.line.left
          );
          expect(read.note.right, `${label} note ends inside the line`).toBeLessThanOrEqual(
            read.line.right + 0.5
          );
          expect(read.noteWhole, `${label} the note is not clipped`).toBe(true);
          expect(read.hitIsNote, `${label} the note is what is painted there`).toBe(true);
          // The description is what gives way, with an ellipsis, before the separator.
          expect(read.descriptionTruncated, `${label} the description gives way`).toBe(true);
          expect(read.descriptionEllipsis, label).toBe('ellipsis');
          expect(read.descriptionRight, `${label} no overlap`).toBeLessThanOrEqual(read.note.left);
          // The line still reads as one: description, separator, note.
          expect(read.lineText, label).toBe(`${RENAL.description} · Build chat deleted`);
          // Still a two-line row, the height of the other.
          expect(
            Math.abs(read.heights[1] - read.heights[0]),
            `${label} row height`
          ).toBeLessThanOrEqual(1);
          await shot(tab, `deleted-chat-long-description-${mode}-${size.width}`);
        } finally {
          await tab.close();
        }
      }
    }
  }, 180_000);

  it('scrolls the full export’s body at the minimum window instead of clipping its last rows', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      for (const size of [
        { width: 1048, height: 600 },
        { width: 1440, height: 900 },
      ]) {
        const label = `${mode} ${size.width}x${size.height}`;
        const tab = await open({ mode, size });
        try {
          await openExport(tab);
          await tab.locator('label[for="export-mode-full"]').click();
          await tab.waitForTimeout(150);
          const geometry = await tab.evaluate(() => {
            const dialog = document.querySelector('[role="dialog"]') as HTMLElement;
            const body = [...dialog.querySelectorAll<HTMLElement>('*')].find(
              (el) => getComputedStyle(el).overflowY === 'auto'
            );
            // The footer is the Cancel button's row (Radix puts its close button after it).
            const cancel = [...dialog.querySelectorAll('button')].find(
              (button) => button.textContent === 'Cancel'
            )!;
            const footer = cancel.parentElement as HTMLElement;
            return {
              dialogBottom: dialog.getBoundingClientRect().bottom,
              hasBody: Boolean(body),
              overflows: body ? body.scrollHeight > body.clientHeight + 1 : false,
              footerRule: getComputedStyle(footer).borderTopWidth,
            };
          });
          expect(geometry.dialogBottom, `${label} inside the window`).toBeLessThanOrEqual(
            size.height
          );
          expect(geometry.hasBody, `${label} a scrolling body`).toBe(true);
          expect(geometry.footerRule, `${label} the footer rule marks the scroll edge`).toBe('1px');
          const daemonRow = tab.getByRole('checkbox', { name: 'Include Biorouter (~110 MB)' });
          // Reachable: scrolled into the body's view, and clickable there.
          await daemonRow.scrollIntoViewIfNeeded();
          await tab.locator('label[for="export-bundle-daemon"]').click();
          expect(await daemonRow.isChecked(), `${label} the last row can be reached`).toBe(true);
          if (size.height === 600) {
            expect(geometry.overflows, `${label} the body is taller than its room`).toBe(true);
          }
          for (const px of await textSizes(tab, '[role="dialog"]')) {
            expect(SCALE, `${label} dialog text ${px}px`).toContain(px);
          }
          await tab.evaluate(() => {
            const dialog = document.querySelector('[role="dialog"]')!;
            const body = [...dialog.querySelectorAll<HTMLElement>('*')].find(
              (el) => getComputedStyle(el).overflowY === 'auto'
            );
            body?.scrollTo({ top: 0 });
          });
          await shot(tab, `export-full-${mode}-${size.width}`);
        } finally {
          await tab.close();
        }
      }
    }
  }, 240_000);

  it('shows the app check’s findings in a warning Note the body scrolls, with focus on them', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open({ mode, lint: true });
      try {
        await openExport(tab);
        await tab.getByRole('button', { name: 'Export', exact: true }).click();
        const list = tab.getByRole('list', { name: 'Problems found' });
        await list.waitFor();
        await tab.waitForTimeout(250);
        const read = await tab.evaluate(() => {
          const dialog = document.querySelector('[role="dialog"]') as HTMLElement;
          const ul = dialog.querySelector('ul[aria-label="Problems found"]') as HTMLElement;
          const note = ul.closest('[data-testid="export-lint-findings"]') as HTMLElement;
          const scrollers = [...dialog.querySelectorAll<HTMLElement>('*')].filter((el) =>
            ['auto', 'scroll'].includes(getComputedStyle(el).overflowY)
          );
          return {
            focused: document.activeElement === ul,
            noteGround: getComputedStyle(note).backgroundColor,
            noteBorder: getComputedStyle(note).borderTopWidth,
            listOverflow: getComputedStyle(ul).overflowY,
            scrollers: scrollers.length,
            itemSize: getComputedStyle(ul.querySelector('li')!).fontSize,
            clamped: Boolean(note.querySelector('.biorouter-note-clamp')),
            titleWeight: getComputedStyle(dialog.querySelector('h2')!).fontWeight,
          };
        });
        expect(read.focused, `${mode} focus on the findings`).toBe(true);
        expect(read.noteGround, `${mode} a wash, not a bare list`).not.toBe('rgba(0, 0, 0, 0)');
        expect(read.listOverflow, `${mode} the list does not scroll itself`).toBe('visible');
        expect(read.scrollers, `${mode} the shell body is the one scroller`).toBe(1);
        // The Note's own role, text-supporting (12 / 16): a finding is not a paragraph of prose.
        expect(read.itemSize, `${mode} findings at the Note's supporting size`).toBe('12px');
        expect(Number(read.titleWeight), `${mode} title weight`).toBeLessThanOrEqual(600);
        // Eleven findings pass the Note's eight-line ceiling: folded, with the control to open.
        expect(read.clamped, `${mode} the Note folds a long list`).toBe(true);
        for (const px of await textSizes(tab, '[role="dialog"]')) {
          expect(SCALE, `${mode} dialog text ${px}px`).toContain(px);
        }
        await shot(tab, `export-lint-${mode}`);

        await tab.getByRole('button', { name: 'Show more' }).click();
        await tab.waitForTimeout(150);
        const lastItem = tab.getByText('src/main.ts: action "step-8" is declared twice');
        await lastItem.scrollIntoViewIfNeeded();
        expect(await lastItem.isVisible(), `${mode} every finding can be read`).toBe(true);
        await shot(tab, `export-lint-expanded-${mode}`);

        // Export anyway keeps this face and says it is working.
        await tab.getByRole('button', { name: 'Export anyway' }).click();
        await tab.waitForTimeout(150);
        expect(await list.isVisible()).toBe(true);
        expect(await tab.getByRole('button', { name: 'Exporting…' }).isDisabled()).toBe(true);
        expect(await tab.getByRole('radiogroup', { name: 'Export mode' }).count()).toBe(0);
      } finally {
        await tab.close();
      }
    }
  }, 240_000);
});
