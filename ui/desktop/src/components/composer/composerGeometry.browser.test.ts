// @vitest-environment node
/**
 * The composer's authored geometry, measured in a real layout engine.
 *
 * jsdom lays nothing out and does not implement `field-sizing`, so the
 * component tests can say what the composer renders but not how big it is.
 * This lays the composer's own recipes (`composer.css`) out in Chromium over a
 * hand-written copy of the card's markup, with the tokens at `main.css`'s
 * values, and measures what spec 3.7 promises:
 *
 *  - the text field is one line (32px) when empty, grows with what is typed,
 *    and stops at ten lines, then scrolls;
 *  - the controls row is 32px, the `+` glyph sits on the text's left edge, and
 *    Send ends the row at the card's inner right edge;
 *  - a queue row is 28px and its actions appear only on hover;
 *  - an image thumbnail is 48px and its loading scrim is translucent.
 *
 * The class names below are asserted to be the ones `ChatInput.tsx` and the
 * composer components use, so this markup cannot drift from the real one
 * unnoticed.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const read = (path: string) => readFileSync(resolve(here, path), 'utf8');

const TOKENS = `:root {
  --control-md: 32px;
  --text-secondary: 13px;
  --text-secondary--line-height: 18px;
  --text-supporting: 12px;
  --text-supporting--line-height: 16px;
  --radius-inner: 4px;
  --radius-element: 8px;
  --border-subtle: #e5e3df;
  --border-danger: #c0392b;
  --background-muted: #f4f3f1;
  --overlay-hover: rgb(31 30 28 / 0.05);
  --scrim: rgba(31, 30, 28, 0.18);
  --text-default: #1f1e1c;
  --text-muted: #6b6862;
  --dur-fast-min: 0ms;
  --ease-out: linear;
  --z-modal-dropdown: 500;
}
* { box-sizing: border-box; }
body { margin: 0; font: 14px/20px Arial, sans-serif; }
button { font: inherit; padding: 0; border: 0; background: none; }
textarea { font: inherit; margin: 0; }
p { margin: 0; }`;

/** The card at `ChatInput`'s values: 1px border, padding 10px 12px 10px 16px. */
const CARD = `display: flex; flex-direction: column; width: 640px; border: 1px solid #ddd;
  padding: 10px 12px 10px 16px; border-radius: 12px;`;
/** A ghost round `sm` button (28px) and the round default one (32px), with a 16px glyph. */
const BUTTON = (size: number) =>
  `display: inline-flex; align-items: center; justify-content: center; width: ${size}px; height: ${size}px; flex: none;`;
const GLYPH = 'display: block; width: 16px; height: 16px; background: #000;';

const MARKUP = `
<div id="card" style="${CARD}">
  <div class="br-queue">
    <ul class="br-queue-list">
      <li id="queue-row" class="br-queue-row">
        <span class="br-queue-arrow">↳</span>
        <button class="br-queue-text br-queue-edit">and rerun the fit</button>
        <div id="queue-actions" class="br-queue-actions">
          <button style="${BUTTON(24)}">x</button>
        </div>
      </li>
    </ul>
  </div>
  <ul class="br-composer-chips">
    <li>
      <div id="thumb" class="br-composer-thumb">
        <div id="scrim" class="br-composer-thumb__scrim"></div>
      </div>
    </li>
  </ul>
  <textarea id="text" rows="1" class="br-composer-input"
    style="display: block; width: 100%; resize: none; border: 0; padding: 6px 0; line-height: 20px; font-size: 14px;"></textarea>
  <div id="controls" class="br-composer-controls">
    <button id="plus" style="${BUTTON(28)}"><i id="plus-glyph" style="${GLYPH}"></i></button>
    <button style="height: 28px; padding: 0 8px;">Tools 15</button>
    <span class="br-composer-controls__spacer"></span>
    <div class="br-composer-controls__model"><button style="height: 28px; padding: 0 8px;">gpt-5.6-sol</button></div>
    <button id="send" style="${BUTTON(32)}"><i style="${GLYPH}"></i></button>
  </div>
</div>`;

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

