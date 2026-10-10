// @vitest-environment node
/**
 * A theme change lands in one frame, measured in a real engine (vision finding
 * V4-15, checklist 12).
 *
 * jsdom runs no transitions, so `motion.test.ts` can only pin the rule's text
 * and the helper's frame count. Here the real rule from `main.css` and the real
 * `suspendTransitionsForThemeChange()` from `motion.ts` (bundled with esbuild)
 * run in Chromium against a control that eases its colours the way the app's
 * buttons and rows do:
 *
 *   - without the helper, flipping `.dark` starts colour transitions, so the
 *     control shows the OLD theme's colours on the NEW ground for ~125ms (the
 *     dark button on a white page the reviewer measured at 48ms);
 *   - with it, nothing transitions, on the element or its pseudo-elements, and
 *     the first styled frame already has the final colours;
 *   - once the class is gone (two frames later) the transitions are back for
 *     the next ordinary state change, and restoring them started none.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { transform } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, beforeEach, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, 'main.css'), 'utf8');
const MOTION_TS = readFileSync(resolve(here, 'motion.ts'), 'utf8');

/** The theme-switch rule, from its first selector line to its closing brace. */
function themeSwitchRule(): string {
  const start = MAIN_CSS.indexOf(':root.biorouter-theme-switching,');
  if (start < 0) throw new Error('main.css has no theme-switch rule');
  return MAIN_CSS.slice(start, MAIN_CSS.indexOf('}', start) + 1);
}

const PAGE_CSS = `
:root { --ground: #ffffff; --ink: #222222; --fill: #e6e6e6; }
:root.dark { --ground: #131312; --ink: #eeeeee; --fill: #2c2c29; }
:root[data-theme='other'] { --fill: #c0c0c0; }
:root.dark[data-theme='other'] { --fill: #404040; }
body { margin: 0; background: var(--ground); }
.control {
  color: var(--ink);
  background-color: var(--fill);
  border: 1px solid var(--ink);
  transition: color 125ms ease-out, background-color 125ms ease-out, border-color 95ms ease-out;
}
.control::before {
  content: '';
  background-color: var(--ink);
  transition: background-color 125ms ease-out;
}
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

type Flip = { useHelper: boolean; mode?: 'dark' | 'light'; family?: string };

describe('a theme change lands in one frame', () => {
  let browser: Browser | null = null;
  let page: Page;
  let motionBundle = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found — skipping the theme switch tests. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
      return;
    }
    motionBundle = (
      await transform(MOTION_TS, { loader: 'ts', format: 'iife', globalName: 'Motion' })
    ).code;
    page = await browser.newPage({ viewport: { width: 400, height: 200 } });
  });

  afterAll(async () => {
    await browser?.close();
  });

  beforeEach(async () => {
    if (!browser) return;
    await page.setContent(
      `<!doctype html><html class="light"><head><style>${PAGE_CSS}</style>` +
        `<style>${themeSwitchRule()}</style></head>` +
        `<body><button class="control" id="c">Details</button></body></html>`
    );
    await page.addScriptTag({ content: motionBundle });
    // Let the first frame paint so the control's resting colours are settled.
    await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => done(null))));
  });

  /** Flip the theme as ThemeContext does, then read the first styled state. */
  const flip = ({ useHelper, mode, family }: Flip) =>
    page.evaluate(
      ({ useHelper: withHelper, mode: nextMode, family: nextFamily }) => {
        const root = document.documentElement;
        const motion = (window as unknown as { Motion: typeof import('./motion') }).Motion;
        if (withHelper) motion.suspendTransitionsForThemeChange(root);
        if (nextMode) {
          root.classList.add(nextMode);
          root.classList.remove(nextMode === 'dark' ? 'light' : 'dark');
        }
        if (nextFamily) root.setAttribute('data-theme', nextFamily);
        const control = document.getElementById('c')!;
        const style = getComputedStyle(control);
        const before = getComputedStyle(control, '::before');
        return {
          color: style.color,
          fill: style.backgroundColor,
          edge: style.borderTopColor,
          beforeFill: before.backgroundColor,
          running: document.getAnimations().length,
          switching: root.classList.contains(motion.THEME_SWITCHING_CLASS),
        };
      },
      { useHelper, mode, family }
    );

  const twoFrames = () =>
    page.evaluate(
      () =>
        new Promise((done) =>
          requestAnimationFrame(() => requestAnimationFrame(() => requestAnimationFrame(done)))
        )
    );

  it('control: without the helper the colours walk from the old theme', async () => {
    if (!browser) return;
    const first = await flip({ useHelper: false, mode: 'dark' });
    expect(first.running).toBeGreaterThan(0);
    expect(first.color).toBe('rgb(34, 34, 34)'); // still the light ink on the dark ground
  });

  it('with the helper a Dark flip has its final colours on the first styled frame', async () => {
    if (!browser) return;
    const first = await flip({ useHelper: true, mode: 'dark' });
    expect(first.switching).toBe(true);
    expect(first.running).toBe(0);
    expect(first).toMatchObject({
      color: 'rgb(238, 238, 238)',
      fill: 'rgb(44, 44, 41)',
      edge: 'rgb(238, 238, 238)',
      beforeFill: 'rgb(238, 238, 238)',
    });
  });

  it('a family change is still too', async () => {
    if (!browser) return;
    const first = await flip({ useHelper: true, family: 'other' });
    expect(first.running).toBe(0);
    expect(first.fill).toBe('rgb(192, 192, 192)');
  });

  it('the class comes off two frames later without starting a transition', async () => {
    if (!browser) return;
    await flip({ useHelper: true, mode: 'dark' });
    await twoFrames();
    const after = await page.evaluate(() => ({
      switching: document.documentElement.classList.contains('biorouter-theme-switching'),
      running: document.getAnimations().length,
      color: getComputedStyle(document.getElementById('c')!).color,
    }));
    expect(after).toEqual({ switching: false, running: 0, color: 'rgb(238, 238, 238)' });

    // The transitions are back for an ordinary change made afterwards.
    const later = await flip({ useHelper: false, mode: 'light' });
    expect(later.running).toBeGreaterThan(0);
  });
});
