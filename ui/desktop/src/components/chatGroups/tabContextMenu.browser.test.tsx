// @vitest-environment node
/**
 * A chat tab's right-click menu closes tabs (vision V2-R6-3).
 *
 * The menu offered "Open in new tab", which only selected the tab under the pointer (20 tabs before
 * and after), and no way to close one. A tab is already the open tab, so the item goes, and "Close
 * tab" and "Close other tabs" take its place beside Rename, Open in new window and Copy chat ID.
 *
 * jsdom lays nothing out, has no `:focus-visible` and runs no Tailwind, so the real strip is bundled
 * with esbuild around the real reducer (and keyed on the active tab, which is how the shell mounts
 * it: a tab switch remounts the strip), `styles/main.css` is compiled with the classes the bundle
 * uses, and Chromium opens the menu. What it holds that jsdom cannot: the menu sits whole in the
 * window with a hairline between its groups, a pointer choice leaves no keyboard ring on a tab the
 * person only clicked, and a keyboard close puts focus (and its ring) on the tab that takes over.
 *
 * Set `TAB_MENU_SHOTS=<dir>` to write the screenshots the vision pass reads.
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
const SHOTS = process.env.TAB_MENU_SHOTS;

/**
 * The strip as ChatGroupsShell mounts it: tabs from the real reducer, keyed on the active tab so a
 * tab switch remounts it, `window.__TABS` tabs and the last one active. `window.__tabs()` reads
 * the titles and the active one back.
 */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ChatTabStrip } from './ChatTabStrip';
import { chatGroupsReducer, createInitialChatGroupsState } from './chatGroupsReducer';

const w = window as unknown as Record<string, unknown>;
const count = (w.__TABS as number) ?? 6;
const initial = () => {
  let state = createInitialChatGroupsState();
  for (let i = 0; i < count; i++) {
    state = chatGroupsReducer(state, {
      type: 'openTab',
      payload: { sessionId: 's' + i, title: 'Chat ' + i },
    });
  }
  return state;
};

