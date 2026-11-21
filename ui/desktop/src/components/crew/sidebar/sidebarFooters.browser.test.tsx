// @vitest-environment node
/**
 * The two sidebar footers on `/crew`, measured side by side in a real layout engine.
 *
 * On `/crew` the app sidebar and the Crew sidebar stand next to each other down to the window's
 * foot, so their footers read as one band: the app sidebar's Settings row and Crew's You row. The
 * owner saw them disagree (2026-11-19): Crew's hairline sat several pixels above the app's, and
 * the two rows did not share a centre line. Nothing caught it, because each sidebar's own suite
 * measures only itself, and jsdom lays nothing out.
 *
 * This file paints the REAL `AppSidebar` inside the real `Sidebar` and `SidebarInset`, composed as
 * `AppLayout` composes them, and in the inset the Crew grid as `CrewLayout` composes it (`.crew-app`
 * holding `.crew-sidebar` around the real `CrewSidebar`), bundled with esbuild. The stylesheet is
 * `styles/main.css` compiled by Tailwind from the classes the bundle uses, followed by every
 * stylesheet the bundle imports (`sidebar.css`, `crew-app.css`, `crew-sidebar.css`, …), in the
 * order the app's bundler appends them. It then asserts, at several window sizes, app sidebar
 * widths and text sizes:
 *
 * - both footers end at the window's foot and draw their hairline at the same y, so the line runs
 *   on across the seam;
 * - the Settings row and the You row share one centre line, before a snapshot names me (one line,
 *   the SSH login) and after (my name, and "on {server}" under it);
 * - with one line, "Settings" and the login share a text baseline;
 * - the You row's text stays inside its own hover ground;
 * - the login is in the sidebar's sans, as every other line of chrome is (design.md D-33).
 *
 * Set `CREW_FOOTER_SHOTS_DIR` to also save light and dark screenshots of the two footers.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { fontScale } from '../../../../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../../styles');
const SHOTS = process.env.CREW_FOOTER_SHOTS_DIR;
const requireFromDesktop = createRequire(resolve(here, '../../../../package.json'));
const ORIGIN = 'http://crew-footer.test';

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { ChatProvider } from '../../../contexts/ChatContext';
import { ConfigProvider } from '../../ConfigContext';
import { Sidebar, SidebarInset, SidebarProvider } from '../../ui/sidebar';
import AppSidebar from '../../BioRouterSidebar/AppSidebar';
import { crewTestController } from '../files/crewTestController';
import { CrewControllerProvider } from '../state/CrewControllerContext';
import { CrewSidebar } from './CrewSidebar';
import '../crew-app.css';
import '../layout/layout.css';

const alice = { id: 'person-alice', uid: 1000, username: 'alice', nickname: 'Alice Chen' };
const connection = {
  id: 'conn-1',
  name: 'Fixture',
  ssh_target: 'alice@hpc.ucsf.edu',
  port: 22,
  socket_path: '/tmp/socket',
  owner_uid: 1000,
  workspace_id: 'workspace-1',
  workspace_public_key: 'workspace-key',
  public_key: 'device-key',
  device_id: 'device-1',
  cluster_connection_id: 'cluster-1',
  mode: 'private',
  policy_epoch: 1,
  institution_id: 'ucsf',
  status: 'connected',
  remote_execution: false,
  remote_access: 'folder',
  workspace_deleted: false,
  new_devices: [],
};
const general = {
  id: 'chan-general', team_id: 'team-lab', name: 'general', created_by: alice.id,
  owner_id: alice.id, members: [alice.id], archived: false, classification: 'restricted',
};
const snapshot = {
  workspace: { id: 'workspace-1', host_uid: 1000, mode: 'private', institution_id: 'ucsf', policy_epoch: 1 },
  actor: alice,
  principals: [alice],
  teams: [{ id: 'team-lab', name: 'Analysis Lab', created_by: alice.id, members: [alice.id], general_channel_id: general.id }],
  channels: [general],
  invitations: [],
  runs: [],
};

// 'login': no snapshot names me yet, so the You row is the SSH login alone (one line).
// 'name': verified, so it is my name with "on hpc.ucsf.edu" under it (two lines).
const named = window.__crewFooterState === 'name';
const controller = crewTestController({
  connections: [connection],
  connectionId: connection.id,
  connection,
  snapshot: named ? snapshot : null,
  observedPrivacy: named
    ? { connectionId: connection.id, mode: 'private', institutionId: 'ucsf', policyEpoch: 1 }
    : null,
  teamId: 'team-lab',
  channelId: general.id,
  channel: general,
  status: named ? 'connected' : 'checking',
});

function Shell() {
  const [chat, setChat] = React.useState({ sessionId: 's1', name: 'Survival analysis', messages: [], workflow: null });
  return (
    <ChatProvider chat={chat} setChat={setChat}>
      {/* AppLayout's own composition. */}
      <div className="relative flex w-full flex-1" style={{ height: '100vh' }}>
        <Sidebar variant="inset" collapsible="offcanvas">
          <AppSidebar onSelectSession={() => {}} currentPath="/crew" />
        </Sidebar>
        <SidebarInset>
          <main className="route-container biorouter-route-surface relative z-[60] flex h-full min-h-0 min-w-0 flex-1 flex-col">
            {/* CrewLayout's: the grid, the sidebar column, then the main area. */}
            <CrewControllerProvider controller={controller}>
              <div className="crew-app" data-crew-screen="channel">
                <div className="crew-sidebar">
                  <CrewSidebar />
                </div>
                <div className="crew-main" />
              </div>
            </CrewControllerProvider>
          </main>
        </SidebarInset>
      </div>
    </ChatProvider>
  );
}

