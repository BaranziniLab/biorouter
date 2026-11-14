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
 *    and stops at 40% of the window, then scrolls (Crew's field);
 *  - the card is ONE ROW under whatever goes with the message (owner,
 *    2026-11-13): `+`, the text, the microphone and Send, 32px with one line of
 *    text, every control on that line's centre, and the controls keep to the
 *    LAST line as the text grows. The card is 8px all round; the `+` glyph sits
 *    on the stack's 16px edge, the text 4px past the `+`, and Send ends the row
 *    at the card's inner right edge, with the 28px microphone immediately left
 *    of it (DD-60) and its recording pill 28px high in the same place. With no
 *    `+` (a decision chat) the text itself starts on the stack's edge;
 *  - a queue row is 28px, reads `↳ text` at rest, and its actions (and its
 *    grip, in place of the arrow) appear on hover and on keyboard focus;
 *  - an image thumbnail is 48px, its loading scrim is translucent, and its
 *    remove control appears on hover and on keyboard focus;
 *  - a quotation is a 2px rule with no fill (never a box in the card), its
 *    words stop at four lines, and in a queue row it folds to the row's line;
 *  - the mention menu enters with the menus' one animation;
 *  - the Commands and @ lists open above the card, 8px clear of its top edge,
 *    from the text's left edge, and never up into the window-drag band
 *    (V2-R5-2). These lay the list out with the real `mentionMenuPlacement.ts`,
 *    bundled into the page, so the arithmetic under test is the shipped one.
 *
 * The class names below are asserted to be the ones `ChatInput.tsx` and the
 * composer components use, so this markup cannot drift from the real one
 * unnoticed.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
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
  --border-strong: #d2d2cd;
  --background-muted: #f4f3f1;
  --overlay-hover: rgb(31 30 28 / 0.05);
  --scrim: rgba(31, 30, 28, 0.18);
  --text-default: #1f1e1c;
  --text-muted: #6b6862;
  --background-default: #ffffff;
  --radius-full: 9999px;
  --dur-fast-min: 0ms;
  --dur-fast-max: 0ms;
  --ease-out: linear;
  --z-modal-dropdown: 500;
}
* { box-sizing: border-box; }
body { margin: 0; font: 14px/20px Arial, sans-serif; }
button { font: inherit; padding: 0; border: 0; background: none; }
textarea { font: inherit; margin: 0; }
p { margin: 0; }`;

/**
 * The card at `ChatInput`'s values: `relative flex flex-col`, a 1px border, and
 * its 8px padding authored in `composer.css` (`.br-composer-chat-card`), so it
 * is not written here.
 */
const CARD = `position: relative; display: flex; flex-direction: column; width: 640px;
  border: 1px solid #ddd; border-radius: 12px;`;
const CARD_CLASS = 'br-composer-chat-card biorouter-composer-card';
/** A ghost round `sm` button (28px) and the round default one (32px), with a 16px glyph. */
const BUTTON = (size: number) =>
  `display: inline-flex; align-items: center; justify-content: center; width: ${size}px; height: ${size}px; flex: none;`;
const GLYPH = 'display: block; width: 16px; height: 16px; background: #000;';
/** `QuotedTextChip`'s markup; the words at `text-secondary`'s 13/18. */
const QUOTE = (id: string, words: string) =>
  `<span id="${id}" class="br-quote-chip"><span class="br-quote-chip__body">` +
  `<span class="br-quote-chip__source">Response</span>` +
  `<span id="${id}-text" class="br-quote-chip__text" style="font-size: 13px; line-height: 18px">${words}</span>` +
  `</span></span>`;

/** The text field as `ChatInput` draws it: `py-1.5` on a 20px line, inside the row's form. */
const TEXTAREA = (id: string) =>
  `<textarea id="${id}" rows="1" class="br-composer-input" ` +
  'style="display: block; width: 100%; resize: none; border: 0; padding: 6px 0; line-height: 20px; font-size: 14px;"></textarea>';
