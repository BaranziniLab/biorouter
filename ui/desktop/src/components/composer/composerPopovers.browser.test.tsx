// @vitest-environment node
/**
 * The composer's two popovers, laid out by Chromium (owner, 2026-11-19): the `+` popover (Add,
 * Extensions, Skills, Knowledge) and the model chip's menu are ONE width, the model menu's, which
 * both take from one token (`--composer-popover-width`, main.css), and at that width the `+`
 * popover's four-way switcher keeps every label whole and evenly spaced, at each Text size the
 * app offers. Their rows and separators sit alike too: the same row height, the same inset from
 * the surface's edge, and separators that run edge to edge.
 *
 * jsdom lays nothing out, so this bundles the REAL `ComposerPlusMenu` and the REAL
 * `ModelsBottomBar` with esbuild (their component stylesheets included), compiles
 * `styles/main.css` with Tailwind over the bundle's classes, runs the result through the Text size
 * scaling the renderer's build applies (`vite-plugins/fontScale.mjs`), and measures. Only the
 * modules that read the daemon are stand-ins: the three tool lists, the config and model contexts
 * and the model switch dialog. Set `COMPOSER_POPOVER_SHOTS_DIR` to save a screenshot of each
 * popover there.
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
import { FONT_SIZE_SCALE, FONT_SIZES } from '../../hooks/useFontSize';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const MAIN_CSS = readFileSync(resolve(STYLES, 'main.css'), 'utf8');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const SHOTS = process.env.COMPOSER_POPOVER_SHOTS_DIR;
const MODEL = 'gpt-5.6-sol-2026-07-09';

/** The one width the two popovers share, as main.css declares it. */
const SHARED_WIDTH = Number(MAIN_CSS.match(/\n\s*--composer-popover-width:\s*(\d+)px;/)?.[1]);

const STUBS: Record<string, string> = {
  tools: `
    export function useExtensionPicker() {
      return { activeCount: 3, visibleCount: 3, answered: true, setQuery() {} };
    }
    export function useSkillPicker() {
      return { activeCount: 2, visibleCount: 2, answered: true, setQuery() {} };
    }
    export function useKnowledgePicker() {
      return { activeCount: 1, visibleCount: 1, answered: true, setQuery() {} };
    }
    export function ExtensionsPanel() { return <p>Extensions list</p>; }
    export function SkillsPanel() { return <p>Skills list</p>; }
    export function KnowledgePanel() { return <p>Knowledge list</p>; }
  `,
  config: `
    const providers = [
      {
        name: 'versa_azure',
        is_configured: true,
        provider_type: 'Builtin',
        metadata: { name: 'versa_azure', display_name: 'Versa API Azure', tier: 'private', runs_locally: false },
        affiliation: null,
        resolved_tier: 'private',
      },
    ];
    const config = { read: async () => '', getProviders: async () => providers, providersList: providers };
    export function useConfig() { return config; }
    export function usePrivacyTiersEnabled() { return true; }
    export function usePrivacyTiersRecord() { return null; }
  `,
  model: `
    const value = {
      currentModel: '${MODEL}',
      currentProvider: 'versa_azure',
      getCurrentModelAndProviderForDisplay: async () => ({ model: '${MODEL}', provider: 'Versa API Azure' }),
      getCurrentModelDisplayName: async () => '${MODEL}',
      getCurrentProviderDisplayName: async () => 'Versa API Azure',
    };
    export function useModelAndProvider() { return value; }
  `,
  dialogs: `
    export function SwitchModelModal() { return null; }
  `,
};

const stubDaemon: Plugin = {
  name: 'stub-daemon-modules',
  setup(builder) {
    const route: Array<[RegExp, keyof typeof STUBS]> = [
      [/^\.\/BottomMenu(Extension|Skill|Knowledge)Selection$/, 'tools'],
      [/(^|\/)ConfigContext$/, 'config'],
      [/(^|\/)ModelAndProviderContext$/, 'model'],
      [/^\.\.\/subcomponents\/SwitchModelModal$/, 'dialogs'],
    ];
    builder.onResolve({ filter: /.*/ }, (args) => {
      const hit = route.find(([filter]) => filter.test(args.path));
      return hit ? { path: hit[1], namespace: 'daemon-stub' } : undefined;
    });
    builder.onLoad({ filter: /.*/, namespace: 'daemon-stub' }, ({ path }) => ({
      contents: STUBS[path],
      loader: 'tsx',
      resolveDir: here,
    }));
  },
};

