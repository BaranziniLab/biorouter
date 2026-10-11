// @vitest-environment node
/**
 * The Slider measured in a real layout engine (owner ruling R-26, spec 5.7).
 *
 * jsdom lays nothing out, runs no CSS and starts no transition, so `slider.test.tsx` can only pin
 * attributes and the rules at the source. Here the real component is bundled with esbuild,
 * rendered by React in Chromium and styled by the authored `@ws WS-PRIMITIVES` blocks of
 * `styles/main.css` with the token values read from that file, and the acceptance numbers are
 * measured: a 24px target, a 4px track, a 6px mark centred on every stop, a 16px thumb centred on
 * its stop and never overhanging the box, the fill ending under the thumb, a press and a drag
 * that snap, the focus edge for keys and not for a press, only `transform` and `clip-path`
 * moving and only for the person's changes, right to left, reduced motion and forced colours.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type BrowserContext, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');

const TOKEN_NAMES = ['--radius-full', '--dur-fast-max', '--ease-out'];

/** The first declaration of each token in main.css, so a token change reaches this test. */
function readTokens(): string {
  const lines = TOKEN_NAMES.map((name) => {
    const match = MAIN_CSS.match(new RegExp(`\\n\\s*${name.replace(/-/g, '\\-')}:\\s*([^;]+);`));
    if (!match) throw new Error(`main.css does not declare ${name}`);
    return `  ${name}: ${match[1].trim()};`;
  });
  return `:root {\n${lines.join('\n')}\n}`;
}

/** Every `@ws WS-PRIMITIVES` block of main.css, in file order. */
function primitiveBlocks(): string {
  const out: string[] = [];
  const re =
    /\/\* @ws WS-PRIMITIVES begin: ([^*]+?) \*\/([\s\S]*?)\/\* @ws WS-PRIMITIVES end: \1 \*\//g;
  for (const match of MAIN_CSS.matchAll(re)) out.push(match[2]);
  if (out.length === 0) throw new Error('main.css has no @ws WS-PRIMITIVES block');
  return out.join('\n');
}