/** The microphone's slot: `DictationButton`'s own `br-dictation-slot` inside the row's slot. */
const MIC =
  `<span class="br-composer-row__slot"><span class="br-dictation-slot">` +
  `<button id="mic" class="br-dictation-button" data-dictation-phase="idle" style="${BUTTON(28)}"><i id="mic-glyph" style="${GLYPH}"></i></button>` +
  '</span></span>';
/** Send, always the row's last child: the tooltip trigger's span, then the 32px round button. */
const SEND = (id: string) =>
  `<span class="br-composer-row__slot"><span style="flex-shrink: 0">` +
  `<button id="${id}" class="br-composer-send" style="${BUTTON(32)}"><span class="br-composer-send__glyph"><i style="${GLYPH}"></i></span></button>` +
  '</span></span>';

const MARKUP = `
<div id="card" class="${CARD_CLASS}" style="${CARD}">
  <div class="br-queue">
    <ul class="br-queue-list">
      <li id="queue-row" class="br-queue-row">
        <span class="br-queue-glyph">
          <span id="queue-arrow" class="br-queue-arrow">↳</span>
          <span id="queue-grip" class="br-queue-grip"><i style="${GLYPH}"></i></span>
        </span>
        <button id="queue-edit" class="br-queue-text br-queue-edit">and rerun the fit</button>
        <div id="queue-actions" class="br-queue-actions">
          <button style="${BUTTON(24)}">x</button>
        </div>
      </li>
      <li id="queue-quote-row" class="br-queue-row">
        <span class="br-queue-arrow">↳</span>
        <button class="br-queue-text br-queue-edit">${QUOTE('queue-quote', 'a\nquoted\nreply')}</button>
      </li>
    </ul>
  </div>
  <ul class="br-composer-chips">
    <li>
      <div id="thumb" class="br-composer-thumb">
        <div id="scrim" class="br-composer-thumb__scrim">
          <span id="spinner-ground" class="br-composer-thumb__spinner"><i style="${GLYPH}"></i></span>
        </div>
        <span id="thumb-remove" class="br-composer-thumb__remove">
          <button id="thumb-remove-button" style="${BUTTON(20)}">x</button>
        </span>
      </div>
    </li>
    <li>${QUOTE('quote', Array.from({ length: 6 }, (_, i) => `line ${i}`).join('\n'))}</li>
  </ul>
  <div id="controls" class="br-composer-row">
    <span class="br-composer-row__slot">
      <button id="plus" class="br-composer-plus" style="${BUTTON(32)}"><i id="plus-glyph" class="br-composer-plus__glyph" style="${GLYPH}"></i></button>
    </span>
    <form id="field" class="br-composer-row__field">${TEXTAREA('text')}</form>
    ${MIC}
    ${SEND('send')}
  </div>
</div>
<div id="decision-card" class="${CARD_CLASS}" style="${CARD}; margin-top: 24px">
  <div id="decision-row" class="br-composer-row" data-plus="none">
    <form class="br-composer-row__field">${TEXTAREA('decision-text')}</form>
    ${SEND('decision-send')}
  </div>
</div>
<div id="mention" class="br-mention-menu" style="position: fixed; left: 40px; top: 860px; width: 320px; height: 64px; transform: translateY(-100%)"></div>`;

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

/** The one row as `ChatInput` draws it (`+`, the text, Send), for the list placement cases. */
const ROW =
  `<div class="br-composer-row"><span class="br-composer-row__slot"><button style="${BUTTON(32)}">+</button></span>` +
  `<form class="br-composer-row__field">${TEXTAREA('text')}</form>${SEND('row-send')}</div>`;
/** A chips row (one thumbnail) and a queue row, the things that push the field down the card. */
const CARD_TOP_ROWS =
  '<div class="br-queue"><ul class="br-queue-list"><li class="br-queue-row">' +
  '<span class="br-queue-arrow">↳</span><button class="br-queue-text br-queue-edit">and rerun it</button>' +
  '</li></ul></div>' +
  '<ul id="chips" class="br-composer-chips"><li><div class="br-composer-thumb"></div></li></ul>';

