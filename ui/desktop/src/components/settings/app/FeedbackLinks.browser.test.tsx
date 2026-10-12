// @vitest-environment node
/**
 * Settings > App > About > Feedback, in Chromium with the whole stylesheet (vision round 6,
 * V4-R6-2).
 *
 * "Report a bug" and "Request a feature" open GitHub in the default browser, and were plain ghost
 * buttons: nothing said they leave the app, while the other exits in Settings (the Switch models
 * quick start guide, View info's official URL) carry the external-link glyph. Each now ends in the
 * same 14px glyph, in the muted ink, and is described "Opens in your browser".
 *
 * jsdom lays nothing out and applies no Tailwind, so the REAL `FeedbackLinks` is bundled with
 * esbuild, `styles/main.css` is compiled by Tailwind over the classes the bundle uses (and then by
 * the renderer's text-size pass), and Chromium paints it beside a reference exit drawn the way the
 * Switch models footer draws its own. What is read: the glyph's size and place against the label,
 * its ink against the muted token, the button's height, that the two buttons stay on one line and
 * inside the row at a narrow window, and the accessible description as the browser computes it.
 *
 * "The instrument" proves the cases can fail. Three mutated bundles: `h-3.5 w-3.5` in place of
 * `size-3.5` (the button primitive's own `[&_svg:not([class*='size-'])]:size-4` outranks a plain
 * height and width, so the glyph comes out 16px), the small button's own 6px gap kept (the
 * glyph hangs off its label), and no glyph at all. A mutation that matches
 * nothing throws, so a refactor cannot turn a case into a no-op.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { fontScale } from '../../../../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../../package.json'));

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { SettingRow, SettingSection } from '../../ui/setting-row';
import { ExternalLink } from '../../icons/app-icons';
import FeedbackLinks from './FeedbackLinks';

function App() {
  const width = (window as unknown as { __frameWidth?: number }).__frameWidth ?? 640;
  return (
    <div style={{ padding: 24, width }} data-testid="frame">
      <SettingSection title="About">
        <SettingRow label="Version" value="1.92.1" valueMono />
        <SettingRow label="Feedback" data-testid="feedback">
          <FeedbackLinks />
        </SettingRow>
      </SettingSection>
      {/* An exit the way the Switch models footer draws its own (SwitchModelModal). */}
      <a href="#guide" data-testid="reference"
        className="inline-flex items-center gap-1 text-secondary text-text-muted">
        <ExternalLink size={14} />
        Quick start guide
      </a>
      <span style={{ position: 'absolute', visibility: 'hidden' }}>
        <span data-testid="ink-muted" className="text-text-muted">m</span>
        <span data-testid="ink-default" className="text-text-default">d</span>
      </span>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<App />);
