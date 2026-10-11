// @vitest-environment node
/**
 * The app sidebar under the whole stylesheet, in a real layout engine: fills before lines (owner
 * message 13, R-27, design.md 9.11).
 *
 * `sidebarGeometry.browser.test.ts` measures the authored rows on stand-in markup. This file paints
 * the REAL `AppSidebar` (and `TitlebarControls`) bundled with esbuild, with `styles/main.css`
 * compiled by Tailwind from the classes the bundle uses plus the authored `sidebar.css`, in light
 * and dark, and asserts what the fill rules promise there:
 *
 * - the sidebar draws no line but its structural ones (the band's edge at y=44, the footer's rule,
 *   the pane divider): no row, header, group or control has a resting outline;
 * - rows rest clear and light with the sidebar's own fill steps;
 * - a trigger keeps its place while its menu is open (the ⋯ stays on its lit row, "View options"
 *   stays pressed on the strong fill), read by `aria-expanded`, because the tooltip wrapped around
 *   each trigger overwrites `data-state`;
 * - the rename editor's edge is a real border, so forced colours keep it;
 * - Crew's neutral count badge steps to the page ground on a lit row instead of vanishing into it;
 * - the titlebar controls hover on the strong fill;
 * - the column is a recessed ground (`.br-ground-recessed`): a filled control placed on it rests on
 *   the strong step, while a Note in the footer still steps its own buttons back to the page;
 * - a menu the pointer opened (a row's ⋯, View options) rings no row, and its first key brings the
 *   ring back (WS-PRIMITIVES-3, the fix for WS-SIDEBAR-2).
 *
 * Colours are compared with probes that paint the token itself, so a token value can change
 * without this file changing. Set `SIDEBAR_SHOTS_DIR` to also save light and dark screenshots.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { fontScale } from '../../../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const SHOTS = process.env.SIDEBAR_SHOTS_DIR;
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const ORIGIN = 'http://sidebar.test';

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { ChatProvider } from '../../contexts/ChatContext';
import {
  Sidebar, SidebarInset, SidebarMenu, SidebarMenuButton, SidebarMenuItem, SidebarProvider,
} from '../ui/sidebar';
import { Badge } from '../ui/badge';
import { TitlebarControls } from '../Layout/TitlebarControls';
import AppSidebar from './AppSidebar';

// The Crew row as AppSidebar draws it with an unread count, at rest and current, on the sidebar's
// ground. (The real count needs a live Crew connection; the row's markup and classes are the same.)
function CrewRows() {
  return (
    <div id="crew-rows" className="bg-sidebar" style={{ width: 288, padding: 8, marginTop: 120 }}>
      <SidebarMenu className="br-nav-list">
        {['rest', 'current'].map((key) => (
          <SidebarMenuItem key={key}>
            <SidebarMenuButton
              id={'crew-' + key}
              className="br-nav-row"
              isActive={key === 'current'}
              aria-current={key === 'current' ? 'page' : undefined}
            >
              <svg className="br-nav-row-icon" viewBox="0 0 16 16" />
              <span className="br-nav-row-label">Crew</span>
              <Badge id={'badge-' + key} tone="neutral" className="br-nav-row-badge tabular-nums">3</Badge>
            </SidebarMenuButton>
          </SidebarMenuItem>
        ))}
      </SidebarMenu>
    </div>
  );
}

function Shell() {
  const [chat, setChat] = React.useState<any>({ sessionId: 's2', name: 'Survival analysis', messages: [], workflow: null });
  return (
    <ChatProvider chat={chat} setChat={setChat}>
      <div style={{ height: '100vh', display: 'flex', width: '100%' }}>
        <Sidebar variant="inset" collapsible="offcanvas">
          <AppSidebar onSelectSession={() => {}} currentPath="/pair" />
        </Sidebar>
        <SidebarInset>
          <TitlebarControls hidden={false} isMacOS onNewWindow={() => {}} />
          <CrewRows />
        </SidebarInset>
      </div>
    </ChatProvider>
  );
}

createRoot(document.getElementById('root')!).render(
  <MemoryRouter initialEntries={['/pair?resumeSessionId=s2']}>
    <SidebarProvider>
      <Shell />
    </SidebarProvider>
  </MemoryRouter>
);
`;

/**
 * The window globals the real sidebar reads (the preload bridge and the app config). `daemon` is
 * what the main process says about the shared daemon: 'lost' shows the footer's notice.
 */
const stubs = (daemon: 'attached' | 'lost') => `
window.electron = {
  platform: 'darwin',
  getUserActionKey: async () => 'test-key',
  createChatWindow: () => {},
  copyConversationId: async () => {},
  onUpdaterEvent: () => () => {},
  getUpdateState: async () => null,
  getDaemonConnection: async () => '${daemon}',
  onDaemonConnection: () => () => {},
  on: () => {},
  off: () => {},
};
window.appConfig = { get: (key) => (key === 'BIOROUTER_HOME_DIR' ? '/Users/me' : undefined) };
`;

