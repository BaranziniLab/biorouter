// @vitest-environment node
/**
 * Keyboard focus comes back to the Add button after a dialog it opened is closed with the keyboard,
 * in a real browser (vision round 3, V3-R3-1).
 *
 * The Skills band's Add menu opens Browse marketplace, "From a repository or .zip…" and "Write a
 * skill…". After Add, Enter, a pick, Enter and Escape, `document.activeElement` was `<body>` for
 * all three, while the same keyboard path on Extensions > "Install from file (.brxt)" came back to
 * Add. The cause was not the mount (`{open && <Modal open />}` is what Extensions does too) but a
 * React `autoFocus` on each dialog's field. React focuses that field as the content commits, and
 * Radix's focus scope announces `onOpenAutoFocus` only when focus is NOT already inside the
 * content, so the announcement never came, `useReturnFocusToOpener` never recorded the control
 * that was pressed, and Escape had nothing to return to. jsdom cannot see this (it runs no focus
 * scope against real focus), so the real `SkillsView` is bundled with esbuild, styled with the
 * whole stylesheet and driven with real key presses in Chromium. Only the daemon edge is stubbed
 * (the generated API module, toasts, the per-machine overrides store).
 *
 * "The instrument": the same bundle with `autoFocus` put back on the three fields must leave focus
 * on `<body>`, and a pointer pick followed by a pointer close must give nothing back (the menus'
 * rule, kept). A mutation that matches nothing throws, so a refactor cannot turn the instrument
 * into a no-op.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { SKILLS_COPY } from './copy';

const here = dirname(fileURLToPath(import.meta.url));
const SRC = resolve(here, '../..');
const STYLES = resolve(SRC, 'styles');
const requireFromDesktop = createRequire(resolve(SRC, '../package.json'));

/** The calls the Skills view and its dialogs make themselves. */
const SKILL_CALLS = [
  'skillCatalogHandler',
  'refreshSkillCatalog',
  'setSessionSkills',
  'removeSkillPackage',
  'previewSkillPackage',
  'installSkillPackage',
];

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import SkillsView from './SkillsView';

