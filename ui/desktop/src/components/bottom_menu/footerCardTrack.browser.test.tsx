// @vitest-environment node
/**
 * The context card's empty track against the card it sits in, in every theme family and mode,
 * under the whole stylesheet in Chromium.
 *
 * The card is a popover surface, and a dark popover lifts its ground one step (`.dark
 * .biorouter-popover-surface` re-points `--background-default` to `--background-medium`, #2c2c29).
 * The neutral progress track fell back to `--background-muted` (#232320) there, 1.13:1 off the
 * card: the empty part of the bar was all but invisible. Light measured 1.10:1. The footer card now
 * grounds its tracks the way the grounds note says a track inside a surface is grounded, one step
 * deeper than the card (`--br-track-ground` on `.br-footer-card`, pickers.css): the strong step,
 * which the dark popover re-points to a well (#1b1b19) for exactly this.
 *
 * THE FLOOR, 1.18:1, is the popover's own precedent for a track a mark rides: the off Switch's
 * track (`--control-track-off`, which the dark popover sinks to the same well, and which the
 * Slider primitive's rail paints) measures 1.19:1 on a light popover and 1.23:1 on a dark one.
 * The fills' "visible step" floor in `check-contrast.mjs`, 1.08, is too low a bar here: the
 * faint track the owner reported cleared it. The ceiling, 1.5, keeps the track a ground under
 * the bar rather than a second bar, the same ceiling the strong step owes a dark popover.
 *
 * The bar's coloured fill (green, then yellow, then red) and the auto-compact marker must still
 * read on the deeper track. Both owe SC 1.4.11's 3:1 to the track (the fill measured 4.2 to
 * 11.7:1 on the old one, and a deeper light track costs it about a tenth), and the marker owes
 * it to the card as well.
 *
 * jsdom runs no cascade, so the real `ContextWindowIndicator` is bundled with esbuild,
 * `styles/main.css` is compiled by Tailwind with the classes the bundle uses, `pickers.css` is
 * appended as the app loads it (after main.css), and Chromium resolves every colour. Only the
 * config store is a stand-in, so no threshold is saved and the marker sits at 80%.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const PICKERS_CSS = readFileSync(resolve(here, 'pickers.css'), 'utf8');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

const FAMILIES = ['parchment', 'alma-mater', 'roche-limit'] as const;
const MODES = ['light', 'dark'] as const;

/** The popover's off Switch track: 1.19:1 light, 1.23:1 dark (see the header). */
const TRACK_FLOOR = 1.18;
const TRACK_CEILING = 1.5;
/** SC 1.4.11, for the bar's fill and the marker the person drags. */
const MARK_FLOOR = 3;

/** The gauge inside the card reads the threshold through ConfigContext: none is saved here (80%). */
const stubConfig: Plugin = {
  name: 'stub-config-context',
  setup(builder) {
    builder.onResolve({ filter: /^\.\/ConfigContext$/ }, (args) => ({
      path: args.path,
      namespace: 'config-stub',
    }));
    builder.onLoad({ filter: /.*/, namespace: 'config-stub' }, () => ({
      contents: `export const useConfig = () => ({
        read: async () => null,
        upsert: async () => undefined,
      });`,
      loader: 'ts',
    }));
  },
};