/** The composer's two popover triggers, near the window's bottom edge as in a chat. */
const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { AppTooltipProvider } from '../ui/Tooltip';
import { ComposerPlusMenu } from './ComposerPlusMenu';
import ModelsBottomBar from '../settings/models/bottom_bar/ModelsBottomBar';

function App() {
  return (
    <AppTooltipProvider>
      <div style={{ position: 'fixed', left: 40, bottom: 64 }}>
        <ComposerPlusMenu
          sessionId="chat-1"
          onAttachFiles={() => {}}
          onInsertTrigger={() => {}}
          onOpenDecisionMonitors={() => {}}
        />
      </div>
      <div style={{ position: 'fixed', right: 40, bottom: 64 }}>
        <ModelsBottomBar
          sessionId="chat-1"
          privacyTier="private"
          setView={() => {}}
          reasoningScope="session:chat-1"
          hideAlertPopover
        />
      </div>
    </AppTooltipProvider>
  );
}

createRoot(document.getElementById('root')!).render(<App />);
`;

async function bundle(): Promise<{ script: string; css: string }> {
  const out = await build({
    stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
    bundle: true,
    write: false,
    // Only names the outputs (nothing is written): the component stylesheets come back as CSS.
    outdir: resolve(here, '.composer-popovers-bundle'),
    format: 'iife',
    jsx: 'automatic',
    platform: 'browser',
    define: {
      'process.env.NODE_ENV': '"production"',
      'import.meta.env': JSON.stringify({ MODE: 'production', DEV: false, PROD: true }),
    },
    loader: {
      '.css': 'css',
      '.svg': 'dataurl',
      '.png': 'dataurl',
      '.woff': 'empty',
      '.woff2': 'empty',
      '.ttf': 'empty',
    },
    logLevel: 'silent',
    plugins: [stubDaemon],
  });
  const script = out.outputFiles.find((f) => f.path.endsWith('.js'))!.text;
  const components = out.outputFiles.find((f) => f.path.endsWith('.css'))?.text ?? '';
  if (!components.includes('.br-plus-popover') || !components.includes('.br-model-menu')) {
    throw new Error('the bundle carried no composer.css or pickers.css: the popovers are unstyled');
  }
  const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
  const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
  const compiler = await compile(MAIN_CSS, { base: STYLES, onDependency() {} });
  const tailwind = compiler.build(
    new Scanner({}).scanFiles([{ content: script, extension: 'js' }])
  );
  // main.css first and the component sheets after, the order the renderer loads them in, through
  // the Text size scaling the renderer's own build applies.
  const sheet = await postcss([fontScale()]).process(tailwind + components, { from: undefined });
  return { script, css: sheet.css };
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

type Box = { left: number; right: number; top: number; bottom: number; width: number };

const PLUS = '[data-testid="composer-plus"]';
const PLUS_POPOVER = '[data-testid="composer-plus-popover"]';
const MODEL_CHIP = '[data-testid="model-chip"]';
const MODEL_MENU = '[role="menu"]';

describe('the composer’s two popovers', () => {
  let browser: Browser | null = null;
  let page = { script: '', css: '' };

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the composer popover geometry cases.');
      return;
    }
    page = await bundle();
  }, 240_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function load(scale = 1, dark = false): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1000, height: 720 } });
    // The open keyframes move and scale the surface; measure the layout, not the motion.
    await tab.emulateMedia({ reducedMotion: 'reduce' });
    const errors: string[] = [];
    tab.on('pageerror', (error) => errors.push(error.message));
    await tab.setContent(
      `<!doctype html><html${dark ? ' class="dark"' : ''} style="--app-font-scale: ${scale}">` +
        `<head><style>${page.css}</style></head>` +
        `<body style="background:var(--background-default)"><div id="root"></div>` +
        `<script>${page.script}</script></body></html>`
    );
    await tab.waitForSelector(PLUS);
    await tab.waitForSelector(MODEL_CHIP);
    expect(errors, 'the page renders without an error').toEqual([]);
    return tab;
  }

  const box = (tab: Page, selector: string) =>
    tab.evaluate((sel) => {
      const r = document.querySelector(sel)!.getBoundingClientRect();
      return { left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width };
    }, selector) as Promise<Box>;

  async function openPlus(tab: Page) {
    await tab.click(PLUS);
    await tab.waitForSelector(`${PLUS_POPOVER} [role="radiogroup"]`);
    await tab.waitForTimeout(150);
  }

  async function openModelMenu(tab: Page) {
    await tab.click(MODEL_CHIP);
    await tab.waitForSelector(MODEL_MENU);
    await tab.waitForTimeout(150);
  }

  async function close(tab: Page, selector: string) {
    await tab.keyboard.press('Escape');
    await tab.waitForSelector(selector, { state: 'detached', timeout: 3000 });
  }

  /**
   * How a surface's rows and separators sit inside it: each row's height and its inset from the
   * surface's border edge, and each separator's span against the surface's inner width.
   */
  const rows = (tab: Page, surface: string, row: string, separator: string) =>
    tab.evaluate(
      ({ surface, row, separator }) => {
        const node = document.querySelector<HTMLElement>(surface)!;
        const outer = node.getBoundingClientRect();
        const innerLeft = outer.left + node.clientLeft;
        const innerWidth = node.clientWidth;
        return {
          rows: [...node.querySelectorAll<HTMLElement>(row)].map((element) => {
            const r = element.getBoundingClientRect();
            return {
              height: r.height,
              inset: r.left - innerLeft,
              end: innerLeft + innerWidth - r.right,
            };
          }),
          separators: [...node.querySelectorAll<HTMLElement>(separator)].map((element) => {
            const r = element.getBoundingClientRect();
            return { left: r.left - innerLeft, width: r.width, innerWidth, height: r.height };
          }),
        };
      },
      { surface, row, separator }
    );

  it('opens the + popover and the model menu at one width, the token both take', async (ctx) => {
    if (!browser) return ctx.skip();
    expect(SHARED_WIDTH, 'main.css declares --composer-popover-width in px').toBeGreaterThan(0);
    const tab = await load();
    try {
      await openModelMenu(tab);
      const model = await box(tab, MODEL_MENU);
      if (SHOTS) await tab.screenshot({ path: resolve(SHOTS, 'model-menu.png') });
      await close(tab, MODEL_MENU);

      await openPlus(tab);
      const plus = await box(tab, PLUS_POPOVER);
      if (SHOTS) await tab.screenshot({ path: resolve(SHOTS, 'plus-popover.png') });

      expect(Math.abs(model.width - SHARED_WIDTH), `model menu ${model.width}px`).toBeLessThan(0.5);
      expect(Math.abs(plus.width - SHARED_WIDTH), `+ popover ${plus.width}px`).toBeLessThan(0.5);
      expect(Math.abs(plus.width - model.width), 'one width').toBeLessThan(0.5);
    } finally {
      await tab.close();
    }
  });

  it('sets the + popover’s Add rows and separator as the model menu sets its own', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await load();
    try {
      await openModelMenu(tab);
      const model = await rows(
        tab,
        MODEL_MENU,
        '[role="menuitem"]',
        '[data-slot="dropdown-menu-separator"]'
      );
      await close(tab, MODEL_MENU);
      await openPlus(tab);
      const plus = await rows(tab, PLUS_POPOVER, '[role="menuitem"]', '.br-plus-add__separator');
      expect(model.rows.length, 'the model menu has a row').toBeGreaterThan(0);
      expect(plus.rows.length, 'Add has its four rows').toBe(4);
      const [reference] = model.rows;
      for (const row of plus.rows) {
        expect(Math.abs(row.height - reference.height), 'the same row height').toBeLessThan(0.5);
        expect(Math.abs(row.inset - reference.inset), 'the same inset').toBeLessThan(0.5);
        expect(Math.abs(row.end - reference.end), 'the same trailing inset').toBeLessThan(0.5);
      }
      expect(model.separators.length).toBeGreaterThan(0);
      expect(plus.separators.length).toBe(1);
      for (const line of [...model.separators, ...plus.separators]) {
        expect(line.height, 'a hairline').toBeCloseTo(1, 1);
        expect(Math.abs(line.left), 'from the surface’s inner edge').toBeLessThan(0.5);
        expect(Math.abs(line.width - line.innerWidth), 'to the other edge').toBeLessThan(0.5);
      }
    } finally {
      await tab.close();
    }
  });

  for (const size of FONT_SIZES) {
    for (const dark of [false, true]) {
      it(`keeps the four segments’ labels whole and evenly spaced at the ${size} Text size${dark ? ', dark' : ''}`, async (ctx) => {
        if (!browser) return ctx.skip();
        const tab = await load(FONT_SIZE_SCALE[size], dark);
        try {
          await openPlus(tab);
          if (SHOTS) {
            await tab.screenshot({
              path: resolve(SHOTS, `plus-segments-${size}${dark ? '-dark' : ''}.png`),
            });
          }
          const measure = () =>
            tab.evaluate((surface) => {
              const box = (element: Element) => {
                const r = element.getBoundingClientRect();
                return {
                  left: r.left,
                  right: r.right,
                  top: r.top,
                  bottom: r.bottom,
                  width: r.width,
                };
              };
              const popover = document.querySelector<HTMLElement>(surface)!;
              const track = popover.querySelector<HTMLElement>('[role="radiogroup"]')!;
              const trackStyle = getComputedStyle(track);
              const trackBox = box(track);
              const thumb = track.querySelector<HTMLElement>('[data-slot="segmented-thumb"]')!;
              return {
                popover: box(popover),
                track: {
                  ...trackBox,
                  innerLeft: trackBox.left + parseFloat(trackStyle.paddingLeft),
                  innerRight: trackBox.right - parseFloat(trackStyle.paddingRight),
                },
                thumb: box(thumb),
                segments: [...track.querySelectorAll<HTMLElement>('[role="radio"]')].map(
                  (segment) => {
                    const label = segment.querySelector<HTMLElement>('.br-segmented-label')!;
                    const own = box(segment);
                    const centre = { x: (own.left + own.right) / 2, y: (own.top + own.bottom) / 2 };
                    return {
                      name: label.textContent,
                      checked: segment.getAttribute('data-state') === 'checked',
                      box: own,
                      label: box(label),
                      clipped: label.scrollWidth > label.clientWidth,
                      hit:
                        document.elementFromPoint(centre.x, centre.y)?.closest('[role="radio"]') ===
                        segment,
                    };
                  }
                ),
              };
            }, PLUS_POPOVER);

          const first = await measure();
          expect(first.segments.map((segment) => segment.name)).toEqual([
            'Add',
            'Extensions',
            'Skills',
            'Knowledge',
          ]);
          // Inside the popover.
          expect(first.track.left).toBeGreaterThanOrEqual(first.popover.left);
          expect(first.track.right).toBeLessThanOrEqual(first.popover.right);
          const lefts: number[] = [];
          for (const [index, segment] of first.segments.entries()) {
            const where = `${size}: ${segment.name}`;
            // Whole: nothing cut, the label's box inside its segment, and the segment on screen.
            expect(segment.clipped, `${where} is not cut short`).toBe(false);
            expect(segment.label.left, where).toBeGreaterThanOrEqual(segment.box.left - 0.5);
            expect(segment.label.right, where).toBeLessThanOrEqual(segment.box.right + 0.5);
            expect(segment.hit, `${where} answers a click at its centre`).toBe(true);
            // Tiled across the track: no gap and no overlap between neighbours.
            const before =
              index === 0 ? first.track.innerLeft : first.segments[index - 1].box.right;
            expect(
              Math.abs(segment.box.left - before),
              `${where} meets its neighbour`
            ).toBeLessThan(1);
            // Centred in its segment, so the room between two labels is the same everywhere.
            const left = segment.label.left - segment.box.left;
            const right = segment.box.right - segment.label.right;
            expect(Math.abs(left - right), `${where} is centred`).toBeLessThan(1);
            lefts.push(left);
          }
          const last = first.segments[first.segments.length - 1];
          expect(
            Math.abs(last.box.right - first.track.innerRight),
            'to the track’s end'
          ).toBeLessThan(1);
          expect(Math.max(...lefts) - Math.min(...lefts), `${size}: even spacing`).toBeLessThan(1);
          // At least 4px of air either side of every label at the standard size, which is the
          // segment's own padding; the leftover room only adds to it.
          if (size === 'standard') expect(Math.min(...lefts)).toBeGreaterThanOrEqual(4);
          // The thumb sits under the chosen segment, at its width.
          const chosen = first.segments.find((segment) => segment.checked)!;
          expect(Math.abs(first.thumb.left - chosen.box.left)).toBeLessThan(1);
          expect(Math.abs(first.thumb.width - chosen.box.width)).toBeLessThan(1);

          // Choosing another list moves the thumb and nothing else on the control.
          await tab.click('[data-testid="tools-segment-knowledge"]');
          await tab.waitForTimeout(300);
          const after = await measure();
          for (const [index, segment] of after.segments.entries()) {
            const was = first.segments[index].box;
            expect(Math.abs(segment.box.left - was.left), `${segment.name} stays`).toBeLessThan(
              0.5
            );
            expect(
              Math.abs(segment.box.width - was.width),
              `${segment.name} keeps its width`
            ).toBeLessThan(0.5);
            expect(segment.clipped).toBe(false);
          }
          const knowledge = after.segments[3];
          expect(knowledge.checked).toBe(true);
          expect(Math.abs(after.thumb.left - knowledge.box.left)).toBeLessThan(1);
          expect(Math.abs(after.thumb.width - knowledge.box.width)).toBeLessThan(1);
        } finally {
          await tab.close();
        }
      });
    }
  }
});
