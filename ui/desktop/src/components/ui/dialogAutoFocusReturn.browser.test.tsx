// @vitest-environment node
/**
 * A dialog whose field takes React's `autoFocus` gives focus back to its opener (WS-SKILLS-2).
 *
 * React focuses an `autoFocus` field as the content commits, before Radix's focus scope mounts.
 * The scope then finds focus already inside and never fires `onOpenAutoFocus`, which is where
 * `useReturnFocusToOpener` read the opener, so Escape left focus on `<body>` and a keyboard user
 * started again at the top of the page. The content now reads its opener when its ref attaches:
 * the control focus left to reach the field (`inputTrail.focusArrivedFrom`).
 *
 * jsdom has no Radix focus scope timing worth trusting and no real key events, so the real
 * `DialogContent`, `SheetContent`, `ModalShell` and `DropdownMenu` are bundled with esbuild and
 * driven with Playwright's keyboard and mouse in Chromium. The first case is the instrument: Radix's
 * own content with the same field leaves focus on `<body>`.
 */
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));

const PAGE_CSS = `
  body { margin: 0; font: 14px sans-serif; }
  [role=menuitem] { display: block; padding: 6px 12px; outline: none; }
  [role=menu] { background: #fff; border: 1px solid #ccc; }
  [data-slot=dialog-content], [data-slot=sheet-content] { position: fixed; top: 40px; left: 40%; width: 420px; background: #fff; }
`;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import { Button } from './button';
import { Dialog, DialogContent, DialogTitle, DialogDescription } from './dialog';
import { Sheet, SheetContent, SheetTitle, SheetDescription } from './sheet';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from './dropdown-menu';
import { ModalShell } from '../ModalShell';

type Kind = null | 'raw' | 'dialog' | 'sheet' | 'shell' | 'select';

