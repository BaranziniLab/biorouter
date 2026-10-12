// @vitest-environment node
/**
 * The typed confirmation has the one dialog anatomy, measured in Chromium with the whole
 * stylesheet (vision round 7, V3-R7-1; spec 2.6; coordinator ruling 1).
 *
 * `DangerousConfirmDialog` was the last confirmation on a raw `DialogContent`: centred on the
 * window (its top edge at y=262 in a 900px window, where every `ModalShell` dialog's is at 90),
 * 24px insets, the title in the body and no header rule. So "Make "Soul" public?" and "Delete
 * Meditation?", both opened from the Knowledge band, appeared at two heights with two anatomies.
 * jsdom lays nothing out, so this file bundles the real component (and the real
 * `ConfirmationModal`, the yardstick) with esbuild, compiles `styles/main.css` with Tailwind over
 * the classes the bundle uses, runs the renderer's text-size pass, and measures both in light and
 * in dark.
 *
 * To see these cases fail on another version of the component, point `DANGEROUS_SOURCE` at that
 * file: it is loaded in place of `./DangerousConfirmDialog`, its imports resolved from this
 * directory.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { fontScale } from '../../../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

const sourcePlugin: Plugin = {
  name: 'dangerous-source',
  setup(b) {
    const source = process.env.DANGEROUS_SOURCE;
    if (source) {
      b.onResolve({ filter: /^\.\/DangerousConfirmDialog$/ }, () => ({
        path: source,
        namespace: 'dangerous-source',
      }));
      b.onLoad({ filter: /.*/, namespace: 'dangerous-source' }, (args) => ({
        contents: readFileSync(args.path, 'utf8'),
        resolveDir: here,
        loader: 'tsx',
      }));
    }
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

/** `?kind=` picks the dialog: the Knowledge tier confirmation, or the plain confirmation. */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { DangerousConfirmDialog } from './DangerousConfirmDialog';
import { ConfirmationModal } from './ConfirmationModal';

const kind = (window as unknown as { __kind: string }).__kind;

