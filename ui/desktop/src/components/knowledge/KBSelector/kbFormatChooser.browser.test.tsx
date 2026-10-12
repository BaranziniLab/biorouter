// @vitest-environment node
/**
 * The Create knowledge base dialog answers an empty Name on the field, in a real layout engine
 * (V3-R4-3).
 *
 * The defect was geometry and focus, which jsdom cannot see: the message was a detached 14px
 * paragraph at the foot of the dialog, so its own height pushed Cancel and Create 36px down from
 * under the pointer (y=422 to y=458), the input drew no danger edge, and focus stayed on the button.
 * `KbFormatChooser.test.tsx` pins the semantics (the alert, `aria-invalid`, the description, focus);
 * this lays the real chooser out in Chromium, with `styles/main.css` compiled by Tailwind over the
 * classes the bundle uses, in light and dark, and measures:
 *  - the footer buttons and the dialog do not move when the message appears, and the Create button
 *    is still the element under the pointer that pressed it;
 *  - the message is the field's own line, 12px, in the danger ink, beneath the input;
 *  - the input draws the danger edge, at the same 1px, and holds the caret;
 *  - a failure the daemon reports is NOT on the field, and the field's edge stays quiet;
 *  - Create is the coral primary only once the Name can be used (V3-R9-4, ruling 22): over an empty
 *    Name it paints what Cancel paints, once a name is typed it paints the accent, and it is
 *    pressable throughout;
 *  - the manager's rename card (the real `KBManagerDialog` over two stand-in contexts) answers an
 *    empty name the same way: the field's 12px line, the danger edge, the caret in the input.
 * Set `KB_CHOOSER_SHOTS` to a directory to also save the screenshots the review reads.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../../package.json'));
const SHOTS = process.env.KB_CHOOSER_SHOTS;

/** The chooser as the manager mounts it, with a `create` the test decides the answer of. */
const CHOOSER_ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { KbFormatChooser } from './KbFormatChooser';

createRoot(document.getElementById('root')!).render(
  <KbFormatChooser
    open
    onOpenChange={() => {}}
    initialName=""
    onCreate={() => window.__createResult()}
  />
);
`;

/** The manager, for its rename card: the real dialog over two stand-in contexts. */
const MANAGER_ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { KBManagerDialog } from './KBManagerDialog';

createRoot(document.getElementById('root')!).render(
  <KBManagerDialog open onOpenChange={() => {}} />
);
`;

/** What the manager reads from the app: two bases, and a client that is never reached. */
const STUBS: Record<string, string> = {
  KnowledgeContext: `
    const value = {
      bases: [
        { id: 'alpha', name: 'Alpha', color: '#cf6d47', tier: 'public' },
        { id: 'beta', name: 'Beta', color: '#b85a32', tier: 'public' },
      ],
      loading: false,
      primaryKbId: 'alpha',
      hiddenKbIds: [],
      refresh: async () => {},
      setPrimaryKbId: () => {},
      toggleKbHidden: () => {},
    };
    export const useKnowledge = () => value;
  `,
  useKnowledgeBases: `
    const value = {
      create: async () => ({ id: 'x' }),
      exportArchive: async () => {},
      importArchive: async () => {},
      remove: async () => {},
      rename: async () => {},
    };
    export const useKnowledgeBases = () => value;
  `,
};

