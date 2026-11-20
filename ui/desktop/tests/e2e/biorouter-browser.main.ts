// The main process for `biorouter-browser.spec.ts`: the REAL Biorouter Browser modules
// (`src/utils/embeddedBrowser.ts`, `src/browser/*`) in a real Electron, with a bare window
// standing in for the renderer. The spec bundles this file with esbuild and drives it through
// `electronApp.evaluate`, calling the functions on `globalThis.__biorouterBrowserTest`.
//
// The renderer half (the panel, the bar) is covered by the Vitest suites; this harness covers
// what only Chromium can answer: real navigation, real windows, real downloads, the proxy.

import { app, BrowserWindow, webContents, type WebContents } from 'electron';
import log from 'electron-log';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import {
  claimEmbeddedBrowser,
  controlEmbeddedBrowser,
  createEmbeddedBrowser,
  destroyEmbeddedBrowser,
  embeddedBrowserSuggestions,
  navigateEmbeddedBrowser,
  releaseEmbeddedBrowser,
  setEmbeddedBrowserBounds,
  setEmbeddedBrowserVisible,
} from '../../src/utils/embeddedBrowser';
import { configureBrowserTestHosts, reserveLocalPort } from '../../src/browser/localNetwork';
import { configureBrowserTestDownloads } from '../../src/browser/downloads';
import { browserTestHooksEnabled } from '../../src/browser/testHooks';
import type { EmbeddedBrowserState } from '../../src/browser/types';

const userData = process.env.BIOROUTER_BROWSER_TEST_USER_DATA;
if (userData) {
  app.setPath('userData', userData);
  // Keep electron-log's file in the run's own folder, never the person's ~/Library/Logs.
  log.transports.file.resolvePathFn = () => path.join(userData, 'logs', 'main.log');
}

// Test only: the fixture is plain http, and some APIs (the clipboard) need a secure context.
if (process.env.BIOROUTER_BROWSER_TEST_SECURE_ORIGINS) {
  app.commandLine.appendSwitch(
    'unsafely-treat-insecure-origin-as-secure',
    process.env.BIOROUTER_BROWSER_TEST_SECURE_ORIGINS
  );
}

// The same gate main.ts uses: an unpackaged app and BIOROUTER_E2E=1.
const hooks = browserTestHooksEnabled(process.env, app.isPackaged);
configureBrowserTestHosts(process.env.BIOROUTER_BROWSER_TEST_HOSTS, hooks);
configureBrowserTestDownloads(
  hooks ? (process.env.BIOROUTER_BROWSER_TEST_DOWNLOAD_DIR ?? null) : null
);

let window: BrowserWindow | null = null;
const states = new Map<string, EmbeddedBrowserState>();
const sent: Array<{ channel: string; payload: unknown }> = [];

function record(viewId: string) {
  return (state: EmbeddedBrowserState) => {
    states.set(viewId, state);
  };
}

function contentsOf(viewId: string): WebContents {
  const state = states.get(viewId);
  const id = Number(state?.sourceRevision.split(':')[0]);
  const contents = Number.isFinite(id) ? webContents.fromId(id) : undefined;
  if (!contents) throw new Error(`no web contents for ${viewId}`);
  return contents;
}

async function centerOf(contents: WebContents, selector: string) {
  return (await contents.executeJavaScript(
    `(() => { const e = document.querySelector(${JSON.stringify(selector)}); if (!e) return null;
      e.scrollIntoView({ block: 'center' }); const r = e.getBoundingClientRect();
      return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) }; })()`
  )) as { x: number; y: number } | null;
}

