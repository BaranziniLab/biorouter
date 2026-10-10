// @vitest-environment node
/**
 * The app sidebar measured in a real layout engine (spec 3.4 acceptance, 5.7;
 * owner message 12: "measured, not assumed").
 *
 * jsdom lays nothing out and runs no CSS, so the component tests can only pin
 * class names. What the redesign promises is geometry: 28px rows on a 30px
 * pitch, the 16px icon column at x=16 and the label column at x=40 at every
 * sidebar width (216, 288 and 360), the band's hairline at y=44, and the ⋯
 * centred over the trailing slot. The authored `sidebar.css` is loaded into
 * Chromium with the REAL token values read out of `styles/main.css`, over
 * markup shaped like the app's.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const SIDEBAR_CSS = resolve(here, 'sidebar.css');
const MAIN_CSS = resolve(here, '../../styles/main.css');

/** The tokens the geometry reads. Their values come from main.css, so a token change is caught. */
const TOKEN_NAMES = [
  '--chrome-height',
  '--control-md',
  '--row-height-nav',
  '--icon-row',
  '--radius-element',
  '--radius-inner',
  '--text-secondary',
  '--text-secondary--line-height',
  '--text-supporting',
  '--text-supporting--line-height',
  '--dur-fast-min',
  '--dur-fast',
  '--dur-fast-max',
  '--dur-slow',
  '--ease-out',
];

function readTokens(css: string): string {
  const lines = TOKEN_NAMES.map((name) => {
    const match = css.match(new RegExp(`${name.replace(/-/g, '\\-')}:\\s*([^;]+);`));
    if (!match) throw new Error(`main.css does not declare ${name}`);
    return `  ${name}: ${match[1].trim()};`;
  });
  return `:root {\n${lines.join('\n')}\n}`;
}

/** WS-ICONS' 16px slot for every chat glyph, read from main.css rather than restated. */
function readChatKindIconRule(css: string): string {
  const match = css.match(/\.br-chat-kind-icon\s*\{[^}]*\}/);
  if (!match) throw new Error('main.css has no .br-chat-kind-icon rule');
  return match[0];
}