function App() {
  const [kind, setKind] = React.useState<Kind>(null);
  const change = (next: boolean) => !next && setKind(null);
  return (
    <div>
      <button id="open-raw" type="button" onClick={() => setKind('raw')}>Raw</button>
      <button id="open-dialog" type="button" onClick={() => setKind('dialog')}>Add skill</button>
      <button id="open-sheet" type="button" onClick={() => setKind('sheet')}>Sheet</button>
      <button id="open-shell" type="button" onClick={() => setKind('shell')}>Shell</button>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button aria-label="More">⋯</Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuItem data-testid="item-dialog" onSelect={() => setKind('dialog')}>Rename</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <button id="elsewhere" type="button">Elsewhere</button>

      <DialogPrimitive.Root open={kind === 'raw'} onOpenChange={change}>
        <DialogPrimitive.Portal>
          <DialogPrimitive.Content data-slot="dialog-content" aria-describedby={undefined}>
            <DialogPrimitive.Title>Raw</DialogPrimitive.Title>
            <input id="raw-field" aria-label="Raw field" autoFocus />
          </DialogPrimitive.Content>
        </DialogPrimitive.Portal>
      </DialogPrimitive.Root>

      <Dialog open={kind === 'dialog'} onOpenChange={change}>
        <DialogContent>
          <DialogTitle>Add a skill</DialogTitle>
          <DialogDescription>From a link.</DialogDescription>
          <input id="dialog-field" aria-label="Link" autoFocus />
          <button id="dialog-cancel" type="button" onClick={() => setKind(null)}>Cancel</button>
        </DialogContent>
      </Dialog>

      <Sheet open={kind === 'sheet'} onOpenChange={change}>
        <SheetContent>
          <SheetTitle>Sheet</SheetTitle>
          <SheetDescription>A side sheet.</SheetDescription>
          <input id="sheet-field" aria-label="Sheet field" autoFocus />
        </SheetContent>
      </Sheet>

      <ModalShell open={kind === 'shell'} onOpenChange={change} title="Write a skill"
        footer={<Button id="shell-ok" onClick={() => setKind(null)}>Save</Button>}>
        <textarea id="shell-field" aria-label="Instructions" autoFocus />
        <button id="shell-inner" type="button">Inner</button>
      </ModalShell>
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

describe('a dialog that opens on an autoFocus field (WS-SKILLS-2)', () => {
  let browser: Browser | null = null;
  let script = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the autoFocus dialog tests.');
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
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function openPage(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 600 } });
    tab.setDefaultTimeout(8000);
    await tab.setContent(
      `<!doctype html><html><head><style>${PAGE_CSS}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#open-dialog');
    return tab;
  }

  const active = (tab: Page) =>
    tab.evaluate(() => {
      const el = document.activeElement as HTMLElement;
      if (el === document.body) return 'BODY';
      // Radix ids its own triggers (`radix-…`); those are named by their label instead.
      const ownId = el.id && !el.id.startsWith('radix-') ? el.id : '';
      return ownId || el.getAttribute('aria-label') || el.tagName;
    });
  const dialog = '[role=dialog],[role=alertdialog]';
  /** Let Radix's exit animation and the focus scope's deferred restore finish. */
  const settle = (tab: Page) => tab.waitForTimeout(250);

  /** Tab onto `id` from the top of the page, then press Enter on it. */
  async function keyOpen(tab: Page, id: string) {
    await tab.focus(`#${id}`);
    await tab.keyboard.press('Enter');
    await tab.waitForSelector(dialog);
  }

  it('the instrument: Radix’s own content with an autoFocus field leaves focus on the body', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openPage();
    try {
      await keyOpen(tab, 'open-raw');
      expect(await active(tab)).toBe('raw-field');
      await tab.keyboard.press('Escape');
      await tab.waitForSelector(dialog, { state: 'detached' });
      await settle(tab);
      expect(await active(tab)).toBe('BODY');
    } finally {
      await tab.close();
    }
  });

  for (const [opener, field] of [
    ['open-dialog', 'dialog-field'],
    ['open-sheet', 'sheet-field'],
    ['open-shell', 'shell-field'],
  ] as const) {
    it(`${opener}: first focus on the field, and Escape gives focus back to the opener`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await openPage();
      try {
        await keyOpen(tab, opener);
        expect(await active(tab)).toBe(field);
        await tab.keyboard.press('Escape');
        await tab.waitForSelector(dialog, { state: 'detached' });
        await settle(tab);
        expect(await active(tab)).toBe(opener);
      } finally {
        await tab.close();
      }
    });
  }

  it('a keyboard Cancel and a pointer open give focus back too', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openPage();
    try {
      await keyOpen(tab, 'open-dialog');
      await tab.keyboard.press('Tab');
      expect(await active(tab)).toBe('dialog-cancel');
      await tab.keyboard.press('Enter');
      await tab.waitForSelector(dialog, { state: 'detached' });
      await settle(tab);
      expect(await active(tab)).toBe('open-dialog');

      await tab.click('#open-dialog');
      await tab.waitForSelector(dialog);
      expect(await active(tab)).toBe('dialog-field');
      await tab.keyboard.press('Escape');
      await tab.waitForSelector(dialog, { state: 'detached' });
      await settle(tab);
      expect(await active(tab)).toBe('open-dialog');
    } finally {
      await tab.close();
    }
  });

  it('a pick from a menu by key gives focus back to the menu’s trigger', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openPage();
    try {
      await tab.focus('[aria-label="More"]');
      await tab.keyboard.press('Enter');
      await tab.waitForSelector('[data-testid="item-dialog"]');
      await tab.keyboard.press('Enter');
      await tab.waitForSelector(dialog);
      await settle(tab);
      expect(await active(tab)).toBe('dialog-field');
      await tab.keyboard.press('Escape');
      await tab.waitForSelector(dialog, { state: 'detached' });
      await settle(tab);
      expect(await active(tab)).toBe('More');
    } finally {
      await tab.close();
    }
  });

  it('a move inside the open dialog does not become the opener', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openPage();
    try {
      await keyOpen(tab, 'open-shell');
      await tab.keyboard.press('Tab');
      expect(await active(tab)).toBe('shell-inner');
      await tab.keyboard.press('Escape');
      await tab.waitForSelector(dialog, { state: 'detached' });
      await settle(tab);
      expect(await active(tab)).toBe('open-shell');
    } finally {
      await tab.close();
    }
  });
});