const stubPlugin: Plugin = {
  name: 'kb-manager-stubs',
  setup(b) {
    b.onResolve({ filter: /\/(KnowledgeContext|useKnowledgeBases)$/ }, (args) => {
      const name = Object.keys(STUBS).find((key) => args.path.endsWith(`/${key}`));
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

type Look = {
  top: number;
  left: number;
  width: number;
  height: number;
  color: string;
  fontSize: string;
  edge: string;
  edgeWidth: string;
};

describe.each(['light', 'dark'] as const)('the Create knowledge base dialog (%s)', (mode) => {
  let browser: Browser | null = null;
  let chooser = { script: '', css: '' };
  let manager = { script: '', css: '' };

  /** An entry bundled with esbuild, and `main.css` compiled by Tailwind over the classes it uses. */
  async function bundleOf(entry: string, plugins: Plugin[] = []) {
    const bundle = await build({
      stdin: { contents: entry, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: { 'process.env.NODE_ENV': '"production"' },
      loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
      plugins,
      logLevel: 'silent',
    });
    const script = bundle.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    return {
      script,
      css: compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }])),
    };
  }

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the format chooser tests.');
      return;
    }
    chooser = await bundleOf(CHOOSER_ENTRY);
    manager = await bundleOf(MANAGER_ENTRY, [stubPlugin]);
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(
    result: 'resolve' | 'reject' = 'resolve',
    { script, css } = chooser,
    ready = '[data-testid="knowledge-format-name"]'
  ): Promise<Page> {
    const page = await browser!.newPage({
      viewport: { width: 1000, height: 800 },
      reducedMotion: 'reduce',
    });
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}">` +
        `<head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div>` +
        `<span id="probe-ink" style="color:var(--text-danger)">x</span>` +
        `<span id="probe-edge" style="border:1px solid var(--border-danger)">x</span>` +
        `<span id="probe-accent" style="background-color:var(--background-accent)">x</span></body></html>`
    );
    // After the content is set (a set document drops earlier init scripts) and before the bundle.
    await page.evaluate((outcome) => {
      (window as unknown as { __createResult: () => Promise<string> }).__createResult = () =>
        outcome === 'reject'
          ? Promise.reject(new Error('The daemon is not reachable.'))
          : Promise.resolve('x');
    }, result);
    await page.addScriptTag({ content: script });
    await page.waitForSelector(ready);
    // Let the dialog's open motion settle, so the first reading is the resting one.
    await page.waitForTimeout(150);
    expect(errors, 'page errors').toEqual([]);
    return page;
  }

  const look = (page: Page, selector: string): Promise<Look> =>
    page.evaluate((sel) => {
      const el = document.querySelector(sel)!;
      const box = el.getBoundingClientRect();
      const cs = getComputedStyle(el);
      return {
        top: Math.round(box.top * 10) / 10,
        left: Math.round(box.left * 10) / 10,
        width: Math.round(box.width * 10) / 10,
        height: Math.round(box.height * 10) / 10,
        color: cs.color,
        fontSize: cs.fontSize,
        edge: cs.borderTopColor,
        edgeWidth: cs.borderTopWidth,
      };
    }, selector);

  const NAME = '[data-testid="knowledge-format-name"]';
  const CREATE = '[data-testid="knowledge-format-submit"]';
  const DIALOG = '[role="dialog"]';

  it('answers an empty Name on the field, and nothing under the pointer moves', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open();
    try {
      const cancelBefore = await page.evaluate(() => {
        const button = [...document.querySelectorAll('button')].find(
          (b) => b.textContent?.trim() === 'Cancel'
        )!;
        const box = button.getBoundingClientRect();
        return { top: box.top, left: box.left };
      });
      const createBefore = await look(page, CREATE);
      const dialogBefore = await look(page, DIALOG);
      const nameBefore = await look(page, NAME);
      expect(nameBefore.edgeWidth).toBe('1px');
      expect(await page.getAttribute(NAME, 'aria-invalid')).toBeNull();
      if (SHOTS) {
        mkdirSync(SHOTS, { recursive: true });
        await page.screenshot({ path: join(SHOTS, `format-chooser-rest-${mode}.png`) });
      }

      // Press Create with the pointer, the way the owner's review did.
      const pointer = { x: createBefore.left + 20, y: createBefore.top + 10 };
      await page.mouse.click(pointer.x, pointer.y);
      await page.waitForSelector('[role="alert"]');
      await page.waitForTimeout(150);

      // Nothing under the pointer moved: the dialog is the same box and so are the footer buttons.
      const dialogAfter = await look(page, DIALOG);
      const createAfter = await look(page, CREATE);
      const cancelAfter = await page.evaluate(() => {
        const button = [...document.querySelectorAll('button')].find(
          (b) => b.textContent?.trim() === 'Cancel'
        )!;
        const box = button.getBoundingClientRect();
        return { top: box.top, left: box.left };
      });
      expect(Math.abs(dialogAfter.height - dialogBefore.height), 'dialog height').toBeLessThan(0.6);
      expect(Math.abs(dialogAfter.top - dialogBefore.top), 'dialog top').toBeLessThan(0.6);
      expect(Math.abs(createAfter.top - createBefore.top), 'Create top').toBeLessThan(0.6);
      expect(Math.abs(cancelAfter.top - cancelBefore.top), 'Cancel top').toBeLessThan(0.6);
      expect(
        await page.evaluate(
          ({ x, y }) => document.elementFromPoint(x, y)?.closest('button')?.textContent?.trim(),
          pointer
        ),
        'still the Create button under the pointer that pressed it'
      ).toBe('Create knowledge base');

      // The message is the field's own line: 12px, the danger ink, below the input.
      const probe = await look(page, '#probe-ink');
      const message = await look(page, '[data-slot="field-error"]');
      const input = await look(page, NAME);
      expect(message.fontSize).toBe('12px');
      expect(message.color).toBe(probe.color);
      expect(message.top).toBeGreaterThan(input.top + input.height - 0.5);
      expect(message.top - (input.top + input.height), 'a 6px gap under the field').toBeLessThan(8);

      // The input draws the danger edge at the same 1px, and holds the caret.
      const edge = await look(page, '#probe-edge');
      expect(input.edgeWidth).toBe('1px');
      expect(input.edge).toBe(edge.edge);
      expect(input.edge).not.toBe(nameBefore.edge);
      expect(await page.getAttribute(NAME, 'aria-invalid')).toBe('true');
      expect(
        await page.evaluate((s) => document.activeElement === document.querySelector(s), NAME)
      ).toBe(true);
      // Its description is the message, linked by id.
      const describedBy = await page.getAttribute(NAME, 'aria-describedby');
      expect(
        await page.evaluate((id) => document.getElementById(id!)?.textContent, describedBy)
      ).toBe('Enter a name.');
      if (SHOTS) {
        await page.screenshot({ path: join(SHOTS, `format-chooser-empty-submit-${mode}.png`) });
      }

      // Typing lifts it: the folder line is back, the edge is quiet, nothing moves.
      await page.keyboard.type('Cohort notes');
      await page.waitForTimeout(100);
      expect(await page.locator('[role="alert"]').count()).toBe(0);
      const typed = await look(page, NAME);
      expect(typed.edge).not.toBe(edge.edge);
      expect(Math.abs((await look(page, CREATE)).top - createBefore.top)).toBeLessThan(0.6);
    } finally {
      await page.close();
    }
  }, 120_000);

  it('draws Create neutral over an empty Name and coral once it can act', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open();
    try {
      const ground = (selector: string) =>
        page
          .locator(selector)
          .first()
          .evaluate((el) => getComputedStyle(el).backgroundColor);
      const CANCEL = 'button:has-text("Cancel")';
      const accent = await ground('#probe-accent');
      const cancel = await ground(CANCEL);
      // The accent and the control ground are different things, or this instrument reads nothing.
      expect(accent).not.toBe(cancel);

      // Opened empty: the same ground as Cancel (both secondary), not the accent, and pressable.
      expect(await ground(CREATE), 'empty Name: the control ground').toBe(cancel);
      expect(await ground(CREATE)).not.toBe(accent);
      expect(await page.isEnabled(CREATE)).toBe(true);
      if (SHOTS) {
        mkdirSync(SHOTS, { recursive: true });
        await page.screenshot({ path: join(SHOTS, `format-chooser-create-neutral-${mode}.png`) });
      }

      // A name that cannot make a folder is still "not yet".
      await page.keyboard.type('!!!');
      await page.waitForTimeout(200);
      expect(await ground(CREATE), 'no letters: still the control ground').toBe(cancel);

      // A usable name: the accent, and Cancel stays where it was.
      await page.fill(NAME, '');
      await page.keyboard.type('Cohort notes');
      await page.waitForTimeout(200);
      expect(await ground(CREATE), 'a usable Name: the accent').toBe(accent);
      expect(await ground(CANCEL)).toBe(cancel);
      expect(await page.isEnabled(CREATE)).toBe(true);
      if (SHOTS) {
        await page.screenshot({ path: join(SHOTS, `format-chooser-create-coral-${mode}.png`) });
      }

      // Emptied again, it steps back, and a click still says what is missing.
      await page.fill(NAME, '');
      await page.waitForTimeout(200);
      expect(await ground(CREATE)).toBe(cancel);
      await page.click(CREATE);
      await page.waitForSelector('[role="alert"]');
      expect(await page.locator('[role="alert"]').innerText()).toBe('Enter a name.');
    } finally {
      await page.close();
    }
  }, 120_000);

  it('keeps a failure the daemon reports off the field', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open('reject');
    try {
      await page.fill(NAME, 'Cohort notes');
      await page.click(CREATE);
      await page.waitForSelector('[role="alert"]');
      const alertText = await page.locator('[role="alert"]').innerText();
      expect(alertText).toContain('The daemon is not reachable.');
      // Not about the name: the field is not marked and it draws no danger edge (the click left
      // it, so it is back at its quiet resting edge).
      expect(await page.getAttribute(NAME, 'aria-invalid')).toBeNull();
      const danger = await look(page, '#probe-edge');
      expect((await look(page, NAME)).edge).not.toBe(danger.edge);
      expect(
        await page.evaluate(() =>
          document.querySelector('[role="alert"]')?.closest('[data-slot="field"]')
        )
      ).toBeNull();
      if (SHOTS) {
        await page.screenshot({ path: join(SHOTS, `format-chooser-failure-${mode}.png`) });
      }
    } finally {
      await page.close();
    }
  }, 120_000);

  it('answers an empty rename on the field too, in the manager’s card', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open('resolve', manager, '[data-testid="knowledge-kb-create"]');
    try {
      await page.click('[aria-label="More actions for Alpha"]');
      await page.getByRole('menuitem', { name: /Rename/ }).click();
      const INPUT = '[data-testid="knowledge-kb-name-input"]';
      await page.waitForSelector(INPUT);
      // Save is coral while the name can be saved, and neutral (Cancel's ground) once emptied.
      const ground = (selector: string) =>
        page
          .locator(selector)
          .first()
          .evaluate((el) => getComputedStyle(el).backgroundColor);
      const SAVE = '[data-testid="knowledge-kb-submit"]';
      const accent = await ground('#probe-accent');
      expect(await ground(SAVE), 'a name to save: the accent').toBe(accent);
      await page.fill(INPUT, '');
      await page.waitForTimeout(200);
      expect(await ground(SAVE), 'emptied: not the accent').not.toBe(accent);
      expect(await ground(SAVE), 'emptied: the control ground').toBe(
        await ground('button:has-text("Cancel")')
      );
      const restEdge = (await look(page, INPUT)).edge;
      await page.click('[data-testid="knowledge-kb-submit"]');
      await page.waitForSelector('[role="alert"]');
      await page.waitForTimeout(100);

      const probe = await look(page, '#probe-ink');
      const edge = await look(page, '#probe-edge');
      const message = await look(page, '[data-slot="field-error"]');
      const input = await look(page, INPUT);
      expect(message.fontSize, 'the field’s 12px line, not a 14px body paragraph').toBe('12px');
      expect(message.color).toBe(probe.color);
      expect(message.top).toBeGreaterThan(input.top + input.height - 0.5);
      expect(input.edge).toBe(edge.edge);
      expect(input.edge).not.toBe(restEdge);
      expect(input.edgeWidth).toBe('1px');
      expect(await page.getAttribute(INPUT, 'aria-invalid')).toBe('true');
      expect(
        await page.evaluate((s) => document.activeElement === document.querySelector(s), INPUT)
      ).toBe(true);
      expect(
        await page.evaluate(
          (s) =>
            document.getElementById(document.querySelector(s)!.getAttribute('aria-describedby')!)
              ?.textContent,
          INPUT
        )
      ).toBe('Enter a name.');
      expect(await page.locator('label[for="kb-rename-name"]').innerText()).toBe('Rename "Alpha"');
      if (SHOTS) {
        await page.screenshot({ path: join(SHOTS, `kb-manager-rename-empty-${mode}.png`) });
      }
    } finally {
      await page.close();
    }
  }, 120_000);
});
