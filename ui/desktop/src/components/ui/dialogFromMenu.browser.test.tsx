// @vitest-environment node
/**
 * A dialog opened from a menu item (vision round 1, V4-03 and V4-04), measured with real input in
 * Chromium. jsdom has no `:focus-visible` and no menu focus trap, and both are the point.
 *
 * The real `DropdownMenu`, `RowContextMenu`, `ModalShell` and `ConfirmationModal` are bundled with
 * esbuild and driven with Playwright's keyboard and mouse:
 *
 * - **Close.** A menu item is gone by the time the dialog it opened closes, so a keyboard user
 *   (⋯, Enter, Enter on the item, Escape) used to land on `<body>`. They land on the control that
 *   opened the menu: the ⋯ for a DropdownMenu, found from its `aria-labelledby` (or `aria-controls`
 *   when a caller's own `id` replaced Radix's), and the row for a ContextMenu, which names nothing.
 *   The close decides, as the menus' own rule says: a key close gives focus back however the item
 *   was picked (V1-R8-1), a pointer close gives nothing back, and a ContextMenu pointer pick, whose
 *   opener is only the last focus, gives nothing back either.
 * - **Open.** After a pointer pick every script focus matches `:focus-visible`, so the first stop of
 *   a dialog with no text field wore the inset ring after a mouse click. Focus goes to the dialog
 *   box, which draws no ring; a text field keeps focus, and a caller that places focus itself is
 *   left alone. Any pointer open now gets the same quiet open, from a menu or not (V4-R5-1,
 *   measured in `dialogPointerOpen.browser.test.tsx`).
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');

/**
 * To see these cases fail on another version of the dialog primitive, point `DIALOG_SOURCE` at
 * that `dialog.tsx`: it is loaded wherever the bundle imports `./dialog` or `./ui/dialog`, its own
 * imports resolved from this directory.
 */
const dialogSourcePlugin: Plugin = {
  name: 'dialog-source',
  setup(b) {
    const source = process.env.DIALOG_SOURCE;
    if (!source) return;
    b.onResolve({ filter: /^\.\/(ui\/)?dialog$/ }, () => ({
      path: source,
      namespace: 'dialog-source',
    }));
    b.onLoad({ filter: /.*/, namespace: 'dialog-source' }, (args) => ({
      contents: readFileSync(args.path, 'utf8'),
      resolveDir: here,
      loader: 'tsx',
    }));
  },
};

/** The authored `.biorouter-modal-surface:focus` rule, read from main.css. */
function boxFocusRule(): string {
  const match = MAIN_CSS.match(
    /\/\* @ws WS-PRIMITIVES begin: dialog box focus \*\/([\s\S]*?)\/\* @ws WS-PRIMITIVES end: dialog box focus \*\//
  );
  if (!match) throw new Error('main.css has no "dialog box focus" block');
  return match[1];
}

/**
 * Stands in for the global focus rules: a ring on anything `:focus-visible`, which the authored
 * rule must take back from the dialog box and from nothing else.
 */
const PAGE_CSS = `
  body { margin: 0; font: 14px sans-serif; }
  [role=menuitem] { display: block; padding: 6px 12px; outline: none; }
  [role=menu] { background: #fff; border: 1px solid #ccc; }
  *:focus-visible { outline: 2px solid red; }
  [data-slot=dialog-content] { position: fixed; top: 40px; left: 50%; width: 420px; background: #fff; }
`;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Button } from './button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from './dropdown-menu';
import { RowContextMenu } from './row-actions';
import { ModalShell } from '../ModalShell';
import { ConfirmationModal } from './ConfirmationModal';

type Kind = 'info' | 'form' | 'confirm' | 'custom' | 'callerClose';

