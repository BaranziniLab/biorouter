// @vitest-environment node
/**
 * The .brxt dialog's fill recipe, measured in a real layout engine (owner message 13, R-27).
 *
 * jsdom runs no stylesheet, so `BrxtInstallModal.test.tsx` can say the drop zone carries
 * `.br-brxt-dropzone` and the right `data-state`, but not that the class paints the subtle fill
 * with a transparent 1px edge, steps to the strong fill under the pointer (and not while a bundle
 * is being read), firms its edge for a file held over it, or that the summary and its skills are
 * told apart by space and no longer by a hairline. This lays the recipe's classes out in Chromium
 * with `brxt-install.css` at the token values, light and dark.
 *
 * The markup mirrors the component's: the class names are the contract, and the component test
 * pins that the real modal emits exactly these.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const css = readFileSync(resolve(here, 'brxt-install.css'), 'utf8');

/** `main.css`'s values for the tokens the recipe reads (light), and the dark overrides. */
const TOKENS = `:root {
  --font-body: Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, sans-serif;
  --radius-container: 12px;
  --radius-element: 8px;
  --background-default: #ffffff;
  --background-fill: #f4f4f2;
  --background-fill-strong: #eaeae6;
  --background-focus: #e0e0dc;
  --border-strong: #b9b7b0;
  --border-danger: #b3261e;
  --border-focus: #6b6963;
  --border-subtle: #e5e3df;
  --text-default: #1d1c1a;
  --text-muted: #6b6963;
  --dur-fast-min: 95ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
}
html.dark {
  --background-default: #1b1b19;
  --background-fill: #232320;
  --background-fill-strong: #2c2c29;
  --background-focus: #35342f;
  --border-strong: #5a5851;
  --text-default: #f4f0e6;
  --text-muted: #a8a49a;
}
* { box-sizing: border-box; }
body { margin: 0; padding: 16px; width: 480px; font: 14px/20px var(--font-body); background: var(--background-default); }
p { margin: 0; }
button { font: inherit; margin: 0; color: inherit; }
svg { width: 16px; height: 16px; }
.text-label { font-size: 14px; line-height: 20px; font-weight: 500; }
.text-supporting { font-size: 12px; line-height: 16px; }`;

const HTML = `<!doctype html><html><head><style>${TOKENS}\n${css}</style></head><body>
  <button type="button" id="zone" class="br-brxt-dropzone">
    <span aria-hidden="true" id="plate" class="br-brxt-drop-plate"><svg viewBox="0 0 16 16"></svg></span>
    <span class="text-label">Drop a .brxt file here</span>
  </button>
  <button type="button" id="reading" class="br-brxt-dropzone" aria-disabled="true">Reading</button>
  <button type="button" id="dragging" class="br-brxt-dropzone" data-state="dragging">
    <span aria-hidden="true" class="br-brxt-drop-plate" id="plate-dragging"></span>
  </button>
  <button type="button" id="failed" class="br-brxt-dropzone" data-state="error">Drop</button>
  <section class="br-brxt-summary" id="summary">
    <p class="text-supporting">Detected from bundle</p>
    <p class="text-label">Test Extension</p>
    <div class="br-brxt-skills" id="skills"><p class="text-supporting">Skills included</p></div>
  </section>
</body></html>`;

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