createRoot(document.getElementById('root')).render(
  <MemoryRouter initialEntries={['/crew']}>
    <ConfigProvider>
      <SidebarProvider>
        <Shell />
      </SidebarProvider>
    </ConfigProvider>
  </MemoryRouter>
);
`;

/**
 * The window globals the real sidebars read (the preload bridge and the app config). `update`
 * makes the updater report a ready update, which adds the "Update to …" row to the app footer.
 */
const stubs = (update: boolean) => `
window.electron = {
  platform: 'darwin',
  getUserActionKey: async () => 'test-key',
  createChatWindow: () => {},
  copyConversationId: async () => {},
  onUpdaterEvent: () => () => {},
  getUpdateState: async () => ${
    update ? "({ updateAvailable: true, latestVersion: '9.9.9', status: 'available' })" : 'null'
  },
  getDaemonConnection: async () => 'attached',
  onDaemonConnection: () => () => {},
  on: () => {},
  off: () => {},
};
window.appConfig = { get: (key) => (key === 'BIOROUTER_HOME_DIR' ? '/Users/me' : undefined) };
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

type State = 'login' | 'name';

interface Measured {
  window: number;
  app: { top: number; bottom: number; border: string };
  crew: { top: number; bottom: number; border: string };
  settings: { top: number; bottom: number };
  you: { top: number; bottom: number };
  settingsBaseline: number;
  youLines: { top: number; bottom: number; baseline: number }[];
  settingsFont: string;
  youFonts: string[];
}

const EPSILON = 0.5;
const near = (actual: number, expected: number, what: string) =>
  expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThanOrEqual(
    EPSILON
  );
const centre = (box: { top: number; bottom: number }) => (box.top + box.bottom) / 2;