function Menu({ triggerId, label }: { triggerId?: string; label: string }) {
  const open = (window as unknown as { openDialog: (kind: Kind) => void }).openDialog;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button id={triggerId} aria-label={label}>{label}</Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent>
        <DropdownMenuItem data-testid="item-info" onSelect={() => open('info')}>View info</DropdownMenuItem>
        <DropdownMenuItem data-testid="item-form" onSelect={() => open('form')}>Rename</DropdownMenuItem>
        <DropdownMenuItem data-testid="item-confirm" onSelect={() => open('confirm')}>Delete</DropdownMenuItem>
        <DropdownMenuItem data-testid="item-custom" onSelect={() => open('custom')}>Custom</DropdownMenuItem>
        <DropdownMenuItem data-testid="item-callerClose" onSelect={() => open('callerClose')}>Caller</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function App() {
  const [kind, setKind] = React.useState<null | Kind>(null);
  (window as unknown as { openDialog: (kind: Kind) => void }).openDialog = setKind;
  const close = () => setKind(null);
  const change = (next: boolean) => !next && close();
  return (
    <div>
      <p><Menu label="Labelled menu" /></p>
      <p><Menu label="Own id menu" triggerId="own-id" /></p>
      <RowContextMenu
        items={[
          { label: 'View info', onSelect: () => setKind('info'), testId: 'ctx-info' },
          { label: 'Delete', onSelect: () => setKind('confirm'), testId: 'ctx-confirm' },
        ]}
      >
        <div id="row" tabIndex={0} style={{ width: 300, height: 40, border: '1px solid #999' }}>Row</div>
      </RowContextMenu>
      <RowContextMenu
        items={[{ label: 'Delete', onSelect: () => setKind('confirm'), testId: 'stats-confirm' }]}
      >
        <div id="stats" style={{ width: 300, height: 40, border: '1px solid #999' }}>Stats, takes no focus</div>
      </RowContextMenu>
      <button id="plain" type="button" onClick={() => setKind('info')}>Plain opener</button>
      <button id="elsewhere" type="button">Elsewhere</button>
      <ModalShell open={kind === 'info'} onOpenChange={change} title="Info"
        footer={<Button id="info-ok" variant="secondary" onClick={close}>Done</Button>}>
        <a id="info-link" href="https://example.com">Official URL</a>
      </ModalShell>
      <ModalShell open={kind === 'form'} onOpenChange={change} title="Rename"
        footer={<Button id="form-ok" onClick={close}>Save</Button>}>
        <input id="form-input" aria-label="Name" defaultValue="Chat" />
      </ModalShell>
      <ModalShell open={kind === 'custom'} onOpenChange={change} title="Custom"
        onOpenAutoFocus={(event) => { event.preventDefault(); document.getElementById('custom-target')?.focus(); }}
        footer={<Button id="custom-ok" onClick={close}>Done</Button>}>
        <div id="custom-target" tabIndex={-1}>The body</div>
      </ModalShell>
      <ConfirmationModal isOpen={kind === 'confirm'} title="Delete chat?" message="This cannot be undone."
        confirmLabel="Delete" cancelLabel="Cancel" onConfirm={close} onCancel={close} />
      <ConfirmationModal isOpen={kind === 'callerClose'} title="Caller decides" message="Its own return."
        confirmLabel="Yes" cancelLabel="No" onConfirm={close} onCancel={close}
        onCloseAutoFocus={(event) => { event.preventDefault(); document.getElementById('elsewhere')?.focus(); }} />
    </div>
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

interface Focus {
  tag: string;
  id: string;
  role: string;
  label: string;
  focusVisible: boolean;
  outline: string;
}

describe('a dialog opened from a menu item (V4-03, V4-04)', () => {
  let browser: Browser | null = null;
  let script = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the dialog-from-menu tests.');
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
      plugins: [dialogSourcePlugin],
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function openPage(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 800, height: 600 } });
    tab.setDefaultTimeout(8000);
    await tab.setContent(
      `<!doctype html><html><head><style>${PAGE_CSS}${boxFocusRule()}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[aria-label="Labelled menu"]');
    return tab;
  }

  const focused = (tab: Page): Promise<Focus> =>
    tab.evaluate(() => {
      const el = document.activeElement as HTMLElement;
      return {
        tag: el.tagName.toLowerCase(),
        id: el.id,
        role: el.getAttribute('role') ?? '',
        label: el.getAttribute('aria-label') ?? '',
        focusVisible: el.matches(':focus-visible'),
        outline: getComputedStyle(el).outlineStyle,
      };
    });

  const dialog = '[role=dialog],[role=alertdialog]';
  const closed = (tab: Page) => tab.waitForSelector(dialog, { state: 'detached' });
  /** Let Radix's exit animation, and the focus scope's deferred restore, finish. */
  const settle = (tab: Page) => tab.waitForTimeout(250);

  /** ⋯ with the keyboard, then the item with the keyboard. */
  async function keyPick(tab: Page, trigger: string, item: string) {
    await tab.focus(trigger);
    await tab.keyboard.press('Enter');
    const row = tab.locator(`[data-testid=${item}]`);
    await row.waitFor();
    await row.focus();
    await tab.keyboard.press('Enter');
    await tab.waitForSelector(dialog);
    await settle(tab);
  }

  /** ⋯ with the mouse, then the item with the mouse. */
  async function mousePick(tab: Page, trigger: string, item: string) {
    await tab.click(trigger);
    await tab.click(`[data-testid=${item}]`);
    await tab.waitForSelector(dialog);
    await settle(tab);
  }

  describe('closing it (V4-03)', () => {
    for (const [name, trigger] of [
      ['a labelled DropdownMenu trigger', '[aria-label="Labelled menu"]'],
      ['a trigger whose own id replaced Radix’s', '#own-id'],
    ] as const) {
      for (const item of ['item-info', 'item-confirm', 'item-form']) {
        it(`gives focus back to ${name} after ${item} and Escape`, async (ctx) => {
          if (!browser) return ctx.skip();
          const tab = await openPage();
          try {
            await keyPick(tab, trigger, item);
            await tab.keyboard.press('Escape');
            await closed(tab);
            await settle(tab);
            const now = await focused(tab);
            expect(now.tag, 'focus is on the ⋯, not <body>').toBe('button');
            expect(now.label).toBe(trigger === '#own-id' ? 'Own id menu' : 'Labelled menu');
          } finally {
            await tab.close();
          }
        });
      }
    }

    it('gives focus back to the row a ContextMenu was opened on (Shift+F10)', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openPage();
      try {
        await tab.focus('#row');
        await tab.keyboard.press('Shift+F10');
        const item = tab.locator('[data-testid=ctx-confirm]');
        await item.waitFor();
        await item.focus();
        await tab.keyboard.press('Enter');
        await tab.waitForSelector(dialog);
        await settle(tab);
        await tab.keyboard.press('Escape');
        await closed(tab);
        await settle(tab);
        expect((await focused(tab)).id).toBe('row');
      } finally {
        await tab.close();
      }
    });

    // V1-R8-1: a ⋯ clicked open, an item clicked, then Escape left focus on <body>, and the next Tab
    // began at the top of the window. The close is what decides, as for the menu itself.
    for (const [name, trigger] of [
      ['a labelled DropdownMenu trigger', '[aria-label="Labelled menu"]'],
      ['a trigger whose own id replaced Radix’s', '#own-id'],
    ] as const) {
      for (const item of ['item-info', 'item-confirm', 'item-form']) {
        it(`gives focus back to ${name} after a pointer pick of ${item} and Escape, ringed`, async (ctx) => {
          if (!browser) return ctx.skip();
          const tab = await openPage();
          try {
            await mousePick(tab, trigger, item);
            await tab.keyboard.press('Escape');
            await closed(tab);
            await settle(tab);
            const now = await focused(tab);
            expect(now.tag, 'focus is on the ⋯, not <body>').toBe('button');
            expect(now.label).toBe(trigger === '#own-id' ? 'Own id menu' : 'Labelled menu');
            // A key closed it, so the returned focus is keyboard focus.
            expect(now.focusVisible).toBe(true);
          } finally {
            await tab.close();
          }
        });
      }
    }

    it('gives focus back after a pointer pick and Enter on Cancel', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openPage();
      try {
        await mousePick(tab, '[aria-label="Labelled menu"]', 'item-confirm');
        await tab.getByRole('button', { name: 'Cancel' }).focus();
        await tab.keyboard.press('Enter');
        await closed(tab);
        await settle(tab);
        expect((await focused(tab)).label).toBe('Labelled menu');
      } finally {
        await tab.close();
      }
    });

    it('gives nothing back after a pointer pick closed with the pointer: no ring on a button the person clicked', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openPage();
      try {
        await mousePick(tab, '[aria-label="Labelled menu"]', 'item-confirm');
        await tab.getByRole('button', { name: 'Cancel' }).click();
        await closed(tab);
        await settle(tab);
        expect((await focused(tab)).tag).toBe('body');
      } finally {
        await tab.close();
      }
    });

    it('gives nothing back to a ContextMenu’s guessed opener after a pointer pick and Escape', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openPage();
      try {
        // Focus sits on another control; a right-click on a part of a row that takes no focus
        // moves none, so the only opener the menu could name is that unrelated control.
        await tab.focus('#elsewhere');
        await tab.click('#stats', { button: 'right' });
        await tab.click('[data-testid=stats-confirm]');
        await tab.waitForSelector(dialog);
        await settle(tab);
        await tab.keyboard.press('Escape');
        await closed(tab);
        await settle(tab);
        expect((await focused(tab)).id, 'not sent to a control the person was not on').not.toBe(
          'elsewhere'
        );
      } finally {
        await tab.close();
      }
    });

    it('gives nothing back when a keyboard pick is closed with the mouse', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openPage();
      try {
        await keyPick(tab, '[aria-label="Labelled menu"]', 'item-info');
        await tab.click('#info-ok');
        await closed(tab);
        await settle(tab);
        expect((await focused(tab)).tag).toBe('body');
      } finally {
        await tab.close();
      }
    });

    it('leaves the choice to a caller whose onCloseAutoFocus cancels the return', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openPage();
      try {
        await keyPick(tab, '[aria-label="Labelled menu"]', 'item-callerClose');
        await tab.keyboard.press('Escape');
        await closed(tab);
        await settle(tab);
        expect((await focused(tab)).id).toBe('elsewhere');
      } finally {
        await tab.close();
      }
    });
  });

  describe('opening it (V4-04)', () => {
    for (const item of ['item-info', 'item-confirm']) {
      it(`puts focus on the dialog box, drawing no ring, after a mouse pick of ${item}`, async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await openPage();
        try {
          await mousePick(tab, '[aria-label="Labelled menu"]', item);
          const now = await focused(tab);
          expect(now.role).toBe('dialog');
          expect(
            now.outline,
            'the box draws no ring even where Chromium calls it focus-visible'
          ).toBe('none');
          // Tab goes on to the first control, which is what the box stood in for.
          await tab.keyboard.press('Tab');
          const next = await focused(tab);
          expect(next.tag === 'button' || next.tag === 'a').toBe(true);
          expect(await tab.evaluate(() => !!document.activeElement?.closest('[role=dialog]'))).toBe(
            true
          );
        } finally {
          await tab.close();
        }
      });
    }

    it('does the same for a ContextMenu pick with the mouse', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openPage();
      try {
        await tab.click('#row', { button: 'right' });
        await tab.click('[data-testid=ctx-confirm]');
        await tab.waitForSelector(dialog);
        await settle(tab);
        const now = await focused(tab);
        expect(now.role === 'alertdialog' || now.role === 'dialog').toBe(true);
        expect(now.outline).toBe('none');
      } finally {
        await tab.close();
      }
    });

    it('keeps a text field as the first stop: a field is ringed in every modality', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openPage();
      try {
        await mousePick(tab, '[aria-label="Labelled menu"]', 'item-form');
        expect((await focused(tab)).id).toBe('form-input');
      } finally {
        await tab.close();
      }
    });

    for (const [how, pick] of [
      ['mouse', mousePick],
      ['keyboard', keyPick],
    ] as const) {
      it(`leaves a caller that places focus itself alone, after a ${how} pick (the menu’s trap does not take it back)`, async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await openPage();
        try {
          await pick(tab, '[aria-label="Labelled menu"]', 'item-custom');
          expect((await focused(tab)).id).toBe('custom-target');
        } finally {
          await tab.close();
        }
      });
    }

    it('does not touch a dialog a keyboard pick opened: Radix’s first stop, as before', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openPage();
      try {
        await keyPick(tab, '[aria-label="Labelled menu"]', 'item-info');
        const now = await focused(tab);
        expect(now.role).not.toBe('dialog');
        expect(now.tag).toBe('button');
      } finally {
        await tab.close();
      }
    });

    it('treats a dialog no menu opened the same way: a mouse open rests on the box (V4-R5-1)', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openPage();
      try {
        await tab.click('#plain');
        await tab.waitForSelector(dialog);
        await settle(tab);
        const now = await focused(tab);
        expect(now.role).toBe('dialog');
        expect(now.outline).toBe('none');
        await tab.keyboard.press('Escape');
        await closed(tab);
        await settle(tab);
        expect((await focused(tab)).id, 'the opener it always returned to').toBe('plain');
      } finally {
        await tab.close();
      }
    });
  });
});