// 90s a test: each one launches a page in a Chromium other suites are loading at the same time.
describe('The .brxt dialog on the fill recipe in a real layout engine', { timeout: 90_000 }, () => {
  let browser: Browser | null = null;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the .brxt geometry tests. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
    }
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(mode: 'light' | 'dark'): Promise<Page> {
    const tab = await browser!.newPage({
      viewport: { width: 640, height: 800 },
      reducedMotion: 'reduce',
    });
    await tab.setContent(HTML);
    if (mode === 'dark') await tab.evaluate(() => document.documentElement.classList.add('dark'));
    return tab;
  }

  const paint = (tab: Page, id: string) =>
    tab.evaluate((target) => {
      const el = document.getElementById(target)!;
      const style = getComputedStyle(el);
      const rect = el.getBoundingClientRect();
      return {
        background: style.backgroundColor,
        edgeWidth: style.borderTopWidth,
        edgeColor: style.borderTopColor,
        radius: style.borderTopLeftRadius,
        shadow: style.boxShadow,
        width: rect.width,
        height: rect.height,
      };
    }, id);

  it('rests on the subtle fill with a transparent 1px edge, and no outline', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const zone = await paint(tab, 'zone');
        expect(zone.background).toBe(mode === 'dark' ? 'rgb(35, 35, 32)' : 'rgb(244, 244, 242)');
        // The edge stays a border, transparent: forced colours draw it, and nothing moves.
        expect(zone.edgeWidth).toBe('1px');
        expect(zone.edgeColor).toBe('rgba(0, 0, 0, 0)');
        expect(zone.radius).toBe('12px');
        expect(zone.shadow).toBe('none');
        // One filled field the dialog's full width: 24px of air above and below the plate and line.
        expect(zone.width).toBe(448);
        expect(zone.height).toBe(24 + 32 + 8 + 20 + 24 + 2);
      } finally {
        await tab.close();
      }
    }
  });

  it('puts a 32px plate on the page ground, one step back from the fill, and not on it', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const plate = await paint(tab, 'plate');
        const zone = await paint(tab, 'zone');
        expect(plate.width).toBe(32);
        expect(plate.height).toBe(32);
        expect(plate.radius).toBe('8px');
        expect(plate.background).toBe(mode === 'dark' ? 'rgb(27, 27, 25)' : 'rgb(255, 255, 255)');
        // A fill never sits on a fill: the plate is not the zone's own colour.
        expect(plate.background).not.toBe(zone.background);
        expect(plate.edgeWidth).toBe('0px');
      } finally {
        await tab.close();
      }
    }
  });

  it('steps to the strong fill under the pointer, except while a bundle is being read', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const strong = mode === 'dark' ? 'rgb(44, 44, 41)' : 'rgb(234, 234, 230)';
        const rest = mode === 'dark' ? 'rgb(35, 35, 32)' : 'rgb(244, 244, 242)';
        await tab.hover('#zone');
        await tab.waitForTimeout(150);
        expect((await paint(tab, 'zone')).background).toBe(strong);
        await tab.hover('#reading');
        await tab.waitForTimeout(150);
        // The zone does nothing while it reads, so it must not look as if it would.
        expect((await paint(tab, 'reading')).background).toBe(rest);
      } finally {
        await tab.close();
      }
    }
  });

  it('firms its edge for a file held over it and takes the danger edge on an error', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light');
    try {
      const dragging = await paint(tab, 'dragging');
      expect(dragging.background).toBe('rgb(234, 234, 230)');
      expect(dragging.shadow).toContain('rgb(185, 183, 176)');
      expect(dragging.shadow).toContain('inset');
      // An inset ring draws on the box, so the edge never moves the plate.
      expect(dragging.edgeWidth).toBe('1px');
      const failed = await paint(tab, 'failed');
      expect(failed.shadow).toContain('rgb(179, 38, 30)');
      expect(failed.background).toBe('rgb(244, 244, 242)');
    } finally {
      await tab.close();
    }
  });

  it('shows keyboard focus as the app’s one treatment: the focus ground and a 2px inset ring', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light');
    try {
      await tab.keyboard.press('Tab');
      const zone = await paint(tab, 'zone');
      expect(zone.background).toBe('rgb(224, 224, 220)');
      expect(zone.shadow).toContain('rgb(107, 105, 99)');
      expect(zone.shadow).toContain('0px 0px 0px 2px');
    } finally {
      await tab.close();
    }
  });

  it('tells the summary from what is above it, and its skills from the sentence, by space alone', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light');
    try {
      const m = await tab.evaluate(() => {
        const read = (id: string) => {
          const style = getComputedStyle(document.getElementById(id)!);
          return {
            top: style.borderTopWidth,
            bottom: style.borderBottomWidth,
            marginTop: style.marginTop,
            paddingTop: style.paddingTop,
          };
        };
        return { summary: read('summary'), skills: read('skills') };
      });
      // No rule above either block, and no padding that stood where a rule used to be.
      expect(m.summary.top).toBe('0px');
      expect(m.summary.paddingTop).toBe('0px');
      expect(m.skills.top).toBe('0px');
      expect(m.skills.paddingTop).toBe('0px');
      // The skills sit 12px under the sentence: the space the rule and its padding made.
      expect(m.skills.marginTop).toBe('12px');
    } finally {
      await tab.close();
    }
  });

  it('draws no resting line anywhere in the sheet: a hairline is only ever a state', () => {
    // The subtle border token is the one every resting divider used. The drop zone's edge is a
    // transparent border, the summary has no rule, and the only hairlines left are an inset ring
    // for a file held over the zone (strong) or a failed one (danger).
    expect(css).not.toMatch(/var\(--border-subtle\)/);
    expect(css).not.toMatch(/border-top\s*:\s*1px solid/);
  });
});