function Harness() {
  const [state, dispatch] = React.useReducer(chatGroupsReducer, undefined, initial);
  const group = state.groups['grp-1'];
  w.__tabs = () => ({
    titles: group.tabs.map((t) => t.title),
    active: group.tabs.find((t) => t.tabId === group.activeTabId)?.title ?? null,
  });
  return (
    <div id="band" style={{ display: 'flex', height: 44, width: 900 }}>
      <ChatTabStrip
        key={group.activeTabId ?? 'none'}
        tabs={group.tabs}
        activeTabId={group.activeTabId}
        groupId="grp-1"
        runningSessionIds={[]}
        onSelect={(tabId) => dispatch({ type: 'activateTab', tabId })}
        onClose={(tabId) => dispatch({ type: 'closeTab', tabId })}
        onCloseOthers={(tabId) => dispatch({ type: 'closeOtherTabs', tabId })}
        onReorder={() => {}}
        reserveTitlebar={false}
        isCompactSidebarOverlayOpen={false}
      />
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<Harness />);
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

const STRIP = '[role="tablist"]';
const MENU = '[role="menu"]';

describe('a chat tab’s right-click menu closes tabs (V2-R6-3)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the tab menu tests.');
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
      compiler.build(
        new Scanner({}).scanFiles([
          { content: script, extension: 'js' },
          {
            content: readFileSync(resolve(here, '../ui/context-menu.tsx'), 'utf8'),
            extension: 'tsx',
          },
        ])
      ) +
      '\n' +
      readFileSync(resolve(here, 'chat-tabs.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(tabs: number, scheme: 'light' | 'dark' = 'light'): Promise<Page> {
    const page = await browser!.newPage({ viewport: { width: 1000, height: 420 } });
    await page.emulateMedia({ reducedMotion: 'reduce', colorScheme: scheme });
    await page.setContent(
      `<!doctype html><html${scheme === 'dark' ? ' class="dark"' : ''}><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await page.addScriptTag({ content: `window.__TABS = ${tabs};` });
    await page.addScriptTag({ content: script });
    await page.waitForSelector(`${STRIP} [data-tab-id]`);
    await settle(page);
    return page;
  }

  /** Two frames and a pause: whatever a render or a menu schedules has run. */
  const settle = (page: Page, ms = 120) =>
    page.evaluate(
      (wait) =>
        new Promise<void>((done) =>
          requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(done, wait)))
        ),
      ms
    );

  const tabButton = (title: string) => `${STRIP} [role="tab"]:has-text("${title}")`;

  const state = (page: Page) =>
    page.evaluate(() =>
      (window as unknown as { __tabs: () => { titles: string[]; active: string | null } }).__tabs()
    );

  const items = (page: Page) =>
    page.evaluate(
      (m) =>
        [...document.querySelectorAll(`${m} [role="menuitem"]`)].map((el) =>
          (el.textContent ?? '').trim()
        ),
      MENU
    );

  /** The focused element, and whether the browser would draw a keyboard ring on it. */
  const focus = (page: Page) =>
    page.evaluate(() => {
      const el = document.activeElement as HTMLElement | null;
      return {
        tag: el?.tagName ?? null,
        role: el?.getAttribute('role') ?? null,
        label: el?.textContent?.trim() ?? null,
        ring: el ? el.matches(':focus-visible') : false,
      };
    });

  /**
   * Arrow down the open menu to the item whose text is `label`. A frame passes between presses:
   * Radix moves focus from the render a key schedules, and a press sent inside it is lost.
   */
  const arrowTo = async (page: Page, label: string) => {
    for (let i = 0; i < 8; i++) {
      const at = await page.evaluate(() => document.activeElement?.textContent?.trim() ?? '');
      if (at === label) return;
      await page.keyboard.press('ArrowDown');
      await settle(page, 30);
    }
    expect(await page.evaluate(() => document.activeElement?.textContent?.trim())).toBe(label);
  };

  const shot = async (page: Page, name: string) => {
    if (!SHOTS) return;
    await page.screenshot({ path: resolve(SHOTS, `${name}.png`) });
  };

  it('closes tabs, opens nothing, and keeps Rename, Open in new window and Copy chat ID', async () => {
    if (!browser) return;
    const page = await open(6);
    await page.click(tabButton('Chat 2'), { button: 'right' });
    await page.waitForSelector(MENU);
    await settle(page);

    expect(await items(page)).toEqual([
      'RenameF2',
      'Open in new window',
      'Copy chat ID',
      'Close tab',
      'Close other tabs',
    ]);

    // Whole in the window, under the band, with a hairline between its four groups.
    const box = await page.evaluate((m) => {
      const r = document.querySelector(m)!.getBoundingClientRect();
      return { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
    }, MENU);
    expect(box.left).toBeGreaterThanOrEqual(0);
    expect(box.right).toBeLessThanOrEqual(1000);
    expect(box.bottom).toBeLessThanOrEqual(420);
    expect(await page.locator(`${MENU} [role="separator"]`).count()).toBe(3);
    await shot(page, 'tab-menu-light');
    await page.close();
  }, 60_000);

  it('Close tab closes the right-clicked tab and leaves the one in front, with no ring', async () => {
    if (!browser) return;
    const page = await open(6);
    expect((await state(page)).active).toBe('Chat 5');

    await page.click(tabButton('Chat 2'), { button: 'right' });
    await page.waitForSelector(MENU);
    await page.click(`${MENU} [role="menuitem"]:has-text("Close tab")`);
    await page.waitForSelector(MENU, { state: 'detached' });
    await settle(page);

    const after = await state(page);
    expect(after.titles).toEqual(['Chat 0', 'Chat 1', 'Chat 3', 'Chat 4', 'Chat 5']);
    expect(after.active).toBe('Chat 5');
    // Nothing in the strip is ringed: a click is not a keyboard visit.
    expect((await focus(page)).ring).toBe(false);
    await page.close();
  }, 60_000);

  it('Close other tabs leaves the right-clicked tab alone, in front, with no ring', async () => {
    if (!browser) return;
    const page = await open(6);
    await page.click(tabButton('Chat 2'), { button: 'right' });
    await page.waitForSelector(MENU);
    await page.click(`${MENU} [role="menuitem"]:has-text("Close other tabs")`);
    await page.waitForSelector(MENU, { state: 'detached' });
    await settle(page);

    const after = await state(page);
    expect(after.titles).toEqual(['Chat 2']);
    expect(after.active).toBe('Chat 2');
    expect(await page.locator(`${STRIP} [data-tab-id]`).count()).toBe(1);
    expect((await focus(page)).ring).toBe(false);

    // A lone tab has no one to close beside it: the item is not offered.
    await page.click(tabButton('Chat 2'), { button: 'right' });
    await page.waitForSelector(MENU);
    expect(await items(page)).toEqual([
      'RenameF2',
      'Open in new window',
      'Copy chat ID',
      'Close tab',
    ]);
    await shot(page, 'tab-menu-lone-tab-light');
    await page.close();
  }, 60_000);

  /**
   * The tab in front stays mounted through a choice that changes nothing in the strip, so Radix
   * hands focus back to it. After a click that is a script focus, and Chromium draws the keyboard
   * ring on a tab the person only clicked; a click's choice returns no focus.
   */
  it('a pointer choice on the tab in front draws no ring on it', async () => {
    if (!browser) return;
    const page = await open(6);
    await page.focus(tabButton('Chat 5'));
    await page.click(tabButton('Chat 5'), { button: 'right' });
    await page.waitForSelector(MENU);
    await page.click(`${MENU} [role="menuitem"]:has-text("Open in new window")`);
    await page.waitForSelector(MENU, { state: 'detached' });
    await settle(page);

    expect((await state(page)).titles).toHaveLength(6);
    expect((await focus(page)).ring).toBe(false);
    await page.close();
  }, 60_000);

  it('a keyboard Close tab puts focus, and its ring, on the tab that takes over', async () => {
    if (!browser) return;
    const page = await open(6);
    // Chat 5 is in front. Focus its button, open its menu from the keyboard, choose Close tab.
    await page.focus(tabButton('Chat 5'));
    await page.keyboard.press('Shift+F10');
    await page.waitForSelector(MENU);
    await settle(page);
    await arrowTo(page, 'Close tab');
    await page.keyboard.press('Enter');
    await page.waitForSelector(MENU, { state: 'detached' });
    await settle(page);

    const after = await state(page);
    expect(after.titles).toEqual(['Chat 0', 'Chat 1', 'Chat 2', 'Chat 3', 'Chat 4']);
    expect(after.active).toBe('Chat 4');
    const at = await focus(page);
    expect(at).toMatchObject({ role: 'tab', ring: true });
    expect(at.label).toContain('Chat 4');
    await page.close();
  }, 60_000);

  it('a keyboard Close other tabs keeps focus on the tab that stays', async () => {
    if (!browser) return;
    const page = await open(6);
    await page.focus(tabButton('Chat 5'));
    await page.keyboard.press('Shift+F10');
    await page.waitForSelector(MENU);
    await settle(page);
    await arrowTo(page, 'Close other tabs');
    await page.keyboard.press('Enter');
    await page.waitForSelector(MENU, { state: 'detached' });
    await settle(page);

    expect((await state(page)).titles).toEqual(['Chat 5']);
    const at = await focus(page);
    expect(at).toMatchObject({ role: 'tab', ring: true });
    expect(at.label).toContain('Chat 5');
    await page.close();
  }, 60_000);

  it('draws in dark', async () => {
    if (!browser) return;
    const page = await open(6, 'dark');
    await page.click(tabButton('Chat 2'), { button: 'right' });
    await page.waitForSelector(MENU);
    await settle(page);
    await shot(page, 'tab-menu-dark');
    expect(await items(page)).toHaveLength(5);
    await page.close();
  }, 60_000);
});