const ACCENT = 'rgb(184, 90, 50)';
const FOCUS = 'rgb(85, 85, 85)';
const PREFLIGHT = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; font-family: Arial, sans-serif; font-size: 14px; line-height: 20px; background: #fff; }
button { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden;
  clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
:root { --control-track-off: #ebebe8; --background-accent: #b85a32; --border-control: #86847f;
  --background-default: #fff; --border-focus: #555; }
`;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { Slider } from './slider';

const STEPS = [
  { value: 'quick', label: 'Quick' },
  { value: 'normal', label: 'Normal' },
  { value: 'deep', label: 'Deep' },
];

const setters: Record<string, (v: string) => void> = {};
const values: Record<string, string> = {};
(window as unknown as { setSlider: (id: string, v: string) => void }).setSlider = (id, v) =>
  flushSync(() => setters[id](v));
(window as unknown as { sliderValues: Record<string, string> }).sliderValues = values;

function Controlled({ id, initial, dir, width = 240 }: { id: string; initial: string; dir?: 'ltr' | 'rtl'; width?: number }) {
  const [value, setValue] = React.useState(initial);
  setters[id] = setValue;
  values[id] = value;
  return (
    <div style={{ width, padding: '8px 16px' }}>
      <Slider aria-label={id} data-testid={id} dir={dir} steps={STEPS} value={value} onValueChange={setValue} />
    </div>
  );
}

function App() {
  return (
    <div>
      <button id="start" type="button">Start</button>
      <Controlled id="main" initial="normal" />
      <Controlled id="low" initial="quick" />
      <Controlled id="high" initial="deep" />
      <div dir="rtl"><Controlled id="rtl" initial="quick" /></div>
      <div id="scaled" style={{ transform: 'scale(0.97)', transformOrigin: '0 0' }}>
        <Controlled id="scaled-slider" initial="quick" />
      </div>
    </div>
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

const EPSILON = 0.5;
const near = (actual: number, expected: number, what: string) =>
  expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThanOrEqual(
    EPSILON
  );

type Box = {
  left: number;
  right: number;
  top: number;
  bottom: number;
  width: number;
  height: number;
};

describe('the Slider in a real layout engine (R-26, spec 5.7)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';
  const contexts: BrowserContext[] = [];

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found (Playwright’s or Google Chrome): skipping the slider tests.');
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
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    css = PREFLIGHT + readTokens() + primitiveBlocks();
  }, 120_000);

  afterAll(async () => {
    for (const context of contexts) await context.close().catch(() => {});
    await browser?.close();
  }, 120_000);

  async function open(
    options: { reducedMotion?: 'reduce' | 'no-preference'; forcedColors?: 'active' } = {}
  ): Promise<Page> {
    const context = await browser!.newContext({
      viewport: { width: 600, height: 600 },
      reducedMotion: options.reducedMotion ?? 'no-preference',
      forcedColors: options.forcedColors ?? 'none',
    });
    contexts.push(context);
    const tab = await context.newPage();
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[data-testid="scaled-slider"]');
    await settle(tab);
    return tab;
  }

  const settle = (tab: Page) =>
    tab.evaluate(
      () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)))
    );

  /** Boxes of one slider's parts. */
  async function measure(tab: Page, id: string) {
    return tab.evaluate((testId) => {
      const plain = (el: Element) => {
        const r = el.getBoundingClientRect();
        return {
          left: r.left,
          right: r.right,
          top: r.top,
          bottom: r.bottom,
          width: r.width,
          height: r.height,
        };
      };
      const root = document.querySelector(`[data-testid="${testId}"]`)!;
      const track = root.querySelector('[data-slot="slider-track"]')!;
      const bar = getComputedStyle(track, '::before');
      return {
        root: plain(root),
        rail: plain(root.querySelector('[data-slot="slider-rail"]')!),
        thumb: plain(root.querySelector('[data-slot="slider-thumb"]')!),
        marks: [...root.querySelectorAll('[data-slot="slider-mark"]')].map(plain),
        barHeight: parseFloat(bar.height),
        thumbColour: getComputedStyle(root.querySelector('[data-slot="slider-thumb"]')!)
          .backgroundColor,
      };
    }, id);
  }

  const centre = (box: Box) => box.left + box.width / 2;
  const middle = (box: Box) => box.top + box.height / 2;

  /** Which layer is painted at (x, y) on the rail: the fill (clipped) or not. */
  async function filledAt(tab: Page, id: string, x: number): Promise<boolean> {
    return tab.evaluate(
      ({ testId, px }) => {
        const root = document.querySelector(`[data-testid="${testId}"]`)!;
        const fill = root.querySelector('[data-slot="slider-fill"]') as HTMLElement;
        const rail = root.querySelector('[data-slot="slider-rail"]')!.getBoundingClientRect();
        // Hit-test the fill alone: clip-path clips hit testing as it clips paint.
        fill.style.pointerEvents = 'auto';
        const hit = document.elementFromPoint(px, rail.top + rail.height / 2);
        fill.style.pointerEvents = '';
        return !!hit && fill.contains(hit);
      },
      { testId: id, px: x }
    );
  }

  it('is a 24px target with a 4px track, a 6px mark on every stop and a 16px thumb on its stop', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      for (const [id, stop] of [
        ['low', 0],
        ['main', 1],
        ['high', 2],
      ] as const) {
        const m = await measure(tab, id);
        near(m.root.height, 24, `${id} height`);
        near(m.barHeight, 4, `${id} track`);
        near(m.rail.left - m.root.left, 8, `${id} rail inset start`);
        near(m.root.right - m.rail.right, 8, `${id} rail inset end`);
        expect(m.marks).toHaveLength(3);
        m.marks.forEach((mark, i) => {
          near(mark.width, 6, `${id} mark ${i} width`);
          near(mark.height, 6, `${id} mark ${i} height`);
          near(centre(mark), m.rail.left + (i / 2) * m.rail.width, `${id} mark ${i} x`);
          near(middle(mark), middle(m.root), `${id} mark ${i} y`);
        });
        near(m.thumb.width, 16, `${id} thumb width`);
        near(m.thumb.height, 16, `${id} thumb height`);
        near(centre(m.thumb), centre(m.marks[stop]), `${id} thumb on its stop`);
        near(middle(m.thumb), middle(m.root), `${id} thumb centred`);
        // The thumb never overhangs the box, at either end.
        expect(m.thumb.left).toBeGreaterThanOrEqual(m.root.left - EPSILON);
        expect(m.thumb.right).toBeLessThanOrEqual(m.root.right + EPSILON);
        expect(m.thumbColour).toBe(ACCENT);
      }
    } finally {
      await tab.close();
    }
  });

  it('fills the track from the start up to the thumb, and no further', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await measure(tab, 'main');
      // Between the first stop and the thumb: filled. Between the thumb and the last: not.
      expect(await filledAt(tab, 'main', m.rail.left + 0.25 * m.rail.width)).toBe(true);
      expect(await filledAt(tab, 'main', m.rail.left + 0.75 * m.rail.width)).toBe(false);
      const low = await measure(tab, 'low');
      expect(await filledAt(tab, 'low', low.rail.left + 0.25 * low.rail.width)).toBe(false);
      const high = await measure(tab, 'high');
      expect(await filledAt(tab, 'high', high.rail.left + 0.9 * high.rail.width)).toBe(true);
    } finally {
      await tab.close();
    }
  });

  it('sets the nearest stop on a click, and snaps from stop to stop while dragged', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await measure(tab, 'low');
      const y = middle(m.root);
      const at = (share: number) => m.rail.left + share * m.rail.width;
      const value = () =>
        tab.evaluate(
          () => (window as unknown as { sliderValues: Record<string, string> }).sliderValues.low
        );

      // A click near the last stop.
      await tab.mouse.click(at(0.9), y);
      expect(await value()).toBe('deep');

      // A drag from the end back across the scale, sampled on the way.
      await tab.mouse.move(at(1), y);
      await tab.mouse.down();
      const seen: string[] = [];
      for (const share of [0.8, 0.6, 0.45, 0.3, 0.1, -0.2]) {
        await tab.mouse.move(at(share), y);
        seen.push(await value());
      }
      // The pointer leaves the slider entirely and is still followed (pointer capture).
      await tab.mouse.move(at(0.95), y + 120);
      seen.push(await value());
      await tab.mouse.up();
      expect(seen).toEqual(['deep', 'normal', 'normal', 'normal', 'quick', 'quick', 'deep']);

      // After release, moving does nothing.
      await tab.mouse.move(at(0), y);
      expect(await value()).toBe('deep');
    } finally {
      await tab.close();
    }
  });

  it('maps the pointer through a scaled surface (a menu mid-entrance)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await measure(tab, 'scaled-slider');
      await tab.mouse.click(m.rail.left + 0.5 * m.rail.width, middle(m.root));
      const value = await tab.evaluate(
        () =>
          (window as unknown as { sliderValues: Record<string, string> }).sliderValues[
            'scaled-slider'
          ]
      );
      expect(value).toBe('normal');
    } finally {
      await tab.close();
    }
  });

  it('draws the focus edge for the keyboard, and none after a click', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const look = () =>
        tab.evaluate(() => {
          const el = document.activeElement as HTMLElement;
          return {
            role: el.getAttribute('role'),
            label: el.getAttribute('aria-label'),
            shadow: getComputedStyle(el).boxShadow,
          };
        });

      await tab.focus('#start');
      await tab.keyboard.press('Tab');
      const keyed = await look();
      expect(keyed.role).toBe('slider');
      expect(keyed.label).toBe('main');
      expect(keyed.shadow, 'Tab draws the edge').toContain(FOCUS);

      const m = await measure(tab, 'low');
      await tab.mouse.click(m.rail.left + 0.5 * m.rail.width, middle(m.root));
      const clicked = await look();
      expect(clicked.label).toBe('low');
      expect(clicked.shadow, 'a click focuses without the edge').not.toContain(FOCUS);

      // The next key brings it back.
      await tab.keyboard.press('ArrowRight');
      expect((await look()).shadow).toContain(FOCUS);
    } finally {
      await tab.close();
    }
  });

  it('glides the person’s change with transform and clip-path only, on one curve', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.focus('#start');
      await tab.keyboard.press('Tab');
      const running = await tab.evaluate(
        () =>
          new Promise<{ property: string; duration: number; easing: string }[]>((done) => {
            const root = document.querySelector('[data-testid="main"]')!;
            (document.activeElement as HTMLElement).dispatchEvent(
              new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true })
            );
            requestAnimationFrame(() => {
              const out = root.getAnimations({ subtree: true }).map((animation) => {
                const timing = animation.effect!.getComputedTiming();
                const first = (animation.effect as KeyframeEffect).getKeyframes()[0];
                return {
                  property: (animation as Animation & { transitionProperty: string })
                    .transitionProperty,
                  duration: Number(timing.duration),
                  // A CSS transition carries its timing function on the effect or the keyframe.
                  easing: `${timing.easing} ${first?.easing ?? ''}`,
                };
              });
              done(out);
            });
          })
      );
      const properties = running.map((r) => r.property).sort();
      expect(properties).toEqual(['clip-path', 'transform']);
      for (const r of running) expect(r.duration).toBe(175);
      expect(new Set(running.map((r) => r.easing)).size, 'one curve for both').toBe(1);
      expect(running[0].easing).toContain('cubic-bezier(0.24, 1, 0.4, 1)');
    } finally {
      await tab.close();
    }
  });

  it('lands at rest on mount and when the value changes without the person', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      // Mounted: nothing running.
      const atMount = await tab.evaluate(
        () =>
          document.querySelector('[data-testid="main"]')!.getAnimations({ subtree: true }).length
      );
      expect(atMount).toBe(0);

      // Another window or a late store moves it: the thumb is at the new stop, nothing glides.
      const after = await tab.evaluate(
        () =>
          new Promise<number>((done) => {
            (window as unknown as { setSlider: (id: string, v: string) => void }).setSlider(
              'main',
              'deep'
            );
            // A style read in the same task, as a sibling measuring would do.
            const root = document.querySelector('[data-testid="main"]')!;
            void (root as HTMLElement).offsetWidth;
            requestAnimationFrame(() => done(root.getAnimations({ subtree: true }).length));
          })
      );
      expect(after).toBe(0);
      const m = await measure(tab, 'main');
      near(centre(m.thumb), centre(m.marks[2]), 'thumb at the new stop');

      // The flag is released: the person's next change glides again.
      await settle(tab);
      await tab.focus('[data-testid="main"] [role="slider"]');
      const glides = await tab.evaluate(
        () =>
          new Promise<number>((done) => {
            const root = document.querySelector('[data-testid="main"]')!;
            (document.activeElement as HTMLElement).dispatchEvent(
              new KeyboardEvent('keydown', { key: 'Home', bubbles: true })
            );
            requestAnimationFrame(() => done(root.getAnimations({ subtree: true }).length));
          })
      );
      expect(glides).toBe(2);
    } finally {
      await tab.close();
    }
  });

  it('moves nothing under reduced motion', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ reducedMotion: 'reduce' });
    try {
      await tab.focus('#start');
      await tab.keyboard.press('Tab');
      const running = await tab.evaluate(
        () =>
          new Promise<number>((done) => {
            const root = document.querySelector('[data-testid="main"]')!;
            (document.activeElement as HTMLElement).dispatchEvent(
              new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true })
            );
            requestAnimationFrame(() => done(root.getAnimations({ subtree: true }).length));
          })
      );
      expect(running).toBe(0);
      const m = await measure(tab, 'main');
      near(centre(m.thumb), centre(m.marks[2]), 'the thumb still moves, at once');
    } finally {
      await tab.close();
    }
  });

  it('runs from the right in a right-to-left layout', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await measure(tab, 'rtl');
      // Quick is the first stop, which sits at the right.
      near(centre(m.thumb), m.rail.right, 'first stop on the right');
      expect(m.thumb.right).toBeLessThanOrEqual(m.root.right + EPSILON);

      await tab.focus('[data-testid="rtl"] [role="slider"]');
      await tab.keyboard.press('ArrowLeft');
      await tab.keyboard.press('ArrowLeft');
      await settle(tab);
      await tab.waitForTimeout(250);
      const deep = await measure(tab, 'rtl');
      near(centre(deep.thumb), deep.rail.left, 'last stop on the left');
      // The fill grows from the right: the right half is filled at Deep.
      expect(await filledAt(tab, 'rtl', deep.rail.left + 0.75 * deep.rail.width)).toBe(true);

      // A click near the right end picks the first stop.
      await tab.mouse.click(deep.rail.left + 0.95 * deep.rail.width, middle(deep.root));
      const value = await tab.evaluate(
        () => (window as unknown as { sliderValues: Record<string, string> }).sliderValues.rtl
      );
      expect(value).toBe('quick');
    } finally {
      await tab.close();
    }
  });

  it('stays visible in forced colours', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ forcedColors: 'active' });
    try {
      const look = await tab.evaluate(() => {
        const root = document.querySelector('[data-testid="main"]')!;
        const thumb = root.querySelector('[data-slot="slider-thumb"]')!;
        const track = root.querySelector('[data-slot="slider-track"]')!;
        const fill = root.querySelector('[data-slot="slider-fill"]')!;
        const mark = root.querySelector('[data-slot="slider-mark"]')!;
        return {
          thumb: getComputedStyle(thumb).backgroundColor,
          track: getComputedStyle(track, '::before').backgroundColor,
          fill: getComputedStyle(fill, '::before').backgroundColor,
          mark: getComputedStyle(mark).backgroundColor,
          canvas: getComputedStyle(document.body).backgroundColor,
        };
      });
      const transparent = 'rgba(0, 0, 0, 0)';
      for (const [part, colour] of Object.entries(look)) {
        if (part === 'canvas') continue;
        expect(colour, part).not.toBe(transparent);
        expect(colour, `${part} differs from the page`).not.toBe(look.canvas);
      }
      expect(look.track, 'track and fill differ').not.toBe(look.fill);
      expect(look.thumb).toBe(look.fill);
    } finally {
      await tab.close();
    }
  });
});
