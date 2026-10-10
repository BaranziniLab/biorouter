// @vitest-environment node
/**
 * A message's floating actions, measured in a real layout engine (spec 3.6,
 * 5.7; acceptance "Hovering a message shows the floating cluster with no layout
 * shift").
 *
 * jsdom lays nothing out and evaluates no `:hover`, so a component test cannot
 * tell a cluster that floats from one that reserves a row. The real
 * `UserMessage` is rendered to markup and laid out in Chromium with the
 * authored stylesheet (`message-meta.css`) inside the 760px chat column. It pins
 * what the redesign is for: no row is reserved for the actions (the old meta row
 * added 28px under every message), the cluster sits 12px above the bubble's top
 * right edge, it is invisible and click-through at rest, and a hover reveals it
 * without moving the bubble by a pixel.
 *
 * Tailwind does not run here, so the 24px buttons are stood in for by one rule
 * (`TAILWIND_STANDINS`); everything the test measures about the cluster is
 * authored CSS.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import UserMessage from '../UserMessage';
import type { Message } from '../../api';

const here = dirname(fileURLToPath(import.meta.url));
const COLUMN = 760;

/** `main.css`'s values for the tokens the cluster reads. */
const TOKENS = `:root {
  --font-body: Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, sans-serif;
  --radius-element: 8px;
  --background-default: #ffffff;
  --shadow-popover: 0 4px 16px rgb(0 0 0 / 0.12);
  --inset-hairline: inset 0 0 0 1px rgb(0 0 0 / 0.08);
  --dur-fast-min: 95ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
}
body { margin: 0; font: 14px/20px var(--font-body); }
* { box-sizing: border-box; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
  overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }`;

/** The few Tailwind utilities the measured boxes depend on. */
const TAILWIND_STANDINS = `
.flex { display: flex; } .flex-col { flex-direction: column; } .justify-end { justify-content: flex-end; }
.w-full { width: 100%; } .w-fit { width: fit-content; } .max-w-\\[85\\%\\] { max-width: 85%; }
.min-w-0 { min-width: 0; } .self-end { align-self: flex-end; }
[data-testid='user-message-bubble'] { padding: 8px 14px; }
.br-message-actions button { width: 24px; height: 24px; padding: 0; border: 0; background: none; }
.br-message-actions svg { width: 14px; height: 14px; }`;

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

const message: Message = {
  id: 'm1',
  role: 'user',
  created: 1_700_000_000,
  content: [{ type: 'text', text: 'Which genes moved?' }],
  metadata: { userVisible: true, agentVisible: true },
};

interface Box {
  top: number;
  left: number;
  right: number;
  bottom: number;
  height: number;
}

interface Measure {
  row: Box;
  anchor: Box;
  bubble: Box;
  cluster: Box;
  opacity: number;
  pointerEvents: string;
}

describe('message actions in a real layout engine', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the message actions ' +
          'geometry tests. Run `npx playwright install chromium` to enable them.'
      );
    }
    css = readFileSync(resolve(here, '../message-meta.css'), 'utf8');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const html = renderToStaticMarkup(<UserMessage message={message} />);
    const tab = await browser!.newPage({ viewport: { width: 1000, height: 400 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${TOKENS}${TAILWIND_STANDINS}${css}</style></head>` +
        `<body><div id="column" style="width:${COLUMN}px;margin:80px 0 0 100px">` +
        `<div id="row">${html}</div></div></body></html>`
    );
    return tab;
  }

  const measure = (tab: Page): Promise<Measure> =>
    tab.evaluate(() => {
      const box = (selector: string) => {
        const r = document.querySelector(selector)!.getBoundingClientRect();
        return { top: r.top, left: r.left, right: r.right, bottom: r.bottom, height: r.height };
      };
      const cluster = document.querySelector('.br-message-actions')!;
      const style = getComputedStyle(cluster);
      return {
        row: box('#row'),
        anchor: box('.br-message-actions-anchor'),
        bubble: box("[data-testid='user-message-bubble']"),
        cluster: box('.br-message-actions'),
        opacity: Number(style.opacity),
        pointerEvents: style.pointerEvents,
      };
    });

  it('reserves no row: the message is exactly as tall as its bubble', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      const m = await measure(tab);
      expect(m.row.height).toBe(m.bubble.height);
      expect(m.anchor.height).toBe(m.bubble.height);
    } finally {
      await tab.close();
    }
  });

  it("floats 12px above the bubble, on the bubble's right edge, hidden and click-through", async () => {
    if (!browser) return;
    const tab = await open();
    try {
      const m = await measure(tab);
      expect(m.cluster.top).toBe(m.bubble.top - 12);
      expect(Math.abs(m.cluster.right - m.bubble.right)).toBeLessThanOrEqual(0.5);
      // Two 24px buttons and the time, with 2px of padding: one compact strip.
      expect(m.cluster.height).toBe(28);
      expect(m.opacity).toBe(0);
      expect(m.pointerEvents).toBe('none');
    } finally {
      await tab.close();
    }
  });

  it('shows on hover without moving the message by a pixel', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      const before = await measure(tab);
      await tab.hover("[data-testid='user-message-bubble']");
      await tab.waitForTimeout(250);
      const after = await measure(tab);
      expect(after.opacity).toBe(1);
      expect(after.pointerEvents).toBe('auto');
      expect(after.bubble).toEqual(before.bubble);
      expect(after.row).toEqual(before.row);
      expect(after.cluster).toEqual(before.cluster);
    } finally {
      await tab.close();
    }
  });

  it('shows on keyboard focus inside the message, so Tab reaches every action', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      await tab.focus('.br-message-actions button');
      await tab.waitForTimeout(250);
      const m = await measure(tab);
      expect(m.opacity).toBe(1);
    } finally {
      await tab.close();
    }
  });
});
