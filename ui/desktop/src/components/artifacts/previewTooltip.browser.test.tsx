// @vitest-environment node
/**
 * The panel's tooltips against real pointer input (V2-R8-4).
 *
 * "Close preview panel" faded in about 100ms AFTER the press on Close and sat alone at the top
 * right, over the space the panel had left, until the panel unmounted (measured again in the
 * artifact harness's panel mode with the app's shared tooltip provider: Close's tooltip
 * `instant-open` 15ms after the click, while `data-panel-open` was already false). jsdom has no
 * shared-provider timing and no pointer events from a real device, so the real
 * `PreviewIconButton` is bundled with esbuild and driven in Chromium, inside the app's own
 * `AppTooltipProvider` (the 300ms skip-delay between neighbours is what opens a tooltip at once).
 *
 * The page holds the header's three controls in its order and its 2px gap, "Open externally",
 * "More actions" and "Close preview panel". Close's press turns `PreviewPanelPresence` false and
 * leaves all three mounted, as the panel stays mounted for its exit, so a tooltip opening
 * during it is observable. "More actions" stands for a menu trigger: its own pointerdown is
 * default-prevented, as a Radix dropdown trigger's is, so the control's handlers and the tooltip's
 * compose the way they do in the header.
 *
 * Nothing here needs the app's CSS: a tooltip is judged by its `data-state` in the DOM, and the
 * content unmounts as soon as it closes because no stylesheet gives it an exit animation.
 */
import { chromium, type Browser, type Page } from 'playwright';
import { build } from 'esbuild';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));

