// @vitest-environment node
/**
 * The Monitors dialog in a real layout engine (monitors spec 1.5, 1.4, 1.1; F-12, F-17).
 *
 * jsdom lays nothing out and react-select draws no menu there, so what this file
 * holds can only be measured in Chromium, with `styles/main.css` compiled by
 * Tailwind and the dialog's own stylesheets: the six-model picker of a private chat
 * fits its menu without scrolling, the segmented tracks hug their segments, the
 * editor takes the place of the row it edits, and the header's × does not move
 * between a row toggle's click and its answer (the dialog's `purpose` is constant
 * through a write, so the header never relocks).
 *
 * The daemon is a stub `fetch` in the page: a read fails (the dialog then uses the
 * settings it was given, its documented fallback) and a write answers its own body
 * after 300 ms, so a toggle is in flight long enough to be measured mid-write.
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
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { MonitorsDialog } from './MonitorsDialog';
import { client } from '../../api/client.gen';
import { parseDecisionModelEntries, parseDecisionSettings } from '../../utils/sessionDecisions';

// The generated client builds a Request from its base URL; this page has none of its own.
client.setConfig({ baseUrl: 'http://daemon.test' });

const realFetch = window.fetch.bind(window);
window.fetch = async (input, init) => {
  const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
  if (!url.includes('decision-settings')) return realFetch(input, init);
  const method = init?.method ?? (input instanceof Request ? input.method : 'GET');
  if (method === 'PUT') {
    const body = init?.body !== undefined ? String(init.body) : await input.text();
    await new Promise((done) => setTimeout(done, 300));
    return new Response(body, { status: 200, headers: { 'content-type': 'application/json' } });
  }
  return new Response('unavailable', { status: 503 });
};

const local = (model, contextTokens) => ({
  provider: 'ollama_decision', model, displayName: model, contextTokens,
  kinds: ['binary', 'choice', 'score'], tier: 'private', adapter: 'system_one', local: true,
});
const hosted = (model, displayName) => ({
  provider: 'baseten_decision', model, displayName, contextTokens: 16000,
  kinds: ['binary', 'choice', 'score'], tier: 'public', adapter: 'system_one', local: false,
  unavailableReason: 'This chat is private, so a public model may not read it (decision_models lists them).',
});
const models = parseDecisionModelEntries([
  local('tev1:0.8b', 2048),
  local('clef-flash:latest', 16384),
  hosted('decision-models/baseten-joey-2b-latest', 'joey-1'),
  hosted('elephant-1', 'elephant-1'),
  hosted('mammoth-1', 'mammoth-1'),
]);
const monitor = (id, name) => ({
  id, name, question: { instructions: 'Is it ' + name + '?', kind: { binary: {} } },
  threshold: 0.85, action: 'stop',
});
const settings = parseDecisionSettings({
  defaultModel: null,
  monitors: [monitor('a', 'Alpha'), monitor('b', 'Beta'), monitor('c', 'Gamma')],
});

function App() {
  const [current, setCurrent] = React.useState(settings);
  return (
    <MonitorsDialog
      open
      onOpenChange={() => {}}
      sessionId="s1"
      chatTier="private"
      settings={current}
      onSaved={setCurrent}
      models={models}
    />
  );
}
createRoot(document.getElementById('root')!).render(<App />);
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

const EPSILON = 1;
const near = (actual: number, expected: number, what: string) =>
  expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThanOrEqual(
    EPSILON
  );

describe('the Monitors dialog in a real layout engine', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Monitors dialog layout tests.');
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
    css =
      compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }])) +
      readFileSync(resolve(here, 'decisionsPanel.css'), 'utf8') +
      readFileSync(resolve(here, 'monitorsDialog.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1100, height: 900 } });
    await tab.emulateMedia({ reducedMotion: 'reduce' });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[data-testid="monitor-item"]');
    await tab.waitForTimeout(100);
    return tab;
  }

  const box = (tab: Page, selector: string) =>
    tab.evaluate((sel) => {
      const node = document.querySelector(sel);
      if (!node) return null;
      const rect = node.getBoundingClientRect();
      return { top: rect.top, left: rect.left, right: rect.right, width: rect.width };
    }, selector);

  it('fits the six-model picker of a private chat in its menu without scrolling', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.click('[data-testid="monitor-add"]');
      await tab.getByRole('button', { name: 'More options' }).click();
      await tab.click('[data-testid="monitor-model"] [class*="control"]');
      await tab.waitForSelector('[data-testid="decision-model-option"]');
      const menu = await tab.evaluate(() => {
        const rows = Array.from(document.querySelectorAll('[data-testid="decision-model-option"]'));
        // The menu's scrolling list: the nearest ancestor that scrolls.
        let list = rows[0].parentElement as HTMLElement;
        while (list && !/(auto|scroll)/.test(getComputedStyle(list).overflowY)) {
          list = list.parentElement as HTMLElement;
        }
        return {
          count: rows.length,
          scrollHeight: list.scrollHeight,
          clientHeight: list.clientHeight,
          heights: rows.map(
            (row) =>
              (
                row.closest('[role="option"], [aria-disabled]') as HTMLElement
              ).getBoundingClientRect().height
          ),
        };
      });
      expect(menu.count).toBe(6);
      // One-line rows, so nothing hides below the menu's 240px list.
      for (const height of menu.heights) expect(height).toBeLessThanOrEqual(33);
      expect(menu.scrollHeight).toBeLessThanOrEqual(menu.clientHeight);
    } finally {
      await tab.close();
    }
  });

  it('draws segmented tracks that hug their segments, not the editor’s width', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.click('[data-testid="monitor-add"]');
      await tab.getByRole('button', { name: 'More options' }).click();
      const shape = await tab.evaluate(() => {
        const editor = document
          .querySelector('[data-testid="monitor-editor"]')!
          .getBoundingClientRect();
        return ['monitor-action', 'monitor-kind', 'monitor-timing', 'monitor-on-error'].map(
          (id) => {
            const track = document.querySelector(`[data-testid="${id}"]`) as HTMLElement;
            const items = Array.from(track.querySelectorAll('.br-segmented-item'));
            const used = items.reduce((sum, item) => sum + item.getBoundingClientRect().width, 0);
            return { id, track: track.getBoundingClientRect().width, used, editor: editor.width };
          }
        );
      });
      for (const { id, track, used, editor } of shape) {
        // The track is its segments plus its 2px padding and border, never the field's width.
        expect(track, id).toBeLessThanOrEqual(used + 8);
        expect(track, id).toBeLessThan(editor - 64);
      }
    } finally {
      await tab.close();
    }
  });

  it('puts the editor where the row it edits was', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      // A row is one flex line (name and facts, ⋯, Switch), whatever the panel's own rules say.
      expect(
        await tab.evaluate(
          () => getComputedStyle(document.querySelector('[data-monitor-id="a"]')!).display
        )
      ).toBe('flex');
      const before = await box(tab, '[data-monitor-id="a"]');
      await tab.getByRole('button', { name: 'More actions for Alpha' }).click();
      await tab.getByRole('menuitem', { name: 'Edit' }).click();
      await tab.waitForSelector('[data-testid="monitor-editor"]');
      const editor = await box(tab, '[data-testid="monitor-editor"]');
      const beta = await box(tab, '[data-monitor-id="b"]');
      expect(await tab.$('[data-monitor-id="a"]')).toBeNull();
      // Its 4px of air above, then the card, in the row's own place; the next row below it.
      expect(editor!.top - before!.top).toBeGreaterThanOrEqual(0);
      expect(editor!.top - before!.top).toBeLessThanOrEqual(6);
      expect(beta!.top).toBeGreaterThan(editor!.top);
    } finally {
      await tab.close();
    }
  });

  it('opens a row’s peek below its line, clear of the row’s own name (R1-3)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.hover('[data-monitor-id="c"] [data-testid="monitor-line"]');
      await tab.waitForSelector('[data-radix-popper-content-wrapper] [data-side]');
      // The popper mounts before it is placed (its first frame sits above the
      // viewport), so measure only once it has been positioned.
      await tab.waitForFunction(() => {
        const peek = document.querySelector('[data-radix-popper-content-wrapper] [data-side]');
        return peek !== null && peek.getBoundingClientRect().top > 0;
      });
      const boxes = await tab.evaluate(() => {
        const rect = (node: Element) => node.getBoundingClientRect().toJSON() as DOMRect;
        const row = document.querySelector('[data-monitor-id="c"]')!;
        return {
          side: document
            .querySelector('[data-radix-popper-content-wrapper] [data-side]')!
            .getAttribute('data-side'),
          peek: rect(document.querySelector('[data-radix-popper-content-wrapper] [data-side]')!),
          name: rect(row.querySelector('.br-monitors__row-name')!),
          line: rect(row.querySelector('[data-testid="monitor-line"]')!),
        };
      });
      expect(boxes.side).toBe('bottom');
      expect(boxes.peek.top).toBeGreaterThanOrEqual(boxes.line.bottom);
      expect(boxes.peek.top).toBeGreaterThan(boxes.name.bottom);
    } finally {
      await tab.close();
    }
  });

  it('says a row’s whole line, timing included, without cutting it (W3-8)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const line = await tab.evaluate(() => {
        const node = document.querySelector(
          '[data-monitor-id="a"] [data-testid="monitor-line"]'
        ) as HTMLElement;
        return {
          text: node.textContent,
          scrollWidth: node.scrollWidth,
          clientWidth: node.clientWidth,
        };
      });
      expect(line.text).toBe('Stop the turn · when P(yes) is at or above 0.85 · after each step');
      // The line is cut with an ellipsis when it is too long; this one is not.
      expect(line.scrollWidth).toBeLessThanOrEqual(line.clientWidth);
    } finally {
      await tab.close();
    }
  });

  it('switches Fires when to Below by pointer and by keyboard (R1-1)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.click('[data-testid="monitor-add"]');
      await tab.getByRole('button', { name: 'More options' }).click();
      // As the report had it: Stop the turn, with Stop only once on.
      await tab.getByRole('radio', { name: 'Stop the turn' }).click();
      await tab.getByRole('checkbox', { name: 'Stop only once' }).check();
      await tab.click('[data-testid="monitor-model"] [class*="control"]');
      await tab.getByText('tev1:0.8b', { exact: false }).first().click();
      // A press on the slider first (it leaves the slider's pointer capture behind it).
      await tab.click('[data-testid="monitor-threshold"]');
      const sentence = tab.locator('[data-testid="monitor-fires-sentence"]');
      await expect
        .poll(() => sentence.textContent())
        .toMatch(/^Fires when P\(yes\) is at or above 0\.\d\d\.$/);
      const below = tab.getByRole('radio', { name: 'Below' });
      await below.click();
      await expect.poll(() => below.getAttribute('aria-checked')).toBe('true');
      await expect
        .poll(() => sentence.textContent())
        .toMatch(/^Fires when P\(yes\) is below 0\.\d\d\.$/);
      // And back, by keyboard: the focused segment's arrow key chooses its neighbour.
      await below.focus();
      await tab.keyboard.press('ArrowLeft');
      await expect
        .poll(() => tab.getByRole('radio', { name: 'At or above' }).getAttribute('aria-checked'))
        .toBe('true');
      await expect
        .poll(() => sentence.textContent())
        .toMatch(/^Fires when P\(yes\) is at or above 0\.\d\d\.$/);
      await tab.keyboard.press('ArrowRight');
      await expect
        .poll(() => sentence.textContent())
        .toMatch(/^Fires when P\(yes\) is below 0\.\d\d\.$/);
    } finally {
      await tab.close();
    }
  });

  it('keeps the header’s × in place and enabled through a row toggle', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const close = async () =>
        tab.evaluate(() => {
          const dialog = document.querySelector('[data-testid="monitors-dialog"]')!;
          const node = Array.from(dialog.querySelectorAll('button')).find(
            (button) => button.textContent?.trim() === 'Close'
          );
          if (!node) return null;
          const rect = node.getBoundingClientRect();
          return { top: rect.top, left: rect.left, disabled: node.disabled };
        });
      const rest = await close();
      expect(rest).not.toBeNull();
      await tab.getByRole('switch', { name: 'Beta' }).click();
      await tab.waitForSelector('[data-testid="monitors-status"][data-state="saving"]');
      const mid = await close();
      await tab.waitForSelector('[data-testid="monitors-status"][data-state="saved"]');
      const after = await close();
      for (const [what, value] of [
        ['mid-write', mid],
        ['after', after],
      ] as const) {
        expect(value, what).not.toBeNull();
        near(value!.top, rest!.top, `${what} top`);
        near(value!.left, rest!.left, `${what} left`);
        expect(value!.disabled, what).toBe(false);
      }
      expect(await tab.getByRole('switch', { name: 'Beta' }).getAttribute('aria-checked')).toBe(
        'false'
      );
    } finally {
      await tab.close();
    }
  });
});