describe('the composer recipes in a real layout engine', () => {
  let browser: Browser | null = null;
  let page: Page | null = null;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found; skipping the composer geometry tests.');
      return;
    }
    page = await browser.newPage({ viewport: { width: 800, height: 900 } });
    await page.setContent(
      `<!doctype html><html><head><style>${TOKENS}${read('composer.css')}</style></head>` +
        `<body style="padding: 40px">${MARKUP}</body></html>`
    );
  }, 120_000);

  afterAll(async () => {
    await page?.close();
    await browser?.close();
  }, 120_000);

  const box = async (selector: string) => {
    const rect = await page!.locator(selector).boundingBox();
    if (!rect) throw new Error(`no box for ${selector}`);
    return rect;
  };

  it('uses the class names the composer really renders', () => {
    const source = [
      read('../ChatInput.tsx'),
      read('ComposerChips.tsx'),
      read('../MessageQueue.tsx'),
    ].join('\n');
    for (const name of [
      'br-composer-input',
      'br-composer-controls',
      'br-composer-controls__spacer',
      'br-composer-controls__model',
      'br-composer-chips',
      'br-composer-thumb',
      'br-composer-thumb__scrim',
      'br-queue-row',
      'br-queue-actions',
    ]) {
      expect(source, name).toContain(name);
    }
  });

  it('keeps an empty field to one 32px line', async (ctx) => {
    if (!page) return ctx.skip();
    await page.fill('#text', '');
    expect((await box('#text')).height).toBe(32);
  });

  it('grows the field with what is typed, then stops at ten lines and scrolls', async (ctx) => {
    if (!page) return ctx.skip();
    await page.fill('#text', 'one\ntwo\nthree');
    expect((await box('#text')).height).toBe(3 * 20 + 12);

    await page.fill('#text', Array.from({ length: 16 }, (_, i) => `line ${i}`).join('\n'));
    expect((await box('#text')).height).toBe(10 * 20 + 12);
    const scrolls = await page.$eval('#text', (el) => el.scrollHeight > el.clientHeight);
    expect(scrolls).toBe(true);
    await page.fill('#text', '');
  });

  it('lays the controls row out at 32px with the + glyph on the text edge', async (ctx) => {
    if (!page) return ctx.skip();
    const card = await box('#card');
    expect((await box('#controls')).height).toBe(32);
    // The card's text edge: its 1px border plus 16px of padding.
    expect((await box('#plus-glyph')).x - card.x).toBe(17);
    expect((await box('#text')).x - card.x).toBe(17);
  });

  it('ends the row with Send on the card inner right edge', async (ctx) => {
    if (!page) return ctx.skip();
    const card = await box('#card');
    const send = await box('#send');
    expect(send.width).toBe(32);
    expect(card.x + card.width - (send.x + send.width)).toBe(1 + 12);
  });

  it('keeps a queue row at 28px and shows its actions only on hover', async (ctx) => {
    if (!page) return ctx.skip();
    expect((await box('#queue-row')).height).toBe(28);
    const opacity = () =>
      page!.$eval('#queue-actions', (el) => Number(getComputedStyle(el).opacity));
    await page.mouse.move(0, 0);
    expect(await opacity()).toBe(0);
    await page.hover('#queue-row');
    expect(await opacity()).toBe(1);
    await page.mouse.move(0, 0);
  });

  it('draws a 48px thumbnail under a translucent scrim', async (ctx) => {
    if (!page) return ctx.skip();
    const thumb = await box('#thumb');
    expect(thumb.width).toBe(48);
    expect(thumb.height).toBe(48);
    const background = await page.$eval('#scrim', (el) => getComputedStyle(el).backgroundColor);
    const alpha = Number(/rgba\([^)]*,\s*([\d.]+)\)/.exec(background)?.[1] ?? '1');
    expect(alpha).toBeGreaterThan(0);
    expect(alpha).toBeLessThan(1);
  });
});