createRoot(document.getElementById('root')).render(<SkillsView />);
`;

/**
 * Every call the generated client exports, so a module elsewhere in the bundle that imports one
 * (a schedule, a session) still links. All of them fail except the six this view makes.
 */
const API_NAMES = [
  ...readFileSync(resolve(SRC, 'api/sdk.gen.ts'), 'utf8').matchAll(/^export const (\w+)/gm),
]
  .map((match) => match[1])
  .filter((name) => !SKILL_CALLS.includes(name));

/** Modules the view reaches the daemon through, replaced by answers that need no daemon. */
const STUBS: Array<[string, string]> = [
  [
    'api',
    `
    const view = {
      generation: 1,
      roots: [],
      bundles: [],
      skills: [{
        name: 'ggplot', description: 'Plots, in one line.', slug: 'ggplot', directory: '/skills/ggplot',
        sourceRoot: '/skills', source: { kind: 'biorouter', extension: null, label: 'Biorouter' },
        bundle: null, builtin: false,
        state: { machineEnabled: true, session: 'default', sessionViaBundle: false, hiddenContext: false, effective: true },
      }],
    };
    const offline = async () => { throw new Error('offline'); };
    ${API_NAMES.map((name) => `export const ${name} = offline;`).join('\n')}
    export const skillCatalogHandler = async () => ({ data: view });
    export const refreshSkillCatalog = async () => ({ data: view });
    export const setSessionSkills = async () => ({ data: view });
    export const removeSkillPackage = async () => ({ data: {} });
    export const previewSkillPackage = offline;
    export const installSkillPackage = offline;
  `,
  ],
  ['toasts', `export const toastError = () => {}; export const toastSuccess = () => {};`],
  [
    'store/skillOverrides',
    `
    export const loadSkillOverrides = async () => true;
    export const saveSkillOverrides = async () => undefined;
    export const setSkillOverride = () => {};
    export const isSkillEnabled = () => true;
    export const getSkillOverrides = () => new Map();
  `,
  ],
  ['utils/userAction', `export const userActionHeaders = async () => ({});`],
];

/**
 * The previous code: React's `autoFocus` back on each dialog's field. Each swap must match, or
 * the build throws.
 */
const OLD_AUTOFOCUS: Array<[RegExp, Array<[string, string]>]> = [
  [/skills\/AddSkillModal\.tsx$/, [['<Input\n', '<Input autoFocus\n']]],
  [/skills\/CustomSkillModal\.tsx$/, [['<Textarea\n', '<Textarea autoFocus\n']]],
  [/baam\/MarketplaceDialog\.tsx$/, [['<Input\n', '<Input autoFocus\n']]],
];

async function bundle(restoreAutoFocus: boolean): Promise<string> {
  const stubs: Plugin = {
    name: 'stubs',
    setup(b) {
      const byPath = new Map(STUBS.map(([name, contents]) => [resolve(SRC, name), contents]));
      b.onResolve({ filter: /^\.{1,2}\// }, (args) => {
        const absolute = resolve(args.resolveDir, args.path);
        return byPath.has(absolute) ? { path: absolute, namespace: 'stub' } : undefined;
      });
      b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
        contents: byPath.get(args.path)!,
        loader: 'ts',
        resolveDir: here,
      }));
      if (restoreAutoFocus) {
        b.onLoad(
          { filter: /\/(AddSkillModal|CustomSkillModal|MarketplaceDialog)\.tsx$/ },
          (args) => {
            let source = readFileSync(args.path, 'utf8');
            for (const [file, swaps] of OLD_AUTOFOCUS) {
              if (!file.test(args.path)) continue;
              for (const [from, to] of swaps) {
                if (!source.includes(from))
                  throw new Error(
                    `mutation target missing in ${args.path}: ${JSON.stringify(from)}`
                  );
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
    define: { 'process.env.NODE_ENV': '"production"', 'process.env.ALPHA': 'false' },
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

/** The three dialogs, by the menu row that opens them and the field that should take first focus. */
const DIALOGS = [
  { item: SKILLS_COPY.browse, field: 'input[type="text"]', title: 'Browse skills' },
  { item: SKILLS_COPY.fromSource, field: '#skill-source-url', title: 'Add skill' },
  { item: SKILLS_COPY.write, field: 'textarea', title: 'Write a skill' },
] as const;

describe(
  'Skills Add menu: focus around the dialogs it opens (V3-R3-1)',
  { timeout: 90_000 },
  () => {
    let browser: Browser | null = null;
    let fixed = '';
    let old = '';
    let css = '';

    beforeAll(async () => {
      browser = await launchChromium();
      if (!browser) {
        console.warn('No Chromium found: skipping the Skills focus tests.');
        return;
      }
      fixed = await bundle(false);
      old = await bundle(true);
      // The whole stylesheet, so the dialog sits where a person sees it and a pointer can press
      // its buttons: Tailwind compiled for the classes this bundle uses, plus the two authored
      // sheets the bundle's `.css` loader drops.
      const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
      const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
      const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
        base: STYLES,
        onDependency() {},
      });
      css = compiler.build(new Scanner({}).scanFiles([{ content: fixed, extension: 'js' }]));
      css += readFileSync(resolve(here, 'skills.css'), 'utf8');
      css += readFileSync(resolve(here, '../baam/marketplace.css'), 'utf8');
    }, 180_000);

    afterAll(async () => {
      await browser?.close();
    }, 120_000);

    async function open(script: string): Promise<Page> {
      const tab = await browser!.newPage({ viewport: { width: 1100, height: 800 } });
      tab.setDefaultTimeout(20_000);
      await tab.route('http://skills.test/**', (route) =>
        route.fulfill({
          status: 200,
          contentType: 'text/html',
          body: `<!doctype html><html><head><style>${css}</style></head><body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`,
        })
      );
      await tab.goto('http://skills.test/');
      await tab.addScriptTag({ content: script });
      await tab.getByRole('button', { name: SKILLS_COPY.add, exact: true }).waitFor();
      return tab;
    }

    /** What holds focus, as a name a failure message can read. */
    const holder = (tab: Page) =>
      tab.evaluate(() => {
        const el = document.activeElement;
        if (!el || el === document.body) return 'BODY';
        return `${el.tagName.toLowerCase()}${el.id ? `#${el.id}` : ''}:${(el.textContent ?? '').trim().slice(0, 24)}`;
      });

    /** Add, Enter, arrow to the row, Enter: a dialog opens and `field` has focus. */
    async function pickByKeyboard(tab: Page, item: string, field: string) {
      await tab.getByRole('button', { name: SKILLS_COPY.add, exact: true }).focus();
      await tab.keyboard.press('Enter');
      await tab.getByRole('menu').waitFor();
      // Radix places focus on the first row a frame after the menu mounts, and an arrow pressed
      // before that settles can be lost, so walk until the wanted row holds focus instead of
      // counting presses (the list wraps, and a stray extra press would otherwise pick another row).
      const onItem = () =>
        tab.evaluate((name) => document.activeElement?.textContent?.includes(name) ?? false, item);
      await tab.waitForFunction(() => document.activeElement?.getAttribute('role') === 'menuitem');
      for (let tries = 0; tries < 8 && !(await onItem()); tries += 1) {
        await tab.keyboard.press('ArrowDown');
        await tab.waitForTimeout(120);
      }
      expect(await onItem(), `${item} holds focus in the menu`).toBe(true);
      await tab.keyboard.press('Enter');
      await tab.getByRole('dialog').waitFor();
      await tab.locator(`[role="dialog"] ${field}`).waitFor();
    }

    const closed = (tab: Page) => tab.getByRole('dialog').waitFor({ state: 'detached' });

    /**
     * The Add button holds focus. Polled: Radix hands focus back a tick AFTER the content has left
     * the document, so a read taken the instant the dialog detaches can still see the page body.
     */
    const expectFocusOnAdd = (tab: Page) =>
      expect.poll(() => holder(tab), { timeout: 5_000 }).toMatch(/^button(#[^:]*)?:Add$/);

    /** Nothing is given back. Read after the same tick plus a margin, so a late return would show. */
    const settledHolder = async (tab: Page) => {
      await tab.waitForTimeout(500);
      return holder(tab);
    };

    for (const dialog of DIALOGS) {
      it(`${dialog.title}: first focus lands on its field, and Escape gives focus back to Add`, async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await open(fixed);
        try {
          await pickByKeyboard(tab, dialog.item, dialog.field);
          // The field the person types in, not the help glyph beside its label and not the box.
          expect(await tab.evaluate((f) => document.activeElement?.matches(f), dialog.field)).toBe(
            true
          );
          await tab.keyboard.press('Escape');
          await closed(tab);
          await expectFocusOnAdd(tab);
        } finally {
          await tab.close();
        }
      });

      it(`${dialog.title}: the Cancel button, pressed with the keyboard, gives focus back too`, async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await open(fixed);
        try {
          await pickByKeyboard(tab, dialog.item, dialog.field);
          await tab
            .getByRole('button', { name: /^(Cancel|Close)$/ })
            .first()
            .focus();
          await tab.keyboard.press('Enter');
          await closed(tab);
          await expectFocusOnAdd(tab);
        } finally {
          await tab.close();
        }
      });
    }

    it('a pointer pick still puts first focus on the field, and a pointer close gives nothing back', async (ctx) => {
      if (!browser) return ctx.skip();
      for (const dialog of DIALOGS) {
        const tab = await open(fixed);
        try {
          await tab.getByRole('button', { name: SKILLS_COPY.add, exact: true }).click();
          await tab.getByRole('menuitem', { name: dialog.item }).click();
          await tab.locator(`[role="dialog"] ${dialog.field}`).waitFor();
          expect(
            await tab.evaluate((f) => document.activeElement?.matches(f), dialog.field),
            dialog.title
          ).toBe(true);
          await tab
            .getByRole('button', { name: /^(Cancel|Close)$/ })
            .first()
            .click();
          await closed(tab);
          // The menus' rule (V4-04): a script-focused button after a mouse click would wear a
          // keyboard ring. Nothing is given back.
          expect(await settledHolder(tab), dialog.title).toBe('BODY');
        } finally {
          await tab.close();
        }
      }
    });

    it('the instrument: with autoFocus back on the fields, Escape leaves focus on the page body', async (ctx) => {
      if (!browser) return ctx.skip();
      for (const dialog of DIALOGS) {
        const tab = await open(old);
        try {
          await pickByKeyboard(tab, dialog.item, dialog.field);
          await tab.keyboard.press('Escape');
          await closed(tab);
          expect(await settledHolder(tab), dialog.title).toBe('BODY');
        } finally {
          await tab.close();
        }
      }
    });
  }
);