function App() {
  if (kind === 'plain') {
    return (
      <ConfirmationModal
        isOpen
        title="Delete Meditation?"
        message="This deletes the knowledge base and its history. This cannot be undone."
        confirmLabel="Delete"
        cancelLabel="Cancel"
        confirmVariant="destructive"
        onConfirm={() => {}}
        onCancel={() => {}}
      />
    );
  }
  return (
    <DangerousConfirmDialog
      open
      title={'Make "Soul" public?'}
      description={
        'Every model, including public ones, will be able to read 0 pages and 0 raw sources, ' +
        'including anything a private chat wrote into it. This cannot be undone for content ' +
        'that has already been read.'
      }
      phrase="soul"
      fieldLabel="Type this knowledge base's ID to confirm"
      confirmLabel="Make public"
      onConfirm={() => {}}
      onCancel={() => {}}
    >
      <div className="rounded-element border border-transparent bg-background-fill p-3">
        <p className="truncate text-label text-text-default">Soul</p>
        <p className="mt-0.5 font-mono text-supporting text-text-muted">soul</p>
      </div>
    </DangerousConfirmDialog>
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

type Box = { x: number; y: number; width: number; height: number; right: number; bottom: number };

type Measure = {
  panel: Box;
  /** The panel's border, so an inset is read from inside it. */
  border: number;
  heading: Box;
  /** Whether the heading is in the panel's first band, the header. */
  headingInHeader: boolean;
  headerRule: number;
  description: Box | null;
  field: Box | null;
  buttons: { name: string; box: Box }[];
};

const MODES = ['light', 'dark'] as const;

describe('the typed confirmation is the shared dialog (V3-R7-1)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the typed confirmation geometry tests.');
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
      loader: { '.svg': 'dataurl', '.png': 'dataurl' },
      plugins: [sourcePlugin],
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

  async function open(kind: 'dangerous' | 'plain', mode: (typeof MODES)[number]): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1440, height: 900 } });
    await tab.setContent(
      `<!doctype html><html class="${mode}" style="--app-font-scale: 1"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div>` +
        `<script>window.__kind = ${JSON.stringify(kind)};</script></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[role="dialog"]');
    // The open motion (250ms) settles before anything is read.
    await tab.waitForTimeout(400);
    return tab;
  }

  const measure = (tab: Page): Promise<Measure> =>
    tab.evaluate(() => {
      const box = (el: Element) => {
        const r = el.getBoundingClientRect();
        return {
          x: r.left,
          y: r.top,
          width: r.width,
          height: r.height,
          right: r.right,
          bottom: r.bottom,
        };
      };
      const panel = document.querySelector('[role="dialog"]')!;
      const heading = panel.querySelector('h2')!;
      const header = panel.firstElementChild!;
      const describedBy = panel.getAttribute('aria-describedby');
      const description = describedBy ? document.getElementById(describedBy) : null;
      const field = panel.querySelector('input');
      const buttons = [...panel.querySelectorAll('button')]
        .filter((b) => b.textContent && b.textContent.trim() !== 'Close')
        .map((b) => ({ name: b.textContent!.trim(), box: box(b) }));
      return {
        panel: box(panel),
        border: (panel as HTMLElement).clientLeft,
        heading: box(heading),
        headingInHeader: header.contains(heading),
        headerRule: parseFloat(getComputedStyle(header).borderBottomWidth),
        description: description ? box(description) : null,
        field: field ? box(field) : null,
        buttons,
      };
    });

  for (const mode of MODES) {
    it(`sits where a ModalShell confirmation sits, at the same width (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const plainTab = await open('plain', mode);
      const plain = await measure(plainTab);
      await plainTab.close();
      const tab = await open('dangerous', mode);
      const typed = await measure(tab);
      await tab.close();
      // The yardstick first: ModalShell's top anchor is max(10vh, 48px) = 90 in a 900px window.
      expect(plain.panel.y).toBeCloseTo(90, 0);
      expect(typed.panel.y).toBeCloseTo(plain.panel.y, 0);
      expect(typed.panel.width).toBeCloseTo(plain.panel.width, 0);
      expect(typed.panel.x).toBeCloseTo(plain.panel.x, 0);
    });

    it(`has the header rule, 16px insets and 32px footer buttons (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open('dangerous', mode);
      const m = await measure(tab);
      await tab.close();
      const inner = m.panel.x + m.border;
      // The title is in the header band, above its hairline, at the 16px gutter.
      expect(m.headingInHeader).toBe(true);
      expect(m.headerRule).toBe(1);
      expect(m.heading.x - inner).toBeCloseTo(16, 0);
      expect(m.heading.y - (m.panel.y + m.border)).toBeCloseTo(16, 0);
      // The message is the description, in the body, on the same gutter.
      expect(m.description).not.toBeNull();
      expect(m.description!.x - inner).toBeCloseTo(16, 0);
      expect(m.description!.y).toBeGreaterThan(m.heading.bottom);
      // The field and its phrase stay inside the gutters at this width.
      expect(m.field).not.toBeNull();
      expect(m.field!.x - inner).toBeCloseTo(16, 0);
      expect(m.panel.right - m.border - 16 - m.field!.right).toBeGreaterThan(0);
      // Cancel, then the destructive action at the trailing edge, both on the 32px rung.
      expect(m.buttons.map((b) => b.name)).toEqual(['Cancel', 'Make public']);
      for (const { box } of m.buttons) expect(box.height).toBeCloseTo(32, 0);
      const action = m.buttons[1].box;
      expect(m.panel.right - m.border - action.right).toBeCloseTo(16, 0);
      expect(m.panel.bottom - m.border - action.bottom).toBeCloseTo(16, 0);
    });
  }
});