const ENTRY = `
import * as React from 'react';
import { flushSync } from 'react-dom';
import { createRoot } from 'react-dom/client';
import { ContextWindowIndicator } from '../ContextWindowIndicator';

let set: (used: number) => void = () => {};

function App() {
  const [used, setUsed] = React.useState(30_000);
  set = setUsed;
  return (
    <div style={{ position: 'fixed', right: 40, bottom: 40 }}>
      <ContextWindowIndicator
        sessionId="chat-a"
        totalTokens={used}
        tokenLimit={100_000}
        isTokenLimitLoaded
        onCompact={() => {}}
        showRemainingPercent
      />
    </div>
  );
}

(window as unknown as { __use: (used: number) => void }).__use = (used) =>
  flushSync(() => set(used));

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

/** sRGB channels on 0..255 and alpha on 0..1. */
type Rgba = [number, number, number, number];

/** A computed colour as the engine serialises a plain one. Any other shape throws by name. */
const parseColour = (value: string): Rgba => {
  const rgb = /^rgba?\(([^)]+)\)$/.exec(value);
  if (!rgb) throw new Error(`footerCardTrack: no parser for the computed colour ${value}`);
  const [r, g, b, a = '1'] = rgb[1].split(/[\s,/]+/).filter(Boolean);
  return [parseFloat(r), parseFloat(g), parseFloat(b), parseFloat(a)];
};

/** WCAG 2.x relative luminance of an opaque colour. */
const luminance = ([r, g, b, a]: Rgba): number => {
  if (a !== 1) throw new Error(`footerCardTrack: a translucent colour has no luminance (${a})`);
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
};

const ratio = (a: Rgba, b: Rgba): number => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};

/** The colours the card paints: its ground, the empty track, the bar's fill and the marker. */
type Paint = { card: Rgba; track: Rgba; fill: Rgba; marker: Rgba; old: Rgba };

describe('the context card’s empty track reads on the card in every family and mode', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the footer card track tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      plugins: [stubConfig],
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
      compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }])) +
      '\n' +
      PICKERS_CSS;
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** The composer's ring in `family` and `mode`, its card pinned open and at rest. */
  async function open(family: string, mode: 'light' | 'dark'): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 480, height: 480 } });
    await tab.setContent(
      `<!doctype html><html data-theme="${family}" class="${mode === 'dark' ? 'dark' : ''}">` +
        `<head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.click('[data-testid="context-window-indicator"]');
    await tab.waitForSelector('[data-testid="context-card"] [role="slider"]');
    await settle(tab);
    return tab;
  }

  /** Every running animation (the card's open) finished, so nothing is mid-transition. */
  const settle = (tab: Page) =>
    tab.evaluate(() =>
      Promise.all(document.getAnimations().map((animation) => animation.finished.catch(() => null)))
    );

  /** What the card paints now, with what the old track (`--background-muted`) would have. */
  async function paint(tab: Page): Promise<Paint> {
    const raw = await tab.evaluate(() => {
      const card = document.querySelector<HTMLElement>('[data-testid="context-card"]')!;
      const bar = card.querySelector<HTMLElement>('[role="progressbar"]')!;
      const probe = document.createElement('div');
      probe.style.backgroundColor = 'var(--background-muted)';
      bar.parentElement!.append(probe);
      const old = getComputedStyle(probe).backgroundColor;
      probe.remove();
      return {
        card: getComputedStyle(card).backgroundColor,
        track: getComputedStyle(bar).backgroundColor,
        fill: getComputedStyle(bar.querySelector('.br-progress__fill')!).backgroundColor,
        marker: getComputedStyle(card.querySelector('.br-context-gauge__glyph')!).borderTopColor,
        old,
      };
    });
    return {
      card: parseColour(raw.card),
      track: parseColour(raw.track),
      fill: parseColour(raw.fill),
      marker: parseColour(raw.marker),
      old: parseColour(raw.old),
    };
  }

  it(`sets the empty track at least ${TRACK_FLOOR}:1 off the card, one step deeper`, async (ctx) => {
    if (!browser) return ctx.skip();
    for (const family of FAMILIES) {
      for (const mode of MODES) {
        const tab = await open(family, mode);
        try {
          const p = await paint(tab);
          const at = `${family} ${mode}`;
          const track = ratio(p.track, p.card);
          // Soft, so a regression names every family and mode it reaches, not only the first.
          expect.soft(track, `${at}: track on card`).toBeGreaterThanOrEqual(TRACK_FLOOR);
          expect.soft(track, `${at}: track on card`).toBeLessThanOrEqual(TRACK_CEILING);
          // Deeper, never lighter: the grounds note's direction for a track in a surface.
          expect
            .soft(luminance(p.track), `${at}: track below the card`)
            .toBeLessThan(luminance(p.card));
          // And never weaker than the muted fallback it replaced (1.10 light, 1.13 dark).
          expect
            .soft(track, `${at}: track against the old one`)
            .toBeGreaterThan(ratio(p.old, p.card));
        } finally {
          await tab.close();
        }
      }
    }
  });

  it('keeps the bar’s fill in every tone, and the marker, readable on the deeper track', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const family of FAMILIES) {
      for (const mode of MODES) {
        const tab = await open(family, mode);
        try {
          // 30% used is green, 65% yellow, 90% (past the 80% marker) red.
          for (const [used, tone] of [
            [30_000, 'success'],
            [65_000, 'warning'],
            [90_000, 'danger'],
          ] as const) {
            await tab.evaluate((n) => {
              (window as unknown as { __use: (n: number) => void }).__use(n);
            }, used);
            await settle(tab);
            const p = await paint(tab);
            const at = `${family} ${mode} ${tone}`;
            expect
              .soft(ratio(p.fill, p.track), `${at}: fill on track`)
              .toBeGreaterThanOrEqual(MARK_FLOOR);
            expect
              .soft(ratio(p.marker, p.card), `${at}: marker on card`)
              .toBeGreaterThanOrEqual(MARK_FLOOR);
            expect
              .soft(ratio(p.marker, p.track), `${at}: marker on track`)
              .toBeGreaterThanOrEqual(MARK_FLOOR);
          }
        } finally {
          await tab.close();
        }
      }
    }
  });
});