const HOUR = 3_600_000;
function sessions() {
  const now = Date.now();
  const row = (i: number, ago: number, name: string, extra: Record<string, unknown> = {}) => ({
    id: `s${i}`,
    name,
    created_at: new Date(now - ago - 2 * HOUR).toISOString(),
    updated_at: new Date(now - ago).toISOString(),
    working_dir: '/Users/me/projects/cohort',
    message_count: 6,
    user_set_name: true,
    privacy_tier: 'public',
    ...extra,
  });
  return [
    row(1, 0.2 * HOUR, 'Which connections in the knowledge graph lead to better care?', {
      privacy_tier: 'private',
    }),
    row(2, 1 * HOUR, 'Survival analysis'),
    row(3, 2 * HOUR, 'Plot cohort survival (branch 2)', { diverged_from: 's2' }),
    row(4, 3 * HOUR, 'Volcano plot'),
    row(5, 26 * HOUR, 'Refactor the OMOP query helper'),
    row(6, 27 * HOUR, 'Nightly dedupe', { session_type: 'scheduled' }),
    row(7, 80 * HOUR, 'Draft the methods section'),
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

describe('the app sidebar under the whole stylesheet (R-27)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the sidebar fill tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: {
        'process.env.NODE_ENV': '"production"',
        'process.env.ALPHA': 'false',
        'process.env.BIOROUTER_TUNNEL': 'true',
        // A production build: no development badge beside the wordmark.
        'import.meta.env': JSON.stringify({ MODE: 'production', DEV: false, PROD: true }),
      },
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
    // The app imports the authored sidebar stylesheet from AppSidebar and RecentChats; the bundle
    // drops CSS imports, so it is appended here exactly as the app's bundler appends it.
    css =
      compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }])) +
      '\n' +
      readFileSync(resolve(here, 'sidebar.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  let scaled: Promise<string> | null = null;
  const scaledCss = () =>
    (scaled ??= postcss([fontScale()])
      .process(css, { from: undefined })
      .then((result) => result.css));

  /**
   * `list` swaps the chat list the daemon serves (`serve` answers each `/sessions/sidebar` URL,
   * and may take its time), the window size, and a script run before the app's own (`init`), for
   * the cases that watch the list itself rather than its paint. `scale` sets the Text size
   * preference's `--app-font-scale` and serves the stylesheet through the app's own font-scale
   * plugin, as the app's bundler does.
   */
  async function open(
    mode: 'light' | 'dark',
    forcedColors = false,
    daemon: 'attached' | 'lost' = 'attached',
    list: { serve?: (url: string) => unknown; height?: number; init?: string; scale?: number } = {}
  ): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1200, height: list.height ?? 900 } });
    if (forcedColors) await tab.emulateMedia({ forcedColors: 'active' });
    if (list.init) await tab.addInitScript(list.init);
    const html =
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"` +
      (list.scale ? ` style="--app-font-scale: ${list.scale}"` : '') +
      `><head><style>${list.scale ? await scaledCss() : css}</style></head>` +
      `<body style="margin:0"><div id="root"></div>` +
      `<script>${stubs(daemon)}</script><script>${script.replace(/<\/script/gi, '<\\/script')}</script>` +
      `</body></html>`;
    await tab.route(`${ORIGIN}/**`, async (route) => {
      const url = route.request().url();
      if (url === `${ORIGIN}/`) {
        await route.fulfill({ status: 200, contentType: 'text/html', body: html });
      } else if (url.includes('/sessions/sidebar')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(
            (await list.serve?.(url)) ?? {
              sessions: sessions(),
              has_more: false,
              next_cursor: null,
            }
          ),
        });
      } else {
        await route.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
      }
    });
    await tab.goto(`${ORIGIN}/`);
    await tab.waitForSelector('[data-testid="recent-chat-s4"]', { timeout: 20_000 });
    // Let every colour transition settle before reading a colour.
    await tab.waitForTimeout(400);
    return tab;
  }

  /** The computed colour a probe inside `within` paints for `value` (a token, in that place). */
  const paint = (tab: Page, value: string, within = '[data-slot="sidebar-container"]') =>
    tab.evaluate(
      ([v, w]) => {
        const probe = document.createElement('div');
        probe.style.backgroundColor = v;
        document.querySelector(w)!.append(probe);
        const out = getComputedStyle(probe).backgroundColor;
        probe.remove();
        return out;
      },
      [value, within] as const
    );
  const ground = (tab: Page, selector: string) =>
    tab.evaluate((s) => getComputedStyle(document.querySelector(s)!).backgroundColor, selector);
  const settle = (tab: Page) => tab.waitForTimeout(250);
  const away = (tab: Page) => tab.mouse.move(1190, 890);

  /**
   * Every visible line the sidebar draws, by the element's test id or class. A line is a border
   * side with width and a colour that is not transparent, or a box-shadow.
   */
  const lines = (tab: Page) =>
    tab.evaluate(() => {
      const root = document.querySelector('[data-slot="sidebar-container"]')!;
      const clear = (c: string) => c === 'transparent' || /rgba\(.*,\s*0\)$/.test(c);
      const name = (el: Element) =>
        el.getAttribute('data-testid') ??
        el.getAttribute('data-slot') ??
        (el.getAttribute('class') ?? el.tagName).split(' ').slice(0, 2).join(' ');
      const out: string[] = [];
      for (const el of [root, ...root.querySelectorAll('*')]) {
        const cs = getComputedStyle(el);
        for (const side of ['Top', 'Right', 'Bottom', 'Left'] as const) {
          const width = parseFloat(cs.getPropertyValue(`border-${side.toLowerCase()}-width`));
          const style = cs.getPropertyValue(`border-${side.toLowerCase()}-style`);
          const color = cs.getPropertyValue(`border-${side.toLowerCase()}-color`);
          if (width > 0 && style !== 'none' && !clear(color)) out.push(`${name(el)}: ${side}`);
        }
        if (cs.boxShadow !== 'none') out.push(`${name(el)}: shadow`);
      }
      return out.sort();
    });

  it('draws only its structural lines: the band edge, the footer rule and the pane divider', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        expect(await lines(tab), `${mode}: the sidebar's lines`).toEqual([
          'sidebar-container: shadow', // the pane divider, `.biorouter-sidebar-shell`
          'sidebar-footer: Top',
          'sidebar-titlebar-band: Bottom',
        ]);
        // Hovering a row and the view button adds a fill, never a line.
        await tab.hover('[data-testid="recent-chat-s4"]');
        await settle(tab);
        await tab.hover('[data-testid="sidebar-view-options"]');
        await settle(tab);
        expect((await lines(tab)).length, `${mode}: hover draws no line`).toBe(3);
      } finally {
        await tab.close();
      }
    }
  });

  it('rests rows clear and lights them with the sidebar’s fill steps', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const clear = 'rgba(0, 0, 0, 0)';
        expect(await ground(tab, '[data-testid="recent-chat-s4"]')).toBe(clear);
        expect(await ground(tab, '[data-testid="recent-chat-s2"]'), `${mode} current row`).toBe(
          await paint(tab, 'var(--sidebar-active)')
        );
        await tab.hover('[data-testid="recent-chat-s4"]');
        await settle(tab);
        expect(await ground(tab, '[data-testid="recent-chat-s4"]'), `${mode} hovered row`).toBe(
          await paint(tab, 'var(--sidebar-hover)')
        );
      } finally {
        await tab.close();
      }
    }
  });

  it('keeps a row’s ⋯ on screen while its menu is open, and View options pressed', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        // The row's ⋯: the pointer leaves the row for the menu; the ⋯ must not vanish under it.
        await tab.hover('[data-testid="recent-chat-s4"]');
        await settle(tab);
        await tab.click('[data-testid="recent-chat-more-s4"]');
        await tab.getByRole('menuitem', { name: 'Diverge' }).hover();
        await settle(tab);
        const more = await tab.evaluate(() => {
          const button = document.querySelector('[data-testid="recent-chat-more-s4"]')!;
          const item = button.closest('.br-chat-row-item')!;
          return {
            expanded: button.getAttribute('aria-expanded'),
            opacity: getComputedStyle(button).opacity,
            trailing: getComputedStyle(item.querySelector('.br-chat-row-trailing')!).opacity,
          };
        });
        expect(more, `${mode}: ⋯ with its menu open`).toEqual({
          expanded: 'true',
          opacity: '1',
          trailing: '0',
        });
        expect(await ground(tab, '[data-testid="recent-chat-s4"]'), `${mode} lit row`).toBe(
          await paint(tab, 'var(--sidebar-hover)')
        );
        if (SHOTS) {
          await tab.screenshot({
            path: resolve(SHOTS, `sidebar-more-open-${mode}.png`),
            clip: { x: 0, y: 200, width: 520, height: 320 },
          });
        }
        await tab.keyboard.press('Escape');
        await settle(tab);

        // View options: pressed on the strong fill while its menu is open, clear once it closes.
        const view = '[data-testid="sidebar-view-options"]';
        await tab.click(view);
        await away(tab);
        await settle(tab);
        expect(await ground(tab, view), `${mode}: View options pressed`).toBe(
          await paint(tab, 'var(--background-fill-strong)')
        );
        // Closed with a key, focus comes back to it and shows (the D-15 focus fill)...
        await tab.keyboard.press('Escape');
        await tab.waitForTimeout(600);
        expect(await ground(tab, view), `${mode}: View options focused by a key close`).toBe(
          await paint(tab, 'var(--background-focus)')
        );
        // ...and once focus moves on, it rests clear again.
        await tab.mouse.click(1190, 890);
        await settle(tab);
        expect(await ground(tab, view), `${mode}: View options at rest`).toBe('rgba(0, 0, 0, 0)');
      } finally {
        await tab.close();
      }
    }
  });

  it('edges the rename editor with a real border on the page ground, kept in forced colours', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const forced of [false, true]) {
      const tab = await open('light', forced);
      try {
        await tab.focus('[data-testid="recent-chat-s4"]');
        await tab.keyboard.press('F2');
        const input = tab.getByRole('textbox', { name: /Rename chat/ });
        await input.waitFor();
        const editor = await input.evaluate((el) => {
          const cs = getComputedStyle(el);
          const row = el.closest('.br-chat-row')!.getBoundingClientRect();
          const box = el.getBoundingClientRect();
          return {
            width: cs.borderTopWidth,
            style: cs.borderTopStyle,
            color: cs.borderTopColor,
            shadow: cs.boxShadow,
            ground: cs.backgroundColor,
            textX: box.left + parseFloat(cs.borderLeftWidth) + parseFloat(cs.paddingLeft),
            height: box.height,
            inset: box.top - row.top,
          };
        });
        expect(editor.width).toBe('1px');
        expect(editor.style).toBe('solid');
        expect(editor.shadow).toBe('none');
        expect(editor.color).not.toBe('rgba(0, 0, 0, 0)');
        if (!forced) {
          expect(editor.color).toBe(await paint(tab, 'var(--border-focus)'));
          expect(editor.ground).toBe(await paint(tab, 'var(--background-default)'));
          if (SHOTS) {
            await tab.screenshot({
              path: resolve(SHOTS, 'sidebar-rename-light.png'),
              clip: { x: 0, y: 200, width: 320, height: 200 },
            });
          }
        }
        // No layout shift: the 24px editor sits 2px into the 28px row, text on the label column.
        expect(editor.height).toBe(24);
        expect(editor.inset).toBe(2);
        expect(Math.abs(editor.textX - 40)).toBeLessThanOrEqual(0.5);
      } finally {
        await tab.close();
      }
    }
  });

  it('steps Crew’s count badge to the page ground on a lit row, so it never vanishes into it', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const strong = await paint(tab, 'var(--background-fill-strong)', '#crew-rows');
        const page = await paint(tab, 'var(--background-default)', '#crew-rows');
        expect(await ground(tab, '#badge-rest'), `${mode}: at rest`).toBe(strong);
        expect(await ground(tab, '#badge-current'), `${mode}: on the current row`).toBe(page);
        expect(await ground(tab, '#badge-current')).not.toBe(await ground(tab, '#crew-current'));
        await tab.hover('#crew-rest');
        await settle(tab);
        expect(await ground(tab, '#badge-rest'), `${mode}: on a hovered row`).toBe(page);
        if (SHOTS) {
          await tab
            .locator('#crew-rows')
            .screenshot({ path: resolve(SHOTS, `crew-badge-${mode}.png`) });
        }
      } finally {
        await tab.close();
      }
    }
  });

  it('floats a row’s hover card 8px clear of the pane divider, not on it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light');
    try {
      await tab.hover('[data-testid="recent-chat-s1"]');
      // Radix renders the content twice inside the card (the text and a hidden copy for the
      // tooltip role); either one finds the card.
      await tab
        .locator('[data-side] [data-testid="recent-chat-summary-s1"]')
        .first()
        .waitFor({ timeout: 5_000 });
      await tab.waitForTimeout(300);
      const gap = await tab.evaluate(() => {
        const card = document
          .querySelector('[data-side] [data-testid="recent-chat-summary-s1"]')!
          .closest('[data-side]')!
          .getBoundingClientRect();
        const column = document
          .querySelector('[data-slot="sidebar-container"]')!
          .getBoundingClientRect();
        return card.left - column.right;
      });
      expect(Math.abs(gap - 8), `card to divider: ${gap}`).toBeLessThanOrEqual(0.5);
      if (SHOTS) {
        await tab.screenshot({
          path: resolve(SHOTS, 'sidebar-hover-card-light.png'),
          clip: { x: 0, y: 200, width: 700, height: 200 },
        });
      }
    } finally {
      await tab.close();
    }
  });

  it('hovers the titlebar controls on the strong fill', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        for (const id of ['titlebar-sidebar-toggle', 'titlebar-new-window']) {
          await tab.hover(`[data-testid="${id}"]`);
          await settle(tab);
          expect(await ground(tab, `[data-testid="${id}"]`), `${mode} ${id}`).toBe(
            await paint(tab, 'var(--background-fill-strong)')
          );
        }
      } finally {
        await tab.close();
      }
    }
  });

  it('is a recessed ground: a filled control on it rests on the strong step, a Note still steps its buttons back', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode, false, 'lost');
      try {
        await tab.waitForSelector('[data-testid="daemon-restart-notice"]');
        await settle(tab);
        const inner = '[data-slot="sidebar-inner"]';
        const strong = await paint(tab, 'var(--background-fill-strong)', inner);
        const rest = await paint(tab, 'var(--background-fill)', inner);
        const page = await paint(tab, 'var(--background-default)', inner);
        // A filled control placed straight on the column (a probe with the shared rest class).
        const probe = () =>
          tab.evaluate((selector) => {
            const el = document.createElement('div');
            el.className = 'br-fill-rest';
            document.querySelector(selector)!.append(el);
            const colour = getComputedStyle(el).backgroundColor;
            el.remove();
            return colour;
          }, inner);
        expect(await probe(), `${mode}: a filled control on the column`).toBe(strong);
        // The instrument can fail: without the class the same probe rests on the page's rest fill,
        // which is 1.03:1 off the sidebar and draws almost nothing.
        await tab.evaluate((selector) => {
          document.querySelector(selector)!.classList.remove('br-ground-recessed');
        }, inner);
        expect(await probe(), `${mode}: the same probe off the recessed ground`).toBe(rest);
        await tab.evaluate((selector) => {
          document.querySelector(selector)!.classList.add('br-ground-recessed');
        }, inner);
        // The footer's Note is a filled box: its secondary button rests on the page ground, not
        // on a fill beside the Note's own.
        const retry = tab.getByRole('button', { name: 'Try again' });
        expect(await retry.evaluate((el) => getComputedStyle(el).backgroundColor)).toBe(page);
        expect(strong).not.toBe(page);
        if (SHOTS) {
          await tab.screenshot({
            path: resolve(SHOTS, `sidebar-daemon-notice-${mode}.png`),
            clip: { x: 0, y: 640, width: 320, height: 260 },
          });
        }
      } finally {
        await tab.close();
      }
    }
  });

  it('rings no row in a menu the pointer opened, and the first key brings the ring back', async (ctx) => {
    if (!browser) return ctx.skip();
    const active = (tab: Page) =>
      tab.evaluate(() => {
        const el = document.activeElement as HTMLElement;
        return {
          role: el.getAttribute('role'),
          label: (el.textContent ?? '').trim(),
          surface: el.closest('[data-menu-input]')?.getAttribute('data-menu-input') ?? null,
          shadow: getComputedStyle(el).boxShadow,
        };
      });
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      // A step that never finds its target fails by name, not by the test's own timeout.
      tab.setDefaultTimeout(8_000);
      try {
        // A chat row's menu: click the ⋯, point at Diverge.
        await tab.hover('[data-testid="recent-chat-s4"]');
        await settle(tab);
        await tab.click('[data-testid="recent-chat-more-s4"]');
        await tab.getByRole('menuitem', { name: 'Diverge' }).hover();
        await settle(tab);
        const pointed = await active(tab);
        expect(pointed, `${mode}: pointed-at row in the row menu`).toMatchObject({
          role: 'menuitem',
          label: 'Diverge',
          surface: 'pointer',
          shadow: 'none',
        });
        await tab.keyboard.press('ArrowDown');
        await settle(tab);
        const keyed = await active(tab);
        expect(keyed.surface, `${mode}: the menu is now driven by keys`).toBe('key');
        expect(keyed.shadow, `${mode}: the focused row rings again`).toMatch(/inset/);
        await tab.keyboard.press('Escape');
        await settle(tab);

        // View options: Group by opens a submenu of radio rows. The trigger row and the radio row
        // the pointer reaches are both unringed; a key rings the one it lands on.
        await tab.click('[data-testid="sidebar-view-options"]');
        await tab.locator('[data-testid="sidebar-view-group-by"]').hover();
        await settle(tab);
        expect(await active(tab), `${mode}: pointed-at Group by`).toMatchObject({
          role: 'menuitem',
          surface: 'pointer',
          shadow: 'none',
        });
        await tab.getByRole('menuitemradio').first().hover();
        await settle(tab);
        expect(await active(tab), `${mode}: pointed-at radio row`).toMatchObject({
          role: 'menuitemradio',
          surface: 'pointer',
          shadow: 'none',
        });
        await tab.keyboard.press('ArrowDown');
        await settle(tab);
        const radio = await active(tab);
        expect(radio.role).toBe('menuitemradio');
        expect(radio.shadow, `${mode}: the radio row rings on a key`).toMatch(/inset/);
      } finally {
        await tab.close();
      }
    }
  });

  /**
   * WS-TOKENS-10 (vision V1-R2-5): a nav row is one ink. Its icon wears `--sidebar-icon`, the
   * muted label ink, at rest, and the label's own ink once the row is lit; the icon used to stay
   * muted beside a default-ink label on hover and on the current row.
   */
  it('paints a nav row’s icon in its label’s ink, at rest and lit', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const inks = (rowSelector: string) =>
          tab.evaluate((selector) => {
            const row = document.querySelector(selector);
            const ink = (part: string) => {
              const el = row?.querySelector(part);
              return el ? getComputedStyle(el).color : `${selector} has no ${part}`;
            };
            return {
              icon: ink('.br-nav-row-icon, .br-nav-chevron'),
              label: ink('.br-nav-row-label'),
            };
          }, rowSelector);
        const muted = await paint(tab, 'var(--text-muted)');
        const ink = await paint(tab, 'var(--text-default)');
        const home = '[data-testid="sidebar-home-button"]';

        await away(tab);
        await settle(tab);
        expect(await inks(home), `${mode}: at rest`).toEqual({ icon: muted, label: muted });
        for (const row of [home, '[data-testid="sidebar-components-disclosure"]']) {
          await tab.hover(row);
          await settle(tab);
          expect(await inks(row), `${mode}: ${row} hovered`).toEqual({ icon: ink, label: ink });
        }
        // The current row: the harness's Crew row drawn current (AppSidebar's own markup).
        await away(tab);
        await settle(tab);
        expect(await inks('#crew-current'), `${mode}: current row`).toEqual({
          icon: ink,
          label: ink,
        });
      } finally {
        await tab.close();
      }
    }
  });

  /**
   * Vision round 2, the list itself under a real layout and real pointer events (jsdom has
   * neither, so the unit tests stand in a ResizeObserver and fire pointer events by hand).
   */
  describe('the list in a real engine (vision round 2)', () => {
    const scrollBox = (tab: Page) =>
      tab.evaluate(() => {
        const el = document.querySelector('[data-testid="recent-chat-scroll"]')!;
        return {
          client: el.clientHeight,
          scroll: el.scrollHeight,
          rows: el.querySelectorAll('[data-row^="chat:"]').length,
        };
      });

    it('reads the next page when its box grows past the rows it holds (V1-R2-1)', async (ctx) => {
      if (!browser) return ctx.skip();
      const now = Date.now();
      const TOTAL = 60;
      const requested: string[] = [];
      // The default view pages by 10 from an opaque cursor, as the daemon does.
      const serve = (url: string) => {
        requested.push(url);
        const query = new URL(url).searchParams;
        const from = Number(query.get('cursor') ?? 0);
        const limit = Number(query.get('limit') ?? 10);
        const rows = Array.from({ length: Math.min(limit, TOTAL - from) }, (_, i) => ({
          id: `s${from + i + 1}`,
          name: `Chat ${from + i + 1}`,
          created_at: new Date(now - (from + i) * 60_000).toISOString(),
          updated_at: new Date(now - (from + i) * 60_000).toISOString(),
          working_dir: '/Users/me/projects/cohort',
          message_count: 2,
          user_set_name: true,
          privacy_tier: 'public',
        }));
        const next = from + rows.length;
        return {
          sessions: rows,
          has_more: next < TOTAL,
          next_cursor: next < TOTAL ? String(next) : null,
        };
      };
      const tab = await open('light', false, 'attached', { serve, height: 480 });
      try {
        await settle(tab);
        const short = await scrollBox(tab);
        expect(short.scroll, 'ten rows overflow the short list').toBeGreaterThan(short.client);
        expect(requested).toHaveLength(1);

        // The window grows: the same ten rows now leave room, and nothing scrolls.
        await tab.setViewportSize({ width: 1200, height: 1400 });
        await expect.poll(() => requested.length, { timeout: 5_000 }).toBeGreaterThan(1);
        await expect
          .poll(
            async () => {
              const box = await scrollBox(tab);
              return box.scroll > box.client || box.rows === TOTAL;
            },
            { timeout: 5_000 }
          )
          .toBe(true);
      } finally {
        await tab.close();
      }
    });

    it('moves a renamed chat to the top at once while the pointer rests on the list (V1-R2-4)', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open('light');
      try {
        const order = () =>
          tab.evaluate(() =>
            [
              ...document.querySelectorAll(
                '[data-testid="recent-chat-scroll"] [data-row^="chat:"]'
              ),
            ].map((el) => el.getAttribute('data-row'))
          );
        expect((await order())[0]).toBe('chat:s1');

        // The usual way: right-click, Rename, type, Enter, with the pointer still on the list.
        await tab.click('[data-testid="recent-chat-s7"]', { button: 'right' });
        await tab.getByRole('menuitem', { name: /^Rename/ }).click();
        const input = tab.getByRole('textbox', { name: /Rename chat/ });
        await input.waitFor();
        await tab.hover('[data-testid="recent-chat-s3"]');
        await input.fill('Methods draft');
        await input.press('Enter');

        await expect.poll(async () => (await order())[0], { timeout: 2_000 }).toBe('chat:s7');
        const after = await tab.evaluate(() => ({
          hovered: document.querySelector('[data-testid="recent-chat-scroll"]')!.matches(':hover'),
          focused: document.activeElement?.getAttribute('data-row'),
        }));
        expect(after).toEqual({ hovered: true, focused: 'chat:s7' });
        if (SHOTS) {
          await tab.screenshot({
            path: resolve(SHOTS, 'sidebar-renamed-moves-light.png'),
            clip: { x: 0, y: 0, width: 600, height: 700 },
          });
        }
      } finally {
        await tab.close();
      }
    });

    /** `total` chats under Group by None, newest first: line N of the list is chat `s{N + 1}`. */
    const openLongList = (total: number, options: { scale?: number } = {}) => {
      const now = Date.now();
      const all = Array.from({ length: total }, (_, i) => ({
        id: `s${i + 1}`,
        name: `Chat ${i + 1}`,
        created_at: new Date(now - i * 1_000).toISOString(),
        updated_at: new Date(now - i * 1_000).toISOString(),
        working_dir: '/Users/me/projects/cohort',
        message_count: 2,
        user_set_name: true,
        privacy_tier: 'public',
      }));
      const serve = (url: string) => {
        const query = new URL(url).searchParams;
        const from = Number(query.get('cursor') ?? 0);
        const rows = all.slice(from, from + Number(query.get('limit') ?? 10));
        const next = from + rows.length;
        return {
          sessions: rows,
          has_more: next < total,
          next_cursor: next < total ? String(next) : null,
        };
      };
      const stored = JSON.stringify({ v: 1, groupBy: 'none', sortBy: 'activity' });
      return open('light', false, 'attached', {
        serve,
        scale: options.scale,
        init: `localStorage.setItem('biorouter:sidebar-chat-view', ${JSON.stringify(stored)});`,
      });
    };

    /** Scroll to the end until every chat is in reach, then to `fraction` of the way down. */
    const scrollLongList = async (tab: Page, fraction: number) => {
      let height = 0;
      for (let i = 0; i < 20; i++) {
        const next = await tab.evaluate(() => {
          const el = document.querySelector('[data-testid="recent-chat-scroll"]')!;
          el.scrollTop = el.scrollHeight;
          return el.scrollHeight;
        });
        await tab.waitForTimeout(100);
        if (next === height) break;
        height = next;
      }
      await tab.evaluate((f) => {
        const el = document.querySelector('[data-testid="recent-chat-scroll"]')!;
        el.scrollTop = el.scrollHeight * f;
      }, fraction);
      await tab.waitForTimeout(200);
    };

    /**
     * What the list draws: rows and the lines the placeholders stand for, the list's height, the
     * line pitch averaged over the rows in view, every visible row not on its own line (more than
     * a pixel off line N × `pitch`, where it sits with every row drawn), and the placeholders in
     * view.
     */
    const readLongList = (tab: Page, expectedPitch = 30) =>
      tab.evaluate((pitch) => {
        const scroll = document.querySelector('[data-testid="recent-chat-scroll"]')!;
        const box = scroll.getBoundingClientRect();
        const list = scroll.querySelector('ul')!;
        const listTop = list.getBoundingClientRect().top;
        const rows = [...list.querySelectorAll<HTMLElement>('button[data-row^="chat:"]')];
        const lineOf = (row: Element) =>
          Number(row.getAttribute('data-row')!.slice('chat:s'.length)) - 1;
        const visible = rows.filter((row) => {
          const rect = row.getBoundingClientRect();
          return rect.bottom > box.top && rect.top < box.bottom;
        });
        const first = visible[0];
        const last = visible[visible.length - 1];
        const gaps = [...list.querySelectorAll<HTMLElement>('.br-chat-row-gap')];
        return {
          drawn: rows.length,
          lines:
            rows.length +
            gaps.reduce(
              (sum, gap) => sum + Number(gap.style.getPropertyValue('--br-gap-lines')),
              0
            ),
          listHeight: list.getBoundingClientRect().height,
          rowHeight: first?.getBoundingClientRect().height ?? NaN,
          pitch:
            visible.length < 2
              ? NaN
              : (last.getBoundingClientRect().top - first.getBoundingClientRect().top) /
                (lineOf(last) - lineOf(first)),
          visible: visible.length,
          misplaced: visible
            .map((row) => {
              const top = row.getBoundingClientRect().top - listTop;
              return Math.abs(top - lineOf(row) * pitch) <= 1
                ? null
                : `${row.getAttribute('data-row')}@${top.toFixed(1)}`;
            })
            .filter(Boolean),
          gapsInView: gaps.filter((gap) => {
            const rect = gap.getBoundingClientRect();
            return rect.bottom > box.top && rect.top < box.bottom;
          }).length,
        };
      }, expectedPitch);

    /**
     * Vision V1-R2-2, round 4 (R-28). The list grew as it was scrolled and never shrank: after a
     * scroll to its end every chat was a mounted row, and with 5,542 drawn a right-click took
     * about 2.2 s, most of it restyling them. Now only the lines near the viewport are rows; the
     * rest are placeholders, and every row must still sit exactly where it would with all of them
     * drawn: line N of the list at N × 30px (a 28px row and the list's 2px), and the list exactly
     * as tall as its lines. Home still reaches the first chat, and the right-click menu is not
     * modal, so it leaves <body> alone.
     */
    it('draws only the rows near the viewport, each where it would be (V1-R2-2)', async (ctx) => {
      if (!browser) return ctx.skip();
      const TOTAL = 400;
      const tab = await openLongList(TOTAL);
      try {
        await scrollLongList(tab, 0.5);
        const state = await readLongList(tab);
        expect(state.lines, 'every chat is in reach').toBe(TOTAL);
        expect(state.pitch).toBe(30);
        expect(state.listHeight, 'the list is as tall as its lines').toBe(TOTAL * 30 - 2);
        expect(state.visible).toBeGreaterThan(10);
        expect(state.misplaced, 'every visible row on its own line').toEqual([]);
        expect(state.gapsInView, 'no placeholder shows').toBe(0);
        expect(state.drawn, 'only the rows near the viewport are drawn').toBeLessThan(80);

        // The right-click menu opens on a visible row and leaves <body> alone.
        const target = await tab.evaluate(() => {
          const scroll = document.querySelector('[data-testid="recent-chat-scroll"]')!;
          const box = scroll.getBoundingClientRect();
          const row = [...scroll.querySelectorAll('button[data-row^="chat:"]')].find(
            (el) => el.getBoundingClientRect().top > box.top + 100
          )!;
          return row.getAttribute('data-testid')!;
        });
        await tab.click(`[data-testid="${target}"]`, { button: 'right' });
        await tab.getByRole('menuitem', { name: /^Rename/ }).waitFor();
        expect(await tab.evaluate(() => document.body.style.pointerEvents)).toBe('');
        await tab.keyboard.press('Escape');
        await tab.getByRole('menu').waitFor({ state: 'detached' });

        // Home from a row in the middle reaches the first chat.
        await tab.focus(`[data-testid="${target}"]`);
        await tab.keyboard.press('Home');
        await expect
          .poll(() => tab.evaluate(() => document.activeElement?.getAttribute('data-row')))
          .toBe('chat:s1');
      } finally {
        await tab.close();
      }
    });

    /**
     * The same list under the Larger text preference (`--app-font-scale` 1.15): the rows and the
     * list's 2px scale with the type (`vite-plugins/fontScale.mjs`), so a line is 34.5px, not 30.
     * Read at 30px, the lines in view came out 15% too far down the list, so from the middle on
     * the view showed placeholder where rows should be; and a placeholder that kept its 2px
     * unscaled drifted from the rows it stands for.
     */
    it('keeps the rows near the viewport drawn under Larger text (V1-R2-2)', async (ctx) => {
      if (!browser) return ctx.skip();
      const TOTAL = 400;
      const tab = await openLongList(TOTAL, { scale: 1.15 });
      try {
        for (const fraction of [0.5, 0.9]) {
          await scrollLongList(tab, fraction);
          // 28px × 1.15 and the list's 2px × 1.15.
          const state = await readLongList(tab, 34.5);
          expect(state.lines, 'every chat is in reach').toBe(TOTAL);
          expect(state.visible, `${fraction}: rows in view`).toBeGreaterThan(10);
          expect(state.pitch, 'a line scales with the type').toBeCloseTo(34.5, 1);
          expect(state.gapsInView, `${fraction}: no placeholder shows`).toBe(0);
          expect(state.misplaced, `${fraction}: every visible row on its own line`).toEqual([]);
          expect(
            Math.abs(state.listHeight - (TOTAL * 34.5 - 2.3)),
            'the list is as tall as its lines'
          ).toBeLessThanOrEqual(1);
        }
      } finally {
        await tab.close();
      }
    });

    /**
     * Vision V1-R3-3. A view that sorts or groups the whole list was drawn from the part already
     * read (the first page, or the ten rows the default view had paged in), and reordered when the
     * rest landed 0.2 to 0.4 s later, under a resting pointer. Every order the list paints, frame
     * by frame, must now be the complete arrangement, or the default one before a switch.
     */
    it('paints a view that needs every chat only once its read has landed (V1-R3-3)', async (ctx) => {
      if (!browser) return ctx.skip();
      const now = Date.now();
      const TOTAL = 120;
      // Names out of activity order (37 is prime to 120), so no part of the list sorts by name
      // into the start of the whole list's name order: that starts "Chat 000", the oldest chat.
      const all = Array.from({ length: TOTAL }, (_, i) => ({
        id: `s${i + 1}`,
        name: `Chat ${String(((i + 1) * 37) % TOTAL).padStart(3, '0')}`,
        created_at: new Date(now - i * 60_000).toISOString(),
        updated_at: new Date(now - i * 60_000).toISOString(),
        working_dir: '/Users/me/projects/cohort',
        message_count: 2,
        user_set_name: true,
        privacy_tier: 'public',
      }));
      const byName = [...all]
        .sort((left, right) => left.name.localeCompare(right.name))
        .map((row) => `chat:${row.id}`);
      const byActivity = all.map((row) => `chat:${row.id}`);
      // The daemon pages from an opaque cursor; the rest of a full read takes a while.
      const serve = async (url: string) => {
        const query = new URL(url).searchParams;
        const from = Number(query.get('cursor') ?? 0);
        const limit = Number(query.get('limit') ?? 10);
        if (from > 0 && limit > 50) await new Promise((done) => setTimeout(done, 600));
        const rows = all.slice(from, from + limit);
        const next = from + rows.length;
        return {
          sessions: rows,
          has_more: next < TOTAL,
          next_cursor: next < TOTAL ? String(next) : null,
        };
      };
      // Every distinct order of drawn rows, sampled each frame from before the app starts.
      const recorder = (stored: string | null) => `
        ${stored ? `localStorage.setItem('biorouter:sidebar-chat-view', ${JSON.stringify(stored)});` : ''}
        window.__orders = [];
        (function tick() {
          const rows = Array.from(
            document.querySelectorAll('[data-testid="recent-chat-scroll"] [data-row^="chat:"]'),
            (el) => el.getAttribute('data-row')
          );
          const last = window.__orders[window.__orders.length - 1];
          if (!last || last.join() !== rows.join()) window.__orders.push(rows);
          requestAnimationFrame(tick);
        })();
      `;
      const painted = (tab: Page) =>
        tab.evaluate(() =>
          (window as unknown as { __orders: string[][] }).__orders.filter((o) => o.length > 0)
        );
      const startsOf = (whole: string[]) => (order: string[]) =>
        order.every((id, index) => whole[index] === id);
      const topRow = async (tab: Page) => {
        const orders = await painted(tab);
        return orders[orders.length - 1]?.[0];
      };

      // A reload with None / Name stored.
      const stored = JSON.stringify({ v: 1, groupBy: 'none', sortBy: 'name' });
      let tab = await open('light', false, 'attached', { serve, init: recorder(stored) });
      try {
        await expect.poll(() => topRow(tab)).toBe(byName[0]);
        const orders = await painted(tab);
        expect
          .soft(
            orders.filter((order) => !startsOf(byName)(order)),
            'every order painted on the reload is the complete name order'
          )
          .toEqual([]);
      } finally {
        await tab.close();
      }

      // The default view, then Sort by Name through the real menu.
      tab = await open('light', false, 'attached', { serve, init: recorder(null) });
      tab.setDefaultTimeout(8_000);
      try {
        await tab.click('[data-testid="sidebar-view-options"]');
        await tab.locator('[data-testid="sidebar-view-sort-by"]').hover();
        await tab.getByRole('menuitemradio', { name: 'Name' }).click();
        await tab.waitForSelector('[data-testid="recent-chat-loading"]');
        await expect.poll(() => topRow(tab), { timeout: 5_000 }).toBe(byName[0]);
        const orders = await painted(tab);
        const firstNamed = orders.findIndex((order) => order[0] !== byActivity[0]);
        expect
          .soft(
            orders.slice(0, firstNamed).filter((order) => !startsOf(byActivity)(order)),
            'before the read lands, the default order'
          )
          .toEqual([]);
        expect
          .soft(
            orders.slice(firstNamed).filter((order) => !startsOf(byName)(order)),
            'after it, only the complete name order'
          )
          .toEqual([]);
      } finally {
        await tab.close();
      }
    });
  });

  it('saves screenshots when asked', async (ctx) => {
    if (!browser || !SHOTS) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        await tab.screenshot({
          path: resolve(SHOTS, `sidebar-${mode}.png`),
          clip: { x: 0, y: 0, width: 600, height: 700 },
        });
        await tab.click('[data-testid="sidebar-view-options"]');
        await away(tab);
        await settle(tab);
        await tab.screenshot({
          path: resolve(SHOTS, `sidebar-view-open-${mode}.png`),
          clip: { x: 0, y: 200, width: 520, height: 260 },
        });
      } finally {
        await tab.close();
      }
    }
  });
});
