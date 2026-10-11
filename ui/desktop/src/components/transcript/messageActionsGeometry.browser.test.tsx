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
 * added 28px under every message), the cluster rises 26px above the message's
 * top right edge and ends 2px into it, so it covers the words of neither the
 * message it acts on nor the block a turn's gap above (vision V2-12, twice), it
 * is invisible and click-through at rest, and a hover reveals it without moving
 * anything by a pixel. The transcript cases lay out the list's own rows (their
 * `data-gap` and margins, `transcriptRhythm.ts`) and measure the cluster
 * against the glyph boxes of the text around it, which is what a reader sees.
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
import { MessageActions } from '../MessageMeta';
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
[data-testid='user-message-bubble'] { padding: 10px 14px; }
.br-message-actions button { width: 24px; height: 24px; padding: 0; border: 0; background: none; }
.br-message-actions svg { width: 14px; height: 14px; }
.relative { position: relative; } .items-center { align-items: center; } .gap-2 { gap: 8px; }
.mb-1 { margin-bottom: 4px; } .max-w-full { max-width: 100%; }
.mt-6 { margin-top: 24px; } .mt-2 { margin-top: 8px; } .mt-0 { margin-top: 0; } .pt-7 { padding-top: 28px; }
[data-provenance-kind] { display: inline-flex; align-items: center; height: 20px; padding: 0 6px; }
p { margin: 0; }`;

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
  text: Box;
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
        text: box("[data-testid='user-message-bubble'] > *"),
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

  it("floats 26px above the bubble, on the bubble's right edge, hidden and click-through", async () => {
    if (!browser) return;
    const tab = await open();
    try {
      const m = await measure(tab);
      expect(m.cluster.top).toBe(m.bubble.top - 26);
      // It ends 2px into the bubble, inside its 10px top padding: never on the
      // first line (vision V2-12).
      expect(m.cluster.bottom).toBe(m.bubble.top + 2);
      expect(m.text.top - m.cluster.bottom).toBeGreaterThanOrEqual(2);
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

/**
 * The cluster among its neighbours: the rows exactly as `ProgressiveMessageList`
 * draws them (a wrapper per message carrying `data-role`, `data-gap` and the
 * gap's margin), measured against the GLYPH boxes of the text around it. A
 * Range's client rects are a line's inline content area, the ascent to descent
 * box the glyphs paint in, without the half-leading, so "no overlap" here means
 * no ink under the cluster. Round 2 of the vision loop measured the old 8px rise
 * covering the right end of an answer's first line (cluster 724..900 x
 * 215..243 over a line at 223..244).
 */
describe('message actions between the rows of a transcript', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    css = readFileSync(resolve(here, '../message-meta.css'), 'utf8');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  const FULL_LINE =
    'PCR, invented by Kary Mullis and enabled in practice by thermostable polymerases ' +
    'like Taq, is a cyclic amplification method whose major variants transformed ' +
    'molecular biology and medicine by making rapid, sensitive DNA detection routine.';

  const user = (id: string, text: string, metadata: Partial<Message['metadata']> = {}) =>
    renderToStaticMarkup(
      <UserMessage
        message={{
          ...message,
          id,
          content: [{ type: 'text', text }],
          metadata: { userVisible: true, agentVisible: true, ...metadata },
        }}
      />
    );

  /** BioRouterMessage's answer: the scope, the cluster first, then the words. */
  const answer = (text: string) =>
    `<div class="br-message-actions-scope flex flex-col" data-answer>` +
    renderToStaticMarkup(
      <MessageActions timestamp="06/10/2026 1:28 PM">
        <button type="button" aria-label="Copy" />
        <button type="button" aria-label="More" />
      </MessageActions>
    ) +
    `<p data-answer-text>${text}</p></div>`;

  const row = (role: 'user' | 'assistant', gap: string, cls: string, html: string) =>
    `<div class="relative ${cls}" data-role="${role}" data-gap="${gap}">${html}</div>`;

  async function lay(rows: string): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1000, height: 600 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${TOKENS}${TAILWIND_STANDINS}${css}</style></head>` +
        `<body><div style="width:${COLUMN}px;margin:40px 0 0 100px">${rows}</div></body></html>`
    );
    return tab;
  }

  type Rect = { top: number; bottom: number; left: number; right: number };

  /** The cluster in the n-th row (its box), and every glyph line box in the page. */
  const geometry = (tab: Page, rowIndex: number) =>
    tab.evaluate((index) => {
      const rect = (r: DOMRect) => ({ top: r.top, bottom: r.bottom, left: r.left, right: r.right });
      const rows = Array.from(document.querySelectorAll<HTMLElement>('[data-gap]'));
      const cluster = rows[index].querySelector('.br-message-actions')!;
      const ink: Array<{ row: number; box: ReturnType<typeof rect> }> = [];
      rows.forEach((r, i) => {
        r.querySelectorAll(
          "[data-answer-text], [data-testid='user-message-bubble'] > *, [data-provenance-kind]"
        ).forEach((el) => {
          const range = document.createRange();
          range.selectNodeContents(el);
          for (const box of Array.from(range.getClientRects())) {
            if (box.width > 0) ink.push({ row: i, box: rect(box) });
          }
        });
      });
      const chip = rows[index].querySelector('[data-provenance-kind]');
      const bubble = rows[index].querySelector("[data-testid='user-message-bubble']");
      return {
        cluster: rect(cluster.getBoundingClientRect()),
        chip: chip ? rect(chip.getBoundingClientRect()) : null,
        bubble: bubble ? rect(bubble.getBoundingClientRect()) : null,
        ink,
      };
    }, rowIndex);

  const overlaps = (a: Rect, b: Rect) =>
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;

  it("over an answer under a question, it covers no ink of either (V2-12's case)", async () => {
    if (!browser) return;
    const tab = await lay(
      row('user', 'first', 'mt-0 pt-7', user('u1', 'Then summarize it in one sentence.')) +
        row('assistant', 'turn', 'mt-6', answer(FULL_LINE))
    );
    try {
      const g = await geometry(tab, 1);
      // The answer's first line runs to the column's right edge, under where
      // the cluster floats.
      const firstLine = g.ink.find((i) => i.row === 1)!.box;
      expect(firstLine.right).toBeGreaterThan(g.cluster.left);
      for (const { box } of g.ink) expect(overlaps(g.cluster, box)).toBe(false);
      expect(g.cluster.bottom).toBeLessThanOrEqual(firstLine.top);
    } finally {
      await tab.close();
    }
  });

  it('over a question under an answer, it covers no ink of either', async () => {
    if (!browser) return;
    const tab = await lay(
      row('assistant', 'first', 'mt-0 pt-7', answer(FULL_LINE)) +
        row('user', 'turn', 'mt-6', user('u1', 'Shorter, please.'))
    );
    try {
      const g = await geometry(tab, 1);
      for (const { box } of g.ink) expect(overlaps(g.cluster, box)).toBe(false);
      expect(g.cluster.bottom).toBe(g.bubble!.top + 2);
    } finally {
      await tab.close();
    }
  });

  it('over a question 8px under another, it rises 18px and touches only paddings', async () => {
    if (!browser) return;
    const tab = await lay(
      row('user', 'first', 'mt-0 pt-7', user('u1', 'please write me a poem about tides')) +
        row('user', 'block', 'mt-2', user('u2', 'Run a shell command that prints a marker'))
    );
    try {
      const g = await geometry(tab, 1);
      expect(g.cluster.top).toBe(g.bubble!.top - 18);
      for (const { box } of g.ink) expect(overlaps(g.cluster, box)).toBe(false);
    } finally {
      await tab.close();
    }
  });

  it("beside a provenance chip, it sits on the chip's line to its left, over nothing", async () => {
    if (!browser) return;
    const tab = await lay(
      row(
        'user',
        'first',
        'mt-0 pt-7',
        user('u1', 'Count the files in the folder', {
          provenance: { kind: 'spawn_context', fromSessionId: 's-parent' },
        })
      )
    );
    try {
      const g = await geometry(tab, 0);
      expect(g.chip).not.toBeNull();
      expect(Math.abs(g.chip!.left - 8 - g.cluster.right)).toBeLessThanOrEqual(0.5);
      const chipMiddle = (g.chip!.top + g.chip!.bottom) / 2;
      const clusterMiddle = (g.cluster.top + g.cluster.bottom) / 2;
      expect(Math.abs(chipMiddle - clusterMiddle)).toBeLessThanOrEqual(0.5);
      expect(overlaps(g.cluster, g.chip!)).toBe(false);
      for (const { box } of g.ink) expect(overlaps(g.cluster, box)).toBe(false);
      // Nothing above the first row's 28px is cut off by the list's top edge.
      expect(g.cluster.top).toBeGreaterThanOrEqual(40);
    } finally {
      await tab.close();
    }
  });

  it('the first row leaves room for the rise: nothing pokes above the list', async () => {
    if (!browser) return;
    const tab = await lay(row('user', 'first', 'mt-0 pt-7', user('u1', 'Which genes moved?')));
    try {
      const g = await geometry(tab, 0);
      // The list starts at y=40 in this page.
      expect(g.cluster.top).toBeGreaterThanOrEqual(40);
    } finally {
      await tab.close();
    }
  });
});