describe('the app sidebar’s footer and Crew’s You row, side by side', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';
  let scaled: Promise<string> | null = null;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the sidebar footer alignment tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      // Named only so esbuild may emit the script and the stylesheet as two outputs; with
      // `write: false` nothing lands on disk.
      outdir: resolve(here, '.footer-bundle'),
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: {
        'process.env.NODE_ENV': '"production"',
        'process.env.ALPHA': 'false',
        'process.env.BIOROUTER_TUNNEL': 'true',
        'import.meta.env': JSON.stringify({ MODE: 'production', DEV: false, PROD: true }),
      },
      // The stylesheets the components import are kept, in import order, as the app's bundler
      // keeps them, so the authored rules come from the files the app ships and nowhere else.
      loader: { '.css': 'css', '.svg': 'dataurl', '.png': 'dataurl' },
      logLevel: 'silent',
    });
    const output = (extension: string) =>
      bundle.outputFiles.find((file) => file.path.endsWith(extension))?.text ?? '';
    script = output('.js');
    const imported = output('.css');
    expect(imported, 'the bundle carries the Crew sidebar’s stylesheet').toContain(
      '.crew-sidebar-you'
    );
    expect(imported, 'the bundle carries the app sidebar’s stylesheet').toContain(
      '.br-sidebar-footer'
    );
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    css =
      compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }])) +
      '\n' +
      imported;
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** The stylesheet through the app's own font-scale plugin, as the app's bundler serves it. */
  const scaledCss = () =>
    (scaled ??= postcss([fontScale()])
      .process(css, { from: undefined })
      .then((result) => result.css));

  async function open(options: {
    width: number;
    height: number;
    sidebar: number;
    state: State;
    mode?: 'light' | 'dark';
    scale?: number;
    update?: boolean;
  }): Promise<Page> {
    const tab = await browser!.newPage({
      viewport: { width: options.width, height: options.height },
      deviceScaleFactor: SHOTS ? 2 : 1,
      reducedMotion: 'reduce',
    });
    await tab.addInitScript(
      ([sidebar, state]) => {
        try {
          window.localStorage.setItem('biorouter:sidebar-width', String(sidebar));
        } catch {
          // a page without storage keeps the default width
        }
        (window as unknown as { __crewFooterState: string }).__crewFooterState = state;
      },
      [options.sidebar, options.state] as const
    );
    const sheet = options.scale ? await scaledCss() : css;
    const html =
      `<!doctype html><html class="${options.mode === 'dark' ? 'dark' : ''}"` +
      (options.scale ? ` style="--app-font-scale: ${options.scale}"` : '') +
      `><head><style>${sheet}</style></head>` +
      `<body style="margin:0"><div id="root"></div>` +
      `<script>${stubs(options.update ?? false)}</script><script>${script.replace(/<\/script/gi, '<\\/script')}</script>` +
      `</body></html>`;
    await tab.route(`${ORIGIN}/**`, async (route) => {
      const url = route.request().url();
      if (url === `${ORIGIN}/`) {
        await route.fulfill({ status: 200, contentType: 'text/html', body: html });
      } else if (new URL(url).pathname === '/config') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: '{"config":{}}',
        });
      } else if (url.includes('/sessions/sidebar')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ sessions: [], has_more: false, next_cursor: null }),
        });
      } else {
        await route.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
      }
    });
    const errors: string[] = [];
    tab.on('pageerror', (error) => errors.push(error.message));
    await tab.goto(`${ORIGIN}/`);
    try {
      await tab.waitForSelector('[data-testid="sidebar-settings-button"]', { timeout: 20_000 });
      await tab.waitForSelector('[data-crew-you] .crew-sidebar-you-trigger', { timeout: 20_000 });
    } catch (error) {
      await tab.close();
      throw new Error(`the page did not render (${errors.join('; ') || String(error)})`);
    }
    if (options.update) {
      await tab.waitForSelector('[data-testid="sidebar-update-button"]', { timeout: 20_000 });
    }
    await tab.waitForTimeout(250);
    return tab;
  }

  const measure = (tab: Page) =>
    tab.evaluate((): Measured => {
      const box = (el: Element) => {
        const r = el.getBoundingClientRect();
        return { top: r.top, bottom: r.bottom };
      };
      const one = (selector: string) => {
        const el = document.querySelector(selector);
        if (!el) throw new Error(`no ${selector}`);
        return el;
      };
      /** A zero-size inline-block at the end of the text sits on its first line's baseline. */
      const baseline = (el: Element) => {
        const probe = document.createElement('span');
        probe.style.cssText = 'display:inline-block;width:0;height:0;vertical-align:baseline';
        el.append(probe);
        const y = probe.getBoundingClientRect().bottom;
        probe.remove();
        return y;
      };
      const hairline = (el: Element) => {
        const cs = getComputedStyle(el);
        return `${cs.borderTopWidth} ${cs.borderTopStyle} ${cs.borderTopColor}`;
      };
      const appFooter = one('[data-testid="sidebar-footer"]');
      const crewFooter = one('[data-crew-you]');
      const settingsLabel = one('[data-testid="sidebar-settings-button"] .br-nav-row-label');
      // The You row's lines, top to bottom: my name (when a snapshot names me), then the place or
      // the login.
      const lines = Array.from(one('.crew-sidebar-you-text').children);
      return {
        window: window.innerHeight,
        app: { ...box(appFooter), border: hairline(appFooter) },
        crew: { ...box(crewFooter), border: hairline(crewFooter) },
        settings: box(one('[data-testid="sidebar-settings-button"]')),
        you: box(one('.crew-sidebar-you-trigger')),
        settingsBaseline: baseline(settingsLabel),
        youLines: lines.map((line) => ({ ...box(line), baseline: baseline(line) })),
        settingsFont: getComputedStyle(settingsLabel).fontFamily,
        youFonts: lines.map((line) => getComputedStyle(line).fontFamily),
      };
    });

  function expectAligned(m: Measured, state: State, what: string) {
    // Both footers run to the window's foot…
    near(m.app.bottom, m.window, `${what}: the app sidebar's footer ends at the window's foot`);
    near(m.crew.bottom, m.window, `${what}: Crew's footer ends at the window's foot`);
    // …and draw one hairline at one height, so the line runs on across the seam.
    near(m.crew.top, m.app.top, `${what}: the two footers' hairlines`);
    expect(m.crew.border, `${what}: the two hairlines are drawn alike`).toBe(m.app.border);
    // The Settings row and the You row share one centre line.
    near(centre(m.you), centre(m.settings), `${what}: the rows' centres`);
    // The You row sits wholly under its hairline, and its text inside its own hover ground.
    expect(m.you.top, `${what}: the You row starts under the hairline`).toBeGreaterThan(m.crew.top);
    expect(m.you.bottom, `${what}: the You row ends inside the footer`).toBeLessThanOrEqual(
      m.crew.bottom
    );
    const first = m.youLines[0];
    const last = m.youLines[m.youLines.length - 1];
    expect(first.top + EPSILON, `${what}: the text starts inside the row`).toBeGreaterThanOrEqual(
      m.you.top
    );
    expect(last.bottom - EPSILON, `${what}: the text ends inside the row`).toBeLessThanOrEqual(
      m.you.bottom
    );
    expect(m.youLines, `${what}: the You row's lines`).toHaveLength(state === 'name' ? 2 : 1);
    if (state === 'login') {
      // One line beside one line: "Settings" and the login sit on one baseline.
      near(first.baseline, m.settingsBaseline, `${what}: "Settings" and the login's baselines`);
    }
    // Chrome is in the sans (design.md D-33): the login and the place included.
    for (const font of m.youFonts) {
      expect(font, `${what}: the You row's type`).toBe(m.settingsFont);
    }
  }

  const WINDOWS = [
    { width: 1200, height: 800 },
    { width: 1600, height: 1000 },
  ] as const;
  const SIDEBARS = [216, 288, 360] as const;
  const STATES: State[] = ['login', 'name'];

  it.each(STATES)(
    'lines up the hairlines and the rows at every app sidebar width and window size (%s)',
    async (state) => {
      if (!browser) return;
      for (const size of WINDOWS) {
        for (const sidebar of SIDEBARS) {
          const tab = await open({ ...size, sidebar, state });
          try {
            expectAligned(
              await measure(tab),
              state,
              `${state}, ${size.width}x${size.height}, sidebar ${sidebar}px`
            );
          } finally {
            await tab.close();
          }
        }
      }
    },
    180_000
  );

  it.each(STATES)(
    'keeps them lined up under a larger text size (%s)',
    async (state) => {
      if (!browser) return;
      for (const scale of [1.15, 1.3]) {
        const tab = await open({ width: 1440, height: 900, sidebar: 288, state, scale });
        try {
          expectAligned(await measure(tab), state, `${state}, text ×${scale}`);
        } finally {
          await tab.close();
        }
      }
    },
    180_000
  );

  // A ready update adds a row above Settings, so the app footer grows by that row and its rule
  // rises above Crew's: a second row is a different footer, not a near miss. Settings stays the
  // footer's last row, so the two rows keep their centre line.
  it.each(STATES)(
    'keeps the rows on one centre line while the app footer also offers an update (%s)',
    async (state) => {
      if (!browser) return;
      const tab = await open({ width: 1440, height: 900, sidebar: 288, state, update: true });
      try {
        const m = await measure(tab);
        near(centre(m.you), centre(m.settings), `${state}, update row: the rows' centres`);
        near(m.app.bottom, m.window, `${state}, update row: the app footer's foot`);
        near(m.crew.bottom, m.window, `${state}, update row: Crew's footer's foot`);
        expect(m.app.top, `${state}, update row: the app footer grew by its row`).toBeLessThan(
          m.crew.top
        );
      } finally {
        await tab.close();
      }
    },
    180_000
  );

  it.each(STATES)(
    'lines them up in dark mode too, and saves the footers when asked (%s)',
    async (state) => {
      if (!browser) return;
      const measured: [string, Measured][] = [];
      for (const mode of ['light', 'dark'] as const) {
        const tab = await open({ width: 1440, height: 900, sidebar: 288, state, mode });
        try {
          if (SHOTS) {
            // Both modes are saved before anything is asserted, so a failing layout is on
            // record too. Device pixels at 2x, so a 1px step between the hairlines shows.
            mkdirSync(SHOTS, { recursive: true });
            const clip = { x: 0, y: 900 - 120, width: 288 + 240 + 160, height: 120 };
            await tab.screenshot({
              path: resolve(SHOTS, `crew-footers-${state}-${mode}.png`),
              clip,
            });
            // The You row's hover ground beside the Settings row's.
            await tab.hover('.crew-sidebar-you-trigger');
            await tab.waitForTimeout(150);
            await tab.screenshot({
              path: resolve(SHOTS, `crew-footers-${state}-${mode}-hover.png`),
              clip,
            });
            await tab.mouse.move(1400, 10);
            await tab.waitForTimeout(150);
          }
          measured.push([mode, await measure(tab)]);
        } finally {
          await tab.close();
        }
      }
      for (const [mode, m] of measured) expectAligned(m, state, `${state}, ${mode}`);
    },
    180_000
  );
});