const api = {
  ready: async () => {
    await app.whenReady();
    if (!window) {
      window = new BrowserWindow({ width: 1100, height: 800, show: false });
      await window.loadURL('data:text/html,<title>Fixture host window</title>');
      window.showInactive();
      // The renderer's half of the IPC: record what main tells the panel.
      const original = window.webContents.send.bind(window.webContents);
      window.webContents.send = (channel: string, ...args: unknown[]) => {
        sent.push({ channel, payload: args[0] });
        if (channel === 'embedded-browser:state') {
          const payload = args[0] as { viewId: string; state: EmbeddedBrowserState };
          states.set(payload.viewId, payload.state);
        }
        original(channel, ...args);
      };
    }
    return true;
  },
  reservePort: (port: number) => reserveLocalPort(port),
  /** The panel shows a tab: attach (or create) it, and give it room on screen. */
  open: (viewId: string, url: string) => {
    if (!window) throw new Error('not ready');
    const state = createEmbeddedBrowser(window, viewId, url, record(viewId));
    if (state) states.set(viewId, state);
    setEmbeddedBrowserBounds(window, viewId, { x: 0, y: 0, width: 1000, height: 760 });
    setEmbeddedBrowserVisible(window, viewId, true);
    return state;
  },
  release: (viewId: string) => window && releaseEmbeddedBrowser(window, viewId),
  claim: (viewId: string) => (window ? claimEmbeddedBrowser(window, viewId) : false),
  destroy: (viewId: string) => window && destroyEmbeddedBrowser(window, viewId),
  navigate: (viewId: string, url: string) =>
    window ? navigateEmbeddedBrowser(window, viewId, url) : false,
  control: (viewId: string, action: 'back' | 'forward' | 'reload' | 'stop') =>
    window ? controlEmbeddedBrowser(window, viewId, action) : false,
  state: (viewId: string) => states.get(viewId) ?? null,
  suggest: (query: string) => embeddedBrowserSuggestions(query),
  sent: (channel?: string) => sent.filter((item) => !channel || item.channel === channel),
  url: (viewId: string) => contentsOf(viewId).getURL(),
  title: (viewId: string) => contentsOf(viewId).getTitle(),
  run: (viewId: string, script: string) => contentsOf(viewId).executeJavaScript(script),
  /** A real click: mouse down and up at the element's centre, as the person makes one. */
  click: async (viewId: string, selector: string) => {
    const contents = contentsOf(viewId);
    const point = await centerOf(contents, selector);
    if (!point) throw new Error(`no ${selector}`);
    contents.focus();
    contents.sendInputEvent({
      type: 'mouseDown',
      x: point.x,
      y: point.y,
      button: 'left',
      clickCount: 1,
    });
    contents.sendInputEvent({
      type: 'mouseUp',
      x: point.x,
      y: point.y,
      button: 'left',
      clickCount: 1,
    });
    return point;
  },
  /** A click with no gesture behind it: the page's own script. */
  scriptClick: (viewId: string, selector: string) =>
    contentsOf(viewId).executeJavaScript(
      `document.querySelector(${JSON.stringify(selector)}).click()`
    ),
  key: (viewId: string, keyCode: string, modifiers: Array<'meta' | 'control' | 'shift'> = []) => {
    const contents = contentsOf(viewId);
    contents.focus();
    contents.sendInputEvent({ type: 'keyDown', keyCode, modifiers });
    contents.sendInputEvent({ type: 'keyUp', keyCode, modifiers });
  },
  windows: () =>
    BrowserWindow.getAllWindows()
      .filter((item) => item !== window && !item.isDestroyed())
      .map((item) => ({ url: item.webContents.getURL(), title: item.getTitle() })),
  cookies: async (url: string) => {
    const { session } = await import('electron');
    return (
      await session.fromPartition('persist:biorouter-embedded-browser').cookies.get({ url })
    ).map((cookie) => cookie.name);
  },
  quarantine: (file: string) => {
    if (process.platform !== 'darwin') return 'not-macos';
    try {
      return execFileSync('/usr/bin/xattr', ['-p', 'com.apple.quarantine', file]).toString().trim();
    } catch {
      return null;
    }
  },
  downloadPath: (filename: string) =>
    path.join(process.env.BIOROUTER_BROWSER_TEST_DOWNLOAD_DIR ?? '', filename),
};

(globalThis as unknown as { __biorouterBrowserTest: typeof api }).__biorouterBrowserTest = api;

app.on('window-all-closed', () => app.quit());
