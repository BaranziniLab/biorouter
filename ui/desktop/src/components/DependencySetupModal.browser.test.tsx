// @vitest-environment node
/**
 * The startup dependency prompt, measured in Chromium with the whole stylesheet (first run,
 * FR-S-7). It opens by itself at startup, before anyone has touched the window, which is the case
 * jsdom cannot see: what Chromium paints, which button is coral, and whether the first button
 * wears the keyboard ring although no key was pressed.
 *
 * The real `DependencySetupModal` is bundled with esbuild against a stubbed `window.electron` (the
 * dependency check is pushed through the same `dependency-event` channel the main process uses),
 * `styles/main.css` is compiled by Tailwind over the classes the bundle uses, and the renderer's
 * text-size pass runs. Set `DEPS_SHOTS_DIR` to also save screenshots there.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { fontScale } from '../../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../styles');
const requireFromDesktop = createRequire(resolve(here, '../../package.json'));
const SHOTS = process.env.DEPS_SHOTS_DIR;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import DependencySetupModal from './DependencySetupModal';

createRoot(document.getElementById('root')!).render(<DependencySetupModal />);
`;

/** The main process's half: the push channel, the CLI probe and the installers. */
const ELECTRON_STUB = `
window.electron = {
  on(channel, handler) {
    if (channel === 'dependency-event') window.__emit = (payload) => handler({}, payload);
    return () => {};
  },
  cliStatus: async () => window.__cli || null,
  installCli: async () => ({ success: true, output: '' }),
  installDependency: async () => ({ started: true }),
  openExternal() {},
  createChatWindow() {},
  dependencyEnvironment: async () => ({}),
};
`;

const dep = (name: string, displayName: string, installCmd: string, required = true) => ({
  name,
  displayName,
  version: null,
  installed: false,
  installCmd,
  requiresSudo: false,
  downloadUrl: `https://example.org/${name}`,
  required,
});

/** The three rows of the vision probe (`vision/firstrun/static/deps-light.png`). */
const DEPS = [
  dep('node', 'Node.js', 'brew install node'),
  dep('uv', 'uv', 'curl -LsSf https://astral.sh/uv/install.sh | sh'),
  dep('rust', 'Rust toolchain', '# Install rust via your system package manager', false),
];

const STALE_CLI = {
  bundled: '/Applications/Biorouter.app/Contents/Resources/bin/biorouter',
  onPath: true,
  pathLocation: '/usr/local/bin/biorouter',
  bundledVersion: '1.92.1',
  pathVersion: '1.90.4',
  needsUpdate: true,
  brokenOnPath: false,
};

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

