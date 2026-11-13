// @vitest-environment node
/**
 * A message a chat's "Open in Crew" points at takes the one-shot wash, measured in a real layout
 * engine (chat parity WI-16).
 *
 * The timeline sets `crew-highlight` on the message row's element (`washMessageRow`), the class a
 * task row takes, and takes it off on `animationend`. jsdom runs no CSS animation, so the jsdom
 * tests can only see the class. This file loads the real `crew-app.css` and `timeline.css` into
 * Chromium and asserts what a person sees: the row's fill starts at `--overlay-selected` and lets
 * it go, under reduced motion it holds still and still ends (so the class is still taken off),
 * and nothing in the message row's own rules (its hover fill, its transition) takes the wash away.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const crewDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const STYLESHEETS = ['crew-app.css', 'timeline/timeline.css'];

/** The wash's colour, distinctive so nothing else on the page can be mistaken for it. */
const OVERLAY = 'rgb(10, 20, 30)';

/** Whichever Chromium build this machine has, headless with a throwaway profile. */
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

interface Washed {
  animation: string;
  /** The row's fill as the wash starts. */
  start: string;
  /** Whether `animationend` fired, so the timeline can take the class off. */
  ended: boolean;
}

describe('a message row takes the highlight wash (WI-16)', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      // Say so out loud: a silently skipped browser test reads as a passing one.
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the message highlight ' +
          'tests. Run `npx playwright install chromium` to enable them.'
      );
    }
    css = STYLESHEETS.map((path) => readFileSync(resolve(crewDir, path), 'utf8')).join('\n');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  const wash = async (reducedMotion: 'reduce' | 'no-preference'): Promise<Washed> => {
    const page = await browser!.newPage({ viewport: { width: 800, height: 400 }, reducedMotion });
    try {
      await page.setContent(
        '<!doctype html><html><head><style>' +
          `:root { --overlay-selected: ${OVERLAY}; --overlay-hover: rgb(200, 200, 200);` +
          ' --dur-slow: 40ms; --ease-out: linear; --radius-element: 8px; }' +
          ' body { margin: 0; font: 14px/20px sans-serif; }' +
          css +
          '</style></head><body><div class="crew-app"><div class="crew-timeline">' +
          '<div class="crew-message-group">' +
          '<div class="crew-message-row" data-crew-row="" id="row">' +
          '<div class="crew-message-gutter"></div><div class="crew-message-main">status update</div>' +
          '</div></div></div></div></body></html>'
      );
      return await page.evaluate(
        () =>
          new Promise<Washed>((done) => {
            const row = document.getElementById('row')!;
            let start = '';
            let animation = '';
            row.addEventListener('animationend', () => done({ animation, start, ended: true }), {
              once: true,
            });
            row.classList.add('crew-highlight');
            const style = getComputedStyle(row);
            animation = style.animationName;
            const [running] = row.getAnimations();
            if (running) {
              running.pause();
              running.currentTime = 0;
              start = getComputedStyle(row).backgroundColor;
              running.play();
            }
            setTimeout(() => done({ animation, start, ended: false }), 2000);
          })
      );
    } finally {
      await page.close();
    }
  };

  it('starts the message row’s fill at the selection overlay, and ends', async () => {
    if (!browser) return;
    const washed = await wash('no-preference');
    expect(washed.animation).toBe('crew-highlight-wash');
    expect(washed.start).toBe(OVERLAY);
    expect(washed.ended).toBe(true);
  });

  it('holds still under reduced motion, and still ends', async () => {
    if (!browser) return;
    const washed = await wash('reduce');
    expect(washed.animation).toBe('crew-highlight-hold');
    expect(washed.start).toBe(OVERLAY);
    expect(washed.ended).toBe(true);
  });
});