describe('the composer recipes in a real layout engine', () => {
  let browser: Browser | null = null;
  let page: Page | null = null;
  /** A 1440x900 window, the size the review measured the overlap at. */
  let placementPage: Page | null = null;
  let placementScript = '';

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
    const bundle = await build({
      entryPoints: [resolve(here, 'mentionMenuPlacement.ts')],
      bundle: true,
      write: false,
      format: 'iife',
      globalName: 'MentionMenu',
      platform: 'browser',
      logLevel: 'silent',
    });
    placementScript = bundle.outputFiles[0].text;
    placementPage = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  }, 120_000);

  afterAll(async () => {
    await placementPage?.close();
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
      read('composerCard.ts'),
      read('ComposerPlusMenu.tsx'),
      read('ComposerChips.tsx'),
      read('../MessageQueue.tsx'),
      read('../QuotedTextChip.tsx'),
      read('../MentionPopover.tsx'),
      read('../dictation/DictationButton.tsx'),
    ].join('\n');
    for (const name of [
      'br-composer-card',
      'br-composer-chat-card',
      'br-composer-input',
      'br-composer-row',
      'br-composer-row__slot',
      'br-composer-row__field',
      "data-plus={decisionChat ? 'none' : undefined}",
      'br-composer-plus',
      'br-composer-plus__glyph',
      'br-composer-send',
      'br-composer-send__glyph',
      'br-composer-chips',
      'br-composer-thumb',
      'br-composer-thumb__scrim',
      'br-queue-row',
      'br-queue-actions',
      'br-queue-glyph',
      'br-queue-grip',
      'br-quote-chip',
      'br-quote-chip__body',
      'br-quote-chip__source',
      'br-quote-chip__text',
      'br-mention-menu',
      'biorouter-composer-card',
      'br-dictation-slot',
      'br-dictation-button',
      'br-dictation-pill',
      'br-dictation-bar',
      'br-dictation-timer',
    ]) {
      expect(source, name).toContain(name);
    }
    // The second row of controls is gone: the model chip is on the footer line.
    for (const name of [
      'br-composer-controls',
      'br-composer-controls__spacer',
      'br-composer-controls__model',
    ]) {
      expect(source, name).not.toContain(name);
    }
    // The list is placed by the module the placement cases below lay out.
    expect(read('../ChatInput.tsx')).toContain('position: mentionMenuAnchor(textArea)');
    expect(read('../MentionPopover.tsx')).toContain(
      'mentionMenuPlacement(position, window.innerWidth)'
    );
    expect(read('../MentionPopover.tsx')).toContain('style={placement}');
  });

  it('keeps an empty field to one 32px line', async (ctx) => {
    if (!page) return ctx.skip();
    await page.fill('#text', '');
    expect((await box('#text')).height).toBe(32);
  });

  it('grows the field with what is typed, then stops at 40% of the window and scrolls', async (ctx) => {
    if (!page) return ctx.skip();
    await page.fill('#text', 'one\ntwo\nthree');
    expect((await box('#text')).height).toBe(3 * 20 + 12);

    // 40vh of the 900px viewport; thirty lines would be 612px.
    await page.fill('#text', Array.from({ length: 30 }, (_, i) => `line ${i}`).join('\n'));
    expect((await box('#text')).height).toBe(0.4 * 900);
    const scrolls = await page.$eval('#text', (el) => el.scrollHeight > el.clientHeight);
    expect(scrolls).toBe(true);
    await page.fill('#text', '');
  });

  const centre = (rect: { y: number; height: number }) => rect.y + rect.height / 2;

  it('lays + , the text, the microphone and Send out on ONE 32px row', async (ctx) => {
    if (!page) return ctx.skip();
    await page.fill('#text', '');
    const card = await box('#card');
    const row = await box('#controls');
    const text = await box('#text');
    expect(row.height).toBe(32);
    // The text IS the row's line: no second row under it.
    expect(text.y).toBe(row.y);
    expect(text.height).toBe(32);
    // 8px all round: the row ends 1px border plus 8px of padding above the card's bottom.
    expect(card.y + card.height - (row.y + row.height)).toBe(1 + 8);
    // Every control on the text line's centre, the 28px microphone included.
    for (const control of ['#plus', '#plus-glyph', '#mic', '#send']) {
      expect(centre(await box(control)), control).toBe(centre(text));
    }
  });

  it('puts the + glyph on the stack edge and the text 4px past the +', async (ctx) => {
    if (!page) return ctx.skip();
    const card = await box('#card');
    const plus = await box('#plus');
    expect([plus.width, plus.height]).toEqual([32, 32]);
    // The card's 1px border, its 8px padding and the 32px button's 8px inset:
    // the glyph sits on the 16px edge the stack above the row starts on.
    expect((await box('#plus-glyph')).x - card.x).toBe(1 + 8 + 8);
    expect((await box('.br-composer-chips')).x - card.x).toBe(1 + 8 + 8);
    expect((await box('.br-queue')).x - card.x).toBe(1 + 8 + 8);
    // The row's 4px gap and the field's own 4px, so the caret has air.
    expect((await box('#text')).x - (plus.x + plus.width)).toBe(4 + 4);
  });

  it('starts the text on the stack edge when there is no + (a decision chat)', async (ctx) => {
    if (!page) return ctx.skip();
    const card = await box('#decision-card');
    const text = await box('#decision-text');
    expect(text.x - card.x).toBe(1 + 8 + 8);
    // Send still ends the row at the card's inner right edge.
    const send = await box('#decision-send');
    expect(card.x + card.width - (send.x + send.width)).toBe(1 + 8);
    expect((await box('#decision-row')).height).toBe(32);
  });

  it('keeps the controls on the last line as the text grows', async (ctx) => {
    if (!page) return ctx.skip();
    await page.fill('#text', 'one\ntwo\nthree');
    const text = await box('#text');
    const row = await box('#controls');
    expect(text.height).toBe(3 * 20 + 12);
    expect(row.height).toBe(text.height);
    const lastLineCentre = text.y + text.height - 16;
    for (const control of ['#plus', '#mic', '#send']) {
      const rect = await box(control);
      expect(rect.y + rect.height, `${control} keeps to the bottom`).toBe(
        control === '#mic' ? text.y + text.height - 2 : text.y + text.height
      );
      expect(centre(rect), `${control} on the last line`).toBe(lastLineCentre);
    }
    await page.fill('#text', '');
  });

  it('ends the row with Send on the card inner right edge', async (ctx) => {
    if (!page) return ctx.skip();
    const card = await box('#card');
    const send = await box('#send');
    expect([send.width, send.height]).toEqual([32, 32]);
    // The card's 1px border and its 8px padding.
    expect(card.x + card.width - (send.x + send.width)).toBe(1 + 8);
    // Send is the row's last child.
    expect(
      await page.$eval('#controls', (row) =>
        row.lastElementChild?.contains(document.getElementById('send'))
      )
    ).toBe(true);
  });

  it('puts the 28px microphone immediately left of Send, and keeps its pill there', async (ctx) => {
    if (!page) return ctx.skip();
    const mic = await box('#mic');
    const send = await box('#send');
    expect([mic.width, mic.height]).toEqual([28, 28]);
    // The row's 4px gap, and nothing between them.
    expect(send.x - (mic.x + mic.width)).toBe(4);
    expect(centre(mic)).toBe(centre(send));
    await page.$eval('#mic', (el) => {
      el.setAttribute('data-dictation-phase', 'recording');
      el.style.width = '';
      el.innerHTML =
        '<span class="br-dictation-pill"><span class="br-dictation-bars">' +
        '<span class="br-dictation-bar" data-bar="0"></span><span class="br-dictation-bar" data-bar="1"></span>' +
        '<span class="br-dictation-bar" data-bar="2"></span><span class="br-dictation-bar" data-bar="3"></span>' +
        '<span class="br-dictation-bar" data-bar="4"></span></span>' +
        '<span class="br-dictation-timer">0:07</span></span>';
    });
    const pill = await box('#mic');
    const card = await box('#card');
    expect(pill.height).toBe(28);
    expect(pill.width).toBeGreaterThan(28);
    expect((await box('#send')).x - (pill.x + pill.width)).toBe(4);
    expect(centre(pill)).toBe(centre(await box('#send')));
    expect(card.x + card.width - ((await box('#send')).x + 32)).toBe(1 + 8);
    expect((await box('#controls')).height).toBe(32);
    await page.$eval('#mic', (el) => {
      el.setAttribute('data-dictation-phase', 'idle');
      el.style.width = '28px';
      el.innerHTML = '<i id="mic-glyph"></i>';
    });
  });

  it('keeps a queue row at 28px and shows its actions only on hover', async (ctx) => {
    if (!page) return ctx.skip();
    expect((await box('#queue-row')).height).toBe(28);
    const opacity = (selector: string) =>
      page!.$eval(selector, (el) => Number(getComputedStyle(el).opacity));
    await page.mouse.move(0, 0);
    expect(await opacity('#queue-actions')).toBe(0);
    // `↳ text` at rest; the grip takes the arrow's place on hover.
    expect(await opacity('#queue-arrow')).toBe(1);
    expect(await opacity('#queue-grip')).toBe(0);
    await page.hover('#queue-row');
    expect(await opacity('#queue-actions')).toBe(1);
    expect(await opacity('#queue-arrow')).toBe(0);
    expect(await opacity('#queue-grip')).toBe(1);
    await page.mouse.move(0, 0);
  });

  it("shows a queue row's actions and grip to the keyboard too, not only to the pointer", async (ctx) => {
    if (!page) return ctx.skip();
    const opacity = (selector: string) =>
      page!.$eval(selector, (el) => Number(getComputedStyle(el).opacity));
    await page.mouse.move(0, 0);
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
    expect(await opacity('#queue-actions')).toBe(0);
    await page.focus('#queue-edit');
    expect(await opacity('#queue-actions')).toBe(1);
    expect(await opacity('#queue-grip')).toBe(1);
    expect(await opacity('#queue-arrow')).toBe(0);
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  });

  it("shows a thumbnail's remove control on hover and on keyboard focus", async (ctx) => {
    if (!page) return ctx.skip();
    const opacity = () =>
      page!.$eval('#thumb-remove', (el) => Number(getComputedStyle(el).opacity));
    await page.mouse.move(0, 0);
    expect(await opacity()).toBe(0);
    await page.hover('#thumb');
    expect(await opacity()).toBe(1);
    await page.mouse.move(0, 0);
    await page.focus('#thumb-remove-button');
    expect(await opacity()).toBe(1);
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
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
    // The spinner sits on a 24px round ground at the thumbnail's centre.
    const ground = await box('#spinner-ground');
    expect(ground.width).toBe(24);
    expect(ground.height).toBe(24);
    expect(ground.x + 12).toBe(thumb.x + 24);
    expect(ground.y + 12).toBe(thumb.y + 24);
  });

  it('draws a quotation as a 2px rule with no fill, its words capped at four lines', async (ctx) => {
    if (!page) return ctx.skip();
    const style = await page.$eval('#quote', (el) => {
      const cs = getComputedStyle(el);
      return {
        background: cs.backgroundColor,
        left: cs.borderLeftWidth,
        top: cs.borderTopWidth,
        padding: cs.paddingLeft,
      };
    });
    expect(style.background).toBe('rgba(0, 0, 0, 0)');
    expect(style.left).toBe('2px');
    expect(style.top).toBe('0px');
    // The words sit 12px inside the rule.
    expect((await box('#quote-text')).x - (await box('#quote')).x).toBe(2 + 12);
    expect((await box('#quote-text')).height).toBe(4 * 18);
    const scrolls = await page.$eval('#quote-text', (el) => el.scrollHeight > el.clientHeight);
    expect(scrolls).toBe(true);
  });

  it('folds a quotation in a queue row to the row line', async (ctx) => {
    if (!page) return ctx.skip();
    expect((await box('#queue-quote-row')).height).toBe(28);
    expect((await box('#queue-quote-text')).height).toBe(18);
  });

  it('opens the mention menu with the menus entrance, composed with its lift', async (ctx) => {
    if (!page) return ctx.skip();
    const motion = await page.$eval('#mention', (el) => {
      const cs = getComputedStyle(el);
      return { name: cs.animationName, origin: cs.transformOrigin, transform: cs.transform };
    });
    expect(motion.name).toBe('br-mention-in');
    expect(motion.origin).toBe('0px 0px');
    // The inline lift is untouched by the animation (it moves `translate` and `scale`).
    expect(motion.transform).toBe('matrix(1, 0, 0, 1, 0, -64)');
  });

  /**
   * Lays a card out at `cardTop` (or 104px off the window's bottom, where the
   * chat's card sits at 1440x900), opens a list of `rows` 32px rows with the real
   * anchor and placement, and returns the boxes. The list's markup is
   * `MentionPopover`'s: a fixed flex column, 4px padding, a scrolling body.
   */
  const placeList = async (cardTop: number | 'bottom', inside: string, rows: number) => {
    const where = cardTop === 'bottom' ? 'bottom: 104px' : `top: ${cardTop}px`;
    const list = Array.from({ length: rows }, () => '<div style="height: 32px"></div>').join('');
    await placementPage!.setContent(
      `<!doctype html><html><head><style>${TOKENS}${read('composer.css')}</style></head><body>` +
        `<div id="card" class="${CARD_CLASS}" style="${CARD}; position: fixed; left: 400px; ${where}">` +
        `${inside}${ROW}</div>` +
        '<div id="list" class="br-mention-menu" style="position: fixed; display: flex; flex-direction: column; overflow: hidden; padding: 4px;">' +
        `<div id="list-body" style="flex: 1; min-height: 0; overflow-y: auto">${list}</div></div>` +
        `<script>${placementScript}</script>` +
        // In a block: `setContent` keeps the window, and with it any top-level `const`.
        `<script>{
          const placed = MentionMenu.mentionMenuPlacement(
            MentionMenu.mentionMenuAnchor(document.getElementById('text')),
            window.innerWidth
          );
          const menu = document.getElementById('list');
          for (const [key, value] of Object.entries(placed)) {
            menu.style[key] = typeof value === 'number' ? value + 'px' : value;
          }
        }</script></body></html>`
    );
    const on = async (selector: string) => {
      const rect = await placementPage!.locator(selector).boundingBox();
      if (!rect) throw new Error(`no box for ${selector}`);
      return rect;
    };
    return { card: await on('#card'), text: await on('#text'), list: await on('#list'), on };
  };

  it('opens the Commands and @ lists 8px above the card, clear of its focus edge', async (ctx) => {
    if (!placementPage) return ctx.skip();
    const { card, text, list } = await placeList('bottom', '', 3);
    // The instrument: the field's top sits 9px inside the card (1px border,
    // 8px padding), so a list hung from the field would cover the card's edge.
    expect(text.y - card.y).toBe(9);
    expect(list.height).toBe(3 * 32 + 8);
    expect(list.y + list.height).toBe(card.y - 8);
    // From the text's left edge, as before.
    expect(list.x).toBe(text.x);
  });

  it('keeps the gap above a card that holds a queue and chips, and covers neither', async (ctx) => {
    if (!placementPage) return ctx.skip();
    const { card, text, list, on } = await placeList('bottom', CARD_TOP_ROWS, 3);
    const chips = await on('#chips');
    // The field is pushed well down the card here.
    expect(text.y - card.y).toBeGreaterThan(9 + 48);
    expect(list.y + list.height).toBe(card.y - 8);
    expect(list.y + list.height).toBeLessThan(chips.y);
  });

  it('shortens the list under a card high in the window, never rising into the drag band', async (ctx) => {
    if (!placementPage) return ctx.skip();
    // Twelve rows want 392px; the card's top at 200px leaves 200 - 8 - 52 = 140.
    const { card, list } = await placeList(200, '', 12);
    expect(list.y + list.height).toBe(card.y - 8);
    expect(list.y).toBe(52);
    const scrolls = await placementPage.$eval(
      '#list-body',
      (el) => el.scrollHeight > el.clientHeight
    );
    expect(scrolls).toBe(true);
  });
});