describe('the startup dependency prompt in Chromium (FR-S-7)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the dependency prompt tests.');
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
    const tailwind = compiler.build(
      new Scanner({}).scanFiles([{ content: script, extension: 'js' }])
    );
    css = (await postcss([fontScale()]).process(tailwind, { from: undefined })).css;
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** Mount, push the check's result, and let the open motion settle. No input of any kind. */
  async function open(
    mode: 'light' | 'dark',
    scene: { deps?: typeof DEPS; cli?: typeof STALE_CLI; error?: string }
  ): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1440, height: 900 } });
    // Served from an origin, not `setContent`'s about:blank: the CLI card reads sessionStorage and
    // localStorage, which throw on an opaque origin (the card then never shows).
    await tab.route('http://deps.test/', (route) =>
      route.fulfill({
        contentType: 'text/html',
        body:
          `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style></head>` +
          `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`,
      })
    );
    await tab.goto('http://deps.test/');
    await tab.addScriptTag({ content: ELECTRON_STUB });
    if (scene.cli)
      await tab.evaluate((cli) => ((window as { __cli?: unknown }).__cli = cli), scene.cli);
    await tab.addScriptTag({ content: script });
    if (scene.deps) {
      await tab.waitForFunction(
        () => typeof (window as { __emit?: unknown }).__emit === 'function'
      );
      await tab.evaluate((deps) => {
        const emit = (window as unknown as { __emit: (p: unknown) => void }).__emit;
        emit({ type: 'check-results', deps });
      }, scene.deps);
    }
    await tab.waitForSelector('[role="dialog"]');
    if (scene.error) {
      await tab.evaluate((error) => {
        const emit = (window as unknown as { __emit: (p: unknown) => void }).__emit;
        emit({ type: 'install-error', dep: 'uv', error });
      }, scene.error);
    }
    await tab.waitForTimeout(450);
    return tab;
  }

  const accent = (tab: Page) =>
    tab.evaluate(() => {
      const probe = document.createElement('div');
      probe.style.backgroundColor = 'var(--background-accent)';
      document.body.append(probe);
      const out = getComputedStyle(probe).backgroundColor;
      probe.remove();
      return out;
    });

  const read = (tab: Page) =>
    tab.evaluate(() => {
      const dialog = document.querySelector('[role="dialog"]')!;
      const buttons = [...dialog.querySelectorAll('button')].map((b) => ({
        name: (b.textContent ?? '').trim() || (b.getAttribute('aria-label') ?? ''),
        ground: getComputedStyle(b).backgroundColor,
      }));
      const ringed = dialog.querySelector(':focus-visible');
      const titles = [...dialog.querySelectorAll('p.text-label')].map(
        (p) => getComputedStyle(p).fontWeight
      );
      return {
        buttons,
        ringed: ringed ? (ringed.textContent ?? ringed.tagName).trim() : null,
        focusOnDialog: document.activeElement === dialog,
        titles,
        text: dialog.textContent ?? '',
      };
    });

  const shoot = async (tab: Page, name: string) => {
    if (SHOTS) await tab.screenshot({ path: resolve(SHOTS, name) });
  };

  it('has one coral button, "Install all", and every row action off the accent', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode, { deps: DEPS });
      try {
        const coral = await accent(tab);
        const m = await read(tab);
        const coralNames = m.buttons.filter((b) => b.ground === coral).map((b) => b.name);
        expect(coralNames, `${mode} coral buttons`).toEqual(['Install all (2)']);
        // The Rust row's command is a shell comment: it has no installer, so no Install.
        expect(m.buttons.filter((b) => b.name === 'Install')).toHaveLength(2);
        // Row titles are the label role, never bolder (Arial draws 600 as bold).
        for (const weight of m.titles) expect(Number(weight)).toBeLessThan(600);
        expect(m.text).not.toMatch(/[✓✗→`]/);
        await shoot(tab, `deps-${mode}-1440.png`);
      } finally {
        await tab.close();
      }
    }
  });

  it('ends the footer Dismiss then "Install all" at the trailing edge, both 32px (FR-R2-2)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light', { deps: DEPS });
    try {
      const footer = await tab.evaluate(() => {
        const dialog = document.querySelector('[role="dialog"]')!;
        const box = dialog.getBoundingClientRect();
        const pick = (name: string) => {
          const b = [...dialog.querySelectorAll('button')].find(
            (el) => (el.textContent ?? '').trim() === name
          )!;
          const r = b.getBoundingClientRect();
          return { left: r.left, right: r.right, height: r.height };
        };
        // The panel's inner edge: its 1px hairline is not part of the footer's gutter.
        const innerRight = box.right - parseFloat(getComputedStyle(dialog).borderRightWidth);
        return { dialogRight: innerRight, dismiss: pick('Dismiss'), all: pick('Install all (2)') };
      });
      // The way out first, the primary last: what Configure, Add provider and the privacy notice do.
      expect(footer.dismiss.right).toBeLessThan(footer.all.left);
      // The primary is the footer's last control: only the 16px gutter sits past it.
      expect(footer.dialogRight - footer.all.right).toBeCloseTo(16, 0);
      // The dialog footer's 32px rung, not the 28px `sm` (FR-R1-11).
      expect(footer.dismiss.height).toBe(32);
      expect(footer.all.height).toBe(32);
    } finally {
      await tab.close();
    }
  });

  it('opens by itself with no ring on any control: focus rests on the dialog box', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const scene of [{ deps: DEPS }, { cli: STALE_CLI }] as const) {
      const tab = await open('light', scene);
      try {
        const m = await read(tab);
        expect(m.ringed, 'nothing wears the keyboard ring before a key is pressed').toBeNull();
        expect(m.focusOnDialog).toBe(true);
        // The first Tab reaches the first control, ringed: the ring is for the keyboard.
        await tab.keyboard.press('Tab');
        expect((await read(tab)).ringed).not.toBeNull();
      } finally {
        await tab.close();
      }
    }
  });

  it('keeps the CLI card with a failed row: one coral, glyphs from the registry', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light', {
      deps: DEPS,
      cli: STALE_CLI,
      error: 'curl: (6) Could not resolve host: astral.sh',
    });
    try {
      const coral = await accent(tab);
      const m = await read(tab);
      expect(m.buttons.filter((b) => b.ground === coral).map((b) => b.name)).toEqual([
        'Install all (2)',
      ]);
      expect(
        await tab.evaluate(() =>
          [...document.querySelectorAll('[role="dialog"] svg[role="img"]')].map((s) =>
            s.getAttribute('aria-label')
          )
        )
      ).toEqual(['to', 'Failed']);
      await shoot(tab, 'deps-err-cli-light-1440.png');
    } finally {
      await tab.close();
    }
  });
});