`;

const GLYPH = '<ExternalLink aria-hidden="true" className="size-3.5 text-text-muted" />';

type Mutation = 'plain-height-width' | 'no-glyph' | 'button-gap';

/** The source with one defect put back. Must match, or throw. */
const MUTATIONS: Record<Mutation, [string, string]> = {
  'plain-height-width': [GLYPH, GLYPH.replace('size-3.5', 'h-3.5 w-3.5')],
  'no-glyph': [GLYPH, '{null}'],
  'button-gap': ['className="gap-1"', 'className=""'],
};

async function bundle(mutation?: Mutation): Promise<string> {
  const mutate: Plugin = {
    name: 'mutate',
    setup(b) {
      if (!mutation) return;
      b.onLoad({ filter: /\/FeedbackLinks\.tsx$/ }, (args) => {
        const [from, to] = MUTATIONS[mutation];
        const source = readFileSync(args.path, 'utf8');
        if (!source.includes(from))
          throw new Error(`mutation target missing in ${args.path}: ${from}`);
        return {
          contents: source.split(from).join(to),
          loader: 'tsx',
          resolveDir: dirname(args.path),
        };
      });
    },
  };
  const out = await build({
    stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
    bundle: true,
    write: false,
    format: 'iife',
    jsx: 'automatic',
    platform: 'browser',
    define: { 'process.env.NODE_ENV': '"production"' },
    loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
    logLevel: 'silent',
    plugins: [mutate],
  });
  return out.outputFiles[0].text;
}

/** The app's stylesheet, compiled over the classes `script` uses and then text-sized. */
async function stylesheetFor(script: string): Promise<string> {
  const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
  const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
  const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
    base: STYLES,
    onDependency() {},
  });
  const tailwind = compiler.build(
    new Scanner({}).scanFiles([{ content: script, extension: 'js' }])
  );
  return (await postcss([fontScale()]).process(tailwind, { from: undefined })).css;
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

type Measure = {
  buttons: Array<{
    name: string;
    height: number;
    top: number;
    left: number;
    right: number;
    scrollWidth: number;
    clientWidth: number;
    glyphs: number;
    glyph: null | {
      width: number;
      height: number;
      left: number;
      centreY: number;
      color: string;
      ariaHidden: string | null;
      isLast: boolean;
    };
    labelRight: number;
    labelCentreY: number;
    labelColor: string;
    describedBy: string | null;
    describedByText: string | null;
    describedByDisplay: string | null;
  }>;
  referenceGlyph: { width: number; height: number };
  referenceGap: number;
  muted: string;
  ink: string;
  rowRight: number;
  rowHeight: number;
  groupRight: number;
  descriptionCount: number;
};

type Axe = { name: string; description: string };

describe('Feedback exits (V4-R6-2)', () => {
  let browser: Browser | null = null;
  const pages: Record<'fixed' | Mutation, { script: string; css: string }> = {} as never;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the feedback exit cases.');
      return;
    }
    for (const key of ['fixed', 'plain-height-width', 'no-glyph', 'button-gap'] as const) {
      const script = await bundle(key === 'fixed' ? undefined : key);
      pages[key] = { script, css: await stylesheetFor(script) };
    }
  }, 240_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(
    key: 'fixed' | Mutation,
    look: { mode?: 'light' | 'dark'; width?: number; frame?: number } = {}
  ): Promise<Page> {
    const tab = await browser!.newPage({
      viewport: { width: look.width ?? 720, height: 420 },
      deviceScaleFactor: 2,
    });
    await tab.setContent(
      `<!doctype html><html class="${look.mode === 'dark' ? 'dark' : ''}" ` +
        `style="--app-font-scale: 1"><head><style>${pages[key].css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    if (look.frame) {
      await tab.evaluate((w) => {
        (window as unknown as { __frameWidth: number }).__frameWidth = w;
      }, look.frame);
    }
    await tab.addScriptTag({ content: pages[key].script });
    await tab.waitForSelector('[data-testid="feedback"] button');
    await tab.waitForTimeout(150);
    return tab;
  }

  const measure = (tab: Page): Promise<Measure> =>
    tab.evaluate(() => {
      const row = document.querySelector('[data-testid="feedback"]')!;
      const group = row.querySelector('[role="group"]')!;
      const rect = (el: Element) => el.getBoundingClientRect();
      const labelOf = (button: Element) => {
        const range = document.createRange();
        const text = Array.from(button.childNodes).find((n) => n.nodeType === Node.TEXT_NODE)!;
        range.selectNodeContents(text);
        return rect(range as unknown as Element);
      };
      const buttons = Array.from(row.querySelectorAll('button')).map((button) => {
        const svgs = Array.from(button.querySelectorAll('svg'));
        const svg = svgs[0] ?? null;
        const label = labelOf(button);
        const describedBy = button.getAttribute('aria-describedby');
        const target = describedBy ? document.getElementById(describedBy) : null;
        return {
          name: button.textContent ?? '',
          height: rect(button).height,
          top: rect(button).top,
          left: rect(button).left,
          right: rect(button).right,
          scrollWidth: button.scrollWidth,
          clientWidth: button.clientWidth,
          glyphs: svgs.length,
          glyph: svg && {
            width: rect(svg).width,
            height: rect(svg).height,
            left: rect(svg).left,
            centreY: rect(svg).top + rect(svg).height / 2,
            color: getComputedStyle(svg).color,
            ariaHidden: svg.getAttribute('aria-hidden'),
            isLast: button.lastElementChild === svg,
          },
          labelRight: label.right,
          labelCentreY: label.top + label.height / 2,
          labelColor: getComputedStyle(button).color,
          describedBy,
          describedByText: target?.textContent ?? null,
          describedByDisplay: target ? getComputedStyle(target).display : null,
        };
      });
      const reference = document.querySelector('[data-testid="reference"] svg')!;
      const referenceText = document.createRange();
      referenceText.selectNodeContents(reference.nextSibling!);
      return {
        buttons,
        referenceGlyph: { width: rect(reference).width, height: rect(reference).height },
        referenceGap: referenceText.getBoundingClientRect().left - rect(reference).right,
        muted: getComputedStyle(document.querySelector('[data-testid="ink-muted"]')!).color,
        ink: getComputedStyle(document.querySelector('[data-testid="ink-default"]')!).color,
        rowRight: rect(row).right,
        rowHeight: rect(row).height,
        groupRight: rect(group).right,
        descriptionCount: row.querySelectorAll('[hidden]').length,
      };
    });

  /** The browser's own computed name and description for every button on the page. */
  async function accessibility(tab: Page): Promise<Axe[]> {
    const cdp = await tab.context().newCDPSession(tab);
    await cdp.send('Accessibility.enable');
    const { nodes } = (await cdp.send('Accessibility.getFullAXTree')) as {
      nodes: Array<{
        ignored?: boolean;
        role?: { value?: string };
        name?: { value?: string };
        description?: { value?: string };
      }>;
    };
    return nodes
      .filter((node) => !node.ignored && node.role?.value === 'button')
      .map((node) => ({
        name: node.name?.value ?? '',
        description: node.description?.value ?? '',
      }));
  }

  it('draws the glyph after each label at the size and ink of the other exits', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open('fixed', { mode });
      try {
        const m = await measure(tab);
        expect(
          m.buttons.map((b) => b.name),
          mode
        ).toEqual(['Report a bug', 'Request a feature']);
        for (const b of m.buttons) {
          const tag = `${mode} ${b.name}`;
          expect(b.glyphs, tag).toBe(1);
          const g = b.glyph!;
          // The size of the reference exit's glyph, which is 14px.
          expect(m.referenceGlyph.width, tag).toBeCloseTo(14, 1);
          expect(g.width, tag).toBeCloseTo(m.referenceGlyph.width, 1);
          expect(g.height, tag).toBeCloseTo(m.referenceGlyph.height, 1);
          // After the label, a gap apart, and last in the button.
          expect(g.isLast, tag).toBe(true);
          // The label-to-glyph gap the reference exit spaces its own with: 4px.
          expect(m.referenceGap, tag).toBeCloseTo(4, 1);
          expect(g.left - b.labelRight, tag).toBeCloseTo(m.referenceGap, 0);
          // On the label's line and the button's centre.
          expect(Math.abs(g.centreY - b.labelCentreY), tag).toBeLessThanOrEqual(1.5);
          expect(Math.abs(g.centreY - (b.top + b.height / 2)), tag).toBeLessThanOrEqual(1);
          // The button keeps the control ladder's 28px.
          expect(b.height, tag).toBeCloseTo(28, 1);
          // Muted ink, the token the reference exit uses, and quieter than the label.
          expect(g.color, tag).toBe(m.muted);
          expect(g.color, tag).not.toBe(b.labelColor);
          expect(b.labelColor, tag).toBe(m.ink);
          expect(g.ariaHidden, tag).toBe('true');
        }
      } finally {
        await tab.close();
      }
    }
  });

  it('keeps both exits on one line, apart, and inside the row at a narrow window', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const frame of [640, 400, 360]) {
      const tab = await open('fixed', { frame, width: frame + 48 });
      try {
        const m = await measure(tab);
        const [a, b] = m.buttons;
        const tag = `frame ${frame}`;
        expect(Math.abs(a.top - b.top), tag).toBeLessThanOrEqual(0.5);
        expect(b.left - a.right, tag).toBeGreaterThanOrEqual(0);
        expect(b.right, tag).toBeLessThanOrEqual(m.rowRight + 0.5);
        expect(m.groupRight, tag).toBeLessThanOrEqual(m.rowRight + 0.5);
        for (const button of m.buttons) {
          // No label clipped by the glyph taking room.
          expect(button.scrollWidth, `${tag} ${button.name}`).toBeLessThanOrEqual(
            button.clientWidth + 1
          );
        }
      } finally {
        await tab.close();
      }
    }
  });

  it('describes both exits "Opens in your browser" without renaming them', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('fixed');
    try {
      const m = await measure(tab);
      // One hidden element both buttons point at: no line of text in the row.
      expect(m.descriptionCount).toBe(1);
      const [a, b] = m.buttons;
      expect(a.describedBy).not.toBeNull();
      expect(b.describedBy).toBe(a.describedBy);
      expect(a.describedByText).toBe('Opens in your browser');
      expect(a.describedByDisplay).toBe('none');

      const axe = await accessibility(tab);
      for (const name of ['Report a bug', 'Request a feature']) {
        const node = axe.find((n) => n.name === name);
        expect(node, name).toBeDefined();
        expect(node!.description, name).toBe('Opens in your browser');
      }
    } finally {
      await tab.close();
    }
  });

  describe('the instrument', () => {
    it('with a plain h-3.5 w-3.5 the button primitive draws the glyph at 16px', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open('plain-height-width');
      try {
        const m = await measure(tab);
        for (const b of m.buttons) {
          expect(b.glyph!.width, b.name).toBeCloseTo(16, 1);
          expect(b.glyph!.width, b.name).not.toBeCloseTo(m.referenceGlyph.width, 1);
        }
      } finally {
        await tab.close();
      }
    });

    it("with the small button's own 6px gap the glyph hangs off the label", async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open('button-gap');
      try {
        const m = await measure(tab);
        for (const b of m.buttons) {
          expect(b.glyph!.left - b.labelRight, b.name).toBeCloseTo(6, 0);
          expect(b.glyph!.left - b.labelRight, b.name).not.toBeCloseTo(m.referenceGap, 0);
        }
      } finally {
        await tab.close();
      }
    });

    it('with no glyph the buttons carry nothing that says they leave the app', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open('no-glyph');
      try {
        const m = await measure(tab);
        for (const b of m.buttons) expect(b.glyphs, b.name).toBe(0);
      } finally {
        await tab.close();
      }
    });
  });
});
