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
 * - the titlebar controls hover on the strong fill.
 *
 * Colours are compared with probes that paint the token itself, so a token value can change
 * without this file changing. Set `SIDEBAR_SHOTS_DIR` to also save light and dark screenshots.
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

/** The window globals the real sidebar reads (the preload bridge and the app config). */
const STUBS = `
window.electron = {
  platform: 'darwin',
  getUserActionKey: async () => 'test-key',
  createChatWindow: () => {},
  copyConversationId: async () => {},
  onUpdaterEvent: () => () => {},
  getUpdateState: async () => null,
  getDaemonConnection: async () => 'attached',
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

  async function open(mode: 'light' | 'dark', forcedColors = false): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1200, height: 900 } });
    if (forcedColors) await tab.emulateMedia({ forcedColors: 'active' });
    const html =
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style></head>` +
      `<body style="margin:0"><div id="root"></div>` +
      `<script>${STUBS}</script><script>${script.replace(/<\/script/gi, '<\\/script')}</script>` +
      `</body></html>`;
    await tab.route(`${ORIGIN}/**`, async (route) => {
      const url = route.request().url();
      if (url === `${ORIGIN}/`) {
        await route.fulfill({ status: 200, contentType: 'text/html', body: html });
      } else if (url.includes('/sessions/sidebar')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ sessions: sessions(), has_more: false, next_cursor: null }),
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