const ENTRY = `
import * as React from 'react';
import { flushSync } from 'react-dom';
import { createRoot } from 'react-dom/client';
import { AppTooltipProvider } from '../ui/Tooltip';
import { PreviewIconButton, PreviewPanelPresence } from './PreviewBar';

let setPanelOpen: (open: boolean) => void = () => {};
(window as unknown as { __setOpen: (open: boolean) => void }).__setOpen = (open) =>
  flushSync(() => setPanelOpen(open));

function App() {
  const [open, setOpen] = React.useState(true);
  setPanelOpen = setOpen;
  return (
    <AppTooltipProvider>
      <PreviewPanelPresence value={open}>
        <div style={{ display: 'flex', gap: 2, padding: 40, justifyContent: 'flex-end' }}>
          <PreviewIconButton label="Open externally">A</PreviewIconButton>
          <PreviewIconButton label="More actions" onPointerDown={(event) => event.preventDefault()}>
            M
          </PreviewIconButton>
          <PreviewIconButton label="Close preview panel" onClick={() => setOpen(false)}>
            B
          </PreviewIconButton>
        </div>
      </PreviewPanelPresence>
      <p style={{ marginTop: 120 }}>Nothing here.</p>
    </AppTooltipProvider>
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

const sleep = (ms: number) => new Promise((done) => setTimeout(done, ms));

/** The centre of the control named `name`. */
async function centre(page: Page, name: string) {
  const box = await page.getByRole('button', { name }).boundingBox();
  if (!box) throw new Error(`no ${name} button`);
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
}

/** Every tooltip content the page shows from now on, with the state it was first seen in. */
async function startRecording(page: Page) {
  await page.evaluate(() => {
    const seen: { label: string; state: string }[] = [];
    const note = () => {
      for (const node of document.querySelectorAll('[data-slot="tooltip-content"]')) {
        const label = (node.textContent ?? '').trim();
        const state = node.getAttribute('data-state') ?? '';
        if (!seen.some((entry) => entry.label === label && entry.state === state)) {
          seen.push({ label, state });
        }
      }
    };
    new MutationObserver(note).observe(document.body, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ['data-state'],
    });
    (window as unknown as { __seen: typeof seen }).__seen = seen;
  });
}

const seen = (page: Page) =>
  page.evaluate(() => (window as unknown as { __seen: { label: string; state: string }[] }).__seen);

const forgetSeen = (page: Page) =>
  page.evaluate(() => {
    (window as unknown as { __seen: unknown[] }).__seen.length = 0;
  });

/** How many tooltips are showing right now, opening or open (not the closed state they leave in). */
const showing = (page: Page) =>
  page
    .locator(
      '[data-slot="tooltip-content"]:is([data-state="delayed-open"], [data-state="instant-open"])'
    )
    .count();

/** A tooltip that was showing: opening or open, never the closed state it leaves in. */
const OPEN = /^(delayed-open|instant-open)$/;
const opened = (entries: { label: string; state: string }[], label: string) =>
  entries.some((entry) => entry.label.startsWith(label) && OPEN.test(entry.state));

describe('the panel’s tooltips (V2-R8-4)', () => {
  let browser: Browser | null = null;
  let script = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the tooltip tests.'
      );
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      loader: { '.css': 'empty' },
      define: { 'process.env.NODE_ENV': '"production"' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const page = await browser!.newPage({ viewport: { width: 600, height: 300 } });
    await page.setContent(
      `<!doctype html><html><head><style>
        body { margin: 0; font: 14px Arial, sans-serif; }
        button { width: 32px; height: 32px; padding: 0; }
      </style></head><body><div id="root"></div></body></html>`
    );
    await page.addScriptTag({ content: script });
    await page.waitForSelector('button');
    await startRecording(page);
    return page;
  }

  /** The pointer onto `name` in `steps` moves, ten milliseconds apart, from where it last was. */
  async function glideTo(page: Page, name: string, steps: number) {
    const from = await page.evaluate(() => ({
      x: (window as unknown as { __x?: number }).__x ?? 0,
      y: (window as unknown as { __y?: number }).__y ?? 0,
    }));
    const to = await centre(page, name);
    for (let i = 1; i <= steps; i++) {
      await page.mouse.move(
        from.x + ((to.x - from.x) * i) / steps,
        from.y + ((to.y - from.y) * i) / steps
      );
      await sleep(10);
    }
    await page.evaluate((point) => Object.assign(window, { __x: point.x, __y: point.y }), to);
  }

  it('opens after the delay for a hover and for the keyboard, as before', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open();
    try {
      await glideTo(page, 'Open externally', 4);
      await sleep(700);
      expect(opened(await seen(page), 'Open externally')).toBe(true);

      await page.mouse.move(300, 250);
      await page.keyboard.press('Tab');
      await sleep(150);
      expect(opened(await seen(page), 'Open externally')).toBe(true);
      await page.keyboard.press('Tab');
      await page.keyboard.press('Tab');
      await sleep(150);
      expect(opened(await seen(page), 'Close preview panel')).toBe(true);
    } finally {
      await page.close();
    }
  });

  // The finding. Radix opens a tooltip on a move only for a control whose "a move opened me"
  // flag is unset, and the flag is unset for a control whose earlier moves were swallowed as "in
  // transit" to a neighbour's tooltip, which is where a pointer crossing a header's open
  // tooltips arrives. The first move after the click then asks for an instant open: the shared
  // provider is skipping the delay because a neighbour's tooltip closed with this very press.
  // The events are sent from the page, in the order a click and the move after it arrive,
  // because a real pointer sets the flag on its way in. The control is More actions, whose press
  // leaves the panel open: Close would pass for the wrong reason, the exit alone refusing the
  // open. A press holds the tooltip shut until the pointer leaves.
  it('opens no tooltip on the control just pressed, when a move follows a press that no move preceded', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open();
    try {
      await glideTo(page, 'Open externally', 4);
      await sleep(700);
      expect(opened(await seen(page), 'Open externally')).toBe(true);
      await forgetSeen(page);
      await page.evaluate(() => {
        const more = document.querySelector<HTMLElement>('[aria-label="More actions"]')!;
        const fire = (type: string) =>
          more.dispatchEvent(
            new PointerEvent(type, { bubbles: true, pointerType: 'mouse', isPrimary: true })
          );
        fire('pointerdown');
        fire('pointerup');
        more.dispatchEvent(new MouseEvent('click', { bubbles: true }));
        fire('pointermove');
      });
      await sleep(300);
      expect(opened(await seen(page), 'More actions')).toBe(false);
    } finally {
      await page.close();
    }
  });

  it('keeps a pressed control’s tooltip shut until the pointer leaves it, then offers it again', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open();
    try {
      // A does nothing when pressed (the panel stays), so only its tooltip reacts.
      await glideTo(page, 'Open externally', 4);
      await sleep(700);
      await page.mouse.down();
      await page.mouse.up();
      const a = await centre(page, 'Open externally');
      await forgetSeen(page);
      for (let i = 1; i <= 4; i++) {
        await sleep(200);
        await page.mouse.move(a.x + (i % 2), a.y);
      }
      expect(opened(await seen(page), 'Open externally')).toBe(false);

      // Away and back: the name is offered again.
      await page.mouse.move(300, 250);
      await sleep(400);
      await page.mouse.move(a.x, a.y, { steps: 3 });
      await sleep(700);
      expect(opened(await seen(page), 'Open externally')).toBe(true);
    } finally {
      await page.close();
    }
  });

  it('opens none while the panel is leaving, with or without a neighbour’s tooltip open', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open();
    try {
      await glideTo(page, 'Open externally', 4);
      await sleep(700);
      await page.evaluate(() =>
        (window as unknown as { __setOpen: (open: boolean) => void }).__setOpen(false)
      );
      await glideTo(page, 'Close preview panel', 10);
      await sleep(700);
      const close = await centre(page, 'Close preview panel');
      await page.mouse.move(close.x + 1, close.y);
      await sleep(300);
      expect(opened(await seen(page), 'Close preview panel')).toBe(false);
    } finally {
      await page.close();
    }
  });

  it('closes a tooltip that is open when the panel starts to leave', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open();
    try {
      await glideTo(page, 'Close preview panel', 4);
      await sleep(700);
      expect(await showing(page)).toBe(1);
      await page.evaluate(() =>
        (window as unknown as { __setOpen: (open: boolean) => void }).__setOpen(false)
      );
      await sleep(50);
      expect(await showing(page)).toBe(0);
    } finally {
      await page.close();
    }
  });

  // The panel can be opened again before its exit unmounts it (an artifact clicked while the
  // last one leaves), and then the same controls are back under a pointer that has not moved.
  it('does not bring a tooltip back when the panel opens again', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await open();
    try {
      await glideTo(page, 'Close preview panel', 4);
      await sleep(700);
      expect(await showing(page)).toBe(1);
      await page.evaluate(() =>
        (window as unknown as { __setOpen: (open: boolean) => void }).__setOpen(false)
      );
      await sleep(50);
      await page.evaluate(() =>
        (window as unknown as { __setOpen: (open: boolean) => void }).__setOpen(true)
      );
      await sleep(150);
      expect(await showing(page)).toBe(0);
    } finally {
      await page.close();
    }
  });
});
