// @vitest-environment node
/**
 * A dialog opened with the pointer draws no keyboard ring (vision round 5, V4-R5-1).
 *
 * Open Reset data with the mouse, close it with Escape (focus goes back to the trigger, ringed,
 * which is right after a key), then click the trigger again. The click changes no focus, so
 * Chromium keeps the keyboard modality, and Radix's auto-focus inherited it: "Select all" opened
 * ringed, and a following click on it kept the ring. The shared dialog now rests a pointer open on
 * its own box (no ring, `.biorouter-modal-surface:focus`) unless the first stop is a text field.
 *
 * jsdom has no `:focus-visible`, so the real `ModalShell`, `DialogContent` and `SheetContent` are
 * bundled with esbuild and driven with Playwright's mouse and keyboard in Chromium. The first case
 * is the instrument: Radix's own content, given the same steps, rings its first button.
 *
 * `DIALOG_SOURCE=<file>` bundles another `ui/dialog.tsx` in place of this one (its imports still
 * resolve from `components/ui`), so the cases can be run against an older version.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');

/** The authored `.biorouter-modal-surface:focus` rule, read from main.css. */
function boxFocusRule(): string {
  const match = MAIN_CSS.match(
    /\/\* @ws WS-PRIMITIVES begin: dialog box focus \*\/([\s\S]*?)\/\* @ws WS-PRIMITIVES end: dialog box focus \*\//
  );
  if (!match) throw new Error('main.css has no "dialog box focus" block');
  return match[1];
}

/** A ring on anything `:focus-visible`: the stand-in for the app's focus rules. */
const PAGE_CSS = `
  body { margin: 0; font: 14px sans-serif; }
  *:focus-visible { outline: 2px solid red; }
  [data-slot=dialog-content], [data-slot=sheet-content] { position: fixed; top: 40px; left: 40%; width: 420px; background: #fff; }
`;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import { ModalShell } from '../ModalShell';
import { Sheet, SheetContent, SheetTitle, SheetDescription } from './sheet';

type Kind = null | 'reset' | 'form' | 'late-field' | 'select' | 'sheet';

/** Reset data's body: a toggle that reads "Select all" until something is chosen, then "Clear". */
function ResetBody() {
  const [all, setAll] = React.useState(false);
  return (
    <div>
      <button id="toggle-all" type="button" onClick={() => setAll(!all)}>{all ? 'Clear' : 'Select all'}</button>
      <label><input id="check-sessions" type="checkbox" checked={all} onChange={() => setAll(!all)} /> Chats</label>
    </div>
  );
}