/** Tailwind's preflight, the parts the layout depends on. */
const PREFLIGHT = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; font-family: Arial, sans-serif; }
button { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
ul { margin: 0; padding: 0; }
svg { display: block; }
:root { --sidebar-hover: #eee; --sidebar-active: #e6e6e6; --sidebar-icon: #333;
  --sidebar-border: #ccc; --accent-bar: #e8603c; --text-muted: #666; --text-default: #111;
  --text-subtle: #777; --background-focus: #ddd; --border-focus: #555; --border-subtle: #ccc;
  --background-default: #fff; }
`;

const navRow = (id: string, label: string, extra = '') =>
  `<li><button type="button" class="br-nav-row" id="${id}"${extra}>` +
  `<svg class="br-nav-row-icon" id="${id}-icon" viewBox="0 0 16 16"></svg>` +
  `<span class="br-nav-row-label" id="${id}-label">${label}</span></button></li>`;

const chatRow = (id: string, title: string, active = false) =>
  `<li class="br-chat-row-item">` +
  `<button type="button" class="br-nav-row br-chat-row" id="${id}"${active ? ' aria-current="page"' : ''}>` +
  `<svg class="br-chat-kind-icon" id="${id}-glyph" viewBox="0 0 16 16"></svg>` +
  `<span class="br-nav-row-label" id="${id}-label">${title}</span>` +
  `<span class="br-chat-row-trailing" id="${id}-trailing"><span class="br-chat-row-ring"></span></span>` +
  `</button>` +
  `<button type="button" class="br-chat-row-more" id="${id}-more" style="width:24px;height:24px"></button>` +
  `</li>`;

function page(css: string, width: number): string {
  return (
    '<!doctype html><html><head><style>' +
    css +
    '</style></head><body>' +
    `<div id="sidebar" style="display:flex;flex-direction:column;width:${width}px;height:760px">` +
    // The band: `h-chrome border-b`, the container now has no padding of its own.
    '<div id="band" style="flex:none;height:var(--chrome-height);border-bottom:1px solid var(--sidebar-border)"></div>' +
    '<div class="br-sidebar-brand" id="brand"><div class="br-sidebar-brand-row" id="brand-row">' +
    '<svg class="br-sidebar-wordmark" id="wordmark" viewBox="0 0 120 24"></svg></div></div>' +
    '<div class="br-nav-group"><ul class="br-nav-list" id="nav">' +
    navRow('home', 'Home') +
    navRow('newchat', 'New chat') +
    navRow('crew', 'Crew') +
    '<li><button type="button" class="br-nav-row" id="components" aria-expanded="false">' +
    '<svg class="br-nav-chevron" id="components-chevron" viewBox="0 0 16 16"></svg>' +
    '<span class="br-nav-row-label" id="components-label">Components</span></button></li>' +
    '</ul></div>' +
    '<div class="br-sidebar-chats" data-expanded="true">' +
    '<div class="br-sidebar-chats-header" id="chats-header">' +
    '<button type="button" class="br-sidebar-chats-toggle" id="chats-toggle"><span class="br-nav-row-label">Chats</span></button>' +
    '<div class="br-sidebar-chats-actions"><button type="button" style="width:24px;height:24px"></button></div>' +
    '</div>' +
    '<div class="br-sidebar-chats-scroll" id="scroll"><div class="br-sidebar-chat-groups">' +
    '<section class="br-sidebar-group"><p class="br-sidebar-bucket" id="bucket">Today</p>' +
    '<ul class="br-nav-list">' +
    chatRow(
      'chat1',
      'Cohort pull for the IRB amendment with a title long enough to truncate',
      true
    ) +
    chatRow('chat2', 'Volcano plot') +
    chatRow('chat3', 'Crew · #general') +
    '</ul></section></div></div></div>' +
    '<div class="br-sidebar-footer" id="footer"><ul class="br-nav-list">' +
    navRow('settings', 'Settings') +
    '</ul></div>' +
    '</div></body></html>'
  );
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

const EPSILON = 0.5;
const near = (actual: number, expected: number, what: string) =>
  expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThanOrEqual(
    EPSILON
  );

type Box = {
  top: number;
  bottom: number;
  left: number;
  right: number;
  width: number;
  height: number;
};

describe('the app sidebar in a real layout engine', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome) — skipping the sidebar geometry ' +
          'tests. Run `npx playwright install chromium` to enable them.'
      );
    }
    const main = readFileSync(MAIN_CSS, 'utf8');
    css = [
      readTokens(main),
      PREFLIGHT,
      readChatKindIconRule(main),
      readFileSync(SIDEBAR_CSS, 'utf8'),
    ].join('\n');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(width: number): Promise<Page> {
    const tab = await browser!.newPage({
      viewport: { width: 1440, height: 900 },
      reducedMotion: 'reduce',
    });
    await tab.setContent(page(css, width));
    return tab;
  }

  const boxes = (tab: Page, ids: string[]) =>
    tab.evaluate((list) => {
      const out: Record<string, Box> = {};
      for (const id of list) {
        const r = document.getElementById(id)!.getBoundingClientRect();
        out[id] = {
          top: r.top,
          bottom: r.bottom,
          left: r.left,
          right: r.right,
          width: r.width,
          height: r.height,
        };
      }
      return out;
    }, ids);

  for (const width of [216, 288, 360]) {
    it(`draws 28px rows on a 30px pitch, icons at x=16 and labels at x=40, at ${width}px`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(width);
      const rows = ['home', 'newchat', 'crew', 'components', 'chat1', 'chat2', 'chat3', 'settings'];
      const b = await boxes(tab, [
        ...rows,
        'home-icon',
        'home-label',
        'newchat-icon',
        'newchat-label',
        'components-chevron',
        'components-label',
        'chat1-glyph',
        'chat1-label',
        'chat2-glyph',
        'chat2-label',
        'settings-icon',
        'settings-label',
        'chat1-trailing',
        'chat1-more',
        'band',
        'brand',
        'wordmark',
        'chats-header',
        'bucket',
      ]);

      for (const id of rows) {
        near(b[id].height, 28, `${id} height`);
        near(b[id].left, 8, `${id} left edge`);
        near(b[id].right, width - 8, `${id} right edge`);
      }
      // The pitch: 28px rows with a 2px gap.
      near(b.newchat.top - b.home.top, 30, 'nav pitch');
      near(b.crew.top - b.newchat.top, 30, 'nav pitch');
      near(b.chat2.top - b.chat1.top, 30, 'chat pitch');
      near(b.chat3.top - b.chat2.top, 30, 'chat pitch');

      // One icon column and one label column for nav rows and chat rows alike.
      for (const icon of [
        'home-icon',
        'newchat-icon',
        'components-chevron',
        'chat1-glyph',
        'chat2-glyph',
        'settings-icon',
      ]) {
        near(b[icon].left, 16, `${icon} x`);
        near(b[icon].width, 16, `${icon} width`);
      }
      for (const label of [
        'home-label',
        'newchat-label',
        'components-label',
        'chat1-label',
        'chat2-label',
        'settings-label',
      ]) {
        near(b[label].left, 40, `${label} x`);
      }
      // The title truncates before the 16px trailing slot, 8px from the row's end.
      near(b['chat1-trailing'].right, width - 16, 'trailing slot right');
      expect(b['chat1-label'].right).toBeLessThanOrEqual(b['chat1-trailing'].left - 8 + EPSILON);
      // The ⋯ (24px) is centred on the trailing slot, vertically centred in the row.
      near(
        b['chat1-more'].left + b['chat1-more'].width / 2,
        b['chat1-trailing'].left + b['chat1-trailing'].width / 2,
        '⋯ centre x'
      );
      near(b['chat1-more'].top - b.chat1.top, 2, '⋯ top inset');

      // The band's hairline at y=44, full width, and the 44px brand block under it.
      near(b.band.bottom, 44, 'band bottom');
      near(b.band.width, width, 'band width');
      near(b.brand.top, 44, 'brand top');
      near(b.brand.height, 44, 'brand height');
      near(b.wordmark.left, 16, 'wordmark x');
      near(b.wordmark.height, 20, 'wordmark height');
      // The Chats header is a 28px row; the bucket label starts on the icon column.
      near(b['chats-header'].height, 28, 'Chats header height');
      const bucketText = await tab.evaluate(() => {
        const range = document.createRange();
        range.selectNodeContents(document.getElementById('bucket')!);
        return range.getBoundingClientRect().left;
      });
      near(bucketText, 16, 'bucket label text x');
      await tab.close();
    });
  }

  it('marks the current row with the 2px accent rail inset 7px', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(288);
    const rail = await tab.evaluate(() => {
      const style = getComputedStyle(document.getElementById('chat1')!, '::before');
      const other = getComputedStyle(document.getElementById('chat2')!, '::before');
      return {
        width: style.width,
        top: style.top,
        bottom: style.bottom,
        background: style.backgroundColor,
        otherContent: other.content,
        fontSize: getComputedStyle(document.getElementById('chat1')!).fontSize,
      };
    });
    expect(rail).toMatchObject({ width: '2px', top: '7px', bottom: '7px' });
    expect(rail.background).toBe('rgb(232, 96, 60)');
    expect(rail.otherContent).toBe('none');
    expect(rail.fontSize).toBe('13px');
    await tab.close();
  });
});