function App() {
  const [kind, setKind] = React.useState<Kind>(null);
  const close = () => setKind(null);
  const change = (next: boolean) => !next && close();
  return (
    <div>
      <button id="open-reset" type="button" onClick={() => setKind('reset')}>Reset data</button>
      <button id="open-form" type="button" onClick={() => setKind('form')}>Rename</button>
      <button id="open-late" type="button" onClick={() => setKind('late-field')}>Late field</button>
      <button id="open-select" type="button" onClick={() => setKind('select')}>Schedule</button>
      <button id="open-sheet" type="button" onClick={() => setKind('sheet')}>Sheet</button>

      <DialogPrimitive.Root>
        <DialogPrimitive.Trigger id="open-raw">Raw reset</DialogPrimitive.Trigger>
        <DialogPrimitive.Portal>
          <DialogPrimitive.Content data-slot="dialog-content" aria-describedby={undefined}>
            <DialogPrimitive.Title>Raw</DialogPrimitive.Title>
            <button id="raw-first" type="button">Select all</button>
          </DialogPrimitive.Content>
        </DialogPrimitive.Portal>
      </DialogPrimitive.Root>

      <ModalShell open={kind === 'reset'} onOpenChange={change} title="Reset data"
        footer={<><button id="reset-cancel" type="button" onClick={close}>Cancel</button><button id="reset-go" type="button">Clear</button></>}>
        <ResetBody />
      </ModalShell>
      <ModalShell open={kind === 'form'} onOpenChange={change} title="Rename"
        footer={<button id="form-ok" type="button" onClick={close}>Save</button>}>
        <input id="form-input" aria-label="Name" defaultValue="Chat" />
      </ModalShell>
      <ModalShell open={kind === 'late-field'} onOpenChange={change} title="Late field"
        footer={<button id="late-ok" type="button" onClick={close}>Save</button>}>
        <button id="late-first" type="button">Paste</button>
        <input id="late-input" aria-label="Value" />
      </ModalShell>
      <ModalShell open={kind === 'select'} onOpenChange={change} title="Schedule"
        footer={<button id="select-ok" type="button" onClick={close}>Save</button>}>
        <div className="br-select"><input id="select-input" aria-label="Every" role="combobox" /></div>
      </ModalShell>
      <Sheet open={kind === 'sheet'} onOpenChange={change}>
        <SheetContent>
          <SheetTitle>Sheet</SheetTitle>
          <SheetDescription>A side sheet.</SheetDescription>
          <button id="sheet-first" type="button">History</button>
        </SheetContent>
      </Sheet>
    </div>
  );
}
createRoot(document.getElementById('root')!).render(<App />);
`;

/** `DIALOG_SOURCE` swaps the shared dialog for another file, resolved as if it sat here. */
function dialogSource(): Plugin[] {
  const alternate = process.env.DIALOG_SOURCE;
  if (!alternate) return [];
  return [
    {
      name: 'dialog-source',
      setup(context) {
        context.onResolve({ filter: /^\.\/(ui\/)?dialog$/ }, () => ({
          path: alternate,
          namespace: 'dialog-source',
        }));
        context.onLoad({ filter: /.*/, namespace: 'dialog-source' }, () => ({
          contents: readFileSync(alternate, 'utf8'),
          loader: 'tsx',
          resolveDir: here,
        }));
      },
    },
  ];
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

interface Focus {
  id: string;
  role: string;
  focusVisible: boolean;
  outline: string;
  /** Every control in an open dialog that matches `:focus-visible` (the box itself left out). */
  ringedInside: string[];
}

describe('a dialog opened with the pointer (V4-R5-1)', () => {
  let browser: Browser | null = null;
  let script = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the pointer-open dialog tests.');
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
      plugins: dialogSource(),
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function openPage(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 600 } });
    tab.setDefaultTimeout(8000);
    await tab.setContent(
      `<!doctype html><html><head><style>${PAGE_CSS}${boxFocusRule()}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#open-reset');
    return tab;
  }

  const focused = (tab: Page): Promise<Focus> =>
    tab.evaluate(() => {
      const el = document.activeElement as HTMLElement;
      const box = document.querySelector('[role=dialog]');
      const ringedInside = box
        ? Array.from(box.querySelectorAll<HTMLElement>('*'))
            .filter((node) => node.matches(':focus-visible'))
            .map((node) => node.id || node.tagName.toLowerCase())
        : [];
      return {
        id: el.id,
        role: el.getAttribute('role') ?? '',
        focusVisible: el.matches(':focus-visible'),
        outline: getComputedStyle(el).outlineStyle,
        ringedInside,
      };
    });

  const dialog = '[role=dialog]';
  /** Let Radix's exit animation and the focus scope's deferred restore finish. */
  const settle = (tab: Page) => tab.waitForTimeout(250);

  /** Mouse open, Escape (focus back on the opener, ringed), mouse open again. */
  async function reopenAfterEscape(tab: Page, opener: string) {
    await tab.click(opener);
    await tab.waitForSelector(dialog);
    await settle(tab);
    await tab.keyboard.press('Escape');
    await tab.waitForSelector(dialog, { state: 'detached' });
    await settle(tab);
    const back = await focused(tab);
    expect(`#${back.id}`, 'Escape gives focus back to the opener').toBe(opener);
    expect(back.focusVisible, 'and rings it: the closing input was a key').toBe(true);
    await tab.click(opener);
    await tab.waitForSelector(dialog);
    await settle(tab);
  }

  it('the instrument: Radix’s own content rings its first button on that reopen', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openPage();
    try {
      await reopenAfterEscape(tab, '#open-raw');
      const now = await focused(tab);
      expect(now.id).toBe('raw-first');
      expect(now.focusVisible, 'the click changed no focus, so the keyboard modality stood').toBe(
        true
      );
    } finally {
      await tab.close();
    }
  });

  it('Reset data reopened with the mouse after Escape rests on its box, and nothing is ringed', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openPage();
    try {
      await reopenAfterEscape(tab, '#open-reset');
      const now = await focused(tab);
      expect(now.ringedInside, 'no control in the dialog matches :focus-visible').toEqual([]);
      expect(now.role).toBe('dialog');
      expect(now.outline, 'the box draws no ring').toBe('none');

      // A mouse click on "Select all" then leaves it unringed as it turns into "Clear".
      await tab.click('#toggle-all');
      const clicked = await focused(tab);
      expect(clicked.id).toBe('toggle-all');
      expect(await tab.textContent('#toggle-all')).toBe('Clear');
      expect(clicked.focusVisible).toBe(false);
      expect(clicked.ringedInside).toEqual([]);
    } finally {
      await tab.close();
    }
  });

  it('a first mouse open rests on the box too, and Tab reaches the first control, ringed', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openPage();
    try {
      await tab.click('#open-reset');
      await tab.waitForSelector(dialog);
      await settle(tab);
      const now = await focused(tab);
      expect(now.role).toBe('dialog');
      expect(now.ringedInside).toEqual([]);
      await tab.keyboard.press('Tab');
      const next = await focused(tab);
      expect(next.id).toBe('toggle-all');
      expect(next.focusVisible).toBe(true);
    } finally {
      await tab.close();
    }
  });

  it('a key open keeps Radix’s first stop, ringed, and Escape gives focus back', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openPage();
    try {
      await tab.focus('#open-reset');
      await tab.keyboard.press('Enter');
      await tab.waitForSelector(dialog);
      await settle(tab);
      const now = await focused(tab);
      expect(now.id).toBe('toggle-all');
      expect(now.focusVisible).toBe(true);
      await tab.keyboard.press('Escape');
      await tab.waitForSelector(dialog, { state: 'detached' });
      await settle(tab);
      expect((await focused(tab)).id).toBe('open-reset');
    } finally {
      await tab.close();
    }
  });

  it('a text field that comes first keeps focus after a mouse open', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openPage();
    try {
      await reopenAfterEscape(tab, '#open-form');
      expect((await focused(tab)).id).toBe('form-input');
    } finally {
      await tab.close();
    }
  });

  it('a text field that is not the first stop does not hand focus to the button before it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openPage();
    try {
      await reopenAfterEscape(tab, '#open-late');
      const now = await focused(tab);
      expect(now.role).toBe('dialog');
      expect(now.ringedInside).toEqual([]);
    } finally {
      await tab.close();
    }
  });

  it('a Select’s typing input is not a text field: the box takes focus', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openPage();
    try {
      await reopenAfterEscape(tab, '#open-select');
      const now = await focused(tab);
      expect(now.role).toBe('dialog');
      expect(now.ringedInside).toEqual([]);
    } finally {
      await tab.close();
    }
  });

  it('a sheet reopened with the mouse after Escape rests on its box too', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openPage();
    try {
      await reopenAfterEscape(tab, '#open-sheet');
      const now = await focused(tab);
      expect(now.role).toBe('dialog');
      expect(now.outline).toBe('none');
      expect(now.ringedInside).toEqual([]);
    } finally {
      await tab.close();
    }
  });
});
